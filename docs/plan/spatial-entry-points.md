# Spatial entry points for the door (lane B hand-off, 2026-10-18)

Every function here is `pub` in `crates/spatial`, integer cm inside, deterministic, and has no caller yet outside
tests. Lane A (or the lead) adds the `scene.query` / `scene.spatial` verbs; this table says which function answers
which agent question. Build the world once (`SightWorld::build(&actors)`) and reuse it across a call.

## Convention (SPR2, settled)

Right-handed, +Y up, +X east, +Z south, north is -Z. `holds(s, rel, o, frame)` reads in English as "`s` is `rel` of
`o`": `holds(a, Left, b, World)` = "a is left_of b" = a sits WEST of b. The old code answered the mirror question
("b is left of a"), so the door's `relation=left_of anchor=X` listed the objects EAST of X. It was inverted; fixed in
`relation.rs` (`holds_in_view`), all 92 directional gold rows swapped, `QA_EXPECTED` re-pinned (indices 0, 14, 15, 19,
20, 24, 25, 30, 44, 45 flipped, derived by an independent node script). `Frame::Subject` now uses the reference `o`'s
yaw (yaw 0 faces +X, +90 deg faces +Z); `Frame::Viewer(w)` uses `w`'s yaw, nearer to the viewer = in front. Hand gold:
`relation::vocab_tests::dir_gold_plain` (40 questions, 3 frames, drawn floor plan). The endpoint test
`query_tool_answers_relation_words_like_the_spatial_oracle` still compares door to oracle only; lane A should add
`relation_plain` (a floor-plan question in English) and put `pair_numbers` in the hits.

## New this wave

| Question an agent asks | Call | Notes |
|---|---|---|
| "What is left of X?" with numbers | `relation::subjects_of(actors, Rel::Left, x, frame)` + `pair::pair_numbers(actors, x, s, frame)` | `dist_cm`, `bearing_deg` (0 ahead, 90 right, 180 behind, 270 left), `gap_cm` |
| "Can A see B? What is in the way?" | `sees::sees_pair(&world, a, b)` | `visible`, `fraction_pct` (5-ray fan), `blocker{id, entry_cm, dist_cm}` |
| "Why is B not on screen?" | `sees::why_hidden(&actors, Draw{has_mesh,has_material}, b, eye_m, yaw)` | verdict word via `Hidden::word()`, one evidence number |
| "How much room around X? Which way do I push it?" | `measure::clearance(&actors, x, need_cm)` | `gap`, `push_pm`, `kind` exact/upper_bound/narrow, `holds` |
| "Gap / overlap / align / contain between A and B" | `measure::measure(&box_a, &box_b, &MeasureArgs)` | `shortfall_cm` is the number to act on |
| "How far can X slide?" | `slide::slide(&actors, x, Dir::parse("+x")?, max_range_cm)` | `free_cm`, `blocker`, `capped`, `already_blocked`; words in `Dir::words()` |
| "What is the floor under (x, z)?" | `slide::ground_below(&actors, x_cm, z_cm, from_y_cm)` | `(top_y_cm, id)` |
| "Is the desk top seated or floating? What holds this up?" | `run_spatial(&actors, &["seatedJoint","supportChain"], None)` (opt-in; `support::support_findings`) | seatedJoint yellow; supportChain red = embed, yellow = floating base (pct = solids stacked on it) |
| "Is this overlap real?" (by component role) | `triage::triage(&actors, &roles, &findings)`; `Role` is mapped by the caller from `ComponentKind` | `Intent` + `info` severity, unplayable errors first |
| "Which surfaces z-fight?" | `triage::z_fights(&actors, &roles, 1, in_view)` | ranked by area / (gap + 1); frustum ids optional |
| "Put X in front of / behind / left / right of Y" | `place::place_dir(&ObjectRef, &[DirWant], &actors)` | edge gap range in cm, `holds`-confirmed; `DirConflict.unmet` = minimal wants |
| "Put X outside Y / keep it where it is" | `place::place_outside(&Intent, &[OutsideTarget], &actors)` | current pose returned unchanged when it works; reason `does_not_fit_on_support` or `no_free_spot` |
| "What is the real extent? What is a stray?" | `extent::robust_extent(&actors)`; text via `fold::extent_text` | `excluded[{id, why, ratio}]`, `inflation_x10` |
| "Fold the 40 identical crates" | `fold::fold_lookalikes(&actors, &roles)` + `fold::fold_text`; `fold::problems_in_words(&actors, &report, 5)` | one counted row per look-alike set; colliders folded into their mesh |
| "Which room is this in? Where are the exits?" | `rooms::find_rooms(&actors, rooms::CAPSULE_CM)` then `.room_of(x_cm, z_cm)`, `.exits_of(id)`, `.rooms[i].members` | rooms numbered 1..n in scan order, exits carry width_cm; outside is id 0 |
| "Is this thing too big for its layout?" | `run_checks(&actors, &["giantObject"])` | now relative to the layout (SPF5); `repair` clamps to the layout span |
| "Why did my placement fail? Who blocks it?" | `place::solve` -> `ConflictSet.blocks` (`WantBlock{want, blocker, miss_cm, near_miss}`); door: `scene.place` `blocked_by` | SPR6. `Blocker::Object{id, overlap_cm, fix}` when the wants point at a taken spot (`fix` is the shortest plan push of that object, `place::escape`); `Wall`/`Target`/`Point` when the wants contradict; `miss_cm` is the cm the nearest clean pose that keeps the other wants misses this want; `reason` stays one counts-only sentence, the audit digest ignores `blocks` |
| "What did my write change?" | `staged::check_staged_with(&before, &after, &touched, &checks)` (or `check_staged_ids` for the full catalog), `staged::fix_for`; door: `spatial_delta` on `tx.apply` and `scene.place` (`delta=false` skips) | SPX3. Findings on touched ids (plus every changed row) only: `new_findings` (<=5, red first, each names both objects and the one-move fix), `resolved` (<=5), `more` (left out by the caps). The door runs the same check list as `scene.spatial`. The tx gate sees 1 m cubes on poses, so a collider-sized overlap it cannot see arrives here |
 (all pub; pick by question)

