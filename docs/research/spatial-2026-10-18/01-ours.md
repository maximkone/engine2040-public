# Spatial audit 01: what engine2040 has today (2026-10-18)

Part 1 of the spatial report for Maxim: read-only audit of OUR spatial and scene-query tooling from code and board
rows. Nothing was run (no cargo, no door, no model). Claims carry file:line or board ids; section 12 lists doubts.

## 1. Short answer

- The geometry core is real, deterministic and well tested: 27k lines in `crates/spatial`, integer cm, FNV
  digests, exact relations, line of sight, cover, a placement solver with minimal conflicts.
- The DOOR over it is thin and partly dishonest: every node is a 1 m cube named `prop_NNNN`, the brief has no
  positions, `reachable` is always true, and ~3k lines of finished geometry have no caller.
- No measurement exists of a real model using any of it. Every quality number is engine-vs-oracle (the gold
  agent echoes gold) or a node spike with no LLM. So "do we need to train?" cannot be answered from data yet.
- Training is NOT the first lever. Almost every gap is plumbing, labels, schema text or a missing exact
  algorithm. A small learned part only fits name grounding (section 8). Measure first (SPR4).

## 2. Inventory of crates/spatial (wc -l)

| Module | Lines | What | Caller outside spatial |
|---|---|---|---|
| index3 (+rstar) | 2418+934 | 3D integer-cm index, incremental rebuild, free-space field | checks, solver |
| relation | 3164 | v1 graph (supports/on_top_of/inside/next_to/facing), v2 vocab 13 words x 3 frames, cycles | brief, query |
| place | 2197 | SP5 solver: 10 cm grid x 8 yaws, minimal conflict by shrink-by-removal, repair hints | scene.place |
| sight | 1308 | LOS, camera visible, cover, reach BFS, `ray_first_hit`, visible area | scene.query only |
| brief, afford, repair, checks | 1778, 2019, 1616, ~3000 | budgeted brief, affordance tags, repairs, 18 frozen checks | door, editor |
| sample, compose | 769, 1431 | free-space sampler; shot scoring | scene.query; only `best_shot` (game/director.rs:17) |
| sense_text, isovist, exposure, fps_visibility, kdtree, tag_lift, scale | 573, 942, 533, 339, 280, 379, 183 | text view/lidar/map, visibility polygon, cover exposure, FOV sets, kNN, label lift, unit detector | NONE |

"NONE" = no `engine_spatial::<module>` in any other crate (grep). AD11 says the same of scale.rs.

## 3. What the MCP door exposes (door-reference.md:428-600, scene_query.rs, scene_tools.rs)

| Verb | Scope | Agent supplies | Returns |
|---|---|---|---|
| scene.read | observe | limit, cursor | id, parent, LOCAL pose only |
| scene.brief | observe | budget_bytes (4096 default, 16384 max) | digest, text, truncated, ids[], objects[] |
| scene.spatial | observe | checks[], limit | findings, checks_run, skipped, size_source |
| scene.query | observe | kind + 15 optional fields | digest, bytes, hits[{object,x,y,z,matched,reason}], conflicts[] |
| scene.place | commit | revision, key, object, wants[<=16] | solved, placement, conflicts, hints, dry_run rows |
| play.step, sim.observe | observe | seed, ticks, inputs | per fighter hp, phase, pos, yaw, velocity, events, PNG |
| physics.raycast | observe | cooked collider hex + ray | hit on supplied colliders, NOT the staged scene |

scene.query kinds (scene_query.rs:70-90): near, between, visible, reachable, cover_near, largest_open,
free_space, cone, relation. Relation words (relation.rs:802-815): above below in_front_of behind left_of
right_of against around aligned near far visible_from reachable_from; frames World (front = north), Subject,
Viewer (`other`). Not on the door: sense.view/lidar/map/scene.pixel (SP69 left), view.observe (SP62 open),
isovist, exposure, kNN, scale detector, tag lift, camera framing.

## 4. What an agent actually gets, question by question

Facts that shape every answer (all read in source):
- Door actors are unit cubes (`sx: 1.0` at scene_query.rs:1378, scene_tools.rs:558; test scene_tools.rs:1846
  "no colliders yet"). AD7 open; forge-adoption A.3 #1 calls it "the spatial door lies by default".
- Nodes have no name or tag: components are Mesh, Collider, Material, Light, Spawner, Destructible, Anatomy,
  Rig (scene/src/component.rs:110-130). The door invents `prop_{index:04}` from the UUID-sorted position
  (scene_tools.rs:114, 554); inserting a node shifts later labels.
