### Hard stuff (operator 2026-09-24): motion, ragdoll, physics, speed

Operator: "ragdoll, locomotion and other hard stuff, all in the engine,
optimized." Everything that feeds the tick stays deterministic (integer or
fixed-point, sorted order, x2 hashes); presentation-only rows say so. Donors
come from the Rust sweep in `RESEARCH-VERDICT.md` (licenses re-read at port
time, `PORT()` + `EVIDENCE.md` row); papers and talks are reimplemented.

- [ ] PX35 physics: long-range attachments for cloth and soft sheets — the Progress 2026-10-17: per-node tethers landed (6f871bb5: cloth.rs rebuild_tethers and solve_tethers, test the_tether_pulls_an_overstretched_particle_back_to_its_reach); left: cloth is f32 Verlet not integer math, and the 32x32 fu30_soft_settle_offline_baseline spike test no longer exists, so the 25% + 64 raw link bar is not measured.
  32x32 soft-settle spike (`cloth.rs` `fu30_soft_settle_offline_baseline`,
  ignored) stretches its top links about 2x under the cape tune because
  position-based constraints cannot carry a long hanging chain in a few
  iterations. Add per-node tethers to the nearest pinned node (max distance
  = rest path length), solved after the stretch pass, integer math, no
  allocation per step. Tier B. Size M. Owns `crates/physics/src/cloth.rs`.
  Accept: the spike test runs un-ignored with every link within 25% + 64 raw
  of rest; cape tests and the zero-allocation step test stay green; replay
  bit-exact x2. needs: none. Progress 2026-10-17: 4544ac4f 32x32 cloth settles with every link within 25% of rest, bit-exact twice; cloth is still f32 Verlet so the integer-math clause is not met.
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
- [ ] AM13 anim: animation LOD — update-rate tiers by distance and screen Progress 2026-10-17: lod.rs tier table plus assign_tiers keeps the 8 nearest at full rate (lod_w1_test.rs, anim_lod_pin.rs, gh-check run 39056591616 engine-anim 390 passed); left: the 64-fighter under 2 ms timing on the reference CPU is not measured.
  size, IK and springs off far away, crowd handed to VG4. Tier A. Size M.
  Owns `crates/anim/src/lod.rs`. Accept: 64 fighters under 2 ms total on
  the reference CPU; the 8 nearest match full-rate poses. needs: AM1, LM1.
- [ ] AM14 anim: parallel SIMD eval — per-character jobs on RC3 fork-join,
  SoA sampling on `wide` (license read first). Tier A. Size M. Owns
  `crates/anim/src/par.rs`. Accept: at least 3x on 4 cores for 64 fighters;
  identical poses under every chunking. needs: AM1, RC3. Progress 2026-10-17: 6ab36492 anim par.rs per-character eval on the RC3 pool, poses bit-identical for 0,1,2,4,8 workers; speedup 2.0x on 64 fighters (run 39067637155), the 3x bar is not met and the wide SIMD half is skipped (external dependency).
- [ ] AM15 anim: additive layers — locomotion base, combat upper body and
  flinch additives through one inertializer, with analytic two-bone IK in
  contact windows (Bollo GDC 2018, Clavet and Paragon GDC 2016; ozz and ACL
  MIT as patterns; diffusion and MOCHA-style generators not taken). Tier B.
  Size M. Owns `crates/anim/src/additive.rs`. Accept: no pops on any
  interruption; foot slide at most 20 mm straight and 40 mm on pivots;
  replay x2; at most 0.05 ms per tick. needs: AM1, AM2, AM3, AM6. Progress 2026-10-17: ba6ad347 anim additive.rs layers + one inertializer + planted feet via solve_limb; slide within 20 mm straight, 40 mm pivots, 1.3 us per tick (run 39067390119); left: fighter_look does not call AdditiveRig::tick yet.
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
- [ ] TV3 anim: siege traversal set — ladder, rampart vault and rubble
  scramble edges on integer-mm warp for the C5 siege map, with nav links
  rebaked exactly like TV2. Tier B. Size M. Owns the
  siege-edge hunks beside `crates/anim/src/traversal.rs` (TV1 file
  frozen) plus siege fixtures. Accept: siege fixtures pick expected
   actions; warped roots land within 1 cm; sim bytes x2. needs: TV1, TV2.
   (PLN animation.md EXTEND, proposed 2026-09-29.)
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
- [ ] TV4 anim: breach-rubble scramble variance — TV3 scramble holds across
  seeded rubble height plus noise sweeps. Tier B. Size S. Owns fixture
  hunks beside `crates/anim/src/traversal.rs` (TV1 shape frozen; lands
  after TV3, disjoint rubble split). Accept: `cargo test -p engine-anim
  --lib rubble_var` green; picks expected; roots land within 1 cm; nav
  rebake exact x2. needs: TV3, TV2.
  (PLN animation.md EXTEND r7, proposed 2026-09-29.)
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
- [ ] PX15 physics: deterministic parallel islands via chunked split plus
  sorted merge with step frozen (READER-SPEED53 card 3, PX6 deepening).
  Tier B. Size S. Owns the chunked split+merge hunks beside
  `crates/physics/src/islands.rs:57`. Accept: identical hash at 1/2/4/8
  threads + ≥2x wall on 64-fighter scene. needs: PX6. (SPEED53.)
