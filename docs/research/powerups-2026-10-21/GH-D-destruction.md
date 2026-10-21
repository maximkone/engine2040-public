# GH-D - destruction and impact physics (powerups wave, lane GH-D)

Lane question: which open-source GitHub repos hold techniques for DESTRUCTION and impact
physics - chunked and fracture destruction, blast and shock propagation, debris that settles
convincingly, rigid body piles that do not jitter, soft body and cloth that reacts to a hit,
and ragdoll and limb physics tricks - that work with a float Rapier-style world and can be
ported as ideas?

## The answer

The best donors are not physics engines, they are destruction systems that wrote down their
rules. `sbobyn/three-voxel-destruction` (MIT) is the richest find: two files,
`src/destruction.ts` and `src/physics.ts`, hold a blast that has three zones (a dust core,
loose clumps, and a thrown ring), a fracture
whose cells are **chosen by a distance weight** so the fragments are small where it broke, bonds
whose strength depends on the fracture level (inside a chunk, across a fragment, between
fragments, rubble), and a rule that settles debris by **parking it as a fixed body** instead of
leaving it to sleep (OBSERVED, read live 2026-10-21). `Kou1236/TriVoxel-Destructible-Terrain` (Apache-2.0) adds the
one thing our tree does not have at all: a blast is a **wavefront**, not a sphere - it starts at
a few pieces and crosses one edge per `stressDelayPerHop` (0.05 s), losing 0.72 of its strength
per hop, with a per-material conductivity and a per-frame budget of 256 applied hits (OBSERVED).
`ZachAR3/Destructibles-CSharp` (MIT) is a Godot 4 plugin that fractures by **repeated half-space
cuts** with real interior faces and an interior material, keeps the larger remainder so a wall
stays cuttable, turns force into a piece count, and spreads the shard activation over frames
under a 2 ms budget (OBSERVED). `Rawframe-Project/maul3d` (MIT) gives the blast a physical
reason: the impulse on a body scales by **the area that shape shows the blast** (OBSERVED).
`sagan-software/bevy-ragdoll` (MIT OR Apache-2.0) is the only donor that talks to our exact
backend: it drives Rapier joints per axis with `MotorModel::AccelerationBased`, a per-axis
`set_motor_max_force`, and a per-joint limit table - three things our `crates/physfx` does not
name today (OBSERVED). For the settling and jitter part, the honest answer is a correction:
Box2D v3 and Bullet already carry the rules we need (restitution only above a velocity
threshold, penetration recovery that adds no velocity, contact softness as `(hertz, zeta)`),
both were mined earlier in this repo for other files, so they are rows marked CORRECTION rather
than new finds.

### What I checked in our tree first (all OBSERVED 2026-10-21, `git -C C:\engine2040`)

- `crates/destruction/src/bonded/blast.rs`: a blast applies one impact per chunk inside the
  radius with a **linear falloff, all in the same tick** (file doc comment, `apply_blast`).
  There is no wave, no delay, no per-hop propagation and no material term.
- `crates/destruction/src/cluster.rs:26`: `CASCADE_FALLOFF_PERMILLE = 500` - damage already
  halves per hop along the bond graph, in the same step.
- `crates/destruction/src/fracture.rs:43-45`: `NEAR_EIGHTHS = 5` - five of every eight Voronoi
  sites cluster near the hit inside `radius_m`, the rest spread over the shell. The near cluster
  is pulled toward the hit with a **squared per-axis offset** (`let fall = unit * unit.abs()` in
  `scatter_sites_into`, OBSERVED); the far half is drawn **uniformly over the shell box**.
- `crates/destruction/src/bond_grade.rs`: grades a bond by contact area, pair mass, anchor
  distance and a seeded jitter (board text for `DX9`, SPRINT-08). Nothing grades a bond by
  fracture level.
- `crates/destruction/src/structure.rs:599` `ground_anchors()`: the ground layer is derived
  automatically, so the "break when the support below is gone" rule is already ours
  (`crates/destruction/src/bonded.rs` support flood, `is_supported`).
