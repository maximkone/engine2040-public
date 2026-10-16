# Counting-allocator floors — prove zero steady-state allocations per tick

Owner crate: `engine-perf` (`crates/perf`). Consumer: every crate
with a per-tick or per-frame hot path. Units: allocations (count)
and bytes. Determinism: thread-local cells, no allocation during
the counting window. Errors: none — the allocator forwards to
`System` and counts.

## Pattern

A thread-local counting allocator wraps `System`. While disabled,
it behaves identically to `System` (zero overhead). While enabled,
it tallies every allocation's count and bytes plus live and peak
high-water marks on thread-local `Cell`s. The `measure` helper
runs a closure with counting enabled and returns the value plus an
`AllocSnapshot`.

This pattern proves CODELAW §5 ("no steady-state allocation in per-tick
simulation or per-frame submission, proven by a counting-allocator
test, never by claim"). A crate claims zero allocations per tick by
running its hot path inside `measure` and asserting `snapshot().count == 0`.

The `CountingAlloc` struct is the `#[global_allocator]` for
`engine-perf`; it is disabled by default so unmeasured code behaves
exactly like `System`. Other crates bring their own probe when they
need a gate (e.g., `engine-combat`'s `alloc_probe::Probe`).

## Why it is reusable

Every crate with a hot path needs the same proof: run the path,
count allocations, assert zero. Instead of each crate hand-rolling
an allocator, `CountingAlloc` provides the pattern and the harness.
A crate either uses `engine-perf::measure` directly or copies the
`Probe` pattern from `crates/combat/src/fight.rs` (line 788) for
crate-local gates. The floor is the invariant; the allocator is the
tool that proves it.

**Transferable to:** any crate that has a per-tick path and needs to
prove it allocates zero. The pattern is crate-agnostic; only the
hot-path function changes.

## Test

The `engine-perf` crate owns the canonical test. It proves that
counting is off by default (so normal tests are unaffected) and that
enabling it actually counts allocations.

```sh
cargo test --locked -p engine-perf alloc::tests
```

Each consumer crate adds a gate test. The pattern from
`crates/combat/src/fight.rs` (line 788, `alloc_probe` module) shows
the crate-local variant: a `#[global_allocator]` probe that counts
only while the calling thread holds a gate open, then asserts the
count is zero for the hot path.

```sh
cargo test --locked -p engine-combat alloc_probe
```

## Reuse counter

This entry starts at **0** reuses. It becomes an asset at **2**
reuses and archives at **0** reuses across 10 waves.

| Wave | Packet | How it reuses |
|---|---|---|
| 28 | `SL18` | `crates/anim/src/space.rs` counts locomotion buffer reuse; `prepare` owns every buffer, ticks reuse them with zero allocs |
| 29 | `SL19` | `crates/destruction/src/bonded.rs` counts support-refresh scratch reuse; `drain_into` and `support_scratch_is_presized_and_reused` prove zero allocs |
| 29 | `SL20` | `crates/combat/src/fight.rs` `alloc_probe` module (line 788) proves combat event-buffer reuse: quiet ticks read 0, event ticks count 0 after warmup |
| 29 | `SL6` (closed) | `crates/arena/src/lib.rs` line 2518: SL6 steady-state allocation gate; the arena floor drops to 0 after SL18–SL20 complete it |
| 31 | `SL21` | Counting-allocator submit test over the RA1 window submit path (open; needs SL6, PF2) |
| future | any quality row | `node tools/proof.mjs` runs counting-allocator gates as part of the proof suite |

## Reference

Full signatures live in rustdoc (`cargo doc -p engine-perf --no-deps`);
the file that owns this page: `crates/perf/src/alloc.rs`.
Key items: `CountingAlloc`, `AllocSnapshot`, `reset_counters()`,
`snapshot()`, `live_bytes()`, `peak_bytes()`, `measure()`.
