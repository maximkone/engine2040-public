# Buffer reuse with counting-allocator floors — reserve once, reuse every tick

Owner crates: `engine-anim`, `engine-destruction`, `engine-combat`.
Consumers: the arena match (LOC2), the destruction system, the combat
step. Units: buffer counts (pieces per fighter per tick). Determinism:
buffers are pre-sized at reserve time; tick reuse must not grow them.
Errors: buffer shape mismatches fail at reserve time, not at tick.

## Pattern

Reserve buffers once (at load, bake, or first tick). On every
subsequent tick, reuse the same sized buffers instead of allocating
new ones. The counting-allocator floor proves the reuse: after
warmup, `snapshot().count` must be 0 for the hot path.

Three board rows (SL18–SL20, all ticked on the board) each
implemented this pattern in a different crate:

1. **SL18 locomotion buffer reuse** (`crates/anim/src/space.rs`):
   `Locomotion::update` previously allocated 3 buffers per fighter
   per tick (pose positions, rotations, contacts). The `prepare`
   method now owns every buffer; ticks reuse them, proven by
   `prepare_reuses_scratch` (`space.rs`). The counting
   allocator floor drops by the locomotion share.

2. **SL19 support-refresh scratch reuse** (`crates/destruction/src/bonded.rs`):
   `refresh_support` previously allocated 3 items per tick
   (supported map + work stack). `drain_into` reuses the scratch
   buffer; `support_scratch_is_presized_and_reused` proves it.

3. **SL20 combat event-buffer reuse** (`crates/combat/src/fight.rs`):
   Event/break ticks previously allocated per event. `try_step_into`
   reuses a reserved event buffer (reserved once to `MAX_EVENTS_PER_TICK`).
   Quiet ticks already read 0. The `alloc_probe` module proves the floor.

## Why it is reusable

The pattern is identical across all three crates: reserve, reuse,
prove with a counting allocator. Any new crate with a per-tick
path can apply it. The reserve size is the only crate-specific
detail; the reuse discipline and the proof harness are generic.

**Transferable to:** any crate that allocates inside a tick loop.
The recipe: (1) identify the per-tick allocation, (2) reserve once
at load or warmup, (3) reuse the buffer every tick, (4) add a
counting-allocator gate test that asserts 0 after warmup.

## Test

Each crate owns a steady-state reuse test. The canonical example is
`engine-anim`:

```sh
cargo test --locked -p engine-anim steady_state_reuses_buffers
```

`engine-combat` uses the `alloc_probe` gate:

```sh
cargo test --locked -p engine-combat alloc_probe
```

`engine-destruction` owns the support-refresh test:

```sh
cargo test --locked -p engine-destruction support_scratch_is_presized_and_reused
```

All three assert that after warmup, the counting allocator reads
0 heap allocations on the hot path.

## Reuse counter

This entry starts at **0** reuses. It becomes an asset at **2**
reuses and archives at **0** reuses across 10 waves.

| Wave | Packet | How it reuses |
|---|---|---|
| 28 | `SL18` | `crates/anim/src/space.rs` — locomotion buffer reuse; `prepare` owns every buffer, ticks reuse them |
| 29 | `SL19` | `crates/destruction/src/bonded.rs` — support-refresh scratch reuse; `drain_into` + presized scratch |
| 29 | `SL20` | `crates/combat/src/fight.rs` — event-buffer reuse; `try_step_into` with reserved scratch |
| 31 | `SL21` | Window-path per-frame allocation audit reuses this pattern for the RA1 submit path (open) |
| future | any new per-tick path | Reserve → reuse → counting-allocator gate; copy the `alloc_probe` pattern from `engine-combat` |

## Reference

Full signatures live in rustdoc (`cargo doc -p engine-anim --no-deps`,
`cargo doc -p engine-combat --no-deps`); the files that own this
page: `crates/anim/src/space.rs` (SL18), `crates/destruction/src/bonded.rs`
(SL19), `crates/combat/src/fight.rs` (SL20, `alloc_probe` module).
The counting allocator: `crates/perf/src/alloc.rs`.
