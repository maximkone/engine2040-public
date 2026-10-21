---
name: spatial-perception
description: "Use when you add tile visibility, FOV, isovist polygons, sight-gated chase, explored-map memory, AI hearing, or placement on shelves and stairs (crates spatial, ai, game, render): opacity versus blocking, integer cm, id order."
---

# Spatial perception

Four areas: sight and FOV, isovist polygon, AI hearing, stacked support. They share one law.

## Shared law

- Integer cm inside the sim. Quantize a float once at the edge (`crates/spatial/src/index.rs::to_cm`, `CM_PER_M`).
- Output is id-sorted. No `HashMap` order, no clock, no float in a hash.
- Same input twice gives the same bytes twice. Prove it with an x2 test.
- PASS needs a test or a measured cost per touched item. A picture alone is not proof.
- Proof lines (through GitHub, skill `github-check`): `cargo test -p engine-spatial --lib`, `cargo test -p engine-ai --lib`.

What is in the tree today (verify before you build on it):
- `crates/spatial/src/sight.rs` (`SightWorld::build`, `los_point`, `digest_answers`): 3D line of sight over boxes.
- `crates/spatial/src/fps_visibility.rs`: per-viewer sight sets (FOV cone plus occlusion), id-sorted, capped by `FPS_MAX_CANDIDATES`. AI brains gate chase on membership. Net interest intersects with the set.
- `crates/spatial/src/isovist.rs`: integer-cm visibility polygon and `area_cm2`.
- `crates/spatial/src/place.rs`, `index3.rs`, `relation.rs`, `afford.rs`: placement with support.
- No tile-grid FOV or explored-map module was found in `crates/` by grep on 2026-10-17. Sections 1 items 1-3 and 7 apply to any new tile-visibility module.

## 1. Sight and FOV (viewer-based)

1. **Opacity apart from blocking.** Vision blocking has its own flag, apart from movement blocking. Flag: one flag drives both, so a slit or window behaves wrong. (spatial for tiles, physics for motion)
2. **Visible set per viewer.** Each viewer keeps its own deduped point set and a radius tuned per kind. Flag: one shared list or one radius for all kinds.
3. **Dirty-gated recompute.** Rebuild the set only when its owner moves. Start dirty, clear after the run. Flag: full rebuild every tick for every viewer.
4. **Order after movement.** The sight pass runs after movement settles and before draw and chase, in each schedule that moves. Flag: draw or chase reads last turn's spots.
5. **Draw gated on sight.** Tiles, bodies and hover tips show only when the viewer set holds the point. Flag: hidden foes drawn or named by hover. (render, ui)
6. **Chase gated on sight.** Pursuit starts only when the target is inside the chaser set and stops when sight breaks. Flag: foes path through walls at any range. (ai)
7. **Memory kept apart.** Explored state is a separate bool array. It is set from the viewer set on move and never cleared. Seen draws bright, recalled draws dim. Flag: recall stored inside the live set, or wiped each move.
8. **Same input same set, plus cost.** Same origin, radius and map give the same set. Measure build cost at real viewer counts. Flag: shifting order leaks into sim state, or full-map scans with no number.

Do: recompute on move, gate chase and draw on set membership, keep recall apart from sight. Don't: scan each tick, chase through walls, fold recall into the live set.

## 2. Isovist polygon (`crates/spatial/src/sight.rs`, `isovist.rs`)

Entry points: `isovist::isovist(origin, walls)`, `split_crossings`, `area_cm2`, `isovist_ray_budget`, `MAX_COORD_CM` (2^24 cm). Integer cm in, sorted output, no float in the hash.

1. **Segment form.** Each wall is p + t*v with t in [0,1] (`sight.rs::seg_hits`, private). Flag: infinite-line hits past endpoints.
2. **Integer origin.** Quantize the eye once with `to_cm`. Flag: float origin drifting per ray.
3. **Parallel guard.** A near-zero denominator returns no-hit. It never divides. Flag: NaN vertex from a parallel ray.
4. **Closest hit wins.** Keep the smallest t >= 0, ties by id order (`sight.rs::blocked`). Flag: a far wall leaking through a near wall.
5. **Split crossings.** `isovist.rs::split_crossings` splits crossing walls at integer-cm points before the sweep. Collinear overlaps stay as they are. Flag: missing shadow vertex behind a 1 m pillar.
6. **Sweep order.** Boxes are id-sorted at `SightWorld::build`. Walls, endpoints and angles are sorted before the sweep. Flag: unsorted angles giving a bow-tie polygon.
7. **Closed area.** `area_cm2` is the shoelace over `i128`, sign-independent, 0 under 3 points. `digest_answers` hashes integer cm bytes in sorted order. Flag: an open ring, or a float area that differs x2.
8. **Sight reuse.** Occlusion agrees with `los_point` / `SightWorld`. No second ray test. Flag: isovist says visible where sight says blocked. The `parity_*` tests compare the ring with a brute-force segment test.
9. **Determinism proof.** Outputs id-sorted, module exported in `lib.rs`, `cargo test -p engine-spatial --lib` green x2. Flag: HashMap order, or an unexported module.

