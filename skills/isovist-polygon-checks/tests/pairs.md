# Pairs for isovist-polygon-checks

## 1. Square room corners
Input: "Review `crates/spatial/src/isovist.rs` sweep from the centre of a 10 m square room."
With skill: "Check 1, 4, 6: each wall is a segment with t in [0,1], closest t>=0 wins, angles sorted. Pass is four corner vertices in id order, `area_cm2()` equals room area, x2 equal."
Without skill misses: accepts infinite-line hits or unsorted angles, so the polygon has extra vertices or a bow-tie and the area does not match the room.

## 2. Pillar shadow
Input: "Review `crates/spatial/src/isovist.rs` with a 1 m pillar in the room."
With skill: "Check 5, 4, 8: split crossings with `break_intersections` first, keep smallest t, agree with `los_point` in `sight.rs`. Pass adds shadow vertices behind the pillar and sight agrees on blocked."
Without skill misses: skips the split, so the pillar casts no shadow vertices and the polygon leaks through it while sight says blocked.

## 3. Parallel wall and determinism
Input: "Review `crates/spatial/src/isovist.rs` where a ray runs parallel to a wall; run `cargo test -p engine-spatial --lib` twice."
With skill: "Check 3, 2, 9: parallel denominator returns no-hit, eye quantized once via `to_cm` in `index.rs`, outputs id-sorted and exported in `lib.rs`. Pass has no NaN vertex and x2 identical."
Without skill misses: divides by near-zero for a NaN vertex, keeps a float eye that drifts per ray, and passes once but differs on the second run.