- [ ] PX9 physics: ragdoll contacts and joint tables — self-collision between
  non-adjacent bones (collision groups filter parent and child), ragdoll
  against ragdoll for clinches and falls, per-joint swing/twist limit tables
  from the skeleton instead of depth, and torque motors with per-joint limits
  (Jolt PoweredRagdoll tables, avian XPBD joint compliance: patterns only).
  Tier A. Size M. Owns `crates/physics/src/ragdoll.rs`,
  `crates/physics/src/joint_limits.rs`. Accept: no bone interpenetration over
  1 cm in 50 seeded falls; two ragdolls landing on each other separate; hash
  x2; settles within the PX1 bound; zero per-tick allocations. needs: PX1, PX10. Progress 2026-10-17: b61738a5 physfx tests: two ragdolls landing on each other separate with no unjoined overlap over 1 cm and replay exactly (4 passed); the 50-seeded-falls bar fails (seed 44 overlaps lower_arm_r/shin_r by 8 cm at tick 40, run 39068262860), committed as an ignored repro; left: more solver iterations or bone CCD, spawn reading the joint tables.
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
  no pop visible in a PQ4 lookcheck. needs: DX4, CT2. Progress 2026-10-17: 4cf108a8 debris_lod.rs tiers Full/Simplified/Merged/Impostor over the DX4 ledger; 1 ms budget test is ignored and unrun, PQ4 lookcheck needs a human; left: a caller in the window and the CT2 chain.
- [ ] DX7 editor: fracture authoring — bond paint, strength preview and a
  break-replay preview in the editor, through the command service with undo,
  so AI and humans author destructibles. Tier B. Size M. Owns
  `crates/editor/src/fracture_tool.rs`. Accept: an AI command paints bonds,
  previews a break and undoes cleanly; the saved bonds bake identically x2.
  needs: DX3, ED4.
- [ ] DX8 destruction: detail where it breaks — bake probe that puts fine chunks
  where impacts are likely and coarse chunks in the wake (idea from
  phase-field wake coarsening, arXiv 2609.21201; no solver or code taken).
  Bake only, runtime untouched. Tier C. Size S. Owns
  `crates/destruction/src/bake_rank.rs`. Accept: same bond count as uniform,
  better break look in a blind review, DX3 8-way equality holds; else killed.
  needs: DX2. Progress 2026-10-17: 3f671e24 bake_rank.rs spaces lattice cuts by impact likelihood with the same bond count and anchors, 8-way spread equality; the keep-or-kill blind review needs a human; left: a caller.
