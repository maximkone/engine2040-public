# SPRINT-26 - motion teacher rows (2026-10-20)

Source: `docs/research/finish-line-2026-10-20/04-arxiv-motion.md`.
Order: AM26, then AM27, then AM28. Weapon sockets stay on EG40.
Every row here is off-tick: a teacher bakes bytes, the tick only samples them.

- [x] AM26 anim: SOMA-to-fighter retarget map - a 77-joint name table
  (52 exact names, 13 renames; `gap.py` GAP PASS) plus bind scale through `BoneMap::build` and
  `retarget_cook`, so a teacher clip can land on the fighter rig at all.
  Tier A. Size M. Owns `crates/assets/src/soma.rs` plus a SOMA
  fixture in `crates/assets/tests/it/retarget_soma.rs`.
  F2P: `node tools/gh-check.mjs cargo test -p engine-assets --features import
  --test it retarget_soma` fails before (no fixture, no map) and passes after.
  P2P: `cargo test -p engine-assets --lib` stays green.
  Accept: the same `--features import --test it` command x2 - the fixed SOMA
  fixture cooks twice to the same `retarget_digest`, every mapped joint is
  within 2 cm of the authored fighter target, and the cooked foot contacts
  equal the fixture's 8 windows.
  Progress 2026-10-20: Landed `cf337d1e`. The name table is measured with
  `gap.py` (52 exact, 13 renames, 12 unused; the traps are `Spine1`,
  `Spine2`, `LeftLeg`, `RightLeg`). The Rust map is `crates/assets/src/soma.rs`
  (`SOMA77`, `RENAMES`, `UNUSED_SOMA`, `soma_pairs`, `coverage`) and the
  fixture is `crates/assets/tests/it/retarget_soma.rs` (77-joint SOMA source,
  65-joint fighter target, bind scale 0.5 on the feet, cook x2 to one digest,
  every mapped joint within 2 cm, 8 contact windows, `MissingBone` path).
  Accept green: `node tools/gh-check.mjs cargo test -p engine-assets
  --features import --test it retarget_soma` 3 passed x2 (gh runs 39468697558,
  39469422404); `cargo test -p engine-assets --lib` 201 passed 2 ignored (gh run
  39469996998; a first run hit the `residency_work_never_hitches_over_two_ms`
  wall-clock flake). Command correction: the fixture is a module of the `it`
  binary, so the test target is `it`, not `--lib`, and it needs
  `--features import`; without the feature
  the filter matches 0 tests and proves nothing.
  (RES-ARXIV 2026-10-20, motion teacher hunt.)

- [x] AM27 anim: offline motion bake from a teacher to `.am5p` - a tool
  reads a Kimodo NPZ or a soma BVH, retargets, and packs; the PartiGen
  endpoint rule picks the last frame instead of a fixed duration.
  Tier A. Size M. Owns `tools/motion_bake/` plus
  `crates/anim/tests/it/motion_bake.rs`. needs: AM26.
  F2P: `cargo test -p engine-anim --lib motion_bake` fails before (no
  tool, no fixture) and passes after.
  P2P: `cargo test -p engine-anim --lib` stays green and the golden tape
  hash `d57b5c8ae3096614` does not move.
  Accept: `node tools/gh-check.mjs cargo test -p engine-anim --lib
  motion_bake` x2 - `fixtures/clips/stagger.am5p` loads, `avg_slip` is
  under 0.08, `unpack_qclip` round-trips, and `qclip_hash` equals the
  seed-7 receipt printed by two tool runs.
  Progress 2026-10-20: built; accept green on the uncommitted tree.
  Pure half: `crates/anim/src/motion_bake.rs` (`bake_len`,
  `last_constrained`, `bake_pack`, `bake_unpack`) over the one AM5P home
  in `pack.rs`; no twin format, and the pack proves its own round trip
  (`qclip_hash` before and after). Tool: `tools/motion_bake/` (Rust, its
  own workspace, never a tick crate; `synth` writes the seed-7 SOMA-77
  stagger in Kimodo's NPZ layout, `bake` reads it, retargets through
  `soma_pairs()`/`BoneMap`/`retarget_cook`, takes the length from the
  last foot-plant tick and packs the two foot lanes). Fixtures:
  `fixtures/clips/stagger.npz` (36 frames, 30 fps, sha256
  `2bc933c4175af98d2a39303b93ef97809ee8b04361d059c91321daf857d01fa1`),
  `stagger.am5p` (820 bytes, sha256
  `f255f453a46d47f2327aead292b31197a7d179af714cd169243a3ba2c9e75d4e`)
  and `stagger.receipt.json`: qclip_hash `0x34f72e4f8b72217a`, ticks 61,
  dur_s 1.000000, tracks leftfoot+rightfoot, avg_slip 0.004975 (left,
  planted) and 0.102805 (right, the stepping foot), plant windows 2+3,
  retarget_digest `0x592af8cffb61ff84`. The accept's `avg_slip` line is
  read on the planted foot (`avg_slip(&clip, 0, ticks)`): 0.004975 under
  0.08; the stepping foot averages 0.102805 m/s because it moves.
  Endpoint rule: the teacher's last pinned frame is tick 60 of 70, so the
  10-tick tail is cut (61 ticks, not 71). Proof (all
  `node tools/gh-check.mjs` on the working tree):
  `cargo test -p engine-anim --lib motion_bake` 5 passed x2 (gh runs
  39483045124, 39483385656); `cargo test -p engine-anim --lib` 456 passed
  4 ignored (39483936681); `cargo test -p engine-anim --test it` 11
  passed (39484514694; the `anim_lod_pin` module moved into that binary);
  `cargo test -p engine-proof --lib golden_hash_untouched` 1 passed
  (39485454358) - the golden tape `d57b5c8ae3096614` did not move. Landed
  `31f2b7d6`; the first cut's `f32::from(u32)` does not compile, so
  `1f19e70a` converts the last key through u16 (MAX_BAKE_TICKS 3600) and
  keeps casts at 109: final-tree `--lib motion_bake` 5 passed (39486870916)
  and `--lib` 462 passed 4 ignored (39487933810). Before:
  0 tests matched (run 39471594265). Target note: for this row `--lib`
  `motion_bake` is real (5 lib tests, and the committed fixture rides
  `include_bytes!` so the fixture gate runs in the lib); the receipt-file
  checks live in the `it` binary (`--test it motion_bake`, 3 passed), so
  both lines are named. Left: the SOMA BVH input (somaskel77, cm root) as
  the second format; the NPZ reader already handles `savez_compressed` and
  the `(1,N,77,3,3)` one-batch shape.
  (RES-ARXIV 2026-10-20, motion teacher hunt.)

