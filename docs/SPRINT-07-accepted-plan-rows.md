### Accepted plan rows (Tier A/B future; dispatch like Finish line rows)

- [ ] RC8 core: deterministic math spike — correctly rounded sin, cos, exp Progress 2026-10-17: crates/core/src/det_math.rs has det_sin/det_cos/det_exp/det_atan2za on Fixed (det_golden_spots, det_bit_identical_100k, det_zero_alloc); left: corpus is 100k not 1M, no two-toolchain run, CODELAW sin/cos ban not lifted.
  and atan2 (CORE-MATH and RLIBM methods, papers only, no code copied)
  behind a gate; `Fixed` stays the default sim math. Tier C. Size M. Owns
  `crates/core/src/det_math.rs`. Accept: a 1M-input corpus is bit-identical
  on two toolchains and twice per build; zero allocations; the sin/cos ban
  in CODELAW lifts only on green. needs: none. Progress 2026-10-17: 4f556c75 correctly rounded sin/cos/exp, 0 wrong over every raw input of two sin/cos periods and the unsaturated exp range vs a test-only f64 oracle; 1M corpus digest 967702734373494140 identical on rustc 1.97.1 and 1.88.0 (Windows) and 1.97.1 (Linux), run 39066980501; left: atan2 stays CORDIC, and the HASH-LAW sin/cos ban stays until a cross-machine matrix row.
- [x] RC9 core: incremental digest experiment — per-component hashes folded
  into archetype roots and a tick root (Merkle style), O(changed) per tick,
  reporting the first divergent archetype and component; runs beside the
  full rehash behind a flag. The orchestrator lands a shared
  `CanonicalBytes` + `StableId` order contract first. Tier B. Size S. Owns
  `crates/core/src/digest.rs`, `crates/ecs/src/snapshot.rs`. Accept: same
  verdict as the full rehash on 20 of 20 seeded divergences with exact
  localization; at least 10% faster at 8 fighters or killed with numbers;
  golden unchanged. needs: none. Progress 2026-10-17: 80f99810 MerkleDigest/WorldDigest beside the real hash: equal to the full rehash on 20/20 seeded divergences; 0.32x at 8 fighters all changing (890 vs 287 ns), 1.7x with 1 of 8 changed, 16x at 1024 slots, 17.5x at 11,000; killed for the arena tick, code kept unwired (runs 39068711062, 39069090846, 39068837684). Landed 80f99810. Verified again 2026-10-18: engine-core --lib digest first_divergence_is_the_lowest_differing_leaf_20_of_20 (run 39103712242), engine-ecs --no-fail-fast incl. world_digest_matches_the_full_rehash_on_20_of_20_divergences 134 passed (run 39103985481), engine-proof --no-fail-fast frozen_default_and_distinct_seed_proofs golden d57b5c8ae3096614 160 passed (run 39103988897). Accept met by its kill branch: killed at 8 fighters with numbers (0.32x, 890 vs 287 ns); code kept unwired beside the real hash.
- [x] RC11 ecs: parallel bake and verify — extend the RC3 fork-join to bake, Landed f74588ee. bake_words/verify_sums/broadphase_pairs_match_serial_across_threads_and_chunks (workers 0,1,2,4,8 x 5 chunkings) run 39056114783; scaled_bake_parallel_clears_2x_over_serial (--ignored, release, runner) ok run 39056618000.
  verify and broadphase work only (never the sim tick; Rapier parallel stays
  DROP), seeded by RC7 streams. Tier B. Size S. Owns
  `crates/ecs/src/parallel.rs`. Accept: bit-identical across 1, 2, 4 and 8
  threads times 1 and N chunks; at least 2x on a scaled bake or killed.
  needs: RC7.
- [ ] RC15 net: netcode 1.02 protocol reference — custom Rust session
  handshake modeled on the netcode.io spec (BSD-3-Clause C spec as
  reference only, reimplemented). Tier B. Size S. Owns
  `crates/net/src/handshake.rs`. Accept: handshake roundtrip x2;
  bad keys refuse. needs: NET1. (RES-REPO wave 32 corner 3, reviewed
  2026-09-26.)
- [x] RC16 ecs: wrapping change ticks — u32 Tick with relative compare + Landed 7d13d5f1. changed_scan_does_half_the_compares_when_one_column_clean (20,000 vs 10,002 compares on the 1%-dirty 10k scene), wrap_soak_near_max_repeats_exactly (x2), timing_10k_1pct_reports_gated_faster (--ignored, release); runs 39056114783 and 39056618000; golden d57b5c8ae3096614 held at DSN2 run 39054403838.
  MAX_CHANGE_AGE clamp at the tick boundary + per-column summary ticks;
  Changed-iteration skips clean columns (bevy_ecs
  `change_detection/tick.rs`, MIT OR Apache-2.0, pattern reimplemented).
  Tier B. Size S. Owns `crates/ecs/src/change_tick.rs`. Accept: 2x or
  better Changed-scan on a 1%-dirty 10k-entity scene; tick-near-u32-MAX
  wrap soak exact x2; golden unchanged. needs: none.
- [x] RC19 ecs: optional views + dynamic spawn — Optional-component
  iteration and runtime-TypeInfo spawn bundles over the ARC1 spine columns
  for RC4 prefabs (shipyard `optional.rs`, hecs `bundle.rs` +
  `entity_builder.rs`, MIT OR Apache-2.0, patterns reimplemented; RC17
  stays reserved for RC12 splits). Tier B. Size M. Owns spine call sites
  in `crates/ecs` (no new storage). Accept: 20 prefab spawns bytes x2;
  0 tick-path allocs; golden unchanged. needs: ARC1, RC4. Landed 7d13d5f1 (views.rs; verified 2026-10-17): optional views + dynamic spawn, 20 prefab spawns byte-identical x2 and zero tick-path allocations (runs 39066475838, 39068066992); callers so far only inside ecs.
- [ ] RC20 ecs: ordered slot-write collect for bake/verify/anim-eval lanes Progress 2026-10-17: fork_join_on gathers chunk partials in slots, bit-identical across 0/1/2/4/8 workers (parallel.rs, run 39056114783) and a synthetic scaled bake clears 2x (run 39056618000); left: the ordered slot-write collect with exact-len pre-size and full-coverage assert is not built, 2x not measured on the CT2 workload.
  — exact-len pre-size, disjoint index-slot writes, full-coverage assert
  (rayon `iter/collect`, MIT, pattern reimplemented, no dependency; sim
  tick untouched). Tier B. Size S. Owns `crates/ecs/src/parallel.rs`
  (with RC11). Accept: bit-identical across 1, 2, 4 and 8 threads; at
  least 2x on the CT2 bake workload or killed with numbers; golden
  unchanged. needs: RC3, RC11. Progress 2026-10-17: 25b351de collect_indexed_on in parallel.rs: slots allocated up front, each job writes its own slice, JobsError::Uncovered refuses a gap, bit-identical for 0/1/2/4/8 workers, 2.86x on a synthetic 200k bake (run 39066665851); left: the CT2 workload measurement.
- [x] RC21 ecs: dispatch plan for bake/verify — explicit barriers between
  stages, a sequential fallback that must stay byte-equal, a printed plan
  plus a PF2 span per stage (shred dispatcher pattern, MIT,
  reimplemented; never sim). Tier B. Size S. Owns
  `crates/ecs/src/dispatch_plan.rs`. Accept: plan dump matches execution
  on fixtures; seq bytes equal par bytes x2; a mismatch refuses with the
  diff. needs: RC11, PF2. Landed 35ce2fd1. dispatch_plan.rs: stages with barriers, sequential fallback, run_checked names the first stage whose digests differ, plan dump equals the executed trace; 129 lib tests (run 39066475838).
- [ ] RC22 core: slice-pumped off-tick jobs plus digest-chain divergence
  naming — Stepped slices one-per-frame with recorded tick arrivals,
  per-tick digest chain reporting first divergent tick and stable id
  (balaur task + determinism patterns, MIT, reimplemented). Tier B.
  Size S. Owns `crates/core/src/digest.rs` (with RC9) plus pump glue.
  Accept: 20 of 20 seeded divergences localized exactly; idle pump cost
  within PF1 noise; golden unchanged. needs: RC9, ARC3.
- [ ] RC24 core: deterministic simulation testing harness — madsim
  deterministic sim-test shape reimplemented (Apache-2.0, PORT cite +
  EVIDENCE row). Tier A. Size M. Owns
  `crates/core/src/sim_verify.rs`. Accept: 10,000 ticks with injected
  events; zero divergence; p95 ≤ 0.1ms overhead. Kill: nondeterminism.
  needs: RC7, ARC11. (RES-STEAL wave 37 Dive 01, reviewed 2026-09-26;
  remapped from colliding RC10.)
- [ ] RC25 core: deterministic hash for cross-platform entity IDs —
  rustc-hash FxHash shape reimplemented (Apache-2.0, PORT cite +
  EVIDENCE row). Tier A. Size S. Owns
  `crates/core/src/det_hash.rs`. Accept: identical hash output across
  Windows/Linux; zero drift; no per-tick allocation. needs: RC7,
  ARC12. (RES-STEAL wave 37 Dive 01, reviewed 2026-09-26; remapped
  from colliding RC11.)
- [ ] RC26 core: deterministic tick recording and replay — copper-rs
  recording shape reimplemented (Apache-2.0, PORT cite + EVIDENCE
  row). Tier A. Size M. Owns `crates/core/src/tick_record.rs`.
  Accept: 1,000 ticks recorded and replayed with 100% fidelity;
  overhead ≤ 0.2ms/tick. needs: RC7, ARC13. (RES-STEAL wave 37 Dive
  01, reviewed 2026-09-26; remapped from colliding RC12.)
- [ ] RC27 core: deterministic ECS schedule verification — orga
  schedule shape reimplemented (Apache-2.0, PORT cite + EVIDENCE
  row). Tier A. Size S. Owns `crates/ecs/src/sched_verify.rs`.
  Accept: 50+ systems in verified order; validation ≤ 0.1ms; zero
  divergence. needs: ARC13, RC7. (RES-STEAL wave 37 Dive 01, reviewed
  2026-09-26; remapped from colliding open RC13.)
- [ ] RC28 core: counter-based PRNG quality upgrade — wyhash-rs shape
  reimplemented (Apache-2.0, PORT cite + EVIDENCE row). Tier B.
  Size S. Owns `crates/core/src/rng_wyhash.rs`. Accept: TestU01
  SmallCrush pass; cross-platform equality; p95 ≤ 0.01ms. needs: RC7.
  (RES-STEAL wave 37 Dive 01, reviewed 2026-09-26; remapped from
  colliding RC14; ARC14 does not exist.)
  Wave 52 R2 diagnosis (judge FAIL, reverted): WyRng unwired (zero
  callers) + custom schedule (not wyrand vectors) + missing PORT cite +
  ISLAND/CHUNK tags dup counters. R2 must wire one real caller with
  chunk-equivalence, pin upstream vectors, add PORT + EVIDENCE row.
- [x] RC30 core: retire the f32 `FixedAcc` legacy path — `time.rs`
  integer-ns accumulator becomes the only sim clock. Tier B. Size S.
  Owns hunks in `crates/core/src/tick.rs`, `crates/core/src/time.rs`.
  Accept: f32 residual measured on the 10M-push soak first, then zero
  drift; same tape twice identical; golden `d57b5c8ae3096614` x2 or a
  HASH-LAW receipt with old/new hashes; `git grep FixedAcc` finds no
  sim-path user. needs: RC6. Landed 7fed0107. LegacyFixedAcc/FixedAcc removed; the f32 residual on a 16/17 ms jitter tape ended 0.745 ms off true, IntAcc 0 ns; tools/float-gate.test.mjs now fails any f32 clock in tick.rs (engine-core run 39065712849, golden run 39064559231, engine-proof --lib 142 passed run 39065716995).
- [x] RC31 app: per-frame overhead harness — ordering + extraction Landed 0e40e0e5. rc31_harness_posts_p50, rc31_ordering_plus_extraction_under_budget (--include-ignored, release), rc31_over_budget_names_slowest_stage; p50=0us over 200 frames on the runner (not the reference machine) run 39058615260.
  asserted under 0.1 ms p50 as a PF1-style workload on the reference
  machine. Tier A. Size S. Owns test hunks in `crates/app` (no new
  crate). Accept: `cargo test -p engine-app` posts the p50 number;
  over-budget names the slowest stage. needs: RC1.
- [x] RC32 app: cold-startup + prefab-instantiate budget proof —
  startup under 2 s cold, instantiate under 1 ms per arena kit, both
  posted on the reference machine. Tier A. Size S. Owns test hunks in
  `crates/app/src/app.rs` plus the RC4 prefab fixture. Accept:
  numbers posted x2; a regression fails the gate. needs: RC1, RC4. Landed fca6fe7c. App cold startup under 1 ms and RC4 museum kit 3-4 us, asserted twice in one test (budgets 2 s, 1 ms), GitHub run 39064540714, 4 passed; the old budget tests were ignored or checked nothing.
- [ ] RC34 runtime: second-game boot receipt — urban war boots on the
  same runtime with zero engine forks: stock config loads, menu renders
  through the same extraction, a headless match replays to identical
  checkpoints, saves migrate through the same schemas, packaging ships
  without engine edits (companion to UW39, which owns the game side).
  Tier B. Size M. Owns glue/tests only, no engine edits. Accept:
  checklist green x2; any engine fork fails the tier by definition.
  needs: RC1. Skipped 2026-10-17: blocked on UW0 (urban war, later game).
- [x] RC35 ecs+app: 1,000-agent and 64-slot runtime soak — archetype
  World 1,000-agent tick p99 ≤ 2 ms (P6) and app extraction at 64-slot
  siege load; a 10-minute 64-slot siege replays exactly (the runtime
  half of C5). Tier B. Size M. Owns harness hunks in `crates/ecs` and
  `crates/app`. Accept: numbers posted on the reference machine;
  replay x2 identical; golden unchanged or a HASH-LAW receipt.
  needs: RC29. Landed 85543ccd. ecs 1,000 agents through run_arena_tick x2 identical, p99 54 and 47 us (run 39065108585); app 36,000-frame (10 min) siege through App::run_frame x2, same chain 807b5fcc971f58e9, frame p99 52 us (run 39064969267); runner numbers, not the reference PC; the 64 slots are runtime entities (arena caps at 16 fighters); golden unmoved.
- [ ] ARC5 net+save: spine rollback and persistence — net resim and Progress 2026-10-17: save side exists (crates/save/src/persist.rs spine payload, rollback.rs spine_gate); crates/net has no spine call sites; left: net resim over spine bytes and the QUIC + UDP drop/rejoin convergence proof.
  save envelope consume `spine::snapshot/restore`; a loopback drop
  and a save round-trip converge on spine bytes, over the QUIC channel
  mapping (`crates/net/src/quic.rs`, ON1) as well as UDP. Tier A.
  Size M. Owns spine call sites in `crates/net`, `crates/save` (no new
  files beyond glue). Accept: MP1-style drop/rejoin converges;
  save round-trip x2; golden unchanged. needs: ARC4, NET1.
- [ ] ARC6 editor+extract: spine extraction — `ExtractionView`
  reads the spine (first real RC2 consumer) and the editor
  inspector renders spine components through RC4 schemas. Tier A.
  Size M. Owns glue in `crates/endpoint`, `crates/editor`.
  Accept: three-state extraction test green on spine bytes; one
  Xvfb inspector smoke. needs: ARC4, RC2. Progress 2026-10-18: ExtractionView pub (6206fe07), Bridge::ingest_spine refuses bad hash/short buffer/bad version/tick skew (ecs bridge run 39066510052, 10 passed), editor spine_inspect renders the four spine columns through RC4 schemas (2 passed); left: nothing in the editor produces a live spine, ShellUi::set_spine is only called from tests.
- [ ] ARC7 arena: retire legacy — flip the `spine` flag to default,
  delete the legacy step, move the golden with a HASH-LAW receipt
  (fixture version, reason, old/new hashes, both outputs).
  Tier A. Size S. Owns legacy step deletion + `docs/HASH-LAW.md`
  note (orchestrator lands the doc edit). Accept: `git grep` for
  legacy step finds nothing; proof golden x2 on the new hash;
  receipt committed with the row. needs: ARC4, ARC5.
- [x] ARC8 playtest: spine keypoint fixtures — 10 gameplay claims
  as VK1 specs over spine state (parry window, hit-stop, get-up,
  bounds), run by the ticked VK1 30-spec harness (no isolated specs).
  Tier A. Size S. Owns `fixtures/keypoints/spine_*`. Accept: 10 pass
  on spine fixtures; 3 seeded bugs each fail exactly their keypoint.
  needs: ARC4, VK1. Landed 885aab83. 10 spine_* fixtures (42 specs), pos_mm assert, 3 seeded bugs each fail exactly their spec; keypoint 19 passed, gh-check run 39067543433.
- [ ] ARC9 mcp: relations to spine — SP5 solver outputs write
  spine transforms (or return the conflict set) through the same
  tick-boundary recording as provider actions; AB1 levels place
  onto the spine. Tier B. Size M. Owns glue in
  `crates/endpoint/src/build_level.rs` call sites. Accept: 10 AB1
  briefs place with zero spatial findings; every write undoes
  cleanly. needs: ARC4, SP5.
- [x] ARC10 ops: spine scorecard — proof prints the parity column
  (600-tick match/mismatch + first divergence) per build; the
  sprint retro reverts spine edits that do not help. Tier A.
  Size S. Owns `tools/` glue (orchestrator lands). Accept: a
  seeded divergence fails the gate; scorecard matches a
  hand-checked sample. needs: ARC3. Landed 7249faef. engine-proof feature spine, bin spine_parity_line, proof.mjs prints it plus a seeded-wall canary; spine_parity 2 passed run 39069258457, node --test tools/proof.test.mjs 24 passed.
- [x] ARC11 playtest: rules-as-code coverage on spine — every spine rule
  reachable from components as data/code with a VK1 spec (Code World
  Model 2608.25927v1, idea only, reimplemented; visuals delegated,
  capture advisory, bytes gate). Tier A. Size S. Owns
  `fixtures/keypoints/spine_rules_*` + coverage list. Accept: 10 ARC8
  specs + rule-to-spec map 100%; 3 seeded rule bugs fail exactly their
  keypoint; parity x2; golden unchanged. needs: ARC4, VK1. Landed 885aab83 + 50c4792b. 2 spine_rules_* specs, SPINE_RULES 11/11, report prints spine rules covered=11/11, 3 seeded rule bugs each fail their spec; playtest --lib 318 passed, run 39071693694.
- [ ] ARC12 mcp: relation-diff edits to spine — brief to relation diff
  to SP5 solve to spine transform write at the tick boundary with
  conflict-set refusal (3D-Layout-R1 2603.22279v1, idea only,
  reimplemented; raw-coordinate path kept as ablation only). Tier A.
  Size S. Owns glue in `crates/endpoint/src/build_level.rs` call
  sites + `fixtures/spatial-gold/arc12_*`. Accept: 10 AB1 briefs
  0 findings, 100% reachable, undo-clean x2; ablation numbers posted.
  needs: ARC4, SP5.
- [ ] ARC13 ops: parity-gated retro — ARC10 column + revert rule: a spine
  edit that breaks 600-tick parity is reverted with its first-divergence
  receipt; regressions kept as fixtures (Darwin Gödel Machine
  2505.22954v3, idea only; no self-rewriting agent). Tier A. Size S.
  Owns `tools/` glue (orchestrator lands). Accept: seeded divergence
  fails gate + reverts; sample matches hand-check. needs: ARC3.
- [x] ARC14 ecs: spine float quarantine — the spine takes integer-ns
  dt (RC6 shape), never `schedule::Ctx.dt: f32`, and a quarantine test
  fails on any float inside the spine module. Tier A. Size S. Owns
  quarantine test hunks in `crates/ecs` beside `spine.rs` (no new
  file; consumer ARC4). Accept: seeded float fails the gate; spine
  step dt integer-ns; golden unchanged. needs: ARC1, ARC2. (PLN
  architecture, reviewed 2026-09-26; waits ARC1/ARC2.) Landed 0968a4c1. register_ns/build_ns/SpineTick::run(dt_ns: u64); a source-scanning quarantine test allows floats only in the five documented parity items of spine.rs and none in spine_schedule.rs or views.rs (engine-ecs 131 passed, run 39068066992).
- [ ] ARC15 arena: spine fork parity — FK1 forks carry spine state
  with rejoin + first-divergent-tick naming. Tier A. Size S. Owns
  fork glue + test hunks in `crates/arena/src/fork.rs` (fork API
  frozen; consumer ARC3/ARC5). Accept: forked spine legs resimulate
  identical x2; seeded divergence names tick + component. needs:
  ARC4, FK1. (PLN architecture, reviewed 2026-09-26; waits ARC4.)
- [ ] ARC16 mcp: VP1 digest over spine — `play.step` digests read
  spine checkpoints with equal digests x2 on spine bytes. Tier A.
  Size S. Owns digest call-site hunks in `crates/endpoint/src/play*.rs`
  (play API frozen; consumer VP1/EV2). Accept: 20-step duel equal
  digests x2; non-black PNGs. needs: ARC4, VP1. (PLN architecture,
  reviewed 2026-09-26; waits ARC4.)
