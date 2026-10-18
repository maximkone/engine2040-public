# Spatial audit 03: Flax Forge, old Flax MCP and the asset worker (2026-10-18)

Part 3 of the spatial report for Maxim. Research on the owner's own private repos, cloned shallow to the
session scratchpad and read as plain text. Nothing was built, run or installed. Policy: steal freely, no
license checks. Compared against `docs/book/door-reference.md:428-625`, `crates/spatial/src/lib.rs`, the E03
audit in `docs/plan/forge-adoption.md` and `01-ours.md` (SPR1..SPR10). Paths below are inside the repo named.

## 1. Access and bottom line

- `maximkone/flax-mcp` (the Forge): readable. 4040 .cs files, depth-1 clone at HEAD a26f960 (history not read).
- `maximkone/flaxmcp-plugin` (old repo): readable. First checkout died on Windows long paths under
  `GameProjectFlax/`; retried with `core.longpaths`, the C# tool sources are on disk. Submodules (FlaxDocs,
  FlaxSource) not fetched, LFS not smudged.
- `maximkone/flax-asset-worker`: readable. No spatial, placement, bounds or camera tool (intake, providers,
  license metadata only). Nothing to steal here.
- Much is already ours: scale detector (`scale.rs`), 3D yaw overlap (AD18), real extents (AD7), suppression
  ledger (N-12), support chain (N-13), brief focus (N-15), per-finding repair (N-16), SPR1..SPR10. This list is
  only what a grep of `docs/SPRINT*.md`, `docs/plan`, `docs/research` found on NO row: seated, coplanar,
  z-fight, view probe, ground trace, safe movement, outside-of, spatial delta, workflowGuide, worstActors.
- Strongest unlanded: a per-actor view summary with an `occluded` list, calibration rules that kill false
  red findings, feedback on the write itself, and a next-step guide for weak models.
- Caveat: the Forge retired its `scene/measure_*` and `assert_contact|containment` tools from the live list
  (skill `part-02:165-177`; absent from `.agents/tool-fingerprint.json`, 1190 tools, undated). Ranked low.
- No token measurement exists in any of the three repos. Sizes below are schema caps, not wire captures.

## 2. Inventory (Forge paths under `src/FlaxMcp.Kernel/Core/Packs/` unless noted)

