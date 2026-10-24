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
- [x] AM31 anim: a 17-role proxy skeleton at the far tier. Bake the proxy and the per-vertex Progress 2026-10-24: `silhouette_px` and `ProxyRig::palette` prove the shipped bake: 43 source joints fold to 15 palette bones (cap 16), worst silhouette 0.355 px at 60 m on 1080p (c113cd2, run 40114857143, 5 tests). Left: 17 roles against the 15 shipped (decide), and the live far tier in anim and render.
  weights once at cook, then skin the far LOD against it instead of all 43 bones. Tier B. Size M.
  Owns `crates/anim/src/lod.rs` (`Tier::Far`, 60 m and the 0.01 screen fraction),
  `crates/anim/src/contract.rs` (the 17 roles), the `crates/assets` cook for the weight merge, and
  `SKIN_BLOCK_JOINTS` in `crates/render/src/present/gpu_present/presenter.rs`.
  Accept: the CT7 hero at `Tier::Far` reports palette 43 -> 17, silhouette error under one pixel at
  60 m; every near-tier pose and the digest unchanged.
  F2P: the far tier still skins 43 bones. Card §1.2. Progress 2026-10-24: lead decision: the 15 shipped roles stand (worst silhouette 0.355 px at 60 m on 1080p, under the 1 px bar), no 17-role bake. Left: the live far tier in anim and render. RETIRED (done): c113cd24a (lead, trim audit 2026-10-24)
- [x] AM32 anim: let motion matching see the world. Add one integer world-geometry penalty per Landed b9a5cf2. `search_in_world` with `WorldQuery` adds one hit cost per blocked horizon; the corridor-arch tape (3 m, 1 m arch, 600 ticks at 1.5 m/s) prints 44 wall-touching ticks before, 0 after; open floor keeps the same picks and `db_hash`; run 40112739225 (24 tests). Opt-in API: the locomotion caller is a later step.
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
- [x] PX47 physfx: small steps in the soft solve. Split one soft-body step into N substeps of one
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
  per-step overhead to vanish. RETIRED (done): 49e5f1566 (lead, trim audit 2026-10-24)
- [x] PX48 physics: clear the CCD fixed-target cache on a teleport. Rapier 0.36 caches CCD fixed Landed 8c79fcc. `World::clear_ccd_targets` runs on restore and `set_body_translation`, the ragdoll's on restore and `part_bone`; 100-seed fast-body and teleport-across-0.2 m-wall tests show zero tunnels, restore replays to the same `state_hash`, `max_ccd_substeps = 0` tunnels (recorded); run 40112904551 (12 tests), golden d57b5c8ae3096614 held.
  targets; call `CCDSolver::invalidate_fixed_targets_cache` after a snapshot restore, a teleport
  or a collider swap, and record that `max_ccd_substeps = 0` disables CCD. Tier B. Size S. Owns
  `crates/physics/src/rapier.rs` (`World::step` and restore), `crates/physics/src/ragdoll.rs`,
  `crates/physfx/src/fx.rs`.
  Accept: teleport a fast body across a 0.2 m wall between ticks over 100 seeds; zero tunnels, and
  the replay after a snapshot restore is identical.
  F2P: a restore is a teleport to the solver and nothing invalidates the cache. Card §3.3.
- [x] MCP31 endpoint: `tx.apply` answers `input_required` instead of refusing. Progress 2026-10-24: `asset.intake` with an unknown licence answers `CallToolResponse::InputRequired` with one `licence` request and `requestState` `asset.intake:licence`, and the retry with `inputResponses` lands the cook (`mcp_input.rs`, 2591e35, run 40116509133, 25 tests under `--features mcp,intake`). Left: the ledger's one call with two rounds (today two audit records), a stdio drive. Return
  `resultType: "input_required"` with `inputRequests` and `requestState`, and land the retried
  call with `inputResponses`; the door emits only `COMPLETE` today (`mcp.rs` matches
  `CallToolResponse::Complete` alone). Tier B. Size M. Owns the result construction in
  `crates/endpoint/src/mcp.rs`, `crates/endpoint/src/packs/asset/intake_verb.rs` (an unknown
  licence is the first ask) and the ledger row in `crates/endpoint/src/audit`.
  Accept: intake an asset with `licence=unknown`, the first result is `input_required` with one
  request, a scripted retry lands it, and the ledger shows one call and two rounds; a compliant
  asset's path is unchanged.
  F2P: no `input_required` exists; the refusal record is `DOOR6`'s. Card §4.1. Landed 69ab1c3be (+ door half 2f8523cc7): the audit ledger folds an ask and its retry into one entry with two rounds (run 40119185677, 33 tests, mcp,intake).