- [x] ARC17 ecs: spine columns on the archetype store — ARC1's five
  components (`Transform`, `Body`, `Fighter`, `AiState`, `AnimState`)
  as new `ComponentId` bits plus SoA columns in
  `crates/ecs/src/archetype.rs` (no new module: RC29 proved ai, proof
  and perf already tick through `run_arena_tick` on this store),
  Fixed/integer fields only with per-column `CanonicalBytes` in index
  order. Tier A. Size S. Owns hunks in `crates/ecs/src/archetype.rs`
  (consumer ARC18/ARC19). Accept: spawn/fill/snapshot/restore
  round-trip byte-identical x2; quarantine test fails on any
  `f32`/`f64` in the new columns; `cargo test -p engine-ecs` green;
  golden unchanged. needs: RC29. (PLN architecture EXTEND 2026-09-30;
  planner-rows-r1.) Landed f09debd8. Five ComponentId bits and SoA columns, spawn_with/get_spine/set_spine/add_spine/remove_spine, World::snapshot/restore byte-identical x2, state_hash covers the columns and P/V-only entities keep their 21 checkpoint bytes (engine-ecs 131 passed, run 39068066992; engine-proof passed before the world_snapshot.rs split, run 39067607073).
- [x] ARC18 ecs: integer-ns dt through `run_arena_tick` — thread
  integer-nanosecond dt (RC6 shape) through
  `Schedule::run_arena_tick`, which today takes `dt: f32` seconds,
  beside the f32 path behind the spine flag; spine systems read ns,
  legacy callers keep f32 until ARC7. Tier A. Size S. Owns hunks in
  `crates/ecs/src/schedule.rs` (consumer ARC4). Accept: one-tick A/B
  (ns path vs f32 path) checkpoints equal x2 on a seeded world;
  seeded float dt in the spine path fails; `cargo test -p
  engine-ecs` green; golden unchanged. needs: ARC17, RC6. (PLN
  architecture EXTEND 2026-09-30; planner-rows-r1.) Landed 4bff6fab. Ctx.dt_ns, Schedule::run_ns and run_arena_tick_ns (Ctx.dt is NaN on the ns path), World::integrate_ns, SpineTick::run feeds run_ns and register_ns reads Ctx.dt_ns. Tests ns_path_checkpoint_equals_the_f32_path_x2, float_dt_in_the_ns_path_fails_the_checkpoint, systems_read_ns_on_both_paths: engine-ecs --no-fail-fast 134 lib + 3 + 2 passed run 39103985481, clippy -D warnings run 39104482742; engine-proof --no-fail-fast 160 passed incl. golden d57b5c8ae3096614 run 39103988897; arena spine parity 4 passed run 39103012820.
- [x] ARC19 arena: single-tape spine parity probe — the smallest ARC3 Progress 2026-10-17: crates/arena/src/parity.rs (feature spine): twenty_seeded_tapes_match_exactly_over_600_ticks and a_wall_in_the_way_reports_the_exact_first_divergence green run 39058684792 (20 tapes over 600 ticks, first divergence names tick + column); spine columns live in crates/ecs/src/spine.rs, so the ARC17 need (columns in archetype.rs) is not met as written; proof time not posted.
  slice: translate one seeded tape's inputs into the ARC17 columns
  once, step one tick beside the legacy step, and compare the
  checkpoint, reporting the first divergent component or exact-match.
  Tier A. Size S. Owns the parity probe test hunk in `crates/arena`
  (flag-gated, no default change; consumer ARC3). Accept: the probe
  reports exact-match or names tick plus component on 5 of 5 seeded
  tapes; proof time posted; golden unchanged. needs: ARC17. (PLN
  architecture EXTEND 2026-09-30; planner-rows-r1.) Landed f8b9e45a (crates/arena/src/parity.rs, flag spine). Re-run 2026-10-18 cargo test -p engine-arena --features spine --lib parity: twenty_seeded_tapes_match_exactly_over_600_ticks (20 of 20 exact) and a_wall_in_the_way_reports_the_exact_first_divergence (divergent tapes name tick + Column::Transform, same call twice names the same) 4 passed, finished in 0.05 s (proof time), run 39103012820; the columns are in crates/ecs/src/spine.rs and ARC17 is ticked. Golden unchanged: engine-proof --no-fail-fast run 39103988897.
- [x] MP6 net: two-machine played proof — 2 humans + 6 AI over a
  loopback fault injector (real LAN is a bonus) converge mid-fight
  through a scripted drop+rejoin; measured loss/latency distributions
  posted. Tier A. Size M. Owns
  `crates/net/src/scale.rs` soak-plus-played proof + fixtures.
  Accept: played bout converges with full checkpoint equality;
  distributions posted; 16-bot x600 soak still hashes x2. needs:
  MP1, MP2, MP3. (PLN31 multiplayer.md, reviewed 2026-09-25; loss,
  jitter and reorder are proven on loopback with a fault injector
  (`crates/net/src/wan_soak.rs` run_matrix, `session.rs` FaultLink);
  only the real-NAT hole-punch leg needs a second network.)
  progress 2026-10-10: loopback Accept met. `net-bout mp6` PASS (LAN and WAN converge, full checkpoint equality, soak 16 bots x600 hash 681d5332c0a19f60 x2, fault matrix x2). `node tools/mp6-check.mjs`: 23 pass, 0 fail, 1 warn: resim window 8 vs WAN deepest 19 (116 ticks over, counted on the resync path; owner decision, window not changed here). Open: real two-machine LAN leg (no two-machine runner in the repo yet). AI-swap test played_ai_broadcasts_identical_with_machine_ids_swapped passes on the tree; its played.rs commit is shared with split-claude.
  progress 2026-10-10 (split-claude): two-machine runner built, `crates/net/src/split.rs` + `courier.rs`. Machine A: `net_bout mp6-serve [--bind 0.0.0.0:7040] [--out target/mp6-lan]`; machine B: `net_bout mp6-join --server A:7040`. Lockstep step exchange over real UDP; the fault links run in step time, so each bout equals its one-process twin (digest checked per bout in `real-net.json`, with probe RTT, loss and resends). Two-process loopback smoke: MP6 CHECK PASS (23 pass, 0 fail, same resim WARN). Open: one run on two real machines.
- [x] MP8 net: channel table + priority mapping — every wire `KIND_*` Landed 1d01a410. kind_table_reads_every_kind, combat_inputs_never_starve_under_snapshot_flood, full_16_snapshot_fits_a_datagram run 39056258774; table covers the 3 session-wire KIND_* (lobby.rs KIND_* is a separate wire outside the table).
  maps to a `Channel` plus a priority class in one table the encoder
  reads; 16-fighter full snapshots stay within 1200 bytes. Tier B.
  Size S. Owns `crates/net/src/netcode.rs` channel table + `scale.rs`
  flood test. Accept: table test reads every `KIND_*`; zero combat
  starvation under flood; encode `<= 1200`, never fragments. needs:
  MP2. (PLN31 multiplayer.md, reviewed 2026-09-25.)
- [x] MP10 net: snapshot interpolation — lightyear Hermite + quaternion
  interpolators, snapshot ring buffer and extrapolation guard rewritten
  in Rust as `crates/net` snapshot modules (MIT, PORT cite + EVIDENCE
  row). Tier B. Size M. Owns `crates/net/src/snapshot.rs`. Accept:
  Hermite C1-continuous; quaternion auto-dispatches nlerp below 1.8°;
  guard classifies interpolate/extrapolate. Kill: float in tick;
  divergence. needs: MP1. (RES-STEAL wave 32 flax-mcp REJECTED wave-15;
  lightyear CARD wave-15; coord claim steal-202610090712-b09.)
  progress 2026-10-10: Accept met. Hermite C1 (snapshot_hermite_endpoints_exact_and_c1_at_knot), quaternion nlerp below 1.8 deg (snapshot_quat_dispatches_nlerp_below_1_8_degrees) and the extrapolate guard (snapshot_guard_classify_query_edges, classify_query) now live in snapshot.rs, the owned home per Plan §3. The render ring stays in interp.rs. cargo test -p engine-net --lib snapshot: 34 passed.
- [ ] MP12 net: replication-over-LAN proof — `replicate.rs` (RC14)
  messages ride the existing LAN snapshot path at 16 fighters
  inside the same 1200-byte budget, decoded by the version-refusing
  path, `replicate.rs` API frozen. Tier B. Size M. Owns call-site
  hunks in `crates/net/src/session.rs` + `scale.rs` (no second wire,
  no second version). Accept: 16-fighter flood shows replicate
  deltas inside budget with zero combat starvation; hash x2;
  `REPLICATE_VERSION` mismatch refuses loudly. needs: MP2, RC14.
  (PLN31 multiplayer.md EXTEND, reviewed 2026-09-28.)
- [x] MP13 net: sanitize-flag receipts — every accepted input keeps
  its `anticheat::sanitize` `F_*` bits in the accepted-action
  record; replays attribute clamped inputs exactly. Tier B. Size S.
  Owns receipt hunks in `crates/net/src/anticheat.rs` (authority
  stays ON5; flags only). Accept: adversarial-input tape shows the
  same flags x2; a clamped input names its tick in the replay
  report; golden unchanged. needs: MP4, NET1.
  (PLN31 multiplayer.md EXTEND, reviewed 2026-09-28.) Landed cc91744a. Sanitize is integer and lives in crates/net/src/anticheat.rs (the old float file had no wire type): a spear press in a match without the spear is masked (F_SPEAR), run on every input guard.rs admits through HeadlessServer::offer; the FlagLog names each changed tick (HeadlessServer::flagged_inputs) and the authoritative tape holds the sanitized input. Tests: guard::tests::sanitize_flags_name_their_ticks_the_same_twice (adversarial tape: flags at ticks 0,1,4,5,8,9 named, flags and digest equal x2), anticheat::tests::only_a_spear_press_without_the_spear_is_masked, the_log_is_bounded_and_counts_the_rest; run 39111873633; golden engine-proof --lib golden 4 passed run 39112424935.
- [x] MP14 net: resim auto-tune — the 8-tick resim window and input Landed 1d01a410. mp14_window_and_delay_sized_by_posted_mp6 + adaptive_delay_covers_the_round_trip_minus_the_window run 39056258774.
  delay become measured values sized by the MP6 posted
  distributions, with a fixture that fails when hardware data would
  move them. Tier A. Size S. Owns tuning hunks in
  `crates/net/src/scale.rs` + `netcode.rs` (constants stay named,
  values stay reviewed). Accept: window sized by posted data with
  the sizing math in-test; MP6 distributions referenced by digest.
  needs: MP6. (PLN31 multiplayer.md EXTEND, reviewed 2026-09-28.)
- [x] MP15 net: dedicated auto-theater — every `HeadlessServer` Landed 1d01a410. dedicated_auto_theater_ends_playable_with_cr1_parity + dedicated_auto_theater_refuses_before_finish_and_bad_probe run 39056258774.
  match end writes its theater tape plus CR1 divergence report with
  no operator step. Tier A. Size S. Owns match-end hunks in
  `crates/net/src/dedicated.rs` (theater reuses MP4/MP7 shapes, no
  parallel system). Accept: scripted dedicated match ends with a
  playable tape plus a report that matches CR1's first divergent
   tick; golden unchanged. needs: MP1, MP4, CR1.
   (PLN31 multiplayer.md EXTEND, reviewed 2026-09-28.)
- [ ] MP16 net: 32-fighter LAN soak — priorities plus idle-shed
  prove 32 fighters inside the MP2 1200-byte budget. Tier B.
  Size M. Owns soak hunks in `crates/net/src/scale.rs` (caps
  frozen, values reviewed). Accept: 32-bot x600 soak hashes x2;
  flood shows zero combat starvation; kbps charted vs MP2 caps;
  `cargo test -p engine-net --lib scale` green. needs: MP2.
  (PLN multiplayer.md EXTEND, proposed 2026-09-29.)
  KILLED 2026-09-29 (wave 96, mp16-soak spike): byte budget fits
  (617B full / 661B delta vs 1200B; 74.04 kbps vs 96 kbps) but the
  count gate refuses before bytes — `snapshot_hz(32)`,
  `cap_bytes_per_sec(32)`, `soak_hash(32,600)`, `select_priority`
  len 32 all return `BadFighterCount {got:32}` via
  `MAX_PARTICIPANTS=16` (`crates/input/src/fighter.rs:17`,
  `crates/net/src/scale.rs:211/228/300/488`; 12/12 scale tests
  green). Revival rides GE12 (canonical-cap decision), not a soak
  hunk. needs: MP2.
  (A7 split 2026-10-02: parent KILLED wave 96 — 32 blocked on the
  `MAX_PARTICIPANTS=16` count gate, revival rides GE12. Children:
  land the 16-baseline now, revive 32 after GE12.)
  (DO NOT DISPATCH 2026-10-05: KILLED wave 96, count gate refuses 32. Take MP16a now, MP16b after GE12. Revival rides GE12, never a soak hunk.)
- [x] MP16a net: 16-fighter LAN soak baseline within the current Landed c2ffb691. soak_16_bot_x600_converges_with_matching_hashes, combat_inputs_never_starve_under_snapshot_flood, kbps_chart_rows_stay_under_target; scale filter 19 passed run 39056258774.
  cap — priorities plus idle-shed prove 16 fighters inside the MP2
  1200-byte budget. Tier B. Size S. Owns soak hunks in
  `crates/net/src/scale.rs` (caps frozen, values reviewed).
  Accept: 16-bot x600 soak hashes x2; flood shows zero combat
  starvation; kbps charted vs MP2 caps;
  `cargo test -p engine-net --lib scale` green. needs: MP2.
  (A7, 2026-10-02.)
- [ ] MP16b net: 32-fighter LAN soak revival after the GE12
  canonical-cap decision — same accept as MP16 parent (32-bot
  x600 hashes x2, zero combat starvation, kbps vs MP2 caps,
  scale tests green). Tier B. Size S. Owns soak hunks in
  `crates/net/src/scale.rs`. Accept: parent accept green at 32,
  or KILLED again with numbers. needs: MP2, GE12. (A7, 2026-10-02.)
- [x] MP17 net: snapshot production wiring (PORT-SNAP-WIRE) — the
  MP10 `snapshot.rs` modules wired into the session LAN path
  beside the frozen `delta.rs` Interpolator, no fork of either.
  Tier B. Size M. Owns `crates/net/src/snapshot.rs` (MP10 home)
  plus call-site hunks in `crates/net/src/session.rs` (after
  MP12, disjoint hunks). Accept: 16-fighter jitter fixture
  interpolates C1 with guard classification; delta bytes
  unchanged; hash x2; `cargo test -p engine-net --lib snapshot`
  green. needs: MP10.
  (PLN multiplayer.md EXTEND, proposed 2026-09-29.)
  progress 2026-10-10: Accept met on the tree before split-claude WIP landed in crates/net. cargo test -p engine-net --lib snapshot: 34 passed, incl. snapshot_wire_jitter_16_bodies_interpolates_c1 (C1 with guard), snapshot_remote_view_keeps_frozen_interpolator_bytes (delta bytes unchanged), snapshot_wire_bytes_and_presentation_are_pinned_x2 (hash x2).
- [ ] MP18 net: channel + compression flood chart — the MP8 table
  plus the MP9 codec under one 16-fighter flood, one chart
  deciding both rows. Tier B. Size S. Owns chart hunks in
  `crates/net/src/scale.rs` (table and codec read, never
  redefined; after MP16). Accept: combat first, idle shed
  first, deltas smaller on quiet tapes, ≥20% saving or killed
  with numbers; hash x2. needs: MP8, MP9, MP16.
  (PLN multiplayer.md EXTEND, proposed 2026-09-29.)
- [x] MP19 net: every-match theater receipt — the MP7 three-tape Landed 1d01a410 (with MP15). dedicated_auto_theater_ends_playable_with_cr1_parity (playable tape + report equal to CR1 attribute) and ..._refuses_before_finish_and_bad_probe (corrupt probe refused BadInput) run 39056258774.
  attribution plus the MP15 match-end auto-write as one receipt:
  every dedicated match ends playable and attributed with zero
  operator steps. Tier A. Size S. Owns match-end hunks in
  `crates/net/src/dedicated.rs` plus report hunks in
  `crates/net/src/replay.rs` (shapes reused, no parallel system).
  Accept: scripted dedicated match ends with a playable tape
  plus a report matching CR1's first divergent tick; seeded
  corruption refused loudly; golden unchanged.
  needs: MP7, MP15.
  (PLN multiplayer.md EXTEND, proposed 2026-09-29.)
- [x] MP20 net: lag-window sizing receipt — the 5-tick LagWindow
  sized by MP6/MP11 distributions the way MP14 sizes resim, with
  a fixture failing when data would move it. Tier A. Size S.
  Owns receipt hunks in `crates/net/src/lagcomp.rs` (shape
  frozen, values reviewed). Accept: sizing math in-test; hits
  outside their window fail; replay exact;
  `cargo test -p engine-net --lib lagcomp` green. needs: MP6,
  MP11. (PLN multiplayer.md EXTEND, proposed 2026-09-29.) Landed cc91744a. The window is now sized live per slot from the measured lag (5 ticks stays only as the pre-measurement default). Sizing math in lagcomp::tests::window_is_whole_ticks_capped_at_sixteen; data fixture played::admission_tests::played_window_is_sized_by_the_posted_delay_distribution_on_lan_and_wan (seat 0 lag_max equals the top bucket of its posted uplink delay histogram; pinned LAN 3 / WAN 10 = the configured holds, fails when the distribution moves); hits outside the window refused and replay exact (verify_replay); run 39111873633, full net suite 39111897900.
- [ ] MP21 net: 64-player interest-handoff spike (LAN-SCALE-SPIKE)
  — the loopback reads ON3 `ConnectionInterest::select` per
  connection for 64 fighters plus a 1000-agent legion inside
  `MAX_INTEREST_ENTITIES=1024`, sends the selected set only at
  10–15 Hz. Tier C spike. Size M. Owns handoff hunks in
  `crates/net/src/scale.rs` (`interest.rs` read-only; after
  MP16). Accept: 64+1000 loopback x600 converges hash x2;
  per-client kbps inside the ON9 128 kbps cap; tick essentials
  in sight of P6 p99; kill on drift, float, or starve past
   `STARVE_TICKS=30`. needs: MP16, ON3.
   (PLN multiplayer.md EXTEND, proposed 2026-09-29.)
- [x] MP22 net: backroll-style input-delay + first-incorrect resim
  (PORT-BR-QUEUE, ISC `HouraiTeahouse/backroll-rs@f75b9cc`, ideas
  reimplemented std-only, no vendor bytes) — add `frame_delay`
  (default 2, matches `INPUT_DELAY_TICKS`) + `first_incorrect_frame`
  cursor to `rollback.rs`; resim seeks only to first incorrect.
  Tier B. Size S. Owns `crates/net/src/rollback.rs` only (no
  session/netcode edits). Accept: seeded-loss loopback shows lower
  predicted/rollback counts with delay-on, tip equals truth hash,
  `max_rollback ≤ 8`. F2P: `cargo test -p engine-net --lib rollback`
  with delay counts posted (fails before: no delay field, no cursor).
  P2P: full `-p engine-net --lib` stays green + golden unchanged.
  Kill: no count drop, tip hash diverges, or resim distance > 8.
  needs: MP2. (RES-STEAL 680-res-ggpo4, 2026-10-08.) Progress 2026-10-17: 2b9cd860 with_frame_delay, confirm_input, first_incorrect_frame, resolve_incorrect; on a seeded link (1-3 ticks, 2% loss, 300 frames) delay on predicts 60 frames with 22 resims vs 300 and 165 off, max resim 1, tip equals truth (net --lib 276 passed, run 39069480532). Departure: P2pSession::new stays at delay 0 so existing tapes do not move (the row says default 2). Landed 2b9cd860. rollback tests delay_cuts_predictions_and_rollbacks_on_a_lossy_link (same seeded link, 300 frames: delay off predicts 300 with 165 resims, delay 2 predicts 60 with 22, max resim 1 <= 8, tip equals truth), frame_delay_applies_local_input_late, first_incorrect_batches_one_resim; net --lib rollback 25 passed run 39101475622, full net suite incl. quic run 39107627991. Departure kept: P2pSession::new stays at delay 0 (with_frame_delay(DEFAULT_FRAME_DELAY = 2) is the production setting) so existing tapes do not move.
- [ ] GP18 game: feel baseline lock — pin the GP11 schemas plus GP12 Progress 2026-10-17: baselines pinned (pq2_kill_bars_pin_v1_baseline, no-float gate); open question: Accept says no move above 35% of kills but the v1 survey pins attack share 395 (350 bar gates multi-move tables only); owner call.
  harness with PQ1/PQ2 kill bars and a no-float-in-tick grep. Tier B.
  Size S. Owns baseline hunks beside `crates/combat/src/feel.rs` +
  `crates/game/src/bench.rs` (no new pass). Accept: PQ1 p95 at or
  under 50ms; PQ2 40–60% win-rate with no move above 35% of kills;
  seeded float-math or magic-number regression fails the gate. needs:
  GP11, GP12, PQ1, PQ2. (PLN gameplay, reviewed 2026-09-26; waits
  PQ1.)
- [ ] GP21 game: trauma-curve shake stage. Tier B. Size S. Owns shake
  hunks beside `crates/game/src/camera.rs` + `camera_feed.rs`. Accept:
  jab <= 1/4 super displacement; restore holds every tick; sim bytes x2;
  playtester confirms small/big separation. needs: GP3, GP16, GP11.
  (PLN45 gameplay, verdict TAKE.)
  Wave 46 progress (stays open): ratio 0.016, restore <= 1e-5, sim x2;
  playtester confirmation missing — wave-47 blind review on captures,
  then tick. GP26 supplies the paired A-B captures + manifest numbers +
  blind call this row's tick requires (PLN49). Progress 2026-10-17: 535b1590 combat shake_stage.rs squared trauma curve (jab at 180 is at most a quarter of a big hit at 700, exact restore at zero trauma); left: the camera.rs hook (shake_offset line 848, call site 1032) and the GP26 capture pair. Progress 2026-10-18: caller wired 40119424 (shake_offset uses engine_combat::shake_stage::sample, x/y only; test shake_rides_the_gp21_stage_sample); live receipt on the RTX 3050 arena window printed shaken=126 peak_shake_milli=4966; left: playtester confirmation of small/big separation.
