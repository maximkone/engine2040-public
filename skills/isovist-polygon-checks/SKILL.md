---
name: isovist-polygon-checks
description: Use when a builder adds or reviews a visibility polygon or isovist sweep in crates/spatial; it checks ray math, ordering, and area before the diff lands.
version: 0.1.0
author: skillworks
---

# Isovist polygon checks

Nine checks for `crates/spatial/src/sight.rs` plus `crates/spatial/src/isovist.rs` (landed GS30). Integer cm in, sorted output, no float in hash.

1. **Segment form.** Ask: does each wall use p+tv with t in [0,1] inside `sight.rs::seg_hits`? Flag: infinite-line hits past endpoints. (spatial)
2. **Integer origin.** Ask: is the eye quantized once via `to_cm`/`CM_PER_M` in `index.rs`? Flag: float origin drifting per ray. (spatial)
3. **Parallel guard.** Ask: does a near-zero denominator return no-hit instead of dividing in `sight.rs::seg_hits`? Flag: NaN vertex from parallel ray. (spatial)
4. **Closest hit wins.** Ask: is the kept hit the smallest t>=0, ties by id order in `sight.rs::blocked`? Flag: far wall leaking through near wall. (spatial)
5. **Split crossings.** Ask: is each box tested alone by `blocked` with no split in `sight.rs`? Flag: missing shadow vertex behind a 1 m pillar. (spatial)
6. **Sweep order.** Ask: are boxes id-sorted at `SightWorld::build` with sorted `digest_answers` in `sight.rs`? Flag: unsorted angles giving a bow-tie polygon. (spatial)
7. **Closed area.** Ask: does `digest_answers` hash integer cm bytes in sorted order in `sight.rs`? Flag: open ring or float area that differs x2. (spatial)
8. **Sight reuse.** Ask: does occlusion agree with `los_point`/`SightWorld` in `sight.rs`, not a second ray test? Flag: isovist says visible where sight says blocked. (spatial)
9. **Determinism proof.** Ask: are outputs id-sorted with `lib.rs` export and `cargo test -p engine-spatial --lib` green x2? Flag: HashMap order or unexported module. (spatial)

## Do / Don't
- Do: keep one integer-cm sweep with sorted angles and id tie-breaks. Don't: add a float fast path that skips the split or the sort.
source: Lengyel, Foundations of Game Engine Development vol 1, ch. 3.3-3.4
