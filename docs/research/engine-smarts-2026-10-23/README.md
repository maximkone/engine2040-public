# Engine smarts, wave 2026-10-23

Scope: five areas of "smartness" a small Rust engine can take from the 2024-2026
literature and from shipped games: animation and locomotion, combat AI, physics,
the agent door, and audio. The question behind the card: what is left to steal
that is bigger than a code clean-up and cheaper than a training run?

Number labels follow the house card: **measured** = read in a file or a page;
**donor claim** = the source's own sentence, no number of ours; **our target** =
the bar a row must meet; **arithmetic** = plain multiplication of measured
numbers, not a benchmark. Costs marked *(estimate)* are my judgement, not a
measurement.

## The one-line answer

The gap is not another planner and not another net. Four places are cheap and
real: the skin palette still pays 64 bytes per joint where a dual quaternion
needs 32 (43 bones, measured), the far LOD still skins all 43 bones though the
rig contract already names 17 required roles (measured), the duel's readable
startup is 15 ticks = 250 ms while the only measured 2025 readability floor is
500 ms (measured both sides), and the door never answers `input_required`, so a
verb that needs one human choice fails the call instead of asking (measured in
`crates/endpoint/src`). Physics and audio are one swap each: Rapier's own joint
motors instead of our velocity servo, and small-steps XPBD in the soft solve.

## What we already have, so nobody steals it twice

| thing | where | source | status |
|---|---|---|---|
| behaviour tree, GOAP, utility, goalstack | `crates/ai/src/{bt,goap,utility,eqs,goalstack}.rs` | Killzone 2, F.E.A.R., IAUS | shipped shapes |
| product-of-considerations with a 0 early-out and response curves | `utility.rs` (`product_of`, `response_curve`) | IAUS, GW2 | **already there** |
| readable enemy tiers and attack anatomy | `crates/ai/src/{tier,depth}.rs`, `crates/combat/src/{anatomy,design}.rs` | house rows | shipped shapes, one number wrong (below) |
| structured tool output and `outputSchema` | `crates/endpoint/src/mcp.rs:298`, `play.rs:1449`, `tool_macro.rs` | MCP tools spec | **already there** |
| Rapier 0.36 with `enhanced-determinism` | `Cargo.toml:198` | Rapier | pinned |
| XPBD-shaped cloth via `SoftBodySet` | `crates/physfx/src/soft.rs`, `fx/cloth.rs` | Rapier | shipped, single-step |

So areas 2 and 4 are not architecture problems. They are one timer and one
protocol round trip.

## 1. Animation and locomotion

What ships today in the industry: motion matching left the GDC talk
(Clavet 2016) and became a production search, with the 2020 learned variant
compressing the database and the 2025 variant adding world geometry to the cost
(all three below). The 43-bone hero rig is the constraint: 43 skinned bones,
45 engine joints, a 15-bone physical rig, and `REQUIRED_BONE_COUNT = 17` roles
(measured, `crates/anim/src/an1.rs:186`, `contract.rs:444`,
`content/bodies/fighter.toml`). Every steal here fits that rig or it is not in.

### 1.1 AM30: a dual-quaternion skin palette

- **What:** skin each joint with a unit dual quaternion (8 floats) and blend the
  joint influences with dual-quaternion linear blending instead of a 4x4 matrix
  (12 used floats) and linear blend skinning.
- **URL:** https://doi.org/10.1145/1409625.1409627 (Kavan et al., *Geometric
  Skinning*, TOG 2008); PDF https://users.cs.utah.edu/~ladislav/kavan08geometric/kavan08geometric.pdf
- **Licence:** paper only, idea-only; no code is taken.
- **Our files:** `crates/anim/src/contract.rs` (`skinning_matrices` already
  builds `world_pos` and `world_rot` before expanding to mat4, so the DQ is the
  cheaper output), `crates/anim/src/an1.rs` (`an1_skinning_matrices`),
  `crates/render/src/present/gpu_present/presenter.rs` (palette buffers and the
  LF5 previous-palette copy), `.../motion.rs` (previous palette bytes),
  `crates/render/src/shaders/forward.wgsl` (`skin_joints`,
  `prev_skin_joints`, `vs_skinned`).
- **Cost:** 1-2 days *(estimate)*.
- **What it replaces:** the 64-byte matrix palette entry and the LBS vertex
  path. Palette for 43 bones: 43 x 64 = 2,752 B today, 43 x 32 = 1,376 B after
  (arithmetic); the LF5 previous copy halves with it.
