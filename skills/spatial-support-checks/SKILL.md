---
name: spatial-support-checks
description: Use when a builder wires stacked placement in crates/spatial that must rest on shelves and stairs before the fixture ships.
---
# Spatial support checks
Nine checks for support rest. Name the crate in each finding (spatial). PASS needs shelf and stair fixtures with 0 findings plus flat goldens unchanged and x2 equal, never a picture alone.
1. **Read the volume, not the floor.** Ask: does `solve` in `crates/spatial/src/place.rs` build one `Index3` in `crates/spatial/src/index3.rs` over scene-minus-object and satisfy `Relation::OnTopOf`/`Supports`/`Inside` in `crates/spatial/src/relation.rs` through `probe_of`/`overlaps_any`? Flag: ground-plane `y = sy/2` only that fails every raised shelf. (spatial)
2. **Ground versus support rest.** Ask: does the rest test in `crates/spatial/src/place.rs` accept `min_y` 0 for ground and supporter top within `SUPPORT_GAP_CM` in `crates/spatial/src/relation.rs` for support? Flag: one rest height that floats stairs or sinks shelves. (spatial)
3. **Flat rooms stay put.** Ask: do flat-room goldens from `crates/spatial/src/place.rs::solve` stay byte-identical after the support hunk? Flag: stacked fix that shifts a flat placement by one `GRID_CM`. (spatial)
4. **Broadphase before exact.** Ask: does clearance read candidates from `sweep_xz`/`live_boxes` in `crates/spatial/src/index3.rs` instead of a full `run` in `crates/spatial/src/checks.rs` per candidate? Flag: per-candidate full verify that grows with actors. (spatial)
5. **Stable order, never hash order.** Ask: does `solve` in `crates/spatial/src/place.rs` scan z then x then `YAW_STEPS` yaw and break ties by id in a `BTreeMap` at `GRID_CM` pitch? Flag: hash-order iteration that reorders the same intent. (spatial)
6. **Support is a query, not a guess.** Ask: is `Supports`/`OnTopOf` decided by `supported` in `crates/spatial/src/afford.rs` plus the `overlaps_any` probe, with `distance_cm` ignored for `Inside`/`OnTopOf`/`Supports` per `Want` in `crates/spatial/src/place.rs`? Flag: nearest-guess fallback that places on air. (spatial)
7. **Conflict, not invention.** Ask: does an impossible `Intent` in `crates/spatial/src/place.rs` return a minimal `ConflictSet` shrunk in intent order? Flag: invented transform that hides the clash. (spatial)
8. **Shelves and stairs prove it.** Ask: do two-floor ludus and siege-wall fixtures place on shelves and stairs with 0 `support` findings from `crates/spatial/src/checks.rs`? Flag: shelf placements that pass `solve` but trip `support` in `run`. (spatial)
9. **Same bytes twice.** Ask: does the same `Intent` over the same actors in any input order give identical `TransformOut` bytes twice via `place` in `crates/spatial/src/lib.rs`? Flag: run-to-run drift that moves bytes. (spatial)
## Do / Don't
- Do: read the index3 volume for support targets and rest on the supporter top; keep flat goldens byte-identical. Don't: rest everything on the ground plane or guess a placement when wants clash.
source: Gregory, Game Engine Architecture (2nd ed.), ch. 15.5.4
