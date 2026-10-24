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
  Progress (2026-10-24, 49e5f1566): mechanism landed behind `SoftWorld::with_solve`, shipped
  single-step path unchanged and still the default, because the Accept does not pass. Measured on
  the 4x4 pinned banner (same spring set, six solves per tick): 1x6 rest stretch 1.0517, peak
  1.0522, settle 16; 6x1 rest 1.0516, peak 1.0523, settle 16 (digest 21664091a2809521 both, and
  the same on a second `--fresh` run). Cost 400 ticks of a cape, 3 interleaved reps: mean
  3.44x one step, spread +-0.93, i.e. far over the +10% bar. Card §3.2 named two readings, Small
  Steps and XPBD; the cheaper (substeps, no new solver) was taken. The premise does not hold
  here: Rapier's soft solver never reads `num_solver_iterations` (that knob is the rigid island
  solver), the cloth's real count is `SoftBodyBuilder::additional_pgs_iterations` (default 3) and
  no tier reaches it, and the rest shape is set by the spring compliance (Hz + damping ratio):
  iter 2 rest 1.0515 against iter 6 rest 1.0517, spread 0.023%, so no stiffness is left for a
  tier knob to move. Next: either drop the row or re-aim it at a mesh dense enough for the
  per-step overhead to vanish.
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
- [x] AAC6 audio: find the path around the corner. Search one or two propagation paths around
  geometry and take the best gain, instead of the one straight listener-to-source ray that steps to
  full wall loss. Tier B. Size M. Owns `crates/audio/src/occlude.rs` (`Wall`,
  `MAX_WALLS_PER_QUERY = 256`) and `crates/audio/src/spatial.rs`.
  Accept: a voice 2 m around a 1 m stone corner reads within 3 dB of a 64-ray reference and costs
  no more than `BUDGET_MS_PER_VOICE` 0.1 ms; the curve no longer steps.
  F2P: an open door around the corner still reads the full wall loss. Card §5.1. Landed f60d6a1dd.
  Method: closed-form visible fraction of a `SOURCE_RADIUS_CM` 50 voice disc, from the two boundary
  rays through each wall end (cheaper form of the card's sampled fraction); each wall keeps
  `pass + (1 - pass) * clear`, so a fully hidden wall still reads the exact table loss.
  Numbers: corner 2.16 m behind a 1 m stone L, 64-ray 0.4000 (-7.96 dB) vs query 0.4265 (-7.40 dB),
  +0.56 dB; fade band 8 angles worst 2.58 dB; 64-step walk past the corner worst step 1.09 dB, still
  reaching the stone table loss 0.4; 64-wall query mean 0.0019 ms over 10k (budget 0.1).
- [ ] AAC7 audio: a measured HRIR sphere behind a flag. Replace the 8-octant synthetic filter with
  about 32 measured directions at 16 taps, interpolated in azimuth and elevation, authored as SOFA.
  Tier B. Size M. Owns `crates/audio/src/hrtf.rs` (`OCTANTS = 8`, `HRTF_TAPS = 16`,
  `MAX_ITD_SAMPLES`) and the dataset licence read.
  Accept: offline, ITD and ILD error at 32 directions against a public SOFA set, and no band more
  than 6 dB off at 8 kHz; state stays under 4 KB; the octant path is unchanged when the flag is
  off. The perceptual claim needs a listening A/B and is not run here.
  F2P: octant panning cannot tell 0 degrees from 40. Card §5.2.
  Progress 2026-10-24: measured sphere landed behind `set_measured_sphere`,
  off by default (octant path bit-exact). 48 measured MIT KEMAR directions
  (16 az x 3 elev at 22.5 deg, 16 taps, 44.1 kHz, 3136-byte `.bin` plus a
  const table; licence and subset in `content/audio/PROVENANCE.md`, KEMAR
  page: "no restrictions on use, provided the authors are cited"), ITD from
  the measured arrival. Held-out (every 4th column) rms 6.29 dB, worst
  10.92 dB; ITD error 3.67 samples mean, 12.00 worst; query 0.35 us.
  Still missing: the 6 dB 8 kHz bar (10.75 dB worst held out: needs a denser
  azimuth grid or magnitude/phase interpolation), the SOFA authored form
  (bytes are a plain `.bin`; HDF5 needs a reader this lane has not got), a
  stated ILD error set and the listening A/B.
  Progress 2026-10-24 (b): denser grid tried, bar still not met, row stays
  open. 36 az x 3 elev (10 deg) = 108 directions from the same KEMAR compact
  download and the same licence line; `.bin` 3136 -> 7016 bytes; 16 taps;
  same held-out protocol (drop every 4th column, midpoint of neighbours):

  | grid | ears | err/sig signed rms | worst err/sig | 8 kHz worst | ITD mean | ITD worst |
  |---|---|---|---|---|---|---|
  | 16 az (landed) | 24 | +6.29 dB | +4.16 dB | 10.75 dB | 3.67 | 12.00 |
  | 36 az (now) | 54 | +11.67 dB | -0.12 dB | 18.13 dB | 0.96 | 4.00 |
  | 72 az (trial) | 108 | +17.40 dB | +5.93 dB | 12.77 dB | 0.48 | 4.50 |

  `err/sig` is `10 log10(err energy / signal energy)`; negative is good, so
  the tap rebuild went from louder than the signal to 11.7 dB under it, and
  every held-out ear is now quieter than its measurement. Density does not
  reach the 8 kHz bar: at 8 kHz the pinna response flips phase across a
  10-20 deg neighbour gap, so a linear tap blend cancels a band the measured
  direction carries (and denser grids drop different columns, so the three
  worst values are not one cell). Left: magnitude/phase interpolation with
  the onset kept separate, extra measured elevation rows, SOFA export, an
  ILD error set, the listening A/B.
