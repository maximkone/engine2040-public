### Smart steals (research 2026-10-23, `docs/research/engine-smarts-2026-10-23/README.md`)

Boarded 2026-10-24 from that card: ten steals with a cheap first experiment each, lifted from
papers, the Rapier changelog and the MCP 2026-07-28 spec. No donor code is taken; every row is
idea-only, and each names the file we already hold. The card's CA1 (tell floor) is not re-filed
here: `OPT59` in `SPRINT-18-gaps.md` is the same row.

- [ ] AM30 anim: dual-quaternion skin palette behind a flag. `skinning_matrices` already builds
  `world_pos` and `world_rot` before expanding to mat4, so the 8-float dual quaternion is the
  cheaper output: palette 2,752 B -> 1,376 B for 43 joints, and the LF5 previous-palette copy
  halves with it. Tier B. Size M. Owns `crates/anim/src/contract.rs`, `crates/anim/src/an1.rs`,
  `crates/render/src/shaders/forward.wgsl`, the palette buffers in
  `crates/render/src/present/gpu_present/presenter.rs` and `.../motion.rs`.
  Accept: 32 bytes per joint on a 43-bone rig, max vertex error under 1 mm against the matrix path
  on normal poses, the candy-wrapper metric below the LBS value on a 170-degree shoulder twist;
  off by default, every near-tier pose and the state digest unchanged when off.
  F2P: the palette is 64 B per joint and LBS collapses the twist. Card §1.1.
- [ ] AM31 anim: a 17-role proxy skeleton at the far tier. Bake the proxy and the per-vertex
  weights once at cook, then skin the far LOD against it instead of all 43 bones. Tier B. Size M.
  Owns `crates/anim/src/lod.rs` (`Tier::Far`, 60 m and the 0.01 screen fraction),
  `crates/anim/src/contract.rs` (the 17 roles), the `crates/assets` cook for the weight merge, and
  `SKIN_BLOCK_JOINTS` in `crates/render/src/present/gpu_present/presenter.rs`.
  Accept: the CT7 hero at `Tier::Far` reports palette 43 -> 17, silhouette error under one pixel at
  60 m; every near-tier pose and the digest unchanged.
  F2P: the far tier still skins 43 bones. Card §1.2.
- [ ] AM32 anim: let motion matching see the world. Add one integer world-geometry penalty per
  horizon to the match cost, so a clip whose future trajectory would hit geometry loses to the
  side-step. `SearchQuery` already carries `jump_cost`, `contact_cost`, `cost_units` and
  `TRAJ_N = 3`. Tier B. Size M. Owns `crates/anim/src/motion_match.rs` and its caller
  `crates/anim/src/locomotion.rs`.
  Accept: a 3 m corridor with a 1 m arch, 600 ticks at 1.5 m/s, counts zero ticks whose capsule
  path intersects a wall (before: non-zero, printed); the open-floor fixture picks the same clip
  and the digest does not move.
  F2P: the cost cannot see a wall. Card §1.3.
- [ ] PX46 physics: Rapier joint motors replace the hand servo. Configure the 15-bone rig's joints
  as PD motors (`motor_position`, `motor_model`, `motor_max_force`) with
  `set_additional_pgs_iterations` on the bone bodies, instead of the velocity servo in
  `crates/physics/src/pd.rs` plus per-tick target writes. Tier B. Size M. Owns
  `crates/physics/src/ragdoll.rs` (joint build), `crates/physics/src/pd.rs`,
  `crates/physics/src/rapier.rs` (`num_solver_iterations`).
  Accept: 600-tick powered stand plus one spear thrust; wrist and elbow open no more than the
  shipped 1 cm margin at four solver passes (today: 2.6 cm at 4, 1.2 cm at 8, 0.8 cm at 12,
  `rapier.rs:53-58`), pose error to target under 2 cm, digest unchanged.
  F2P: four passes open the wrist 2.6 cm. Card §3.1.