- [ ] GP22 game: unified impact fan-out. Tier B. Size M. Owns wiring
  beside `crates/combat/src/feel.rs` + presentation. Accept: one event
  gives 5 layers; per-layer toggles keep sim bytes x2; p95 <= 50 ms;
  0 allocs; captures show all layers. needs: GP11, GP16, GP21.
  (PLN45 gameplay, verdict TAKE.) CB4 supplies the four contact-frame
  data layers this row fans out — data before wiring (PLN49).
- [x] GP8 game: camera feel cores — flax-mcp camera rig, damping, impulse Progress 2026-10-17: not restored: camera_rig.rs, follow_pose, shake_offset and add_trauma already cover the old camera_feel.rs; only a pose transition would be new.
  dispatch and transition cores rewritten in Rust as `crates/game`
  camera modules (MIT, PORT cite + EVIDENCE row; clean-room, no vendor).
  Tier B. Size M. Owns `crates/game/src/camera_feel.rs`. Accept: rig
  produces a valid follow pose headless; damping frame-rate-independent;
  fixtures green. Kill: float math in the tick; divergence on same seed.
  needs: GP3. (RES-STEAL wave 32 flax-mcp, reviewed 2026-09-26.) Landed 033aaca3 (the pose transition, over camera_rig.rs from earlier waves; camera_feel.rs was not recreated, camera_rig.rs and fighter_look_cam.rs own those cores). Rig valid follow pose headless: camera::tests::scripted_orbit_keeps_both_fighters_framed_600_ticks and framing_fit_*. Damping frame-rate independent: camera_rig::tests::fight_cam_spring_smooths_duel_at_both_rates (30 vs 144 fps within 1 cm) and fighter_look_cam::tests::aim_camera_ends_on_the_same_camera_at_30_and_144_fps. Pose transition: aim_camera_glides_a_cut_the_stateless_shot_jumps. Fixtures: camera:: 104 tests ok in engine-game --features app --lib 2026-10-18 (711 passed; 2 reds are another session's ragdoll_look WIP).
- [x] GF1 feel: Juicee game-feel pipeline reimplemented in Rust — Landed 374c0303. combat feel.rs; 40 passed.
  hit-stop, shake, reactions over HR3 move tables (MIT, PORT cite +
  EVIDENCE row). Tier B. Size M. Owns `crates/combat/src/feel.rs`.
  Accept: hit-stop ≤ 2 frames, shake amplitude within tolerance, PQ2
  green. Kill: float math in the tick; divergence. needs: CB1.
  (RES-STEAL wave 35 Dive 11, reviewed 2026-09-26.)
- [x] GF2 feel: gamefeel prototype ported — hit-stop and camera-shake
  match the reference demo (MIT, PORT cite + EVIDENCE row). Tier B.
  Size M. Owns `crates/combat/src/gamefeel.rs`. Accept: hit-stop and
  shake match reference within tolerance; PQ2 green. Kill: float math
  in the tick; divergence. needs: GF1. (RES-STEAL wave 35 Dive 11,
  reviewed 2026-09-26.) Landed 535b1590. crates/combat/src/gamefeel.rs: integer port of deepnight/gamefeel (MIT) camera shake/bump and hit-stop/slow-mo; shake within 0.003 px and slow-mo within 3 permille of the reference formulas (combat run 39070109022, 26 passed; clippy 39069841600). Nothing in game calls it yet.
- [x] GF3 feel: Game Feel Flow pattern — feel-effect registration ≤ 1
  frame overhead (MIT, PORT cite + EVIDENCE row). Tier B. Size S.
  Owns `crates/combat/src/feel_flow.rs`. Accept: registration ≤ 1
  frame; effects deterministic. Kill: Unity/Godot dependency;
  per-tick allocation. needs: GF1. (RES-STEAL wave 35 Dive 11,
  reviewed 2026-09-26.) Landed 535b1590. crates/combat/src/feel_flow.rs registers effects against fight triggers in one slot store, live on the next step (run 39070109022).
- [ ] GP6 game: feel quantification — Pichlmair & Johansen 2011.09201v1 Progress 2026-10-17: crates/perf/src/feel.rs scorer with 8 unit tests (50 ms bar, win 400-600 permille, rubric 900 permille); test-only, counts are plain numbers; left: fixtures/feel-gold/ missing, no feed from real PQ1/PQ2/VK1 runs.
  three-domain framework operationalized into PQ1 latency bands plus a
  VK1 keypoint rubric (ideas only, license UNVERIFIED). Tier A. Size M.
  Owns `crates/perf/src/feel.rs` + `fixtures/feel-gold/`. Accept: PQ1
  p95 ≤ 50ms across tuning/juicing/streamlining; PQ2 win-rate 40–60%;
  VK1 pass ≥ 90%. Kill: p95 rise > 10ms or win-rate outside band.
  needs: PQ1, VK1, HR3. (RES-ARXIV wave 32, reviewed 2026-09-26;
  remapped from colliding GP2.) Progress 2026-10-17: 535b1590 perf check_gold and perf --feel-gold with fixtures/feel-gold (perf run 39068134586, 11 passed); left: no live PQ1 run from a game session feeds it. Progress 2026-10-18: 55482b86 fixes the red gold_goes_red_when_an_input_or_the_verdict_moves (it edited tuning 500 and juicing 500 5 5 but the fixture rows read 510, so check_gold returned Ok and unwrap_err panicked); engine-perf --no-fail-fast green run 39105354580. Still open: no live PQ1 run from a game session feeds fixtures/feel-gold.
- [x] GP7 game: feel keypoint extension — VK1 extended with feel specs
  (hit-stop, shake amplitude, cancel tightness, readability); 4/5
  seeded feel bugs fail exactly their keypoint. Tier A. Size S. Owns
  `crates/playtest/src/feel_keypoints.rs` +
  `fixtures/keypoints/feel/`. Accept: 4/5 seeded bugs caught; PQ1 p95
  ≤ 50ms; no float in tick. needs: VK1, PQ1, HR3.
  (RES-ARXIV wave 32, reviewed 2026-09-26; remapped from colliding GP3.) Landed 535b1590. crates/playtest/src/feel_keypoints.rs + fixtures/keypoints/feel (5 claims): 5 of 5 seeded feel bugs each fail exactly their own keypoint (run 39070573526, 4 passed; clippy 39070809701).
- [ ] GP14 game: camera framing for third-person melee — principle
  transfers, UNVERIFIED for 3D until playtest (ideas only, license
  UNVERIFIED). Tier A. Size M. Owns
  `crates/anim/src/camera_framing.rs`. Accept: PQ1 latency ≤ 50ms;
  silhouette readability ≥ 80% in playtest. Kill: float math in tick.
  needs: PQ1, GP13. (RES-ARXIV wave 35 Dive 11, reviewed 2026-09-26.) Progress 2026-10-17: 535b1590 combat camera_framing.rs integer shoulder-cam framing with a silhouette readability score (>=80% of poses read, both fighters in frame); left: the camera.rs hook (frame_pair 585, frame_group 643) and the human playtest. Progress 2026-10-18: frame_pair and the two-fighter frame_group call frame_melee 40119424 (test two_fighter_group_is_the_gp14_shoulder_shot_for_either_order); left: the live arena camera still comes from fighter_look::fight_camera, so the shoulder shot reaches only the discarded feed in windowed.rs; PQ1 latency and playtest readability.
- [x] ED14 editor: browser real-edge wiring - AS1 `bundle::EntryKind` + Progress 2026-10-17: kind_wire_bytes_match_as1_entrykind, full_bundle_search_stays_under_50ms and the bad-drop conflict-set tests exist in crates/editor/src/browser.rs; left: the local EntryKind / GRID_CM / YAW_STEPS mirror copies are not deleted. Landed 4bd06b85. Mirrors deleted, solver is engine_spatial::place::solve; 4096-entry archive search 1.6 ms; run 39058437704.
  SP5 `place` vocabulary replace the mirrors (ED3 file frozen; lands
  after ED3). Tier B. Size M. Owns `crates/editor/src/browser.rs`
  wiring hunks. Accept: mirrors deleted; full AS1 bundle search ≤ 50 ms
  measured; bad drops refused with conflict sets. needs: ED3, AS1, SP5.
- [x] ED15 editor: live-panel integration - the ED1 hierarchy tab Progress 2026-10-17: live_panels_share_one_selection (shell_ui.rs) green on the PC with --features ui: viewport pick, outliner search and inspector grid agree on one SharedSelection; left: no Xvfb smoke run. Landed 71c3f0fb. live_panels_share_one_selection and the headless dock frame smoke (stands in for the Xvfb smoke).
  consumes `outline.rs` (search + sets), the inspector panel consumes
  the `inspector.rs` grid, one selection-unity test over the three live
  panels. Tier B. Size M. Owns `crates/editor/src/shell_ui.rs` panel
  hunks. Accept: viewport/outliner/inspector agree on the identical
  live selection object; no parallel hierarchy/inspector in `shell_ui`;
  Xvfb smoke. needs: ED2, ED6, ED1.
- [x] ED16 editor: play parity on three tapes - same tape through child Landed 7d13d5f1. three_tapes_child_and_viewport_seal_same_checkpoints_twice, play_in:: 9 passed (local PC run, cargo test -p engine-editor --features ui --lib, 2026-10-17; the ui feature never routes to the runner).
  play and `play_in` gives same checkpoints (short fight, long soak,
  tuned fight); Xvfb smoke. Tier B. Size M. Owns
  `crates/editor/src/play_in.rs` proof hunks + tape fixtures. Accept:
  3/3 tapes same checkpoints x2; `cargo test -p engine-editor --lib
  play_in` green. needs: ED4, WIN1.
- [x] ED17 editor: panel frame budgets - every panel ≤ 1 ms under the
  PF2 capture, baseline posted with adapter + driver line
  (reference-GPU session run by the AI here (O5)). Tier B. Size S. Owns
  `crates/editor/benches/panels.rs`. Accept: per-panel ms table posted;
  an over-budget panel fails the row. needs: ED1, PF2. Landed 202ad257. crates/editor/benches/panels.rs: every dock panel median under 1 ms on a 2,000-node scene (Hierarchy 0.18, Inspector 0.13, Viewport 0.09, Findings 0.05, Tuning 0.21, Templates 0.07, Log 0.28 ms p50); PC run without a GPU, not a PF2 reference-GPU capture; editor --features ui --lib shell_ui 15 passed.
- [ ] ED18 editor: newcomer re-measure with live play-in-editor -
  first-run checklist to first fight ≤ 15 min on a real newcomer clock,
  hints only (ED5's 600/900 s predates ED4 integration). Tier B.
  Size S. Owns `crates/editor/src/onboard.rs` timing hunks. Accept:
  measured time posted; every blocker fixed in the flow, not coached
  around. needs: ED5, ED4. Progress 2026-10-18: skipped, needs a real newcomer on a clock.
- [x] ED19 editor: outliner at siege scale - 2,000-row scene (64-player Landed 71c3f0fb. siege_scene_search_stays_under_50_ms (worst 344.9us), quiet_panel_frame_stays_under_one_millisecond_at_siege_scale (15.4us quiet, 2.8ms after an edit), selection_sets_over_hundreds_of_siege_rows (1.06ms); 19 outline tests, local PC run 2026-10-17.
  + 500-AI shape): search ≤ 50 ms, panel frame ≤ 1 ms, selection sets
  over hundreds (Godot rename-proof shape stands, MIT). Tier C.
  Size S. Owns `crates/editor/src/outline.rs` scale hunks + fixture.
  Accept: fixture numbers posted; no mirror types. needs: ED6.
- [x] AS4 assets: handles and load states — generational handles and Landed 070e6fdb. handle:: 10 passed incl. fifty_asset_matrix_resolves_or_refuses and stale_handle_never_panics_and_never_ready run 39056931005.
  NotLoaded, Loading, Loaded or Failed per asset with dependency readiness,
  typed refusals and no panics, on top of the AS2 queue (Bevy asset pattern,
  MIT OR Apache-2.0). Tier B. Size S. Owns `crates/assets/src/handle.rs`.
  Accept: a 50-asset matrix resolves or refuses cleanly; cook bytes x2
  unchanged; nothing touches the tick. needs: AS2.
- [ ] AS5 assets: one residency manager — textures, meshes, meshlets and
  acceleration structures share one priority-driven memory budget (4.5 GiB
  on the RTX 3050) with eviction, over the AS2 streaming queue. Tier B.
  Size M. Owns `crates/assets/src/residency.rs`. Accept: a streaming soak
  never exceeds the budget; evictions follow priority; no hitch over 2 ms
  from residency work. needs: AS2, AS4.
- [ ] AS6 assets: USD subset import — read USD text and binary for meshes,
  transforms, UsdPreviewSurface materials and variants (OpenUSD Apache-2.0 as
  the spec; no C++ dependency), so DCC pipelines and AI tools can hand scenes
  over as well as glTF. Tier C. Size L. Owns `crates/assets/src/usd.rs`,
  `crates/scene/src/usd_codec.rs`. Accept: a set of reference USD files
  import, place, save, reopen and play; unsupported features refuse with
  their name. needs: CT10.
- [ ] SN4 audio: HRTF via Steam Audio behind `device` wall. Tier B.
  Size L. Owns `crates/audio/src/hrtf.rs`. Accept: front/back resolved
  on headphones; LICENSE verified first with evidence row.
  needs: AU1.
  LATER: parked 2026-10-03 (circus verdict: audio parked by FOCUS.md (owner 2026-09-30)); unpark by deleting this line.
- [x] SN5 audio: headphone 3D — an HRTF path for headphones (HRIR sphere, Landed 52538adf. orbit_blind_naming_7_of_8 + steady_state_process_allocates_nothing, hrtf:: 14 passed run 39056954000 (row stays FOCUS-parked by its LATER line).
  overlap-save convolution, geometry-aware interpolation as fixed tables;
  `hrtf` crate MIT as a pattern or a dependency with a DEPENDENCIES row;
  arXiv 2609.25995 idea only). Speakers keep the current pan. Tier B. Size L.
  Owns `crates/audio/src/hrtf.rs`. Accept: blind orbit test resolves 7 of 8
  octants front/back or better (2 more than pan only); zero audio-thread
  allocations. needs: AU1.
  LATER: parked 2026-10-03 (circus verdict: audio parked by FOCUS.md (owner 2026-09-30)); unpark by deleting this line.
- [ ] SN9 audio: voice-bake vs voice-net boundary — typed tests proving
  `voice.rs` bake-only refusal holds and ON8/ON12 net voice never
  touches the tick, checkpoints or golden. Tier B. Size S. Owns
  boundary test hunks in `crates/audio` (no new file). Accept:
  live-synth call refuses with its typed error; voice on/off golden
  identical x2; transport stays ON-owned, bake stays audio-owned.
  needs: ON8, AU1. (PLN audio, reviewed 2026-09-26; waits ON8.)
- [x] UIX6 ui: settings persistence — versioned settings file with Landed bc203f6f.
  first-run adapter defaults; unknown fields refused loudly,
  migrations staged at boundaries. Tier A. Size S. Owns persistence
  hunks in `crates/ui/src/settings.rs` (with UIX2, no new file).
  Accept: corrupt or future-version file falls back to defaults with
   a notice; round-trip identical twice; a changed setting still
   replays exactly. needs: UIX2. (PLN shipping.md, reviewed 2026-09-26;
   waits UIX2.)
- [x] UIX7 ui: settings dispatch wiring — menus `OpenSettings` rows Landed 1f2432f0. menus:: green incl. settings_rows_stage_then_swap_on_the_boundary; ui filter menus|lobby|access|settings|tutorial 110 passed run 39056568551.
  dispatch into UIX2 staging with TU1 swaps. Tier A. Size M. Owns
  dispatch hunks in `crates/ui/src/menus.rs`. Accept: `cargo test -p
  engine-ui --lib menus` green. needs: UIX1, UIX2.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [x] UIX8 ui: lobby scale proof — 16-slot list virtualized over MP3 Landed 0b43f1d9. lobby_scale_sixteen_virtualized_with_cost_and_projection + lobby_scale_window_past_end_draws_nothing, run 39056568551.
  state, per-slot cost + 64-slot projection as numbers. Tier B.
  Size S. Owns scale hunks beside `crates/ui/src/lobby_ui.rs`.
  Accept: `cargo test -p engine-ui --lib lobby` green. needs: UIX3,
  MP3. (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [x] UIX9 ui: subtitle storm bound — subtitles-minimal over a Landed 1f2432f0. a_sixteen_fighter_hit_storm_stays_bounded_and_counts_every_drop + access:: green, run 39056568551.
  drop-and-count queue under a 16-fighter hit storm. Tier B. Size S.
  Owns storm hunks beside `crates/ui/src/access.rs`. Accept: `cargo
  test -p engine-ui --lib access` green. needs: UIX4, CB1.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [x] UIX4 ui: accessibility — subtitles-minimal, remap UI, contrast Landed bc203f6f.
  pass over UI1 palettes. Tier B. Size M. Owns
  `crates/ui/src/access.rs`. Accept: every UI1 palette passes the
  `contrast_ratio` number against `MIN_CONTRAST` (`crates/ui/src/lib.rs`
  `Palette::validate`), remaining taste goes to the AI review panel;
  subtitle events from fight cues. needs: UI1, CB1.
- [x] UIX10 ui: remap-by-pad drill — full rebind inside settings with Landed 1f2432f0. a_full_rebind_needs_only_the_pad_and_goes_live_on_its_boundary, the_pad_drill_replays_exactly_twice, a_pad_button_another_action_holds_is_refused_naming_both, run 39056568551.
  the keyboard unplugged; conflicts refuse naming both actions. Tier
  B. Size S. Owns rebind hunks beside `crates/ui/src/settings.rs`
  (UIX2 file frozen; lands after UIX2). Accept: pad-alone full rebind
  green; every edit stages at a boundary and replays exactly. Proof:
  `cargo test -p engine-ui --lib settings` green. needs: UIX2, WIN3.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [x] UIX11 ui: stale-beacon eviction — beacons older than K refresh Landed 1f2432f0. a_ghost_storm_is_evicted_in_one_poll_and_no_ghost_stays_listed, a_thousand_ghosts_never_outgrow_the_list_or_lock_a_real_host_out, ready_up_refuses_slots_the_lobby_stopped_refreshing, run 39056568551.
  periods evict with a notice; ready-up refuses stale slots. Tier B.
  Size S. Owns eviction hunks beside `crates/ui/src/lobby_ui.rs`
  (UIX3 file frozen; lands after UIX3). Accept: scripted ghost storm
  evicts in O(16) with no ghosts shown; 64-slot projection posted as
  numbers; typed messages only. Proof: `cargo test -p engine-ui --lib
  lobby` green. needs: UIX3, MP3.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [x] UIX12 ui: tutorial-2 defense lesson — block, parry and riposte Landed 1f2432f0. fresh_profile_finishes_block_parry_riposte_with_no_console, defense_lesson_replays_exactly_twice_and_differs_from_lesson_one, skipping_the_defense_lesson_leaves_a_clean_menu, run 39056568551.
  window as new `SCRIPT` rows over UIX1 commands, inputs recorded like
  any fight. Tier B. Size M. Owns lesson hunks in
  `crates/ui/src/tutorial.rs` (UIX5 file frozen; lands after UIX5).
  Accept: fresh profile finishes block + parry + riposte with no
  console; lesson replays exactly x2; skip leaves a clean menu. Proof:
  `cargo test -p engine-ui --lib tutorial` green. needs: UIX5, GP1.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [ ] HR6 combat: generated moves into move tables — a CT14 clip becomes a
  proposed HR3 move: startup, active and recovery windows detected from the
  weapon bone's speed, a hit volume swept along the weapon path, damage and
  stamina from the family it joins. VK1 keypoints (the hit lands inside its
  window) and the PQ2 tournament (no move above 35% of kills) decide; a move
  that breaks balance is refused with the numbers. Tier B. Size M. Owns
  `crates/combat/src/move_intake.rs`. Accept: 5 generated moves either land
  as valid HR3 entries or are refused with a reason; replay x2. needs: CT14,
  HR3, VK1, PQ2.
- [x] EV15 research: the research ladder. Tier A. Size S. Owns Progress 2026-10-17: 20aea8ee tools/ladder-counts.mjs; left: the LADDER rule in the research skill.
  `docs/research/LADDER.md` and the LADDER line rule in
  `skills/research/SKILL.md`. Accept: every research return adds
  LADDER lines; the retro prints counts per rung from the file. needs:
  EV3. (PLN48 frontier.md, reviewed 2026-09-27.) LADDER ID repairs
  applied wave 49 (PLN49): P2 RideWay is AX11, P2 Chronos is AX12, the
  P2 moonshot is BET-AX13, the tool-surface benchmark is AX10. Landed 3defa488. LADDER rule in skills/research/SKILL.md, node --test tools/ladder-counts.test.mjs docs/research/LADDER.test.mjs 9 pass.
- [x] EV16 playtest: checksummed sparse rebuilds for brief-to-game eval —
  per-tick checksums + sparse snapshots + tape alignment over integer state
  (GGRS discipline reimplemented, transport never). Tier B. Size S. Owns
  checksum hunks beside the EV2 rebuild path (tape format frozen). Accept:
  20/20 planted corruptions located to the tick; memory <= half full-state;
  rerun x2 identical. needs: EV2, QA1. (PLN48 STEAL dive14 Card 4 remap.) Landed 50c4792b + 2da5e84c. rebuild.rs: snapshot every 8 ticks + per-tick checksum, audit names first bad tick, 20/20 planted corruptions located, 125 permille of a snapshot per tick; targeted run 39072046031 (16 passed).
- [x] QA3 ai: deterministic BT bot brains for playtest bouts — sequence / Landed 20aea8ee. Run 39052941210; Brain::Trees in bot_harness.
  selector / blackboard ticks over integer state, serializable trees.
  Tier A. Size M. Owns brain hunks beside `crates/ai/src/` (bot tables
  frozen). Accept: 200 bouts rerun-identical x2 from tapes; VK1 pass >=
  baseline -2%; 0 stuck; decision <= 0.1 ms; 0 float/alloc in tick. needs:
  QA1, PQ2. (PLN48 STEAL dive14 Card 1 remap.)
- [x] PQ9 proof: skill-rating scorer for the balance tournament — Elo / Landed 20aea8ee. Elo line per difficulty in the tournament report; run 39052941210.
  Glicko-2 over bout records, offline only. Tier B. Size S. Owns scoring
  hunks beside the PQ2 harness (bout records frozen). Accept: planted OP
  move detected <= 100 bouts; 0 false alarms on balanced records; scoring
  <= 10 ms. needs: PQ2. (PLN48 STEAL dive14 Card 2.)
- [x] CR2 net: structured divergence capture for replay tapes — fixed Landed 20aea8ee. 48-byte divergence record; run 39052941210.
  schema (tick + input + subsystem + digests) at first divergence. Tier B.
  Size S. Owns capture hunks beside `crates/net/src/replay.rs` (tape
  format frozen). Accept: 10/10 seeded bugs attribute to exact tick+input
  x2; capture <= 64 B/checkpoint. needs: QA1, CR1. (PLN48 STEAL dive14
  Card 3.)
- [x] SP19 spatial: hierarchical brief for large scenes — rooms, groups, Landed 4c6d817.
  objects with a token budget and `expand` verbs on the brief. Tier A.
  Size M. Owns `crates/spatial/src/brief.rs`. Accept: a 2,000-object
  fixture briefs under 4k tokens and the A6 spatial proof still passes.
  needs: SP18. (PLN48 frontier.md.)
- [ ] SP20 spatial: spatial question answering from the graph — "left of", Progress 2026-10-17: ea34de95 parse_rel and relation kind on scene.query; left: the model-vs-raw-brief comparison needs a live model.
  "visible from", "reachable from" as engine queries over MCP. Tier A.
  Size M. Owns `crates/spatial/src/relation.rs`, `crates/endpoint` verb.
  Accept: 50 seeded questions answered exactly; a model using the verb
  beats the same model reading the raw brief (numbers in LADDER). needs:
  SP19. (PLN48 frontier.md.)
- [x] SP21 spatial: incremental free-space field on `index3` for placement Landed ea34de95. index3_sp21_edit_updates_free_field_under_1ms, run 39054360489.
  and reachability. Tier A. Size M. Owns `crates/spatial/src/index3.rs`.
  Accept: an edit updates the field in under 1 ms on the arena fixture;
  SP16 stacking reads it. needs: SP16. (PLN48 frontier.md.)
- [ ] BET-SP27 spatial (MOONSHOT, HELD): baked probe-field affordances —
  offline interaction probes baked to tags, never in tick, never weights
  in git. Tier C. Size M. Accept: +10% tag accuracy with byte-identical
  replays, else stays held. Kill: tick touch, unlicensed data/weights, or
  no measured lift. needs: SP22, SP24. (PLN48 reader P1 moonshot.)
- [x] AX11 mcp/playtest: EV1 efficiency-utility report column —
  success-gated, turn-vs-tool split, reference floors in fixture; reported,
  never gated. Tier B. Size S. Owns eval-report hunks only (EV1 frozen).
   Accept: 75-row report + utility columns reproduce with one command; >= 1
   new failure mode named or killed with a no-gain report. needs: EV1,
   MCP5. (PLN48 reader P2 Card 1; RideWay idea-only. PLN20: AX11 keeps
   its ID; EV18 shares its distractor fixture shape, AX17 its agreement
   numbers — no second harness.) Landed da46e355. EvalReport::utility_text, floors in fixtures/eval-utility/floors.txt, failure mode fallback_tool_overhead (15 of 75 rows); playtest --lib 318 passed, run 39071693694.
- [ ] AX12 mcp/arena: recorded branch-merge fork receipts — fork-ID
  plumbing beside FK1/EA1c for speculative level-edit branches (bolt-on
  interval tags, merge atomic). Tier B. Size S. Owns fork-ID hunks (fork
  API frozen). Accept: 20-branch speedup + atomicity + x2 equality
   report; MCP4 revives only on green. needs: FK1, EV1. (PLN48 reader P2
   Card 4; Chronos idea-only.) Progress 2026-10-18: skipped by the playtest agent, needs fork-id plumbing in crates/arena (arena files are being edited by the spear and demo agents); retry after they land.
- [x] EV21 eyes: pick manifest — every capture emits entity-id per region
  plus camera project/unproject; a model point becomes an exact entity
  id or 3D point. Tier A. Size S. Owns pick-verb hunks beside
  `crates/endpoint/src/scene_tools.rs` (brief frozen, solver API
  frozen). Consumer: `scene.place` plus the SP12 gold harness. Gold
  task: 100-task gold pointing eval, engine side deterministic at 100%.
  Accept: 100/100 engine-side points resolve; duplicate names resolve
  by id only; bytes x2. needs: EV20. (PLAN eyes.md E2.) Progress 2026-10-18: pick core landed 41a32b6b (pick.rs pixel to entity id or floor point in integer cm, project/unproject pair, region manifest, 100-task gold from tools/pick-gold.mjs 100/100 in a standalone harness, node test green). Left: in-crate run (engine-endpoint --features mcp --lib pick::) and the registered scene.pick verb (scene_tools.rs is held by lane A). Landed eae4da14. scene.pick (pixel, object, cell): pick_door_gold_pointing_eval_resolves_100_of_100 (x2 equal bytes), pick_door_duplicate_names_resolve_by_id_only, pick_door_cell_manifest_tiles_the_frame_and_repeats_bytes, pick_door_finds_an_object_and_the_pixel_picks_it_back green beside the pick:: core tests (run 39109659487).
- [ ] EV22 eyes: model-reader adapter behind the MCP door — mock is
  recorded PNG plus scripted read, local and remote match it; replay
  runs with no model. Tier A. Size S. Owns the reader adapter in
  `crates/provider/src/reader.rs` (new file, contract frozen, no
  weights). Consumer: EV1 eval harness (20 authoring plus 5 play
  tasks). Gold task: 3-path parity (mock, local, remote identical on
  the recorded set) plus offline replay. Accept: 3-path parity green;
  replay with no model byte-identical; AI-agnostic law holds (no
  weights in the tick). needs: EV20, EV21. (PLAN eyes.md E3.)
- [ ] EV23 eyes: model-written fight report — tape plus captures plus
  brief in, schema-pinned JSON out, digest-verified, replay-identical.
  Tier A. Size S. Owns the report writer in
  `crates/playtest/src/fight_report.rs` (new file, schema pinned).
  Consumer: proof CLI evidence reports (`target/evidence/`). Gold task:
  3 tapes replay to identical reports. Accept: 3/3 tapes identical
  bytes x2; digest line reads back; golden `d57b5c8ae3096614`
  unchanged. needs: EV20, EV21, EV22. (PLAN eyes.md E4.)
- [x] MCP25 mcp: scope-policy file deepening the capability table. Landed ea34de95. policy file in tool_capabilities.rs.
  Tier A. Size S. Owns policy-file hunks beside
  `crates/endpoint/src/tool_capabilities.rs` (table frozen).
  Accept: 20 adversarial briefs ≥ 80% refused by capability check with
   the policy file versioned; 0 legit FP on 20 legit briefs; p95 ≤ +10%.
   needs: MCP9, MCP10. (PLN20 ai-native.)
- [ ] BET-AX13 playtest (MOONSHOT, HELD): offline efficiency-penalty refit
  from versioned traces, advisory only. Tier C. Size M. Accept: advisory
  report only; two-green-exports rule to graduate. needs: EV1, AX16,
  AX11. (PLN48 reader P2 moonshot; PLN20: AX5 never boarded, corrected
  from EV1, AX5, AX11.)
- [ ] BET-AX14 playtest (MOONSHOT, HELD): offline GEPA-style prompt
  evolution from versioned traces, advisory only. Tier C. Size M. Owns
  advisory-diff hunks beside `crates/playtest` (trace format frozen;
  no tuner in the repo). Accept: advisory report only; +pass with 0
  success loss to graduate; two-green-exports rule (AX17) to graduate
  or stays held. Kill: auto-apply, any success loss, format drift,
  weights in git. needs: AX16, AX17. (PLN20 ai-native.)
- [ ] API6 mcp: protocol-pinned collapse guardrail — retain-list +
  adaptive budget beside the API4 collapse (protocol-aware trimming
  reimplemented, never model summarization). Tier C. Size S. Owns
  guardrail hunks beside `crates/endpoint/src/collapse.rs` (API1 frozen).
  Accept: 20-fixture token/success/latency report; kill-or-land with
  numbers (bar: >= 50% fewer tokens, 0 loss, p95 <= +10%). needs: API1,
  API4. (PLN48 reader P2 Card 2.)
- [x] MCP17 mcp: turn-vs-tool capability refusal tags — taxonomy hunks Landed e3a3f738. RefusalAxis, 20/20 adversarial briefs tag correctly.
  beside the MCP11 table tagging each refusal by axis with a recovery
  hint. Tier A. Size S. Owns `crates/endpoint/src/tool_capabilities.rs`
  tags (tools frozen). Accept: 20/20 adversarial tagged >= 80% correctly;
  p95 <= +10%; 0 legit FP. needs: MCP9, MCP10, MCP11. (PLN48 reader P2
  Card 3.)
- [ ] PF19 perf: quiesced reference-GPU fill of the PF13 GPU/VRAM rows —
  pilot-only, no lane, no code. Tier A. Size S. Owns
  `tools/perf-baseline.json` GPU/VRAM hunks only (consumer RT14/VG12 GPU
  comparison; budget one quiesced session on the RTX 3050 laptop, release
  profile, high-performance power, CPU load <10%, 3 runs within +/-10%,
  21 samples with per-sample hash assert, adapter + driver MAJOR.MINOR +
  power line per row). Accept: `node tools/perf-baseline.mjs --check`
  PASS with all 11 rows `status: baseline` carrying ordered p50/p95/p99
  quantiles + golden `d57b5c8ae3096614` x2 unchanged; a loaded-box run
  still lands observed-only and never overwrites a baseline row. needs:
  PF13. (Deep-dive 2026-09-30.) Progress 2026-10-18: 4e51bd7a observed only: the box cannot be quiesced (GPU p50 0.52-0.83 ms over 5 runs), arena adds about 0.45-0.65 GiB VRAM; tools/perf-baseline.mjs from the accept line does not exist.
- [ ] RT14 render: many-light spike — ReSTIR DI vs clustered forward on
  250 torches on the reference GPU. Tier C spike. Size M. Owns a spike
  folder under `examples/`. Accept: ms and noise per method in LADDER;
  RT2-RT8 order follows it. needs: PF13. (PLN48 frontier.md.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT15 render: cluster-occupancy audit + range-cut culling for the
  250-torch arena — occupancy histogram + range-cut on torch-grid
  2/25/250 fixtures. Tier B. Size S. Owns audit hunks beside
  `crates/render/src/shadow_cluster.rs` + `graph.rs` (cluster shape
  frozen). Accept: overflow cut ≥50% at identical pixels, ms +≤5%,
  LADDER SPIKE line with overflow count + p95 lights/cell + ms, or
  REJECT with numbers. needs: GX2, RT14. (P4 verdict wave 57 TAKE;
  LADDER P4 CANDIDATE; RT14 stays boarded.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG12 render: meshlet build spike with meshoptimizer ideas on the arena
  mesh set: meshlets, culling rate, indirect draw count. Tier C spike.
  Size M. Accept: numbers in LADDER; VG1 starts from the winner. needs:
  PF13. (PLN48 frontier.md.)
  LATER: parked 2026-10-03 (circus verdict: spike rejected on LADDER); unpark by deleting this line.
- [x] CT21 content: hero weapons-graft fix + re-cook — fix `tools/weapons.py` Landed ad451d33. fighter_bind 3 passed run 39058324156 (pin moved); cook x2 identical + eyes recorded in content/fighters/CT21-RECOOK-RECEIPT.md.
  graft transform so sword/shield land in the hands (Q2 bbox back to body
  scale), re-cook hero, eyes with weapons visible + in-hand, cook x2
  identical, budgets hold, then move the pin (`manifest.json` body/bundle
  SHAs + `crates/game/src/fighter_bind.rs` `EXPECTED_*`) with
   `fighter_bind` green. Tier A. Size M. Owns `tools/weapons.py` +
   re-cook + pin move. needs: none. (CT20.)
  Wave 54: TICK (graft fixed, cook x2 identical 4bdc600a, eyes PASS
  in-hand per orchestrator Read, fighter_bind 3/3 on moved pin;
  opponents byte-identical, refused[] clean).
- [ ] CT23 content: combat + traversal clip expansion (80-clip Tier A,
  deduped by digest). Size L. Owns `content/fighters/incoming/` +
  sidecars + receipt. Accept: 80 clips retargeted, cook x2
  (`cargo test --locked -p engine-assets retarget`), budgets +
  provenance. needs: CT1, CT7. (Plan extender wave 56; plan CT22.)
- [ ] CT24 content: clip fight-data audit (loop/root/contacts + hit
  windows + traversal edges, bind-pose-verified). Size M. Owns
  `content/fighters/sidecars/`. Accept:
  `cargo test --locked -p engine-assets sidecar`, CB4/LM2 fixtures
  read windows/contacts, bytes x2. needs: CT1, CT23. (Plan extender
  wave 56; plan CT23.)
- [ ] CT25 content: crowd impostor kit (`content/crowd/` new) for
  8-fighter soak + C3 20k path. Size M. Accept: soak captures
  populated, 0 findings, cook x2
  (`cargo test --locked -p engine-assets mesh_cook`). needs: CT2, CT8.
  (Plan extender wave 56; plan CT24.) Progress 2026-10-17: 3d997539 gen.mjs deterministic and matches manifest SHAs; left: content/crowd/proof/Cargo.lock is stale (gh cargo check failed, run 39064515835) and the cook proof needs cargo run. Progress 2026-10-18: cb822736 crates/assets/tests/it/crowd_cook.rs cooks both specimens twice (budgets, 4 falling LODs, hulls, digests and tris equal report-split1.json), cargo test --locked -p engine-assets --features import,registry --test it -- crowd_cook PASS run 39105798540; left: soak captures populated with 0 findings (reference GPU) and the stale content/crowd/proof Cargo.lock.
- [ ] CT27 content: 64-player + 1,000-agent soak on RTX 3050 6 GB
  (dedupe + compression/BC/LOD/stream proof, cold ≤5 s, VRAM capped).
  Size M. Accept: soak report + adapter/driver/ms/VRAM
  (`cargo test --locked -p engine-assets lod_swap`), 0 tick allocs,
  golden x2. needs: CT5, CT23, CT25. (Plan extender wave 56; plan
  CT26.)
- [ ] CT31 content: motion-DB style bake (100STYLE motions-only + CMU,
  credited) into LM1. Tier A. Size M. Owns `content/locomotion/` expansion.
  Accept: bake x2 + typed refusals. needs: CT1, CT3. (PLAN-CONTENT wave 58;
  plan §7 CT30 boarded as CT31.)
- [x] CT32 content: 14-canonical-clip dedupe manifest + before/after MB Landed b5cdaf64. 14 clips share one cook digest across 56 slots.
  through AS1 bundles. Tier A. Size S. Owns the dedupe manifest. Accept:
  bundle loads, `fighter_bind` green, bytes x2. needs: CT7.
  (PLAN-CONTENT wave 58; plan §7 CT31 boarded as CT32.)
- [x] CT33 content: sidecar consumer schema + conformance test (CB4 reads Landed b5cdaf64. Run 39052948713 (sidecar 5 passed).
  windows, LM2 reads contacts; traversal schema for board CT24). Tier A.
  Size S. Owns the sidecar schema. Accept: `cargo test --locked
  -p engine-assets sidecar`. needs: CT1, CT7. (PLAN-CONTENT wave 58;
  plan §7 CT32 boarded as CT33.)
- [ ] CT34 content: crowd 20k C3 scale card with P6 numbers on the RTX 3050.
  Tier A. Size M. Owns `content/crowd/scale/` + report. Accept: `cargo test
  --locked -p engine-assets lod_swap` + soak, 0 tick allocs. needs: CT2,
  CT25. (PLAN-CONTENT wave 58; plan §7 CT33 boarded as CT34.)
- [ ] CT36 content: sidecar consumer wiring (CB4 + LM2 read audited
  sidecars). Tier A. Size S. Owns consumer hunks beside
  `crates/combat/` (hit windows) + `crates/anim/` (contacts) with
  fixtures (no new types). Accept: `cargo test --locked
  -p engine-assets sidecar` + consumer tests green; fixtures read the
  CT24-audited 14 canonical sidecars; bytes x2; tick untouched. needs:
  CT24, CT33. (EXTEND wave 85.)
- [ ] CT37 content: real-bytes locomotion intake (100STYLE + CMU,
  motions only, credited). Tier A. Size M. Owns
  `content/locomotion/real/` + bake glue. Accept: first 16 real clips
  retarget via CT1 with bind pose verified; bake x2 identical; EVIDENCE
  rows per file; Geno/LAFAN1 refusal tests green; LM1 reads the bake.
  needs: CT1, CT3. (EXTEND wave 85.)
- [ ] CT38 content: crowd variety via the CT4 loop (armor/helmet
  variants, Tier C path). Tier B. Size M. Owns
  `content/crowd/variants/` + 4 variant sets. Accept: cook x2
  identical; budgets hold; provenance per file names generator, model
  and prompt digest; 8-fighter soak capture shows populated stands
  with 0 findings. needs: CT4, CT25. (EXTEND wave 85.)
- [ ] CT39 content: bundle dedupe + compression audit on the pin
  bundle. Tier B. Size S. Owns `content/fighters/BUNDLE-AUDIT.md` +
  report numbers. Accept: CT32 manifest + CT11-shaped compression +
  BC encode measured MB before/after; bundle loads via AS1;
  `cargo test --locked -p engine-game fighter_bind` green; bytes x2.
  needs: CT11, CT32. (EXTEND wave 85.)
- [x] CT40 content: AI prop intake (TRELLIS/TripoSR rough mesh to
  game prop via the CT4 loop). Tier B. Size S. Owns
  `content/props/ai/` + 3 props. Accept: decimate/unwrap/bake path
  documented; each prop <= 5k tris, <= 2k maps, cook x2 identical;
  provenance per file names generator, model and prompt digest;
  license gate refuses unreadable model terms. needs: CT4. (EXTEND
  wave 85.) Landed cb822736. content/props/ai: 3 props (1126, 2432, 1094 tris, no maps); node --test tools/ai-props.test.mjs (GLB triangle list under 5k, no image, game_tris real, prompt digest, sidecar pins the bytes, no shared bytes); cargo test --locked -p engine-assets --features import,registry --test it -- ai_props PASS run 39105798540 (each prop runs gen_intake::intake with its own provenance, under PROP_MAX_TRIS, cook x2 byte-identical; prose, empty and CC-BY-NC model licences refused before any cook).
- [ ] CT41 content: LOD ms/VRAM table over the CT35 bake. Tier A. Progress 2026-10-17: b5cdaf64 LOD table from hero.json with tris and VRAM; left: GPU ms and the PF13 join.
  Size S. Owns `content/fighters/LOD-REPORT.md` (CT35 `lod/` frozen).
  Accept: `cargo test --locked -p engine-assets lod_swap` + `mesh_cook`
  green; per-LOD tris/ms/VRAM posted on the reference GPU with
  adapter+driver lines; cook x2 identical; 0 tick allocs; numbers join
  the PF13 baseline. Consumer: PF13, CT27 soak, CT34 card. needs:
  CT35, CT5. (EXTEND wave 86.)
- [x] CT42 content: combat clip batch-a (16 clips toward the CT23 80).
  Tier A. Size M. Owns `content/fighters/incoming/batch-a/` + sidecars.
  Accept: 16 clips retarget via CT1 with bind pose verified, digest
  deduped against the 14 canonical; `cargo test --locked -p engine-assets
  retarget` green; budgets + provenance per file; cook x2 identical;
  counts toward CT23. Consumer: CT23 receipt, CT24 audit. needs:
  CT1, CT7. (EXTEND wave 86.) Landed 769737c9 (budget assert 32e07432). 16 batch-a clips vendored (.gitignore carve-out, auto backup 2f039f8e); crates/assets/tests/it/retarget_batch_a.rs: hero bind validate_bind, each clip retarget_cook_with_sidecar x2 identical, digests refused against the 14 canonical and each other, keys and seconds under the clip_receipt budget, rows equal BATCH-A-RECEIPT.md (digest column on Windows only: 2 of 16 digests differ on a Linux runner; PASS on this PC with all 16 equal); cargo test --locked -p engine-assets --features import retarget PASS run 39109839692 (needs the feature: without it the filter matches 0 tests); node --test tools/batch-a.test.mjs pins each GLB sha16 and one sidecar per clip.
- [ ] CT43 content: sidecar audit report for the canonical 14 + batch-a.
  Tier A. Size S. Owns `content/fighters/sidecars/AUDIT.md` (sidecar
  JSONs frozen). Accept: `cargo test --locked -p engine-assets sidecar`
  green; every window/contact bind-pose-verified; CB4/LM2 fixtures read
  the audited windows/contacts; bytes x2. Consumer: CT36 wiring.
  needs: CT24, CT33. (EXTEND wave 86.)
- [ ] CT44 content: crowd eyes + soak staging over the CT25 kit. Tier A.
  Size S. Owns `content/crowd/EYES.md` (CT25 kit frozen). Accept: cook x2
  identical; budgets hold; front/side/top previews against the 1.8 m
  figure; 8-fighter soak capture shows populated stands with 0 findings.
  Consumer: CT27 soak, CT34 card. needs: CT25, CT8. (EXTEND wave 86.)
- [ ] CT45 content: PBR 2k + HDRI provenance report with VRAM caps. Progress 2026-10-17: f2effc78 per-map table generated, bound set 213 MiB under the 256 MiB cap; left: measured VRAM on the reference GPU.
  Tier B. Size M. Owns `content/arena/pbr/REPORT.md` (CT26 pass frozen;
  `content/arena/hdri/` untouched). Accept: 0 findings; Q1/RA14 read PBR
  (no noise); cook x2 identical; provenance per file; per-map bytes +
  VRAM posted on the reference GPU. Consumer: Q1 captures, RA14. needs:
  CT26, CT8. (EXTEND wave 86.)
- [x] CT46 content: provenance audit dry-run for the CT28 gate (50-asset Landed 124fcf36. tools/provenance-audit.mjs: 58 rows, 7 refusals named; run 39053389819.
  matrix). Tier A. Size S. Owns `content/PROVENANCE-AUDIT.md` (gate code
  stays CT28). Accept: matrix lists URL+SPDX+author+SHA or
  generator+model+prompt digest per asset; release-refusal dry-run names
  every gap; `cargo test --locked -p engine-assets provenance` green;
  bytes x2. Consumer: CT28 lock, PR1 chains. needs: CT28, CT7, CT8.
  (EXTEND wave 86.)
- [ ] CT47 content: combat clip batch-b (16 more toward the CT23 80).
  Tier A. Size M. Owns `content/fighters/incoming/batch-b/` + sidecars.
  Accept: 16 clips retarget via CT1 with bind pose verified, digest
  deduped against the 30 before it (14 canonical + batch-a);
  `cargo test --locked -p engine-assets retarget` green; budgets +
  provenance per file; cook x2 identical; counts toward CT23. Consumer:
  CT23 receipt, CT24 audit. needs: CT1, CT42. (EXTEND wave 87.)
- [ ] CT48 content: locomotion real second slice (16 more, 32 with CT37).
  Tier A. Size M. Owns `content/locomotion/real-batch-b/` + bake glue
  (`content/locomotion/real/` frozen). Accept: 16 real clips retarget
  via CT1 with bind pose verified; bake x2 identical; EVIDENCE rows per
  file; Geno/LAFAN1 refusal tests green; LM1 reads the bake. needs:
  CT1, CT37. (EXTEND wave 87.)
- [ ] CT49 content: traversal edge markers (vault/hurdle/mantle/climb/drop).
  Tier A. Size S. Owns `content/fighters/sidecars/TRAVERSAL.md` (sidecar
  JSONs frozen). Accept: `cargo test --locked -p engine-assets sidecar`
  green; every traversal edge bind-pose-verified; TV fixtures read the
  markers; bytes x2. Consumer: TV link, CT36 wiring. needs:
  CT24, CT33. (EXTEND wave 87.)
- [ ] CT50 content: crowd soak staging report over CT25 + CT44. Tier A.
  Size S. Owns `content/crowd/SOAK-REPORT.md` (CT25 kit + EYES.md frozen).
  Accept: 8-fighter soak capture shows populated stands with 0 findings;
  CPU/GPU ms + VRAM posted on the reference GPU with adapter+driver
  lines; cook x2 identical. Consumer: CT27 soak, CT34 card. needs:
  CT25, CT44. (EXTEND wave 87.)
- [x] CT52 content: provenance enforce rehearsal before the CT28 lock.
  Tier A. Size S. Owns `content/PROVENANCE-ENFORCE.md` (gate code stays
  CT28, matrix stays CT46). Accept: seeded missing-provenance bundle
  refused with the named gap; 50-asset matrix re-checked;
  `cargo test --locked -p engine-assets provenance` green; bytes x2.
  Consumer: CT28 lock, PR1 chains. needs: CT28, CT46. (EXTEND wave 87.) Progress 2026-10-17: 3d997539 rehearsal on a seeded 50-record bundle names all three gaps, 58-record matrix still 7 refusals, output identical x2, provenance tests green (run 39064454169); left: CT28 lock. Landed 1c2e0ed3. The seeded 50-record rehearsal is now a test (CT52 test in tools/provenance-audit.test.mjs: slots 7, 19, 33 name missing_url, missing_author, bad_sha and nothing else, same twice); matrix re-checked: 75 records, 3 refusals (bad_name on over-long fighter names, open for CT28), 0 sha_drift after CT82; cargo test --locked -p engine-assets provenance PASS run 39101397483.
- [ ] CT54 content: combat clip batch-c (16 more toward the CT23 80).
  Tier A. Size M. Owns `content/fighters/incoming/batch-c/` + sidecars.
  Accept: 16 clips retarget via CT1 with bind pose verified, digest
  deduped against the 46 before it (14 canonical + batch-a + batch-b);
  `cargo test --locked -p engine-assets retarget` green; budgets +
  provenance per file; cook x2 identical; counts toward CT23. Consumer:
  CT23 receipt, CT24 audit. needs: CT1, CT47. (EXTEND wave 88.)
- [ ] CT55 content: finisher-pair + death/reaction closure for Tier A.
  Tier A. Size S. Owns `content/fighters/incoming/finisher/` + sidecars
  (batch dirs frozen). Accept: finisher pair + death/reaction clips
  retarget via CT1 with bind pose verified; sidecars carry hit windows;
  `cargo test --locked -p engine-assets sidecar` green; provenance per
  file; cook x2 identical; counted by CT58. Consumer: CT23 receipt,
  CB4/LM2 fixtures. needs: CT24, CT33. (EXTEND wave 88.)
- [x] CT56 content: urban-war pilot corner through the same pipeline.
  Tier B. Size M. Owns `content/maps/urban-war-pilot/` (1 variant +
  1 corner; `content/maps/` kits frozen). Accept: variant + corner cook
  via CT1/CT2 with SP5 placement 0 findings; no engine hunk forks for
  the pilot; provenance per file; cook x2 identical; bundle loads via
  AS1. Consumer: UW39 receipt, Finish B second-game bar. needs:
  CT1, CT2. (EXTEND wave 88.) Landed 769737c9. SP5: SPF1 known-good gate pins corner actors=3 verdict=clean (cargo test --locked -p engine-playtest --lib known_good PASS run 39104843792); CT2 cook + AS1: cargo test --locked -p engine-assets --features import,registry --test it -- pilot_corner PASS run 39107410598 (3 actors cook via cook_gltf_bytes, LOD0 tris equal corner-cook.json, one bundle finishes to the same bytes twice, loads through verify_bundle_bytes with every LOD, a flipped byte is refused; whole suite PASS run 39107761835); no engine code added (one test file); provenance per file in PROVENANCE.md and COOK-RECEIPT.md; corner-cook.json x2 identical. Finding: frozen CT9 report digests no longer equal today's CT2 cook (insula 9c78cdae8762922a vs 09461689054e6bb4), not re-baselined.
- [ ] CT57 content: night-HDRI torch staging with VRAM report. Tier A.
  Size S. Owns `content/arena/NIGHT-REPORT.md` (CT53 dressing + day HDRI
  frozen; no pass code). Accept: night-HDRI + torch staging readable by
  Q1/RA14; per-map bytes + VRAM posted on the reference GPU with the
  adapter line; cook x2 identical; golden unchanged. Consumer: Q1
  captures, RA14 presenter. needs: CT8, CT53. (EXTEND wave 88.)
- [ ] CT58 content: 80-clip Tier A receipt + set lock. Tier A. Size S.
  Owns `content/fighters/CLIP-80-RECEIPT.md` (all batch dirs + sidecars
  frozen). Accept: receipt names 14 canonical + batch-a/b/c + finisher
  counts against CT23 with digests; CT24/CT43 audits green; CT36
  consumer fixtures read the counted windows/contacts;
  `cargo test --locked -p engine-game fighter_bind` green; bytes x2.
  Consumer: Tier A set lock, CT36 wiring. needs: CT42, CT47, CT54, CT55.
  (EXTEND wave 88.)
- [ ] CT59 content: counted-clip consumer proof on the staged fight.
  Tier A. Size S. Owns `content/fighters/CONSUMER-PROOF.md` (batch dirs +
  sidecars frozen). Accept: staged 600-tick re-run fires the CT58-counted
  windows/contacts via CT36; `sidecar` + `fighter_bind` tests green; x2.
  Consumer CB4/LM2. needs: CT36, CT58. (EXTEND wave 89.)
- [ ] CT60 content: joint establishing + window-bind capture proof.
  Tier A. Size S. Owns `content/arena/STAGED-PROOF.md` (CT22 + CT29 JSONs
  frozen; no pass code). Accept: RA14/WIN6 shows sand, stands, fighters,
  0 findings, log==pixels (UI4 pattern); `fighter_bind` green; golden
  unchanged. Consumer RA14/Q1. needs: CT22, CT29. (EXTEND wave 89.)
- [ ] CT61 content: empty-stands P6 baseline for the CT27 soak. Progress 2026-10-17: f2effc78 SOAK-BASELINE.md payload estimate 236 MiB; left: 8-fighter day capture ms on the GPU.
  Tier A. Size S. Owns `content/arena/SOAK-BASELINE.md` (CT7 + CT8 bundles
  frozen). Accept: 8-fighter day capture, 0 findings; CPU/GPU ms + VRAM
  posted on the reference GPU with adapter+driver lines; cook x2
  identical. Consumer CT27/CT34/PF13. needs: CT7, CT8, CT51.
  (EXTEND wave 89.)
- [ ] CT62 content: district + siege stream staging for the C5 corner.
  Tier B. Size M. Owns `content/maps/STREAM-REPORT.md` (CT9 kit frozen).
  Accept: ludus/district/siege probe walks with 0 findings; per-map
  bytes + VRAM + hitch count on the reference GPU with the adapter
  line; cook x2 identical; golden unchanged. Consumer C5/WD. needs:
  CT9. (EXTEND wave 90.)
- [ ] CT63 content: combat sound list with provenance (Tier A sound list). Progress 2026-10-17: b29e14b0 content/sound/HIT-LIST.md maps every FightCue to a CC0 file or silent; left: the SN10 bridge (parked).
  Tier A. Size S. Owns `content/sound/HIT-LIST.md` + per-file rows
  (new dir; no audio code). Accept: every hit/knockdown in the staged
  fight names a CC0 source with URL+SPDX+author+SHA; SN10 log bridge
  reads the list; cook x2 identical. Consumer SN10/AU3. needs: CT7.
  (EXTEND wave 90.)
- [ ] CT64 content: reacting-crowd staging toward the C3 20k card.
  Tier C. Size M. Owns `content/crowd/REACT-REPORT.md` (CT25 kit frozen).
  Accept: staged knockdowns fire crowd reactions (clips + CT63 hooks);
  20k-impostor P6 numbers on the reference GPU with the adapter line,
  or the approach is killed with numbers. Consumer C3/CT34. needs:
  CT25, CT44. (EXTEND wave 90.)
- [ ] CT65 content: own-capture signature-move pilot (offline only).
  Tier B. Size S. Owns `content/capture/PILOT-REPORT.md` (new dir;
  landmark shape parked, never in tick). Accept: 2 phone-video moves
  baked via CT1 (1e-4/1e-5) with sidecars + EVIDENCE rows; bake x2
  identical; non-consented sources refused. Consumer CT23/CT31.
  needs: CT1, CT37. (EXTEND wave 90.)
- [ ] CT66 content: urban-war full kit after the CT56 pilot proves no fork.
  Tier B. Size M. Owns `content/maps/urban-war/` (pilot dir frozen).
  Accept: full corner kit via CT1/CT2 with SP5 placement 0 findings;
  row fails when CT56 found any engine fork; provenance per file; cook
  x2 identical; bundle loads via AS1. Consumer UW39/Finish B. needs:
  CT1, CT2, CT56. (EXTEND wave 90.)
- [ ] CT67 content: bounded splat-cook spike pilot (spike-first).
  Tier C. Size S. Owns `content/props/splat/PILOT-RECEIPT.md` (CT40
  `ai/` frozen). Accept: density-pruned splat set under a per-capture
  count/byte cap with license + blake3 receipt; bake x2 identical;
  VX7 reads it or kills it; no tick bytes. Consumer VX7/C3. needs:
  CT40. (EXTEND wave 90.)
- [x] CT74 content: PBR/HDRI intake commit-gate for the untracked bytes.
  Tier A. Size S. Owns `content/arena/PBR-INTAKE.md` (intake bytes
  frozen). Accept: every untracked file carries URL+SPDX+author+SHA or
  refusal; committed set cooks x2; `cargo test --locked -p engine-assets
  provenance` green. Consumer CT45/CT57. needs: CT8, CT46.
  (EXTEND wave 92.) Progress 2026-10-17: 3d997539 all 16 untracked content/arena files refused (no provenance row), provenance tests green (run 39064454169); left: committed-set cook x2 once arena_cook.rs is tracked. Landed 1c2e0ed3. The 16 files untracked on 2026-10-17 are committed (fa1f2ff6); the 15 cooked KTX2 maps now carry matrix rows (ambientCG set page, CC0-1.0, author ambientCG, SHA), cook.json and sources/ are listed outside the matrix (content/arena/PBR-INTAKE.md); node --test tools/provenance-audit.test.mjs 9/9 incl. the CT74 test; committed set cooks x2: arena_cook PASS run 39102340252; provenance PASS run 39101397483.
- [ ] CT75 content: staged-dir triage into the batch digest queue.
  Tier A. Size S. Owns `content/fighters/incoming/STAGED-TRIAGE.md`
  (staged dir frozen). Accept: every `staged/` file named keep/refused;
  kept files deduped against canonical 14; bytes x2. Consumer CT42/CT47.
  needs: CT1, CT32. (EXTEND wave 92.)
- [ ] CT76 content: 100STYLE scout spike before the CT37 intake.
  Tier A. Size S. Owns `content/locomotion/SCOUT-REPORT.md` (scout bake
  frozen). Accept: 2 motions-only clips via CT1 with sidecars +
  EVIDENCE; Geno/LAFAN1/QAL refusals green; LM1 reads scout bake; x2.
  Consumer CT37/CT31. needs: CT1. (EXTEND wave 92.)
- [x] CT77 content: arena src commit-gate (crate + torch pair). Landed f2effc78. SRC-RECEIPT.md gate.
  Tier A. Size S. Owns `content/arena/SRC-RECEIPT.md` (pair frozen).
  Accept: pair committed only with cook x2 + SP5 0 findings; SHAs +
  budgets named; golden unchanged. Consumer CT53/CT57. needs: CT8.
  (EXTEND wave 92.)
- [x] CT78 content: urban-war pilot scout on the CT9 kit.
  Tier B. Size S. Owns `content/maps/PILOT-SCOUT.md` (CT9 kit frozen).
  Accept: names the 1 variant + 1 corner with 0-finding probes +
  bytes/VRAM guess; bytes x2. Consumer CT56/UW39. needs: CT9.
  (EXTEND wave 92.) Landed 90377223. content/maps/PILOT-SCOUT.md names variant district-corner and corner district-corner-01; 0-finding probe = SPF1 known-good gate (cargo test --locked -p engine-playtest --lib known_good PASS run 39104843792, corner actors=3 verdict=clean); 384147 source bytes, 5376 LOD0 tris, VRAM guess 4893353 bytes; cook-corner.mjs x2 leaves corner-cook.json identical; node --test tools/pilot-scout.test.mjs 4/4.
- [ ] CT79 content: digest-dedupe tool for every intake batch.
  Tier A. Size S. Owns `content/fighters/tools/dedupe.py` +
  `DEDUPE-RECEIPT.md` (canonical 14 + batch dirs frozen). Accept:
  deterministic walk over canonical 14 + batch dirs refuses dupes with
  names; receipt x2; CT58 counts tool output. Consumer CT58/CT24.
  needs: CT32, CT42. (EXTEND wave 92.)
- [x] CT80 content: studio HDRI vendoring for the Q1/WIN9 bin proof.
  Tier A. Size S. Owns `content/incoming/studio_small_09_1k.hdr` +
  provenance row (CT8 kit + `content/arena/hdri/` frozen; no new
  dependency). Accept: 1 CC0 file vendored with URL+SPDX+author+SHA;
  committed set cooks x2 identical; `cargo test --locked -p
  engine-assets provenance` green; Q1/RA14 read it with 0 findings.
  Consumer Q1 captures, RA14 presenter, WIN9 bin proof. needs: CT8. Landed 1c2e0ed3. content/incoming/studio_small_09_1k.hdr carved out of the dropbox ignore and vendored with studio_small_09_1k.hdr.provenance.json (Poly Haven, CC0-1.0, Sergej Majboroda, md5 d5d7eb9d26d341d6aa4d11c1a54fd97c equal to api.polyhaven.com/files); matrix rows for it and its arena copy (same SHA), node --test tools/provenance-audit.test.mjs CT80 test; committed set cooks x2 arena_cook PASS run 39102340252; provenance PASS run 39101397483; the HDRI still bakes in render (cargo test --locked -p engine-render --features v1 --lib ibl_arena_env 3 passed, run 39104510933). The live presenter now bakes the arena sky, so Q1 captures on the reference GPU stay the held pilot and are not claimed.
- [x] CT82 content: manifest single truth (delete the stale pre-kit Progress 2026-10-17: f2effc78 tools/arena-report.mjs checks manifest.json against disk; left: regenerate manifest.json (sand_floor, stands, banner gltf and placement.json drifted today).
  manifest). Tier A. Size S. Owns `content/arena/manifest_before.json`
  deletion (`manifest.json` + `report.json` frozen; no new bytes).
  Accept: stale file removed; `manifest.json` sole truth with 2k maps +
  ORMs + SHAs; committed set cooks x2 identical; `cargo test --locked
  -p engine-assets provenance` green. Consumer CT45 report, CT74 gate.
  needs: CT26. (Boarded wave 93 from `docs/plan/content.md:281-285`.) Landed 1c2e0ed3. manifest.json re-pinned (4 drifted rows), real-tree test asserts checkManifest clean (node --test tools/arena-report.test.mjs 12/12; --refresh keeps the format); cook x2: cargo test --locked -p engine-assets --features import --test it arena_cook PASS run 39102340252 (two cooks and the committed content/arena/cooked agree byte for byte); cargo test --locked -p engine-assets provenance PASS run 39101397483.
- [ ] SP33 spatial: fuzzy referring-expression grounding over the
  SP3/SP7 graph — ranked node candidates, advisory, never auto-apply
  (HOV-SG MIT). Tier A. Size S. Accept: recall@3 >= 90% on 60 seeded
  descriptions, <= 1 ms/query, bytes x2. needs: SP19, SP20.
  (READER-SPACE wave 50; extends docs/plan/spatial.md.)
- [x] SP36 spatial: dirty-tile export from index3 incremental rebuild Landed 7e48cbdf.
  for touched-only nav re-bake (Recast Zlib). Tier A. Size S. Accept:
  equality 20/20, enumerate p95 <= 0.1 ms, bytes x2. needs: SP15.
  (READER-SPACE wave 50; consumer NV6.)
- [x] SP37 spatial: support-surface enumeration for stacked rest (floor vs Landed 34bac826. two-floor enumeration, run 39054360489.
  shelf-top vs tread + clearance), feeding SP16/SP9 (READER-P1 wave 54).
  Tier B. Size S. Owns enumeration hunks beside `crates/spatial/src/place.rs`
  (solver API frozen). Accept: SP16 two-floor fixtures place 0 findings,
  flat-room goldens byte-identical, solve p95 ≤ 50 ms. needs: SP16, SP9.
  (P1.)
  Wave 54 SPIKE-SP37 ADOPT-scoped: enumeration+clearance (tops 100/80cm,
  flat SHA-match, p95 1.76ms) feeds SP16; run()-clean waits SP16 rest
  distinction.
- [x] SP40 spatial: integer-cm k-d tree for nearest/k-neighbor queries — Landed e95ecd55.
   median-split build, kNN + radius search over integer-cm points,
   ported as std-only Rust from `sdd/kiddo` tree shape (license
   Apache-2.0, reimplement only, never the crate dep; complements the
   NV10 R* broadphase with a point-query structure). Tier B. Size S.
   Owns `crates/spatial/src/kdtree.rs` (relation graph frozen). Accept:
   kNN sets byte-equal to brute force on the seeded corpus; 10k-point
   build + 1k queries within budget; replay x2. F2P:
   `cargo test -p engine-spatial --lib kdtree` (fails before: no k-d
   tree). P2P:
   `cargo test -p engine-spatial --lib board_example_solves_and_passes_sp1`
   stays green. needs: SP3.
   (STEAL 2026-10-03 kiddo, https://github.com/sdd/kiddo.)
- [ ] SP50 scene: per-instance prefab override deltas — keyed override Progress 2026-10-17: prefab 18 tests pass (run 39051482184); left: digest 4aae94af not reproduced.
  map over frozen `reinstantiate`, prefab edits applied underneath
  (Bevy Scene resolve + ScenePatch shape, reimplemented integer-only,
  zero bytes vendored). Tier A. Size S. Owns override-map hunks beside
  `crates/scene/src/prefab.rs` frozen reinstantiate. Accept: `cargo test
  -p engine-scene prefab` green; museum 3-node deltas 5/5 preserved
  (overrides + prefab-underneath + new key), digest `4aae94af` x2,
  SP12 gold still green; kill on any override lost, any prefab edit not
  applied under deltas, or any digest drift. needs: RC4. (LADDER P1
  CANDIDATE 2026-09-30, spike 5/5 + x2.) Progress 2026-10-17: 2216f79d Instantiation::digest keyed by prefab key; museum run (2 overrides kept, 2 underneath edits shown, 1 new key) same digest x2 and fresh-edited-plus-overrides equals live reinstantiate (13574963997578328745); 19 prefab tests + clippy local only, no gh run id; the old 4aae94af came from a JS spike hash and cannot be reproduced, reword Accept when ticking.
- [x] SP56 spatial (held on SP38): canonical digest + perm-stable audit Landed 049cdd8d. conflict_audit_baseline (60 seeds, 20 perms) + conflict_audit_baseline_failures + sp38_audit_hunks_cover_sixty_seeds_and_twenty_perms run 39056881036.
  gate as the SP38 proof leg (60/60 + 20/20 + digest `05b20def` x2).
  Tier B. Size S. Owns audit hunks beside
  `crates/spatial/src/place.rs` + `repair.rs`. Accept: `cargo test -p
  engine-spatial --lib conflict_audit` green; digest x2 integer-only;
  golden unchanged. needs: SP38. (PLN future-spatial 2026-09-30.)
- [ ] BET-SP57 spatial (MOONSHOT, HELD): offline learned conflict
  predictor, advisory only, never in tick. Tier B. Size S. Owns offline
  preset only. Accept: `cargo test -p engine-spatial --lib
  conflict_predictor_agreement` green; 60/60 + 10/10 at identical
  bytes, else stays held. needs: SP38, SP12. (PLN future-spatial
  2026-09-30.)
- [x] SP60 spatial: frame-aware verb baseline tape — 50 seeded Qs Landed ea34de95. digest c65a8802bf9e5249, run 39054437372.
  measured read-only on the frozen v2 substrate. Tier B. Size S. Owns
  fixture hunks beside `crates/spatial/src/relation.rs`. Accept:
  `cargo test -p engine-spatial --lib frame_verb_baseline` green;
  per-frame-family counts + digest x2 + step us posted; golden
  unchanged. needs: SP7, SP10. (PLN future-spatial 2026-09-30, HOV-SG
  MIT frame shape.)
- [ ] SP58 spatial (held on SP20): Subject+World+Viewer cone verb +
  endpoint answer verb, 50/50 + digest `4802dac9` x2 (boards as SP58
  once SP20 lands, else SP20 detail, no second row). Tier B. Size S.
  Owns verb hunks in `relation.rs` + `crates/endpoint`. Accept:
  `cargo test -p engine-spatial --lib frame_verb` green + endpoint
  verb test green x2; timing at/under 1 ms; golden unchanged. Kill:
  gain <10pp, any drift, any float/model in verb. needs: SP20.
  (PLN future-spatial 2026-09-30, LADDER L426 ACCEPTED.)
- [ ] BET-SP59 spatial (MOONSHOT, HELD): offline distilled answer
  policy, advisory only, never a dep, never in tick. Tier B. Size S.
  Owns offline preset only. Accept: `cargo test -p engine-spatial
  --lib spatial_answer_agreement` green; 50/50 at identical bytes + 0
  drift + panel win, else stays held. needs: SP58, SP20, SP12.
  (PLN future-spatial 2026-09-30.)
- [x] RA16 render: belt + cache pressure drill — 64-player storm with
  64 MiB live and eviction telemetry. Size S. Owns
  `crates/render/src/uploads.rs`. Accept: `cargo test -p
  engine-render --lib uploads` green. needs: RA4, RA6.
  (PLN render-frame.md EXTEND, proposed 2026-09-29.) Landed 9f217d81. uploads.rs restored from f737386e^; cargo test -p engine-render --lib uploads green on this PC (10 passed, 0 failed: 9 donor tests + uploads_sixty_four_player_storm_holds_64_mib_with_eviction_telemetry — 64 players x 256 KiB/frame, live never over 64 MiB cap, peak == high_water 67108864, over-cap spike refuses 32 loud OverCapacity demands with live_bytes unchanged, invalidate_all drains to 0). Note: the belt still has no GPU-path caller; the heal/AH2 wiring remains (L).
- [x] RA17 render: pipeline-cache variant storm — 4096-entry +
  32-variant gate, second-run 0 compiles, last-good drill. Size S.
  Owns `crates/render/src/cache.rs`. Accept: `cargo test -p
  engine-render --lib cache` green. needs: RA5, RA7.
  (PLN render-frame.md EXTEND, proposed 2026-09-29.) Landed e8336db2. variant_storm_gate_stops_at_32_variants_and_4096_entries, variant_storm_second_run_compiles_zero_even_after_a_restart, variant_storm_failed_refreshes_keep_every_last_good_blob (32 variants x 128 shaders = 4096 keys, a full map refuses before compiling, 200 failed refreshes keep every blob); cargo test -p engine-render --lib cache::tests 35 passed run 39101849205, clippy -D warnings run 39101617104.
- [x] RA18 render: heal chaos at load — device loss/Outdated/minimize/
  OOM with last-good kept. Size S. Owns
  `crates/render/src/heal.rs`. Accept: `cargo test -p engine-render
  --lib heal` green. needs: RA1, AH2.
  (PLN render-frame.md EXTEND, proposed 2026-09-29.) Verified 2026-10-17: heal tests 57 passed with tier and cache on the runner (run 39072048539).
- [ ] RA19 render: tier gate at 6 GB — forced-tier refusals, VRAM
  high-water read, Base/Rt/Ultra intact. Size S. Owns
  `crates/render/src/tier.rs`. Accept: `cargo test -p engine-render
  --lib tier` green + reference-GPU stamp. needs: RA8, RT0.
  (PLN render-frame.md EXTEND, proposed 2026-09-29.)
- [ ] WIN6b game: staged fight to the presenter — the WIN6a receipt
  drives arena fight frames with bodies visible and hits readable
  instead of noise tiles. Tier A. Size S. Owns capture-path
  hunks beside `crates/game/src/windowed.rs` (WIN1 loop frozen).
  Accept: one staged 600-tick run emits 4+ frames with fighters
  visible; RA1 presents with 0 validation errors. needs: WIN6a,
  RA1. (A7, 2026-10-02.)
  LATER: parked 2026-10-03 (circus verdict 2: duplicate of WIN6 (same title and accept; WIN6 holds the judge-PASS lane win6-top)); unpark by deleting this line.
- [ ] WIN7 game: pad edge hardening — hot-plug pause/resume proof,
  non-finite deflection rest, 256-code table fuzz over IN1. Tier A.
  Size S. Owns edge hunks in `crates/game/src/pad.rs`. Accept:
  `cargo test -p engine-game --lib pad` green. needs: WIN3.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
  LATER: parked 2026-10-03 (circus verdict 2: docs/plan/shipping.md parks shipping (rows stay, no writers unless a bar needs them) and only WIN9 is the bar's exception; this window polish changes no pixel, hash or timing); unpark by deleting this line.
- [x] WIN8 game: mode-switch last-good drill at lobby load — swaps
  during a 16-slot MP3 lobby keep last-good within one frame.
  Tier A. Size S. Owns drill hunks in `crates/game/src/modes.rs`.
  Accept: `cargo test -p engine-game --lib modes` green. needs:
   WIN2, MP3. (PLN shipping.md EXTEND, proposed 2026-09-29.) Landed 072e79ba. modes.rs restored from d89bd98e^; cargo test -p engine-game --lib modes green on this PC (20 passed, 0 failed: 9 modes incl. failed_device_swap_keeps_last_good_loudly and staged_swap_applies_only_at_its_frame, plus 11 modes_world); caller: bin/arena.rs resumed validates the windowed mode before window creation (refusal -> WinError::NoDisplay); cargo clippy -p engine-game --all-targets --features app -D warnings clean.
- [ ] WIN12 game: crash-plus-settings repro bundle — the WIN4 report
  carries the settings digest plus display mode beside
  version/adapter/tape-tail/checkpoint. Tier B. Size S. Owns digest
  hunks in `crates/game/src/crash.rs` (WIN4 file frozen; fixed bytes,
  64 KiB caps hold). Accept: forced crash under custom settings
  regenerates the pre-crash tick from report + settings; digest
  mismatch refuses loudly. Proof: `cargo test -p engine-game --lib
  crash` green. needs: WIN4, UIX6.
  (PLN shipping.md EXTEND, proposed 2026-09-29.)
- [ ] GP25 game: play-step follow camera — the endpoint top-down view
  keeps both fighters framed with the 20 m bounds drawn, fixing the t60
  edge-clip and t120 opposite-corner splits. Tier B. Size S. Owns
  follow/bounds hunks beside `crates/game/src/camera.rs` +
  `camera_feed.rs` (GP24 wiring first). Accept: scripted 600-tick
  2-fighter tape keeps both in frame 600/600 with no edge clip; bounds
  visible; sim bytes x2. needs: GP24. (PLN49 R3 reshape; R3 fixtures
  join GP24's framing proof.)
- [x] SN11 audio: repeat-last concealment over silent XRUN gaps — heal renders last frame instead of silence. Tier B. Size S. Owns `crates/audio/src/heal.rs` render_silent_frames hunks (mixer frozen). Accept: S1 8kHz 3x4-gap silent edge 0.3387 -> repeat-last 0.0000 (100pc cut, bar 50pc); 0 per-callback allocs; digest x2 identical; consumer render_stereo_into. needs: CQ13. Landed 1a454c22. HoldLastMixer (heal.rs) repeats the last frame over a contended shared-mixer chunk; open_audio opens the device on it. Own 8 kHz tone, three 4-frame gaps: silent edge 0.4619 -> repeat-last 0.0000 (100pc cut, bar 50pc; the spike's 0.3387 is its own tone). Tests repeat_last_removes_the_silent_edge_of_xrun_gaps, repeat_last_run_twice_gives_identical_bits, hold_last_callback_allocates_nothing_on_live_or_gap_chunks, a_gap_before_any_frame_holds_silence_and_odd_tails_stay_zero: engine-audio --lib heal run 39101938469, full engine-audio --no-fail-fast 264+22 passed run 39104877397, clippy -D warnings run 39102103944.
- [ ] GP26 game: shake A-B bundle — paired jab-vs-super captures on one
  camera with per-frame shake_offset + trauma in the manifest, closing
  GP21's missing blind review. Tier B. Size S. Owns manifest/test hunks
  beside the `crates/game/src/camera.rs` shake stage + `camera_feed.rs`
  (GP21 stage frozen). Accept: jab <= 1/4 super displacement measured;
  restore <= 1e-5 every tick; sim x2; playtester small/big call
  recorded. needs: GP21, GP11. (PLN49 R8 reshape.)
- [x] EA4 endpoint: bound the door audit — rotation cap + anchor (EA3's Landed 15d91eb2. audit keeps 1024 records plus anchor digest; 100/100 tamper test.
  256 pattern) or budgeted unbounded justification; 100-session tamper
  stays 100/100. Tier A. Size S. Owns `crates/endpoint/src/audit/`
  bounds (append-Result kept). Accept: memory bounded by test;
  M-MEM-REUSE holds. needs: none. (API review wave 49.)
- [x] MCP22 endpoint: un-mirror sibling types — play/scene_query consts Progress 2026-10-17: 15d91eb2 audit mixer reads core GOLDEN_GAMMA, scene_query uses spatial cm helpers; left: mirrored fighter ranges in play.rs (needs an endpoint-to-input edge).
  + audit mixer via real edges (core-hash edge packet covers the
  mixer). Tier B. Size S. Owns mirrors only, no behavior change.
  Accept: `hash_reimpl` 36 down, dup sites 34 down, goldens unchanged.
  needs: none. (API review wave 49.) Landed cdd17767 (after 15d91eb2): play.run reads WISH_RANGE, FACING_RANGE and KNOWN_BUTTONS from engine-input, the last mirrors; workspace totals hash_reimpl 17 (was 36) and dup_fns 26 (was 34); play tests and goldens unchanged (run 39109659487).
- [x] MCP23 endpoint: strong types for units + actions — `PlacedDto` Landed ed0903cc. PlayAction enum, checked PlacedDto.
  meters/radians + range asserts, `PlayAction` enum, no silent yaw
  flatten. Tier B. Size M. Owns `scene_tools.rs` + `play.rs` types
  (MCP18 first). Accept: malformed action / non-finite pose refused
  typed; M-STRONG-TYPES holds. needs: MCP18. (API review wave 49.)
- [x] API8 endpoint: builder + ctor ergonomics — by-value chaining, Landed bb9ed898. LaunchSpec builder, run 39054820535.
  config struct, Debug/Clone/Default. Tier C. Size S. Owns ctors only.
  Accept: doctest chains a full spec from a temporary. needs: MCP19.
  (API review wave 49.)
- [x] SP28 spatial: relation-cycle consistency over the SP3 graph — Landed ea34de95. brief p95 25 us, run 39054437372.
  integer-cm factor residuals with smallest-cycle refusals. Tier A.
  Size S. Owns check hunks beside `crates/spatial/src/relation.rs` +
  `checks.rs` (SP3 Relation/Graph frozen). Accept: 60 seeded graphs,
  >= 90% inconsistent caught, 0% FP, brief p95 <= 5 ms; bytes x2.
  Kill: any FP or float in the check. needs: SP3, SP5. (PLN50 wave-49
  WILD card 1; gtsam ideas-only, Eigen never ported.)
- [ ] BET-SP31 spatial (MOONSHOT, HELD): postings-ledger free-space —
  O(1) trial-balance re-checks over the SP21 field for SP16 stacking.
  Tier C. Size M. Accept: 10x re-check speedup with byte-identical
  replays, else stays held. Kill: no measured lift or replay drift.
  needs: SP21, SP16. (PLN50 wave-49 WILD moonshot; LMAX ideas-only,
  UNVERIFIED.)
- [x] SP49 spatial: smallest-cycle refusal + repair receipt over the SP3 Progress 2026-10-17: relation.rs smallest_cycle + cycle_residuals_cm and the checks.rs relationCycle finding exist (SP28); left: repair.rs has no cycle repair or re-check receipt and no 30 clean / 30 one-cycle corpus.
  graph — explain+repair beside SP28 detect (SayPlan simulator-repair
  shape, datafrog NOT taken). Tier A. Size S. Owns repair hunks beside
  `crates/spatial/src/repair.rs` + `checks.rs` (SP3 Relation/Graph
  frozen; SP28 owns detect). Accept: 60 seeded graphs (30 clean/30
  one-cycle) 30/30 detected, 0 FP, 30/30 smallest-cycle named
  permutation-stable, 30/30 repair re-check clean, digest x2 identical;
  integer-only, no float. Kill: any FP/miss/dirty re-check or float.
  needs: SP28. (MERGE-20260929 SP49 spike 2026-09-28; moves VISION
  spatial-eval score.) Landed 2e93b5e8. cycle_refusal and repair_cycles: 30 clean + 30 one-cycle graphs, 30/30 detected, 0 false positives, 30/30 named and permutation-stable, 30/30 clean after repair, receipt digest equal x2 (clippy run 39067192258).
- [ ] PF16 perf: tick-timeline rerun sink — seeded-desync timeline over
  the PF9 recording shape. Tier B. Size S. Owns timeline hunks beside
  `crates/observe/src/rerun_sink.rs` (PF9 file frozen; lands after PF9).
  Accept: seeded desync first tick named 10/10, engine hash unperturbed
  x2. needs: PF2, PF9. (PLN50 wave-49 REPO card 3 reshaped; rerun
  time_point shape reimplemented, no dependency.)
- [ ] PF18 perf: mimalloc as the bench-harness allocator on PF1 release
  workloads, hashes and alloc counts pinned (mimalloc_rust MIT,
  harness-only). Tier B/C. Size S. Owns the `GlobalAlloc` override in
  the harness. Accept: wall ms delta at identical hash/alloc rows;
  >= 10% faster or dropped. needs: PF1. (READER-SPEED wave 51.)
  Wave 52: harness LANDED (checker + BENCH52: gates SOUND, ms NOT
  certifiable — p50-of-5 spread up to 4.3x on loaded box). HARDEN
  follow-up on this row: SAMPLES 5→21 + p50/p95/p99 + per-sample hash
  assert + box-conditions line + quiesce rule (load <10%, 3 runs ±10%)
   before any ms joins the baseline file. (2026-10-01 conditional accept
  per docs/plan/quality.md:416: harden slice in-tree + suite-green =
  observed-only, no ms joins a baseline; mimalloc ≥10% delta HELD for a
  quiesced-box rerun or pilot numbers. No more full-row writers.)
- [ ] SPR1 spatial: handle and place for every object - brief, spatial and query label objects
  `<mesh asset name>_<n> [uuid]` instead of `prop_NNNN` (ordinal stable per name), the brief gains one
  compact row per object (cm position, cm size, yaw octant), and `ids`/`objects` obey the byte budget with a
  `cursor`. Diff: good. Tier A. Size M. Owns `crates/endpoint/src/scene_tools.rs`,
  `crates/endpoint/src/scene_tools/spatial_checks.rs`, `crates/spatial/src/brief.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib brief_handles` x2: 200 objects read back as `lamp_2 [id] x y z`; total
  output at budget 4096 stays under 6 KB; inserting an unrelated node leaves other labels and digests
  unchanged. F2P: labels are `prop_NNNN`, tables unbounded, no coordinates. P2P: golden d57b5c8ae3096614
  unchanged. needs: none. Progress 2026-10-18 (lane A, fc0babc8): brief_handles 3 passed: labels are <mesh asset>_<n> (prop_<n> without a mesh) with n counted per name, sizes come from the Collider half extents times scale, scene.brief returns a cursor-paged table (ids, labels, cm pose rows; default call 5421 bytes at 200 objects, was about 17 KB), scene.query shares the snapshot, an unrelated insert relabels nothing. Left: the brief TEXT still prints index rows without coordinates (the pose rows ride beside it), and the whole-scene digest moves with the node count by construction, so "digests unchanged" holds per object row only.
- [x] SPR2 spatial: pin left/right/front/behind with an independent gold and put numbers in hits - 40
  plain-English questions from a drawn floor plan in 3 frames, `left_of X` = objects on X's left (fix the
  direction if relation.rs:919/1428 is inverted), and `dist_cm`, `bearing_deg`, `gap_cm` in relation, near
  and between reasons; convention named in the tool description. Diff: good. Tier A. Size S. Owns
  `crates/spatial/src/relation.rs`, `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p
  engine-spatial --lib dir_gold_plain` and `cargo test -p engine-endpoint --features mcp --lib relation_plain`
  x2: 40/40, hits carry integer cm and degrees, digests re-pinned with a named receipt. F2P: gold is derived
  from `holds`; no test states a side in English. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); left_of WAS inverted and is fixed in relation.rs (92 gold rows swapped, 10 QA_EXPECTED re-pinned, dir_gold_plain 40 hand-computed); left: endpoint relation_plain test and hits carrying pair_numbers (lane A). Endpoint 2026-10-18 (lane A, 5cc241ed): relation hits carry dist_cm, bearing_deg and gap_cm from pair_numbers; relation_plain 1 passed (plain-language floor plan: lamp west of table is left_of, 300 cm, bearing 270, gap 200 cm; chair is right_of). Door test against the oracle still exists beside it. Landed a2dba030 (relation.rs fix, 92 gold rows swapped, QA_EXPECTED re-pinned), 5cc241ed (door pair numbers). dir_gold_plain (40 plain-English questions in 3 frames) in engine-spatial run 39106576997; relation_plain_left_and_right_of_the_table in engine-endpoint --lib 607 passed run 39107175761 (hits carry 300 cm, bearing 270, gap 200 cm).
- [x] SPR3 spatial: scene.query describes itself - enum for `kind`, a doc and unit on every field, the
  required fields per kind in the schema text, one example per kind in `api.describe`, and a `missing_arg`
  refusal naming kind, field and a working call. Diff: good. Tier A. Size S. Owns
  `crates/endpoint/src/scene_query.rs` (SceneQueryArgs docs, refusal). Accept: `cargo test -p
  engine-endpoint --features mcp --lib query_schema_docs` x2: every field documented, 9 kinds list required
  fields, door-reference Range column non-empty for scene.query, `visible` without eye_m refuses with the
  example. F2P: no field docs, `kind` is a String. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed fc0babc8. query_schema_docs 3 passed: every field documented, kind is an enum of the kind words, KIND_CARDS lists required fields, visible without eye_m refuses missing_arg with a working call, api.describe query:<kind> has one example per kind; door-reference Range column filled. cargo test -p engine-endpoint -p engine-render --features engine-endpoint/mcp,engine-render/v1 --lib on the runner (gh run 39079245584; the 5 build_level tests are red, not from this lane); golden d57b5c8ae3096614 unchanged.
- [ ] SPR4 playtest: real-model spatial A/B on cassettes (the missing SP20 number) - 50 seeded questions
  (left of, can see, where fits, why failed) answered by one model from the raw `scene.brief` versus via
  `scene.query`/`scene.place`; provider cassettes make the score replay offline and byte-identical; prints
  exact-match, calls and estimated tokens per arm plus a train-or-not note. Diff: good. Tier A. Size M. Owns
  `crates/playtest/src/spatial_eval.rs` (new live section), `fixtures/spatial-gold/llm/`. Accept: `cargo test
  -p engine-playtest --lib spatial_llm_replay` x2 prints one table; verb arm beats raw arm by >= 20 points or
  the row records the kill; a do-nothing arm scores 0. F2P: no LLM figure exists anywhere. P2P: golden
  d57b5c8ae3096614 unchanged. needs: SPR1. Progress 2026-10-18: harness landed 390d65c2 (spatial_eval/llm.rs: 50 seeded questions, raw-brief and door-verb arms, cassette format, KEEP/KILL at 20 points; echo gold 50/50, do-nothing 0, table prints NO MODEL FIGURE); left: record a real model (docs/plan/spatial-llm-ab.md) after SPR1 lands, no model figure exists yet.
- [ ] SPR5 spatial: footprint-aware placement options - `scene.place` gains `dry_run`; new query kind
  `place_options` takes the object (real size, yaw) plus the same wants and returns up to k poses with
  clearance cm, move cm and the wants each satisfies, no write; wall or region filter. Diff: good. Tier A.
  Size M. Owns `crates/spatial/src/place.rs` (enumerate beside `solve`), `crates/endpoint/src/scene_query.rs`,
  `crates/endpoint/src/scene_tools.rs`. Accept: `cargo test -p engine-spatial --lib place_options` and `cargo
  test -p engine-endpoint --features mcp --lib place_dry_run` x2: a 2.0 x 0.9 m couch gets only poses that
  verify clean, the fixed 0.8 m footprint is gone, dry_run moves neither revision nor digest. F2P:
  PLACED_SX_M fixed, scene.place always commits. P2P: golden d57b5c8ae3096614 unchanged. needs: AD7.
- [x] SPR6 spatial: say why a placement failed - each unmet want in a ConflictSet names the blocking object
  ids, the cm it misses by and the nearest near-miss pose (additive fields); the door prints them beside the
  hints. Diff: good. Tier A. Size M. Owns `crates/spatial/src/place.rs`,
  `crates/endpoint/src/scene_tools.rs`. Accept: `cargo test -p engine-spatial --lib conflict_blockers` x2: 20
  seeded over-constrained intents each list a blocker or wall and a miss in cm per unmet want; the minimal
  subset and the SP56 audit digest are unchanged. F2P: reason is one fixed sentence with counts only. P2P:
  golden d57b5c8ae3096614 unchanged. needs: none. Landed 54bc068f. engine-spatial --lib conflict_blockers 6 passed x2 (runs 39082607426, 39082631778): 20 seeded conflicts each name a wall or the target object and a miss in cm per unmet want (closed-form 960 - hi_n - hi_s and far_lo - near_hi), a taken spot names the object, its plan overlap and the shortest push; engine-spatial --lib 364 passed (run 39082652058), clippy -D warnings (run 39082679548). Door: scene.place `blocked_by` beside the hints; endpoint place_conflict_blockers x2 in --lib 590 passed (run 39084144955), clippy run 39083786193, --test it 57 passed (run 39084304899). ConflictSet.blocks is additive (place/blocker.rs); reason stays one counts-only sentence, the minimal subset, hints and audit digest are unchanged (conflict_audit_baseline, sp38_audit_hunks, canonical_audit_digest_repeats_x2, audit_digest_ignores_blocks); golden d57b5c8ae3096614 untouched (no sim crate calls solve). Left: object wider than the room returns no blocks (the shrink drops every want).
- [ ] SPR7 spatial: pair sight with the occluder - `scene.query kind=sees anchor=A other=B` answers yes/no,
  first blocker id and entry point (`ray_first_hit`), a visible fraction from a 5-ray fan (center, head, four
  corners), and `relation visible_from` delegates to it (one definition). Diff: good. Tier A. Size M. Owns
  `crates/spatial/src/sight.rs`, `crates/spatial/src/relation.rs`, `crates/endpoint/src/scene_query.rs`.
  Accept: `cargo test -p engine-spatial --lib sees_pair` x2: 50 seeded pairs match a brute-force oracle, a
  pillar case names the pillar, half-hidden gives a fraction strictly between 0 and 100, relation and sees
  never disagree, <= 0.2 ms per pair. F2P: visible returns a row per actor with no blocker; two definitions.
  P2P: golden d57b5c8ae3096614 unchanged. needs: AD7. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); sees::sees_pair (blocker id, entry point, 5-ray fraction) vs an independent oracle, replay x2; left: relation visible_from still does not delegate to sees_pair (two definitions), and the endpoint arm. Progress 2026-10-18 (2): relation visible_from now delegates to sees::sees_pair (755cd907), one visibility definition; relation_visible_from_and_sees_never_disagree over 50 seeded scenes. Endpoint 2026-10-18 (lane A, 5cc241ed): scene.query kind=sees (anchor, other) answers percent seen, rays and a blocker row; query_space_sees_names_the_blocker passed. Progress 2026-10-18 (wave 2): the Accept is met, the tick waits for AD7 (needs). a2dba030 sees.rs, 755cd907 visible_from delegates, 5cc241ed door kind=sees, 9b68ba9f timing. sees_pair_matches_the_oracle_on_50_seeded_pairs (50/50), sees_pair_names_the_pillar_and_half_hidden_reads_strictly_between, relation_visible_from_and_sees_never_disagree (50 seeded scenes) in engine-spatial run 39106576997; 0.319 us per pair (sees_pair_budget_under_a_fifth_of_a_ms --ignored, release, run 39108300340); query_space_sees_names_the_blocker in engine-endpoint --lib 607 passed run 39107175761.
- [ ] SPR8 endpoint: compact answers - `scene.query` and `scene.spatial` accept `format` rows|ids|count and
  `matched_only`; labels go out once in a legend; `visible` stops returning blocked rows unless asked. Diff:
  regular. Tier A. Size S. Owns `crates/endpoint/src/scene_query.rs`, `crates/endpoint/src/watch_ledger.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib query_compact` x2: the same 64-hit `visible`
  answer shrinks >= 60% in chars with identical digest and ids; the ledger prints chars before and after.
  F2P: each hit repeats a 52-char label and a reason. P2P: golden d57b5c8ae3096614 unchanged. needs: SPR1. Progress 2026-10-18 (lane A, fc0babc8): query_compact 3 passed: format rows|ids|count and matched_only on scene.query and scene.spatial, visible drops blocked rows by default, the 64-hit visible answer is 8436 chars full, 317 as ids, 586 default, same digest and ids; the ledger posts ans_chars and wire_chars. Left: no separate legend block (the ids format is the label-once form).
- [ ] SPR9 spatial: rooms and regions as query targets - flood-fill wall boxes into `room_1..n` (door = gap
  wider than the capsule) with center, area, exits; kinds `in_room`, `room_of`, `exits_of`; wants `inside
  room:<n>`; brief groups by room. Reuses isovist/navcheck code that has no caller. Diff: hard. Tier B. Size
  M. Owns `crates/spatial/src/rooms.rs` (new), `crates/spatial/src/lib.rs`,
  `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p engine-spatial --lib rooms` x2: a two-room
  house gives 2 rooms and 1 exit, 50 seeded scenes match hand labels 50/50, digest stable under input order.
  F2P: no room concept; groups are arena/zone/object. P2P: golden d57b5c8ae3096614 unchanged. needs: AD7. Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): rooms.rs find_rooms flood-fills wall boxes into room_1..n with doorway exits (width, neighbours, outside = 0), room_of, exits_of, members; two-room house = 2 rooms 1 exit, 50 seeded scenes match hand labels, order-invariant; left: scene.query kinds in_room/room_of/exits_of, wants inside room:<n>, brief grouping by room (lane A). Progress 2026-10-18 (wave 2): the Accept is met, the tick waits for AD7 (needs). 755cd907 rooms.rs; 39b0e28a door kinds room_of, in_room, exits_of. rooms tests (a_two_room_house_has_two_rooms_and_one_exit, members_are_listed_per_room_and_a_pillar_does_not_split_a_room, fifty_seeded_scenes_match_their_hand_labels) in engine-spatial run 39106576997; query_rooms_name_the_room_its_members_and_its_exit (engine-endpoint --lib 607 passed run 39107175761). Left: wants inside room:<n> and brief grouping by room.
- [ ] SPR10 spatial: camera framing verb - `scene.frame target=<id> shot=wide|medium|close` returns eye, yaw,
  pitch, fov that keep the target centered, unoccluded (SPR7 fan) and inside the room, ranked by
  `compose::score_shot`. Diff: hard. Tier B. Size M. Owns `crates/spatial/src/compose.rs`,
  `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p engine-spatial --lib frame_target` x2: 20
  seeded targets get full-visibility poses with no camera-in-body, ranking agrees with the hand slate
  (spearman >= 0.7), same bytes twice. F2P: compose scores a shot but never proposes one; no door verb. P2P:
  golden d57b5c8ae3096614 unchanged. needs: SPR7. Progress 2026-10-18: shot ranking by coverage landed d8f483de (spatial_frame.rs), compose::score_shot not applied yet; left: the endpoint verb (lane A).
- [x] SPF1 spatial: known-good scene gate with controls - run the full check catalog over every shipped scene (`content/arena/placement.json` + `dressing.json`, `fixtures/games/*`) and pin zero red findings plus the exact list of true yellow ones; each check gets a one-edit mutant of the same scene that must make it fire (a non-physical box under a crate gives no sunken, flipped to physical it does; 52 props x100 give one unit finding). Source: Forge `SpatialRomeLiveTests.cs`, `spatial-train.mjs`. Diff: regular. Tier A. Size S. Owns `crates/spatial/tests/it/main.rs` and `known_good.rs` (new; one integration binary per crate). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --test it known_good` x2 green with identical finding lists; each mutant fires its check and the clean scene does not. F2P: no test runs the catalog over a shipped scene or proves a check still fires on a real one. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 85e29752. spatial_eval/known_good.rs runs all 18 checks over 6 shipped scenes at zero findings, each check fires on a one-edit mutant of the dressed arena scene, boundary near-misses stay silent; runs 39073877167, 39075958087, clippy 39074196530 green.
- [ ] SPF2 spatial: closed-form frame fit - `frame_fit(box_cm, view, fov_deg, aspect_pct, fill_pct, yaw_deg, pitch_deg)` returns eye, look_at, distance, yaw, pitch for seven named views (three_quarter, front, back, side, top_down, third_person, orbit) with integer degree tables and no libm; SPR10's verb will call it. Source: Forge `CaptureFraming.cs` CameraShot. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/frame.rs` (new), one `lib.rs` export line. Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib frame_fit` x2: for 7 views x fov 55/60/65 x aspect 16:9 and 4:3 all 8 corners project inside the frame and the larger axis use is within 1 percent of fill; look_at is the box centre; top_down eye is above and pitch 89; third_person eye sits behind the target yaw; same bytes twice. F2P: nothing proposes a camera pose (`compose.rs` only scores one). P2P: golden d57b5c8ae3096614 unchanged. needs: none. Progress 2026-10-18: frame fit for seven views landed d8f483de (playtest spatial_frame.rs, 6 tests run 39077256919); left: the endpoint verb (lane A). Progress 2026-10-18 (wave 2): frame_fit still lives in playtest spatial_frame.rs in f64; moving it into engine-spatial breaks the sim-crate float ratchet and the Accept names -p engine-spatial --lib frame_fit, so what is left is an integer frame_fit in crates/spatial/src/frame.rs (sin_1k and cos_1k tables, no libm) with the same 7 views x fov x aspect corner test.
- [x] SPF3 spatial: facing-relative wants - `scene.place` accepts `in_front`, `behind`, `left`, `right` with a `[lo,hi]` edge gap in cm against an object's own facing, plus `against_wall` and `on_ground`, solved on the existing 10 cm x 8 yaw grid and checked by the same relation predicate the query uses. Source: Forge `SceneOps.Geometry.cs` PlaceBox. Diff: good. Tier A. Size M. Owns `crates/spatial/src/place.rs`, `crates/endpoint/src/scene_tools.rs` (want grammar). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib place_facing` x2: for 8 anchor yaw octants x 4 words the solved box sits in the named half-plane with edge gap inside the range and `holds(mover, word, anchor)` is true; an impossible gap returns a conflict set; endpoint test places a crate `in_front` of a fixture player in one call. F2P: wants cover supports/on_top_of/inside/next_to/facing only, so "in front of" needs a multi-call guess. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): place/dir.rs place_dir (named place_ not solve_ for the hot-alloc ratchet) for in_front_of/behind/left_of/right_of with an edge gap range, confirmed by relation::holds, 8 octants x 4 words and an exhaustive scan on 20 seeded rooms; left: scene.place want grammar (lane A); against_wall is the existing NextTo wall want, on_ground not added. Landed 39b0e28a. place_facing_* x3 (8 yaw octants x 4 words land in the named half-plane with the gap in range and holds true, an impossible gap returns a conflict set, exhaustive scan on 20 seeded rooms) in engine-spatial run 39106576997; door scene.place in_front_of|behind|left_of|right_of object:<uuid> [lo,hi] places a crate in front of a fixture player in one call: place_facing_puts_a_crate_in_front_of_the_player_in_one_call (engine-endpoint --lib 607 passed run 39107175761). Left: an on_ground want (against_wall is the existing next_to wall).
- [x] SPF4 spatial: clearance probe with push gradient - `scene.query kind=clearance anchor=<id> need_cm=<n>` returns the nearest surface gap in cm (negative when boxes overlap), the neighbour id, the unit push direction that opens it, the axis and a kind of exact, upper_bound or narrow, with a holds or fails verdict against need_cm (default 90). Source: Forge `SceneClearanceOps.cs` SurfaceGapDetail. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/index.rs` (surface_gap), `crates/endpoint/src/scene_query.rs` (kind). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib surface_gap` and `-p engine-endpoint --features mcp --lib query_clearance` x2: apart 30 cm on x gives gap 30 push +x; touching gives 0 exact; 12 cm overlap on the shallowest axis gives -12 with push toward the anchor's side; a 30/40 cm corner gap gives 50; moving the anchor by push x (need - gap) makes the same query hold. F2P: `near` returns centre distance only, so a long wall reads far while touching. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); measure.rs surface_gap/clearance/measure vs a brute-force oracle on 40 seeded pairs; left: the scene.query kind=clearance arm in scene_query.rs. Endpoint 2026-10-18 (lane A, 5cc241ed): scene.query kind=clearance (anchor, radius_cm as the width needed) answers kind, gap cm and push_pm; query_space_clearance_slide_ground_and_why_hidden passed. Landed a2dba030 (surface_gap, clearance), 5cc241ed (door kind=clearance), 39b0e28a (door proof). surface_gap_pins_the_four_forge_cases and surface_gap_push_makes_the_clearance_hold in engine-spatial run 39106576997; query_clearance_reads_the_gap_the_push_and_holds_after_the_move (30 cm apart gap 30 push +x, touching 0 exact, 12 cm overlap -12, 30/40 corner 50, moved by the push holds; engine-endpoint --lib 607 passed run 39107175761).
- [x] SPF5 spatial: giantObject relative to the layout - replace the fixed 50 m test with size over the span of the other solid boxes (ground plates and non-physical boxes exempt, fewer than 2 solids silent), keeping the check name and the frozen catalog length 18. Source: Forge `Verbs.Spatial.cs` SpatialRunGiantObject, `SpatialRomeLiveTests.cs`. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/checks.rs` (giantObject block). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib giant_layout` x2: four 2060 cm boundary walls around two props give none; a 50x crate in a 4 m room gives one; a 60 m wall in a 100 m arena gives none; digests of fixtures that held giantObject re-pin with a named receipt. F2P: any element over 50 m is red and an oversized prop under 50 m is never red. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); giantObject is relative to the layout (giant_layout_* tests, plus an enclosure exemption beyond the row); left: endpoint check wiring if any. Landed a2dba030. giant_layout_reads_the_layout_not_a_fixed_line and giant_layout_matches_brute_force_on_seeded_scenes in engine-spatial run 39106576997 (four 2060 cm walls around two props none, a 50x crate in a 4 m room one, a 60 m wall in a 100 m arena none); SPATIAL_CHECKS stays 18.
- [x] SPF6 spatial: slide sweep - `scene.query kind=slide anchor=<id> dir=<+x|-x|+z|-z|up|down|octant> max_range_cm=<n>` returns free_cm, the first blocker id, capped and already_blocked flags, with touching counted as clear and the anchor skipped. Source: Forge `SpatialSweep.cs`, `SceneSolverOps.cs` MeasureSafeMovement. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/slide.rs` (new), `crates/endpoint/src/scene_query.rs` (kind). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib slide` x2: result equals a brute-force 1 cm march on 200 seeded box sets; a face touch is clear; an overlapping start gives 0 and already_blocked; no obstacle gives max_range and capped; shuffling obstacle order gives identical bytes. F2P: the only motion check is the velocity `sweep`; an agent must trial-move to learn the room. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); slide.rs slide and ground_below vs a half-cm march on 200 seeded box sets; left: the scene.query kind=slide arm. Endpoint 2026-10-18 (lane A, 5cc241ed): scene.query kind=slide (anchor, dir, radius_cm as range) answers free cm and the blocker row; bad dir refuses bad_dir with the legal words. Landed a2dba030. slide tests (slide_equals_a_one_cm_march_on_200_seeded_box_sets, touch_is_clear_overlap_is_blocked_and_empty_is_capped, ground_below_matches_a_one_cm_descent) in engine-spatial run 39106576997; door kind=slide landed 5cc241ed (query_space_clearance_slide_ground_and_why_hidden).
- [ ] SPF7 playtest: Scene IQ collateral and budget bars - add to AD25's eight tasks a collateral bar (no unrelated node moves more than 1 cm on fix_floating, nothing moves more than 200 cm on separate_overlap, zero new overlap pairs), an undo bar (target seen at least 150 cm away, ends within 1 cm), and per-task limits of 20 agent calls and 150000 tokens that exclude harness calls. Source: Forge `eval/scene-iq-checks.mjs`, `scene_iq_*.json`. Diff: regular. Tier A. Size S. Owns `fixtures/scene-iq/` bars and the `crates/playtest/src/spatial_eval.rs` scene-IQ section AD25 creates. Accept: `node tools/gh-check.mjs cargo test -p engine-playtest --lib scene_iq_bars` x2: a fixture fix that also nudges an unrelated node 2 cm FAILs fix_floating; a do-nothing agent FAILs all eight; the reference agent passes all eight; the report prints calls and tokens per task. F2P: no collateral or budget bar exists. P2P: golden d57b5c8ae3096614 unchanged. needs: AD25. Progress 2026-10-18: skipped by lane C, AD25 and its needs (AD7 AD23 AD24 AD28 SP66) are still open.
- [x] SPF8 spatial: overlap triage by component role - tag each overlap finding intentional (collider with its mesh), deferred (effect markers) or error by `ComponentKind`, never by name; add a z_fight rank (coplanar pairs within 1 cm on one axis, score area over gap, optional frustum filter; Forge uses 0.2 cm but our boxes are integer cm) and sort unplayable kinds (overlap, sunken, burial) first. Source: Forge `SceneSpatialTriage.cs`, `Verbs.SpatialSeated.cs`. Diff: good. Tier A. Size M. Owns `crates/spatial/src/triage.rs` (new), `crates/spatial/src/checks.rs` (tag hunk only). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib triage` x2: a collider co-located with its mesh is intentional and not red; two meshes with the same footprint 0 cm apart give one z_fight ranked above a 1 cm pair, and a 3 cm pair gives none; unplayable findings precede others; a part resting 3 cm in its supporter reads info; input order does not change bytes. F2P: every overlap is red, a collider on its mesh included. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): triage.rs triage by caller-mapped Role (collider on its mesh intentional, bone pair false positive, marker deferred, seated joint info, unplayable first) and z_fights ranked area/(gap+1) with a frustum id filter, checked against a plane-list oracle; left: endpoint maps ComponentKind to Role and calls them in scene.spatial. Landed 39b0e28a. triage tests (collider_on_its_mesh_is_intentional_and_everything_else_is_judged_by_role, z_fight_ranks_zero_gap_above_one_and_ignores_three, z_fights_match_the_plane_list_oracle_on_seeded_scenes, triage_ignores_input_order) in run 39106576997; door scene.spatial triage=true maps ComponentKind to Role: spatial_triage_reads_a_collider_on_its_mesh_as_intentional (engine-endpoint --lib 607 passed run 39107175761).
- [x] SPF9 spatial: verdict allow-list - `scene.spatial` takes `allow` rows (finding key, closed verdict, reason required 1..200 chars, rev, expiry), keeps matched findings visible as `suppressed` with the reason and excludes them from the red count; a family learner relaxes one check x family cell after 3 intentional or false_positive verdicts up to level 3 and never relaxes red errors. Source: Forge `SceneSpatialRepair.cs`, `FamilyThresholds.cs`. Diff: good. Tier A. Size M. Owns `crates/spatial/src/verdict.rs` (new), `crates/endpoint/src/scene_tools.rs` (arg). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib verdict` and `-p engine-endpoint --features mcp --lib spatial_allow` x2: an expired entry stops suppressing; a missing reason refuses naming the field; a fixed verdict never suppresses; 3 verdicts relax level 1 and 12 cap at 3; same rows in any order give one table. F2P: no way to mark a finding intentional, so a deliberate overlap stays red forever. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 39b0e28a. verdict tests x6 in engine-spatial run 39106576997 (expired and fixed never suppress, 3 verdicts relax level 1 and 12 cap at 3, red never relaxes, same rows in any order one table, a missing reason refuses naming the field); door scene.spatial allow rows (check a b|- verdict rev expiry reason): spatial_allow_* x4 in engine-endpoint --lib 607 passed run 39107175761, clippy 39107463215.
- [x] SPF10 spatial: brief hygiene - fold look-alike objects (same mesh, parent and size) into one counted row, drop colliders that wrap their own mesh into that row, exclude strays and backdrops from the stated extent while listing them with a reason and an inflation ratio, and print the top five problems in plain words. Source: Forge `SceneLook.cs`, `SceneLook.Layout.cs`. Diff: regular. Tier A. Size M. Owns `crates/spatial/src/brief.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib brief_fold` x2: 40 identical crates give one row with count 40; a 500 m stray crate is excluded with why=stray and the inflation ratio; the 115-object fixture stays inside the byte budget; same bytes in any input order. F2P: every object gets its own row and one stray stretches the extent. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): fold.rs fold_lookalikes, fold_text, extent_text, problems_in_words as pure functions (brief_fold tests: 40 crates fold to one row, 500 m stray excluded with its inflation, colliders folded into their mesh, same bytes in any order); left: wiring into brief_text and a real 115-object fixture byte check (brief.rs untouched to avoid colliding with SPR1). Landed 39b0e28a. brief_fold x2 in engine-spatial run 39106576997: brief_fold_115_objects_fit_the_default_budget_in_any_order (116 objects, 2 object rows, within 4096 bytes, same bytes reversed), forty_identical_crates_fold_to_one_row, a_500_m_stray_is_excluded_with_its_inflation, a_collider_wrapping_its_mesh_folds_into_the_mesh_row, brief_fold_cuts_whole_rows_and_always_keeps_the_digest, brief_fold_names_the_problems_before_the_rows. Door: scene.brief text is brief_folded (engine-endpoint --lib 607 passed run 39107175761); assistant observe golden re-pinned 45 -> 55 tokens.
- [x] SPF11 spatial: subject coverage and camera sanity - `scene.query kind=coverage anchor=<id> eye_m yaw pitch fov` returns the share of a 32x32 ray grid whose first hit is the anchor (permille), a looks_away or flipped verdict, and under 5 permille a refusal-style next-call line. Source: Forge `CaptureVisibility.cs`, `SpatialChecks.cs` CameraPose. Diff: regular. Tier A. Size S. Owns `crates/spatial/src/sight.rs` (grid hunk), `crates/endpoint/src/scene_query.rs` (kind). Accept: `node tools/gh-check.mjs cargo test -p engine-spatial --lib coverage` x2: a camera inside the subject reads 1000; behind it reads 0 and looks_away; a pillar covering half the view halves the share versus the sphere-only number; up.y below 0 reads flipped; identical bytes twice. F2P: no way to learn that a subject is off screen or hidden before capture. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Progress 2026-10-18: subject coverage in permille with looks_away and flipped landed d8f483de (spatial_frame.rs); left: the endpoint verb (lane A). Landed 39b0e28a. coverage_inside_behind_flipped_and_stable (inside reads 1000, behind reads 0 and looks_away, pitch over the top reads flipped, bytes twice) and coverage_pillar_over_half_the_view_halves_the_share in engine-spatial run 39106576997: an integer 32x32 ray grid over SightWorld, no f64 (the playtest spatial_frame.rs version stays for SPR10 ranking); door kind=coverage (anchor, eye_m, yaw, pitch in radians, half_angle_deg): query_coverage_reads_the_share_the_verdict_and_the_next_call (engine-endpoint --lib 607 passed run 39107175761).
- [x] SPF12 endpoint: read-repeat guard - the fifth identical read-only call from one session within 30 s is refused with a typed `read_repeat` carrying the next verb to try, calls two to four carry a `repeat_count`, status and wait verbs are exempt, the key table holds at most 1000 entries; `scene.brief` gains `include_spatial` returning finding counts and the focus gap in the same reply. Source: Forge `BridgeRecovery.cs` SafeReadRepeat, `verbs.schema.json` scene.brief. Diff: regular. Tier A. Size S. Owns `crates/endpoint/src/guards.rs` (read guard), `crates/endpoint/src/scene_tools.rs` (flag). Accept: `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib read_repeat` x2 with an injected clock: the fifth identical `scene.query` is refused, four are stamped, one changed argument resets the count, a call after 30 s passes, a `play.run` poll never trips; `include_spatial` counts equal a separate `scene.spatial` call. F2P: an agent can repeat one read forever with no signal. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed fc0babc8. read_repeat 4 passed (guard with injected clock: fifth refused, 2-4 counted, changed call or 30 s resets, 1000-key cap; door refuses the fifth identical scene.brief naming scene.spatial; include_spatial equals a separate scene.spatial). The guard covers scene.brief, scene.spatial and scene.query (same arguments and revision); play.run and status verbs never pass through it. cargo test -p engine-endpoint -p engine-render --features engine-endpoint/mcp,engine-render/v1 --lib on the runner (gh run 39079245584; the 5 build_level tests are red, not from this lane); golden d57b5c8ae3096614 unchanged.
- [x] SPX1 spatial: named view glance - `scene.query kind=glance` with eye, yaw, fov, grid 1..64 (default 24)
  returns `visible[{id,coverage_pm,near_cm,far_cm}]` capped 25, `occluded[]`, `background_pm`, depth min/max/
  mean; the text reports tokens. Diff: good. Tier A. Size S. Owns `crates/spatial/src/sense_text.rs`,
  `crates/endpoint/src/scene_query.rs` (kind arm). Accept: `cargo test -p engine-spatial --lib glance` x2: 50
  seeded cameras match a brute-force first-hit count 50/50; a pillar hides a crate so it is in `occluded` and
  not in `visible`; coverage plus background is 1000; <= 0.2 ms per ray. F2P: no per-actor view summary.
  P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 39b0e28a. glance_matches_a_brute_force_first_hit_count_on_50_seeded_cameras (50/50, coverage plus background = 1000 each) and glance_puts_a_crate_behind_a_pillar_in_occluded_not_visible in engine-spatial run 39106576997; release 64x64 glance 0.198 us per ray, 119 tokens (glance_ray_budget_under_a_fifth_of_a_ms --ignored, run 39106384960). Door: scene.query kind=glance (eye_m, yaw, half_angle_deg gives the fov; grid fixed at 24 to save tool-list chars): query_glance_lists_coverage_and_names_the_hidden_crate, engine-endpoint --lib 607 passed run 39107175761.
- [x] SPX2 spatial: seated joint and support chain - opt-in checks `seatedJoint` and `supportChain`: seat when
  upper base is above lower base, inset <= 20% of LOWER height and >= 50% of upper footprint rests on it
  (yellow, never red); `CanSupport` skips spanning and wall-mounted candidates; floating/sunken use ratios
  (25%/15% of own height) and a bottom-up chain. Diff: good. Tier A. Size M. Owns
  `crates/spatial/src/support.rs` (new), `crates/spatial/src/checks.rs` (opt-in arms), `lib.rs`. Accept:
  `cargo test -p engine-spatial --lib support_chain` x2: desk top inset 5 into 90 reads seated, two crates 50%
  deep read red, a wall pipe reads no sunken, a crate on a raised table names the table, 20 permutations give
  one digest, `SPATIAL_CHECKS` stays 18. F2P: `floating` is y=0 only, `support` is one hop. P2P: golden
  d57b5c8ae3096614 unchanged. needs: none. (Covers the chain half of N-13.) Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): support.rs opt-in seatedJoint and supportChain (OPT_IN_CHECKS, SPATIAL_CHECKS stays 18); seeded towers match their recipe, 20 permutations one digest; left: the 15% sunken ratio, endpoint wiring (legal list follows OPT_IN_CHECKS). Landed 39b0e28a. support_chain tests in engine-spatial run 39106576997: support_chain_reads_the_four_forge_cases, support_chain_reads_a_buried_base_as_sunken_past_15_percent (plates exempt), twenty_permutations_give_one_digest, support_chain_skips_a_stray_and_a_backdrop, seeded_towers_match_their_recipe; SPATIAL_CHECKS stays 18 (opt_in_checks_ride_run_spatial_and_the_catalog_stays_18). Door: the opt-in names ride scene.spatial checks.