- **Cheap experiment:** one 170-degree shoulder twist pair on the 43-bone rig.
  Assert 32 bytes per joint, max vertex error under 1 mm against the matrix path
  on normal poses, and the candy-wrapper collapse metric below the LBS value.
  Kill: any normal pose over 2 mm, or a state digest that moves.
- **Rig need:** rigid transforms only. `LocalPose` carries position plus
  quaternion and no scale (measured), so DQS loses nothing today. A future
  scale lane would break this, and that is the one thing to write down.
- **Donor claim to weigh:** 8 floats against 12, "slightly slower" than LBS,
  more than 2x faster than log-matrix blending; test model 5,002 vertices,
  54 joints. So the win is bytes and artifacts, not speed.

### 1.2 AM31: a reduced skeleton at the far tier

- **What:** bake a 17-role proxy skeleton and per-vertex weights once, then skin
  the far LOD against it instead of all 43 bones.
- **URL:** https://doi.org/10.1002/cav.181 (*Progressive skinning for character
  animation*, 2007); crowd batching shape:
  https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-2-animated-crowd-rendering
- **Licence:** papers, idea-only. Simplygon's Bone Reducer is paid
  (https://www.simplygon.com/features/bonereduction), so idea-only as well.
- **Our files:** `crates/anim/src/lod.rs` (`Tier::Far`, `tier_for`, 60 m and
  0.01 screen fraction), `crates/anim/src/contract.rs` (the 17 roles),
  the `crates/assets` cook for the weight merge, `forward.wgsl` plus
  `presenter.rs` (`SKIN_BLOCK_JOINTS`).
- **Cost:** 2-3 days *(estimate)* plus one cook pass.
- **What it replaces:** a full 43-joint skin at the far tier, where the fighter
  is already past 60 m or under the 0.01 screen fraction.
- **Cheap experiment:** bake the proxy for the CT7 hero, render at `Tier::Far`,
  assert palette 43 -> 17 and silhouette error under one pixel at 60 m. Kill:
  any near-tier pose changes, or the silhouette is over a pixel.

### 1.3 AM32: let motion matching see the world

- **What:** add a world-geometry term to the motion-match cost. The 2025
  environment-aware search penalises clips whose future trajectory would hit
  scene geometry, so the chosen clip turns or side-steps instead of clipping an
  arch.
- **URL:** https://arxiv.org/abs/2510.22632 (*Environment-aware Motion
  Matching*, ACM TOG / SIGGRAPH Asia 2025).
- **Licence:** paper, idea-only.
- **Our files:** `crates/anim/src/motion_match.rs` (`SearchQuery` already
  carries `jump_cost`, `contact_cost`, `cost_units`, three horizons
  `TRAJ_N = 3`), plus its caller in `crates/anim/src/locomotion.rs`.
- **Cost:** 2-4 days *(estimate)*.
- **What it replaces:** a cost that only knows pose, trajectory and clip
  continuity; the search cannot see a wall today (measured: no obstacle term).
- **Cheap experiment:** a 3 m corridor with a 1 m arch, 600 ticks at 1.5 m/s.
  Count ticks whose capsule path intersects a wall, before and after. Kill: the
  open-floor fixture picks a different clip, or the digest moves.
- **Why it fits:** the three horizons are already in the feature, so the paper's
  per-horizon collision test is one integer penalty, not a new query layer.

### 1.4 Not stolen here: learning to walk