- scene.brief text = digest, `name [id]`, layer groups (max 3), v1 relation edges, findings, afford rows.
  No coordinates, sizes or real names (brief.rs:34-110, 359-374).
- `ids[]` and `objects[]` come back in full on brief AND spatial, outside the byte budget
  (scene_tools.rs:1007, 1095; spatial_checks.rs:115-130).
- `SceneQueryArgs` has no field docs and `kind` is a free String (scene_query.rs:1290-1307); the
  door-reference Range column is empty. A weak model learns per-kind args only by refusal.

**What is left of X?** `scene.query kind=relation relation=left_of anchor=<label> [other=<viewer>]`. One hit
per match, reason `left_of: world frame`, x,y,z floats, no distance or bearing (scene_query.rs:1089-1127;
only `cone` gives side, degrees, cm, :1064). Limits: 45-degree cone so a diagonal reads as front AND right;
20 m horizon; same story; XZ-overlapping boxes excluded (relation.rs:925-960). RISK, derived by hand not
run: `holds(s, Left, o)` means "o is left of s" (relation.rs:919-938, gold :1832-1838) and `subjects_of`
returns every s for which that holds (:1428). Read naturally, "what is left of X" is the opposite set. The
door test (scene_query.rs:2296-2365) compares door to oracle only, so a shared convention error passes.

**Can A see B?** No pair form. `kind=visible` takes an eye point and yaw, returns a row for EVERY actor up
to 64: `visible`, `blocked:out_of_view`, `blocked:occluded` (scene_query.rs:884-927). One center ray, 90-degree
cone, 20 m, 5 m vertical window (sight.rs:455-482). The blocker id is not reported though `ray_first_hit`
returns it (sight.rs:320, 377). `relation visible_from` is a different test (2D segment vs boxes,
relation.rs:1279-1320), so the two can disagree; relation returns matches only, so "no" = absent.

**Where can I place C?** (a) `free_space` samples Poisson points for a fixed 0.8 m cube (PLACED_SX_M,
scene_query.rs:52, 1148): C's real size, yaw, "by the north wall" cannot be asked. (b) `scene.place` solves
wants but commits: no dry_run (scene_tools.rs:393-401). Wants are only supports, on_top_of, inside, next_to,
facing against wall, object or point; no left_of, between, aligned, row, "in room 2".