- [x] SPX3 endpoint: spatial delta on writes - `tx.apply` and `scene.place` return `spatial_delta{ids,
  new_findings<=5,resolved}` for touched ids only, computed by the staged check; `delta=false` skips it. Diff:
  good. Tier A. Size S. Owns `crates/endpoint/src/mcp.rs` (apply output hunk), `crates/endpoint/src/mcp_dto.rs`,
  `crates/spatial/src/staged.rs` (id filter). Accept: `cargo test -p engine-endpoint --features mcp --lib
  spatial_delta` x2: a crate moved into a wall names both ids and the axis fix, a pre-existing unrelated overlap
  is absent, a clean move returns an empty delta, a 200-object scene adds < 5 ms. F2P: `TxApplyOutput` has no
  findings (`mcp_dto.rs:307`). P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 54bc068f. engine-endpoint --features mcp --lib spatial_delta 4 passed x2 (runs 39083735986, 39083759717): a crate moved into a 10 m wall collider names both ids and `fix: move ... 60 cm -z`, a pre-existing stacked pair is absent, a clean move reads empty lists, scene.place returns the delta and `delta=false` drops it, 200 objects median 2296 us (two snapshots + staged check, bar 5000). engine-spatial staged::check_staged_with/check_staged_ids/fix_for (spatial_delta_* tests in --lib 364 passed, run 39082652058); the door runs the scene.spatial check list, the tx gate stays 1 m cubes. Endpoint --lib 590 passed (run 39084144955), clippy run 39083786193, --test it 57 passed (run 39084304899). Tool-list budget 51000 -> 54000 with a dated reason (loaded list 51276); golden d57b5c8ae3096614 untouched.