- [ ] AM28 anim: distilled trajectory student in the tick - train the
  `TrajNet` teacher offline, export `.e2nn` through `distill`, call
  `infer_into`; the default stays `predict_linear` so the golden does not
  move. Tier A. Size L. Owns `crates/anim/src/motion_match.rs`,
  `crates/distill/src/export.rs`, `fixtures/nets/traj.e2nn`.
  needs: AM27.
  F2P: `cargo test -p engine-anim --lib traj_student` fails before (no
  net, no fixture) and passes after.
  P2P: `cargo test -p engine-anim --lib` stays green; the two-fighter
  arena sim hash is unchanged with no net.
  Accept: `node tools/gh-check.mjs cargo test -p engine-anim --lib
  traj_student` x2 - mean endpoint error at least 20% under
  `predict_linear` on the held-out fixture, `infer_into` allocates
  nothing, and the net-on tape repeats its `arena sim:` hash.
  Correction 2026-10-21: the Accept filter `traj_student` matches no test (`Grep traj` over crates/anim is empty) and `fixtures/nets/traj.e2nn` does not exist (`Test-Path` false; only `depth_student.e2nn` is in that folder), so the command exits 0 with 0 passed. The net, the fixture and the test are the work; Accept is `--lib traj_student` printing a nonzero passed count, and the row may not be ticked until it does.

- [ ] AM29 anim: a baked teacher clip is playable in the tick - the pack
  carries the whole fighter body, the engine loads an `.am5p` from disk and
  samples it onto the fighter rig. AM27 bakes and proves the wire, but
  nothing played it: the bake wrote only the two foot tracks (the gate's
  evidence), the pack carried no track names, `sample_packed` had no caller
  outside `pack.rs`, there was no `.am5p` file loader, and the fighter-65
  skeleton the bake retargets onto lived in `tools/motion_bake/src/rig.rs`
  instead of in the engine. So a teacher clip could be made and checked, and
  the game could not play it.
  Tier A. Size L. Owns `tools/motion_bake/`, the fighter skeleton in the
  engine, a pack loader in `crates/anim`, and the runtime hook that plays a
  loaded clip.
  needs: AM27.
  F2P: `node tools/gh-check.mjs cargo test -p engine-anim --lib
  pack_file` fails before (no loader, no fixture) and passes after.
  P2P: `node tools/gh-check.mjs cargo test -p engine-anim --lib` stays green;
  the golden tape hash `d57b5c8ae3096614` does not move.
  Accept: the same command x2 - the committed `fixtures/clips/stagger.am5p`
  loads from disk, it carries every fighter joint, a sampled pose holds the
  rig's own stance, a pack baked for another rig refuses, and the feet slip
  stays under 0.08 like the bake gate reads it.
  Progress 2026-10-21: the loader landed (`crates/anim/src/pack_file.rs`: a
  path in, a clip out, the wire checked twice, so a cut or edited file
  refuses instead of playing wrong motion) and the rig moved into the engine
  (`crates/assets/src/fighter.rs`: the SOMA rest pose, the teacher skeleton
  and the fighter-65 skeleton with its bind; the bake tool imports it and
  keeps only its math). The bake is unchanged by the move: the tool's own
  test still reproduces the committed fixture byte for byte. Then the pack
  grew the whole body: the bake writes one track per fighter joint (65,
  named in the receipt, in the rig's joint order) instead of the two feet,
  and `pack_file::sample_pose` plays a loaded pack on a rig of the same
  joint count, refusing a pack baked for another rig. Then
  `pack_file::sample_pose_named` plays the pack on a rig whose joints carry
  the same names in another order: the caller names the pack's tracks, each
  rig joint takes the track with its name, and a rig of another size, a name
  list of another size, or a joint with no track refuses loud (unit tests in
  `pack_file`). Left in the row: the pack carries positions only, so the
  rig's bind rotations stand, and no game-side caller plays a loaded clip yet.
