---
name: sand-rest-checks
description: "Use when a builder wires a down pose that must rest on the sand in crates/game and when a judge checks sunk bodies before it lands."
---

# Sand rest checks

Eight checks for bodies on sand. Name the crate per finding (game). PASS needs `sunk_cm_max` under 2 with `cargo test -p engine-game --lib` green and golden unchanged, never a picture alone.

1. **One sand plane.** Ask: does `GROUND_Y` in `crates/game/src/view_truth.rs` own the floor once for every sunk test? Flag: per-clip `y = 0` numbers that drift apart. (game)
2. **Sunk is a gap to the plane.** Ask: does `ViewLaws::sunk_m` in `crates/game/src/view_truth.rs` keep `GROUND_Y - body.min[1]` as the max over drawn bodies? Flag: root y used instead of the lowest vertex, so long limbs still bury. (game)
3. **Lowest vertex wins.** Ask: does `BodyTruth::fold_vertices` plus `has_bounds` in `crates/game/src/view_truth.rs` fold every finite posed vertex into `min`/`max` before the sunk read? Flag: skipped parts or non-finite vertices counted in the bounds. (game)
4. **Down lifts, idle floats apart.** Ask: does `Act::Down` in `crates/game/src/fighter_look.rs` lift only down bodies while `idle_float_m` in `crates/game/src/view_truth.rs` watches idle hover? Flag: one offset that lifts idle or hides float as sunk. (game)
5. **Lift up only, eased over the fall.** Ask: is the fix a +y lift of the down pose by its below-sand depth, spread over the fall, view only? Flag: snap to zero in one frame, sideways shift, or a sim position change. (game)
6. **Eye under floor stays separate.** Ask: does `ViewLaws::note` in `crates/game/src/view_truth.rs` count `eye_below_ground` from `eye[1] < GROUND_Y` apart from sunk? Flag: camera fix counted as body sunk, or `screen_rect` in `crates/game/src/view_truth.rs` trusted for depth. (game)
7. **The bar proves rest.** Ask: does the `arena view:` receipt `sunk_cm_max` in `crates/game/src/view_truth.rs` read under 2 and `tools/product-bar.json` `view_sunk_cm_max` tighten to 3? Flag: fixture lowest vertex more than 1 cm off the sand with no bar change. (game)
8. **View only, hash still.** Ask: does the hunk touch only `FighterLook` present data in `crates/game/src/fighter_look.rs` with `cargo test -p engine-game --lib` green and golden `d57b5c8ae3096614` unchanged? Flag: sim, tape, or hash moved for a picture fix. (game)

## Do / Don't
- Do: rest the down pose on the one sand plane by its lowest vertex and prove `sunk_cm_max` under 2 with the bar tightened.
- Don't: snap bodies to zero, move the sim, or land a sunk fix with no receipt number.
source: Lengyel, Foundations of Game Engine Development Vol 1, ch. 3.4