- `crates/physfx/src/fx/joints.rs:50`: hinges get `.limits(JointAxis::AngX, [low, high])`; the
  ball joints still take their swing cone from joint depth (`PX9` asks for a table instead).
- `crates/physfx/src/fx/hit.rs:44-63`: the PD motors call `set_motor_position(axis, angle,
  STIFFNESS*strength, DAMPING*strength)` on each free turning axis. **No** max force and **no**
  motor model is named anywhere, so which of Rapier's two motor models we use is UNKNOWN until
  the Rapier source is read.
- `git grep -il "soft_body\|softbody" -- crates/` hits only `crates/destruction/src/shape_match.rs`,
  `crates/physics/src/powered.rs`, `crates/physics/src/ragdoll.rs`, `crates/physics/src/rapier.rs`
  (mentions). There is no soft body in `crates/physfx`, so `C10` is still open.
- `git grep -il -e shockwave -e split_impulse -e relax_iter -e persistent_manifold -- crates/`
  returns only `crates/audio/src/gunfire.rs` (a sound file). Nothing in the tree separates a
  penetration recovery from a velocity.
- `crates/physfx/Cargo.toml`: `rapier3d = { version = "0.36", default-features = false,
  features = ["dim3", "f32", "std", "enhanced-determinism"] }` (workspace `Cargo.toml`).