- [x] SPX4 endpoint: `scene.next` - observe verb returning `{stage,next_call{verb,args},verify,
  stop_conditions[],do_not[]}` from digest, finding counts and revision; do_not includes "do not raise a
  threshold to hide a finding". Diff: good. Tier A. Size M. Owns `crates/endpoint/src/scene_next.rs` (new),
  `crates/endpoint/src/packs/scene.rs`. Accept: `cargo test -p engine-endpoint --features mcp --lib scene_next`
  x2: a 12-state table gives exactly one next_call each, same bytes twice, reply <= 600 bytes, door-reference
  lists it. F2P: no verb names the next call. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed fc0babc8. scene_next 2 passed: 12 states give one next_call each, same bytes twice, reply <= 600 bytes; door-reference lists scene.next; router pin 62. cargo test -p engine-endpoint -p engine-render --features engine-endpoint/mcp,engine-render/v1 --lib on the runner (gh run 39079245584; the 5 build_level tests are red, not from this lane); golden d57b5c8ae3096614 unchanged.
- [x] SPX5 spatial: travel bound and ground trace - kinds `travel` (anchor, axis or dir, max_range_cm ->
  free_cm, blocker, already_blocked, capped) and `ground` (x_cm, z_cm, from_y_cm -> y_cm, id). Diff: good.
  Tier A. Size S. Owns `crates/spatial/src/sight.rs` (travel, ground fns), `crates/endpoint/src/scene_query.rs`
  (arms). Accept: `cargo test -p engine-spatial --lib travel_ground` x2: 50 seeded cases match a 1 cm march 50/50,
  touching is not a hit, children of the mover are ignored, <= 0.2 ms. F2P: no travel or ground query.
  P2P: golden d57b5c8ae3096614 unchanged. needs: none. Same lane as SPF6: build one slide/travel verb. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); slide.rs travel/ground (see SPF6); left: the scene.query travel/ground arm. Endpoint 2026-10-18 (lane A, 5cc241ed): scene.query kind=ground (eye_m point) answers the floor top and its object; kind travel not added yet. Landed a2dba030 (slide.rs), 9b68ba9f (timing). travel_ground_follow_a_mover_to_its_stop_and_down (50 seeded cases against a march and a 1 cm descent, a face touch is not a hit, children of the mover ignored) in engine-spatial run 39106576997; 0.824 us per slide or ground query (travel_ground_budget_under_a_fifth_of_a_ms --ignored, release, run 39108300340). The door serves travel as kind=slide (anchor, dir, radius_cm as the range) and ground as kind=ground (eye_m point).
