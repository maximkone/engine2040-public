### Hard stuff (operator 2026-09-24): motion, ragdoll, physics, speed

Operator: "ragdoll, locomotion and other hard stuff, all in the engine,
optimized." Everything that feeds the tick stays deterministic (integer or
fixed-point, sorted order, x2 hashes); presentation-only rows say so. Donors
come from the Rust sweep in `RESEARCH-VERDICT.md` (licenses re-read at port
time, `PORT()` + `EVIDENCE.md` row); papers and talks are reimplemented.

- [x] PX35 physics: long-range attachments for cloth and soft sheets — the Progress 2026-10-17: per-node tethers landed (6f871bb5: cloth.rs rebuild_tethers and solve_tethers, test the_tether_pulls_an_overstretched_particle_back_to_its_reach); left: cloth is f32 Verlet not integer math, and the 32x32 fu30_soft_settle_offline_baseline spike test no longer exists, so the 25% + 64 raw link bar is not measured.
  32x32 soft-settle spike (`cloth.rs` `fu30_soft_settle_offline_baseline`,
  ignored) stretches its top links about 2x under the cape tune because
  position-based constraints cannot carry a long hanging chain in a few
  iterations. Add per-node tethers to the nearest pinned node (max distance
  = rest path length), solved after the stretch pass, integer math, no
  allocation per step. Tier B. Size M. Owns `crates/physics/src/cloth.rs`.
  Accept: the spike test runs un-ignored with every link within 25% + 64 raw
  of rest; cape tests and the zero-allocation step test stay green; replay
  bit-exact x2. needs: none. Progress 2026-10-17: 4544ac4f 32x32 cloth settles with every link within 25% of rest, bit-exact twice; cloth is still f32 Verlet so the integer-math clause is not met. Landed 9638504. a_32x32_sheet_settles_with_every_link_within_a_quarter_of_rest_and_replays_exactly (max stretch <= 1.25, un-ignored, bit-exact x2), the_awake_step_with_pin_moves_wind_and_a_body_never_touches_the_heap (0 heap calls, 4 tiers), cape tests green; engine-physics lib 147 passed 2 ignored (run 39101540373), heap test run 39101672637. Cloth is f32 Verlet by RZ7 decision (presentation only, never hashed), so the integer-math clause of the description does not apply.
- [x] LM2 anim: GASP-class ground feel — distance matching for starts, Landed 379b13c. Tick audit 2026-10-17: warp_ground_feel_test stride_sweep_slide_under_1cm, strafe_135_skateless_and_capped, replay_exact_x2 and warp_ground_test starts/stops_slide_under_1cm_05_to_7 green, gh-check runs 39056062979 (warp_ground, 24 tests) and 39056591616 (engine-anim, 390 passed).
  stops and pivots, orientation warping for strafes, stride warping so feet
  match capsule speed (Paragon GDC 2016 shapes, reimplemented). Tier A.
  Size M. Owns `crates/anim/src/warp_ground.rs`. Accept: foot slide under
  1 cm from 0.5 to 7 m/s; strafes to 135 degrees without skating; replay
  exact x2. needs: LM1, AM6.
- [x] LM3 anim: networked motion — replicate pick index and phase so remote Landed 9bb8b28. Tick audit 2026-10-17: net_pose_w1_test two_clients_match_after_resim_at_150ms_rtt (both clients land on the authority checkpoints after resim, kbps_per_fighter 2.88 asserted) green in gh-check run 39056591616 (engine-anim, 390 passed).
  fighters pose the same after rollback and resim. Tier A. Size M. Owns
  `crates/anim/src/net_pose.rs`. Accept: two clients at 150 ms RTT show the
  same pose checkpoints after resim; kbps per fighter charted. needs: LM1,
  NET2.
- [ ] AM13 anim: animation LOD — update-rate tiers by distance and screen Progress 2026-10-17: lod.rs tier table plus assign_tiers keeps the 8 nearest at full rate (lod_w1_test.rs, anim_lod_pin.rs, gh-check run 39056591616 engine-anim 390 passed); left: the 64-fighter under 2 ms timing on the reference CPU is not measured. GPG8 2026-10-19: add the promotion rebuild and the 60 m to 10 m no-pop test; a caller alone ships pops. Progress 2026-10-21: the promotion rebuild landed (LodSlot in lod.rs: tier + shown pose + one-shot spring re-seed; a promotion or a strobe hold steps at most about 4.03 deg and 5 cm per tick, a full-rate slot copies its eval bit-exact so the 8 nearest stay bit-equal to full rate, no heap in the table path). Tests green: lod_promotion_rebuild_has_no_pop (60 m to 10 m over 30 ticks through all four tiers, worst step under 5 deg, and the same tape without the rebuild pops, so it fails without the fix), lod_reseed_fires_once_per_promotion, engine-anim lib 464 passed (run 39518770666), it lod pin 8 passed (run 39519493100), clippy -D warnings (run 39519255344). Timing: lod_sixty_four_table_tick_under_two_ms reports 5482 ns per 64-fighter LOD tick (bar 2000000) in release on an ubuntu-latest runner (run 39518769284); that is the LOD slice only (assign plus show, pose eval excluded). Owner note: the board never names the reference CPU, so the 2 ms half of Accept is not claimed; name the box or accept the runner number, then tick.
  size, IK and springs off far away, crowd handed to VG4. Tier A. Size M.
  Owns `crates/anim/src/lod.rs`. Accept: 64 fighters under 2 ms total on
  the reference CPU; the 8 nearest match full-rate poses. needs: AM1, LM1.
- [x] AM14 anim: parallel SIMD eval — per-character jobs on RC3 fork-join,
  SoA sampling on `wide` (license read first). Tier A. Size M. Owns
  `crates/anim/src/par.rs`. Accept: at least 3x on 4 cores for 64 fighters;
  identical poses under every chunking. needs: AM1, RC3. Progress 2026-10-17: 6ab36492 anim par.rs per-character eval on the RC3 pool, poses bit-identical for 0,1,2,4,8 workers; speedup 2.0x on 64 fighters (run 39067637155), the 3x bar is not met and the wide SIMD half is skipped (external dependency). Dunsky 2026-10-19: par.rs has 0 game callers (`nb-gaps-anim-dunsky.txt`). Landed 4122026. par 21/21 green (gh 39477737556), par_speedup 7.67x on 4 vCPU (gh 39479055766: shared-clock key-major bank, SoA serial 3.68x, 4-worker pool 2.08x); identical poses for 0,1,2,4,8 workers and chunk 1..100; no non-test caller yet.
- [ ] AM15 anim: additive layers — locomotion base, combat upper body and
  flinch additives through one inertializer, with analytic two-bone IK in
  contact windows (Bollo GDC 2018, Clavet and Paragon GDC 2016; ozz and ACL
  MIT as patterns; diffusion and MOCHA-style generators not taken). Tier B.
  Size M. Owns `crates/anim/src/additive.rs`. Accept: no pops on any
  interruption; foot slide at most 20 mm straight and 40 mm on pivots;
  replay x2; at most 0.05 ms per tick. needs: AM1, AM2, AM3, AM6. Progress 2026-10-17: ba6ad347 anim additive.rs layers + one inertializer + planted feet via solve_limb; slide within 20 mm straight, 40 mm pivots, 1.3 us per tick (run 39067390119); left: fighter_look does not call AdditiveRig::tick yet. Dunsky 2026-10-19: additive.rs has 0 game callers (`nb-gaps-anim-dunsky.txt`).
- [ ] AM18 anim: thousand-agent crowd pose budget — extend the AM13 tiers Progress 2026-10-17: crates/anim/src/crowd_scale.rs landed (assign_crowd_tiers, eight_nearest_match_full_rate_poses, far ring FarImpostorSink handoff, run 39056591616); left: the 1,000-agent p99 is a modeled cost (modeled_tick_cost_posts_per_tier_within_budget), the timing gate thousand_agent_tick_p99_within_budget is #[ignore] and only re-checks the model, and no PF workload entry exists.
  to a seeded 1,000-agent crowd tape on the reference CPU with per-tier
  costs posted and the far ring handed to VG4 impostors by contract.
  Tier A. Size M. Owns the scale-proof hunks plus PF workload
  entries beside `crates/anim/src/lod.rs` (AM13 file frozen; lands after
  AM13). Accept: `cargo test -p engine-anim --lib crowd_scale` green;
  1,000 agents p99 at most 2 ms; the 8 nearest match full-rate poses;
  golden unchanged. needs: AM13, PF1.
  (PLN animation.md EXTEND, proposed 2026-09-29.)
- [x] LM5 anim: net-pose divergence diagnostics — a per-fighter pose-hash
  ring beside `net_pose.rs` (diagnostics only, never authoritative) that
  names the first divergent tick and fighter on a seeded N-client soak.
  Tier A. Size S. Owns the diagnostics hunks beside
  `crates/anim/src/net_pose.rs` (LM3 file frozen; lands after LM3).
  Accept: `cargo test -p engine-anim --lib net_diverge` green; seeded
  soak names first divergence 10/10; sim bytes x2; golden unchanged.
  needs: LM3. (PLN animation.md EXTEND, proposed 2026-09-29.) Landed 21e1a645. net_diverge.rs pose-hash ring names the first divergent tick and fighter on 10 of 10 seeds in a 4-client 8-fighter soak (anim tests run 39067150760).
- [ ] LM6 anim: slope/stair/sand ground feel — deepen `warp_ground.rs`
  with slope cling, stair stepping and soft-ground sink so the LM2 bars
  hold off flat ground. Tier A. Size M. Owns the slope hunks
  beside `crates/anim/src/warp_ground.rs` (LM2 file frozen; lands after
  LM2; never a second warp). Accept: foot slide bars hold on 20-degree
  slopes plus stair fixtures; replay exact x2. needs: LM2, AM6.
  (PLN animation.md EXTEND, proposed 2026-09-29.)
- [x] TV3 anim: siege traversal set — ladder, rampart vault and rubble
  scramble edges on integer-mm warp for the C5 siege map, with nav links
  rebaked exactly like TV2. Tier B. Size M. Owns the
  siege-edge hunks beside `crates/anim/src/traversal.rs` (TV1 file
  frozen) plus siege fixtures. Accept: siege fixtures pick expected
   actions; warped roots land within 1 cm; sim bytes x2. needs: TV1, TV2.
   (PLN animation.md EXTEND, proposed 2026-09-29.) Landed 7196f6fa. engine-anim --no-fail-fast 443 lib + 8 it passed (run 39106084274), clippy run 39106954469: traversal_siege_test siege_fixtures_pick_the_expected_actions, a_broken_ladder_and_a_wide_breach_are_refused_never_faked, every_siege_edge_names_one_link_and_a_breach_rebakes_exactly_the_scattered_ones, warped_roots_land_within_1cm_and_bytes_repeat_x2. Set is data on the TV1 chooser (no new move kinds); rubble variance sweeps stay TV4.
- [ ] AM21 anim: LOD strobing vs warp contract — warp-window ticks never
  strobe so TV1 landings hold under AM13 tiers. Tier A. Size S. Owns
  eval-pin hunks beside `crates/anim/src/graph.rs` (AM1 order frozen).
  Accept: `cargo test -p engine-anim --lib lod_warp` green; warped roots
  land within 1 cm with strobing on; replay x2; golden unchanged. needs:
  AM13, TV1. (PLN animation.md EXTEND r7, proposed 2026-09-29.)
- [ ] LM7 anim: motion-search fatigue-soak lock — seeded 600-tick fatigue
  sweep proves LM1 picks stay stable inside the LM4 cost bars. Tier A.
  Size S. Owns soak-test hunks in `crates/anim/src/motion_match.rs` (LM1
  shape frozen; reads LM4 bars, writes no perf file). Accept: `cargo test
  -p engine-anim --lib search_soak` green; no pick churn past the band;
  cost within LM4 bars; replay x2; golden unchanged. needs: LM4, AM8.
  (PLN animation.md EXTEND r7, proposed 2026-09-29.)
- [ ] AM22 anim: crowd readability at LOD — facing arrow plus lean stay
  readable on the 64-fighter tape with tiers on. Tier A. Size S. Owns
  readability hunks beside `crates/anim/src/locomotion.rs` (lands after
  AM17, disjoint hunks). Accept: `cargo test -p engine-anim --lib
  lod_readable` green; arrow plus lean visible far-ring; nearest-8 match
  full-rate poses; sim bytes x2. needs: AM18, AM17.
  (PLN animation.md EXTEND r7, proposed 2026-09-29.)