## Donor table

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 (GH-D1) | A blast with three zones: a dust core, loose clumps and a thrown ring. The damage radius and the push radius are different (the push reaches 1.6x the damage radius), and the pressure of the blast reaches 3x the radius | `sbobyn/three-voxel-destruction`, https://github.com/sbobyn/three-voxel-destruction, read live 2026-10-21 (OBSERVED; 17 stars, pushed 2026-09-28, GitHub API) | MIT (`LICENSE`, 1068 B, "Copyright (c) 2026 Steven Bobyn", read live 2026-10-21) | `RZ6` (also `DX17`) | `src/destruction.ts` (16912 B): the `Hit` record (`radius`, `push`, `core` default 0.5), the split of the hit voxels into `gone` (dust) and `loose` (knocked loose), the constant `PRESSURE = 3`, `CRUSHED_TO_DUST = 0.4`, and the last line `physics.blast(at, radius * 1.6, push)`. Shape only, our own Rust, no bytes vendored | M | Kill if one landing of the three zones leaves the arena blast capture with the same rubble for a blast on the wall face and a blast at the wall foot, or if the three zones need more than the 1 ms break budget in `crates/destruction/src/budget.rs` | PORT |
| 2 (GH-D2) | The blast is a wavefront, not a sphere: it starts at a few pieces inside the radius and crosses one edge per delay, losing strength per hop, with a per-material conductivity and a per-frame budget on the applied hits | `Kou1236/TriVoxel-Destructible-Terrain`, https://github.com/Kou1236/TriVoxel-Destructible-Terrain, read live 2026-10-21 (OBSERVED; 1 star, pushed 2026-03-07) | Apache-2.0 (`LICENSE` read live 2026-10-21, "Apache License Version 2.0, January 2004") | `RZ6` (also `DX17`) | `Assets/Scripts/Systems/TriStressPropagationService.cs` (20524 B): `stressSourceCount = 3`, `stressMaxHops = 10`, `stressFalloffPerHop = 0.72`, `stressDelayPerHop = 0.05`, `stressMinDamage = 0.03`, `stressMaxQueuedHits = 20000`, `stressApplyBudgetPerFrame = 256`, the token-stamped visited arrays (`visitMarks` plus a `visitToken`), and `getTriangleStressConductivity`; `Assets/Scripts/Orchestration/TriDestructionOrchestrator.Explosion.cs` (3373 B) for the radius query and the `1 - dist / safeRadius` damage. All of it becomes integer (delay in ticks, falloff in permille, conductivity in the material table) so it can live in the hashed layer | M | Kill if no row asks for a travelling break and the wall still breaks in one piece in the capture, then the wave is not built; or if the delayed damage cannot be expressed in integer ticks without breaking the golden, then it stays a read | PORT |
| 3 (GH-D3) | The fracture cells are **chosen** by a distance weight: the seed draw is weighted by `(1 + d / reach) ** (-2 * falloff)` from the break, and 75% of the seeds are drawn by that weight, the rest uniformly. So the density grades continuously with the distance to the break instead of jumping at the edge of a cluster | `sbobyn/three-voxel-destruction`, https://github.com/sbobyn/three-voxel-destruction, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE` read live 2026-10-21) | `DX17` (also `RZ4`) | `src/destruction.ts` (16912 B) `fragmentChunks()` (the weight, the 75/25 draw, the count `voxels / mean`) and `fragments()` for the shell clumps. **CORRECTION to our tree**: `crates/destruction/src/fracture.rs` already pulls the near cluster toward the hit with a squared per-axis offset (`let fall = unit * unit.abs()` in `scatter_sites_into`, 5 of 8 sites near the hit via `NEAR_EIGHTHS`), but the far sites are then drawn **uniformly over the whole shell box**. The donor's new part is the weight on the *choice of cell*, which grades the far half too | S | Kill if the far half of the sites stays uniform over the shell box and the capture shows the far pieces are the same size as the near ones, then the weight is not written | PORT (correction: the near cluster already exists) |
| 4 (GH-D4) | The bonds follow the fracture level: a bond inside a chunk is the strongest, a bond across a chunk seam inside a fragment is a little weaker (the piece cracks there first), a bond between fragments is the weakest (a section comes down whole and splits along them where it lands), and the shell round a blast is rubble | `sbobyn/three-voxel-destruction`, https://github.com/sbobyn/three-voxel-destruction, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE` read live 2026-10-21) | `DX17` (also `RZ4`) | `src/destruction.ts` (16912 B) `bonds()` and the `Bond` enum in `src/physics.ts` (35115 B). Our side: `crates/destruction/src/bond_grade.rs` gains a fracture-level term (chunk / fragment / seam) beside the contact-area grade it has today, and `crates/destruction/src/bonded.rs` reads it | S | Kill if `crates/destruction/src/bond_grade.rs` already ranks a bond by a fracture level (it grades by contact area, pair mass, anchor distance and jitter), then nothing changes | PORT |
| 5 (GH-D5) | Debris that settles: when a piece comes to rest it is **parked as a fixed body** at its resting pose instead of staying a sleeping rigid body, and only the loose bodies are read back | `sbobyn/three-voxel-destruction`, https://github.com/sbobyn/three-voxel-destruction, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE` read live 2026-10-21) | `RZ4` (also `DX12`) | `src/physics.ts` (35115 B): `settleAt()` (the rubble is rewritten as fixed bodies at the pose it reached), `shatter()` (the share nearest the impact flies, the rest settles as rubble), `readLoose()` (only the loose list is gathered back from the GPU) and `blast()` (push loose bodies with a lift and a spin). Our side: `crates/physfx/src/fx.rs` gains a "park at rest" step and `crates/game/src/break_look.rs` keeps the parked count in the receipt | M | Kill if `RZ4`'s own accept ("every debris body sleeps within 6 s") is already met by the current physfx sleep ladder and the arena debris count stays under the cap, then the parking path is not built | PORT |
| 6 (GH-D6) | A blast impulse scaled by **the area a shape shows the blast**: a sphere shows pi r^2, a capsule pi r^2 plus 2 r times the axis cross direction, a hull shows half the summed cross products of the triangles that face the blast | `Rawframe-Project/maul3d`, https://github.com/Rawframe-Project/maul3d, read live 2026-10-21 (OBSERVED; 3 stars, pushed 2026-10-01) | MIT (`LICENSE` read live 2026-10-21, "MIT License, Copyright (c) 2026 Sirac Ozmen") | `RZ6` | `src/destruction.c` (10957 B): the header comment "an impulse on every body a blast reaches, scaled by the area a shape shows the blast and its distance from the center" and `ShapeProjectedArea()` for the sphere, capsule and hull cases; the same file couples the carve to the voxel fracture sweep and hands the fragment events to the host. Our side: `crates/physfx/src/fx/blast.rs` currently scales by distance only (OBSERVED) | S | Kill if no arena blast ever meets a flat plate edge-on, or if the area term cannot be computed from the collider we already have, then the impulse stays distance-only | PORT |
| 7 (GH-D7) | Soft bodies as XPBD particle lattices (boxes, ropes, cloth) that collide with the rigid world and with each other, anchor to bodies or to other lattices, and answer a wind | `Rawframe-Project/maul3d`, https://github.com/Rawframe-Project/maul3d, read live 2026-10-21 (OBSERVED; README read live) | MIT (`LICENSE` read live 2026-10-21) | `C10` (soft bodies for capes and banners, physfx only, float, never hashed) | `src/softbody.c` (25578 B), `src/softbody_solver.c` (39892 B), `src/softbody_pairs.c` (5731 B), `include/maul3d/softbody.h` (5545 B), `test/test_softbody.c` (36374 B) and `test/test_softsoft.c` (18006 B) for the test shape. Shape only: our own Rust in `crates/physfx`, float, never hashed | L | Kill if `C10` stays Tier C in `docs/FOCUS.md` and no cape or banner needs a soft body, then no soft body is written | PORT (C to Rust; float, physfx only) |
| 8 (GH-D8) | Rapier joint motors the way our backend wants them: the motor model named explicitly (`MotorModel::AccelerationBased`), the target angle and velocity per axis, a **per-axis max torque**, locked axes skipped, and the motor state not rewritten when the target has not changed. The limit table is `JointLimits { x, twist, z }` of `AngleRange { min, max }` with `is_locked()` | `sagan-software/bevy-ragdoll`, https://github.com/sagan-software/bevy-ragdoll, read live 2026-10-21 (OBSERVED; 1 star, pushed 2026-10-12) | MIT OR Apache-2.0 (`LICENSE-MIT` 1067 B "Copyright (c) 2026 Bill Curry" and `LICENSE-APACHE` 11358 B, both read live 2026-10-21) | `PX9` (also `C10`) | `crates/bevy_ragdoll_rapier3d/src/joint.rs` (11182 B): `apply_joint_motors()` with `set_motor_model(axis, MotorModel::AccelerationBased)`, `set_motor(axis, angles, angular_velocity, stiffness, damping)` and `set_motor_max_force(axis, max_torque)`, the `axis_mask` mapping, the `state.applied` cache; the table shape lives in the shared crate `src/profile/limits.rs` (3799 B) and `src/profile/joint.rs` (5401 B) - the `JointLimits { x, twist, z }` of `AngleRange { min, max }` with `is_locked()` is OBSERVED in the rapier3d joint file and its tests; the acceptance path is the backend-neutral conformance crate `crates/bevy_ragdoll_conformance/src/physics/hits.rs` (15167 B) whose named scenario `pistol_to_the_chest_does_not_move_the_pelvis_far` asserts that a chest hit moves the chest and not the pelvis and that the muscle recovers | M | Kill if the owner keeps the ragdoll drive on our own PD law (`crates/physics/src/pd.rs`) and never calls Rapier's motor API, then this row dies as a read | PORT |
| 9 (GH-D9) | Runtime fracture by repeated half-space cuts: real interior faces with their own material and UV scale, a surface roughness inside each cap triangle, a sharp hit that makes **one exact plane cut** and keeps the larger remainder (the remainder can be cut again), a blunt hit that bursts near the hit, a piece count taken from the force (`2 + floor(force / MinimumForce * (1 - sharpness))`, capped), a minimum volume ratio that refuses slivers, and the shard activation spread over frames under a 2 ms budget where the strictest budget among the queued jobs wins | `ZachAR3/Destructibles-CSharp`, https://github.com/ZachAR3/Destructibles-CSharp, read live 2026-10-21 (OBSERVED; 63 stars) | MIT (`LICENSE` 1074 B "Copyright (c) 2023 Zachary Schwentor", read live 2026-10-21) | `RZ4` (also `DX7`, `DX17`) | `addons/DestructiblesCSharp/MeshFracture.cs` (24056 B), `CapTriangulator.cs` (5432 B), `Destructible.cs` (23060 B), `DestructionScheduler.cs` (2571 B, the per-frame budget queue), `FractureOptions.cs` (2792 B, the pattern presets), `MeshRepair.cs` (7861 B, the non-manifold repair). C# to Rust, shape only; our side: `crates/destruction/src/shards3d.rs` gains the interior face and the interior material, `crates/destruction/src/budget.rs` gains the frame-budgeted activation, `crates/editor/src/fracture_tool.rs` (`DX7`) gains the cut preview | L | Kill if after one landing `RZ4` still draws every chunk as a box and the capture shows no rubble, then the shard mesh work stops (the row's own words) | PORT |
| 10 (GH-D10) | Contact softness as a frequency and a damping ratio instead of magic constants: `b2Softness { biasRate, massScale, impulseScale }` from `b2MakeSoft(hertz, zeta, h)` with the invariant `massScale + impulseScale == 1`; a separate relax stage for the position error and a separate restitution stage; restitution armed only when the approach speed passes a threshold, and the Poisson allowance `restitution * (compressionImpulse + approachImpulse) - restitutionImpulse` caps the bounce so a resting stack can never gain energy | `erincatto/box2d`, https://github.com/erincatto/box2d, read live 2026-10-21 (main branch, OBSERVED). **CORRECTION**: box2d and box3d were already read on 2026-10-12 in `docs/research/steals-2026-10-12/physnet.md` (rows 2, 8, 12, 13) for the constraint graph, the ragdoll joint table, the hit events and the replay log. This row is a **new fact from a different file**, not a repeat | MIT (`LICENSE` 1067 B "Copyright (c) 2022 Erin Catto", read live 2026-10-21) | `RZ4` (the settling leg; no row, propose a new row "debris settles without jitter" under `RZ4`) | `src/solver.h` (8534 B): `b2Softness`, `b2MakeSoft(hertz, zeta, h)` and the stage list `b2_stageSolve`, `b2_stageRelax`, `b2_stageRestitution`; `src/contact_solver.c` (25030 B): the `armed` test `relativeVelocity < -threshold` and the Poisson allowance. Our side: `crates/physfx` reads the material row of `RZ8` (landed) and adds a restitution threshold and an energy cap; the integer authority layer gets the same three numbers wherever it does its contact work - this lane did not find that file (`crates/physics/src/rigid.rs` is integration only: "fixed-dt semi-implicit Euler bodies + gravity + ground clamp", OBSERVED) | M | Kill if the physfx material table already applies a restitution threshold per material and the debris pile in the arena capture shows no jitter, then nothing is ported | PORT (CORRECTION) |
| 11 (GH-D11) | A penetration recovery that adds no velocity (the split penetration impulse), a bounce that only happens above a velocity threshold, and a **seeded shuffle of the constraint rows** that breaks the symmetry of a pile | `bulletphysics/bullet3`, https://github.com/bulletphysics/bullet3, read live 2026-10-21 (master, OBSERVED). **CORRECTION**: bullet3 was mined on 2026-10-20 in `docs/research/powerups-2026-10-20/11-collisions-cloth-layers.md` for the soft-body clusters and the channel matrix, and again in the sibling lane file `GH-S-simulation.md` row 10 for the raycast wheel. This row is a new fact from a different file | zlib (`LICENSE.txt` read live 2026-10-21: "The files in this repository are licensed under the zlib license") | `RZ4` | `src/BulletDynamics/ConstraintSolver/btSequentialImpulseConstraintSolver.h`: `m_resolveSplitPenetrationImpulse` (the penetration is pushed out by an impulse that does not become velocity), `restitutionCurve(rel_vel, restitution, velocityThreshold)` (bounce only above the threshold) and `m_btSeed2` ("used for re-arranging the constraint rows, improves convergence/quality of friction", with `setRandSeed`). Our side: `crates/physfx/src/fx/blast.rs` and the `RZ8` material rows | S | Kill if the Rapier build in `crates/physfx` (rapier3d 0.36) turns out to have no way to separate the penetration recovery from the velocity, and the arena debris pile shows no jitter, then this stays a read | PORT (CORRECTION) |
| 12 (GH-D12) | A partial break whose radius grows with the impact: the radius is `relativeVelocity / velocityToBreak * breakageMultiplier` around the hit point, and only the pieces inside that radius are released, so a light hit takes a small bite and a hard one takes a large one | `williambl/unity-destruction`, https://github.com/williambl/unity-destruction, read live 2026-10-21 (OBSERVED; 364 stars, pushed 2019-03-22) | MIT (`LICENSE` 1082 B "Copyright (c) 2017 William Bradford Larcombe", read live 2026-10-21) | `DX17` (also `RZ4`) | `Destruction/Assets/Scripts/Destruction.cs` (6359 B): the `sphereRadius` line, the `Physics.OverlapSphere(spherePoint, sphereRadius, 1 << 8)` query and the release of only the pieces inside it, the `useMomentum` switch (break on momentum or on velocity) and `BreakWithExplosiveForce(force, radius)`. Correction to our tree: its `breakOnNoSupports` raycast is **already ours** (`crates/destruction/src/bonded.rs` support flood), so only the severity-scaled radius is new | S | Kill if `crates/destruction/src/impact_table.rs` already carries a per-impact radius that scales with the impulse (it carries direction and power buckets and a **fixed** `radius_m` of 0.8 in the spec at `:357`, with `PUSH_EIGHTHS = 4.0` at `:42`), then nothing is ported | PORT |

