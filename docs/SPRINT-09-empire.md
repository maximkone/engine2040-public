### Empire (owner 2026-09-24, `center/EMPIRE.md`; dispatch beside the 2040 tier)

General engine (GE), living world (LW), AI builds inside (AB). Models never run
inside the tick: every model output becomes a validated, tick-stamped, recorded
action or line, so replays reproduce without a provider.

- [ ] GE6 ui: scoreboard moves to game — killer/victim fighter rows become
  a `game`-owned arena HUD model; ui keeps generic rows, text, layout,
  quads. Tier A. Size S. Owns `crates/ui/src/lib.rs` plus the new game HUD
  model. Accept: HUD tests green; ui builds with no `engine-input` fighter
  import. needs: GE2.
- [ ] GE11 proof: ledger refresh for post-GE2 files — re-audit crates for
  `Fighter|Gladiator|Arena` names and caps added since the GE2 ledger
  (`ai/src/legion.rs`, `game/src/siege.rs` shards,
  `game/src/ludus_npcs.rs` slots and any newer files), extend the
  `workspace-check` ledger verdicts and keep the seeded-refusal gate
  green. Tier A. Size S. Owns the ledger hunks in
  `tools/workspace-check.mjs` plus the ledger table. Accept: zero
  unlisted genre names; a seeded new assumption fails the gate; proof
   generality gates green. needs: GE2. (PLN18 runtime.md, reviewed
   2026-09-26.)
- [ ] GE12 input: canonical participant-cap decision — rule whether
  `MAX_PARTICIPANTS` stays 16 or moves to 32 across
  `crates/input/src/fighter.rs`, arena slots, net scale/select/soak
  and the `workspace-check` edge, with the HASH-LAW/golden review
  and the MP2 cap-table consequences posted. Tier B. Size S. Owns
  the decision + cap hunks (one value, every gate). Accept: 32-bot
  x600 soak hashes x2 or 16 stays with the split-match-32 harness
  defined; golden `d57b5c8ae3096614` unchanged x2; GE2 ledger
  green. needs: GE3, MP2. (MP16 kill, proposed 2026-09-29.)
- [ ] LW5 ai: reflection and importance — bounded importance 0..=1000, Progress 2026-10-17: bounded importance (memory salience 0..=1000) and Mind::reflect(tick) exist (memory.rs, mind.rs, ai suite green run 39056802035) and the living.rs 20-fixture suite asserts grounded_count == rows; left: no crates/ai/src/reflect.rs, no fixed-cadence reflection, no capped surprise.
  reflection on a fixed cadence, and a grounded check that every line an NPC
  says matches its memory (generative agents 2304.03442, ideas only; the
  Apache-2.0 repo needs a pinned SHA before any port). Tier B. Size M. Owns
  `crates/ai/src/reflect.rs`. Accept: 20 fixtures lower the contradiction rate
  on mock, local and remote; same bytes x2; a 10k-tick soak holds the memory
  cap. Deep sweep 2026-09-25: every line an NPC says cites its recall set; a negation or contradiction falls back and is recorded; importance stays integer; reflection on a fixed cadence plus capped surprise. needs: LW2, EV1.
- [ ] AB14 playtest: bad-build brief fails loudly — a seeded bad brief
  lowers exactly its stage score and reproduces x2 (clean EV2 b01 vs
  bad seed95 must differ); current `sim.plan_goals`
  bad-brief-still-solved fixed. Tier A. Size S. Owns eval hunks beside
  `crates/playtest/src/brief_eval.rs`. Accept: bad brief deltas >0 and
  x2 identical; empty brief still refused. needs: EV2, AB4. (PLAYTEST56
  bet1 delta-0 fail-to-fail wave 56.)
- [ ] LW12 ai: multi-NPC recall/drain budget soak — 64 minds recall
  top-8 and drain through `apply_at` at the tick boundary, 0-per-tick.
  Tier B. Size S. Owns soak hunks in `crates/ai/src/memory.rs` plus
  `crates/game/src/ludus_npcs.rs`. Accept: 64-NPC soak holds caps with
  recall/drain p95 printed; `cargo test -p engine-ai --lib memory` plus
  `cargo test -p engine-game --lib ludus_npcs` green; golden unchanged
  twice. needs: LW1, LW2, LW8. (PLN19 living-world.md.)
- [ ] LW13 playtest: grounded-line plus contradiction ratchet — every Progress 2026-10-17: living.rs consistency_numbers_follow_the_gate asserts grounded_count == rows and the one-pair repair quarantines seeded lines; left: contradiction_pairs stays 2 per backend (6 rows dropped) on the seeded set, not 0 after repair. engine-playtest suite not run in this audit.
  said line cites its recall set, seeded pairs quarantine, replay x2.
  Tier B. Size S. Owns eval hunks beside `crates/playtest/src/living.rs`.
  Accept: `cargo test -p engine-playtest --lib living` green with
  `grounded_count == rows` and `contradiction_pairs == 0` after repair;
  bytes x2. needs: LW2, LW4. (PLN19 living-world.md.)
- [ ] LW14 ai: reflection cadence plus capped surprise — fixed-cadence Progress 2026-10-17: mind.rs reflect(tick) folds the ring into Felt (mind_tick_delayed_reflection_*) and memory soak_10k_holds_cap holds the cap (ai suite green run 39056802035); left: no fixed-cadence scheduler and no capped surprise.
  `reflect` folds top recall into `Felt`, surprise capped, 10k-tick soak
  holds 512/128 caps. Tier B. Size S. Owns reflect hunks in
  `crates/ai/src/mind.rs` plus `crates/ai/src/memory.rs`. Accept:
  `cargo test -p engine-ai --lib mind` plus `cargo test -p engine-ai
  --lib memory` green; bytes x2. needs: LW1, LW2. (PLN19 living-world.md.)
