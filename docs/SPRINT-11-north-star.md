### North star 2026 (S44) (owner 2026-10-03, `docs/plan/frontier.md`; S47: Bevy is the body, engine2040 the brain — pillars 3 and 5 name the Bevy cover per row)

Six pillars, 24 rows, gaps only — every row names the existing ids it
extends (verified by `node tools/plan-atlas.mjs --find` and board-slice at
write; NS prefix verified free). Each Accept: F2P test + P2P golden
`d57b5c8ae3096614` unchanged.

Physics (extends PX1, BX4, BX12, DX5, BX13):

- [x] NS1 physics: articulated chains as one constraint graph — winch plus
  rope plus load (chariot trace, portcullis lift) with integer servo targets
  and canonical solve order. Size M. Owns hunks beside
  `crates/physics/src/powered.rs`. Accept: F2P `cargo test -p
  engine-physics --lib articulated` green (3-chain lift, same ticks x2
  shuffled); P2P golden `d57b5c8ae3096614` unchanged. needs: PX1. Landed 078fd79. cargo test -p engine-physics --lib articulated: articulated_three_chain_lift_raises_the_platform_the_same_way_x2_shuffled (3 chains x 3 links, 200 kg platform, replay + 8 shuffled builds give the same lift tick, height and hash) and 5 more, local run; golden untouched (std-only module, no caller yet).
- [x] NS3 physics: ragdoll-vs-ragdoll clinch contacts — two clinching Progress 2026-10-17: `resolve_spacing` in crates/physics/src/ragdoll.rs parts clinched bone pairs and the 660-tick clinch tape runs in ragdoll_tape.rs (it reports floor penetration only). Left: a fighter-vs-fighter max-penetration under 1 cm test x2 shuffled.
  fighters separate without interpenetration under canonical (fighter, joint)
  order. Size M. Owns hunks beside `crates/physics/src/ragdoll.rs`. Accept:
  F2P clinch tape max penetration under 1 cm x2 shuffled; P2P golden
  unchanged. needs: PX1, AR2. Landed e5cfe68. Local run (GitHub mirror push failing today, see report): ragdoll_tape::tests::clinch_fighters_part_under_a_centimeter_x2_shuffled (clinch overlap 0.0020 m parted vs 0.3000 m unparted; repeat and B-then-A hand-over give the same result) and parting_pushes_two_fighters_that_start_inside_each_other_apart; engine-physics --features rapier --no-fail-fast 252 lib + 17 it passed, clippy -D warnings clean; engine-proof --features ragdoll ragdoll_matrix 5 and golden 5 passed (Ragdoll::step and state_hash untouched).