**Why did it fail?** ConflictSet = minimal unmet-want subset (place.rs:1085-1112), one fixed sentence ("no
placement on 10 cm grid x 8 yaws satisfies N want(s); minimal conflict covers M"), and single-want-drop hints
with move_cm and a dry-run relation diff (place.rs:105-175). It does not name blocking objects, the slack in
cm, or the nearest near-miss. Overlap is XZ-only, so a lamp on a table reads RED (checks.rs:137-165, AD18).

**Reachable?** `open_floor` puts every actor in nav polygon 0, so all pairs are reachable
(scene_query.rs:1391-1400; AD20 open). Boxes do block `relation reachable_from`, but that is not nav.

## 5. Token cost per answer (ESTIMATES from field sizes; nothing measured on the wire)

- Repo estimator max(ceil(chars/4), words) (brief.rs:387); UUID text tokenizes worse, so read these as lower bounds.
- One query hit (52-char label, 3 floats, reason) ~ 120 JSON bytes ~ 30-45 tokens. Default cap 64 hits =
  2k-3k tokens; `visible` fills the cap in any busy scene.
- scene.brief: text <= 4096 B (~1k tokens) PLUS ids+objects at ~94 B/object (25-45 tokens each):
  200 objects ~ 5k-9k tokens, 2,000 objects ~ 50k-90k tokens per call, repeated on scene.spatial.
  SP19's "2,000 objects under 4k tokens" counts the text field only.
- Library text is cheap: SP69 128x96 view 873 tokens at ~0.4 us per ray.

## 6. Determinism

Library level is strong: meters in, integer cm inside, id-sorted scans, FNV-1a with domain tags (`e2query:1`, `e2sight:1`),
octant yaw (no libm), any input order gives one digest, golden d57b5c8ae3096614 untouched. Answer text is byte-identical
for the same scene and args (scene_query.rs:1-8). Weak spots: hit x,y,z are f32 printed `{:.2}` (:479); digests hash the
positional index, not the UUID (:449-473), so an unrelated insert can shift later digests and labels (inferred, not run).

## 7. Eval and measured quality

| Harness | Measures | Number | Honest reading |
|---|---|---|---|
| scene_query.rs `gold_query_rate` :2104 | engine vs hand gold, >= 100 Qs incl. refusals | all pass, 95% floor | real sizes, not the 1 m cube door |
| relation.rs 200-case gold :1810+ | frame-aware vocab | all pass | author's own convention |
| playtest spatial_eval.rs | 100 place/query/fix (30/40/30) | `GoldAgent` echoes gold = 100/100 by design; seeded `between` bug drops exactly its tasks | tests the harness, not an agent; no cone/relation words (:116-125) |
| spatial/src/spatial_eval.rs | 10 scenes, dual propose + <= 3 repair rounds | clean-first vs clean-after, replay x2 | scripted proposals, no model |
| sense_text tests | 50 seeded vs brute force | 50/50 over 22,800 rays | exact, library only |
| LADDER node spikes | SP58 14/50 SP3-only, 20/50 world-only, 50/50 frame-aware; SP33 60/60 at 0.096 ms | LADDER.md:137,150 | vocabulary simulations in node, no LLM |
| SP20 accept: model+verb vs raw brief | NOT DONE ("needs a live model") | none | the number that decides training |

No file records an LLM accuracy, token or call-count figure for spatial tasks. N-17 is plan-only; AD25 and
AG1 are open and use scripted reference agents (forge-adoption.md:257, 551).

## 8. Would training help? (crates/nn i8 dense/embedding/GRU, NN3 QAT landed, crates/distill)

Plain algorithms win and are cheap: relations, LOS, cover, reach, overlap, free space, placement, conflict
minimality, camera framing, rooms, compaction. The plan already CUTs learned encoders, weights in git and
auto-apply (spatial.md:80-90, 195-205). A learned part fits only the language edge, and only if a gap shows:
- Phrase grounding ("the lamp by the door") over mesh names: SP33 lexicon + vote spiked 60/60 untrained.
  Try SP67 tiny embedding only if recall on REAL asset names stays under 90%.
- Tag guess for unlabeled meshes (tiny bbox/ratio classifier) and shot ranking (compose.rs spearman >= 0.7 gate)
  are later advisory offline spikes, only if the hand rules miss.
The caller is already an LLM. "Training the agent" here means field docs, examples, refusals that teach
(AD26), playbooks (AD27), and a cassette eval that proves they work (SPR4).

## 9. GAP table

| Capability | State | Evidence | Fix by |
|---|---|---|---|
| Real sizes at the door | missing | scene_query.rs:1378, AD7 | algorithm (AD7) |
| Labels, positions, bounded tables in brief | missing | component.rs:110, brief.rs:359, scene_tools.rs:1007 | plumbing: derive from mesh asset |
| Query schema self-description | missing | scene_query.rs:1290 | docs |
| Left/right convention, numbers in hits | unknown/partial | relation.rs:919,1428 | independent gold |
| Pair LOS with occluder, multi-ray fraction | missing | scene_query.rs:884, sight.rs:377,455 | algorithm |
| 3D yaw-aware overlap | missing | checks.rs:137, AD18 | algorithm |
| Real reachability | missing | scene_query.rs:1391, AD20 | plumbing |
| Footprint-aware "where fits" | missing | PLACED_S* 0.8, no dry_run | algorithm |
| Why-failed blockers and slack | partial | place.rs:1102 | algorithm |
| Richer wants (left_of, between, room) | missing | door-reference bad_relation | algorithm |
| Rooms/regions as targets | missing | brief groups = layers | algorithm |
| Sense verbs | partial | sense_text.rs, SP69 | plumbing |
| Camera resolver | partial | compose.rs, E03-14 | algorithm |
| Live view-truth verbs | partial | SP62, SP65 open | plumbing |
| Fuzzy NL grounding | missing | SP33 | lexicon first; embedding if gap |
| Real-model spatial eval | missing | SP20, N-17, AD25, AG1 | harness |

## 10. Already on the board. AD7 extents, AD20 reach, AD18 3D overlap, SP69 door half, AX23 change list, SP66 light,
SP62/SP65 view verbs, SP20/SP58 frame verb, AD25/AG1 scene IQ, AD26 teaching refusals, AD29 resources. Do these first;
the SPR rows below are items I found on no board row.

## 11. Ten improvements, ranked by value to an AI building games through MCP

Order: handles, correctness, proof, cost, new capability. SPR1..SPR10 are unused ids (board-slice, 2026-10-18).

- [ ] SPR1 spatial: handle and place for every object - brief, spatial and query label objects
  `<mesh asset name>_<n> [uuid]` instead of `prop_NNNN` (ordinal stable per name), the brief gains one
  compact row per object (cm position, cm size, yaw octant), and `ids`/`objects` obey the byte budget with a
  `cursor`. Diff: good. Tier A. Size M. Owns `crates/endpoint/src/scene_tools.rs`,
  `crates/endpoint/src/scene_tools/spatial_checks.rs`, `crates/spatial/src/brief.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib brief_handles` x2: 200 objects read back as `lamp_2 [id] x y z`; total
  output at budget 4096 stays under 6 KB; inserting an unrelated node leaves other labels and digests
  unchanged. F2P: labels are `prop_NNNN`, tables unbounded, no coordinates. P2P: golden d57b5c8ae3096614
  unchanged. needs: none.
- [ ] SPR2 spatial: pin left/right/front/behind with an independent gold and put numbers in hits - 40
  plain-English questions from a drawn floor plan in 3 frames, `left_of X` = objects on X's left (fix the
  direction if relation.rs:919/1428 is inverted), and `dist_cm`, `bearing_deg`, `gap_cm` in relation, near
  and between reasons; convention named in the tool description. Diff: good. Tier A. Size S. Owns
  `crates/spatial/src/relation.rs`, `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p
  engine-spatial --lib dir_gold_plain` and `cargo test -p engine-endpoint --features mcp --lib relation_plain`
  x2: 40/40, hits carry integer cm and degrees, digests re-pinned with a named receipt. F2P: gold is derived
  from `holds`; no test states a side in English. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] SPR3 spatial: scene.query describes itself - enum for `kind`, a doc and unit on every field, the
  required fields per kind in the schema text, one example per kind in `api.describe`, and a `missing_arg`
  refusal naming kind, field and a working call. Diff: good. Tier A. Size S. Owns
  `crates/endpoint/src/scene_query.rs` (SceneQueryArgs docs, refusal). Accept: `cargo test -p
  engine-endpoint --features mcp --lib query_schema_docs` x2: every field documented, 9 kinds list required
  fields, door-reference Range column non-empty for scene.query, `visible` without eye_m refuses with the
  example. F2P: no field docs, `kind` is a String. P2P: golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] SPR4 playtest: real-model spatial A/B on cassettes (the missing SP20 number) - 50 seeded questions
  (left of, can see, where fits, why failed) answered by one model from the raw `scene.brief` versus via
  `scene.query`/`scene.place`; provider cassettes make the score replay offline and byte-identical; prints
  exact-match, calls and estimated tokens per arm plus a train-or-not note. Diff: good. Tier A. Size M. Owns
  `crates/playtest/src/spatial_eval.rs` (new live section), `fixtures/spatial-gold/llm/`. Accept: `cargo test
  -p engine-playtest --lib spatial_llm_replay` x2 prints one table; verb arm beats raw arm by >= 20 points or
  the row records the kill; a do-nothing arm scores 0. F2P: no LLM figure exists anywhere. P2P: golden
  d57b5c8ae3096614 unchanged. needs: SPR1.