- [ ] AB15 mcp: brief-chain cost plus latency receipt — every AB1-AB4
  stage reports latency plus cost beside its digest, unknown stays
  unknown. Tier B. Size S. Owns receipt hunks in
  `crates/endpoint/src/build_game.rs` plus `crates/endpoint/src/brief.rs`.
  Accept: 20-brief receipt prints latency plus cost per stage;
  `cargo test -p engine-endpoint --lib build_game` green; live/offline
  digests match x2. needs: AB4, AB9, EV2. (PLN19 living-world.md.)
- [x] AB16 mcp: canon rule-cap ratchet plus VK1 gate — 17-rule set stays Landed 74654b2. Tick audit 2026-10-17: canon thirty_seeded_sets_shrink_by_thirty_percent, no_false_merges_on_keypoint_distinct_pairs, rules_stay_bounded_without_dependencies (RULE_CAP 30) green in gh-check run 39057874705 (endpoint --features mcp --lib).
  under `RULE_CAP=30` with 0 false merges on VK1 inside 60 s. Tier B.
  Size S. Owns hunks in `crates/endpoint/src/canon.rs`. Accept:
  `cargo test -p engine-endpoint --lib canon` green; 30 sets shrink
  ≥30% with 0 false merges on VK1; bytes x2. needs: AB6, VK1.
  (PLN19 living-world.md.)
- [ ] AB10 mcp: procedural-vs-model content spike — one AI brief (e.g. sword
  slash) through the CT14→HR6 model path vs a demoscene-style procedural
  path (noise/implicit math from the same seed, no model in the tick).
  Experiment. Size S. Owns scratch lane files only. Accept: procedural
  scores ≥7/10 blind at ≤50% model-path cost with byte-identical replay
  x2, else killed with numbers (quality <5/10 or replay fails kills it).
  needs: CT14, HR6.
- [ ] OB4 observe: deterministic event queue — sends queued during phases, Progress 2026-10-17: crates/observe/src/event_queue.rs landed (thousand_events_match_across_chunkings, handler_changes_during_flush_apply_at_boundary, tape digest; observe green run 39057303818); left: file says test-only with no production caller, and the PF1 within 5% bar is not measured.
  flushed at the phase boundary in priority-band/insertion order, recorded
  in the tape (evenio `event.rs` + `handler.rs`, MIT, dispatch discipline
  reimplemented; immediate-send path never taken). Tier B. Size S. Owns
  `crates/observe/src/event_queue.rs`. Accept: 1k-event tape identical x2
  across chunkings; mid-iteration spawn/remove safe; PF1 within 5%.
  needs: OB1.
- [x] SP43 spatial: VLM marked-proposal + engine-solver repair loop for Landed 2ef5ef22.
  brief compile (LayoutVLM ideas, reimplement). Tier A. Size M. Owns
  repair-loop hunks beside the brief pipeline + `spatial_eval.rs`.
  Accept: 10 AB1 briefs, SP23 clean-first-pass today → dual-proposal +
  self-consistent decode + engine repair, +pp clean-first-pass at ≤3
  rounds, bytes x2. needs: SP23. (Pillar reader wave 57 Card 1.)
- [ ] SP44 spatial: quantitative metric spatial-VQA from the graph
  (distances, sizes, counts in integer cm; SpatialVLM ideas,
  reimplement). Tier A. Size M. Owns metric-query verb hunks beside
  `crates/spatial/src/relation.rs` + `crates/endpoint`. Accept: 50
  seeded metric questions, model on raw brief vs model with engine
  query verb, exact-match rate + tokens posted. needs: SP20. (Pillar
  reader wave 57 Card 2.)
- [ ] SP45 spatial: region-proposal metric relations (caller passes
  region handles, engine returns direction/distance; SpatialRGPT ideas,
  reimplement). Tier A. Size M. Owns region-relation hunks beside the
  SP33 grounding path + `crates/endpoint`. Accept: 60 seeded region
  pairs, recall@3 ≥ 90%, ≤ 1 ms/query, bytes x2. needs: SP33. (Pillar
  reader wave 57 Card 3.)
- [x] SP46 spatial: offline open-vocabulary tag lift from labeled Landed 45c4abe. Tick audit 2026-10-17: tag_lift_thirty_labeled_rows_hit_gold_and_never_auto_apply (30 rows, precision/recall printed, auto_applied 0), tag_lift_report_bytes_replay_twice green in gh-check run 39057177498 (engine-spatial, 303 passed).
  captures into SP3 nodes (human-verified gold, never auto-apply;
  ConceptGraphs ideas, reimplement). Tier A. Size M. Owns tag-lift
  hunks beside the SP22 affordance gold + SP35 brief pipeline.
  Accept: 30 staged captures, +open-vocab tags with gold
  precision/recall posted, 0 auto-applies, bytes x2. needs: SP22.
  (Pillar reader wave 57 Card 4.)
- [ ] SP67 spatial: tiny offline embedding model for scene queries, a spike (EB-2026-10-09-S113;
  donor candidates in inbox item S130: MiniLM-L6-v2 Apache Q8 about 40 MB, tokenizers Apache,
  instant-distance MIT, candle MIT or Apache). Under 50 MB, no key, weights kept out of git, vocab
  wired, ms per query printed; killed with numbers if over budget. Diff: hard. Tier C. Size M. Owns
  `crates/spatial/src/embed.rs`, `crates/spatial/src/lib.rs`. Accept: `cargo test -p engine-spatial
  --lib embed` green x2 with scene queries answered offline inside the ms budget, the vocab hash gate
  holding, and no weights file in `git ls-files`. F2P: no embed module exists today. P2P: golden
  `d57b5c8ae3096614` x2 (the tick never sees a vector); `cargo test -p engine-spatial --lib` green.
  needs: none.