| Tool (live?) | File:lines | Args (default; cap) | Output | Ours |
|---|---|---|---|---|
| scene/spatial_report (live) | Scene/Scene.Spatial.cs:37-164 | maxFindings 20 (code 1000, text 200), overlapRatio .02, checks all/overlap/support/visibility, ignorePairs, highlight, hideSuppressed | verdict, finding_count/returned_count/truncated, findings[10 kinds], summary.worstActors[], thresholds, suppressed_findings[] | 18 checks; no seated, coplanar, stray, rollup |
| scene/brief (live) | Scene.Spatial.cs:528-617 | focus, radius (3x own size), maxGroups 20, maxProblems 3, objective | summary, counts, extent minus strays, strayActors, groups, problems, freshness.fingerprint, workflowGuide | groups yes; no focus, stray extent, guide |
| scene/get_next_step (live) | Scene.Spatial.cs:622-960 | objective | workflowGuide{stage(12), nextCall, verify, stopConditions[], doNot[]} | none |
| scene/find_placement (live) | Scene.Spatial.cs:1111-1238; Core/Utils/PlacementSolver.cs:91-193 | actor, support, outsideOf, clearance, preferX/Z, gridSteps 24 (256) | found, reason(4), world+local position, apply string | scene.place; no outsideOf |
| scene/layout_intent, get_ (live) | Scene.Spatial.cs:1242-1346; Live/Services/SpatialAllowlist.cs | action get/allow/remove, pairs, floating, notes | allowOverlap[], expectFloating[], path | N-12 |
| scene/clearance (live) | SceneClearanceTool.cs:44-69; Live/Services/SceneSpatialService.Pairs.cs:546-604 | actorId, actorIdB, worldPos, agentRadius, tolerance | signed minDistance, clearance, gradient, needsDecision | doorClear only |
| scene/nav_clearance, egress_check (live) | Scene/NavMesh.Tools.cs:165, 203 | position, agentRadius, exits[], requiredCorridorWidth | passes, footprintCost 0/253/254; allReachable, worstGeodesic, bottlenecks[] | none |
| scene/ground_trace (live) | Scene/Scene.Seat.cs:483 | x, z, fromY, maxRayLen | hit{point,normal}, distance, actor | none (SP69 is camera rays) |
| scene/measure_seating (source only) | Scene.Seat.cs:351 | points 4 (64), maxRayLen, alignToNormal | seatHeight, overhangRatio, unevenness, lift, blockers, applyPart | none |
| scene/measure_safe_movement (source only) | Scene.Movement.cs:196 | actor, axis(9 values) or direction, maxRange | results[{freeDistance, blocker, alreadyBlocked, capped}] | none |
| scene/measure_distance, gap, overlap, alignment; assert_contact, assert_containment (source only) | Scene.Measure.cs:336,410,483,570; Scene.Assert.cs:35,131 | from, to, tolerance, axes, reference CENTER/MIN/MAX, maxGap, minClearance | surfaceGap, axisGap, relation, penetrationRatio, deltas, passed, shortfall | none |
| viewport/read_view_probe (live) | plugins/tooling/.../ViewportTools.cs:1231-1350; Core/Utils/ViewProbe.cs:101-307 | actorId, view(6), grid 24 (64), fieldOfView 10-120, maxVisible 25 | visible[{name,coverage,near,far}], occluded[], depth, backgroundFraction | SP69 RLE text; no names list |
| render/visibility_report (live) | Render/RenderGeometricDiagnosticsTools.cs:27 | actorId, cameraId | verdict: visible, disabled, zero_scale, no_mesh, no_material, degenerate_bounds, frustum_culled, behind_camera, size_culled, occluded_by:name | none |
| render/zfight_candidates (live) | same file:8 | epsilonCm .5 (.01-10), cameraId, maxPairs 20 (100) | pairs[{axis, plane, gapCm, overlapArea, score, inFrustum, fix}] | none |
| scene/assert, health_summary (live) | SceneAssertionRunner.cs:17; Scene/SceneHealthSummaryTools.cs:12 | predicates(6), epsilonCm .5, detail; maxBlockers 10 | byPredicate; blockers[{nextTool,priority}], counts, <8KB, <1s | test.run partial |
| scene/repair_plan (live) | Scene/Scene.RepairPlan.cs:40-92 | maxFindings, kinds | steps[{finding, calls[]}], verify | repair.rs (N-16) |
| scene/raycast, raycast_all, overlap_query (live) | Scene/SceneRaycastTools.cs:115,171,227 | origin, direction, maxDistance 10000, layerMask, pageSize 50 (200), shape | point, normal, actorId | physics.* needs cooked hex |
| scene/inspect_visual, understand (live) | Scene/Scene.Understand.cs:142-190, 445-520 | question, includeMeasured, evidenceLighting | verdict ok only if VLM answered; void/black frame abstains | look.judge partial |
| SpatialFeedback (internal) | Scene/SpatialFeedback.cs:1-114 | after actor/spawn, set_transform | warnings[<=5], spatialDelta{findings} on the write | none |
| SpatialMath/Placement/ViewProbe (no engine) | src/FlaxMcp.Core/Core/Utils/*.cs | see section 4 | pure functions | partial |
| Old repo: get_actor_bounds, raycast_from_camera | flaxmcp-plugin `.../Tools/Scene/Core/SceneValidationBridgeTools.cs:106,157` | name_or_id; screen_x/y | bounds+source; ray hit | n/a |
| Old repo: get_terrain_height | `.../Scene/Terrain/TerrainTools.Query.cs:171` | terrain_name, x, z | hit, y, normal (down-ray from +100000) | none |
| Old repo: physics_ext_ops line/sphere/box/capsule casts, overlaps | `.../Scene/Physics/PhysicsQueryTools*.cs` | start,end / center,radius,direction, return_all | hit or all hits | shapecast partial |
| Old repo: scatter_place EQS, populate_from_layout | `.../Scene/Layout/ScatterPlacementTool.cs:22-59,262-330`; `SceneLayoutTools.PlacementUpgrades.cs:22-37` | pattern, min_distance, eqs ring/grid, eqs_path_cost_budget, snap_to_terrain | points scored by distance and nav cost | `ai/eqs.rs`, sample.rs |

The old README lists `terrain_raycast` and `query_actors_in_radius`; neither is implemented (grep of the tree).
`nav/query` in the Forge is C# code navigation, not spatial; ignore it.

## 3. Prompt and skill text worth copying (no board row; feeds AD26 refusals and AD27 playbooks)

- "Position carries no extent; never compute layout in your head" and "do not raise overlapRatio until the
  finding disappears" (flax-mcp `.opencode/skills/flax-scene-workflow/references/part-02:59-88`).
- Findings carry their numbers and a machine fix, `fix` is null when a decision is needed
  (Scene.Spatial.cs:58-63); fix by actor, not by finding (`:147-154`).
- Detect the disease, not the symptoms: one unit mismatch beat 541 pair findings (SceneSpatialService.Scale.cs:1-30).
- Seating exactly on contact makes a coplanar z-fight; run the verify step (Scene.RepairPlan.cs:81-90).
- State first, shot last; stale frame, echoed input, 0 findings meaning hidden (forge/gold/state-doctrine.md:7-35).
- Connection map before spawn, chunks of 3-5 then verify (part-01:87-154); hero camera eye 165-170 cm (part-05).
- Abstain, never pass: a visual verdict is ok only when the model answered (Scene.Understand.cs:142-190).
- Calibration scars: sameMesh is not duplicate; coincident means centres within 2% of the SMALLER actor and
  20% volume (Pairs.cs:175-215); scale check needs 16 props; 402 of the first 730 findings were zero-extent
  engine actors such as cameras and lights (Support.cs:785-798).

Evals: none for spatial in either repo. `tests/FlaxMcp.Tests/Packs/SceneAssertTests.cs` has two fixtures (3
inactive crates; Podium/Hero_Road_Basalt, 1,080,000 cm2 at gap 0). Comments cite `SupportTransitiveTests.cs`,
which is not in HEAD (`git ls-files`). Old repo `FlaxMCP.Tests/Baselines/mcp_eval_scenarios.read_only.json` is
regex expected/forbidden patterns, not spatial.

## 4. Top 12 ideas, ranked (value to an AI building games through MCP, then port cost)

Our side: `checks.rs:231-266` floating/support (y=0 plane, one hop, no CanSupport); `sense_text.rs:120-160`
view; `sight.rs:320-377` `ray_first_hit`; `checks.rs:40` `OPT_IN_CHECKS` (the way to add checks without moving the
18-check hash). New checks below are opt-in for that reason.

**1. Named view glance.** Grid^2 rays; per actor coverage, nearest/farthest, background share, and `occluded` =
reached by a ray but never first. Lighting-independent. Source: ViewProbe.cs:101-116 slab, 160-199 ray, 220-307
summary; schema ViewportTools.cs:1231. Flax cost: 576 rays default, 4096 max, 25 rows. Port: `glance()` in
`sense_text.rs` over `pixel_dir` (:94) plus a per-box slab "reached" flag, integer permille; kind `glance`.
- [ ] SPX1 spatial: named view glance - `scene.query kind=glance` with eye, yaw, fov, grid 1..64 (default 24)
  returns `visible[{id,coverage_pm,near_cm,far_cm}]` capped 25, `occluded[]`, `background_pm`, depth min/max/
  mean; the text reports tokens. Diff: good. Tier A. Size S. Owns `crates/spatial/src/sense_text.rs`,
  `crates/endpoint/src/scene_query.rs` (kind arm). Accept: `cargo test -p engine-spatial --lib glance` x2: 50
  seeded cameras match a brute-force first-hit count 50/50; a pillar hides a crate so it is in `occluded` and
  not in `visible`; coverage plus background is 1000; <= 0.2 ms per ray. F2P: no per-actor view summary.
  P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**2. Seated joint and support rules.** (a) A top inset shallowly into its frame is a joint, judged against the
LOWER height (5 of 90 = 5.5%, not 50%): yellow, ranked last. (b) `CanSupport`: reject a surface that spans the
prop's whole height, and a vertical shell covering over half of it (wall pipes were "sunk 350"). (c) Kahn sort
on direct supports so a stack on a floating base names the base; cycles say "needs decision". Source:
SpatialMath.cs:383-407, 426-446, 455-460, 844-969; thresholds SceneSpatialService.cs:99-127 (float 25% of own
height, sink 15%, seat 20%, footprint 50%). Port: integer ports in a new `support.rs`; opt-in checks.
- [ ] SPX2 spatial: seated joint and support chain - opt-in checks `seatedJoint` and `supportChain`: seat when
  upper base is above lower base, inset <= 20% of LOWER height and >= 50% of upper footprint rests on it
  (yellow, never red); `CanSupport` skips spanning and wall-mounted candidates; floating/sunken use ratios
  (25%/15% of own height) and a bottom-up chain. Diff: good. Tier A. Size M. Owns
  `crates/spatial/src/support.rs` (new), `crates/spatial/src/checks.rs` (opt-in arms), `lib.rs`. Accept:
  `cargo test -p engine-spatial --lib support_chain` x2: desk top inset 5 into 90 reads seated, two crates 50%
  deep read red, a wall pipe reads no sunken, a crate on a raised table names the table, 20 permutations give
  one digest, `SPATIAL_CHECKS` stays 18. F2P: `floating` is y=0 only, `support` is one hop. P2P: golden
  d57b5c8ae3096614 unchanged. needs: none. (Covers the chain half of N-13.)

**3. Spatial delta on the write.** After a spawn or move, test only the touched actors and return up to 5
findings in the same reply: "buried a crate on the call that buried it". Best effort, never fails the write,
env switch for bulk loads. Source: SpatialFeedback.cs:1-114. Port: `tx.apply` and `scene.place` outputs gain
`spatial_delta` from the staged check delta (`staged.rs`), touched ids only.
- [ ] SPX3 endpoint: spatial delta on writes - `tx.apply` and `scene.place` return `spatial_delta{ids,
  new_findings<=5,resolved}` for touched ids only, computed by the staged check; `delta=false` skips it. Diff:
  good. Tier A. Size S. Owns `crates/endpoint/src/mcp.rs` (apply output hunk), `crates/endpoint/src/mcp_dto.rs`,
  `crates/spatial/src/staged.rs` (id filter). Accept: `cargo test -p engine-endpoint --features mcp --lib
  spatial_delta` x2: a crate moved into a wall names both ids and the axis fix, a pre-existing unrelated overlap
  is absent, a clean move returns an empty delta, a 200-object scene adds < 5 ms. F2P: `TxApplyOutput` has no
  findings (`mcp_dto.rs:307`). P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**4. Next-step guide for weak models.** One stage, ONE next call, a verify condition, stop conditions and
`do_not[]`; stages undo_inspection, save_preflight, structural, spatial_plan, visual_verify, playtest,
ready. Source: Scene.Spatial.cs:622-960, brief half `:747-960`. Port: pure fn from brief/report counts.
- [ ] SPX4 endpoint: `scene.next` - observe verb returning `{stage,next_call{verb,args},verify,
  stop_conditions[],do_not[]}` from digest, finding counts and revision; do_not includes "do not raise a
  threshold to hide a finding". Diff: good. Tier A. Size M. Owns `crates/endpoint/src/scene_next.rs` (new),
  `crates/endpoint/src/packs/scene.rs`. Accept: `cargo test -p engine-endpoint --features mcp --lib scene_next`
  x2: a 12-state table gives exactly one next_call each, same bytes twice, reply <= 600 bytes, door-reference
  lists it. F2P: no verb names the next call. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**5. Travel bound and ground trace.** How far can this slide before touching something (blocker named,
`already_blocked`, `capped`), and what surface is under (x,z). Source: Scene.Movement.cs:196-268, Scene.Seat.cs:
483-517 (+ seating grid `:351`: overhang, unevenness, lift). Port: expand the mover box by half extents and run
the slab test already in `sight.rs`; ground = highest top under the point. Integer cm.
- [ ] SPX5 spatial: travel bound and ground trace - kinds `travel` (anchor, axis or dir, max_range_cm ->
  free_cm, blocker, already_blocked, capped) and `ground` (x_cm, z_cm, from_y_cm -> y_cm, id). Diff: good.
  Tier A. Size S. Owns `crates/spatial/src/sight.rs` (travel, ground fns), `crates/endpoint/src/scene_query.rs`
  (arms). Accept: `cargo test -p engine-spatial --lib travel_ground` x2: 50 seeded cases match a 1 cm march 50/50,
  touching is not a hit, children of the mover are ignored, <= 0.2 ms. F2P: no travel or ground query.
  P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**6. Why is it hidden.** First failing reason wins, in fixed order. Source: RenderGeometricDiagnosticsTools.cs:27
(verdict list in the table; also in the Forge core-12). Port: CPU verdicts from Mesh/Material, scale, bounds, view cone, and
`occluded_by:<id>` from `ray_first_hit` (also feeds SPR7).
- [ ] SPX6 spatial: why hidden - `scene.query kind=why_hidden anchor eye_m yaw` -> `verdict` one of visible,
  not_found, zero_scale, no_mesh, no_material, degenerate_bounds, out_of_view, behind_eye, too_far,
  occluded_by:<id>, plus the evidence number. Diff: good. Tier A. Size S. Owns
  `crates/spatial/src/why_hidden.rs` (new), `crates/endpoint/src/scene_query.rs` (arm). Accept: `cargo test -p
  engine-spatial --lib why_hidden` x2: 10 fixtures, one per verdict, 10/10; occluded_by names the pillar; fixed
  check order. F2P: `visible` says only visible or blocked. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**7. Z-fight pairs.** Faces on one axis within epsilon (0.5 cm default, 0.01-10), positive overlap area, ranked
area/gap, optional camera filter. In the owner's Forge core-12 verb list (`forge/core/VERBS.md`). Source: file:8;
fixture SceneAssertTests.cs (Podium). Port: opt-in check `zfight`, integer cm, `epsilon_cm` arg.
- [ ] SPX7 spatial: z-fight pairs - opt-in check `zfight` (and `epsilon_cm` on `scene.spatial`) lists box pairs
  with coplanar faces within epsilon and shared area, ranked area/gap, with the axis and the fix delta. Diff:
  regular. Tier A. Size S. Owns `crates/spatial/src/checks.rs` (opt-in arm), `crates/spatial/src/lib.rs`,
  `crates/endpoint/src/scene_tools/spatial_checks.rs`. Accept: `cargo test -p engine-spatial --lib zfight` x2:
  two boxes sharing a face at y=10 cm with 1,080,000 cm2 overlap give 1 pair, 2 cm apart give none, 20
  permutations equal digest, catalog stays 18. F2P: no coplanar detector. P2P: golden d57b5c8ae3096614
  unchanged. needs: none.

**8. Stray-robust extent.** Median centre, stray = beyond 20x the median distance, backdrop = diagonal over 50x
median, need 4+ boxes, never flag half the scene; the brief's extent and pair/support passes skip them. Live
case: one actor at -100000 made the scene read 44x too big. Source: SpatialMath.cs:603-718,
SceneSpatialService.Pairs.cs:30-52. Port: integer fn in a new `extent.rs` beside `scale.rs`.
- [ ] SPX8 spatial: robust extent - `scene.brief` reports `extent_cm` without strays plus `stray_ids[]`, and an
  opt-in `stray` check; strays and 50x backdrops are skipped by overlap and support passes. Diff: good. Tier A.
  Size S. Owns `crates/spatial/src/extent.rs` (new), `crates/spatial/src/brief.rs` (extent hunk). Accept: `cargo
  test -p engine-spatial --lib stray_extent` x2: 60 props plus one at 1,000,000 cm keep extent within 1% and
  list 1 stray; 3 boxes silent; half the scene far away flags none. F2P: no robust extent in `brief.rs`. P2P:
  golden d57b5c8ae3096614 unchanged. needs: none.

**9. Rollup and honest counts.** One bad pipe makes three findings; the rollup shows one actor with three.
`finding_count` = before cap and suppression; `returned_count`; `suppressed_count` (the 0-versus-29 lie).
Source: Scene.Spatial.cs:112-163, forge/gold/state-doctrine.md "suppression as health". Port: fold Finding rows.
- [ ] SPX9 endpoint: worst-actor rollup - `scene.spatial` adds `worst_actors[{id,findings,red,checks[]}]` top
  10 and `counts{total,returned,suppressed}`. Diff: regular. Tier A. Size S. Owns
  `crates/endpoint/src/scene_tools/spatial_checks.rs`, `crates/endpoint/src/scene_tools.rs` (output hunk).
  Accept: `cargo test -p engine-endpoint --features mcp --lib spatial_rollup` x2: one actor with 3 kinds leads
  with 3, total = returned + deferred, same bytes under permutation. F2P: output has no per-actor view
  (`scene_tools.rs:357`). P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**10. Outside-of want, anchor first, honest failure.** `outsideOf` keeps a spawn out of a room's footprint at any
height (a room floor is free space). The preferred pose is tried first so "leave it where I asked" works. Two
reasons: wrong surface (`does_not_fit`) versus crowded (`no_free_spot`). Source: PlacementSolver.cs:83-90,
151-168, 230-239; Scene.Spatial.cs:1159-1167. Port: new want `outside` in `place.rs`; reasons beside SPR6.
- [ ] SPX10 spatial: outside want and anchor first - wants gain `outside object:<id>|wall:<w>|region`; the
  preferred pose is returned unchanged when it satisfies all wants and verifies; ConflictSet reason is
  `does_not_fit_on_support` or `no_free_spot`. Diff: good. Tier A. Size M. Owns `crates/spatial/src/place.rs`,
  `crates/endpoint/src/scene_tools.rs` (want parser). Accept: `cargo test -p engine-spatial --lib
  place_outside` x2: a spawn outside a room footprint has 0 XZ overlap and hugs it within 2 cells; a free
  preferred pose is exact; 3 reason fixtures; SP56 audit digest unchanged. F2P: no negative want; one reason
  string. P2P: golden d57b5c8ae3096614 unchanged. needs: none.

**11. Pairwise measure.** Signed gap, per-axis gap, contact within tolerance, penetration ratio of the SMALLER
volume, alignment (flush or co-linear), containment shortfall; exact numbers for weak models. Forge retired
these tools, so build one kind, not six. Source: Scene.Measure.cs:336-614, Scene.Assert.cs:35-188,
SpatialMath.cs:235-303.
- [ ] SPX11 spatial: pairwise measure - `scene.query kind=measure what=gap|overlap|align|contain` with anchor,
  other, tolerance_cm, axes, ref, min_clearance_cm returns signed `gap_cm`, `axis_gap`, `relation`,
  `penetration_pm`, `passed`, `shortfall_cm`. Diff: regular. Tier B. Size S. Owns
  `crates/spatial/src/measure.rs` (new), `crates/endpoint/src/scene_query.rs` (arm). Accept: `cargo test -p
  engine-spatial --lib measure_pair` x2: 40 fixtures match a brute-force oracle 40/40, touching within
  tolerance is contact, a poking crate reports the axis and shortfall. F2P: `near` gives distance only. P2P:
  golden d57b5c8ae3096614 unchanged. needs: none.

**12. Egress and choke point.** Route length from a start to every exit and the narrowest door on the way.
Source: NavMesh.Tools.cs:203-260 (schema only; bridge not read). Port: `nav/funnel.rs` path length plus
`NavView.doors`.
- [ ] SPX12 endpoint: egress - `scene.query kind=egress` with start, exits[], capsule_radius_cm returns
  `all_reachable`, `worst_route_cm`, per-exit route and `bottlenecks[{door_id,width_cm}]` narrowest first.
  Diff: hard. Tier B. Size M. Owns `crates/endpoint/src/scene_query.rs`, `crates/spatial/src/navcheck.rs`.
  Accept: `cargo test -p engine-endpoint --lib egress_door` x2: two rooms and one 80 cm door give the door as
  bottleneck, 20 seeded layouts match a brute-force BFS route within 5 cm, an unreachable exit is listed. F2P:
  no route length at the door. P2P: golden d57b5c8ae3096614 unchanged. needs: AD20.

## 5. Not rowed

- Yaw-aware overlap and a confidence label: AD18 owns it. Add one rule there: a rotated pair with no oriented
  proof never ranks red (Scene.Spatial.cs:135-142). Layout intent: N-12. Camera pick for findings (17 viewpoints,
  Highlight.cs:85-152): `compose.rs` has `best_shot`; see SPR10.
- Raycast against the staged scene: SPR7 plus SPX5 cover it; `physics.raycast` stays collider-blob based.
- Terrain height (old repo): no terrain heightfield on our door; skip until a scene has one.

## 6. Not verified

- Read, not run: no tool was executed, no token figure measured. Costs are caps and arithmetic.
- Not read: `RenderingLiveBridge.ViewProbe.cs` (live ray code), `SceneSpatialService.Support.cs:153-590`
  (floating verdict body), `Brief.cs` bodies, `Scene.Understand.cs:200-444`, vision/* tools, nav bridge code for
  egress, `NavCrowd*`, plugin `Source/Game/*`, `docs/CAPABILITIES.generated.md` past its head, `external/`.
- Live vs retired is inferred from one undated fingerprint file and one skill sentence.
- "On no row" is a keyword grep over `docs/SPRINT*.md` (all 25 shards), `docs/plan`, `docs/research`; a row
  using other words would be missed. Line numbers in the Forge are from the depth-1 HEAD only.