Do: one integer-cm sweep with sorted angles and id tie-breaks. Don't: add a float fast path that skips the split or the sort.

## 3. AI hearing (`crates/ai`)

`heard_by` does not exist yet (grep of `crates/ai/src` found nothing). This section is a design target. Model it on `crates/ai/src/eqs.rs` (`EqQuery`, `MAX_POINTS` 256, `SCORE_MAX` 1000, `EqError`). Record hears with `crates/ai/src/mind.rs::MemoryKind::Heard`. PASS needs table tests plus x2 equal with `cargo test -p engine-ai --lib`.

1. **Kinds as data.** Kind, range and life come from a table: gunfire 4000 cm for 120 ticks, footstep 2400 cm for 90, quiet 1000 cm for 30. Caller-owned buffers like `EqQuery`. Flag: per-kind branches or hidden globals.
2. **Range before loudness.** Test distance first, strength only inside range. A far footstep at 2500 cm never hears. A near gunshot at 3900 cm does. Flag: loud-far beats near-quiet.
3. **One fixed pitch.** One fixed distance step in the table, not per-kind cells (compare the fixed cap `MAX_POINTS`). Flag: unequal grouping that costs more per query.
4. **Static once, movers per tick.** Fixed noises are built once. Moving ones are re-seated each tick, with insertion order kept (`crates/ai/src/blackboard.rs::Blackboard`). Flag: rebuild-all per tick, or hash-order listeners.
5. **Keep the last list.** The brain keeps the prior audible set, marks all for drop, then unmarks the still-audible (`crates/ai/src/memory.rs::MemoryStore`). Flag: a fresh list each tick that flaps.
6. **Loudest first, stop when full.** Read candidates loudest-first, scale `0..=1000`, copy only until the channels fill, then stop. Flag: a low-priority scan after full.
7. **Louder replaces.** A louder live noise replaces the current one by one integer compare (`crates/ai/src/depth.rs::DepthBrain` style). Flag: two noises mixed, or first-wins.
8. **Integers twice equal.** `heard_by(listener_cm, noise, tick)` returns `Option<u16>` integers, refuses bad ids like `EqError`, and gives identical bytes twice. Flag: float strength or run-to-run drift.

Do: table kinds with range plus life, distance first, old set with drop flags, loudest-first until full. Don't: branch per kind, hear far-quiet over near-loud, mix noises, return floats.

## 4. Stacked support (`crates/spatial/src/place.rs`)

PASS needs shelf and stair fixtures with 0 findings, flat-room goldens unchanged, x2 equal.

1. **Read the volume, not the floor.** `place.rs::solve` builds one volume index (`index3.rs`) over scene-minus-object. It satisfies `Relation::OnTopOf` / `Supports` / `Inside` (`relation.rs`) through `probe_of` and `overlaps_any`. Flag: ground-plane `y = sy/2` only, which fails every raised shelf.
2. **Ground versus support rest.** The rest test accepts `min_y` 0 for ground, and a supporter top within `relation.rs::SUPPORT_GAP_CM` (5 cm) for support. Flag: one rest height that floats stairs or sinks shelves.
3. **Flat rooms stay put.** Flat-room goldens stay byte-identical after a support change. Flag: a stacked fix that shifts a flat placement by one `GRID_CM` (10 cm).
4. **Broadphase before exact.** Clearance reads candidates from `index3.rs::sweep_xz` / `live_boxes`. It does not run a full `checks.rs::run` per candidate. Flag: per-candidate verify that grows with actors.
5. **Stable order, never hash order.** `solve` scans z, then x, then `YAW_STEPS` (8) yaws, ties by id in a `BTreeMap`, at `GRID_CM` pitch. Flag: hash-order iteration that reorders the same intent.
6. **Support is a query, not a guess.** `Supports` / `OnTopOf` is decided by `afford.rs::supported` plus the `overlaps_any` probe. `distance_cm` is ignored for `Inside` / `OnTopOf` / `Supports`. Flag: a nearest-guess fallback that places on air.
7. **Conflict, not invention.** An impossible `Intent` returns a minimal `ConflictSet`, shrunk in intent order. Flag: an invented transform that hides the clash.
8. **Shelves and stairs prove it.** Two-floor ludus and siege-wall fixtures place on shelves and stairs with 0 `support` findings from `checks.rs`. Flag: placements that pass `solve` but trip `support` in `run`.
9. **Same bytes twice.** The same `Intent` over the same actors, in any input order, gives identical `TransformOut` bytes twice via `place` (`lib.rs`).

Do: read the volume for support targets, rest on the supporter top, keep flat goldens byte-identical. Don't: rest everything on the ground plane, or guess when wants clash.

Sources: Wolverson, Hands-on Rust ch. 10 (FOV, hiding entities); Lengyel, Foundations of Game Engine Development vol 1 ch. 3.3-3.4; Game Engine Gems 1 ch. 5 (hearing); Gregory, Game Engine Architecture 2nd ed. ch. 15.5.4.
