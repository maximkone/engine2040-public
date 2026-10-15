# Contract-test patterns — typed cross-crate invariants proved by test

Owner crates: `engine-ecs`, `engine-combat`. Consumers: any crate
with a cross-crate boundary that needs a typed invariant proved by
test. Units: bytes, digests, counts. Determinism: tests use seeded
inputs; same inputs give same digests. Errors: typed test failures
that name the violated invariant.

## Pattern

A contract test proves an invariant across crate boundaries. Unlike
a unit test (which tests one function) or an integration test (which
tests a feature), a contract test proves that two crates agree on
their shared protocol. The invariant is typed: the test fails if
the bytes, digests, or structures diverge.

Two canonical examples on the board:

1. **SL10 bridge wire roundtrip** (`crates/ecs/src/bridge.rs`):
   `encode_arena`/`encode_scene` output must roundtrip through
   `ingest_*` with identical bytes. The test lives in the bridge
   module and asserts `roundtrip preserves bytes + digest`. This
   proves the cross-crate wire format is lossless.

2. **SL20 alloc_probe** (`crates/combat/src/fight.rs`, line 788):
   A crate-local counting-allocator probe that proves the combat
   event-buffer reuse invariant: quiet ticks read 0, event ticks
   allocate nothing after warmup. The probe is a `#[global_allocator]`
   that counts only while the calling thread holds a gate open.

The common recipe: (1) define the invariant as a typed assertion,
(2) run the hot path inside the probe, (3) assert the counts match
the floor.

## Why it is reusable

Every cross-crate edge needs a contract test. The pattern is
generic: pick the shared protocol, write a typed assertion that
proves it holds, run it under the counting allocator. Instead of
each crate inventing its own test harness, the `alloc_probe` module
in `engine-combat` and the `bridge` roundtrip test in `engine-ecs`
provide the template.

**Transferable to:** any crate pair with a wire format, data exchange,
or shared invariant. The recipe: (1) identify the invariant, (2)
write a typed test that fails when it breaks, (3) add a counting-allocator
gate to prove the performance floor.

## Test

The `engine-ecs` bridge roundtrip test:

```sh
cargo test --locked -p engine-ecs wire_roundtrip
```

The `engine-combat` alloc_probe gate:

```sh
cargo test --locked -p engine-combat alloc_probe
```

Both assert typed invariants: the bridge test asserts byte-and-digest
preservation; the alloc_probe asserts zero allocations on the hot path.

## Reuse counter

This entry starts at **0** reuses. It becomes an asset at **2**
reuses and archives at **0** reuses across 10 waves.

| Wave | Packet | How it reuses |
|---|---|---|
| 21 | `SL10` | `crates/ecs/src/bridge.rs` — bridge wire roundtrip test; `encode_arena`/`encode_scene` must roundtrip with identical bytes |
| 29 | `SL20` | `crates/combat/src/fight.rs` `alloc_probe` module — counting-delegate allocator proves zero-alloc event-buffer reuse |
| future | any cross-crate edge | Define the invariant as a typed test; add a counting-allocator gate; copy the `alloc_probe` pattern from `engine-combat` |

## Reference

Full signatures live in rustdoc (`cargo doc -p engine-ecs --no-deps`,
`cargo doc -p engine-combat --no-deps`); the files that own this
page: `crates/ecs/src/bridge.rs` (SL10 roundtrip), `crates/combat/src/fight.rs`
(SL20, `alloc_probe` module at line 788).
The counting allocator: `crates/perf/src/alloc.rs`.