- [ ] SP68 spatial: quantized spatial runtime for nav and place queries, a spike
  (EB-2026-10-09-S114; donor candidates in inbox item S130: recast Zlib shapes, tract convert-dep).
  Under 20 MB, advisory only, offline, the tick never sees a float. Diff: hard. Tier C. Size M. Owns
  `crates/spatial/src/place.rs`, `crates/nav/src/quant.rs`, `crates/nav/src/lib.rs`. Accept:
  `cargo test -p engine-nav --lib quant` and `cargo test -p engine-spatial --lib place` green x2;
  the quantized runtime size and the nav and place query results printed against the heuristic path.
  F2P: no quant module exists in crates/nav today. P2P: nav crowd tests green; golden
  `d57b5c8ae3096614` x2. needs: none.
- [ ] SP69 spatial: CPU sense verbs, text only so D7 is not touched (EB-2026-10-09-S118, Maxim yes).
  From the index3 ray casts in sight.rs: sense.view(camera, w, h) is a run-length text picture, each
  cell an entity id plus a depth bucket; sense.lidar(origin, fan) is 72 distances plus ids;
  sense.map(center, radius) is a top-down text grid with legend, height and walkable cells;
  scene.pixel(x, y, camera) is one ray: entity id, 3D point in integer cm, distance. No render, no
  vision model. sense.cone stays SP58; sense.look waits for the first real frame (D7). Door verbs are
  one line each in scene_tools.rs, and tool_count_pin moves with them (E71). Diff: hard. Tier B. Size M.
  Owns `crates/spatial/src/sight.rs`, `crates/spatial/src/sense_text.rs`, `crates/spatial/src/lib.rs`,
  `crates/endpoint/src/scene_tools.rs`. Accept: `cargo test -p engine-spatial --lib sense` green x2:
  50 seeded camera cases make view, lidar and map match a brute-force check 50 of 50, at most 0.2 ms
  per ray (the SP8 bar), the same text twice, tokens and ms per view posted. F2P: no sense module
  exists today. P2P: integers only in the hash; golden `d57b5c8ae3096614` x2. needs: none. Progress 2026-10-17: 99ef60ab sense_text.rs view/lidar/map/pixel and sight.rs ray_first_hit; 50 seeded cases match an exact brute force 50/50 over 22,800 rays, 128x96 view about 0.4 us per ray, 873 tokens; left: the door verbs in scene_tools.rs and the tool_count_pin move.
- [ ] SP70 spatial: capture-to-symbol lift (EB-2026-10-09-S120): a capture PNG plus depth becomes
  Actor proposals in index3, std-only, integer cm, any VLM advisory and offline, no weights in git
  (ER-2 sees pixels; we read symbols only). Diff: hard. Tier B. Size M. Owns
  `crates/spatial/src/lift.rs`, `crates/spatial/src/lib.rs`. Accept: `cargo test -p engine-spatial
  --lib lift` green x2 on the 200-image gold with box recall and cm error printed. F2P: lift.rs does
  not exist today. P2P: golden `d57b5c8ae3096614` x2. needs: none.
- [ ] SP71 spatial: SP12-B parity bench (EB-2026-10-09-S121): one command reproduces the 200-task
  score x2 through MCP mock, local and remote, tokens posted. scene.report and the memory ledger from
  the same item are not in this row (metric questions are SP44; the ledger waits for a consumer).
  Diff: good. Tier B. Size M. Owns `crates/playtest/src/spatial_eval.rs` (bench hunks only). Accept:
  `cargo test -p engine-playtest --lib spatial_eval` green with the SP12-B score printed identical
  twice. F2P: no 200-task bench exists today (spatial_eval.rs holds 100 text tasks). P2P: golden
  `d57b5c8ae3096614` x2. needs: SP44.
- [ ] SP72 spatial: grasp points and trajectory waypoints (EB-2026-10-09-S120, second half): afford is
  a tag today and place solves one transform, not a path. Std-only, integer cm. Diff: hard. Tier B.
  Size M. Owns `crates/spatial/src/grasp.rs`, `crates/spatial/src/traj.rs`,
  `crates/spatial/src/lib.rs`. Accept: `cargo test -p engine-spatial --lib grasp` and
  `cargo test -p engine-spatial --lib traj` green x2 with grasp clearance in cm printed on the SP70
  gold. F2P: grasp.rs and traj.rs do not exist today. P2P: golden `d57b5c8ae3096614` x2. needs: SP70.
- [x] SP16 spatial: stacked placement — the SP5 solver reads the Landed de92a4b5.
  index3 volume for `OnTopOf`/`Supports`/`Inside` targets with
  ground-vs-support rest distinction; ground-plane behavior unchanged
  for flat rooms. Tier A. Size M. Owns the support-rest hunks in
  `crates/spatial/src/place.rs` (solver API frozen). Accept: two-floor
  ludus and siege-wall fixtures place on shelves and stairs with 0
  findings; flat-room goldens byte-identical; bytes x2.
  Landed so far (2026-10-13 check): 33c5216f (support_gap_cm helper), 7d6c882f (whole shape: shelf rest, ludus slab/stairs, siege-wall top, 0 findings, bytes x2, flat-room goldens byte-identical). Remaining: none open — row lands on judge PASS of place green. needs: SP6, SP7. (PLN25 spatial.md, reviewed 2026-09-25.)

- [ ] SL3 ops: CPU-only render references — once each GPU pass is live,
  its CPU twin (`v1_ibl_shadow`, `v1_lighting`, `v1_vfx`, `gi`, `shadow_post`)
  moves under `#[cfg(test)]` as the parity oracle or is deleted; the parked
  R6 FSR stub (`v1_temporal`) goes. Tier A. Size M. Owns those files.
  Accept: no CPU reference reachable from the window build; parity tests
  still run; render tests green. needs: GX2, GX3, GX4.