- [x] NS4 proof: joint and contact conformance vs Jolt/Rapier oracles —
  offline oracles only (ideas reimplemented, no bytes), our servos and limits
  inside tolerance. Size S. Owns hunks beside `crates/physics/src/verify.rs`.
  Accept: F2P 50-pose oracle report green with max deviation posted; P2P
  golden unchanged. needs: PX1. Progress 2026-10-18: be32861 tests/it/oracle.rs, 50-pose report printed: servo max dev 0 (Rapier PdController vs pd_raw), contact max dev 3.4e-5 m (parry capsule vs clinch overlap), joint limits 0.65 rad over the table free and 1.10 rad over on a landing (Rapier limits are soft, table is depth only), pinned as a known gap; left: limits inside a tight tolerance (PX9/AD16 joint tables), a Jolt-side oracle. Progress 2026-10-20: 66d73ae crates/physics/tests/it/oracle.rs, the report now has six channels and the Jolt-side oracle (a swing-twist split, Jolt's SwingTwistConstraint idea reimplemented, no bytes): servo max dev 0, contact max dev 3.41e-5 m, limit max dev 2.47e-2 rad, jolt cone gap 0.2853 rad, landing 1.2141 rad, seam 0.6723 rad; the free limit fixture now runs with no ground under it, because the pinned 0.65 rad measured the coupling seam's floor probe lifting a chain that had fallen through the floor plane, not the joint, and the limits are now read the way Rapier reads an angular limit (the twist about the axis); proof gh runs 39473474246 (engine-physics --features rapier --no-fail-fast: 270 lib + 18 it + 10 other green, oracle report green), 39471098511 (clippy -D warnings clean), 39471097667 (golden d57b5c8ae3096614 unchanged); left: the soft Rapier limit still lets a violent landing overrun the table by 1.21 rad and our per-axis box lets the diagonal of a swing turn 0.285 rad further than Jolt's cone, so the hard joint limits are the joint table rows' work (PX9, AD16; the ragdoll fall digest test is already red in the tree since 763c2197, not from this row, so that re-pin is open too). Landed 7c89af3. fifty_pose_oracle_report_stays_inside_its_tolerances (gh run 39476434413): the 50-pose report is green and every max deviation is posted — servo 0.00e0 (our pd_raw against Rapier's PdController), contact 3.41e-5 m (our clinch overlap against parry's capsule contact), joint 2.47e-2 rad over the table row in free flight (bar 0.05: the joint holds its own row), Jolt-style swing cone gap 0.2853 rad; the landing channel now runs with the coupling seam off and reads the same 1.2141 rad, so that overrun is Rapier's soft angular limit alone and belongs to the hard joint limits (PX9, AD16), and the seam's own gap stays pinned at 0.6723 rad. P2P: engine-physics --features rapier --no-fail-fast green (run 39474276798), clippy --all-targets -D warnings green (run 39476433978), golden d57b5c8ae3096614 unchanged (run 39477077549).

Animation (extends AN1, AM2, TV1, PX4, LOC2, LMM1, PX1):

- [x] NS6 anim: full-body IK solve for carry and climb — spine plus limbs over Progress 2026-10-17: crates/anim/src/fbik.rs holds the reach, aim and grip full-body solve (reach_lands_inside_1cm_analytic_first, two_hand_grip_meets_at_the_weapon, steady_state_reuses_caller_buffers). Left: carry and climb fixtures over TV1 edges.
  TV1 edges with balance (PX4) as the veto. Size M. Owns hunks beside
  `crates/anim/src/fbik.rs`. Accept: F2P carry fixture: hands on load, no
  joint over its limit, x2; P2P golden unchanged. needs: TV1, PX4. Landed 8c8ba1f. carry_fixture_hands_on_load_and_within_limits green (cargo test -p engine-anim --lib carry_fixture, gh run 39183259521); full anim lib 446 passed (gh run 39183494005); golden d57b5c8ae3096614 unchanged (engine-proof golden_hash_untouched, gh run 39183496607).
- [x] NS7 anim: motion-matching style and terrain dials — LM1 integer search
  over style-tagged and slope-tagged clips from the LOC2 bake. Size M. Owns
  hunks beside `crates/anim/src/motion_match.rs`. Accept: F2P style query
  returns style clips 99% of 1,000 seeded searches, byte-equal x2; P2P golden
  unchanged. needs: LOC2, LMM1. Landed 957936a9. engine-anim --no-fail-fast 443 lib + 8 it passed (run 39106084274), clippy run 39106954469: motion_style a_style_query_returns_style_clips_in_99_percent_of_1000_searches (styled and graded, about a third without the dial), an_off_dial_is_the_plain_search_bit_for_bit, the_same_asks_give_the_same_bytes_twice; motion_match::search is now search_penalized with zero penalty, picks unchanged. No locomotion caller yet: it asks no style.

Look — Bevy is the body here (extends RA14, VS5b, BX7, RT14):


AI-native (extends EV1, MCP5, EA1e, LW5, HS21, CT4):

- [x] NS14 ai: LLM NPC brain with provider-off replay — recorded talk over a
  deterministic tick brain; provider off replays byte-equal. Size L. Owns
  hunks beside `crates/ai/src/mind.rs`. Accept: F2P Roman visit replays x2
  with the provider off, talk log identical; P2P golden unchanged. needs:
  EA1e, LW5. Landed b01c6337 (the brain, adapters, recorded log and replay_offline were built under LW1 to LW10 and the ludus; this adds the Accept test). ludus_npcs roman_visit_talk_log_replays_twice_with_the_provider_off: 20 provider-on visits record 60 talk lines, MindRuntime::replay_offline of that log runs twice with no provider and returns the talk log byte for byte plus the live checkpoint, and one changed line changes the checkpoint; with ludus_providers_off_still_plays and drain_twenty_offline_visits_apply_identically_twice. cargo test -p engine-game --lib ludus_npcs 13 passed, gh run 39106398594. No simulation code touched, golden unchanged.
- [x] NS15 endpoint: refusal catalog growth loop — HS21's 20 adversarial
  briefs grow from every real refusal with a dated reason; false refusals
  stay 0. Size S. Owns hunks beside `crates/endpoint/src/tool_capabilities.rs`.
  Accept: F2P catalog grows 20 to 30 with 0 legitimate false refusals; P2P
  golden unchanged. needs: MCP8. Landed 94d3e419. Catalog 20 -> 30 briefs (26 refuse, 87%), ten dated FIELD_BRIEFS, one legitimate brief admitted untagged; catalog_growth_is_dated_and_adds_no_false_refusal, adversarial_suite_refuses_at_least_80pct_and_writes_nothing, refusals_carry_the_right_axis_tag_and_hint green (run 39109659487). The repo keeps no log of client refusals, so the new briefs grow from refusal classes the gate emits; brief 25 found the unbounded unknown-tool echo, now cut to 64 escaped chars.
- [x] NS16 playtest: brief-to-game bundle that fails a seeded bad build — one
  brief, one seeded-bad variant, the bundle FAILs loud on the bad one and
  PASSes on the good one. Size M. Owns hunks beside
  `crates/playtest/src/brief_eval.rs`. Accept: F2P bad build refused with a
  typed reason, good build passes, x2; P2P golden unchanged. needs: CT4,
  EV1. Landed 2089d5bc. brief_eval run_bundle(brief, bad) returns the score or a typed BundleFault (Level, Character, Encounter, Replay{first_bad_tick}, Keypoints); seeded bad builds NoSpawn, NoProse, TamperedTick. Tests bundle_fails_loud_on_each_seeded_bad_build_and_passes_the_good_one (good build passes and is equal x2, each bad build refused with its typed reason x2), tampered_checksum_is_named_by_its_own_tick (EV16 audit names ticks 0, 8, 63, 119 exactly), empty_brief_bundle_is_refused_at_the_level_gate. No simulation code touched, golden unchanged. gh run 39105225319 (engine-playtest --lib highlights brief_eval trajectory scorecard frame_stale: 36 passed, 1 ignored) and full engine-playtest --lib 373 passed, run 39103617132.

Asset generation, intake only (extends CT40, CT28, LOC2, AS1):


Crowds (extends CT64, AU2, NV2, CT25, GA2, AM18, PX16):

- [x] NS24 proof: crowd determinism harness — 20k agents, one hash, two PCs.
  Size S. Owns hunks beside `crates/proof`. Accept: F2P 20k-agent 600-tick
  tape hashes x2; P2P golden unchanged. needs: PX16. Landed f6f90aa5. crates/proof/src/crowd.rs: 20,000 agents, 600 ticks, seed 2040, 30 seeded shoves, wake recall 30 of 30, same report x2, one hash pinned at 0x9627a1119822c2e0 (checkpoints every 100 ticks). The pin held on the Linux runner (engine-proof --no-fail-fast 160 passed incl. golden d57b5c8ae3096614, run 39103988897; first crowd run 39103482584 printed the value) and on this Windows PC (local cargo test -p engine-proof --lib crowd, 3 passed) : two machines, one hash. Clippy -D warnings (observe, proof, perf, --no-deps) run 39106155401 because engine-physics door.rs, not mine, fails workspace clippy.

Rules (extends RC5, decision D2 in `FOCUS.md`):

- [x] NS32c rules: quiet-box p99 for the arena adapter — new ignored test `arena_drive_p99` in Landed 47146997. `cargo test --release -p engine-rules --features arena --test arena_budget -- --ignored --nocapture`: arena drive p99 0.0411 ms per tick (16 scripted fighters, 600 ticks, 0 faults, bar 0.2 ms), fresh GitHub runner, gh run 39058176282.
  `crates/rules/tests/arena_budget.rs` (feature `arena`): 16 fighters, all scripted with
  `shield_bash_arena.rhai`, 600 ticks, time `RulesController::control` per tick. Size S.
  Accept: F2P `cargo test --release -p engine-rules --features arena --test arena_budget --
  --ignored` p99 at most 0.2 ms per tick on a quiet box (loops off or heavy queue idle); numbers
  in the row. needs: NS32a.
- [x] NS32d rules: second-PC replay of the scripted match - new test `arena_drive_hashes` in
  `crates/rules/tests/arena_drive.rs` plus `crates/rules/tests/data/arena_drive_hashes.txt`
  (seed 2040 and 7, 600 ticks, 2 fighters, half 12 m, slot 0 scripted: tick 599 hash per line;
  written once by an `#[ignore]` test `bless_arena_hashes`). Size S. Accept: F2P `cargo test -p
  engine-rules --features arena --test arena_drive arena_drive_hashes` equal on this PC and on a
  second PC; floats enter the arena hash, so a mismatch is a finding to write into
  `docs/BLOCKERS.md`, not a test to loosen. needs: NS32a. Landed 20789956.
  `cargo test -p engine-rules --features arena --test arena_drive arena_drive_hashes -- --nocapture`
  1 passed with both lines printed (run 39216183495): `2040 599 891ed62693a9c5dd` and
  `7 599 9a442b4b06b34310`, blessed on this PC and matched byte for byte by a Linux x86_64 GitHub
  runner, which is the row's second machine (a different OS, so the arena hash is machine-independent
  across the two machines tested). `bless_arena_hashes` is the ignored writer and resolves the fixture
  from `CARGO_MANIFEST_DIR`; every reported hash is computed twice before it is printed, so a drive
  that does not repeat fails in the helper instead of printing a number, and the mismatch message
  points at `docs/BLOCKERS.md` rather than inviting a looser bar. Whole target 8 passed, 1 ignored
  (run 39216603655); rust-quality engine-rules PASS 99 (A); workspace-check PASS; no simulation code
  changed, so the golden is untouched (test and fixture only).
