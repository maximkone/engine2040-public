# DP1 deep-dive 2026-10-12 (planner thinker for the stuck gate)

Row: DP1 endpoint door packs `net` + `perf` (board `docs/SPRINT-18-gaps.md`).
Tier A, Size M. Still open.

## Symptom

12 helpers in 72 h, still open, no planner in 48 h. All builders and
judges: perf door pack, net pack first slice, conditioner slice,
latency-knob split. Two landings: `3a1cd34c` (net pack), `819371be`
(perf pack).

## Root cause

The row bundles two packs and four proofs, and every writer lands one
slice and leaves. Perf is done: `perf.table` reads the newest quiet csv
into typed integer rows, newest-pick, read-only, listed by api.describe.
Net converges at 10% loss with `first_divergence: None` and repeats
identical. The latency knob (`latency_ticks` 0..=32) is typed, tested
and stable.

Three conjuncts keep the row open, all named in its own Remaining line:

1. Burst-drop knobs have no verb surface. `ConditionerConfig` carries
   jitter and bad-state loss fields, but `loopback_report` takes only
   loss, seed and latency. So writers keep landing "latency only, burst
   stays out" and the row waits on burst in silence.
2. The seeded-desync test proves nothing. It asserts `Some(0)` for full
   loss at two seeds. Full loss diverging at tick 0 is trivially true.
   The row wants the exact tick a seeded desync names.
3. No writer ever carries the full proof line: `packs::net` plus
   `packs::perf` x2, the full endpoint lib count, and the golden.

## Different approach

Stop slicing. One writer finishes in this order:

1. Decide burst in the open: either wire two typed burst knobs into
   `net.loopback` (same style, `bad_args` outside range, x2 repeat
   test), or write the deferral into the row and close DP1 on latency
   only. The silent split is what holds the row open.
2. Harden the desync test to an exact non-zero tick: partial loss with
   a fixed seed pair, assert the exact tick per seed plus x2 repeat.
   Full-loss `Some(0)` stays as the boundary case only.
3. Land with the single proof command below. Nothing else ticks the row.

## Proof command

`cargo test -p engine-endpoint --lib packs::net` green x2,
`cargo test -p engine-endpoint --lib packs::perf` green x2,
`cargo test -p engine-endpoint --lib` green with its count pasted,
`api.describe` lists `net.loopback` and `perf.table`,
golden `d57b5c8ae3096614` unchanged.