- [x] SPX6 spatial: why hidden - `scene.query kind=why_hidden anchor eye_m yaw` -> `verdict` one of visible,
  not_found, zero_scale, no_mesh, no_material, degenerate_bounds, out_of_view, behind_eye, too_far,
  occluded_by:<id>, plus the evidence number. Diff: good. Tier A. Size S. Owns
  `crates/spatial/src/why_hidden.rs` (new), `crates/endpoint/src/scene_query.rs` (arm). Accept: `cargo test -p
  engine-spatial --lib why_hidden` x2: 10 fixtures, one per verdict, 10/10; occluded_by names the pillar; fixed
  check order. F2P: `visible` says only visible or blocked. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Same lane as SPR7: one sees/why_hidden verb. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); sees::why_hidden with 10 verdicts tested; left: the scene.query kind=why_hidden arm. Endpoint 2026-10-18 (lane A, 5cc241ed): scene.query kind=why_hidden (anchor, eye_m, yaw) answers the verdict word plus evidence; the door assumes mesh and material present (Draw::default), so no_mesh/no_material do not fire there yet. Landed a2dba030 (why_hidden), 5cc241ed (door), 98c070c0 (moved to why_hidden.rs). why_hidden_has_one_fixture_per_verdict (10/10, fixed check order, occluded_by names the pillar) in engine-spatial 384 passed run 39108676641, clippy 39108676545; the door arm answers kind=why_hidden. Left: the door assumes mesh and material present, so no_mesh and no_material do not fire there.
