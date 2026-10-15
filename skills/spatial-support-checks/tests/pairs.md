# Skill-on vs skill-off pairs: spatial-support-checks
Three stacked-placement reviews. Off is general habit; On names the checklist item and the proof. Pass = On cites the item, the failure, and the test.
## P1: ground-only solver fails the shelf (items 1, 2, 8)
Input: `solve` in `crates/spatial/src/place.rs` rests every candidate at `y = sy/2`. A book on a shelf fails verification and falls to the floor. Flat rooms pass.
With skill: Item 1: build one `Index3` in `crates/spatial/src/index3.rs` over scene-minus-object and read `OnTopOf`/`Supports` through `probe_of`/`overlaps_any`. Item 2: accept supporter top within `SUPPORT_GAP_CM` in `crates/spatial/src/relation.rs` for support rest. Item 8: prove two-floor ludus and siege-wall fixtures place with 0 `support` findings from `crates/spatial/src/checks.rs`.
Without skill misses: the ground-only rest; approves the floor fallback because flat goldens stay green and no shelf fixture runs.
## P2: hash order plus full verify per candidate (items 4, 5, 9)
Input: `solve` in `crates/spatial/src/place.rs` iterates a hash map and calls full `run` per candidate. Same intent in shuffled actor order returns a different shelf slot. Cost grows with actors.
With skill: Item 5: scan z then x then `YAW_STEPS` yaw in a `BTreeMap` at `GRID_CM` pitch with id tie-breaks. Item 4: read candidates from `sweep_xz`/`live_boxes` in `crates/spatial/src/index3.rs`, not full `run` in `crates/spatial/src/checks.rs`. Item 9: prove the same `Intent` in any input order gives identical `TransformOut` bytes twice via `place` in `crates/spatial/src/lib.rs`.
Without skill misses: the order drift and the quadratic verify; calls one order-correct run proof enough and ships nondeterminism.
## P3: invented placement hides the clash (items 6, 7)
Input: an impossible `Inside` plus `OnTopOf` intent in `crates/spatial/src/place.rs` returns a floating transform with no error. Reviewer cannot tell which want failed.
With skill: Item 7: return a minimal `ConflictSet` shrunk in intent order from `crates/spatial/src/place.rs`, never a guess. Item 6: decide `Supports`/`OnTopOf` by `supported` in `crates/spatial/src/afford.rs` plus the probe, with `distance_cm` ignored per `Want`.
Without skill misses: the invented air placement; praises no-failure placement and misses that the solver now lies about support.
