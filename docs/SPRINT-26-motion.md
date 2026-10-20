# SPRINT-26 - motion teacher rows (2026-10-20)

Source: `docs/research/finish-line-2026-10-20/04-arxiv-motion.md`.
Order: AM26, then AM27, then AM28. Weapon sockets stay on EG40.
Every row here is off-tick: a teacher bakes bytes, the tick only samples them.

- [ ] AM26 anim: SOMA-to-fighter retarget map - a 77-joint name table
  (52 exact names, 13 renames; `gap.py` GAP PASS) plus bind scale through `BoneMap::build` and
  `retarget_cook`, so a teacher clip can land on the fighter rig at all.
  Tier A. Size M. Owns `crates/assets/src/retarget.rs` plus a SOMA
  fixture in `crates/assets/tests/it/retarget_soma.rs`.
  F2P: `cargo test -p engine-assets --lib retarget_soma` fails before
  (no fixture, no map) and passes after.
  P2P: `cargo test -p engine-assets --lib` stays green.
  Accept: `node tools/gh-check.mjs cargo test -p engine-assets --lib
  retarget_soma` x2 - the fixed SOMA fixture cooks twice to the same
  `retarget_digest`, every mapped joint is within 2 cm of the authored
  fighter target, and the cooked foot contacts equal the fixture's 8
  windows.
  Progress 2026-10-20: the name table is measured with `gap.py` (52 exact,
  13 renames, 12 unused; the traps are `Spine1`, `Spine2`, `LeftLeg`,
  `RightLeg`). The Rust map and its fixture are not written yet.
  (RES-ARXIV 2026-10-20, motion teacher hunt.)

- [ ] AM27 anim: offline motion bake from a teacher to `.am5p` - a tool
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
