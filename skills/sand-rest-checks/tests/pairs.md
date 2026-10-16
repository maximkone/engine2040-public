# Pairs for sand-rest-checks

## 1. Death clip buries the lowest hand

Input: a `down` body in `crates/game/src/fighter_look.rs` rests on its root y at `GROUND_Y`, but one posed hand vertex sits 12 cm under the sand; `ViewLaws::sunk_m` in `crates/game/src/view_truth.rs` reads 12.0 and `tools/product-bar.json` still allows 20.

Answer with the skill: fail checks 2 and 3. Fold every finite vertex with `BodyTruth::fold_vertices` plus `has_bounds`, keep `GROUND_Y - body.min[1]` as `sunk_m`, lift the down pose by its below-sand depth eased over the fall, then tighten `view_sunk_cm_max` to 3 with `sunk_cm_max` under 2.

What a skill-less answer misses: it trusts root height, so the receipt stays green while the hand stays buried and the bar never moves.

## 2. Snap to zero pops the fall

Input: the down pose is fixed by setting every below-sand vertex y to `GROUND_Y` in one frame; the fall pops and `idle_float_m` in `crates/game/src/view_truth.rs` now hides the step.

Answer with the skill: fail checks 4 and 5. Lift only `Act::Down` bodies by +y spread over the fall, keep `idle_float_m` for idle hover apart, view only in `FighterLook` present data with golden unchanged.

What a skill-less answer misses: it likes the zero number, so the fix pops on video and an idle hover is counted as sunk rest.

## 3. Low camera counted as sunk bodies

Input: the fight camera dips under the floor, `eye_below_ground` in `crates/game/src/view_truth.rs` rises, and the diff claims bodies sank because `sunk_m` moved.

Answer with the skill: fail checks 6 and 7. Count `eye[1] < GROUND_Y` apart from body sunk, never trust `screen_rect` for depth, and prove rest with the `arena view:` `sunk_cm_max` receipt plus a fixture lowest vertex within 1 cm of the sand.

What a skill-less answer misses: it mixes the camera dip into the body metric, so a camera fix lands as a body fix with no fixture proof.