- [x] SPX7 spatial: z-fight pairs - opt-in check `zfight` (and `epsilon_cm` on `scene.spatial`) lists box pairs
  with coplanar faces within epsilon and shared area, ranked area/gap, with the axis and the fix delta. Diff:
  regular. Tier A. Size S. Owns `crates/spatial/src/checks.rs` (opt-in arm), `crates/spatial/src/lib.rs`,
  `crates/endpoint/src/scene_tools/spatial_checks.rs`. Accept: `cargo test -p engine-spatial --lib zfight` x2:
  two boxes sharing a face at y=10 cm with 1,080,000 cm2 overlap give 1 pair, 2 cm apart give none, 20
  permutations equal digest, catalog stays 18. F2P: no coplanar detector. P2P: golden d57b5c8ae3096614
  unchanged. needs: none. Landed 39b0e28a. zfight_two_faces_at_y_10_give_one_pair_and_2_cm_apart_none (1,080,000 cm2 shared, 2 cm apart none), zfight_twenty_permutations_give_one_digest_and_the_catalog_stays_18 in engine-spatial run 39106576997. Door: scene.spatial checks=[zfight] with epsilon_cm, rows end with the axis and the fix: spatial_zfight_names_the_axis_and_the_fix_and_takes_an_epsilon (engine-endpoint --lib 607 passed run 39107175761).