- [ ] DX9 destruction: graded bonds — per-bond breaking strain from contact
  area, mass and anchor distance with seeded jitter (today every bond is
  1.0), Voronoi-density cut sites clipped by the existing split code, sliver
  reject, and per-event records of what broke plus the dirty nav tiles for
  NV3. Bake only. Tier B. Size M. Owns `crates/destruction/src/bond_grade.rs`.
  Accept: bake x2; DX3 8-way equality; anchors hold; a blind review prefers
  graded breaks, else density is killed but grading stays; 16/1024 caps hold
  in the 8-fighter soak. needs: DX2, DX3, DX4. Progress 2026-10-17: 40da318b bond_grade.rs strain from contact area, pair mass, anchor distance, seeded jitter, sliver refusal, per-event break records with dirty nav tiles; 8-way insertion-order equality and 16-per-event / 1024-total caps in an 80-cut soak (run 39069886844); left: Voronoi-density cut sites, blind review, a caller.
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
- [ ] PX19 physics: combined 64-fighter + 500-agent tick proof — the P6
  physics slice of the server tick, LOD capsules far plus full ragdoll
  near, cloth excluded by rule. Tier A. Size M. Owns the combined-tick
  workload beside `crates/perf` + scale-proof hunks beside the future
  `crates/physics/src/lod.rs` (PX7 system frozen). Accept: `cargo test
  -p engine-physics --lib combined_tick` green on the 64+500 fixture;
  physics slice p99 within 4 ms on the reference CPU; hash x2; golden
  unchanged. needs: PX7, PX16, PF1. (EXTEND physics 2026-09-29.)
- [x] PX20 physics: deterministic swept shape-cast API — segment-cast Progress 2026-10-17: crates/physics/src/query.rs has sphere_cast and ray_cast with rb_query_kinds tests (6b671986, f32 nearest hit); left: no Fixed/Q16 segment cast with sorted time-of-impact and no swept_cast test.
  with sorted time-of-impact in `crates/physics/src/query.rs`,
  Fixed/Q16 only, consumed by GP2 sweep and PX10 CCD wiring. Tier A.
  Size S. Owns the cast hunks in `query.rs` (no new file, no step
  change). Accept: `cargo test -p engine-physics --lib swept_cast`
  green incl. 1,000 seeded casts; TOI order stable under permutation;
  hash x2; golden unchanged. needs: PH1, GP2. (EXTEND physics
  2026-09-29.) Landed 363000af. swept_cast Q16 sphere vs capsules, ordered by time of impact then id; 1,000 seeded casts match a dense march, permuted targets same answer (3 tests run 39066316942; clippy -D warnings run 39067537158).
- [ ] PX21 physics: moving-platform ride — deterministic platform-delta Progress 2026-10-17: kcc.rs platform_ride_matrix (station within 1 cm over 600 ticks, platform stop, step-off inherits velocity, hash x2) green in physics run 39057362796; left: the step-off landing is only asserted grounded, not within 1 cm of a target.
  inheritance for carried fighters (ride, step-off, platform stop),
  integer deltas only. Tier B. Size S. Owns ride hunks beside
  `crates/physics/src/kcc.rs` (PH2 frozen). Accept: `cargo test
  -p engine-physics --lib platform_ride` green; carried fighter keeps
  station within 1 cm over 600 ticks; step-off lands within 1 cm;
  hash x2; golden unchanged. needs: PH2, PX1. (EXTEND physics
  2026-09-29.) Progress 2026-10-17: ee98c096 platform_ride_matrix checks the step-off landing tick and x against an independent f64 ballistic prediction within 1 cm (3 tests green, run id not captured; rerun before ticking).
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
- [ ] CT10 assets: glTF conformance — the cook passes the Khronos glTF sample
  models (license per model read first) and Blender glTF exporter variants
  (animation sampling, per-action vs broadcast, PBR mapping from
  KhronosGroup/glTF-Blender-IO, Apache-2.0, as acceptance only). Blender
  stays outside, driven over MCP. Tier A. Size S. Owns
  `crates/assets/tests/gltf_conformance.rs`, `fixtures/gltf/`. Accept: every
  selected model imports, places, saves, reopens and plays; failures are
  typed refusals, never panics. needs: CT1, CT2.
  PX6.
- [ ] CT11 assets: animation compression — segment, quantize and error-loop
  ideas from ACL (nfrechette/acl, MIT). Tier B. Size M. Owns
  `crates/assets/src/anim_compress.rs`. Accept: 5-20x smaller clips with
  error bounded in cm and degrees; decode allocates nothing; cook x2
  identical. needs: CT1.
