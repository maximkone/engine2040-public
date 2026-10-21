# The spine's slot cap was 16 while the participant cap is 32 (2026-10-21)

## The bug

The participant cap moved 16 -> 32 on 2026-10-20 (owner decision GE12):
`MAX_PARTICIPANTS` is 32 in `crates/input/src/fighter.rs:29`, the arena's
`MAX_SLOTS` follows it, the ui's `PARTICIPANT_CAP` is 32, and the combat
crate's `SLOT_COUNT` is `MAX_PARTICIPANTS`.

`crates/ecs/src/spine.rs` did not follow. It still read:

```rust
/// Most actor slots one spine holds (the input contract's participant cap).
pub const MAX_SPINE_SLOTS: usize = 16;
```

The doc comment already called it the participant cap; the value was stale.

What that cost: `Spine::with_capacity` clamps the reservation to the cap
(`spine.rs:367`) and `Spine::spawn` refuses past it (`spine.rs:402`, with
`SpineError::TooManySlots`). The arena's `spawn_columns`
(`crates/arena/src/spine_step.rs:326`) spawns one slot per fighter and turns
that refusal into `None` (`spine_step.rs:345`), so `SpineMatch::build`
refused a 32-fighter match and `run_spine` answered with an EMPTY checkpoint
list. The legacy path seated the same fight fine.

So the two arena paths disagreed on a match size the engine claims to
support, and the spine path said nothing at all: no match, no error, no
checkpoint.

## The fix

- `crates/ecs/src/spine.rs`: `MAX_SPINE_SLOTS` 16 -> 32, with the reason in
  the constant's doc comment.
- `crates/arena/src/spine_step.rs`: the regression test
  `a_full_size_match_builds_a_spine`. A 32-fighter match builds a spine,
  runs its 60 ticks, and the spine checkpoints equal the legacy path's
  checkpoints - so the two paths agree again on the full-size fight.
- `crates/perf/src/workloads.rs`: the canonical spine shapes follow the
  constant instead of a copied 16 (`ENTITIES` = `MAX_SPINE_SLOTS` x 625 =
  20,000 entity-steps, `AGENTS` = `MAX_SPINE_SLOTS` x 64 = 2,048
  agent-ticks). The pass count and the digest-per-pass mix did not move, so
  the rate the OPT14 bars were written for is unchanged.
- `crates/perf/src/baselines.rs`: the two spine rows re-recorded from the
  live runs (a deliberate design change - the fixed array size moved - which
  the table's own rule allows). Measured:
  `spine_transform_body` hash 17614562845881895340 -> 9370682329320749301,
  `spine_full_tick` hash 5694617205498698940 -> 1258135662891929159, and
  both allocation totals 1424 -> 2848 bytes with 6 allocations (the wider
  reservation). The 16-slot numbers live in the comment above the rows; no
  row was deleted.
- `crates/perf/src/baselines.rs`: the two OPT14 millisecond bars keep the
  same rate per unit of work, so they scale with the work the run now
  covers: `spine_transform_body` 0.5 -> 1.0 ms, `spine_full_tick` 2.0 ->
  4.0 ms.

## What did not move

The legacy golden `d57b5c8ae3096614` stays: the proof crate's spine parity
tapes seat 2 to 4 fighters, so the cap never bit them
(`crates/proof/src/spine_parity.rs`).

## Still owed

The release-mode p50 re-measurement of the two spine rows
(`cargo test --release -p engine-perf -- --ignored --test-threads=1`) is an
open item on this box: the local release build is memory-bound while other
lanes build (4.8 GB free of 15.7 GB, and the release link was killed twice
at exit 255). The two bars above are the derived rate, not a fresh
measurement, and the receipt says so.

## Proof

- `cargo test -p engine-ecs --all-targets` -> 138 passed; 0 failed; 9 ignored (lib), 3 passed (tests), 2 passed (tests)
- `cargo test -p engine-arena --all-targets` -> 293 passed; 0 failed; 7 ignored
- `cargo test -p engine-perf --all-targets` -> 149 passed; 27 passed; 53 passed (3 ignored); 10 passed (1 ignored); 2 passed (1 ignored); 0 failed in every target
- `cargo test -p engine-proof --lib golden` -> 6 passed; 0 failed (the golden hash `d57b5c8ae3096614` unmoved)
- `cargo clippy -p engine-ecs -p engine-arena -p engine-perf --all-targets -- -D warnings` -> exit 0
- the same arena run names the new test: `spine_step::tests::a_full_size_match_builds_a_spine ... ok`
- `cargo test -p engine-ecs --all-targets` and `-p engine-arena --all-targets` also ran green on the GitHub runner (runs 39636725864 and 39637740089, the same counts); the clippy leg is green there too (run 39638216244)