"Learning to Walk in Minutes" (https://arxiv.org/abs/2109.11978, code
https://github.com/leggedrobotics/legged_gym, BSD-3-Clause read at the repo) is
a 4096-robot GPU recipe: flat terrain under 4 min, uneven under 20 min, on an
A6000 (donor claim). MimicKit (https://arxiv.org/abs/2510.13794, Apache-2.0) is
the DeepMimic successor. Both are already the subject of the PX5 card, and the
minutes are training minutes on a GPU we do not have. The CPG route in
`crates/physics/src/gait.rs` stays the cheap one.

## 2. Combat AI

Shipped practice is settled: Killzone 2 runs a hierarchically layered HTN at
5 Hz with a "continue current plan" branch
(https://www.guerrilla-games.com/read/killzone-2-multiplayer-bots); F.E.A.R.
wraps GOAP in a three-state FSM
(https://alumni.media.mit.edu/~jorkin/gdc2006_orkin_jeff_fear.pdf); utility
scores multiply consideration curves with a zero early-out
(https://gameai.com/iaus.php, and the GW2 Heart of Thorns detail in
https://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter13_Choosing_Effective_Utility-Based_Considerations.pdf).
We have all four shapes on disk, and `legion.rs` already re-decides on input
change instead of every tick, which is the Killzone twitch fix. Nothing to port.

The one measurable gap is a timer.

### 2.1 CA1: the readable startup floor

- **What:** raise the duel's tell from 15 ticks to at least 30 ticks (250 ms to
  500 ms) for tiers 1-3, and keep the counter answer inside that window.
- **URL:** http://theseus.fi/handle/10024/900621 (2025 thesis on enemy attack
  readability: heuristic window >= 500 ms, median measured 800 ms, IQR 525-900,
  n = 30).
- **Licence:** thesis, idea-only.
- **Our files:** `crates/ai/src/depth.rs` (`TierRow.tell_ticks`, default rows
  at 15), `crates/combat/src/design.rs` (`startup_ticks: 15`, accepted range
  12..=60), any designer rows under `content/`.
- **Cost:** hours *(estimate)* plus a playtest.
- **What it replaces:** the 250 ms tell, which is half the only measured floor
  in the literature.
- **Cheap experiment:** two tier files (15 vs 30) through the same six seeded
  fights; count parries that land and hits taken; the same seed twice must be
  identical. Kill: no movement in either direction over six seeds.
- **Warning:** this changes fight timing, so it moves the golden hash. It needs
  an explicit hash receipt (seed 2040, 600 ticks), not a quiet data edit.
- **Related, not a steal:** the Dark Souls 3 five-phase attack anatomy
  (https://www.gamedeveloper.com/game-platforms/anatomy-of-an-enemy-attack-in-dark-souls-3)
  is already what `crates/combat/src/anatomy.rs` models.

## 3. Physics

Our physics is Rapier 0.36 with `enhanced-determinism`, f32, no SIMD, no
parallel (`Cargo.toml:198`, `crates/physics/src/rapier.rs:1-16`). PhysX 5 is not
a candidate: its interesting new work (FEM soft bodies, cloth, inflatables, SDF
collision) runs **only on the GPU**, while the CPU source is BSD-3 and
multi-threaded with SIMD
(https://nvidia-omniverse.github.io/PhysX/physx/latest/). A GPU-only soft body
cannot sit in our fixed tick, so the verdict is: stay on Rapier, take the three
0.36 features we already pay for but do not use.

### 3.1 PX46: use Rapier's joint motors, not our own servo

- **What:** drive the 15-bone physical rig with Rapier joint motors configured
  as PD (`motor_position`, `motor_model`, `motor_max_force`) and give the bone
  bodies extra contact iterations (`RigidBody::set_additional_pgs_iterations`),
  instead of the hand-written velocity servo plus per-tick target writes.
- **URL:** https://rapier.rs/docs/user_guides/rust/joints and the 0.34-0.36
  changelog https://github.com/dimforge/rapier/blob/master/CHANGELOG.md (motors
  lost `VelocityBased`; now `AccelerationBased` and `ForceBased`).
- **Licence:** Apache-2.0, an existing dependency.
- **Our files:** `crates/physics/src/ragdoll.rs` (joint build; lines 1211-1224
  set only a friction motor with `motor_velocity`, never a model),
  `crates/physics/src/pd.rs` (our servo), `crates/physics/src/rapier.rs`
  (`num_solver_iterations: self.solver_passes`).
- **Cost:** 2-3 days *(estimate)*.
- **What it replaces:** two mechanisms with one. Today the pose is a hand servo
  and the joint friction is a Rapier motor; after, both are motors with a model
  and a force cap.
- **Cheap experiment:** 600-tick powered stand plus one spear thrust; assert the
  wrist and elbow open no more than the shipped 1 cm margin at four solver
  passes, and pose error to target under 2 cm. Kill: any joint opens past the
  margin or the digest moves.
- **Measured baseline in our tree:** joint opening 2.6 cm at 4 passes, 1.2 cm at
  8, 0.8 cm at 12 (`rapier.rs:53-58`). Those numbers are the bar, not a guess.

### 3.2 PX47: small steps in the soft solve

- **What:** split one soft-body step into N substeps of one solver iteration
  each, at the same constraint count, so stiffness stops depending on the
  iteration count.
- **URL:** https://mmacklin.com/smallsteps.pdf (*Small Steps in Physics
  Simulation*, 2019) and http://mmacklin.com/xpbd.pdf (*XPBD*, 2016).
- **Licence:** papers, idea-only.
- **Our files:** `crates/physfx/src/fx/cloth.rs`, `crates/physfx/src/soft.rs`,
  `crates/physfx/src/caps.rs` (tier `solver_iterations` 2/3/4/6),
  `crates/physfx/src/fx.rs` (`max_ccd_substeps`).
- **Cost:** 1-2 days *(estimate)*.
- **What it replaces:** one step x N iterations, which is why the cloth's
  stiffness moves when a tier changes.
- **Cheap experiment:** one pinned cloth, fixed constraint total, (1 step x 6
  iterations) against (6 substeps x 1), assert lower longest-edge stretch,
  fewer ticks to settle, digest x2 identical. Kill: cost over +10%, or a lane
  leaves the pinned row.
- **Donor claims:** XPBD adds under 2% per step; for a 64x64 cloth, 0.95 ms at
  20 iterations to 5.65 ms at 160. Small Steps: 150k particles, 30 iterations
  12.4 ms against 30 substeps x 1 iteration 13.5 ms, with less stretch.

### 3.3 PX48: clear the CCD fixed-target cache on a teleport

- **What:** Rapier 0.36 rewrote CCD as a sweep-based time of impact and caches
  fixed targets; call `CCDSolver::invalidate_fixed_targets_cache` after a
  teleport, a snapshot restore or a collider swap. Also record that
  `max_ccd_substeps = 0` disables CCD.
- **URL:** https://github.com/dimforge/rapier/blob/master/CHANGELOG.md (v0.36.0,
  24 Sep 2026, and the CCD rewrite notes).
- **Licence:** Apache-2.0, an existing dependency.
- **Our files:** `crates/physics/src/rapier.rs` (`World::step`, restore),
  `crates/physics/src/ragdoll.rs`, `crates/physfx/src/fx.rs:163`.
- **Cost:** hours *(estimate)*.
- **What it replaces:** nothing; it closes a hole the new cache opens. Rollback
  restores positions directly, which is a teleport from the solver's view.
- **Cheap experiment:** teleport a fast body across a 0.2 m wall between ticks,
  100 seeds; assert zero tunnels and an identical replay after a snapshot
  restore. Kill: any tunnel, or a restore that differs.

## 4. The door and the agent

The door already serves the 2026-07-28 spec (`discover.rs:202`) and already
returns `structuredContent` with `outputSchema` (`mcp.rs:298`, `play.rs:1449`).
So the two classic "make the tool surface agent-friendly" steals are done. What
is missing is the round trip and the memory.

### 4.1 MCP29: let a verb ask one question

- **What:** a verb that needs one human choice returns `resultType:
  "input_required"` with `inputRequests` and `requestState`; the client retries
  the same call with `inputResponses`. Our door only ever emits `COMPLETE`
  (measured: `mcp.rs:1239-1247` matches only `CallToolResponse::Complete`, and
  `mcp.rs:1500` is the only explicit `result_type`, on the tool list).
- **URL:** https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr.md
  and https://modelcontextprotocol.io/specification/2026-07-28/changelog.md
- **Licence:** spec text, implementation ours.
- **Our files:** `crates/endpoint/src/mcp.rs` (result construction),
  `crates/endpoint/src/packs/asset/intake_verb.rs` (an unknown licence is the
  natural first ask), `crates/endpoint/src/audit` (two rounds must stay one
  ledger row).
- **Cost:** 1-2 days *(estimate)*.
- **What it replaces:** refuse, then the user re-types the whole call. The 2025
  form-style elicitation is flat-object only and must not ask sensitive data
  (https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation);
  MRTR replaced server-initiated requests in 2026-07-28, so it is the shape to
  build, not elicitation.
- **Cheap experiment:** intake an asset with `licence=unknown`; assert the first
  result is `input_required` with one request, a scripted retry lands it, and
  the ledger shows one call and two rounds. Kill: a second ledger row, or a
  compliant asset's path changes.

### 4.2 MCP30: remember the workflow, not just the session

- **What:** induce a reusable workflow from a finished task, key it by task
  signature, and recall it at the start of the next similar task instead of
  rediscovering the tool order.
- **URL:** https://arxiv.org/abs/2409.07429 (*Agent Workflow Memory*).
- **Licence:** paper, idea-only; no repo code is used.
- **Our files:** `crates/endpoint/src/packs/session.rs`
  (`session.activity`, snapshots: activity and state, no procedure), a store in
  the project folder, `crates/endpoint/src/discover.rs` manifest.
- **Cost:** 2-5 days *(estimate)*.
- **What it replaces:** nothing; the door starts cold every session.
- **Cheap experiment:** ten repeated build tasks, cold arm against recall arm;
  count tool calls and chars. Kill: the recall arm needs more calls, or a
  remembered workflow applies to a changed scene without re-checking.
- **Donor claim:** +24.6% relative success on Mind2Web, +51.1% on WebArena, with
  fewer steps. τ-bench is the reason to care about reliability, not just
  capability: gpt-4o under 50% task success and pass^8 under 25% in retail
  (https://arxiv.org/abs/2406.12045).
- **Do not do:** make outputs stricter. "Let Me Speak Freely?"
  (https://arxiv.org/abs/2408.02442) finds strict JSON/XML format restrictions
  measurably degrade reasoning. Anthropic's tool notes say the same in
  practice: a concise response format is about a third of the detailed one
  (206 against 72 tokens) and is enough for the next step
  (https://www.anthropic.com/engineering/writing-tools-for-agents).

## 5. Audio

Steam Audio is Apache-2.0 (https://github.com/ValveSoftware/steam-audio/blob/master/LICENSE.md)
and its feature list is the map: HRTF binaural rendering, SOFA import, raycast
and volumetric occlusion, reflections, multi-path propagation, GPU/CPU
simulation (https://valvesoftware.github.io/steam-audio/doc/capi/index.html).
Our audio is integer, 8 octants, one synthetic 16-tap FIR per ear
(`crates/audio/src/hrtf.rs`), and a per-wall gain plus an air-loss term
(`crates/audio/src/occlude.rs`). Two steals fit; nothing needs the C library.

### 5.1 AAC6: find the path around the corner

- **What:** instead of one listener-to-source ray, search one or two propagation
  paths around geometry and take the best path's gain. Steam Audio calls this
  multi-path propagation and exposes the ray budget as the cost knob.
- **URL:** https://valvesoftware.github.io/steam-audio/doc/capi/index.html and
  https://valvesoftware.github.io/steam-audio/doc/capi/simulation.html
  (`numOcclusionSamples`, `maxNumRays`, `numBounces`).
- **Licence:** Apache-2.0; shape only, no bytes vendored.
- **Our files:** `crates/audio/src/occlude.rs` (`Wall`,
  `MAX_WALLS_PER_QUERY = 256`, per-wall absorption, air loss),
  `crates/audio/src/spatial.rs`.
- **Cost:** 3-5 days *(estimate)*.
- **What it replaces:** a straight-ray occlusion that steps to the full wall
  loss when the direct line is blocked, even when a door is open around the
  corner.
- **Cheap experiment:** a voice 2 m around a 1 m stone corner; assert the new
  gain is within 3 dB of a 64-ray reference and costs no more than
  `BUDGET_MS_PER_VOICE` 0.1 ms. Kill: over budget, or the curve still steps.

### 5.2 AAC7: a measured HRIR sphere

- **What:** replace the 8-octant synthetic filter with a measured HRIR sphere
  (about 32 directions, 16 taps) interpolated in azimuth and elevation; SOFA is
  the authoring format.
- **URL:** https://valvesoftware.github.io/steam-audio/doc/capi/hrtf.html
  (built-in HRTF plus SOFA import; `iplHRTFCreate` is "somewhat expensive" and
  not thread-safe) and https://www.sofaconventions.org/
- **Licence:** Steam Audio Apache-2.0; each SOFA dataset carries its own
  licence, so read it per file before shipping one.
- **Our files:** `crates/audio/src/hrtf.rs` (`OCTANTS = 8`, `HRTF_TAPS = 16`,
  `MAX_ITD_SAMPLES`).
- **Cost:** 2-3 days *(estimate)*, plus a dataset licence read.
- **What it replaces:** octant panning, which cannot tell 0 degrees from 40.
- **Cheap experiment:** offline, compare ITD and ILD error at 32 directions
  against a public SOFA set; the perceptual claim needs a listening A/B and is
  not run here. Kill: any band more than 6 dB off at 8 kHz, or state over 4 KB.

## Ranked: what I would take first

| # | steal | gain | cost (estimate) | first experiment |
|---|---|---|---|---|
| 1 | CA1 tell floor 30 ticks | readable startup doubles to the only measured floor, 250 -> 500 ms | hours + playtest | two tier files, six seeded fights, parries landed |
| 2 | AM30 dual-quaternion palette | palette halves (2,752 -> 1,376 B), candy-wrapper artifact goes | 1-2 days | 43-bone 170-degree twist, bytes and vertex error |
| 3 | PX46 Rapier joint motors | one mechanism instead of two, a force cap for free | 2-3 days | 600-tick stand + thrust, joint opening vs 0.8 cm |
| 4 | AM32 collision-aware motion match | the search stops clipping arches and corners | 2-4 days | corridor and arch, wall-intersecting ticks |
| 5 | MCP30 workflow memory | cold start becomes recall; fewer calls per repeated task | 2-5 days | ten build tasks, cold vs recall, call count |
| 6 | MCP29 one mid-call question | a refusal becomes a single round trip | 1-2 days | unknown-licence intake, one ledger row |
| 7 | PX47 small-steps XPBD | stiffness stops depending on iteration count | 1-2 days | pinned cloth, stretch at equal constraint count |
| 8 | AAC6 path around the corner | a voice behind a corner keeps its level | 3-5 days | 1 m stone corner, gain vs 64-ray reference |
| 9 | AM31 reduced skeleton at far tier | far tier skins 17 bones, not 43 | 2-3 days | 60 m silhouette error under a pixel |
| 10 | PX48 CCD cache clear | no tunnel after a restore or teleport | hours | teleport across a wall, 100 seeds |
| 11 | AAC7 measured HRIR sphere | height and front/back become audible | 2-3 days + licence | ITD/ILD error against a SOFA set |

## Proposed rows (board untouched)

| id | one line | F2P | P2P |
|---|---|---|---|
| AM30 | dual-quaternion skin palette behind a flag | palette 32 B/joint, vertex error under 1 mm | existing skin tests, digest |
| AM31 | 17-role proxy skeleton at `Tier::Far` | palette 43 -> 17, silhouette under 1 px at 60 m | near tiers unchanged |
| AM32 | world-geometry penalty in motion match | zero wall-intersecting ticks in the arch tape | open-floor clip choice identical, digest |
| CA1 | tell floor 30 ticks for tiers 1-3 | tell reads 500 ms, parry window inside it | six-seed determinism, hash receipt |
| PX46 | Rapier joint motors replace the servo | joint opening within the 1 cm margin at 4 passes | joint-opening test at 12 passes, digest |
| PX47 | small-steps soft solve | lower stretch at equal constraint count | pinned row holds, digest |
| PX48 | clear the CCD fixed-target cache on restore | zero tunnels over 100 seeds | replay after restore identical |
| MCP29 | `input_required` round trip on one verb | first result is `input_required`, retry lands | compliant asset path, one ledger row |
| MCP30 | induced workflow memory in the project folder | fewer calls on the tenth task | cold arm unchanged, no scene mutation |
| AAC6 | two-path propagation search | gain within 3 dB of a 64-ray reference | 0.1 ms per voice budget, curve still slews |
| AAC7 | measured HRIR sphere behind a flag | 32 directions, ITD/ILD error down | octant path unchanged when off |

## Still unverified

- No number here was produced by running code. The CPU cost of DQS, of the
  reduced skeleton and of the path search are estimates, not measurements; the
  donor papers give their own hardware and their own scenes.
- The tell-floor gain is a **readability hypothesis** with one 2025 thesis
  behind it (n = 30, median 800 ms). Nobody has played our build at 30 ticks.
  The thesis number is a donor claim, and its population is not ours.
- The Rapier motor model question has a real unknown: which model
  (`AccelerationBased` or `ForceBased`) holds our 15-bone rig stiffer at 60 Hz
  is untested. The 12-pass 0.8 cm number is our own test's, but for the impulse
  joints, not for motors.
- PX48 assumes a restore is a teleport for the CCD cache. I read the API name
  and the changelog entry, not the solver source, so the hole may not exist in
  our path.
- Workflow memory has no donor repo read and no licence check beyond the paper;
  the +24.6% / +51.1% are donor claims from web-agent benchmarks, not a game
  door.
- SOFA dataset licences vary per file and none was read here; AAC7 cannot ship
  a measured sphere until one is.
- Motion matching 2024-2026: the learned variant (https://dl.acm.org/doi/10.1145/3386569.3392440)
  and the Lipschitz/SMoE variant (https://dl.acm.org/doi/10.1145/3623264.3624442)
  were read at abstract level only, so their cost claims are not in this card.