- [ ] SPR5 spatial: footprint-aware placement options - `scene.place` gains `dry_run`; new query kind
  `place_options` takes the object (real size, yaw) plus the same wants and returns up to k poses with
  clearance cm, move cm and the wants each satisfies, no write; wall or region filter. Diff: good. Tier A.
  Size M. Owns `crates/spatial/src/place.rs` (enumerate beside `solve`), `crates/endpoint/src/scene_query.rs`,
  `crates/endpoint/src/scene_tools.rs`. Accept: `cargo test -p engine-spatial --lib place_options` and `cargo
  test -p engine-endpoint --features mcp --lib place_dry_run` x2: a 2.0 x 0.9 m couch gets only poses that
  verify clean, the fixed 0.8 m footprint is gone, dry_run moves neither revision nor digest. F2P:
  PLACED_SX_M fixed, scene.place always commits. P2P: golden d57b5c8ae3096614 unchanged. needs: AD7.
- [ ] SPR6 spatial: say why a placement failed - each unmet want in a ConflictSet names the blocking object
  ids, the cm it misses by and the nearest near-miss pose (additive fields); the door prints them beside the
  hints. Diff: good. Tier A. Size M. Owns `crates/spatial/src/place.rs`,
  `crates/endpoint/src/scene_tools.rs`. Accept: `cargo test -p engine-spatial --lib conflict_blockers` x2: 20
  seeded over-constrained intents each list a blocker or wall and a miss in cm per unmet want; the minimal
  subset and the SP56 audit digest are unchanged. F2P: reason is one fixed sentence with counts only. P2P:
  golden d57b5c8ae3096614 unchanged. needs: none.