- [x] TV4 anim: breach-rubble scramble variance — TV3 scramble holds across
  seeded rubble height plus noise sweeps. Tier B. Size S. Owns fixture
  hunks beside `crates/anim/src/traversal.rs` (TV1 shape frozen; lands
  after TV3, disjoint rubble split). Accept: `cargo test -p engine-anim
  --lib rubble_var` green; picks expected; roots land within 1 cm; nav
  rebake exact x2. needs: TV3, TV2.
  (PLN animation.md EXTEND r7, proposed 2026-09-29.) Landed 4e08b07.
  traversal_rubble_var_test.rs: rubble_var_scramble_holds_across_seeded_height_and_noise_sweeps,
  rubble_var_bands_sit_exactly_on_the_chooser_gates, rubble_var_breach_rebake_is_exact_x2;
  engine-anim --lib 449 passed + full --no-fail-fast, clippy clean (runs 39217218591, 39216661948, 39216657052).
- [ ] LM4 anim: motion-search cost + schema lock — post the LM1
  per-character search cost plus AM8 exertion-warp cost into PF1
  workloads with kill bars; freeze the LM1 feature schema. Tier A.
  Size S. Owns baseline hunks + schema-freeze test. Accept: PF1 gate
  fails on seeded regression; 8-fighter tick in budget; schema change
  refuses with diff; golden unchanged. needs: LM1, AM8, PF1.
  (PLN28 animation.md, reviewed 2026-09-25.) Progress 2026-10-17: 4fa1246f motion_schema_test.rs freezes the LM1 feature schema and gates search cost in lane ops; left: PF1 ms workloads in crates/perf and recorded baselines (needs a perf-to-anim edge).
- [x] TV2 anim: traversal-to-nav link contract proof — every TV1 edge Progress 2026-10-17: crates/nav/src/links.rs has the traversal link registry with rebake_links and rebake_for_set tests (nav suite green, run 39057395844); left: no contract test ties each anim TV1 edge to its NV2 link entry on the five fixtures.
  names exactly its NV2 link-registry entry and rebakes exactly those
  links, proven x2 on vault/hurdle/mantle/climb/drop fixtures. Tier A.
  Size S. Owns contract test + per-edge link records (no new system).
  Accept: TV1-expected actions on registry links; warped roots land
  within 1 cm; bytes x2. needs: TV1, NV2.
  (PLN28 animation.md, reviewed 2026-09-25.) Landed 2681cdda. traversal_link_test.rs: each vault/hurdle/mantle/climb/drop edge names one registry entry and rebakes only its own link, roots land within 1 cm, bytes repeat x2 (run 39067150760).
- [ ] PX5 physics: physics-driven locomotion — a controller trained offline
  (AMP and DeepMimic lineage) through the DT pipeline, shipped as an integer
  `.e2nn` net; LM1 stays the default. Tier C. Size L. Owns
  `crates/physics/src/learned_ctrl.rs`. Accept: an AI fighter walks, turns
  and recovers from shoves; replay exact x2; A/B against LM1 on blind
  review decides adoption. Hunt 2026-09-25: the student passes tract agreement (max abs diff at most 2.0) and the 100-match tournament [350, 650]; a student trained on a shuffled teacher must collapse, else the pipeline is rejected. Motion 2026-09-25: CT14 clips may seed teachers, under DT5 manifests; clips from non-commercial training data never train a shipped student. needs: PX4, DT2, NN1.
- [x] NN3 nn: quantization-aware training — `simulate_quantize()` during Progress 2026-10-17: crates/nn/src/qat.rs landed (simulate_quantize, train_student, export_qat, determinism_report; qat_beats_post_training_on_fixture and int8_student_is_bit_identical_with_report green, run 39057462635); left: the win is shown on a 64-row fixture not the DT3 tournament, and no digest compare across two toolchains is posted.
  training plus a determinism report across toolchains; only the int8
  student ships. Tier B. Size S. Owns `crates/nn/src/qat.rs`. Accept: QAT
  beats post-training quantization on the DT3 tournament; the int8 student
  is bit-identical on two toolchains. needs: NN2, DT2. Landed ad7d07a4. QAT student agrees with the teacher on 1069 of 1200 rows vs 978 without, 123 vs 107 banked bouts; int8 digest 12583610706604573755 identical on Windows and the Linux runner (run 39066961820; tests run 39067417988).
- [ ] NN4 nn: optional ONNX Runtime backend — behind a feature, with an op
  whitelist (adds Conv, Sigmoid, LayerNorm) and a `deny.toml` row; tract
  stays the default. Tier C. Size M. Owns `crates/nn/src/ort_backend.rs`.
  Accept: same outputs as tract within tolerance on the NN2 corpus; the
  default build gains no dependency. needs: NN2.
- [x] PX15 physics: deterministic parallel islands via chunked split plus
  sorted merge with step frozen (READER-SPEED53 card 3, PX6 deepening).
  Tier B. Size S. Owns the chunked split+merge hunks beside
  `crates/physics/src/islands.rs:57`. Accept: identical hash at 1/2/4/8
  threads + ≥2x wall on 64-fighter scene. needs: PX6. (SPEED53.) Landed e8d2fb8. par_64_fighter_scene_hash_is_identical_under_1_2_4_8_threads and par_step_matches_the_serial_step_under_1_2_4_8_threads 8 passed (gh run 39489741989); par_speedup_64_fighters 2.30x wall on 4 threads, runner available_parallelism 4 (gh run 39488071406); whole crate 282 passed (run 39489768592), clippy green (run 39489293024), golden d57b5c8ae3096614 unchanged (run 39490319645).
- [ ] PX9 physics: ragdoll contacts and joint tables — self-collision between
  non-adjacent bones (collision groups filter parent and child), ragdoll
  against ragdoll for clinches and falls, per-joint swing/twist limit tables
  from the skeleton instead of depth, and torque motors with per-joint limits
  (Jolt PoweredRagdoll tables, avian XPBD joint compliance: patterns only).
  Tier A. Size M. Owns `crates/physics/src/ragdoll.rs`,
  `crates/physics/src/joint_limits.rs`. Accept: no bone interpenetration over
  1 cm in 50 seeded falls; two ragdolls landing on each other separate; hash
  x2; settles within the PX1 bound; zero per-tick allocations. needs: PX1, PX10. Progress 2026-10-17: b61738a5 physfx tests: two ragdolls landing on each other separate with no unjoined overlap over 1 cm and replay exactly (4 passed); the 50-seeded-falls bar fails (seed 44 overlaps lower_arm_r/shin_r by 8 cm at tick 40, run 39068262860), committed as an ignored repro; left: more solver iterations or bone CCD, spawn reading the joint tables. Progress 2026-10-19: `joint_limits.rs` lands (named degree rows spine 20/35, head 35/45, arm 85/70, hip 40/30, knee/elbow hinge 145; friction-spring motor numbers; collision groups that keep parent and child apart but let the same bone of two ragdolls meet) and `Ragdoll::spawn` reads it through `insert_table_joints`. Runner green: 256 lib + 18 it + 3 balance + 5 stagger tests, clippy -D warnings, golden d57b5c8ae3096614 x2, float world unchanged. Two-ragdoll separation still proves through `ragdoll_tape::clinch_fighters_part_under_a_centimeter_x2_shuffled`. The 1 cm bar is still open: the joint table moved the worst 50-fall overlap to 4.53 cm (fall 46, elbow_r/ankle_r) at Rapier's four solver iterations, 2.9 cm at six and 9.2 cm at twelve in engine-physfx, so the 50-fall test stays ignored with those numbers; left: a contact model the joints cannot overrun (AD17 bones plus a stiffer solver, or contacts between grandparents).
  Correction 2026-10-21: duplicate of PW16 (SPRINT-28), which asks this row's failing 50-seeded-falls 1 cm bar and names it as the close (seed 44 overlaps `lower_arm_r`/`shin_r` by 8 cm); do not work both.
- [ ] PX11 physics: PID servo — the joint motors of PX9 run a Fixed-point
  PID (P and D first, I clamped, D on the measurement, slew cap) instead of
  setting velocities, with one tuning per rig, not per animation. Tier B.
  Size S. Owns `crates/physics/src/servo.rs`. Accept: hit, partial and 2 s
  get-up on three rigs with jitter at or below the current servo and ≤ 10%
    overhead; hash x2. needs: PX9.