- [ ] PX47 physfx: small steps in the soft solve. Split one soft-body step into N substeps of one
  solver iteration each at the same constraint count, so cloth stiffness stops moving when a tier
  changes `solver_iterations` (2/3/4/6 today). Tier B. Size M. Owns
  `crates/physfx/src/fx/cloth.rs`, `crates/physfx/src/soft.rs`, `crates/physfx/src/caps.rs`.
  Accept: one pinned cloth at a fixed constraint total, (1 step x 6 iterations) against (6
  substeps x 1), lower longest-edge stretch and fewer ticks to settle, digest twice identical,
  cost within +10%.
  F2P: stiffness follows the iteration count. Card §3.2.
- [ ] PX48 physics: clear the CCD fixed-target cache on a teleport. Rapier 0.36 caches CCD fixed
  targets; call `CCDSolver::invalidate_fixed_targets_cache` after a snapshot restore, a teleport
  or a collider swap, and record that `max_ccd_substeps = 0` disables CCD. Tier B. Size S. Owns
  `crates/physics/src/rapier.rs` (`World::step` and restore), `crates/physics/src/ragdoll.rs`,
  `crates/physfx/src/fx.rs`.
  Accept: teleport a fast body across a 0.2 m wall between ticks over 100 seeds; zero tunnels, and
  the replay after a snapshot restore is identical.
  F2P: a restore is a teleport to the solver and nothing invalidates the cache. Card §3.3.
- [ ] MCP31 endpoint: `tx.apply` answers `input_required` instead of refusing. Return
  `resultType: "input_required"` with `inputRequests` and `requestState`, and land the retried
  call with `inputResponses`; the door emits only `COMPLETE` today (`mcp.rs` matches
  `CallToolResponse::Complete` alone). Tier B. Size M. Owns the result construction in
  `crates/endpoint/src/mcp.rs`, `crates/endpoint/src/packs/asset/intake_verb.rs` (an unknown
  licence is the first ask) and the ledger row in `crates/endpoint/src/audit`.
  Accept: intake an asset with `licence=unknown`, the first result is `input_required` with one
  request, a scripted retry lands it, and the ledger shows one call and two rounds; a compliant
  asset's path is unchanged.
  F2P: no `input_required` exists; the refusal record is `DOOR6`'s. Card §4.1.
- [ ] MCP32 endpoint: remember the workflow, not just the session. Induce a reusable workflow from
  a finished task, key it by task signature in the project folder, and recall it at the start of
  the next similar task. Tier C. Size L. Owns `crates/endpoint/src/packs/session.rs`, a store
  under the project folder, and the `crates/endpoint/src/discover.rs` manifest.
  Accept: ten repeated build tasks, a cold arm against a recall arm, tool calls and characters
  both counted and printed; the recall arm needs no more calls, and a remembered workflow on a
  changed scene re-checks before it applies.
  F2P: the door starts cold every session. Card §4.2.
- [ ] AAC6 audio: find the path around the corner. Search one or two propagation paths around
  geometry and take the best gain, instead of the one straight listener-to-source ray that steps to
  full wall loss. Tier B. Size M. Owns `crates/audio/src/occlude.rs` (`Wall`,
  `MAX_WALLS_PER_QUERY = 256`) and `crates/audio/src/spatial.rs`.
  Accept: a voice 2 m around a 1 m stone corner reads within 3 dB of a 64-ray reference and costs
  no more than `BUDGET_MS_PER_VOICE` 0.1 ms; the curve no longer steps.
  F2P: an open door around the corner still reads the full wall loss. Card §5.1.
- [ ] AAC7 audio: a measured HRIR sphere behind a flag. Replace the 8-octant synthetic filter with
  about 32 measured directions at 16 taps, interpolated in azimuth and elevation, authored as SOFA.
  Tier B. Size M. Owns `crates/audio/src/hrtf.rs` (`OCTANTS = 8`, `HRTF_TAPS = 16`,
  `MAX_ITD_SAMPLES`) and the dataset licence read.
  Accept: offline, ITD and ILD error at 32 directions against a public SOFA set, and no band more
  than 6 dB off at 8 kHz; state stays under 4 KB; the octant path is unchanged when the flag is
  off. The perceptual claim needs a listening A/B and is not run here.
  F2P: octant panning cannot tell 0 degrees from 40. Card §5.2.
