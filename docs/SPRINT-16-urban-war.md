### Urban war (owner 2026-09-26, Finish B second game; `docs/plan/urban-war.md`)

The second game: a Ready or Not / SWAT 4-style tactical CQB shooter in a
fictional modern city, growing into dense ruined cities with structural
destruction. Engine rows (UW1–UW26, UC1–UC12) are genre-neutral and
dispatch like 2040-tier rows, after the FOCUS "Now" writers; game and
content rows wait for UW0 (Finish A met). Combat never imports physics
(the composer adapts `SegmentCast`); no new engine dependency edges.

- [x] UW9 physics: stances and weapon collision — stand, crouch, prone
  capsules, ceiling-blocked changes, lean checked against walls, the
  weapon raised when its length meets geometry. Tier B. Size M. Owns
  `crates/physics/src/stance.rs`. Accept: 1,000 seeded rooms with no
  clipping; hashes x2. needs: PX8. Landed e4417ba. stance::tests::a_thousand_seeded_rooms_have_no_clipping_and_replay_x2 (1,000 rooms x 60 ticks, independent sampling oracle, hashes x2, ceiling/wall/raised counts above 100) plus 7 unit tests, local run; clippy -D warnings clean.
- [x] UW10 physics: doors — hinged doors with integer angles and states
  (closed, ajar, open, locked, wedged, trapped, broken); push, peek,
  kick, ram, shotgun and charge outcomes by table; state events feed
  nav links and audio. Tier B. Size M. Owns `crates/physics/src/door.rs`.
  Accept: every state × action fixture replays x2; a wedged door holds
  against a push; broken doors stop blocking casts. needs: UW1. Landed d670c68. cargo test -p engine-physics --lib door: every_state_by_action_fixture_replays_x2 (7 states x 6 actions), a_wedged_door_holds_against_a_push, broken_doors_stop_blocking_casts (real ray_cast, layer drops to debris) and 5 more, 8 passed in a local run; clippy -D warnings clean.
- [ ] UW11 anim: infantry layer — stance transitions, weapon upper body
  over AM15, aim through AM9 IK, reload, throw, cuff, hands-up,
  kneel-and-surrender, cower, and a true first-person view from the same
  body. Tier B. Size L. Owns `crates/anim/src/infantry.rs`. Accept: no
  pops across interrupts; the first-person camera never clips the body;
  0 allocations per frame. needs: AM15, AM9.
- [x] UW18 brain: squad orders — move, hold, fall in, stack, mirror,
  wedge, breach (open, kick, ram, shotgun, charge; then bang, stinger,
  gas or none), restrain, collect, split red/blue, as validated
  recorded actions. Tier B. Size S. Owns `crates/ai/src/orders.rs`.
  Accept: order tapes replay x2; bad orders refuse typed; legion
  goldens unchanged. needs: GA2. Landed 1751ea1a.
  `cargo test -p engine-ai --lib orders` 9 passed; `cargo test -p engine-ai
  --lib legion` 14 passed, 1 ignored; clippy -D warnings clean.
- [x] UW19 world: spawn rosters — seeded rosters over tagged spawn
  points (min/max per group, archetype, weapon, trap, evidence,
  objective); same seed, same placement. Tier B. Size S. Owns
  `crates/world/src/roster.rs`. Accept: 100 seeds place legally x2; an
  impossible roster refuses naming the group. needs: WD1. Landed 9b6ad79. engine-world roster tests, local run: a_hundred_seeds_place_legally_and_replay_exactly, an_impossible_roster_names_the_group (Short names the group), a_group_that_took_a_scarce_point_gives_it_up and 4 more; clippy -D warnings clean.
- [x] UW24 render: shadowed spot lights - flashlights and lamps in the
  clustered path with a shadow atlas; presentation only. Tier B. Size M.
  Owns `crates/render/src/spot.rs`. Accept: 16 shadowed spots in a dark
  room within the P6 GPU budget (reference GPU); llvmpipe parity with a
  CPU reference; GX6 gate submits live. needs: GX2. Landed 44ffb0e6a (the
  bake) and 0e55a1330 (the live frame and the registry row). spot device
  bake on NVIDIA GeForce RTX 3050 6GB Laptop GPU: submit_ms 0.729
  budget_ms 2.000 shadowed 16 (cold 12.435 ms, samples 0.651/0.729/3.074);
  spot parity on NVIDIA GeForce RTX 3050 6GB Laptop GPU: compared 65536
  mismatched 0 max_diff 0.0000026226044 agreement 1; run 39637725126:
  1267 passed; 0 failed, spot parity on Microsoft Basic Render Driver:
  compared 65536 mismatched 0 max_diff 0.0000025629997 agreement 1;
  clippy -D warnings clean (run 39635848211). Left: nothing samples the
  atlas yet, so the spot row reads PROBE in the pixel-truth table, like
  the sky and decals rows; the GX6 proof-crate row list
  (crates/proof/src/gpu_live.rs) still carries nine rows without "spot".