- [x] MCP32 endpoint: remember the workflow, not just the session. Induce a reusable workflow from
  a finished task, key it by task signature in the project folder, and recall it at the start of
  the next similar task. Tier C. Size L. Owns `crates/endpoint/src/packs/session.rs`, a store
  under the project folder, and the `crates/endpoint/src/discover.rs` manifest.
  Accept: ten repeated build tasks, a cold arm against a recall arm, tool calls and characters
  both counted and printed; the recall arm needs no more calls, and a remembered workflow on a
  changed scene re-checks before it applies.
  F2P: the door starts cold every session. Card §4.2. RETIRED (low): workflow induction: research (lead, trim audit 2026-10-24)
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
- [x] AAC7 audio: a measured HRIR sphere behind a flag. Replace the 8-octant synthetic filter with
  about 32 measured directions at 16 taps, interpolated in azimuth and elevation, authored as SOFA.
  Tier B. Size M. Owns `crates/audio/src/hrtf.rs` (`OCTANTS = 8`, `HRTF_TAPS = 16`,
  `MAX_ITD_SAMPLES`) and the dataset licence read.
  Accept: offline, ITD and ILD error at 32 directions against a public SOFA set, and no band more
  than 6 dB off at 8 kHz; state stays under 4 KB; the octant path is unchanged when the flag is
  off. The perceptual claim needs a listening A/B and is not run here.
  F2P: octant panning cannot tell 0 degrees from 40. Card §5.2.
  Decision 2026-10-24 (owner: "you decide for best quality"): keep the 6 dB bar at 8 kHz and
  meet it with a denser measured grid, not a looser bar: take a free dense set (SADIE II or
  CIPIC, both measured on real heads; read the licence), resample to at least 15 degrees in
  azimuth around the pinna-null band, and keep the state under the 4 KB budget by storing only
  the taps the flag path reads.
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
  same held-out protocol (drop every 4th column, midpoint of neighbours): RETIRED (done): 4d20fb3eb (lead, trim audit 2026-10-24)

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
  Progress 2026-10-24 (c): the interpolation changed, not the measurement;
  the 6 dB 8 kHz bar is still not met and the row stays open. The taps now
  blend in the log-magnitude/phase domain (16-point real DFT per ear, 9 bins,
  magnitudes combine as powers so the midpoint is the geometric mean, phases
  turn the short way round, then the taps rebuild), the measured arrivals
  still blend on their own line, and measured directions return the
  measurement byte for byte. Same 108-direction dataset, 16 taps, same
  held-out protocol (drop every 4th column, midpoint of neighbours), and
  **0 bytes added** (`content/audio/` untouched, same licence line):

  | method | ears | err/sig signed rms | worst err/sig | 8 kHz worst | 8 kHz mean | ITD mean | ITD worst | query us |
  |---|---|---|---|---|---|---|---|---|
  | linear taps (was) | 54 | +11.67 dB | -0.12 dB | 18.13 dB | 4.42 dB | 0.96 | 4.00 | 0.61 |
  | log-mag/phase (now) | 54 | +11.07 dB | +1.56 dB | 17.89 dB | 2.02 dB | 0.96 | 4.00 | 2.4-10.7 |

  `err/sig` is `10 log10(err energy / signal energy)`; negative is good. The
  band improved where a linear blend was cancelling: mean 4.42 -> 2.02 dB and
  the worst among every held-out ear except one measured null 13.55 -> 4.94 dB
  (inside the 6 dB bar). What is left is one measurement, not the method: at
  azimuth 80, elevation -40 the ipsilateral ear reads -39.9 dB at 8 kHz
  against neighbours at -15.6 and -25.8 dB, a pinna null 40 dB deep and
  narrower than the 20-degree gap, and its mirrored cell with it; the best
  either neighbour alone can give is 24.3 dB off, so no blend of those two
  reaches it and the worst stays 17.89 dB. Also tried and rejected on these
  numbers: minimum-phase rebuild from the interpolated magnitude (err/sig rms
  3.65 dB, but worst err/sig 6.92 dB crosses the 6 dB bar for the tap rebuild
  and the band reads 18.43 dB), order-2/3/4 spherical-harmonic fits over the
  kept columns (err/sig rms 5.66/6.51/7.28 dB, band worst 31.07/34.00/38.15 dB:
  smoothing cannot hold a one-column null either). Left: a measured elevation
  grid dense enough to see the null (or a pinna model fitted to both ears),
  SOFA export, the ILD error set, the listening A/B.

### Plugin hunt steals (research 2026-10-24, `docs/research/plugins-2026-10-24/05-unreal-deep.md`)

