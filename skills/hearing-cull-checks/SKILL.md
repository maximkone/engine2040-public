---
name: hearing-cull-checks
description: "Use when a builder adds or reviews hearing events in crates/ai and when a judge checks the sound table before it lands."
---

# Hearing cull checks

Eight checks for AI hearing. Name the crate per finding (ai). PASS needs table tests plus x2 equal with `cargo test -p engine-ai --lib` green, never ears alone.

1. **Kinds as data.** Ask: does the new hearing query `heard_by` (no module yet; model it on `crates/ai/src/eqs.rs`, record hears with `crates/ai/src/mind.rs::MemoryKind::Heard`) read kind, range and life from a table (gunfire 4000cm for 120 ticks, footstep 2400cm for 90, quiet 1000cm for 30) with caller-owned buffers like `crates/ai/src/eqs.rs::EqQuery`? Flag: per-kind branches or hidden globals. (ai)
2. **Range before loudness.** Ask: is distance tested first and strength only inside range, so a far footstep at 2500cm never hears while a near gunshot at 3900cm does, in capped scans like `crates/ai/src/eqs.rs::MAX_POINTS`? Flag: loud-far beats near-quiet. (ai)
3. **One fixed pitch.** Ask: does the table use one fixed distance step, not per-kind cells, like the fixed cap in `crates/ai/src/eqs.rs::MAX_POINTS`? Flag: unequal grouping that costs more per query. (ai)
4. **Static once, movers per tick.** Ask: are fixed noises built once and moving ones re-seated each tick, with insertion order kept like `crates/ai/src/blackboard.rs::Blackboard`? Flag: rebuild-all per tick or hash-order listeners. (ai)
5. **Keep the last list.** Ask: does the brain keep the prior audible set, mark all for drop, then unmark still-audible ones, like `crates/ai/src/memory.rs::MemoryStore` keeps rows? Flag: fresh list each tick that flaps. (ai)
6. **Loudest first, stop when full.** Ask: are candidates read loudest-first and copied only until channels fill, in `0..=1000` scale like `crates/ai/src/eqs.rs::SCORE_MAX`, then bail? Flag: low-priority scan after full. (ai)
7. **Louder replaces.** Ask: does a louder live noise replace the current one instead of mixing, decided by one integer compare in `crates/ai/src/depth.rs::DepthBrain` style? Flag: two noises mixed or first-wins. (ai)
8. **Integers twice equal.** Ask: will the new `heard_by(listener_cm, noise, tick)` return `Option<u16>` integers, refuse bad ids like `crates/ai/src/eqs.rs::EqError`, and give identical bytes twice? Flag: float strength or run-to-run drift. (ai)

## Do / Don't
- Do: table kinds with range plus life, test distance first, keep the old set with drop flags, copy loudest-first until full, and prove x2 equal.
- Don't: branch per kind, hear far-quiet over near-loud, rebuild all per tick, mix noises, or return floats.
source: Game Engine Gems 1, ch. 5