## Rejections

- `Glavin001/Shardwright`, https://github.com/Glavin001/Shardwright, read live 2026-10-21: "a
  toolkit that fractures any mesh along its real material and structural weak points, producing
  render-ready fragments and exact bond graphs for destruction solvers" - the closest match to
  this lane's subject that we found, and the root listing has **no LICENSE file** (the listing
  holds `.cargo`, `.github`, `benchmarks`, `crates`, `docs`, `schemas`, `tools`, `README.md`,
  `THIRD_PARTY_NOTICES.md`, `Cargo.toml`, `Cargo.lock`, `Dockerfile` and the API reports no
  license), so there is no grant: read it, write our own code. REJECT as a port target.
- `elalish/manifold`, https://github.com/elalish/manifold, read live 2026-10-21: Apache-2.0
  (`LICENSE` read live), 2365 stars, a robust boolean library that would give a watertight
  chunk with a true interior face. It is the same gap as `dgreenheck/OpenFracture` (row 4 of
  `docs/research/powerups-2026-10-20/02-physics-vfx-destruction-steals.md`) and
  `sinanata/unity-mesh-fracture` (row 3 of `docs/research/hunt-2026-10-21/agent-3-physics-body.md`),
  both already mined. Not re-mined.
- `sbobyn/three-avbd`, https://github.com/sbobyn/three-avbd, read live 2026-10-21: MIT, 141 stars,
  AVBD in TypeScript with a WGSL backend. The AVBD idea itself is already carded on 2026-10-04
  in `docs/research/LADDER.md:202` from `savant117/avbd-demo3d` (MIT), so this is a bigger host
  for the same idea, not a new one. REJECT as a duplicate.