- [x] AM33 anim: the animation budget allocator (Unreal's Animation Budget Allocator, entry 1). One
  per-frame pose budget in milliseconds spent through the existing `lod::Tier` ladder (full, every
  2nd, every 4th, held far pose), least significant first; significance is screen size plus
  nearness plus a fight bonus; the player and the current opponent are pinned to `Tier::Full`;
  a rung holds `hold_frames`, going finer needs `headroom`, a spike past the emergency line skips
  the hold going coarser; the unit cost is learned from the measured pose time. Tier A. Size M.
  Owns `crates/anim/src/anim_budget.rs` and `crates/anim/tests/it/anim_budget.rs`.
  Accept: over budget the least significant drop first, the player never drops, a rung holds N
  frames, under budget everyone returns to full rate, one tape gives the same tiers twice.
  Landed 2026-10-24: `node tools/gh-check.mjs cargo test -p engine-anim --test it anim_budget`
  10 passed (run 40086563415). Left: the wiring, AM34.
- [ ] AM34 anim: wire the allocator into the live pose step. `fighter_look.rs` `pose_frame` is
  `&self` and samples every seen slot each frame; `lod.rs` (AM13) still has no live caller. Build
  one `AnimBudget` in the look, feed it per slot `AnimActor` (camera distance and screen fraction,
  `player` = the local slot, `opponent` = its current target, `fighting` from the act), measure
  `fighters_pose_us` as `measured_ms`, hold the last shown pose through a `LodSlot` on the ticks
  `Tier::updates_this_tick` skips, and print the rung counts on the `arena frame-split:` line.
  Tier A. Size M. Owns `crates/game/src/fighter_look.rs` (pose_frame), `crates/game/src/bin/arena.rs`
  and `crates/game/src/bin/arena_frame/mod.rs` (the split line), needs AM33 and AM13.
  Accept: with a 0.1 ms budget on the 8-fighter arena the player and opponent poses are bit-equal to
  the unbudgeted run, the rest report `Tier::Half` or coarser, the state digest and golden
  `d57b5c8ae3096614` do not move (presentation only); a 1.0 ms budget leaves all eight at `Tier::Full`.
  F2P: no budget reaches `pose_frame`, every slot poses every frame.

### Plugin hunt steals, procedural generation (research 2026-10-24, `docs/research/plugins-2026-10-24/04-github-awesome.md` entry 13)

- [x] MJ1 world: a seeded rewrite-rule grid generator an AI authors as text (MarkovJunior's idea:
  `IN=OUT` rules with rotations and mirrors in `one`, `all`, `prl`, `markov` and `sequence` nodes,
  step caps, a seed; idea only, our own text format, 2D). `crates/world/src/rewrite.rs`
  (`Program::parse`, `Program::run(seed) -> Grid`, `Grid::hash`/`render`; format in the module docs),
  three example rule sets in `content/maps/rewrite/` (`maze_streets.rules`, `insula_block.rules`,
  `ruined_wall.rules`). Bounded: 128 cells a side, 200000 steps, 400M cell comparisons, a typed
  `RunError` instead of a partial grid; a bad file is refused with its line number. Tier B. Size M.
  Owns `crates/world/src/rewrite.rs`, `crates/world/src/rewrite/tests.rs`, `content/maps/rewrite/`.
  Accept: `node tools/gh-check.mjs cargo test -p engine-world --lib rewrite` green: the three
  examples terminate and give byte-identical grids twice, a tiny rule gives an exact grid, 14 bad
  rule files are refused with the right line, the maze hash is pinned. State digest and golden
  `d57b5c8ae3096614` do not move (bake-time only, no sim code).
  Landed 2026-10-24: 18 passed (run 40086984202), clippy `-D warnings` clean (run 40087050820).
  Left: the two consumers, MJ2.
- [x] MJ2 endpoint/world: put the generator behind a `world.rewrite` door and use it for the first
  real map. The door takes the rules text and a seed and returns the grid text, its hash and the
  step count (or the `RulesError` line), exactly as `world.wfc` does for tile maps in
  `crates/endpoint/src/packs/world.rs`; then the UW12/UW20 insula floor plan
  (`docs/plan/urban-war.md` section 6) is authored as a rules file in `content/maps/rewrite/` and
  baked from it instead of being hand-dressed. 3D rules (MarkovJunior's voxel nodes) stay out until
  a multi-storey insula needs a second layer. Tier B. Size S. Owns `crates/endpoint/src/packs/world.rs`
  (the door) and `content/maps/rewrite/` (the insula rules). Needs MJ1.
  Accept: `world.rewrite` with `maze_streets.rules` text and seed 2040 returns hash
  `0xf51cb0aeb99ba512` twice; a rules file with a bad symbol on line 4 comes back as line 4; the
  door refuses a grid over 128 a side before running.
  F2P: an AI can only get a generated block layout by writing Rust.
  Landed de2ba4380: 304 passed in `packs::` (run 40094672500), the six `world.rewrite` tests included.