| Module | Entry point | Answers |
|---|---|---|
| `sense_text` | `view(&world, &Camera, w, h)`, `lidar(&world, origin_cm, fan_deg)`, `map(&world, center_cm, radius_cm)`, `pixel(...)` | "What does the player see?" as ~900 tokens of text (SP69); `Camera{eye_cm, yaw_deg}` |
| `isovist` | `isovist(origin_xz_cm, &walls)` then `area_cm2(&poly)`; walls from `sight::visible_area_cm2` | "How open is this spot?" visibility polygon / area |
| `exposure` | `exposure_per_tile(&world, &threats_m, &tiles)` + `exposure_lines` | "Which tiles are covered from threats?" (cover maps) |
| `kdtree` | `KdTree::build(&points).knn(q, k)` / `.radius(q, r)` | "Nearest k objects to a point", "everything within r" without scanning |
| `tag_lift` | `lift_label(label)` | "What tags does this mesh label suggest?" (SP46 proposals) |
| `scale` | `unit_scale_finding(&sizes_cm, extent_cm)` | "Is the whole scene in the wrong unit?" |
| `fps_visibility` | `fps_sight_set(&world, eye_m, yaw, &candidates, &mut out)` | "Which of these ids does this viewer see?" (cone + occlusion, set) |

## SPR2 receipt

Before: `holds(100, Right, 101, World)` meant "101 is right of 100"; door `right_of 101` returned things WEST of 101.
After: it returns things EAST. Digests that moved: relation v2 edge digests (Left/Right and Front/Behind edges swap
owner); no hashed simulation path reads them. Golden `d57b5c8ae3096614` untouched (spatial is not in the sim hash).

SPX3/visible_from: `relation::holds(.., Rel::Visible, ..)` now delegates to `sees::sees_pair` (same story + 3D center ray), so the two never disagree (`relation_visible_from_and_sees_never_disagree`).