- [ ] SL5 ops: one FNV — the FNV-1a basis `0xcbf29ce484222325` is hand-rolled Progress 2026-10-17: rust-quality hash_reimpl is down to 17 hits (render 12, proof 2, arena 1, audio 1, editor 1) from 307; physics, net, endpoint, scene, spatial and game are at 0.
  in about 90 places across 25 crates (git grep 2026-09-24). One
  `engine_core::Fnv64` hasher (write_u8/u32/u64/f32-bits, same byte order)
  replaces them crate by crate, when that crate is touched anyway; the
  orchestrator adds the core edges. Tier B. Size S per crate. Owns
  `crates/core/src/lib.rs` first, then one crate per packet. Accept: every
  migrated crate's hash tests and the golden are unchanged. needs: none.
  2026-09-26 (CQ1): the owner landed in `crates/core/src/hash.rs`
  (`Fnv64`, `hash_bytes`, `hash_u64s`, `splitmix64`, `mix64`), every crate
  may now depend on engine-core, and the packet gate refuses new copies;
  307 copied lines remain (`node tools/rust-quality.mjs --list hash_reimpl`).
- [ ] SL16 ops: shared balance constants — `build_encounter.rs`
  mirrors six constants from playtest/ai/combat instead of importing
  them (janitor wave 25, CODELAW-1). Tier A. Size S. Owns the shared
  contract + edge. Accept: one owner per constant; endpoint imports;
  clippy + deny green. needs: none.
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [ ] SL21 ops: window-path per-frame allocation audit —
  counting-allocator submit test over the RA1 window submit path
  (JAN28-1: per-frame `Vec`s in the submit path have no
  headless-equivalent gate); reuse buffers or record explicit N/A with
  numbers. Tier A. Size S. Owns the audit hunks plus the submit test
  (no new pass code). Accept: 0 per-frame heap allocs after warmup on
  the   submit path; suites green; golden unchanged. needs: SL6, PF2.
  (PLN29 quality.md, reviewed 2026-09-25.)