- `Terabase-Studios/Godot-Voxel-Destruction`,
  https://github.com/Terabase-Studios/Godot-Voxel-Destruction, read live 2026-10-21: MIT
  (`LICENSE` 1073 B), 61 stars. `addons/VoxelDestruction/Nodes/voxel_damager.gd` (5971 B) has a
  damage curve and a separate power curve and squares the curve's distance axis, and it knocks
  the debris back with an impulse scaled by the curve. Real, but a GDScript addon with no
  mapping onto our integer authority (our blast falloff is already a table), and the squares-the-
  distance-axis trick is a one-liner we can write ourselves. RESEARCH-ONLY, not in the table.
- `GlebTsereteli/Fracture`, https://github.com/GlebTsereteli/Fracture, read live 2026-10-21: MIT
  with its own `Fracture Demo/datafiles/Fracture License.txt`, 15 stars, a GameMaker project.
  `Fracture Demo/objects/objDemoBlast/Collision___FracturePiece.gml` applies one impulse away
  from the blast with the piece's own strength - weaker than what
  `crates/physfx/src/fx/blast.rs` already does (a radial falloff). REJECT.
- `Ideefixze/DTerrain`, https://github.com/Ideefixze/DTerrain, read live 2026-10-21: MIT
  (`LICENSE` 1074 B), 461 stars, a Unity destructible terrain whose `QuadTreeChunkCollider.cs`
  (4133 B) rebuilds the collision as chunks go. Our
  `crates/destruction/src/breach.rs` already records the dirty nav tiles of a breach
  (`:227`, `:401`, and the test `a_charge_opens_a_route_that_its_dirty_tiles_name` at `:761`,
  OBSERVED), so the gap is not there. REJECT.