- [x] NS32e rules: scripted slot on a walled scene through the door — new test
  `rules_drive_on_a_walled_scene` in `crates/endpoint/tests/it/rules_door.rs` (copy `insert_wall`
  from `tests/scene_drives_match.rs`): `rules.set`, one 2 m wall, `play.run` with `scene_rev`.
  Size S. Accept: F2P `cargo test -p engine-endpoint --features rules --test it
  rules_door::rules_drive_on_a_walled_scene` the scripted walled run differs from the plain walled run, equal
  x2, no fighter inside the wall in 18 of 20 seeds. needs: NS32b. Landed 5115a24.
  `rules_drive_on_a_walled_scene` in the endpoint `it` binary
  (`crates/endpoint/tests/it/rules_door.rs` — the module already existed, so no new
  test binary): a revision-2 document with the marker node and one 2 m x 0.4 m x 2 m
  wall, `rules.set` of the shield bash on slot 0, `play.run` with `scene_rev`: the
  scripted walled run differs from the plain walled run and its 600 checkpoints
  repeat on the same seed. The wall leg drives the same controller hook the door
  installs (`engine_rules::arena::RulesController` over `SpineMatch::with_controller`)
  for 20 seeds: 18 of 20 fights never put a fighter inside the wall (the assert is the
  bar). `cargo test -p engine-endpoint --features rules --test it rules_door::` 3 passed,
  gh run 39179264813; rust-quality scoped to the file PASS; workspace-check PASS. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-endpoint --features rules --test it rules_door::rules_drive_on_a_walled_scene` 1 passed, run 39230103525.

Make (extends HS26, decision D3 in `FOCUS.md`): generated props the AI can ask for.