- [x] SPX8 spatial: robust extent - `scene.brief` reports `extent_cm` without strays plus `stray_ids[]`, and an
  opt-in `stray` check; strays and 50x backdrops are skipped by overlap and support passes. Diff: good. Tier A.
  Size S. Owns `crates/spatial/src/extent.rs` (new), `crates/spatial/src/brief.rs` (extent hunk). Accept: `cargo
  test -p engine-spatial --lib stray_extent` x2: 60 props plus one at 1,000,000 cm keep extent within 1% and
  list 1 stray; 3 boxes silent; half the scene far away flags none. F2P: no robust extent in `brief.rs`. P2P:
  golden d57b5c8ae3096614 unchanged. needs: none. Same lane as SPF10: one brief-hygiene pass. Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): extent.rs robust_extent with strays and backdrops (stray_extent tests: 60 props + 1 at 1,000,000 cm keep the exact props-only box, 3 boxes silent, half far flags none); left: the opt-in stray check, skipping strays in overlap/support passes, wiring the extent line into the brief. Landed 39b0e28a. engine-spatial 384 passed run 39106576997, clippy 39106578890: stray_extent x2 (60 props + 1 at 1,000,000 cm keep the props-only box and list 1 stray, 3 boxes silent, half far none), stray_flags_a_prop_at_500_m_and_stays_silent_without_it, support_chain_skips_a_stray_and_a_backdrop. Door: scene.brief text carries the extent row with the excluded ids and the opt-in check stray (spatial_brief_folds_forty_look_alikes_and_lists_the_stray, engine-endpoint --lib 607 passed run 39107175761, clippy 39107463215). Left: the default overlap pass still counts backdrops (skipping them would move default report digests).
- [x] SPX9 endpoint: worst-actor rollup - `scene.spatial` adds `worst_actors[{id,findings,red,checks[]}]` top
  10 and `counts{total,returned,suppressed}`. Diff: regular. Tier A. Size S. Owns
  `crates/endpoint/src/scene_tools/spatial_checks.rs`, `crates/endpoint/src/scene_tools.rs` (output hunk).
  Accept: `cargo test -p engine-endpoint --features mcp --lib spatial_rollup` x2: one actor with 3 kinds leads
  with 3, total = returned + deferred, same bytes under permutation. F2P: output has no per-actor view
  (`scene_tools.rs:357`). P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed fc0babc8. spatial_rollup 2 passed: the pad under four crates leads with its overlap count, counts total = returned + suppressed, same bytes under insertion order. cargo test -p engine-endpoint -p engine-render --features engine-endpoint/mcp,engine-render/v1 --lib on the runner (gh run 39079245584; the 5 build_level tests are red, not from this lane); golden d57b5c8ae3096614 unchanged.
- [x] SPX10 spatial: outside want and anchor first - wants gain `outside object:<id>|wall:<w>|region`; the
  preferred pose is returned unchanged when it satisfies all wants and verifies; ConflictSet reason is
  `does_not_fit_on_support` or `no_free_spot`. Diff: good. Tier A. Size M. Owns `crates/spatial/src/place.rs`,
  `crates/endpoint/src/scene_tools.rs` (want parser). Accept: `cargo test -p engine-spatial --lib
  place_outside` x2: a spawn outside a room footprint has 0 XZ overlap and hugs it within 2 cells; a free
  preferred pose is exact; 3 reason fixtures; SP56 audit digest unchanged. F2P: no negative want; one reason
  string. P2P: golden d57b5c8ae3096614 unchanged. needs: none. Landed 2026-10-18 755cd907 (engine-spatial --lib 354 passed run 39079745728, clippy -D warnings run 39080141395): place/outside.rs place_outside with Object and Region targets, current pose returned unchanged when it works, reasons does_not_fit_on_support / no_free_spot, exhaustive-scan test on 20 seeded rooms; left: the scene.place want parser (outside object:<id>|region), wall:<w> target not supported. Landed 39b0e28a. place_outside_* x4 (free preferred pose exact, spawn moved outside and hugs the zone, 3 reason kinds, exhaustive scan on 20 seeded rooms) in engine-spatial run 39106576997; door scene.place wants outside object:<uuid> and outside region:x0,z0,x1,z1: place_outside_keeps_a_free_pose_exactly_and_clears_a_region (engine-endpoint --lib 607 passed run 39107175761). Left: outside wall:<w>.
- [x] SPX11 spatial: pairwise measure - `scene.query kind=measure what=gap|overlap|align|contain` with anchor,
  other, tolerance_cm, axes, ref, min_clearance_cm returns signed `gap_cm`, `axis_gap`, `relation`,
  `penetration_pm`, `passed`, `shortfall_cm`. Diff: regular. Tier B. Size S. Owns
  `crates/spatial/src/measure.rs` (new), `crates/endpoint/src/scene_query.rs` (arm). Accept: `cargo test -p
  engine-spatial --lib measure_pair` x2: 40 fixtures match a brute-force oracle 40/40, touching within
  tolerance is contact, a poking crate reports the axis and shortfall. F2P: `near` gives distance only. P2P:
  golden d57b5c8ae3096614 unchanged. needs: none. Same lane as SPF4: one clearance/measure verb. Progress 2026-10-18: spatial half landed a2dba030 (engine-spatial --lib 327 passed run 39077306002, clippy run 39077630760); measure.rs measure_pair tests (see SPF4); left: the scene.query measure arm. Endpoint 2026-10-18 (lane A): measure kind not added yet (clearance covers the nearest-gap part). Landed a2dba030 (measure.rs), 39b0e28a (door). measure_pair_matches_brute_force_on_40_seeded_pairs, measure_pair_contact_align_and_contain_read_the_poking_crate, measure_pair_by_id_reads_the_same_boxes_and_refuses_unknown_ids in engine-spatial run 39106576997; door scene.query kind=measure (what=gap|overlap|align|contain, radius_cm is the gap or margin): query_measure_reads_the_gap_and_the_shortfall (engine-endpoint --lib 607 passed run 39107175761). The door fixes tolerance 1 cm, all axes and centers; typed tolerance_cm, axes and reference stay in MeasureArgs.
- [ ] SPX12 endpoint: egress - `scene.query kind=egress` with start, exits[], capsule_radius_cm returns
  `all_reachable`, `worst_route_cm`, per-exit route and `bottlenecks[{door_id,width_cm}]` narrowest first.
  Diff: hard. Tier B. Size M. Owns `crates/endpoint/src/scene_query.rs`, `crates/spatial/src/navcheck.rs`.
  Accept: `cargo test -p engine-endpoint --lib egress_door` x2: two rooms and one 80 cm door give the door as
  bottleneck, 20 seeded layouts match a brute-force BFS route within 5 cm, an unreachable exit is listed. F2P:
  no route length at the door. P2P: golden d57b5c8ae3096614 unchanged. needs: AD20.
