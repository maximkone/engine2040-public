---
name: anim-combat
description: "Use when you change animation clips, blending, root motion, foot IK, character movement, the camera boom, fight timing or parry windows, combat rows, hit pause or shake, or a fighter sunk in the sand (crates anim, combat, game)."
---

# Animation, combat and body feel

Crates: `anim`, `combat`, `game` (look side), `physics` (bodies). Ragdoll, debris and the hashed-vs-float split: skill `physics-bodies`.

Search books for more: `node tools/books.mjs search "animation blending root motion"` (skill `research`).

## 1. Animation and body rules

1. Animate from sim state. Never feed an animated float pose into the hash. If an outcome must always happen one way, bake it as a table or animate it.
2. Order each frame: anim clocks, then event dispatch, then blend. Any other order costs one lag frame at clip end.
3. Cross-fade with a binary LERP node on the tree root, or with old group weights times (1 - lambda) and new weights times lambda. Rotations take the shortest arc (skill `render-pipeline-math`, quaternion check).
4. A looping clip of N frames stores N samples. Keeping the last sample too repeats the first, and the loop pops.
5. An additive clip is S times inverse(R): S is the pose, R the neutral reference. Keep shoulders and elbows neutral in R, or the limbs over-rotate.
6. Extract root motion before you zero the root. Scale playback rate to speed. Apply foot IK only in contact windows. Player feel beats animation purity.
7. Never LERP a live pose into a ragdoll pose. Drive bones with powered (PD) constraints, then release. Go ragdoll as soon as any bone touches solid.
8. Move characters with capsule casts and a manual slide, step and slope, not forces. Casts never change the world.
9. Limit ball joints by swing-twist: swing as an ellipse in quaternion space, twist as a clamped angle. Never three Euler clamps.
10. Remove a limb by flattening its child bone matrices to the cut joint on the shared mesh. Reverse the flattening for the detached limb.

Where it runs today: the live arena runs ragdolls and debris through `crates/game/src/ragdoll_look.rs` and `crates/game/src/break_look.rs` (physfx, float layer). Verify with Grep before you say a system is or is not live.

## 2. Fight timing tables (`crates/combat`)

Ask for each row of `crates/combat/src/tables.rs::MoveTableSet`:

| Check | Rule | Flag |
|---|---|---|
| From and to | each row names source and target | a `*` row that lets death go to attack |
| Kind | each change is instant, timed blend, or staged (`fight.rs::Tables`) | a cross-fade where down-to-stand needs a staged state |
| Length | startup 12-18 and parry 6-8 as integer ticks at 60 Hz | float seconds; v1 values carried into v2 |
| Curve | each timed blend names its ease shape | a blend factor that snaps at the ends |
| Window | a cancel fires only inside its source window (`FighterState`) | a punch-to-react in the first half of the swing |
| Sparse map | move matrix is sparse, death is a sink (`tables.rs::validate_tables`) | a dense matrix that lets death reach any move |
| Defaults first | `fight.rs::TABLES_V1` stays frozen; `tables.rs::tables_for_version(2)` overrides listed rows | a v2 edit that rewrites v1 bytes and breaks old tapes |
| Proof | `cargo test -p engine-combat --lib tables_v2` passes x2, golden unchanged | a green result with no digest, or a hash move with no receipt |

Today's numbers: v1 is startup 4, parry 4, hitstop 4 (`ATTACK_STARTUP_TICKS`, `PARRY_WINDOW_TICKS`, `HITSTOP_TICKS` in `fight.rs`) and is grandfathered. v2 (`tables.rs::v2_rows`) is startup 15, parry 7, inside the bands. The replacement rule: parry stays below startup. Start new rows from v2. List from, to, kind, length, curve and window per row. Never ship a timed change with no window or curve.

## 3. Combat rows text (`content/design/combat.rows`)

Numbers live in rows, Rust holds the truth, the text must match it. Loader: `crates/combat/src/tables_text.rs` (`pub mod tables_text` in `lib.rs`). Format doc: `docs/design/FORMAT.md`.

1. **Header first.** The first non-comment line is `table combat version <n>` (`parse_header`). A zero or missing version must refuse, not load.
2. **Known keys only.** Every key is in `KNOWN` (16 names, `Tables` order). A typo must refuse, not become a default.
3. **Whole numbers only.** Digits come from `engine_core::checked::decimal_u32` (no local copy). No dot, sign or empty value.
4. **Units in keys.** `_ticks` at 60 Hz, `_permille`, stamina points. No bare `damage` or `startup`.
5. **Validate before use.** `parse_tables` runs `tables.rs::validate_tables` before it builds a `SwapInput`.
6. **Digests equal truth.** Parsed v1 and v2 rows equal `TABLES_V1`, `TABLES_V2` and `v2_rows()`. A text fork that moves the golden is a bug.
7. **Refuse with a line.** Each refusal is a `CombatError` that carries the line number (parse) or the culprit value (range).
8. **Cold path only.** Std-only, called at a tick boundary, never inside `step_with_tables`. No heap in the step.
9. **Doc plus x2 proof.** `docs/design/FORMAT.md` matches the file. `cargo test -p engine-combat --lib tables_text` passes x2.

Never hard-code a tuned number in Rust. Never accept a float key.