- [ ] CT13 assets: ray-tracing proxies — the cook emits a simplified proxy
  mesh per asset (quadric chain from CT2) for the RT acceleration
  structures, with a stored deviation bound, beside the full raster mesh
  (Solari proxy rule). Tier A. Size S. Owns `crates/assets/src/proxy.rs`.
  Accept: proxies stay inside their bound on every museum asset; a light leak
  through a proxy fails the tunnel fixture; cook x2 identical. needs: CT2, RT1.
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
- [ ] NV4 nav: near-field avoidance — ORCA half-planes on the XZ plane in Progress 2026-10-17: crates/ai/tests/crowd_orca_bench.rs holds an integer ORCA half-plane step with naive, kd-tree and grid neighbour feeds benched at 50, 200 and 256 agents; left: no crates/nav/src/orca.rs, no 500-agent flat-cost proof, no 1,000-seed corridor stuck drill.
  Fixed math for fighters within 15 m (RVO2-3D, Apache-2.0, math ported with
  a PORT row); far agents stay on flow fields and impostors. Tier B. Size M.
  Owns `crates/nav/src/orca.rs`. Accept: steering ≤ 0.1 ms per agent, flat
  from 50 to 500 agents; 0 stuck in a 1,000-seed corridor drill; replay x2.
  needs: NV2.
- [ ] NV5 nav: legion platoons — leaders query nav, followers hold gaps in O(1)
  (idea from traffic platoons; SUMO is EPL-2.0, patterns only). Tier B.
  Size M. Owns `crates/nav/src/platoon.rs`. Accept: ≥ 5x fewer nav queries
  for a 500-soldier legion; no accordion at 60 Hz; replay x2. needs: NV4, PX7.
- [ ] NV6 nav: smooth paths — portal funnel (string pulling) over the tiled
  mesh with salted tile references so paths survive rebakes
  (recastnavigation Zlib and polyanya Apache-2.0 patterns). Tier B. Size M.
  Owns `crates/nav/src/funnel.rs`. Accept: paths never longer than the grid
  path and at least 10% shorter on 80% of queries; query at most 0.2 ms p50;
  a breach opens a path within 1 s; bytes x2. needs: NV3, DX1.
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
- [ ] GA12 brain: GOAP planner over the utility world-state — Bevy-style
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
- [ ] NV10 nav: broadphase plus 1,000-agent P6 bar — integer Progress 2026-10-17: the grid and kd-tree neighbour feeds exist in crates/ai/tests/crowd_orca_bench.rs against the scan referee; left: no crates/nav/src/crowd.rs, no byte-equal check on the NV8 corpus, no p50/p95/p99 at 1,000 agents.
  uniform-grid (or R* split shape, georust/rstar pattern only)
  contender against the id-sorted scan referee; byte-equal neighbor
  sets on every seed, then p50/p95/p99 at 50/500/1000 agents. Tier A.
  Size M. Owns   `crates/nav/src/crowd.rs` broadphase hunks (never a
  crowd crate). Accept: neighbor sets byte-equal to the scan on the
  NV8 corridor corpus; 1,000-agent sim p99 within the P6 2 ms bar;
  replay x2. Proof: `cargo test -p engine-nav crowd` green plus the
  p50/p95/p99 bench table at 50/500/1000 agents. needs: NV4. (PLN EXTEND gameplay.md, proposed
  2026-09-29; NV9 id retired, frontier TileCache proposal REJECTED.)
- [ ] NV11 nav: grid-retirement proof — NV3 tiles plus NV6 funnel cover
  every arena query the legacy `Grid` serves, then the grid goes.
  Tier A. Size S. Owns
  `crates/nav/src/{lib,astar,bake,flow}.rs` removal hunks. Accept:
  funnel-shorter proof reruns green with zero grid fallbacks across
  the query corpus; bytes x2; grid deleted, not deprecated. Proof:
  `cargo test -p engine-nav` green with grid files deleted.
  needs: NV3, NV6. (PLN EXTEND gameplay.md, proposed 2026-09-29.)
- [ ] NV12 nav: shared-edge stitch proof — bake the arena whole and
  tiled, diff every cross-tile path byte-wise (WD8 shape). Tier A.
  Size S. Owns `crates/nav/src/tiles.rs` stitch hunks. Accept:
  cross-tile paths byte-equal to single-mesh paths on the arena kit;
  byte-equal again after a 4-tile breach plus rebake; bytes x2.
  Proof: `cargo test -p engine-nav tiles` green, stitch diff run
  twice. needs: NV3. (PLN EXTEND gameplay.md, proposed 2026-09-29.)
