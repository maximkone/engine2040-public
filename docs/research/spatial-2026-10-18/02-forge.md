# Spatial research 02: what C:\forge teaches us (2026-10-18)

Read-only pass over the owner's C# Forge repo (no build, no run, nothing edited there). Policy: stealing
from the owner's own code is approved. Paths below are inside C:\forge unless they start with `crates/`,
`docs/` or `fixtures/` (those are engine2040). Companion: `01-ours.md` (what we have; its SPR1..SPR10 rows
and the board's AD25, AD18, AD20, SP66, N-07, N-12, N-15 are cited, not repeated).

## 1. Short answer

- Forge has no region/zone query; its nav queries are a baked-grid BFS with engine FindPath first
  (packs/nav/NavQueries.cs headers only, bodies not read).
  Its spatial value is in four places: (a) how quality was MEASURED (false-positive controls on real scenes,
  agent-in-the-loop Scene IQ), (b) a few exact box tools we lack (frame fit, surface-gap gradient, slide sweep),
  (c) triage that stops the checks crying wolf, (d) tool-surface hygiene.
- Our Rust spatial layer is already ahead on relations (frames, relation graph), placement (10 cm x 8 yaw
  solver, conflict sets), sight, isovist, exposure. Most Forge check names (overlap, floating, oob99, navConnect,
  accessFront, doorClear, dupeSpam, giantObject) were ported already (`crates/spatial/src/checks.rs:11-28`).
- Quality evidence is thin: the Scene IQ numbers are n=1 per task, one free model, one scene (Racing template).
  Treat them as symptoms of missing tools, not as a benchmark.
- Honest negative: Forge's 33-mode "view math" (octree, BVH, splat, point store) is tested but absent from
  `bridge/census-needs.json` and every eval task; do NOT port it (we have kdtree/index3 and physics.raycast).

## 2. Inventory (status = ours today; "row" = proposed here; "board" = already planned)

| Idea | Forge path:lines | Status in ours | Verdict |
|---|---|---|---|
| False-positive controls on a real saved scene | bridge/tests/SpatialRomeLiveTests.cs:52-306 | no real-scene gate (spatial_eval is synthetic) | SPF1 |
| Train-on-known-good: run checks over 12 Flax sample scenes, write priors | scripts/spatial-train.mjs, scripts/spatial-priors.mjs, bridge/tests/SpatialTrainTests.cs | none | SPF1 |
| Closed-form camera fit (target + view + fill + aspect) | bridge/CaptureFraming.cs:475-535 (CameraShot.Fit), tests bridge/tests/CameraShotTests.cs:36-60 | compose.rs scores a shot, never proposes (SPR10 board plan) | SPF2 |
| Relative placement words in_front/behind/left/right/against_wall/on_ground + gapCm | bridge/SceneOps.Geometry.cs:437-570 | place.rs wants lack them; Rel::Front etc. are query-only | SPF3 |
| Clearance probe: signed gap, unit push gradient, axis, confidence, kind | packs/scene/spatial/SceneClearanceOps.cs:150-200, 501-575 | `near` = centre distance only (scene_query.rs:790-820) | SPF4 |
| giantObject relative to rest-of-layout span, plates and sky exempt | bridge/Verbs.Spatial.cs:637-700, bridge/Verbs.SpatialSolid.cs:44-57 | fixed 50 m (`checks.rs:44,269-276`) | SPF5 |
| Free-travel sweep for one box (touch is clear, capped, already_blocked) | packs/scene/spatial/SpatialSweep.cs:49-125, packs/scene/SceneSolverOps.cs:202-255 | none | SPF6 |
| Scene IQ: 8 agent tasks, harness-measured truth, restore proof | eval/scene-iq.mjs, eval/scene-iq-checks.mjs:233-340, eval/scene-iq-sim.mjs, eval/scene_iq_*.json | board AD25 (same source) | SPF7 adds bars |
| Overlap intent triage, z-fight rank, unplayable-first, seated-joint | packs/scene/spatial/SceneSpatialTriage.cs:17-115,191-272, bridge/Verbs.SpatialSeated.cs | none | SPF8 |
| Verdict allow-list (reason required, rev, expiry) + family threshold learner | packs/scene/spatial/SceneSpatialRepair.cs:156-210, packs/scene/FamilyThresholds.cs:30-93 | N-12 plan only, no row | SPF9 |
| Brief: fold look-alikes and colliders, exclude strays from extent, problems in words, text map | bridge/SceneLook.cs, bridge/SceneLook.Layout.cs, tests bridge/tests/SceneLookTests.cs:374,513,539 | N-15 plan, SPR1 | SPF10 |
| On-screen coverage of a subject + cameraPose (looks away / flipped) | bridge/CaptureVisibility.cs:25-75, packs/scene/spatial/SpatialChecks.cs:358-390 | frame_check.rs has black/white only; SPR7 sight pair | SPF11 |
| Read-repeat guard, one-call includeSpatial, whenToCall/whenNotToCall text | bridge/BridgeRecovery.cs:123-175, bridge/verbs.schema.json (scene.brief) | guard missing; when_not = N-07 | SPF12 |
| 17-viewpoint scorer (fib sphere, 0.5 vis + 0.3 edge + 0.2 overlap) | packs/scene/SceneViewOps.cs:411-507 | compose.rs score_shot is richer | skip (no occlusion) |
| 33 view-math modes (AABB/octree/BVH/splat) | packs/scene/SceneViewMath.cs, SceneViewBvh.cs, SceneViewOctree.cs | have kdtree, index3, raycast | skip |
| Scene-SQL: query language + saved views + plan cache | packs/scene/SceneQuery.cs:1-120 | typed `scene.query` kinds | skip |
| Rooms/doors layout validate -> WFC seed | packs/scene/spatial/SpatialLayout.cs, packs/world/RoomCorridors.cs | level.build; SPR9 rooms | skip |
| NavEgress: doorway width from input, not grid (100 cm cell cannot show a 90 cm door) | packs/nav/NavEgress.cs | doorClear exists | note only |
| Sleeping statics, delta cursors in warm store | packs/scene/spatial/SpatialModel.cs | scene.watch digest | skip |
| Contrastive bench pairs (8 pairs, one relation flipped) | kits/smart-scene/bench-pairs.json, bench.validate.mjs | spatial_eval 100 gold tasks | skip (validator is a JS mirror of the oracle) |
| seeWalk, snapTurn, hierarchyDrift, colliderVisual checks | packs/scene/spatial/SpatialChecks.cs:337,458,531,575 | none | later (see 5) |

## 3. How Forge measured quality (numbers it recorded)

- Scene IQ (state/eval-reports/scene_iq_*.report.json; model muse-spark-1.3-contributor-free, seed 7, one run):
  find PASS 11 calls 27,761 tok; fix_floating PASS 6 calls 13,286; place_on_ground PASS 20 calls 45,476 (hit
  the 20-call cap); camera_follow FAIL 8 calls 21,688 (posErr 1556 cm vs 50, centre 0.629 vs 0.333);
  separate_overlap FAIL 5 calls 12,853 (moved 620 cm vs cap 200); top_down_shot FAIL 17 calls 46,768 (pitch
  35.7 vs 45, capture black); light_scene FAIL 20 calls 46,990; undo FAIL 6 calls 13,065. Limits: 20 calls,
  150,000 tokens (eval/scene_iq_place_on_ground.json). So 3/8 pass, 12.9k-47k tokens per task.
- Method: harness seeds a break, proves it landed (else ABSTAIN), agent works, harness reads truth from
  state.read, scores plain numbers, restores the scene and proves it (README eval:158-215). Harness calls are not
  charged to the agent. A lazy agent must FAIL and a reference agent must PASS (eval/scene-iq-sim.mjs:4,215-256).
- Real-scene controls (SpatialRomeLiveTests): header says every old finding on the real Rome scene was false.
  Each fix has a CONTROL: same box as trigger gives no sunken, same box as mesh gives it back (:74-110); boundary
  walls are no giants but a 50x crate in the same room is (:133-170); exact true set pinned (:290-306).
- Known-good corpus (C:/forge-data/flax-intake/spatial/*.report.json, 12 Flax sample scenes + 10 own): the
  430-actor GraphicsFeaturesTour gave 262 overlap findings, 640-actor MaterialsFeaturesTour 373. That is the
  false-positive pressure a gate must watch.
- Size/latency: Racing (35 actors) default brief reply asserted under 9,000 bytes (SceneLookTests.cs:539-560);
  40 look-alikes fold to 1 row (:374); tools.search slim budget 822 tokens, max 2,500 (BridgeCaps.cs:39-40);
  batch caps 50 calls, 500 ops, 8 parallel (:10-14); spatial 2 s TTL cache (Verbs.Spatial.cs:12); verb timeout
  4,000 ms. No measured per-check latency exists; HARD.md says "reads p95 300 ms" as a target only.

## 4. Top 12, ranked (value x cheapness x not already planned)

1. SPF1 corpus gate with controls. Why first: it caught at least 4 real bugs in Forge (trigger as floor, boundary wall
   as giant, engine primitives as dangling refs, sphere zone blocking a waypoint). Cheap, protects every other row.
2. SPF2 frame fit. Camera tasks failed on the free model; no door tool aims a camera (`capture` takes only
   width/height/max_size, door-reference.md `capture`). Pure function that SPR10 can build on.
3. SPF3 facing-relative wants. place_on_ground cost 20 calls and 45k tokens for "3 m in front of Player".
4. SPF4 clearance gradient. Gives the agent the exact move to clear a gap.
5. SPF5 giant vs layout. Our fixed 50 m flags any long arena wall; a 49 m prop in a small room passes.
6. SPF6 slide sweep. Answers "how far can I push this" without trial moves (separate_overlap moved 620 cm).
7. SPF7 Scene IQ bars. Adds collateral and budget bars to AD25.
8. SPF8 triage. Cuts false reds before an agent wastes repair rounds.
9. SPF9 allow-list + learner. Lets an agent say "intentional" and have it stick, auditable.
10. SPF10 brief hygiene. Fewer rows, honest extent.
11. SPF11 coverage + cameraPose. Refuse a capture that cannot show its subject.
12. SPF12 tool surface. Loop breaker plus one-call brief.

### SPF1 Known-good scene gate with negative controls
- What: ship scenes must give zero red findings and an exact expected-true list; each check must still fire on a
  one-edit mutant of the same scene (the control). Optional priors: p10/p50/p90 of sizes, spacing, light values.
- Source: SpatialRomeLiveTests.cs:52-306; spatial-train.mjs (manifest -> host test -> report.json per scene ->
  priors.json); SpatialTrainTests.cs. Shape: input = scene rows; output = findings by check and severity, plus
  `known` count (findings that rest on a real box).
- Eval: exact-set equality, not "no findings"; the control is what makes it honest.
- Port: loader = existing staged path (`crates/spatial/src/staged.rs`); I did not confirm it reads
  `content/arena/placement.json` directly.
- [ ] SPF1 spatial: known-good scene gate with controls - run the full check catalog over every shipped scene (`content/arena/placement.json` + `dressing.json`, `fixtures/games/*`) and pin zero red findings plus the exact list of true yellow ones; each check gets a one-edit mutant of the same scene that must make it fire (a non-physical box under a crate gives no sunken, flipped to physical it does; 52 props x100 give one unit finding). Source: Forge `SpatialRomeLiveTests.cs`, `spatial-train.mjs`. Diff: regular. Tier A. Size S. Owns `crates/spatial/tests/it/main.rs` and `known_good.rs` (new; one integration binary per crate). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --test it known_good` x2 green with identical finding lists; each mutant fires its check and the clean scene does not. F2P: no test runs the catalog over a shipped scene or proves a check still fires on a real one. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF2 Closed-form frame fit
- What: nearest eye on the view ray that keeps all 8 box corners inside `fill` of the frame on both axes.
- Source: CaptureFraming.cs:475-535 (`ViewAngles`, `EyeDir`, `Fit`), scene `pose` schema in
  packs/scene/pack.schema.json. In: min/max box (cm), view in threeQuarter 35/30, front 0/10, back 180/10, side
  90/10, topDown 0/89, thirdPerson behind facing 20 deg fill 0.4, orbit 45/25; fov 55..65; aspect; fill 0.1..1
  (default 0.85). Out: eye, look_at (box centre), distance (>= 50 cm), yaw, pitch, fov.
- Eval: CameraShotTests.cs:36-60: frame use within +-0.01 of fill for 4 views x several fovs; eye above subject.
- Port: new `crates/spatial/src/frame.rs`, integer cm, sin/cos/tan from a degree table (no libm, as
  `sense_text.rs`); SPR10's `scene.frame` verb calls it. Do not port the 17-view scorer (no occlusion).
- [ ] SPF2 spatial: closed-form frame fit - `frame_fit(box_cm, view, fov_deg, aspect_pct, fill_pct, yaw_deg, pitch_deg)` returns eye, look_at, distance, yaw, pitch for seven named views (three_quarter, front, back, side, top_down, third_person, orbit) with integer degree tables and no libm; SPR10's verb will call it. Source: Forge `CaptureFraming.cs` CameraShot. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/frame.rs` (new), one `lib.rs` export line. Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib frame_fit` x2: for 7 views x fov 55/60/65 x aspect 16:9 and 4:3 all 8 corners project inside the frame and the larger axis use is within 1 percent of fill; look_at is the box centre; top_down eye is above and pitch 89; third_person eye sits behind the target yaw; same bytes twice. F2P: nothing proposes a camera pose (`compose.rs` only scores one). P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF3 Facing-relative placement wants
- What: place a box by word relative to an anchor's facing: in_front, behind, left, right, against_wall
  (back face gap_cm off the wall on the mover's side), on_ground (rest on surface under it), plus on_top.
- Source: SceneOps.Geometry.cs:437-570 (`PlaceBox`): distance = half-extents projected on the direction + gap;
  left = (-fz, fx); bottoms level; anchor facing = transform.direction else +Z. Refuses unknown relation,
  missing anchor, anchor == mover, gap outside 0..100000 cm.
- Eval: Scene IQ place_on_ground (crate bottom within 2 cm of floor top, 3 m ahead within 30 cm, lateral 30 cm,
  size 20 percent, no overlap); passed in 20/20 calls, 45,476 tokens without this tool.
- Port: `place.rs` Want gains these words in the Subject frame of `relation.rs`; round trip = the placed pose
  must satisfy `scene.query kind=relation` for the same word. Left/right sign must follow SPR2's pinned gold.
- [ ] SPF3 spatial: facing-relative wants - `scene.place` accepts `in_front`, `behind`, `left`, `right` with a `[lo,hi]` edge gap in cm against an object's own facing, plus `against_wall` and `on_ground`, solved on the existing 10 cm x 8 yaw grid and checked by the same relation predicate the query uses. Source: Forge `SceneOps.Geometry.cs` PlaceBox. Diff: good. Tier A. Size M. Owns `crates/spatial/src/place.rs`, `crates/endpoint/src/scene_tools.rs` (want grammar). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib place_facing` x2: for 8 anchor yaw octants x 4 words the solved box sits in the named half-plane with edge gap inside the range and `holds(mover, word, anchor)` is true; an impossible gap returns a conflict set; endpoint test places a crate `in_front` of a fixture player in one call. F2P: wants cover supports/on_top_of/inside/next_to/facing only, so "in front of" needs a multi-call guess. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF4 Clearance probe with push gradient
- What: surface (not centre) gap between boxes, signed (penetration depth when overlapping on all axes), unit
  gradient to open the gap, axis, confidence, kind.
- Source: SceneClearanceOps.cs:501-575 (`SurfaceGapDetail`): overlap -> gap = -shallowest squeeze, gradient on
  that axis signed by centre order; separated -> Euclid of per-axis gaps, gradient = sign(centre delta) * d/gap;
  kind exact (touch/overlap) / upper_bound (separated) / narrow (< 90 cm flow bar); confidence 1.0 / 0.8 / 0.5.
  In: actorId, needCm 1..10000 default 90. Out text: `min Ncm nearest X need 90cm holds|fails gradient [..]`.
- Eval: SceneClearanceOpsTests.cs (narrow under the bar, exact at zero, upper bound above) - not re-read.
- [ ] SPF4 spatial: clearance probe with push gradient - `scene.query kind=clearance anchor=<id> need_cm=<n>` returns the nearest surface gap in cm (negative when boxes overlap), the neighbour id, the unit push direction that opens it, the axis and a kind of exact, upper_bound or narrow, with a holds or fails verdict against need_cm (default 90). Source: Forge `SceneClearanceOps.cs` SurfaceGapDetail. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/index.rs` (surface_gap), `crates/endpoint/src/scene_query.rs` (kind). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib surface_gap` and `-p engine-endpoint --features mcp --lib query_clearance` x2: apart 30 cm on x gives gap 30 push +x; touching gives 0 exact; 12 cm overlap on the shallowest axis gives -12 with push toward the anchor's side; a 30/40 cm corner gap gives 50; moving the anchor by push x (need - gap) makes the same query hold. F2P: `near` returns centre distance only, so a long wall reads far while touching. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF5 giantObject relative to the layout
- What: a prop is giant when it is bigger than the layout it lives in. The span for a candidate that exceeds the
  overall extent is that of the REST of the layout, so a wall that IS the edge is never giant; ground plates
  (height <= 50 cm, sides <= 50 m) and sky/skydome are exempt; fewer than 2 bounded solids -> unavailable.
- Source: Verbs.Spatial.cs:637-700, Verbs.SpatialSolid.cs:44-57 (`SpatialRestSpan`). Test pair: four 2060 cm
  boundary walls are no giants, a 50x crate in the same room is (SpatialRomeLiveTests.cs:133-170).
- Ours: `crates/spatial/src/checks.rs:269-276` flags max dimension > 5000 cm only, so a 60 m wall in a 100 m
  arena is red and a 49 m prop in a 6 m room is silent. `scale.rs` (AD11) covers the typical-object case.
- [ ] SPF5 spatial: giantObject relative to the layout - replace the fixed 50 m test with size over the span of the other solid boxes (ground plates and non-physical boxes exempt, fewer than 2 solids silent), keeping the check name and the frozen catalog length 18. Source: Forge `Verbs.Spatial.cs` SpatialRunGiantObject, `SpatialRomeLiveTests.cs`. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/checks.rs` (giantObject block). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib giant_layout` x2: four 2060 cm boundary walls around two props give none; a 50x crate in a 4 m room gives one; a 60 m wall in a 100 m arena gives none; digests of fixtures that held giantObject re-pin with a named receipt. F2P: any element over 50 m is red and an oversized prop under 50 m is never red. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF6 Slide sweep (free travel)
- What: how far a box can move along one direction before strictly overlapping another; touch counts as clear;
  nearest hit wins; mover skipped by id; none inside max_range -> free = max_range, capped true; overlap at
  distance 0 -> 0, already_blocked true.
- Source: SpatialSweep.cs:49-125 (slab test, still-axis needs strict overlap), SceneSolverOps.cs:202-255.
  In: mover box, obstacles <= 256 with id/name, axis word or vector, maxRange default 1000 cm. Read-only.
- Port: integer cm, 6 axis words plus 8 octant directions only, so every slab time is exact.
- [ ] SPF6 spatial: slide sweep - `scene.query kind=slide anchor=<id> dir=<+x|-x|+z|-z|up|down|octant> max_range_cm=<n>` returns free_cm, the first blocker id, capped and already_blocked flags, with touching counted as clear and the anchor skipped. Source: Forge `SpatialSweep.cs`, `SceneSolverOps.cs` MeasureSafeMovement. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/slide.rs` (new), `crates/endpoint/src/scene_query.rs` (kind). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib slide` x2: result equals a brute-force 1 cm march on 200 seeded box sets; a face touch is clear; an overlapping start gives 0 and already_blocked; no obstacle gives max_range and capped; shuffling obstacle order gives identical bytes. F2P: the only motion check is the velocity `sweep`; an agent must trial-move to learn the room. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF7 Scene IQ collateral and budget bars (extends AD25)
- What: AD25 already ports the harness. Add the bars Forge learned matter: collateral movement
  (`othersMovedMaxCm <= 1`, `movedMaxCm < 200`), a pair penetration bar (<= 1 cm), `newOverlapPairs == 0`, an
  undo task that requires the target to have been seen >= 150 cm away, and per-task limits (20 calls, 150k tokens)
  charged to the agent only.
- Source: scene-iq-checks.mjs:233-340, eval/scene_iq_*.json `iq.limits`, README eval:158-215.
- Note: two Forge failures (camera_follow, top_down_shot) are tool gaps (SPF2/SPR10), not model failures.
- [ ] SPF7 playtest: Scene IQ collateral and budget bars - add to AD25's eight tasks a collateral bar (no unrelated node moves more than 1 cm on fix_floating, nothing moves more than 200 cm on separate_overlap, zero new overlap pairs), an undo bar (target seen at least 150 cm away, ends within 1 cm), and per-task limits of 20 agent calls and 150000 tokens that exclude harness calls. Source: Forge `eval/scene-iq-checks.mjs`, `scene_iq_*.json`. Diff: regular. Tier A. Size S. Owns `fixtures/scene-iq/` bars and the `crates/playtest/src/spatial_eval.rs` scene-IQ section AD25 creates. Accept: `node tools/gh-check.mjs cargo test -p engine-playtest --lib scene_iq_bars` x2: a fixture fix that also nudges an unrelated node 2 cm FAILs fix_floating; a do-nothing agent FAILs all eight; the reference agent passes all eight; the report prints calls and tokens per task. F2P: no collateral or budget bar exists. P2P: golden d57b5c8ae3096614 unchanged. needs: AD25.

### SPF8 Overlap triage by role, z-fight rank, unplayable first
- What: classify an overlap before reporting it: collider-vs-visual is intentional, bone-vs-bone false_positive,
  effect-marker pair deferred, else error; z-fight = near-coplanar pair within epsilon, ranked by area/gap with an
  optional camera-frustum filter; findings of error severity or kinds burial/sunken/overlap sort first
  ("unplayable"); a shallow part resting in its supporter is a seated joint (info, no fix).
- Source: SceneSpatialTriage.cs:17-115 (z-fight), :191-272 (intent, unplayable), Verbs.SpatialSeated.cs.
  Forge matches NAMES ("Collider", bone names, "slice"); that is Rome-specific. Ours has component kinds.
- [ ] SPF8 spatial: overlap triage by component role - tag each overlap finding intentional (collider with its mesh), deferred (effect markers) or error by `ComponentKind`, never by name; add a z_fight rank (coplanar pairs within 1 cm on one axis, score area over gap, optional frustum filter; Forge uses 0.2 cm but our boxes are integer cm) and sort unplayable kinds (overlap, sunken, burial) first. Source: Forge `SceneSpatialTriage.cs`, `Verbs.SpatialSeated.cs`. Diff: good. Tier A. Size M. Owns `crates/spatial/src/triage.rs` (new), `crates/spatial/src/checks.rs` (tag hunk only). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib triage` x2: a collider co-located with its mesh is intentional and not red; two meshes with the same footprint 0 cm apart give one z_fight ranked above a 1 cm pair, and a 3 cm pair gives none; unplayable findings precede others; a part resting 3 cm in its supporter reads info; input order does not change bytes. F2P: every overlap is red, a collider on its mesh included. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF9 Verdict allow-list and threshold learner (shape for N-12)
- What: `allow[]` rows {finding or pair key, verdict in fixed|intentional|false_positive|deferred, reason 1..200
  REQUIRED, rev, expiry}, at most 256. A match keeps the finding in the output flagged `suppressed:true` with the
  reason (never dropped); `fixed` never suppresses; suppress iff expiry == 0 or rev == 0 or rev <= expiry. Learner:
  3 intentional/false_positive verdicts in one check x name-family cell relax it one step, cap 3, errors never
  relax; every cell carries count + last verdict as the audit trail.
- Source: SceneSpatialRepair.cs:156-210, SceneClearanceOps.cs:53-70,105-110, FamilyThresholds.cs:30-93.
- [ ] SPF9 spatial: verdict allow-list - `scene.spatial` takes `allow` rows (finding key, closed verdict, reason required 1..200 chars, rev, expiry), keeps matched findings visible as `suppressed` with the reason and excludes them from the red count; a family learner relaxes one check x family cell after 3 intentional or false_positive verdicts up to level 3 and never relaxes red errors. Source: Forge `SceneSpatialRepair.cs`, `FamilyThresholds.cs`. Diff: good. Tier A. Size M. Owns `crates/spatial/src/verdict.rs` (new), `crates/endpoint/src/scene_tools.rs` (arg). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib verdict` and `-p engine-endpoint --features mcp --lib spatial_allow` x2: an expired entry stops suppressing; a missing reason refuses naming the field; a fixed verdict never suppresses; 3 verdicts relax level 1 and 12 cap at 3; same rows in any order give one table. F2P: no way to mark a finding intentional, so a deliberate overlap stays red forever. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF10 Brief hygiene
- What: fold look-alikes (same kind, type, name stem, parent, size) into one counted row with area; fold a
  collider that wraps its mesh parent; exclude strays and backdrops from the play-area extent and report
  `excluded[{name,why}]` plus `extentInflation`; top 5 problems in plain words; top-down text map with legend;
  `includeSpatial` appends finding counts, focus extent, nearest gap in the same reply.
- Source: SceneLook.cs, SceneLook.Layout.cs, tests SceneLookTests.cs:227,374,464,513,539. Numbers: 40 look-alikes
  -> 1 row; Racing 35 actors < 9,000 bytes; strays flagged with inflation > 10x.
- Overlap: N-15 and SPR1 touch the same file; land after SPR1.
- [ ] SPF10 spatial: brief hygiene - fold look-alike objects (same mesh, parent and size) into one counted row, drop colliders that wrap their own mesh into that row, exclude strays and backdrops from the stated extent while listing them with a reason and an inflation ratio, and print the top five problems in plain words. Source: Forge `SceneLook.cs`, `SceneLook.Layout.cs`. Diff: regular. Tier A. Size M. Owns `crates/spatial/src/brief.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib brief_fold` x2: 40 identical crates give one row with count 40; a 500 m stray crate is excluded with why=stray and the inflation ratio; the 115-object fixture stays inside the byte budget; same bytes in any input order. F2P: every object gets its own row and one stray stretches the extent. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF11 Coverage of a subject and camera sanity
- What: share of the frame a subject covers from a pose (Forge: 128x128 pixel-centre rays against a bounds
  SPHERE, no occlusion); refuse a capture when < 0.5 percent with a "Next call: ..." line. cameraPose check: dot of
  forward and to-target < 0 looks away (fail), up.y < 0 flipped (fail), dot < 0.2 or |up.y| < 0.3 warns.
  Framing gate: camera > 300 m away, below y -500, or fewer than max(3, 10 percent) renderables in view fail.
- Source: CaptureVisibility.cs:25-75, CaptureFraming.cs:17-23, SpatialChecks.cs:358-390.
- Port: use our box + occluder ray grid (`sight.rs`, 32x32 = 1,024 rays is enough), not the sphere.
- [ ] SPF11 spatial: subject coverage and camera sanity - `scene.query kind=coverage anchor=<id> eye_m yaw pitch fov` returns the share of a 32x32 ray grid whose first hit is the anchor (permille), a looks_away or flipped verdict, and under 5 permille a refusal-style next-call line. Source: Forge `CaptureVisibility.cs`, `SpatialChecks.cs` CameraPose. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/sight.rs` (grid hunk), `crates/endpoint/src/scene_query.rs` (kind). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib coverage` x2: a camera inside the subject reads 1000; behind it reads 0 and looks_away; a pillar covering half the view halves the share versus the sphere-only number; up.y below 0 reads flipped; identical bytes twice. F2P: no way to learn that a subject is off screen or hidden before capture. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

### SPF12 Read-repeat guard and one-call brief
- What: the 5th identical safe read from one principal in 30 s is refused with code `safe_read_repeat` and a next
  step; reads 2-4 stamp `repeatCount`; poll/status/wait names exempt; 1,000 keys max; anonymous bypass. Idea came
  from the old Flax MCP filter (named in the header, repo not read here).
- Source: BridgeRecovery.cs:123-175; scene.brief `includeSpatial` note in bridge/verbs.schema.json.
- [ ] SPF12 endpoint: read-repeat guard - the fifth identical read-only call from one session within 30 s is refused with a typed `read_repeat` carrying the next verb to try, calls two to four carry a `repeat_count`, status and wait verbs are exempt, the key table holds at most 1000 entries; `scene.brief` gains `include_spatial` returning finding counts and the focus gap in the same reply. Source: Forge `BridgeRecovery.cs` SafeReadRepeat, `verbs.schema.json` scene.brief. Diff: regular. Tier A. Size S. Owns `crates/endpoint/src/guards.rs` (read guard), `crates/endpoint/src/scene_tools.rs` (flag). Accept: `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib read_repeat` x2 with an injected clock: the fifth identical `scene.query` is refused, four are stamped, one changed argument resets the count, a call after 30 s passes, a `play.run` poll never trips; `include_spatial` counts equal a separate `scene.spatial` call. F2P: an agent can repeat one read forever with no signal. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

## 5. Left out on purpose, and what I did not read

- Later candidates: seeWalk (hero-path sight links mean >= 4 at eye 160 cm, exit seen within 10 m and 2 turns),
  snapTurn (yaw within 5 deg of 0/90/180/270 or wall-parallel), colliderVisual (ratio past 2x), hierarchyDrift
  (SpatialChecks.cs:337-575). Ours has isovist/exposure which may cover seeWalk; not checked.
- Not read: SceneViewMath/Bvh/Octree bodies, SceneDiffOps, SceneBatch*, SceneFused beyond SpawnFinder,
  nav pack bodies beyond headers (NavQueries, NavFlow, NavEgress), world/Wfc*, Rome crowd/placement tests
  (RomeCrowd*, game/slice/rome), SceneClearanceOpsTests, SpatialSweepTests, factory/, gateway/, plugins, lab/.
- Not verified: no Forge test was run; all eval numbers are copied from checked-in reports in
  state/eval-reports (one run, one free model, Racing template only, run date not recorded in the files read).
- `scripts/fixtures/spatial/rome-live.Main.scene` and `C:/forge-data/flax-intake/spatial/priors.json` were
  listed, not parsed (priors.json head only). Flax sample scenes are EULA ideas-only; take the method, never data.
- "Old Flax MCP repo" is not in C:\forge; only its name appears (BridgeRecovery.cs:125, `crates/endpoint/src/guards.rs`
  header cites flax-mcp ToolGuards.cs). I did not find or open it; C:\flax exists (spatial-train.mjs defaults to C:/flax/FlaxSamples) but I did not open it.
- Our-side facts used here (checks.rs giant block, place.rs Want, scene_query.rs near, capture args) were read
  directly; whether `staged.rs` loads `placement.json` and whether endpoint files carry uncommitted edits from
  other sessions (git status showed many) was not checked. SPF rows touching `scene_query.rs` and `scene_tools.rs`
  will conflict with SPR rows on the same files; sequence them in one lane.