## 4. Feel: hit pause, shake, squash, flash, slow motion

PASS needs an off/on pair plus three gates: frozen sim with live view, pure injected-time tests, release bench against the frame budget. Feel code: `crates/combat/src/feel.rs`, `hitstop_kind.rs`, `feel_instrument.rs`.

1. **Layer pair.** Show pause-off feels soft, pause-only feels empty, all-on lands. One all-on clip is not proof.
2. **Frozen sim, live view.** While frozen, sim dt reads zero but the view still trembles from the live clock. A locked view or a sim that advances during the freeze fails.
3. **Fixed-step carry.** The loop accrues real dt, runs whole fixed steps, carries the leftover slice. Raw frame dt into the sim fails (skill `tick-and-runtime`).
4. **Visible numbers.** The HUD prints live load plus remaining freeze. An eye-only verdict fails.
5. **Pure tests.** Feel tests inject dt. No window, canvas, wall clock, random call or timer mock in the tested path.
6. **Release bench.** Update and impact cost measured in release, with calls per frame at real counts. A debug-only number fails (skill `perf-bench`).
7. **Decay table.** Halved load falls to about a quarter move. Linear falloff, or no table in the diff, fails.
8. **Slow stretch.** Under slow motion the tremble stretches calm and cinematic. Layers do not fade during a freeze. Shake must respond to the slow factor.

Feel is view state. It never enters the hash (skill `physics-bodies` section 2).

## 5. Camera boom (`crates/game/src/camera.rs`, `camera_arm.rs`)

`camera_arm.rs::arm_length(pivot, dir, max_len, radius, boxes)` is a pure sphere sweep against `camera_arm::Aabb` boxes inflated by the radius, slab-tested per axis. Returns `[FLOOR_LEN (0.3), max_len]`. `camera.rs::DirectorParams` holds `base_dist_m`, `min_dist_m`, `max_dist_m`, `boom_up`, `boom_back`; `DirectorParams::validate` returns `CameraError`.

1. **Sweep with radius.** A zero-radius ray clips pillar corners.
2. **Floor on inside start.** A pivot inside a box returns the floor length, never zero or NaN.
3. **Front block has a rule.** A pillar between pivot and fighter zooms in or fades the blocker. The lens never pierces it, and never swings wide around it.
4. **Keep heading, shorten length.** A side hit shortens the arm only. No yaw kick on light contact.
5. **Cap the pitch.** Vertical adjust stays small so the horizon stays readable.
6. **Look only, never sim.** Arm length is not in state or replay bytes. Golden `d57b5c8ae3096614` unchanged.
7. **Same inputs, same length.** Non-finite input is refused or floored. Test x2 equal.
8. **Cheap and heap-free.** O(boxes), no alloc in the loop, inside the 0.05 ms budget stated in `camera.rs`.

Check before you say it is wired: `camera_arm.rs` says `camera.rs` calls `arm_length` "in a later hunk". Grep for the call. PASS needs a length test plus x2 equal, not a picture alone. Do: shorten along the arc, ease back when clear. Don't: swing yaw or pitch to dodge stone.

## 6. Down pose on sand (`crates/game/src/view_truth.rs`, `fighter_look.rs`)

1. **One sand plane.** `view_truth.rs::GROUND_Y` owns the floor for every sunk test. No per-clip `y = 0`.
2. **Sunk is a gap to the plane.** `ViewLaws::sunk_m` keeps the max of `GROUND_Y - body.min[1]` over drawn bodies. Use the lowest vertex, never the root y.
3. **Lowest vertex wins.** `BodyTruth::fold_vertices` plus `has_bounds` fold every finite posed vertex into `min`/`max`. Non-finite vertices stay out of the bounds.
4. **Down lifts, idle floats apart.** Only bodies that lie on the sand lift (`fighter_look.rs::lies_on_sand`: `Act::Down` or `GaitPhase::Fallen`). `idle_float_m` watches idle hover on its own.
5. **Lift up only, eased over the fall.** The fix is a +y lift by the below-sand depth (`fighter_look.rs::sand_lift_m`, cap `SAND_LIFT_MAX_M`), spread over the fall, view only. No snap to zero, no sideways shift, no sim position change.
6. **Eye under floor stays separate.** `ViewLaws::note` counts `eye_below_ground` (`eye[1] < GROUND_Y`) apart from sunk. Do not trust `screen_rect` for depth.
7. **The bar proves rest.** The `arena view:` receipt line shows `sunk_cm_max`. It reads under 2. `tools/product-bar.json` `view_sunk_cm_max` is 3 today.
8. **View only, hash still.** The diff touches `FighterLook` present data. `cargo test -p engine-game --lib` is green. Golden unchanged.

Do: rest the down pose on the one plane by its lowest vertex and show the `sunk_cm_max` number. Don't: snap bodies to zero, move the sim, or land a sunk fix with no receipt number.

Sources: Gregory, Game Engine Architecture 2nd ed. ch. 11.4, 11.6, 11.10, 11.11, 12.3-12.5; Game Engine Gems 1 ch. 19; Game Engine Gems 3 ch. 10; Lengyel, Foundations of Game Engine Development 1 ch. 3.4; Wolverson, Hands-on Rust ch. 15; game-feel-hitstop-juice v1.0.0.