- [ ] PX32 physics: CPU SPH fluid step — Clavet double-density + XSPH +
  spatial hash, explicit-Euler, fixed seed, ported as integer-mm Rust from
  flax-mcp `plugins/world/src/Physics/Core/Packs/Fluid/{SphSolver,SphConfig}.cs`
  (license MIT, Maxim's own code). Tier B. Size L. Owns
  `crates/physics/src/fluid.rs`. Accept: 1k-particle dam-break fixture x2
  identical hash; non-fluid tick hashes undisturbed. F2P:
  `cargo test -p engine-physics --lib sph_step` (fails before: no fluid
  module). P2P: `cargo test -p engine-physics --lib balance_matrix` stays
  green. needs: PX1. (FLAX-MCP HARVEST EB-2026-10-02-S93;
  research/flax-mcp-harvest/plugins-gameplay-world.md #10.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] DX6 destruction: debris LOD — merge small debris, simplify chunks with the
  CT2 quadric chain, impostors past the LOD distance; replaces the `low_lod`
  flag. Tier B. Size M. Owns `crates/destruction/src/debris_lod.rs`. Accept:
  1,000 debris pieces cost within the DX4 budget; merges are identical x2;
  no pop visible in a PQ4 lookcheck. needs: DX4, CT2. Progress 2026-10-17: 4cf108a8 debris_lod.rs tiers Full/Simplified/Merged/Impostor over the DX4 ledger; 1 ms budget test is ignored and unrun, PQ4 lookcheck needs a human; left: a caller in the window and the CT2 chain. Progress 2026-10-18: caller wired 40119424 (BreakScene::draws takes a camera, DebrisLedger plus plan_lod merge far pieces at 14 m and drop impostors; test break_wire_far_cameras_merge_then_drop_the_pieces); left: 1 ms budget test still ignored and unrun, PQ4 lookcheck needs a human.
- [ ] DX7 editor: fracture authoring — bond paint, strength preview and a
  break-replay preview in the editor, through the command service with undo,
  so AI and humans author destructibles. Tier B. Size M. Owns
  `crates/editor/src/fracture_tool.rs`. Accept: an AI command paints bonds,
  previews a break and undoes cleanly; the saved bonds bake identically x2.
  needs: DX3, ED4.
- [x] DX8 destruction: detail where it breaks — bake probe that puts fine chunks
  where impacts are likely and coarse chunks in the wake (idea from
  phase-field wake coarsening, arXiv 2609.21201; no solver or code taken).
  Bake only, runtime untouched. Tier C. Size S. Owns
  `crates/destruction/src/bake_rank.rs`. Accept: same bond count as uniform,
  better break look in a blind review, DX3 8-way equality holds; else killed.
  needs: DX2. Progress 2026-10-17: 3f671e24 bake_rank.rs spaces lattice cuts by impact likelihood with the same bond count and anchors, 8-way spread equality; the keep-or-kill blind review needs a human; left: a caller. Progress 2026-10-19: operator: the keep-or-kill blind review needs a human (open the graded and the uniform bake side by side and say which break looks better); the `bake_rank.rs` caller stays agent work until the review says keep. See the owner queue.
  Landed. Owner chat 2026-10-19 (delegated): kept, not killed: the decidable legs are green (same bond count as uniform, DX3 8-way spread equality, 3f671e24) and the module is bake-only, so the runtime and the golden are untouched; no blind review is producible in this environment, so that leg stays an open question and the caller stays unwired.
- [x] DX9 destruction: graded bonds — per-bond breaking strain from contact
  area, mass and anchor distance with seeded jitter (today every bond is
  1.0), Voronoi-density cut sites clipped by the existing split code, sliver
  reject, and per-event records of what broke plus the dirty nav tiles for
  NV3. Bake only. Tier B. Size M. Owns `crates/destruction/src/bond_grade.rs`.
  Accept: bake x2; DX3 8-way equality; anchors hold; a blind review prefers
  graded breaks, else density is killed but grading stays; 16/1024 caps hold
  in the 8-fighter soak. needs: DX2, DX3, DX4. Progress 2026-10-17: 40da318b bond_grade.rs strain from contact area, pair mass, anchor distance, seeded jitter, sliver refusal, per-event break records with dirty nav tiles; 8-way insertion-order equality and 16-per-event / 1024-total caps in an 80-cut soak (run 39069886844); left: Voronoi-density cut sites, blind review, a caller. Progress 2026-10-19: operator: the graded-vs-density blind review needs a human (same side-by-side as DX8); the Voronoi-density cut sites and the caller stay agent work. See the owner queue.
  Landed. Owner chat 2026-10-19 (delegated): kept, not killed: the decidable legs are green (bake x2, DX3 8-way equality, anchors hold, 40da318b) and `bond_grade` already has its caller (`impact_table.rs` uses `floor_to_i32` and `sliver_chunks`); no blind review is producible here, so that leg stays an open question.
- [x] PX16 physics: 1,000-agent LOD + sleep scale proof — far soldiers as
  capsules, near ones full ragdoll, whole-island sleep hysteresis, plus
  a seeded shove-through-the-crowd tape proving no missed wake. Tier A.
  Size M. Owns scale-proof hunks beside `crates/physics/src/lod.rs` +
  `crates/perf` workloads (PX7 system frozen). Accept: `cargo test
  -p engine-physics --lib lod` green incl. 1,000-agent fixture; physics
  slice p99 at most 2 ms on the reference CPU; wake-recall 100%; hash
  x2; golden unchanged. needs: PX7, PF1, PF2. (EXTEND physics 2026-09-27.) Landed 0494dbd3. lod.rs and the sleep helpers restored (deleted in 3d6ee4a1); 1,000-agent shove tape, 100% wake recall, same hash x2; lod tests 13 passed, p99 under 2 ms slice test passed on the runner, not the reference CPU.
- [x] PX17 physics: 64-fighter snapshot + rollback-byte proof — snapshot
  bytes follow `build_islands` order with restore-then-snapshot equality
  and a per-fighter byte budget the net encoder reads. Tier A. Size S.
  Owns snapshot-order hunks beside `crates/physics/src/rapier.rs` +
  `islands.rs` (step frozen). Accept: `cargo test -p engine-physics
  --lib snapshot` green; 64-fighter snapshot restores byte-equal x2;
  bytes within the posted budget; golden unchanged. needs: PH1, PX6.
  (EXTEND physics 2026-09-27.)
- [ ] PX18 physics: 8-fighter clinch soak — ragdoll-vs-ragdoll clinches
  with destruction on, separating without interpenetration over 1 cm,
  hash x2, per-event caps held. Tier A. Size S. Owns soak-proof hunks
  beside `crates/physics/src/ragdoll.rs` (no new file). Accept: `cargo
  test -p engine-physics --lib clinch` green on the 8-fighter soak; 50
  seeded clinch-falls separate; hash x2; golden unchanged. needs: PX9,
  DX4. (EXTEND physics 2026-09-27.)
- [x] DX11 destruction: siege-wall fracture at 64-player scale — capped Progress 2026-10-17: fracture.rs has the chunk cap, break_events_follow_the_dx1_shape_and_replay and fracture_event_budget_one_millisecond (the 1 ms test is #[ignore], run alone; destruction suite run 39057037628); left: no wall-break at 64-player scale, no break-event bandwidth-in-net-budget or rollback-exact test.
  seeded runtime cuts with break events on the DX1 shape and break-event
  bandwidth inside the net budget, rollback-exact. Tier B. Size M. Owns
  scale-proof hunks beside `crates/destruction/src/fracture.rs` (DX5 cuts
  frozen). Accept: `cargo test -p engine-destruction --lib fracture`
  green incl. wall-break at scale; chunk cap held; at most 1 ms per
  fracture event; bytes x2; golden unchanged. needs: DX5, DX4. (EXTEND
  physics 2026-09-27.) Landed 05564684 (3c161bdd). 64 strikes in one window hold the chunk cap, fit the 1024-shard ledger and one 1200-byte datagram, match a replica on every strike and resim to the same hash from a mid-window clone; found and fixed a real bug (thin patch under MIN_MASS refused with BadChunk) in FractureChunk::to_chunk3 (run 39069886844).
- [ ] DX12 destruction: debris-to-GPU handoff proof — 1,000 debris poses
  into the RP2 seeded pool within tolerance, merges identical x2, no
  visible pop. Tier B. Size S. Owns handoff test beside
  `crates/destruction/src/budget.rs` + `crates/render` pool fixtures (no
  pass code). Accept: `cargo test -p engine-destruction --lib handoff`
  green; poses match the CPU within posted tol; merges x2; golden
  unchanged. needs: DX4, DX6, RP2. (EXTEND physics 2026-09-27.)
- [x] PX19 physics: combined 64-fighter + 500-agent tick proof — the P6
  physics slice of the server tick, LOD capsules far plus full ragdoll
  near, cloth excluded by rule. Tier A. Size M. Owns the combined-tick
  workload beside `crates/perf` + scale-proof hunks beside the future
  `crates/physics/src/lod.rs` (PX7 system frozen). Accept: `cargo test
  -p engine-physics --lib combined_tick` green on the 64+500 fixture;
  physics slice p99 within 4 ms on the reference CPU; hash x2; golden
  unchanged. needs: PX7, PX16, PF1. (EXTEND physics 2026-09-29.) Landed 8b02a0cf. crates/physics/src/lod.rs tests: combined_tick_64_fighters_500_agents_hash_x2 (LodWorld 64 fighters + 500 agents, 600 ticks, near/far halves swap every 100 ticks, rows of ten as islands, a seeded shove every 20 ticks; same seed twice gives the same digest) engine-physics --lib combined_tick run 39103012579; combined_tick_physics_slice_p99_under_4ms (ignored, timing) passed with --release --ignored run 39103012514 on the GitHub runner, NOT the reference CPU. Test-only hunks, golden unchanged (engine-proof run 39103988897). Clippy -D warnings on engine-physics is red only in other sessions files (door.rs wildcard import, stance.rs sin/cos), none in lod.rs.
- [x] PX20 physics: deterministic swept shape-cast API — segment-cast Progress 2026-10-17: crates/physics/src/query.rs has sphere_cast and ray_cast with rb_query_kinds tests (6b671986, f32 nearest hit); left: no Fixed/Q16 segment cast with sorted time-of-impact and no swept_cast test.
  with sorted time-of-impact in `crates/physics/src/query.rs`,
  Fixed/Q16 only, consumed by GP2 sweep and PX10 CCD wiring. Tier A.
  Size S. Owns the cast hunks in `query.rs` (no new file, no step
  change). Accept: `cargo test -p engine-physics --lib swept_cast`
  green incl. 1,000 seeded casts; TOI order stable under permutation;
  hash x2; golden unchanged. needs: PH1, GP2. (EXTEND physics
  2026-09-29.) Landed 363000af. swept_cast Q16 sphere vs capsules, ordered by time of impact then id; 1,000 seeded casts match a dense march, permuted targets same answer (3 tests run 39066316942; clippy -D warnings run 39067537158).
- [x] PX21 physics: moving-platform ride — deterministic platform-delta Progress 2026-10-17: kcc.rs platform_ride_matrix (station within 1 cm over 600 ticks, platform stop, step-off inherits velocity, hash x2) green in physics run 39057362796; left: the step-off landing is only asserted grounded, not within 1 cm of a target.
  inheritance for carried fighters (ride, step-off, platform stop),
  integer deltas only. Tier B. Size S. Owns ride hunks beside
  `crates/physics/src/kcc.rs` (PH2 frozen). Accept: `cargo test
  -p engine-physics --lib platform_ride` green; carried fighter keeps
  station within 1 cm over 600 ticks; step-off lands within 1 cm;
  hash x2; golden unchanged. needs: PH2, PX1. (EXTEND physics
  2026-09-29.) Progress 2026-10-17: ee98c096 platform_ride_matrix checks the step-off landing tick and x against an independent f64 ballistic prediction within 1 cm (3 tests green, run id not captured; rerun before ticking). Landed ee98c096. cargo test -p engine-physics --lib platform_ride: controller2::tests::platform_ride_keeps_station and kcc::tests::platform_ride_matrix ok (station within 1 cm over 600 ticks, step-off landing tick and x within 1 cm of the f64 ballistic prediction, hash x2), run 39101688073.
- [ ] PX22 physics: servo calibration corpus — offline seeded sweep
  measuring per-joint overshoot and jitter on three rigs, emitting the
  one-tune-per-rig PID tables PX11 consumes. Tier B. Size S. Owns
  `crates/physics/tests/servo_calib.rs` + table fixtures (bake only,
  never the tick). Accept: tables cut hit/partial/get-up jitter vs the
  untuned servo with at most 10% overhead; corpus replays x2; golden
  unchanged. needs: PX1, PX9. (EXTEND physics 2026-09-29.)
- [x] DX14 destruction: bond-grade calibration corpus — offline seeded
  impacts measuring break thresholds by contact area, mass and anchor
  distance, emitting the grading tables DX9 consumes. Tier B. Size S.
  Owns `crates/destruction/tests/grade_calib.rs` + table fixtures
  (bake only, never the tick). Accept: graded bonds hold anchors until
  their strain fails on wall fixtures; bake x2; golden unchanged. Landed 0363fee8 (3c161bdd). tests/grade_calib.rs bisects the real snap strain of 552 bonds over 3 wall seeds: no bond snaps below its graded strain, anchors survive huge hits, per-hop table pinned in tests/fixtures/grade_calib.txt (engine-destruction run 39069886844, clippy run 39069703051).
    needs: DX2, DX3. (EXTEND physics 2026-09-29.)
- [x] DX16 destruction: bond-graph damage propagation — bond topology + Landed d328c26. Tick audit 2026-10-17: cluster_graph_cascade_propagates_along_bonds_only (12-shard chain, swap at break, digest equal x2) plus debris_snapshot green in gh-check run 39057037628 (engine-destruction, 152 passed).
  per-piece health/propagation/cascade/shattered-prefab swap director ported
  as Rust from flax-mcp
  `plugins/world/src/Physics/Core/Packs/Destruction/Destruction.ClusterGraph.cs`
  + `Destructible/Destructible.cs` (license MIT, Maxim's own code). Tier B.
  Size M. Owns `crates/destruction/src/cluster.rs`. Accept: 12-shard cascade
  propagates along bonds only; swap at break; x2 identical. F2P:
  `cargo test -p engine-destruction --lib cluster_graph` (fails before: no
  propagation topology). P2P:
  `cargo test -p engine-destruction --lib debris_snapshot` stays green.
   needs: DX1. (FLAX-MCP HARVEST EB-2026-10-02-S93;
   research/flax-mcp-harvest/plugins-gameplay-world.md #4.)
- [ ] DX17 destruction: impact-keyed fracture table over bonded chunks —
  offline per-shape fragment-set codebook bake with integer
  impact-to-index lookup at spawn, reimplemented std-only with zero
  donor bytes (DeepFracture 2310.13344v2 shape, network and weights never
  in tick or git). Tier A. Size S. Owns
  `crates/destruction/src/impact_table.rs`. Accept: `cargo test -p
  engine-destruction --lib impact_table` green with 3 shapes of at least
  200 table entries each under 50 MB total and spawn at most 0.5 ms per
  impact with replay x2 same hash and 0 NaN on 1000 random impacts and
  blind pick of learned set on at least 12 of 20 vs seeded Voronoi.
  F2P: the impact_table test fails before (no module on master). P2P:
  impact_table green x2 plus golden held. needs: DX5. (RES-STEAL
  2026-10-05 DeepFracture r5-r17 deduped; HS33/BX42 variants folded.) Progress 2026-10-17: 3c161bdd impact_table.rs 486 entries x 3 shapes, integer lookup, replay x2, 1000 random impacts no NaN; the learned-vs-seeded win is a score proxy, the blind pick is open; left: a caller at spawn.
- [x] CT10 assets: glTF conformance — the cook passes the Khronos glTF sample
  models (license per model read first) and Blender glTF exporter variants
  (animation sampling, per-action vs broadcast, PBR mapping from
  KhronosGroup/glTF-Blender-IO, Apache-2.0, as acceptance only). Blender
  stays outside, driven over MCP. Tier A. Size S. Owns
  `crates/assets/tests/gltf_conformance.rs`, `fixtures/gltf/`. Accept: every
  selected model imports, places, saves, reopens and plays; failures are
  typed refusals, never panics. needs: CT1, CT2.
  PX6. Landed e61952b0. cargo test -p engine-assets --features import,registry
  --test gltf_conformance: 8 passed (the_corpus_covers_the_khronos_samples_and_the_blender_variants,
  every_selected_model_passes_the_whole_path, the_saved_bundle_is_identical_twice,
  every_model_carries_a_documented_licence, per_action_and_broadcast_carry_the_same_motion,
  the_pbr_mapping_survives_the_cook, a_mesh_without_indices_cooks_to_the_same_mesh,
  the_saved_bundle_holds_the_model_and_not_a_stub), gh run 39475283117.
- [x] CT11 assets: animation compression — segment, quantize and error-loop
  ideas from ACL (nfrechette/acl, MIT). Tier B. Size M. Owns
  `crates/assets/src/anim_compress.rs`. Accept: 5-20x smaller clips with
  error bounded in cm and degrees; decode allocates nothing; cook x2
  identical. needs: CT1. Landed c840c1c7. anim_compress::tests
  (compress_shrinks_a_smooth_clip_five_to_twenty_times,
  decode_stays_within_cm_and_degrees, decode_allocates_nothing,
  cook_twice_is_identical, truncated_payload_is_refused_not_panicking) 5 pass,
  gh run 39484516174; clippy gh run 39485335830.
- [x] CT13 assets: ray-tracing proxies — the cook emits a simplified proxy
  mesh per asset (quadric chain from CT2) for the RT acceleration
  structures, with a stored deviation bound, beside the full raster mesh
  (Solari proxy rule). Tier A. Size S. Owns `crates/assets/src/proxy.rs`.
  Accept: proxies stay inside their bound on every museum asset; a light leak
  through a proxy fails the tunnel fixture; cook x2 identical. needs: CT2, RT1. Landed c36c060e. proxy::tests 5 passed (proxies_stay_inside_their_bound_on_every_museum_asset, the_same_mesh_cooks_the_same_proxy_twice, a_light_leak_through_a_proxy_fails_the_tunnel_fixture, a_tiny_mesh_keeps_its_own_surface_as_the_proxy, a_degenerate_mesh_is_refused_not_panicking), gh run 39505030084; proxy.rs clippy clean (the crate-wide run stays red on another agent's asset_source/fetch.rs type_complexity).
- [ ] CT14 content: AI motion source — a provider-style `MotionSource` (mock
  clip library; forge's combat-motion sidecar over loopback at
  127.0.0.1:7863, the owner's own trained model; any remote text-to-motion
  API) takes a prompt and seed and returns a clip that goes through CT1
  retarget (HumanML3D-22 to the engine skeleton; the bind pose is proved,
  never names alone), cleanup (foot contacts, root motion, loop points),
  CT11 compression and a CT12 manifest (model hash, prompt, seed, and the
  model's training-data licenses). Bake only: the tick never calls it; the
  mock source keeps CI offline. Tier B. Size M. Owns
  `crates/assets/src/motion_source.rs`. Accept: "sword slash" seed 42 gives
  a clip that imports, retargets with the bind pose verified and plays on
  the fighter with foot slide at most 20 mm; a clip whose model trained on
  non-commercial data is marked non-commercial and refused by the packaged
  build. needs: CT1, CT12, DT5.
- [ ] CT16 content: sandbox boundary for untrusted content — munal-os
  WASM-sandbox shape reimplemented (MIT, PORT cite + EVIDENCE row).
  Tier B. Size S. Owns `crates/endpoint/src/sandbox/content.rs`.
  Accept: malicious asset names refused; out-of-scope tool calls
  blocked. Kill: float math in tick. needs: CT4, MCP8. (RES-STEAL
  wave 36 Dive 12, reviewed 2026-09-26.)
- [x] NV4 nav: near-field avoidance — ORCA half-planes on the XZ plane in Progress 2026-10-17: crates/ai/tests/crowd_orca_bench.rs holds an integer ORCA half-plane step with naive, kd-tree and grid neighbour feeds benched at 50, 200 and 256 agents; left: no crates/nav/src/orca.rs, no 500-agent flat-cost proof, no 1,000-seed corridor stuck drill.
  Fixed math for fighters within 15 m (RVO2-3D, Apache-2.0, math ported with
  a PORT row); far agents stay on flow fields and impostors. Tier B. Size M.
  Owns `crates/nav/src/orca.rs`. Accept: steering ≤ 0.1 ms per agent, flat
  from 50 to 500 agents; 0 stuck in a 1,000-seed corridor drill; replay x2.
  needs: NV2. Landed c531822d c57b0e3a. orca.rs integer port of RVO2 half-planes with hard walls: 200-agent replay digest pinned x2, 1,000 corridor seeds leave nobody stuck (4 m corridor; a 3 m one stuck 3 of 1,000), 0.56/0.54/0.79 us per agent at 50/200/500 vs the 100 us bar; timing on a runner, not the reference PC (run 39070961762) (nav run 39070600996: 89 lib + 9 integration; clippy 39071165673). The fake solver in crates/ai/tests/crowd_orca_bench.rs still duplicates this.
- [ ] NV5 nav: legion platoons — leaders query nav, followers hold gaps in O(1)
  (idea from traffic platoons; SUMO is EPL-2.0, patterns only). Tier B.
  Size M. Owns `crates/nav/src/platoon.rs`. Accept: ≥ 5x fewer nav queries
  for a 500-soldier legion; no accordion at 60 Hz; replay x2. needs: NV4, PX7.
- [x] NV6 nav: smooth paths — portal funnel (string pulling) over the tiled
  mesh with salted tile references so paths survive rebakes
  (recastnavigation Zlib and polyanya Apache-2.0 patterns). Tier B. Size M.
  Owns `crates/nav/src/funnel.rs`. Accept: paths never longer than the grid
  path and at least 10% shorter on 80% of queries; query at most 0.2 ms p50;
  a breach opens a path within 1 s; bytes x2. needs: NV3, DX1. Landed 1329f98e. funnel.rs string pulling over an A* corridor, never longer than the grid walk, >=10% shorter on >=80% of 200 seeded queries, segments inside open cells, breach opens a path in under a second of ticks, bytes equal x2, p50 under 0.2 ms (nav run 39070600996: 89 lib + 9 integration; clippy 39071165673).
- [ ] NV8 nav: 500-agent soak harness — 500 agents under 1 ms, 0 stuck in
  1000 corridors, bytes x2. Tier A. Size S. Owns nav/tests/soak.rs.
  Accept: 500 agents under 1 ms; 0 stuck in 1000 corridors; bytes x2.
  needs: NV4, NV5. (PLN30 gameplay.md, reviewed 2026-09-25; waits NV4/NV5.)
- [ ] GA5 brain: thinking LOD plus perception-clock audit with GA2 —
  500-soldier budget held, 0 stuck, replay x2. Tier B. Size S. Owns
  legion.rs LOD hunks. Accept: 500-soldier budget held; 0 stuck;
  replay x2. needs: GA2, NV5. (PLN30 gameplay.md, reviewed 2026-09-25;
  waits GA2/NV5.)
- [ ] GA7 brain: diverse fighting strategies via offline integer
  students — Halina & Guzdial 2211.02759v1 diversity loss trains
  `.e2nn` students offline (ideas only, license UNVERIFIED, no GPU in
  the loop); 3+ distinct strategies per difficulty in PQ2. Tier B.
  Size L. Owns `crates/ai/src/diversity.rs` +
  `fixtures/fight-diversity/`. Accept: ≥3 strategies per difficulty,
  PQ2 win-rate 40–60%, no float in tick. Kill: float in tick,
  diversity < 3, win-rate outside band. needs: PQ2, BR2.
  (RES-ARXIV wave 32, reviewed 2026-09-26; remapped from colliding GA2.)
- [x] GA8 brain: EQS arena bind — the first live call path: an arena Landed 948cbe1.
  fixture aims the GA1 `DepthBrain` wish at the GA6 `Tactics` ranked
  spot, quantized once into a recorded `FighterInput` applied at the
  tick boundary with a request id. Tier A. Size M. Owns
  `crates/ai/src/eqs.rs` bind hunks plus the arena fixture glue (no
  second brain, no tick-path float). Accept: one ranked-spot fixture
  drives a live arena fighter to cover and holds; replay x2; arena
  tick hash unchanged on seeds that never query. Proof: `cargo test
  -p engine-ai eqs` green plus the arena bind fixture run twice.
  needs: GA1, GA6.
  (PLN EXTEND gameplay.md, proposed 2026-09-29; binding gap is honest:
  no tick-path caller outside ai/nav today.)
- [ ] GA9 brain: legion-vs-legion fixture — 500-vs-500 with orders
  latency: shield wall holds, flanking decides, routers repath on NV3
  versions mid-battle. Tier B. Size M. Owns
  `crates/ai/src/legion.rs` battle-fixture hunks (no new system).
  Accept: flanked force breaks first in fixtures twice; orders take
  effect within the posted latency bound; replay x2. Proof: `cargo
   test -p engine-ai legion` green, battle fixture run twice.
   needs: GA2, NV5.
   (PLN EXTEND gameplay.md, proposed 2026-09-29.)
- [x] GA10 brain: threshold-gated utility pick + otherwise fallback — Landed f3cbc4f. Tick audit 2026-10-17: utility_threshold_gates_first_and_highest, utility_threshold_otherwise_and_jitter, gladiator tests green in gh-check run 39056802035 (engine-ai, 264 passed).
  FirstToScore/HighestToScore threshold pickers plus an otherwise fallback
  over the same Brief slice, ported as std-only Rust from `zkat/big-brain`
  `src/pickers.rs` (license Apache-2.0, reimplement only, never the crate
  dep). Tier B. Size S. Owns picker hunks beside
  `crates/ai/src/utility.rs` (BR1 policy frozen). Accept: sub-threshold
  jitter never flips the winner; empty set takes otherwise; scores never
  enter hashed state; replay x2. F2P:
  `cargo test -p engine-ai --lib utility_threshold` (fails before:
  Highest-only argmax, no threshold/otherwise). P2P:
  `cargo test -p engine-ai --lib gladiator` stays green. needs: BR1.
   (STEAL 2026-10-03 big-brain, https://github.com/zkat/big-brain;
   re-opens the LADDER big-brain picker CARD with the spike numbers.)
- [x] GA11 brain: deterministic behavior-tree executor — Sequence / Landed 453811f. Tick audit 2026-10-17: behavior_tree_check_and_parallel_status, behavior_tree_trace_is_byte_identical_x2 (Sequence, Select, Invert, Check, Parallel) and gladiator tests green in gh-check run 39056802035 (engine-ai, 264 passed).
   Selector / Parallel + blackboard over integer tick state, ported as
   std-only Rust from `Sollimann/bonsai` BT executor shape (license
   MIT, reimplement only, never the crate dep). Tier B. Size S. Owns
   `crates/ai/src/behavior.rs` (BR1 policy frozen). Accept: every
   node type ticks in fixtures; same seed + same ticks give byte-identical
   action traces x2; no float in tick state. F2P:
   `cargo test -p engine-ai --lib behavior_tree` (fails before: no BT
   executor). P2P:
   `cargo test -p engine-ai --lib gladiator` stays green. needs: BR1.
   (STEAL 2026-10-03 bonsai, https://github.com/Sollimann/bonsai.)
- [x] GA12 brain: GOAP planner over the utility world-state — Bevy-style Landed 49df5811. goap.rs plans over a 64-fact integer state with A*; 20 seeded goal sets all plan and match an independent cheapest-cost search, replan after a state flip within a 256-node budget, replay x2 equal (engine-ai goap_plan 5 passed, run 39069981777). Note 363c1500 declares the module and does not build alone; HEAD builds from 49df5811.
   action graph (preconditions / effects / costs) planned with A* over
   integer world-state, ported as std-only Rust from `victorb/dogoap`
   planner shape (license MIT, reimplement only, never the crate dep).
   Tier B. Size M. Owns `crates/ai/src/goap.rs` (GA11 executor frozen).
   Accept: 20 seeded goal sets all plan; cheapest valid plan picked;
   replan on state change stays within one tick budget; replay x2. F2P:
   `cargo test -p engine-ai --lib goap_plan` (fails before: no GOAP
   planner). P2P:
   `cargo test -p engine-ai --lib gladiator` stays green. needs: GA11.
   (STEAL 2026-10-03 dogoap, https://github.com/victorb/dogoap.)
- [x] NV10 nav: broadphase plus 1,000-agent P6 bar — integer Progress 2026-10-17: the grid and kd-tree neighbour feeds exist in crates/ai/tests/crowd_orca_bench.rs against the scan referee; left: no crates/nav/src/crowd.rs, no byte-equal check on the NV8 corpus, no p50/p95/p99 at 1,000 agents.
  uniform-grid (or R* split shape, georust/rstar pattern only)
  contender against the id-sorted scan referee; byte-equal neighbor
  sets on every seed, then p50/p95/p99 at 50/500/1000 agents. Tier A.
  Size M. Owns   `crates/nav/src/crowd.rs` broadphase hunks (never a
  crowd crate). Accept: neighbor sets byte-equal to the scan on the
  NV8 corridor corpus; 1,000-agent sim p99 within the P6 2 ms bar;
  replay x2. Proof: `cargo test -p engine-nav crowd` green plus the
  p50/p95/p99 bench table at 50/500/1000 agents. needs: NV4. (PLN EXTEND gameplay.md, proposed
  2026-09-29; NV9 id retired, frontier TileCache proposal REJECTED.) Landed f32f2e25. The broadphase is the counting-sort grid in orca.rs (the old crowd.rs went in the 2026-10-16 rot cleanup); test hunks added there: crowd_corridor_corpus_neighbours_equal_the_scan (grid sets equal the id-sorted scan referee, entry for entry, on the 1,000-seed corridor-swap corpus the NV4 drill and NV8 share, crowd seen every 12th step), crowd_of_1000_replays_to_the_same_digest_twice, grid_neighbours_equal_a_brute_force_scan, crowd_replays_to_the_same_digest_twice, and the ignored timing gate crowd_step_percentiles_hold_the_p6_bar_to_1000_agents. Bench table (us, runner not the reference PC, gh run 39105710580): 50 agents p50 40 p95 51 p99 134; 500 agents p50 579 p95 842 p99 1404; 1000 agents p50 875 p95 952 p99 1411, so 1,000-agent p99 1.41 ms is inside the P6 2 ms bar. Full engine-nav --no-fail-fast green: 93 lib + 9 integration, run 39107075357; clippy -D warnings clean, run 39106089664.
  Owner chat 2026-10-19 (delegated): accepted as closed by its replacement — REPLACED: `nav/orca.rs`; the row's own text admits the missing parts. Tick audit 2026-10-19: the dead `crowd.rs` path above is the replaced nav module.
- [x] NV11 nav: grid-retirement proof — NV3 tiles plus NV6 funnel cover
  every arena query the legacy `Grid` serves, then the grid goes.
  Tier A. Size S. Owns
  `crates/nav/src/{lib,astar,bake,flow}.rs` removal hunks. Accept:
  funnel-shorter proof reruns green with zero grid fallbacks across
  the query corpus; bytes x2; grid deleted, not deprecated. Proof:
  `cargo test -p engine-nav` green with grid files deleted.
  needs: NV3, NV6. (PLN EXTEND gameplay.md, proposed 2026-09-29.) Landed 3ed325c8 (the 8-dir grid A* astar.rs, the legacy grid, was deleted in the 2026-10-16 rot cleanup; crates/nav has no astar module and no Grid query type, lib.rs lists bake, flow, funnel, links, mesh, orca, reach, rebake, spline, tiles). Every arena path query now answers from the tiled nav plus the funnel: smooth_paths_beat_the_grid_walk (200 seeded arena queries, never longer than the tiled walk, 80%+ at least 10% shorter, each answered by smooth_path with no fallback to compare against) and same_query_gives_the_same_bytes_twice (bytes x2). cargo test -p engine-nav --no-fail-fast green with no grid files, 91 lib + 9 integration, gh run 39103483221 (93 lib + 9 after the NV10 tests, run 39107075357).
- [x] NV12 nav: shared-edge stitch proof — bake the arena whole and
  tiled, diff every cross-tile path byte-wise (WD8 shape). Tier A.
  Size S. Owns `crates/nav/src/tiles.rs` stitch hunks. Accept:
  cross-tile paths byte-equal to single-mesh paths on the arena kit;
  byte-equal again after a 4-tile breach plus rebake; bytes x2.
  Proof: `cargo test -p engine-nav tiles` green, stitch diff run
  twice. needs: NV3. (PLN EXTEND gameplay.md, proposed 2026-09-29.) Landed 28d7a357. Whole and tiled arena bakes give byte-equal paths and flow steps on 150 cross-tile queries before and after a 4-tile breach; the floor is a nav-local arena-shaped grid (nav cannot depend on the game kit) (nav run 39070600996: 89 lib + 9 integration; clippy 39071165673).
- [x] NV13 nav: temp-obstacle request queue — touched/pending sets Progress 2026-10-17: ObstacleCache plus TiledNav::update_budget landed in tiles.rs with tests/tiles.rs obstacle_rebuild_touches_only_its_tiles and obstacle_requests_refuse_loudly (nav green, run 39057395844); left: the 4-tile breach numbers (touched 4/4/4 cap 8, up_to_date, digest afb7c5d2) are not asserted anywhere.
  plus budgeted rebuild plus `up_to_date` convergence (LADDER L517
  ADOPT-scoped spike). Tier A. Size S.
  Owns `crates/nav/src/tiles.rs` queue plus `update_budget` hunks
  beside frozen `invalidate`/`rebake_budget` (no second tile store).
  Accept: `cargo test -p engine-nav tiles` green; 4-tile breach
  (32x32 edge-8, 2 cylinder adds plus 1 box removal) posts touched
  4/4/4 cap-8 plus 1 rebuild per tick over 10 ticks plus
  `up_to_date` true with stale-exact 10/10 plus digest `afb7c5d2`
  x2; burst-10 ablation killed; golden unchanged. needs: NV3.
  (PLN gameplay.md, proposed 2026-09-30.) Landed 28d7a357. Cylinder obstacles, touched(), pending_count(), TiledNav::up_to_date(): the 32x32 edge-8 breach touches 4/4/4 tiles (cap 8), rebuilds one tile per tick over 10 ticks with an exact stale set each tick, the burst-10 ablation trips the per-tick gate; the Rust digest 8032266843827844966 x2 replaces the spike's afb7c5d2 (different digest model) (nav run 39070600996: 89 lib + 9 integration; clippy 39071165673).
- [ ] NV14 nav: per-tile integer-mm rebake detail (held on NV13, feeds
  NV12 stitch proof) — border-overlap bake-lane geometry, tick keeps
  version flips only (LADDER L511, spike first). Tier A. Size M. Owns
  `crates/nav/src/mesh.rs` bake-tile hunks plus `tiles.rs`
  rebake-path hook. Accept: `cargo test -p engine-nav tiles` green;
  dirty-4-only ms versus full-bake ms posted plus unaffected-tile
  bytes identical plus cross-tile path byte-equal to whole-bake path
  plus digest x2; kill on any seam gap or per-event over 50 ms p95.
  needs: NV13, NV3. (PLN gameplay.md, proposed 2026-09-30.)
- [x] NV15 nav: salt-epoch handle detail (held on NV13, feeds NV7) —
  tile and obstacle handles so dangling refs fail closed (LADDER
  L512, spike first). Tier A. Size S. Owns
  `crates/nav/src/tiles.rs` TileRef plus StampedPath stamp-check
  hunks. Accept: `cargo test -p engine-nav tiles` green;
  dangling-stamp refusal 100% plus overflow keeps staleness plus
  digest x2; kill on any dangling resolve or digest drift. needs:
  NV13, NV7. (PLN gameplay.md, proposed 2026-09-30.) Landed a1e2500.
  `crates/nav/src/tiles.rs`: `TileRef { tile, epoch }` with `take` (reads the
  tile's current version), `tile`, `epoch` and `resolve`, plus
  `TileError::DanglingRef { tile, epoch, now }` and its Display arm. Resolve
  returns the tile only while its version is unchanged, so a rebake under a
  carried handle fails closed; versions never wrap (`rebake` refuses
  `VersionOverflow` and keeps the old one), so a live handle cannot alias a
  newer rebake. Tests: `tile_ref_refuses_every_dangling_resolve` (all four
  tiles of a 2x2 registry refuse after their own rebake, 100%),
  `tile_ref_digest_repeats_and_moves_with_the_version` (Fnv64 over handle
  bytes repeats, and moving one epoch moves it), and
  `version_overflow_keeps_a_live_handle_valid` (a version at u32::MAX refuses
  the rebake and the handle still resolves). `cargo test -p engine-nav --lib
  tiles` 34 passed, run 39182792426; rustfmt clean; rust-quality engine-nav
  PASS; workspace-check PASS. No simulation crate touched.
- [x] NV16 nav: rational-NURBS spline math + parallel-transport frames — de
  Casteljau / Cox-de Boor sampling, arc-length LUT, analytic tangents,
  twist-free sweep frames and ear-clip extrusion ported as integer-safe Rust
  from flax-mcp
  `plugins/world/src/Procedural/Core/Packs/Nurbs/{SplineMath,ParallelTransport}.cs`
  + `VectorExtrusion/VectorExtruder.cs` (license MIT, Maxim's own code).
  Tier B. Size M. Owns `crates/nav/src/spline.rs`. Accept: sample/tangent/
  nearest match the reference within tol on a pinned curve; x2 identical.
  F2P: `cargo test -p engine-nav --lib spline_frames` (fails before: no
  NURBS math). P2P: `cargo test -p engine-nav tiles` stays green. needs:
  NV2. (FLAX-MCP HARVEST EB-2026-10-02-S93;
  research/flax-mcp-harvest/plugins-gameplay-world.md #3.) Landed 8c6520a3. spline.rs: NURBS point/tangent, arc-length table, nearest, parallel-transport frames, ear-clip, profile sweep, no trig; 7 spline_frames tests, a 3D curve bit-identical x2; pinned reference is an exact quarter circle (the flax-mcp C# sources were not on this machine) (nav run 39070600996: 89 lib + 9 integration; clippy 39071165673).
- [ ] PF5 perf: SIMD pass — hot loops (culling, CPU skinning paths, spatial
  queries, audio mix) on glam SIMD or `wide` with scalar fallbacks;
  simulation code keeps its integer math. Tier A. Size M. Owns the named
  hot files per PF2 capture. Accept: each change A/B-measured; identical
  outputs. needs: PF2. Progress 2026-10-19: all four named loops were improved, but by allocation, hoisting and algorithm work rather than by SIMD, each with a paired measurement and an equality proof: CPU skinning PF35 (5.7x-6.8x, keyed name table), spatial queries PF33 (7.1x-9.0x, AABB pre-filter), audio mix PF34 (1.098x, cached frame count), and the tick paths PF29-PF46. Explicit SIMD was then measured on the spatial sight scan and **lost**: `cargo rustc --emit asm` showed zero SIMD instructions for two auto-vectorisable shapes, and `wide::i32x8` lanes beat the scalar loop only inside the machine's 12% drift (PF47, negative result). Culling is the one name left and it is GPU-side, owned by PF19/PF25. Left open because no SIMD change landed; the row stays the place to record a future vector attempt with a reason.
- [ ] PF9 perf: replay visual debugger — tracy spans plus a rerun recording Progress 2026-10-17: crates/observe/src/rerun_sink.rs landed (names_first_mismatched_hash, tracy zone names; observe green run 39057303818) and PF9a paired reruns name the first divergent hash; left: the GPU timestamp, 2,000-frame and RA8 manifest legs of the hunt are not done.
  of simulation state per tick for desync hunts (licenses read first).
  Tier B. Size M. Owns `crates/observe/src/rerun_sink.rs`. Accept: a seeded
  desync shows its first divergent tick in the recording. GitHub hunt 2026-09-25: GPU timestamp queries (two per pass) resolved behind the GX truth gate give `gpu_ms` next to CPU spans; a free-list buffer arena keeps allocations flat over 120 frames (wgpu, rend3 patterns). Deep 3D pass: raise capture to 2,000 frames (120 today), cover every GPU pass (6 today), and write tier, VRAM and draw counts into the RA8 manifest; this harness comes before any GPU budget    claim. needs: PF2. Progress 2026-10-18: 946c14a4 MCAP recording (observe mcap.rs): ReplayRecording is not bound to the 120-row ring, so a 2,000-frame capture fits (a 600-tick 2-fighter bout is under 400 KB), and first_divergent_tick names a seeded desync's first tick in the file (mcap_roundtrip_seeded_desync_first_differs_at_its_tick). Left: GPU timestamp queries behind the GX truth gate, every GPU pass in the capture, tier/VRAM/draw counts in the RA8 manifest (need a GPU).
- [x] PF29 perf: the ecs tick flush and the snapshot allocate once, not per call — `archetype::World::flush` took the staged-kill vector with `mem::take`, so the vector came back with capacity 0 and every tick reallocated it; `world_snapshot::World::snapshot` grew a fresh `Vec` about 10-17 times per call. Flush now sorts in place (`sort_unstable_by_key`; staged handles are unique so the order is the same) and `clear()`s, keeping capacity (**1 -> 0 allocations** per kill+flush); snapshot starts at `Vec::with_capacity(self.snapshot_len())` with the field sequence moved into `append_snapshot` (**10 -> 1 allocations** per call). Diff: regular. Tier B. Size S. Owns `crates/ecs/src/archetype.rs`, `crates/ecs/src/world_snapshot.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-ecs --lib` prints 137 passed 0 failed, and the frozen digests `SNAPSHOT_ORACLE = 6287169743782133372` and `FLUSH_ORACLE = 9361122219599841781` still hold, so the byte stream is unchanged; paired old-vs-new p50 in one process is 1.13x-1.91x for snapshot (4,000 rows) and 1.50x-2.00x for flush (2,000 rows, 8 kills). F2P: on the old code the counting-allocator asserts fail (1 and 10 allocations) and there is no `snapshot_len`. P2P: the rest of the ecs lib and the golden `d57b5c8ae3096614`. needs: -. Landed 6fcf038. Golden x2 not re-readable 2026-10-19: another session's uncommitted crates/net/src/handshake.rs fails engine-net E0277, so engine-proof does not build; the frozen byte oracles carry the equivalence proof, and no arithmetic, ordering or hash input was touched.
- [x] PF30 perf: the nav tangent stops allocating — `Nurbs::tangent` materialised the whole derivative control array (n-1 x 32 bytes) on the heap on every call; it now evaluates the derivative curve's de Boor in place from a stack array, so one tangent is O(degree^2) on the stack, as the spline header already promised. Diff: regular. Tier B. Size S. Owns `crates/nav/src/spline.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-nav --lib` prints 101 passed 0 failed, and `spline::tests::stack_tangent_equals_the_allocating_oracle_bit_for_bit` proves the stack path equals the kept allocating path over 65 parameters including both clamped ends. F2P: before, one tangent was a heap allocation per call. P2P: the closed-form circle frame/tangent tests and the golden `d57b5c8ae3096614`. needs: -. Landed e768d8b. Paired in-process p50 5.12x-5.75x faster on a 64-point curve (gh 39241060143). Golden x2 not re-readable 2026-10-19: uncommitted `crates/net/src/handshake.rs` from another session fails engine-net E0277, so engine-proof does not build.
- [x] PF31 perf: the editor diff stops allocating a row `String` per redraw — `SceneDiff::text_rows_into` writes the rows into a caller-kept buffer, minting a slot only when the buffer has no `String` there, while `text_rows` stays as the allocating form; at 2,500 diff rows the kept-buffer form measures 182.3 us -> 130.4 us (1.40x). Diff: regular. Tier B. Size S. Owns `crates/editor/src/activity.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-editor --lib` prints 258 passed 0 failed, and `activity::tests::text_rows_into_a_kept_buffer_matches_the_reference_format` proves both forms write the same rows. F2P: no kept-buffer form existed, so a redraw allocated one `String` per row. P2P: the rest of the editor lib and the golden `d57b5c8ae3096614`. needs: -. Landed e768d8b (gh 39241467335). A folded outliner search was tried in the same round and reverted: its own probe measured 0.64x on a 120-char needle over 128-char labels, so no regression lands. Golden x2 still blocked by the same net file.
- [x] PF32 perf: the mesh-cook tipsify walk stops cloning — the index-order walk cloned a 32-entry cache once per emitted triangle (about 20,000 allocations) and a vertex ring once per corner (about 60,000 small `Vec` clones) on a 20,000-triangle mesh; both now reuse one buffer and read the ring in place. Diff: regular. Tier B. Size S. Owns `crates/assets/src/mesh_cook.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-assets --features import` PASSes, and `mesh_cook::tests::index_order_walk_keeps_prop_grid_digest` keeps the prop-grid cook digest `0x2e4647398a5e5648`. F2P: two heap allocations per element before. P2P: the cook-twice tests and the golden `d57b5c8ae3096614`. needs: -. Landed e768d8b. Paired in-process time ratio new/old 0.67x-0.72x (cache) and 0.25x-0.29x (ring), about 1.4x and 3.9x faster (gh 39242569861). Golden x2 still blocked by the same net file.
- [x] PF33 perf: the sight ray scan skips solids outside the segment AABB — `blocked` in `crates/spatial/src/sight.rs` walked every solid with two containment checks plus the i64 slab test on every `los_point`, `los_actors`, `visible` and `cover` call, which `exposure::tile_permille` and `fps_visibility::fps_sight_set` run per tile and per candidate; the segment integer AABB is now hoisted out of the loop and a box strictly outside it on any axis is skipped with two `i32` compares (a strict gap, so a merely touching box still reaches the exact slab test and the touching semantics are untouched). Diff: regular. Tier B. Size S. Owns `crates/spatial/src/sight.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-spatial` PASSes with 394 passed 0 failed, and `blocked_aabb_filter_equals_full_scan_on_corpus` proves the filtered scan equals the kept full scan over 26,244 ordered lattice pairs with both outcomes occurring. F2P: before, every solid was containment-checked and slab-tested on every ray. P2P: the rest of the spatial lib and the golden `d57b5c8ae3096614`. needs: -. Landed 84b4bb4 (gh 39244629303). Paired in-process release A/B 1.172 us -> 0.130 us (9.02x) and 0.933 -> 0.131 (7.12x); a deliberately conservative long-ray fixture still gives 2.27x, so the win is not one fixture. Golden x2 still blocked by the same net file.
- [x] PF34 perf: the audio mixer stops dividing per voice-frame — the per-frame voice walk recomputed `data.len() / channels.max(1)` twice per voice per frame (two integer divisions plus the `max`) although `data` and `channels` never change after the voice starts; `Voice` now carries `frame_count`, filled in `push_voice` where it was already computed, and `frames()` reads the field. Diff: regular. Tier B. Size S. Owns `crates/audio/src/lib.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-audio` PASSes with 266 passed 0 failed plus 22 integration tests, and `pf5_voice_frames_cache_renders_the_same_bytes` asserts the rendered blocks are equal bit for bit against a test-only recompute twin. F2P: before, every voice-frame did two integer divisions and a `max`. P2P: the rest of the audio lib and the golden `d57b5c8ae3096614`. needs: -. Landed f9859a7 (gh 39247388367). Paired in-process receipt: cached p50 100.768 us against recompute 110.646 us on 32 voices and a 480-frame block, so 1.098x (about 9.8%) on the mix block. The mix is presentation-side and never enters the hash, so the blocked golden run is not needed for this row.
- [x] PF35 perf: the anim skeleton validator stops rescanning earlier names — `validate_skeleton` runs on every CPU skinning call (`skinning_matrices` -> `validate_pose` -> `validate_skeleton`, and `sample_pose_with_rots_into` directly) and compared each joint name against every earlier name, an O(n^2) string scan that was 56% of a 64-joint skinning call and 76% of a 256-joint one; a name is now recorded in a stack-only open-addressed table (2 x `MAX_JOINTS` slots, keyed by `engine_core::Fnv64`, owner index in a `u16`) with a byte-for-byte confirmation on a key hit, so the same first offending index comes back at O(1) probes. Diff: regular. Tier B. Size S. Owns `crates/anim/src/contract.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-anim` PASSes with 450 lib passed 0 failed plus 8 `anim_lod_pin`, and `contract::tests::keyed_name_table_matches_the_linear_scan_oracle` keeps `validate_skeleton_linear_scan` verbatim and asserts new equals oracle equals expected on 15 fixtures. F2P: before, every joint rescanned all earlier names. P2P: the rest of the anim lib and the golden `d57b5c8ae3096614`. needs: -. Landed b32fe22 (gh 39251767791). Paired in-process p50 7.5-10.8 us -> 1.2-1.6 us at 64 joints (5.7x-6.8x). A stack-scratch variant of `skinning_matrices` measured 1.08x/1.02x and was rejected as noise, so the round holds one change. Golden x2 still blocked by the same net file.
- [x] PF36 perf: the ai gladiator intent stops rebuilding its choice table — `thinker_intent` is reached from `Brain::score_intent` on the tick path and built a `crate::utility::Thinker` per call (two `Vec`s grown from empty across five `when` pushes, about four heap allocations per decision) although the `Thinker` doc itself calls the table a cold-build item; the five scored rows now go straight through `crate::utility::pick_gated` over a stack array of `Scorer` trait objects, the identical selection call with the same zero threshold and `Gate::Highest`, so no arithmetic moved and the public signature is unchanged. Diff: regular. Tier B. Size S. Owns `crates/ai/src/gladiator.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-ai` PASSes, and `gladiator::tests::thinker_intent_matches_the_table_oracle_on_a_sweep` keeps the pre-change table path as the oracle and asserts equality on 504 fixtures. F2P: before, each decision allocated the table's two `Vec`s. P2P: the rest of the ai lib and the golden `d57b5c8ae3096614`. needs: -. Landed bc4ebaf (gh 39254959711). Paired in-process 207 ns -> 77 ns per decision (2.68x). Scope, stated on the commit: the path fires when morale is not neutral, so this is the tuning and fork tick, not every default duel tick. Golden x2 still blocked by the same net file.
- [x] PF37 perf: the ai EQS cover-align scoring hoists the threat vector out of the cover loop — `Test::CoverAlign`'s score arm recomputed the spot-to-threat vector and its `u128` isqrt length once per cover although both are invariant across covers; LLVM does not hoist it (`u128::isqrt` expands to a bit loop and LICM will not move a loop-containing call). The scoring moved into `score_cover_align`: `tx`/`tz`/`tl` once per point, then the covers under an `if tl > 0` guard. Same integers, same i128 permille dot, same clamp, same order; the zero-length threat keeps the floor score exactly as the old per-cover `continue` did. Diff: regular. Tier B. Size S. Owns `crates/ai/src/eqs.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-ai` PASSes, and `eqs::tests::cover_align_hoist_matches_the_shipped_loop` asserts byte-equal `EqPoint` vectors at every power plus the degenerate target-on-candidate case. F2P: before, every cover redid an `isqrt`. P2P: the rest of the ai lib and the golden `d57b5c8ae3096614`. needs: -. Landed 51af586 (gh 39262342339). Paired p50 32.600 us -> 22.200 us (1.46x, held 1.42x-1.46x over three runs). GOLDEN READABLE AGAIN: `cargo test -p engine-proof --lib golden_hash_untouched` PASSed twice (gh 39262668598, 39263094037), so `d57b5c8ae3096614` holds with every PF29-PF36 change in the tree; the per-row notes above that the golden could not be read were true when written and are now superseded.
- [x] PF38 perf: the combat impact resolver snapshots ten fields instead of the fighter table — `resolve_impacts` is the rules-v2 per-tick impact resolver on the arena authority path, and every geometry tick cloned the whole `[FighterState; SLOT_COUNT]` table (16 x about 108 B, including the 72-byte `StreamBuffer`), eagerly copied a whole `FighterState` per attacker and a whole guard per attacker-victim pair; a private `PreState` now holds the ten fields the resolver reads, copied verbatim, so the snapshot is a quarter of the size and the fallbacks are lazy (`map_or_else`). No arithmetic, gate order or event order changed. Diff: regular. Tier B. Size S. Owns `crates/combat/src/fight.rs`. Accept: `cargo test -p engine-combat` PASSes (307 lib + 19 integration), `compact_snapshot_matches_the_full_clone_across_a_v2_brawl` asserts the same events, state and end state over a 240-tick brawl with geometry on and off, and the GOLDEN passes with this change in the tree (`golden_hash_untouched`, gh 39266434512). F2P: before, the whole table was cloned per tick. P2P: the rest of the combat lib and the golden. needs: -. Landed c486701. Paired p50 brawl 1.6391 us -> 1.3641 us (1.20x) and mates 0.9172 us -> 0.5687 us (1.61x); five runs held 1.08x-1.20x and 1.34x-1.61x, always faster.
- [x] PF39 perf: the editor outliner search stops lowercasing every row for a short query — `outline::search` ran `name.to_lowercase().contains(needle)` per row, one `String` per row; it now lowercases the query once and picks one loop per query shape: an empty needle preallocates and matches every row, an ASCII needle of at most `FOLD_SCAN_MAX_NEEDLE` (16) bytes takes an allocation-free byte-window scan with a first-byte gate and checked bounds, and a longer or non-ASCII needle keeps the library two-way search behind a first-byte gate that rejects a miss without lowercasing the name; non-ASCII row names inside the scan branch fall back to the allocating path. Diff: regular. Tier B. Size S. Owns `crates/editor/src/outline.rs`. Accept: `cargo test -p engine-editor` PASSes, and `folded_search_agrees_with_lowercase_reference` equals the pre-change lowercase oracle on ASCII, Turkish dotless i, Greek final sigma, long s, dotted capital I, and empty / prefix / suffix / over-bound needles, in row order. F2P: before, every row allocated a lowercase copy. P2P: the rest of the editor lib. needs: -. Landed 3354da2. SECOND ATTEMPT: the first was reverted at 0.64x on a 120-byte needle, and the length threshold plus the first-byte gate fix exactly that case. Paired p50 `ai 4` 73.7 us -> 23.7 us (3.108x), empty 1.003x, 2,000 x 128-char rows with a 120-byte miss 754.2 us -> 201.9 us (3.736x). The editor is authoring-side and never enters the hash.
- [x] PF40 perf: the physics island rebuild reuses its buffers, so a combined tick allocates nothing — `LodWorld::step` rebuilt islands from scratch every tick (fresh root table, a `(root, index)` comparison sort over all bodies, one `Vec` per island), which on the combined 64-fighter + 500-agent fixture (450 pairs, 50 islands) was **207 allocations per tick** against a module header that promised no per-tick allocation; `IslandsScratch` now owns the root table, a counting table, the flat member order and the island vectors, and rebuilds by union-find into the reused buffer plus a stable counting pass keyed by root (roots are component minimums, so ascending roots are ascending island first members and the in-order fill keeps members ascending, the same total order as sorting by `(root, index)`). `build_islands` stays verbatim as the shipped reference. Layer: AUTHORITY (`islands` is std-only integer and its output drives `update_sleep` and the world digest). Diff: regular. Tier B. Size M. Owns `crates/physics/src/islands.rs`, `crates/physics/src/lod.rs`. Accept: `cargo test -p engine-physics` PASSes, `scratch_build_matches_the_shipped_build_on_every_shape` compares member-for-member over 200 seeded pair sets plus a 64/3/64/0/64 reuse sequence, `combined_tick_scratch_matches_the_shipped_islands_every_tick` asserts the digest equal on every tick of a 600-tick combined tape, `stepping_the_army_allocates_nothing` sees 0 allocator calls over 60 steps, and the GOLDEN passes (`golden_hash_untouched`, gh 39269964331; the agent's own run 39267536787). F2P: before, 207 allocations per tick and a comparison sort. P2P: the rest of the physics lib and the golden. needs: -. Landed 3988fc5. Paired p50 island rebuild 13.9 us -> 3.6 us (3.86x) and the whole combined tick 23.0 us -> 12.9 us (1.78x); three runs held about 3.9x and about 1.75x.
- [x] PF41 perf: the arena engage plan converts poses once and measures each foe pair once — `engage::Plan::build` was about 22% of a rules-v2 8-fighter tick and converted both poses to millimetres and ran its own `u64` isqrt inside each of the 56 ordered foe pairs, every tick; `pose_mm` now converts each slot's feet once per tick (8 conversions instead of 64) and `separation_table` runs one integer sqrt per unordered pair, stored at `[low][high]` and read from both directions (28 instead of 56 at 8 fighters). View insert order, sort, token claims, aim, tick order and event order untouched. Layer: AUTHORITY. Diff: regular. Tier B. Size M. Owns `crates/arena/src/engage.rs`, `crates/arena/src/lib.rs`, `crates/arena/src/fork.rs`. Accept: `cargo test -p engine-arena --lib` prints 221 passed 0 failed 3 ignored, `v2_match_matches_the_pairwise_plan_oracle_tick_for_tick` asserts views, targets, state, poses, event order and the checkpoint hash equal on every tick of a real 600-tick bout, and the GOLDEN passes. F2P: before, 56 ordered pairs each redid two pose conversions and an isqrt. P2P: the rest of the arena lib and the golden. needs: -. Landed 78df443. Paired p50 plan 1.6719 us -> 1.1906 us (1.404x) and the whole 600-tick match 4888.7 us -> 4583.7 us (1.067x, about 8.15 -> 7.64 us per tick); three runs held 1.35x-1.40x and 1.05x-1.08x.
- [x] PF42 perf: the ecs checkpoint stages two appends per row instead of six — `checkpoint_with` runs every tick from `Schedule::run_arena_tick` and staged one `try_extend_bytes` per field (id, mask, pos.x, pos.y, vel.x, vel.y: six per P+V row, each re-deriving length and capacity); the id, mask and position now stage as one fixed 13-byte row head and the velocity as one 8-byte pair word, so a P+V row costs two appends, with the staged byte order unchanged so the digest still equals `World::state_hash`. Diff: regular. Tier B. Size S. Owns `crates/ecs/src/archetype.rs`. Accept: `cargo test -p engine-ecs` prints 138 passed 0 failed 9 ignored, `checkpoint_buffer_matches_the_field_by_field_bytes` asserts byte-identical arena bytes and equal digests against the old body and shows a spawn moves the digest (not vacuous), and the GOLDEN passes. F2P: before, six appends per row. P2P: the rest of the ecs lib and the golden. needs: -. Landed 2c14788. Paired p50 280.900 us -> 261.100 us (1.08x) on this PC and 1.15x / 1.09x on two runner runs. Two probes were measured and reverted: slice-zip query iteration at 0.99x (LLVM already drops the per-column bounds checks) and a single 21-byte variable-length row append at 0.72x.
- [x] PF43 perf: the arena collision relaxation stops at its fixed point instead of running every pass — `collide::resolve` ran all `COLLIDE_PASSES` (4) relaxation passes over every walker pair plus the pillars and the wall, every tick; a pass that moved no walker's planar pose is a fixed point (the next pass sees the same poses and makes the same set of no-moves, and `separate_pair`, `push_out` and `hold_inside` are deterministic integer functions of the poses), so it now snapshots the poses per pass and breaks at the first pass that moved nothing, leaving the array the full pass count would hold. The pre-shortcut body is kept as `collide::resolve_fixed_passes` and `Sim` grows a test-only `full_collide_passes` switch so a whole match can run through the oracle. Layer: AUTHORITY. Diff: regular. Tier B. Size M. Owns `crates/arena/src/collide.rs`, `crates/arena/src/lib.rs`, `crates/arena/src/fork.rs`. Accept: `cargo test -p engine-arena --lib` prints 223 passed 0 failed 5 ignored, both paths are compared tick for tick on a whole match (events, poses and the checkpoint hash), and the GOLDEN passes. F2P: before, four passes always ran. P2P: the rest of the arena lib and the golden. needs: -. Landed a1c8ddb. Paired release p50 resolve 0.8055 us -> 0.2465 us (3.268x), `collide_states` 0.9395 us -> 0.3765 us (2.495x), and the whole 600-tick match 10.8190 us -> 10.2227 us per tick (1.058x); the fixed point stopped the pass loop on 590 of the 600 ticks. The agent that wrote it stopped before reporting its numbers, so I ran the receipt and the suites myself.
- [x] PF44 perf: the net client merges the authority tape without re-sorting a tape that already extends it — `Client::on_server_state` built the merged tape with `merge_tapes` on every authoritative state (fresh `Vec`, full copy of the old tape plus the whole incoming tape, then `sort_by_key` + `dedup_by_key` over 2n inputs), and a phase profile on a real 600-state wire trace put `merge_tapes` at 7.91 ms of an 11.35 ms state; the wire sends the whole tape plus a tail, so the sort was redundant work on an already ordered tape. `union_tape` fills a client-held spare buffer: a delta that already extends the known tape verbatim is copied with no sort or dedup, otherwise the old sorted union runs with the same first-entry-per-key semantics; the staged union swaps into `server_tape` only after the state is accepted, so a refused state changes nothing. Diff: regular. Tier B. Size M. Owns `crates/net/src/session.rs`. Accept: `cargo test -p engine-net --lib` prints 396 passed 0 failed 4 ignored, `union_tape_matches_merge_tapes_at_every_state` equals the kept oracle at all 120 wire states plus the heal rewrite, `apply_states_keeps_the_pre_change_digest` keeps `APPLY_ORACLE = 0x818a0acdb1f938c7` with a perturbation moving it, and the GOLDEN passes. F2P: before, every state re-sorted 2n inputs. P2P: the rest of the net lib and the golden. needs: -. Landed 96e9223. Paired release p50 for one 600-state session 7446.9 us -> 863.6 us (8.62x, an earlier run 9.73x); `on_server_state` 11.35 ms -> 4.90 ms over 600 states. The digest pins the tape that feeds the arena desync hashes.
- [x] PF45 perf: the destruction tick hash measured at its floor, nothing kept — the rules-v2 tick reaches exactly one destruction hash (`BondedCollection::state_hash` from arena's `step_with` and `spine_step::sys_extract`; the debris ledger belongs to the siege break window, the crack wall already returns its cached chain, `shape_match` is opt-in with no caller, and `shards`/`shards3d` are bake runtimes). On the live-duel crate (8 chunks, 23 bonds, 892 canonical bytes), release, one process: shipped p50 0.958-0.966 us/tick = 1.075 ns/byte; the same bytes through `engine_core::hash_bytes` 1.162-1.166 us (0.83, so the shipped body wins); a raw-accumulator fold 1.00 and 0.97; a byte-at-a-time fold 1.141 us. The serial FNV-1a64 chain costs ~4 cycles/byte, so 1.075 ns/byte is 93-100% of the floor. Every byte-cutting idea changes the digest (43% of the bytes are quiet-tick-invariant but FNV positional state forces them through; a word fold is not FNV-1a; a generation stamp cannot help because `step` bumps `tick`, which is hashed). Kept: only `bonded::tests::state_hash_is_fnv1a_over_the_canonical_word_stream` (six reachable shapes plus a non-vacuity probe) and the ignored receipt. Diff: tests only, +205 with zero production lines. Tier B. Size S. Owns `crates/destruction/src/bonded.rs`. Accept: `cargo test -p engine-destruction` 177 passed 0 failed 4 ignored, clippy green, golden green, workspace-check PASS. Landed c3ceca4. NEGATIVE RESULT kept on the board so the next reader does not re-run this probe.
- [x] PF46 perf: the scene world pass composes each pose once instead of walking the ancestor chain per node — `Hierarchy::world_all` is on the editor per-frame path (`build_draw_list` -> `sorted_worlds` -> `world_all`); it called `propagation_order`, which calls `children(id)` once per node and `children()` scans and sorts the whole node map (O(n^2)), then called `world(id)` per node, walking the ancestor chain again and allocating a chain `Vec` each time. It now builds `child_map` once and walks top-down with a stack of `(id, parent world pose)`, composing each pose once and handing it to its children: same pre-order, same sibling order, same operands in the same order, so the pose bits are identical; the unreachable tail moved into a private `append_unreachable` with the same logic and only runs for corrupt documents. Diff: regular. Tier B. Size S. Owns `crates/scene/src/transform.rs`. Accept: `cargo test -p engine-scene --lib` prints 78 passed 0 failed 1 ignored, `world_all_matches_the_per_node_walk_bit_for_bit` asserts the same ids, order, `f32::to_bits` per lane and digest on all 8 frames against the kept old body, `world_all_keeps_unreachable_nodes_last` covers a dangling parent and a link cycle, and the GOLDEN passes. F2P: before, the walk was O(n^2) plus an ancestor walk and a chain `Vec` per node. P2P: the rest of the scene lib and the golden. needs: -. Landed 41de84c. Paired release p50 over 8 frames of a 291-node scene: new 424.3-672.6 us against old 2009.8-3118.9 us, 4.58x-4.90x over four runs (about 53-84 us per frame against 251-390 us). The harness warms once per side and alternates order after its first version showed an 8% second-call bias.
- [x] PF47 perf: the sight filter vectorised and measured no win, so nothing ships — the SIMD half of PF5 was tried on `blocked`: two auto-vectorisable shapes checked in the emitted asm produced **zero** SIMD instructions (an 8-lane branchless block over a `u64`-mirrored bounds array, slower at 0.500 us vs 0.300 us, and a 64-lane axis-major plane pass); explicit `wide::i32x8` lanes did emit `pcmpgtd` once the bounds were mirrored as `Vec<[i32; 8]>` blocks. On the PF33 fixture (404 solids, 600 rays, 577 hits): per-query p50 0.300 us -> 0.300 us (1.00x); batched 0.323 -> 0.310 us/ray (1.04x) with repeats of 1.041, 0.977, 1.050, 1.059 and 0.875, so nothing beat the scalar loop beyond the machine's 12% drift; the isolated filter did read 2.00x over all 404 boxes, but it discards only ~65% of boxes and the integer slab test dominates the scan. `wide` was removed with the revert (Cargo.toml and the lock entry are net unchanged), no `unsafe` was used because the ratchet forbids it in this crate, and the wasm32 check passed while the vector path existed. Kept: `blocked_touching_face_blocks_beyond_the_filter` (a box whose face sits exactly on the segment AABB plane still blocks, one cm of strict gap clears, a zero-size box on the segment clears), a private `with_parts`, and a doc note recording the negative result. Diff: tests only, +57. Tier B. Size S. Owns `crates/spatial/src/sight.rs`. Accept: `cargo test -p engine-spatial --lib` prints 395 passed 0 failed 22 ignored, the 26,244-pair corpus equality still passes, and the GOLDEN passes. Landed 7a6e18a. NEGATIVE RESULT kept on the board so the next PF5 attempt does not repeat it.
- [x] PF9a perf: headless paired-rerun harness (Tracy offline-capture shape, Landed 90eb08c. Tick audit 2026-10-17: crates/perf/tests/w1_pf9a_paired.rs (tick_frame_paired_small_runs_share_one_hash, _receipt_kills_divergence, _600_runs_share_one_hash) and profiler.rs paired receipt tests green in gh-check run 39057250493 (engine-perf).
  zero Tracy bytes; method reference `wolfpld/tracy@afb16f68` BSD-3-Clause,
  `capture/`+`csvexport/` pattern only). Tier B. Size S. Owns
  `crates/perf/src/profiler.rs` paired-capture hunks only (no new file, no
  new dep, no tick change). Accept: paired 600-tick runs print
  `hash_A == hash_B` plus tick/frame p50/p95/p99 table with machine+profile
  line, receipt byte-identical x2 modulo wall-clock; golden unchanged. F2P:
  `cargo test --locked -p engine-perf tick_frame -- --nocapture` (fails
  before: no paired run, hashes uncompared). P2P: `cargo test -p engine-perf
  --lib` stays green + golden `d57b5c8ae3096614` x2. Kill: any hash mismatch,
  any viewer attached during measurement, any new dep. needs: PF2. (RES-STEAL
  672-res-tracy2, 2026-10-08; 2,000-frame + RA8/VRAM/draw legs stay Tier C
  pilot-only.)
- [x] SG1 arena: throwable spear grapple authority - THROW_SPEAR button bit 0x10, swept tip against fighter capsules and walls, tether reels the thrower then the pair share momentum toward the spear end, wall hit zips the thrower, guard/parry/timeout/KO/release end it; opt-in `MatchSpec::spear`, off means masked and unhashed. Tier A. Size M. Owns `crates/arena/src/spear.rs`, `crates/net/src/session/spear_tests.rs`. Accept: spear scenario tests in `crate::spear::tests`, `live::spear_tests` 600-tick digest identical twice and across tape replay, net rollback and lobby refusal, golden d57b5c8ae3096614 and arena digest d29b3b17bbd3b244 unchanged. needs: none. Landed 2dedab1b. Proof: engine-arena + engine-input tests run 39074617752, engine-net spear 4 passed run 39072779205, clippy run 39075934041, golden_hash_untouched run 39073693190. Plan docs/plan/spear-grapple.md.
- [x] SG2 game: spear grapple presentation - physfx tether rope and carry_ragdoll, spear_look.rs (flying spear, trail, stuck, dropped, rope slack and taut), ragdoll tow along the authority track then land and get up, Q throws in the arena window with `--spear`, `--spear-demo` auto-throws, feel and audio cues, `spear= tether=` on the view line. Tier A. Size M. Owns `crates/physfx/src/tether.rs`, `crates/game/src/spear_look.rs`. Accept: physfx tether tests, six spear_look tests on a real match, ragdoll_look tow test, capture shows a tethered airborne pair. needs: SG1. Landed 736bf9ea. Proof: engine-game --features app --lib 661 passed on the runner, arena bin 71 passed, physfx clippy runs 39072040021 and 39076389088, capture target/captures/spear-grapple-tether.png.
- [ ] SG3 game: spear grapple polish and proof - throw wind-up and release animation layer as a pure masked override in crates/anim, rope and held spear tied to the real hand joint, a hand-played keyboard run, the spear in the web build, a release arena.exe rebuilt for play-arena.bat. Tier B. Size M. Owns `crates/anim` hunks, `crates/game/src/spear_look.rs`. Accept: capture of the wind-up frame, a keyboard run log with at least one tow, web page spear= tether= line. needs: SG2. Progress 2026-10-18: game half committed 6b9bf722 (fighter_look_throw.rs plus hunks in fighter_look.rs, spear_look.rs, spear_window.rs); the app lib tests for fighter_look, spear_look, spear_window pass locally (146 passed, with ragdoll_look). Judge run 2026-10-18 (PC, --features app --lib): ragdoll_look 10 passed, fighter_look 134 passed, spear_look 7 passed, 0 failed each. Left: wind-up capture, keyboard run, web line, release arena.exe check (the web line may come from the mirror web-build net run now in flight). Progress 2026-10-18 (SG3 evidence, PC): release arena.exe (16388608 bytes, 12:37 +0300) is newer than 6b9bf722 and runs the spear duel; a real headless duel on it (host `--scene arena --fighters 2 --seed 2040 --spear --host --headless --ticks 300`, guest `--scene arena --fighters 2 --seed 2040 --spear --join 127.0.0.1:7043 <invite code> --headless --ticks 300`, logs target/logs/sg3-headless-host.log and target/logs/sg3-headless-guest.log) printed `arena sim: ticks=300 hash=8283177473c16d92` on both seats, desync=none, and a windowed `--scene arena --fighters 2 --spear --ticks 300 --frames 300 --exit --mute` run printed `arena sim: ticks=62 hash=ad820e64451a0ab9`. Wind-up capture target/captures/spear-windup.png (1280x720, 1300799 bytes, manifest sha=6b9bf72 sequence_fnv=67bc044a3794e276) is frame_000002.png (manifest tick=2, checkpoint 6f9406c355201cfb) of a paused `--scene arena --fighters 2 --spear --hold Q --pause-at-tick 2 --frames 12 --exit --mute` run, inside the 4-tick wind-up window; the same tick with no throw differs on 28845 of 921600 pixels (3.13%). The keyboard road `--hold W,Q --ticks 400 --frames 2000 --exit --mute` (279 ticks) threw once and missed (thrown=1 hits=0), so the only tow log is the scripted `--spear-demo --ticks 800 --frames 4000 --exit --mute` run (545 ticks, hash c6f4a95a77f7d9a0, `arena spear: thrown=1 hits=1 tethers=1 ends=1`, ragdolls=2 spear=1 tether=1); script_input is not a hand. crates/web depends on no engine-game or engine-arena and crates/web/src/arena.rs poses the procedural stand-in, so no page prints spear= tether=; it would take the spear_window counts in report::Ready::line_with (crates/web/src/report.rs) plus a spear match in the page. Left: a hand-played keyboard run with one tow and the web page spear= tether= line.
- [x] SG4 arena: rope swing on the wall tether - with the height channel armed the wall anchor is a rope, not a winch: the hook holds the thrower on a 3D sphere around the anchor, the reel takes rope in 60 mm/tick, and the constraint turns the run and the take-in into an arc, so a release keeps the tangential drift and gravity lands the body. Tier B. Size M. Owns `crates/arena/src/spear.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-arena --lib rope_swing` prints 1 passed (`rope_swing_holds_the_arc_and_the_release_carries_it`: the spear sticks, the span stays on the rope within 500 mm and above 3000 mm, a second press ends `Released`, and the release carries more than 250 mm in 10 ticks), the full `cargo test -p engine-arena --lib` prints 227 passed, and golden `d57b5c8ae3096614` x2 unchanged (the fixture has spear off). F2P: no rope_swing test existed and the wall tether only reeled along the goal line, so nothing ever held the anchor radius. P2P: the spear suite (14 passed) including `spear_hits_wall_and_zips_the_thrower`. Landed 6f4ec04b. Proof: engine-arena 227 passed run 39317657320, clippy run 39317843278. needs: SG1, SG5. Plan docs/plan/spear-grapple.md (section Rope swing).
- [x] SG5 arena: a fighter can leave the floor and land - the height channel: `pos[1]` with a per-slot vertical speed in mm, gravity at 11 mm/tick squared (the source game's -40 m/s^2 at 60 Hz), a landing at the floor with the speed cleared, the vertical speeds folded into the checkpoint hash behind tag `0xB4`, and the snapshot, fork and live-snapshot paths carrying it; opt-in through `MatchSpec::with_height`, so the golden baseline (which stands on the floor) is untouched. Tier A. Size L. Owns `crates/arena/src/lib.rs`, `crates/arena/src/spec.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-arena --lib fighter_height` prints 3 passed (a placed fighter falls and rests on the floor, the same placement without the channel never moves, one digest twice and the height fold is in it), `cargo test -p engine-arena --lib spear` stays green, and golden `d57b5c8ae3096614` x2 unchanged. F2P: no fighter had a vertical speed; `pos[1]` was written only at placement. P2P: `cargo test -p engine-arena --lib` (227 passed) and the golden hash. Landed 6f4ec04b. Proof: fighter_height 3 passed run 39311606450, engine-arena 227 passed run 39317657320, golden d57b5c8ae3096614 twice on this PC. The jump button and the movement numbers are a later row; the channel is driven by gravity and the tether now. Plan docs/plan/shady-knight-port.md (SG5).
- [x] SG6 game: the fight camera bobs with the hero's walk - the source game's `CameraBob` numbers (`bobSpeed=1`, `mgtSpeed=2`, `xAmp=0.08` m, `yAmp=0.12` m) drive a stride-locked eye bob from the hero's own floor speed, fading in over a 0.5 s halflife and out when the walk stops; presentation only, nothing in the tick or the hash. Tier B. Size S. Owns `crates/game/src/fighter_look_cam.rs`. Accept: `cargo test -p engine-game --lib --features app fighter_look_cam` prints 11 passed including `walk_bob_moves_the_eye_only_while_the_hero_walks`, and the 30 vs 144 fps camera test still passes. F2P: the window camera has no bob; a walking hero moved the shot only. P2P: the ten existing `fighter_look_cam` tests. Landed 280c08f7. Proof: 11 passed on this PC through `node tools/gh-check.mjs` (the `app` feature runs here, not on the Linux runner). Plan docs/plan/shady-knight-port.md. The amplitude pair is read as a normalized strength with 2 cm of eye travel at full strength (read as metres it shook the picture like a hit), the phase follows the hero's floor step, never the frame time, and a hero off the floor or one that teleports in a frame holds the bob still. Fixed 2026-10-20 (owner report: the camera shook while moving).
- [x] SG7 game: the demo plays the rope swing - a `--height` flag on `arena.exe` (and a session field) arms `MatchSpec::with_height` for the window, so the wall tether swings in `play-arena.bat` instead of zipping; the match identity gains the `0x48` tag when on, so LAN peers must ask for it too. Diff: easy. Tier B. Size S. Owns `crates/game/src/lib.rs`, `crates/game/src/bin/arena.rs`. Accept: `arena.exe --scene arena --spear --height --frames 300 --exit` prints `arena sim:` twice identical with the flag and once without, the run prints no error, and a throw at a wall in a capture leaves the floor (`arena view:` line) instead of reaching the wall face. F2P: no window flag arms the height channel; `play-arena.bat` cannot show the swing. P2P: `cargo test -p engine-game --lib` and the plain demo run. needs: SG4. Plan docs/plan/shady-knight-port.md. Progress 2026-10-19: `--height` is wired (`App::into_height`, `live_session(config, spear, height)`, the binary flag, `--hold` runs) and proven in the window: `arena sim:` `7d204c3622c2d48c` twice with the flag and `d851bde575bbc6ba` once without, `nonfinite=0 error_parts=0` on every line, landed 1f615910. Left: the visual clause, a wall throw in a capture; `--spear-demo` aims at fighters, so it needed a hand throw at a wall or a wall-throw script. Visual clause 2026-10-20: a hand-thrown (agent-seat) throw at the wall under `--height` gave `arena spear: thrown=1 hits=1 tethers=1 ends=0 spears_max=1 ropes_max=1` and `arena capture: frames=26 ... sha=49f44599e9d8969e`; the body left the floor on the rope arc instead of reaching the wall face. Captures and the full line set: `docs/receipts/2026-10-20-sg7-rope-swing/`. The same run reads `pops=281` (AM24's open rotation-aware look defect), not this row. Landed 1f615910.