- `NVIDIA-Omniverse/PhysX`, https://github.com/NVIDIA-Omniverse/PhysX, read live 2026-10-21:
  BSD-3-Clause per the GitHub API (the LICENSE file itself was not read), 4805 stars. The blast
  and the articulated ragdoll are there, but the brief
  excludes NVIDIA SDKs from porting and the tree already runs Rapier, so no shape we cannot get
  from the rows above. Not mined.
- `HunterProduction/unity-libre-fracture-2.0` (BSD-3) was already rejected on 2026-10-21 in
  `docs/research/hunt-2026-10-21/agent-3-physics-body.md`: the readable part is NvBlast, which
  is not open. `dgreenheck/OpenFracture` and `sinanata/unity-mesh-fracture` were mined on
  2026-10-20 and 2026-10-21. `jrouwe/Jolt` and `dimforge/rapier` are named in the brief as
  already mined. Nothing from those four goes in the table.
- Rows named in the brief that this lane does not fill: `DX13` (archived, its snapshot hunks
  landed), `RZ7` (cloth, ticked on 2026-10-17), `RZ9` (the budget ladder, ticked), `DX6`
  (debris LOD, landed on 2026-10-18). They stay as they are.

## Top 3

1. `sbobyn/three-voxel-destruction`, https://github.com/sbobyn/three-voxel-destruction, MIT,
   read live 2026-10-21. Four rows in two files: `src/destruction.ts` and `src/physics.ts`. It
   closes `DX17` with the distance-weighted cell choice and the bond tiers that follow the
   fracture level, and it closes the settling
   leg of `RZ4` with the "park the rubble as a fixed body" rule. Cost M.
2. `ZachAR3/Destructibles-CSharp`, https://github.com/ZachAR3/Destructibles-CSharp, MIT, read
   live 2026-10-21. It closes `RZ4` (the picture still draws every chunk as a box) with real
   interior faces, an interior material and a remainder that stays cuttable, and it gives `DX7`
   its cut preview. Cost L.
3. `Kou1236/TriVoxel-Destructible-Terrain`,
   https://github.com/Kou1236/TriVoxel-Destructible-Terrain, Apache-2.0, read live 2026-10-21.
   It closes `RZ6` with the one thing our tree has no way to express today: a blast that
   travels, one hop per delay, with a per-material conductivity and a per-frame budget. Cost M.