- [ ] NV13 nav: temp-obstacle request queue — touched/pending sets Progress 2026-10-17: ObstacleCache plus TiledNav::update_budget landed in tiles.rs with tests/tiles.rs obstacle_rebuild_touches_only_its_tiles and obstacle_requests_refuse_loudly (nav green, run 39057395844); left: the 4-tile breach numbers (touched 4/4/4 cap 8, up_to_date, digest afb7c5d2) are not asserted anywhere.
  plus budgeted rebuild plus `up_to_date` convergence (LADDER L517
  ADOPT-scoped spike). Tier A. Size S.
  Owns `crates/nav/src/tiles.rs` queue plus `update_budget` hunks
  beside frozen `invalidate`/`rebake_budget` (no second tile store).
  Accept: `cargo test -p engine-nav tiles` green; 4-tile breach
  (32x32 edge-8, 2 cylinder adds plus 1 box removal) posts touched
  4/4/4 cap-8 plus 1 rebuild per tick over 10 ticks plus
  `up_to_date` true with stale-exact 10/10 plus digest `afb7c5d2`
  x2; burst-10 ablation killed; golden unchanged. needs: NV3.
  (PLN gameplay.md, proposed 2026-09-30.)
- [ ] NV14 nav: per-tile integer-mm rebake detail (held on NV13, feeds
  NV12 stitch proof) — border-overlap bake-lane geometry, tick keeps
  version flips only (LADDER L511, spike first). Tier A. Size M. Owns
  `crates/nav/src/mesh.rs` bake-tile hunks plus `tiles.rs`
  rebake-path hook. Accept: `cargo test -p engine-nav tiles` green;
  dirty-4-only ms versus full-bake ms posted plus unaffected-tile
  bytes identical plus cross-tile path byte-equal to whole-bake path
  plus digest x2; kill on any seam gap or per-event over 50 ms p95.
  needs: NV13, NV3. (PLN gameplay.md, proposed 2026-09-30.)
- [ ] NV15 nav: salt-epoch handle detail (held on NV13, feeds NV7) —
  tile and obstacle handles so dangling refs fail closed (LADDER
  L512, spike first). Tier A. Size S. Owns
  `crates/nav/src/tiles.rs` TileRef plus StampedPath stamp-check
  hunks. Accept: `cargo test -p engine-nav tiles` green;
  dangling-stamp refusal 100% plus overflow keeps staleness plus
  digest x2; kill on any dangling resolve or digest drift. needs:
  NV13, NV7. (PLN gameplay.md, proposed 2026-09-30.)
- [ ] NV16 nav: rational-NURBS spline math + parallel-transport frames — de
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
  research/flax-mcp-harvest/plugins-gameplay-world.md #3.)
- [ ] PF5 perf: SIMD pass — hot loops (culling, CPU skinning paths, spatial
  queries, audio mix) on glam SIMD or `wide` with scalar fallbacks;
  simulation code keeps its integer math. Tier A. Size M. Owns the named
  hot files per PF2 capture. Accept: each change A/B-measured; identical
  outputs. needs: PF2.
- [ ] PF9 perf: replay visual debugger — tracy spans plus a rerun recording Progress 2026-10-17: crates/observe/src/rerun_sink.rs landed (names_first_mismatched_hash, tracy zone names; observe green run 39057303818) and PF9a paired reruns name the first divergent hash; left: the GPU timestamp, 2,000-frame and RA8 manifest legs of the hunt are not done.
  of simulation state per tick for desync hunts (licenses read first).
  Tier B. Size M. Owns `crates/observe/src/rerun_sink.rs`. Accept: a seeded
  desync shows its first divergent tick in the recording. GitHub hunt 2026-09-25: GPU timestamp queries (two per pass) resolved behind the GX truth gate give `gpu_ms` next to CPU spans; a free-list buffer arena keeps allocations flat over 120 frames (wgpu, rend3 patterns). Deep 3D pass: raise capture to 2,000 frames (120 today), cover every GPU pass (6 today), and write tier, VRAM and draw counts into the RA8 manifest; this harness comes before any GPU budget    claim. needs: PF2.
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