- [ ] SPR7 spatial: pair sight with the occluder - `scene.query kind=sees anchor=A other=B` answers yes/no,
  first blocker id and entry point (`ray_first_hit`), a visible fraction from a 5-ray fan (center, head, four
  corners), and `relation visible_from` delegates to it (one definition). Diff: good. Tier A. Size M. Owns
  `crates/spatial/src/sight.rs`, `crates/spatial/src/relation.rs`, `crates/endpoint/src/scene_query.rs`.
  Accept: `cargo test -p engine-spatial --lib sees_pair` x2: 50 seeded pairs match a brute-force oracle, a
  pillar case names the pillar, half-hidden gives a fraction strictly between 0 and 100, relation and sees
  never disagree, <= 0.2 ms per pair. F2P: visible returns a row per actor with no blocker; two definitions.
  P2P: golden d57b5c8ae3096614 unchanged. needs: AD7.
- [ ] SPR8 endpoint: compact answers - `scene.query` and `scene.spatial` accept `format` rows|ids|count and
  `matched_only`; labels go out once in a legend; `visible` stops returning blocked rows unless asked. Diff:
  regular. Tier A. Size S. Owns `crates/endpoint/src/scene_query.rs`, `crates/endpoint/src/watch_ledger.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib query_compact` x2: the same 64-hit `visible`
  answer shrinks >= 60% in chars with identical digest and ids; the ledger prints chars before and after.
  F2P: each hit repeats a 52-char label and a reason. P2P: golden d57b5c8ae3096614 unchanged. needs: SPR1.
- [ ] SPR9 spatial: rooms and regions as query targets - flood-fill wall boxes into `room_1..n` (door = gap
  wider than the capsule) with center, area, exits; kinds `in_room`, `room_of`, `exits_of`; wants `inside
  room:<n>`; brief groups by room. Reuses isovist/navcheck code that has no caller. Diff: hard. Tier B. Size
  M. Owns `crates/spatial/src/rooms.rs` (new), `crates/spatial/src/lib.rs`,
  `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p engine-spatial --lib rooms` x2: a two-room
  house gives 2 rooms and 1 exit, 50 seeded scenes match hand labels 50/50, digest stable under input order.
  F2P: no room concept; groups are arena/zone/object. P2P: golden d57b5c8ae3096614 unchanged. needs: AD7.
- [ ] SPR10 spatial: camera framing verb - `scene.frame target=<id> shot=wide|medium|close` returns eye, yaw,
  pitch, fov that keep the target centered, unoccluded (SPR7 fan) and inside the room, ranked by
  `compose::score_shot`. Diff: hard. Tier B. Size M. Owns `crates/spatial/src/compose.rs`,
  `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p engine-spatial --lib frame_target` x2: 20
  seeded targets get full-visibility poses with no camera-in-body, ranking agrees with the hand slate
  (spearman >= 0.7), same bytes twice. F2P: compose scores a shot but never proposes one; no door verb. P2P:
  golden d57b5c8ae3096614 unchanged. needs: SPR7.

## 12. Not verified (be skeptical of these)
- Nothing was executed: no cargo test, no live door call, no model call. "Returns" claims are read from
  source; token figures are arithmetic on field sizes, not wire captures.
- The left/right inversion (SPR2) is a hand derivation from relation.rs:919-938 and gold :1832; I may be
  misreading the intended `holds(s, rel, o)` convention. A lead, not a finding.
- "No name or tag on nodes" and "no outside caller" rest on component.rs plus greps for `engine_spatial::<mod>`;
  provenance.rs, prefab.rs, the editor and re-exports were not read for another name store or caller.
- Other sessions have uncommitted edits in endpoint, playtest, render; line numbers come from the checked-out files
  and may shift.
- Plan budgets (1 ms, 50 ms, 0.2 ms) and LADDER spikes were not re-run; Forge/old-Flax only via forge-adoption.md.