- [ ] SL23 ops: single-owner texture bounds — `MAX_TEXTURE_BYTES` and
  siblings are mirrored across `assets::texture`, `render::live` and
  `game::practice` (JAN42-1). Tier A. Size S. Owns the edge decision
  (render→assets and/or game→render import) + import migration.
  Accept: exactly one definition site per bound; all three suites
  green; deny green. needs: none.
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [x] SL24 ops: one FNV in game — `crates/game` re-derives FNV-1a/SplitMix Progress 2026-10-17: rust-quality hash_reimpl is 0 in engine-game; left: engine-game --lib has 1 red test in gh-check run 39057596405, tests::refilled_outcome_matches_fresh_tick_for_tick (lib.rs:2421, tree is dirty with other sessions' game work), so the cargo test -p engine-game green bar is not met. Landed 30d3e1a8. refilled_outcome_matches_fresh_tick_for_tick passes; the twin now fills tick_breaks (run 39058909593).
  in 8 files (capture, camera x2, staged_fight, package, ludus_npcs,
  ballistics, siege, modes_world) while depending on `engine-core`
  (JAN50-1). Tier B. Size S. Owns the 8 import swaps + local deletes.
  Accept: `cargo test -p engine-game` green + quality `hash_reimpl`
  down. needs: none. (JAN50.)
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [x] SL28 ops: one FNV owner in physics — FNV-1a re-derived in ~10 Landed f4adec1. Tick audit 2026-10-17: rust-quality hash_reimpl 0 hits in engine-physics, physics suite green in gh-check run 39057362796.
  authoritative hash sites (islands, solver, rigid, kcc, vehicle,
  grapple, character, powered, rapier, controller) (JAN51-1). Tier A.
  Size S. Owns `engine-core` dep + swaps (keep byte order so goldens
  don't move). Accept: physics suite green + `hash_reimpl` 29→0.
  needs: none. (JAN51.)
  Wave 52: TICK f4adec1 (judge 3/3; SL32 saturating covered same commit).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [x] SL33 ops: shared spawn throttle in render — duplicated bodies in Landed a231b9d. Tick audit 2026-10-17: throttle_spawn in particles_gpu.rs is the single owner, used by particles.rs, particles_gpu.rs and vfx_hanabi.rs; render lib green in gh-check run 39057649152 (719 passed).
  `particles.rs:241-258` vs `particles_gpu.rs:471-491` + `as` trio
  (JAN52-F1). Tier B. Size S. Owns one `throttle_spawn` helper + 2
  swaps (or documented divergence). Accept: render suite green +
  `dup_fns` down. needs: none. (JAN52.)
  Wave 52: TICK a231b9d (checker PASS).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [x] SL34 ops: one FNV owner in endpoint — re-derived across canon, Landed ddf8925. Tick audit 2026-10-17: rust-quality hash_reimpl 0 hits in engine-endpoint, endpoint lib green in gh-check run 39057713380 (158 passed, tests/ dirs not run), golden green in run 39057745085.
  build_game/character/level, brief, scene_query, insight, assistant,
  play_session, build_encounter (JAN52-F2). Tier A. Size S. Owns
  `engine-core` dep + swaps (keep `^ domain` order). Accept:
  endpoint suites green + goldens unchanged + `hash_reimpl` 36→~0.
  needs: none. (JAN52.)
  Wave 52: TICK ddf8925 (judge 3/4).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [ ] SL35 ops: zero-alloc render frame — `stage_frame` per-frame heap Progress 2026-10-17: submit.rs cq7_present_alloc_tests::steady_120_frames_read_zero_allocs counts 0 allocs over 120 frames of a mirror of the submit CPU slice (effective_camera, eg38_submit_order, uniforms); left: stage_frame itself is not under the probe. Needs --features window, not run in this audit.
  (`present.rs:3117,3132`, callers 3295/3533) (JAN52-F3). Tier A.
  Size S. Owns scratch reuse (or `alloc-ok` with numbers).
  Accept: counting frame test 0 allocs + render suite green.
  needs: none. (JAN52.)
- [x] SL36 ops: timer out of spatial production — `full_check_timed` Landed d1f174a. Tick audit 2026-10-17: rust-quality nondet 0 hits in engine-spatial, spatial suite green in gh-check run 39057177498 (303 passed).
  `Instant` in prod, callers test-only (JAN52-F4). Tier B. Size S.
  Owns `#[cfg(test)]` move or `det-ok` waiver. Accept: spatial suite
  green + `nondet` 2→0. needs: none. (JAN52.)
  Wave 52: TICK d1f174a (checker PASS).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [ ] SL37 ops: checked conversions sample in render tick — silent
  `as` in particles/aa_scale (JAN52-F5). Tier B. Size S. Owns named
  conversions at flagged sites. Accept: render suite green + golden
  x2 + `casts` down. needs: none. (JAN52.)
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [x] SL38 ops: sim-clippy test-only expects in scene/spatial — adopt Landed d1f174a. Tick audit 2026-10-17: cargo clippy -p engine-scene -p engine-spatial --all-targets -- -D warnings PASS in gh-check run 39057832197; scene suite run 39057370994 and spatial suite run 39057177498 green.
  blocked: `clippy --all-targets -D warnings` rejects test-only
  `Instant::now` (spatial index3/probe/sight/staged) + test-helper
  `f32::sin/cos` (scene transform.rs:551 quarter_turn_y).
  Tier B. Size S. Owns scoped `#[expect(clippy::disallowed_*, reason)]`
  on each test-only use per `tools/sim-clippy.toml` header (production
  already nondet 0, config stays unwakened). Accept: clippy green both
  crates --all-targets + suites green, then SIMCLIPPY-ADOPT re-runs
  unchanged. needs: none. (SIMCLIPPY-ADOPT wave 52.)
  Wave 52: TICK (SL38 worker: 92 clippy errors → 0, sha256-identical
  copies, scene+spatial suites green; pending trim ae98ba8).
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [ ] SL42 arena: checked conversions in the recorder tick — Progress 2026-10-17: arena casts are down to 37 from 46 and the recorder.rs sites are gone, arena suite green in gh-check run 39057483466; left: lib.rs still has config.ticks as usize at 837, 911, 1186 and 2021.
  `recorder.rs:182 index as u8`, `recorder.rs:160-177` range checks,
  `lib.rs:391,465 config.ticks as usize`. Tier B. Size S. Owns the
  conversion hunks in `crates/arena/src/recorder.rs` +
  `crates/arena/src/lib.rs`. Accept: arena suite green + golden x2 +
  `casts` 46 down. needs: none. (JAN53-F4.)
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [ ] SL43 game: board-ID/history comment scrub sample —
  `practice.rs` HR headers, `camera.rs:2140` GP wave history,
  `camera_feed.rs:945` packet note; keep live refs. Tier C. Size XS.
  Owns comment-only hunks (no behavior change). Accept: game suite
  green + `board_ids` 365 down, zero API/doc rows removed. needs: none.
  (JAN53-F5.)
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [ ] SL44 render: window-submit per-frame allocation audit with Progress 2026-10-17: same test as SL35, submit.rs steady_120_frames_read_zero_allocs, shows 0 allocs per frame for the submit CPU slice mirror; left: the live window submit path is not itself probed and suites were not run here.
  headless-equivalent gate (READER-SPEED53 card 2; renumbered from SL39:
  JAN53 filed SL39-SL43 same wave). Tier B. Size S. Owns RA1 submit
  audit hunks beside `crates/render/src/present.rs:3084`. Accept:
  0 heap allocs/frame steady-state or explicit N/A with numbers +
  suites green + golden x2. needs: none. (SPEED53.)
- [x] SL45 net: engine-core owner for the leftover folds — `relay.rs:110` Landed 3ba9bfd. Tick audit 2026-10-17: rust-quality hash_reimpl 0 hits in engine-net, net lib green in gh-check run 39057462548.
  splitmix mix, `tick_budget.rs:439,619`, `wan_soak.rs:192,338`
  (NETSLIM-R2 judge: out of that packet's scope, all production).
  Tier B. Size S. Owns the three files' fold hunks. Accept: net suite
  green + `hash_reimpl` 5→0 + goldens unchanged. needs: SL29. (Judge
  wave 53.)
  Wave 54: TICK (checker PASS; hash 5→0, 168 serial green).
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [x] SL29 ops: one FNV owner in net — FNV/splitmix re-derived in Landed 3ba9bfd. Tick audit 2026-10-17: rust-quality hash_reimpl 0 hits in engine-net, net lib green in gh-check run 39057462548 (258 passed).
  rollback/replay hash stream (rollback, server, netcode, lagcomp,
  replay, replicate, relay) (JAN51-2). Tier A. Size S. Owns
  `engine-core` dep + swaps (relay loss-matrix bit-identical).
  Accept: net lib serial green + `hash_reimpl` 18→0. needs: none.
  (JAN51.)
  Wave 52 R2 diagnosis (judge FAIL, reverted): lane built a SECOND hash
  owner (`crates/net/src/hash.rs` dupes core consts) + AoiGrid still
  unwired + `Instant`/`HashMap` residues (dedicated/lobby/session) +
  unproven equivalence. R2 must `use engine_core::{Fnv64,...}` (no new
  hash.rs), wire-or-remove AoiGrid, waive or fix nondet sites, pin
  old-vs-new digests + published vectors.
  Wave 53: TICK 3ba9bfd (judge 4/4; 7 folds bit-identical, aoi.rs deleted,
  0 waivers, 168 serial green; leftovers → SL45).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [x] SL30 ops: BTree hierarchy in scene — `Hierarchy.nodes` is a Landed 26e7a6a. Tick audit 2026-10-17: rust-quality nondet 0 hits in engine-scene, scene suite green in gh-check run 39057370994.
  `HashMap` iterated in `validate` (JAN51-3). Tier A. Size S. Owns
  `BTreeMap` + `Ord` on `ObjectId` (+ codec/prefab scratch).
  Accept: scene suite green + `nondet` 10→0. needs: none. (JAN51.)
  Wave 52: TICK 26e7a6a (checker PASS).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [x] SL31 ops: BTree maps + timer waiver in net — `HashMap` rollback/ Landed 3ba9bfd. Tick audit 2026-10-17: rust-quality nondet 0 hits in engine-net, SIM_CLIPPY_PENDING is empty in tools/rust-quality.mjs, net lib green in gh-check run 39057462548.
  interest state + `Instant` transport timer (JAN51-4). Tier A. Size S.
  Owns `BTreeMap` swaps + `det-ok` waiver or timer move (+ AoiGrid
  wire-or-remove decision). Accept: net lib serial green + `nondet`
  22→0 + SIM_CLIPPY_PENDING exit. needs: none. (JAN51.)
  LATER: parked 2026-10-03 (circus verdict 2: code hygiene (hash, cast, comment and constant dedupe) that changes no pixel, hash or timing in the arena duel, and the owner's Claude session lands CQ/SL work (78c8d55)); unpark by deleting this line.
- [x] SL32 ops: checked conversions in physics tick — silent `as` in Landed f4adec1. Tick audit 2026-10-17: physics suite green in gh-check run 39057362796, golden test engine-proof --lib golden green in run 39057745085; the 6 saturating casts are in the landing commit.
  solver/islands math (JAN51-5). Tier B. Size S. Owns named checked
  conversions at the flagged sites (saturating vs expect needs lane
  sign-off). Accept: physics suite green + `casts` drops + golden x2
  rerun. needs: none. (JAN51.)
  Wave 52: TICK f4adec1 (same commit as SL28).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [ ] SHOW3 playtest: highlight reel — rank moments in tournament tapes (PQ2)
  by event metadata (KO, clutch parry, comeback, long combo) and cut a reel
  within a length budget (knapsack over moments, VASTSum-style, arXiv
  2605.09507), deterministic; vision scoring only as an advisory note. Feeds
  devlogs and trailers. Tier B. Size S. Owns `crates/playtest/src/highlights.rs`.
  Accept: 200 tapes give the same reel twice; a seeded best moment is picked.
  needs: SHOW1, PQ2.
- [ ] PB51 playtest: play.step digest audit — 2-fighter digests drifted
  low digits vs wave 48 while staged hash holds (PLAYTEST51 wave 51).
  Tier B. Size S. Owns `crates/endpoint/src/play.rs` digest hunk.
  Accept: same seed+inputs byte-identical x2 + cross-build policy;
  t10 pinned, t30/t60 delta explained. needs: VP1, QA1. (PLAYTEST51.)
- [ ] PB52 game: staged exchange — hero-vs-dummies shutout
  (`final_hp=[1000,0,0,0]`, PLAYTEST51 wave 51). Tier B. Size S. Owns
  `crates/game/src/bin/staged_fight.rs` script hunk. Accept: hits land
  both ways or receipt declares hero-vs-dummies. needs: CT15, GA1.
  (PLAYTEST51.)
- [ ] PB53 game: captures bracket the fight — 600-tick fight captured Progress 2026-10-17: staged_fight_frames.rs capture_ticks picks the first, third-way and last hit ticks of the staged run; left: no frame brackets each knockdown and no sequence_fnv x2 check.
  only 0/30/60/90, knockdowns invisible (PLAYTEST51 wave 51). Tier B.
  Size S. Owns staged capture request. Accept: frames bracket first
  hit + each knockdown (or tick→cue links), `sequence_fnv` x2.
  needs: CT15, QA4. (PLAYTEST51.)
- [ ] PB56 game: staged captures are noise not fight (PLAYTEST54 ct15
  64x48 static, hashes differ but unreadable, 27 hits invisible).
  Tier B. Size S. Owns `crates/game/src/capture.rs` headless-capture
  hunk. Accept: 600-tick fight captures bracket first hit + each
  knockdown, bodies/hits readable, `sequence_fnv` x2. needs: CT15, WIN1.
  (PLAYTEST54.)
- [ ] PB57 game: camera loses fight (PLAYTEST54 t60 edge clip, t120 Progress 2026-10-17: camera.rs scripted_orbit_keeps_both_fighters_framed_600_ticks proves framing on a scripted pair; left: not run on the real 6800/6900 arena tape that clipped.
  opposite corners). Tier B. Size S. Owns `crates/game/src/camera.rs`
  framing hunk. Accept: 600-tick 2-fighter tape keeps both in frame
  600/600, no edge clip. needs: GP3. (PLAYTEST54. PLAYTEST68: 6800 repro
  spawn plus post-move clipping; PLAYTEST69: 6900 drift exits frame.)
- [ ] PB58 combat: missing hit layers on contact (PLAYTEST54 t30/t180
  overlap only). Tier B. Size M. Owns `crates/combat/src/feel.rs`
  presentation hunk. Accept: one event gives hit-stop + reaction +
  sound + trauma same tick window, sim bytes x2, captures show layers.
  needs: GP11, GP21. (PLAYTEST54. PLAYTEST68: 30 attack ticks step060-090
  overlapped, no flash/HUD.)
- [ ] PB59 ui: HUD claim vs pixels (PLAYTEST54 smoke `hud_cells 12
  hud_quads 12` but no on-screen text). Tier B. Size S. Owns
  `crates/ui/src/menus.rs` HUD hunk. Accept: smoke HUD counts match
  on-screen cells + HP/facing/bounds overlay on play-step captures.
  needs: WIN1, UI1. (PLAYTEST54.)
- [ ] PB60 playtest: `play.step` still software in-agent (PLAYTEST54
  adapter `endpoint-software`, VP1 GPU path unwired here) + PB51 drift
  refuted (t10/t30/t60/t120/t180 pinned x2). Tier B. Size S. Owns
  `crates/endpoint/src/play.rs` digest/session hunk. Accept: GPU-path
  non-black PNGs when device exists, software fallback labeled; 5 pins
  byte-identical x2 documented, PB51 closed or delta explained.
  needs: VP1, QA1. (PLAYTEST54.)
- [ ] PB61 audio: no hit-synced numbers (PLAYTEST54 only `audio live 2ch
  48000Hz`). Tier C. Size S. Owns `crates/audio/src/coherence.rs`
  bridge hunk. Accept: per-hit loudness/clipping/hit-sync delta in
  smoke/staged logs, 10 scenarios synced. needs: AU1. (PLAYTEST54.)
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] PB62 mcp: index play_run/play_step for api.search/describe
  (PLAYTEST62 bug 4: search 0 hits, describe unknown_path).
  Tier B. Size S. Owns registry hunks in
  `crates/endpoint/src/insight/index.rs` (play-tool entries beside
  the existing MCP tool schemas). Accept: api.search finds both
   paths, api.describe returns full schemas; `cargo test -p
   engine-endpoint --lib insight` green. needs: API1. (PLAYTEST62.
  PLAYTEST68: 0-hits confirmed this run.)
- [ ] PB63 playtest: resend-divergence 3-seed re-probe — seed 6500 resend
  was idempotent (identical digest+frame), contradicting seeds 61/62/64.
  Tier B. Size S. Owns re-probe hunks beside `crates/endpoint/src/play.rs`
  (resend path only). Accept: 3 seeds x identical-digest+frame resend;
  divergent seeds name the first divergent tick or the probe closes the
  question. needs: PB51. (PLAYTEST65. PLAYTEST68: 4 clean seeds
  6500/6600/6700/6800, 6800 digest 3859107849197126949 x2 frame-stable;
  PLAYTEST69: 6900 clean, 5th seed.)
- [ ] PB64 playtest: arena600 frozen capture — frozen 600-tick 2-fighter
  fight capture on pinned seed 6800 bracketing first contact.
  Tier B. Size S. Owns tape hunks beside `crates/endpoint/src/play.rs`
  (TOOL_PLAY_RUN). Accept: `cargo test -p engine-endpoint --lib play`
  green plus tape hash pinned. needs: GP3. (PLAYTEST68.)
- [ ] PB65 playtest: axis-mapping cue — on-screen mx-sign legend explaining
  6600-herd versus 6700/6800-split in duel captures.
  Tier B. Size S. Owns capture-draw hunks in
  `crates/endpoint/src/play/play_session.rs` plus
  `crates/game/src/camera.rs` (pixels only, no sim change). Accept:
  `cargo test -p engine-endpoint --lib play` green plus
  `cargo test -p engine-game --lib camera` green. needs: GP3. (PLAYTEST68.)
- [ ] PB66 playtest: per-fighter identity and facing cue — f0/f1 labels plus
  facing wedge on step060/090 duel captures.
  Tier B. Size S. Owns capture-draw hunks in
  `crates/endpoint/src/play/play_session.rs` plus
  `crates/game/src/camera.rs` (pixels only, no sim change). Accept:
  `cargo test -p engine-endpoint --lib play` green plus
  `cargo test -p engine-game --lib camera` green. needs: GP3. (PLAYTEST68.)
- [x] ADV51a tools: docs-coverage gaming — empty `///` counts as Landed ae98ba8. Tick audit 2026-10-17: node --test tools/docs-coverage.test.mjs tools/showcase.test.mjs 13/13 (empty doc markers count undocumented; removed API fails).
  documented and deleting undocumented API passes as improvement
  (ADV51 wave 51, fixtures in `target/adv51/`). Tier B. Size S. Owns
  `tools/docs-coverage.mjs` hunks. Accept: `///`-only counts undoc;
  undoc-deletion FAILs/names removal. needs: none. (ADV51.)
  Wave 52: TICK ae98ba8 (checker PASS).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [ ] ADV51b tools: lock anonymous-release kills foreign lease — Progress 2026-10-17: tools/sprint-lock.mjs no longer exists (the standalone repo cut sprints and locks 2026-10-17), so the guard hunk and the LCK1 probe cannot be proven; the row looks moot, owner call to close it.
  bare `release` skips the foreign-holder guard (ADV51 wave 51,
  `tools/sprint-lock.mjs:87-93`). Tier B. Size S. Owns the guard
   hunk. Accept: foreign anonymous release refused, owned works, LCK1
   probe 0/12. needs: none. (ADV51.)
  Wave 52: TICK ae98ba8 (checker PASS).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [x] ADV51c tools: showcase frameless tamper bypass + clip escape — Landed ae98ba8. Tick audit 2026-10-17: tools/showcase.test.mjs frameless manifest fails without --allow-unhashed, tampered PNG fails, unsafe manifest.clip refused; node --test 13/13 with docs-coverage.
  check-only PASSes with no `frames` hashes + unsanitized
  `manifest.clip` basename (ADV51 wave 51; incident proposed).
  Tier A. Size S. Owns `tools/showcase.mjs` hunks. Accept: frameless
   needs `--allow-unhashed`, tamper FAILs both paths, `../` refused.
   needs: none. (ADV51.)
  Wave 52: TICK ae98ba8 (checker PASS; merged with ADV52a).
  LATER: parked 2026-10-03 (circus verdict 2: its own wave note says TICK with a commit that exists, but the box was never ticked, so builders re-verify finished work; the lead ticks it with its receipt); unpark by deleting this line.
- [ ] ADV55c perf: seal parser migration onto engine-core, parser-only. Progress 2026-10-17: crates/core/src/seal_key.rs landed and playtest eval.rs plus seeded_bug.rs delegate to engine_core::parse_seal_key; left: proof a9.rs:427-457 still holds local decode_seal_hex, parse_seal_key and seal_key_error, and seal_key_reason was not added.
  Tier A. Size S. Owns `crates/core/src/seal_key.rs` (add `seal_key_reason`) +
  `crates/proof/src/a9.rs:410-457` + `crates/playtest/src/eval.rs:1324-1365`
  + `crates/playtest/src/seeded_bug.rs:160-205` hunks (delete local copies,
  delegate to the core owner; core stays std-only, zero new deps, off-tick
  parsing only; perf `seal_row` HMAC deferred). Accept: proof/playtest/perf
  forgery suites green + `node tools/rust-quality.mjs --crate engine-proof`
  PASS (dup 0 on seal helpers) + golden x2 unchanged. needs: ADV55a.
- [ ] AQ2 arena-errors: derive `Eq` (+`Hash` where `Copy`) on Progress 2026-10-17: ForkError and DecodeError already derive Eq; ArenaError (integer variants only) still derives PartialEq without Eq and none of the three derive Hash.
  `ArenaError`/`ForkError`/`DecodeError` to match the combat error
  convention. Tier C. Size XS. Owns the three enum headers only.
  Accept: arena suite green + no other file touched + quality PASS.
  needs: none. (API-REVIEW wave 53.)
- [x] FB2 proof: version/cook receipt — stable prefab/scene/asset Landed 386ee01a.
  versions, migration fixtures, typed failed-import refusals,
  incremental cook bytes x2, reimport + project move with no
  reference loss. Tier B. Size S. Owns
  `crates/proof/src/fb_versions.rs` plus the
  `target/evidence/b/versions/` report. Accept: `cargo test
  -p engine-proof --lib fb_versions` green with the report
  reproducing x2; zero unlisted versions, zero lost refs; golden
  unchanged. Consumer: every v1 importer. needs: AS1, AS2, AS7,
  RC4. (planner-rows-r8.)
- [x] FB3 proof: ship-policy receipt — a template boots headless Landed 386ee01a.
  from packaging instructions verbatim, compat policy enforced by
  test, provenance manifest on every AI-made object, remap and
  accessibility basics pass. Tier B. Size S. Owns
  `crates/proof/src/fb_ship.rs` plus the
  `target/evidence/b/ship/` report. Accept: `cargo test
  -p engine-proof --lib fb_ship` green with the report
  reproducing x2; a template boots with zero coaching. Consumer:
  the unfamiliar developer. needs: TPL1, ED5, UIX1.
  (planner-rows-r8.)
- [ ] FB1 proof: dedicated-server fault-matrix receipt — N = 16
  dedicated server under documented loss, jitter, reorder and
  reconnect converges with full checkpoint equality; every
  mismatch carries an authority/replay diagnosis naming the
  culprit tick. Tier B. Size M. Owns
  `crates/proof/src/fb_transport.rs` plus the
  `target/evidence/b/transport/` report. Accept: `cargo test
  -p engine-proof --lib fb_transport` green with the report
  reproducing x2; 4 fault cells x2 runs, zero undiagnosed;
  golden unchanged. Consumer: the dedicated-server operator.
  needs: MP1, MP2, MP3, ON2. (planner-rows-r8.)
- [ ] FB4 proof: AI-pillars-in-sample receipt — the distilled
  integer `.e2nn` gladiator policy replays bit-exactly, every
  AI-made object carries a provenance chain, the museum loads,
  migrates and replays alpha tapes + scenes. Tier B. Size M.
  Owns `crates/proof/src/fb_pillars.rs` plus the
  `target/evidence/b/pillars/` report. Accept: `cargo test
  -p engine-proof --lib fb_pillars` green with the report
  reproducing x2; digest identical x2, chain complete, museum
  green; golden unchanged. Consumer: the shipped sample.
  needs: LW6, CT6. (planner-rows-r8.)
- [ ] FB5 proof: cycle + hour soak receipt — repeat
  load/unload/play cycles with resource accounting plus the PQ3
  hour read into one report: memory within 5%, no desync, no
  unresolved crash/data-loss, first bad minute named on failure.
  Tier B. Size M. Owns `crates/proof/src/fb_soak.rs` plus the
  `target/evidence/b/soak/` report (reads, never rewrites, the
  PQ3 soak report). Accept: `cargo test -p engine-proof --lib
  fb_soak` green with the report reproducing x2; a seeded leak
  names its first bad minute. Consumer: the v1 soak gate.
  needs: PQ3, MP2, DX4. (planner-rows-r8.)
- [ ] FB6 proof: hardware-matrix + baseline receipt — Q1 pixels,
  PF1 budgets and the GM3 package bound per hardware cell with
  an issue/repro process and a versioned baseline. Tier B. Size
  S. Owns `crates/proof/src/fb_matrix.rs` plus the
  `target/evidence/b/matrix/` report (pilot-measured cells land
  as receipt lines). Accept: `cargo test -p engine-proof --lib
  fb_matrix` green; every cell has a receipt line or a filed
  blocker; baseline versioned. Consumer: the release checklist.
  needs: Q1, PF1, GM3. (planner-rows-r8.)
- [ ] FB7 proof: stranger-dev workflow receipt — an unfamiliar
  developer runs create→edit→ship timed, blockers + time
  recorded, every fix lands in the flow not around it. Tier B.
  Size S. Owns `crates/proof/src/fb_workflow.rs` plus the
  `target/evidence/b/workflow/` report. Accept: `cargo test
  -p engine-proof --lib fb_workflow` green (harness + fixture
  run); the human run posts time with every blocker fixed or
  filed with a row. Consumer: the v1 workflow gate. needs:
  ED18, TPL1, UIX2. (planner-rows-r8.)
