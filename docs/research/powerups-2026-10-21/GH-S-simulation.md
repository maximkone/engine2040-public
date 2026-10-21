# GH-S - simulation that runs inside a fixed tick (powerups wave, lane GH-S)

Lane question: which open-source GitHub repos hold techniques for SIMULATION that runs inside a
fixed tick - crowd and pedestrian simulation, traffic on roads, cloth and soft bodies, water and
shore behaviour, and simple agent-based simulation (flocking, boids, steering) that stays
deterministic.

## The answer

The best open donors for this lane are a traffic microsimulator whose car-following, stop-line and
gate-phase maths is closed form and ports to Q16 integers (CityFlow, Apache-2.0), the missing half
of ORCA - obstacle segments and the corner rule, which is exactly what a crowd at a war gate needs
(snape/RVO2, Apache-2.0, already half-credited in our tree), and an open-source game-grade shore
model with both a state tile and a swash front (dgreenheck/tidewater, MIT). Pedestrian behaviour
comes as a shape, not as code we can take: CARLA's walker controller and the social force model
give the rule set, but both are float and one is Unreal-bound, so they are research rows. Cloth and
soft bodies are mostly already here: `crates/physics/src/cloth/` is a Verlet sheet with
intercollision and max-distance gradients, and Rapier soft bodies are already a dependency, so the
only new cloth fact is the tet-mesh volume constraint from Müller's PBD soft bodies. The honest
gap is therefore not the maths - our tree already has an integer ORCA solver
(`crates/nav/src/orca.rs`), a flock term (`crates/ai/src/depth/steer.rs`) and a 512-agent ORCA
bench (`crates/ai/tests/crowd_orca_bench.rs`), but nothing in the game calls any of them, there is
no traffic, no vehicle, no water and no shore anywhere in `crates/`, and the donor credits in the
tree are partly stale. The donors below are worth reading for the behaviour and the wiring, not for
the solver.

### What I checked in our tree first (all OBSERVED 2026-10-21, `git -C C:\engine2040`)

- `git grep "orca::" -- crates/` returns no hit: `crates/nav/src/orca.rs` is exported at
  `crates/nav/src/lib.rs:12` and called from nowhere in `crates/`.
- `crates/ai/tests/crowd_orca_bench.rs` is standalone: it carries its own `solve_agent` and its own
  `isqrt`, it does not call `engine_nav::orca`. NV8's target file `crates/nav/tests/soak.rs` does not
  exist (that folder holds only `tiles.rs`).
- `crates/ai/src/depth/steer.rs:235` calls `flock_wish(mx, mz, &[])` - the flock path is wired into
  the steer path but never receives a neighbour.
- `crates/nav/src/orca.rs:23-25` credits `PORT(snape/RVO2@main:Agent.cpp ...)`. Today the file is
  `src/Agent.cc`; the `.cpp` path does not resolve. The same credit says walls are hard half-planes
  "without polygon obstacles".
- No traffic, vehicle, wheel or suspension code in `crates/` (file listing of `crates/physics/src`
  plus `git grep` for wheel/tyre/suspension); `docs/plan/frontier.md:89` calls wheeled vehicles
  COLD and points at `crates/physics/src/vehicle.rs:1`, which does not exist.
- No water or shore simulation in `crates/` (only audio "swell" envelopes and a tissue density
  constant); `docs/plan/frontier.md:83` marks "Water surface (waves, buoyancy probe, no tick
  coupling)" as COLD with 0 plan files and 0 research.
- `crates/rome/src/geo.rs:557` has `SegmentKind::Road` (a frontier line in the atlas data), and
  there is no lane graph and no traffic model anywhere.
- `crates/physics/src/cloth/` holds `mod.rs`, `collide.rs`, `intercollision.rs`,
  `max_distance.rs`, `strand.rs`, `convert.rs` - Verlet sheets, intercollision particle sets, max
  distance gradients, Q16 strands. Soft bodies exist only as Rapier's `SoftBodySet` in
  `crates/physics/src/powered.rs` and `crates/physics/src/ragdoll.rs`.
- `docs/plan/body-and-break.md` cuts, in the tick: GPU-authoritative physics, material-point/FEM
  soft bodies, authoritative fluids. Every water and soft-body row below is therefore presentation
  only by policy.

## Donor table

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | A traffic microsimulator: a lane graph, a closed-form safe-speed car-following model, a stop-before-distance curve, a junction clearance rule (the distance a vehicle needs to clear the junction: `v^2/(2a) + v*interval*2`), and a traffic-light phase table with a per-link mask | `cityflow-project/cityflow`, https://github.com/cityflow-project/cityflow, read live 2026-10-21 through the GitHub contents API (OBSERVED) | Apache-2.0 (`LICENSE.txt`, OBSERVED) | No board row for road traffic: propose a new row "road traffic: carts follow the road, the war gates are phased lights"; also NV5 (its text points at SUMO, EPL-2.0, patterns only - CityFlow is Apache-2.0 and can be read as code) | `src/vehicle/vehicle.cpp` (`getCarFollowSpeed`, `getNoCollisionSpeed`, `getStopBeforeSpeed`, the `approachingIntersectionDistance` guard), `src/roadnet/trafficlight.cpp` + `src/roadnet/trafficlight.h` (phase list, per-phase duration, per-link availability mask, `passTime` that carries leftover time), `src/engine/engine.cpp` (`nextStep` with a fixed `interval`, `seed` into `rnd.seed`, `reset`) - port the algebra, write our own Q16 code | M | If the integer port needs one float to keep the same stop decisions; or if a cart's stop line drifts more than 10 cm over 600 ticks; or if the phase table cannot be driven by the hashed tick without a wall clock | PORT |
| 2 | The obstacle half of ORCA: static obstacle segments, convex-polygon obstacles, the corner rule, and the branch that feeds obstacle lines into the velocity solver | `snape/RVO2`, https://github.com/snape/RVO2, read live 2026-10-21 (OBSERVED) | Apache-2.0 (`LICENSE` and the SPDX headers in `src/Obstacle.h`, OBSERVED) | NV4 follow-on (near-field avoidance is ticked but the solver has no caller) and NV8 (its target file `crates/nav/tests/soak.rs` is missing): propose a new row "gate-mouth obstacle segments in the near-field crowd" | `src/Obstacle.cc` + `src/Obstacle.h` (segment obstacle, left/right neighbour links, `isConvex_`), `src/RVOSimulator.cc` (`addObstacle` from vertices, convexity test, `processObstacles`), `src/Agent.cc` (`insertObstacleNeighbor`, `linearProgram3(orcaLines_, numObstLines, ...)`) - port the obstacle branch into `crates/nav/src/orca.rs` in fixed point | S | If the obstacle pass changes the golden `d57b5c8ae3096614` (it must not: the crowd is not hashed); or if a crowd at a gate still overlaps the gate posts after the row | PORT (CORRECTION: the tree's citation says `Agent.cpp`, the file today is `src/Agent.cc`, and the obstacle half was never ported) |
| 3 | A shore state tile: four channels per cell - foam on the water, sand wetness that dries over about half a minute, foam residue left on the sand, and a depth-averaged flow speed that advects the foam; spray falling back deposits foam with an atomic add | `dgreenheck/tidewater`, https://github.com/dgreenheck/tidewater, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE`, OBSERVED) | No board row; propose a new row "shore tile: wetness, foam and flow on the sand (presentation only)". `docs/plan/frontier.md:83` marks the water surface COLD with 0 plan files | `src/ocean/ShoreSim.js` - the four-channel state, the dry/foam/residue lifetimes as data, the deposit buffer; we write a CPU Q16 tile of our own | S | If the tile needs the GPU or a float; or if any of it is read by the hashed path | PORT |
| 4 | PBD soft bodies on a tetrahedral mesh: edge constraints and volume constraints, each with an XPBD-style compliance (`alpha = compliance / dt / dt`) | `matthias-research/pages`, https://github.com/matthias-research/pages, read live 2026-10-21 (OBSERVED) | MIT header inside the page; no LICENSE file at the repo root (root listing, OBSERVED) - treat as MIT-by-header | BX21 (hero-tier flesh and cloth tearing, Tier C, listed as cut presentation in `docs/plan/body-and-break.md:164`) | `tenMinutePhysics/10-softBodies.html` - the tet-mesh edge and volume constraint pair with per-constraint compliance; we write our own integer code for banners and tabards | M | If a soft body ever has to be hashed (body-and-break.md cuts FEM/MPM soft bodies in the tick); or if the tet solve costs more than 0.2 ms per body at 30 Hz | PORT (CORRECTION: the same donor was mined on 2026-10-20 for `14-cloth.html`; this file is new) |
| 5 | A pedestrian walker: two nav filters (a crosser plans where the crosswalk is cheap and the road expensive, a non-crosser plans on a filter that excludes road and crosswalk areas), a clamped walk speed, and traffic lights grouped with per-group states | `carla-simulator/carla`, https://github.com/carla-simulator/carla, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE`, OBSERVED) | CT64 (content: reacting-crowd staging toward the C3 20k card) plus a new row "crowd agents at the gates: walking, waiting, and the gate opening" | `Unreal/CarlaUnreal/Plugins/Carla/Source/Carla/Walker/WalkerController.cpp` (the two filter classes, the speed clamp, the goal-point walk), `.../Traffic/TrafficLightManager.cpp` and `.../Traffic/TrafficLightGroup.cpp` (light groups and states) | L | If the pedestrian rule cannot be stated in integers without a navmesh query per agent per tick; or if the crowd needs a per-agent nav query the tick cannot afford | RESEARCH-ONLY (the code is Unreal-bound; read the shape, port the rule later) |
| 6 | The social force model as code: a pairwise pedestrian force, a 1-D in-front/behind term, and a space/wall potential | `svenkreiss/socialforce`, https://github.com/svenkreiss/socialforce, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE`, OBSERVED) | No board row; propose a new row "crowd at the gate: the wall pressure term" (our ORCA walls are half-planes ahead of the agent, so a gate mouth has no pressure term) | `socialforce/potentials/pedped_2d.py` (pairwise term), `socialforce/potentials/pedped_1d.py` (in-front/behind term), `socialforce/potentials/pedspace.py` (the wall/space potential) - read the model, write our own Q16 term | M | If the wall term cannot be written in Q16 integers; or if it changes the ORCA bench numbers | RESEARCH-ONLY (float and Python; the neighbour part is already better done by our integer ORCA) |
| 7 | The shore wave and swash: the wave grows in shallow water (Green's law) until the height passes a fraction of the depth, breaks into a turbulent bore, and runs up the beach as a thin swash sheet with an uprush and a backwash (0.4 and 0.55 of the wave period) and a foam line on the leading edge | `dgreenheck/tidewater`, https://github.com/dgreenheck/tidewater, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE`, OBSERVED) | Same new row as row 3 ("shore tile: wetness, foam and flow on the sand"), second half: the water line that runs up the sand | `src/ocean/ShoreWaves.js` - the shallow-water height growth, the bore trigger, the run-up front with its uprush/backwash timing, the foam line; today all of it is WGSL, we write a CPU 1-D swash front per shore column | M | If the swash front cannot be reproduced twice from the same seed in our own code; or if it needs a GPU compute pass | PORT (same donor as row 3, different idea) |
| 8 | A particle fluid as a density constraint: compute the SPH density, clamp the density error, take the Lagrange multiplier `lambda_i = -C_i / (sum ||grad C_k||^2 + eps)`, and move particles by the weighted kernel gradient sum | `InteractiveComputerGraphics/PositionBasedDynamics`, https://github.com/InteractiveComputerGraphics/PositionBasedDynamics, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE`, OBSERVED) | PX32 (physics: CPU SPH fluid step) - parked LATER by D7, and body-and-break.md cuts authoritative fluids, so this is an offline advisor only | `PositionBasedDynamics/PositionBasedFluids.h` (`computePBFDensity`, `solveDensityConstraint`) and `Simulation/SimulationModel.cpp` for the density error clamp | M | If the spike needs a float or a GPU; or if it is ever wired into the hashed tick | RESEARCH-ONLY (CORRECTION: this donor was mined on 2026-10-20 for the XPBD cloth side; the fluid side is new but out of scope while PX32 is parked) |
| 9 | Junction right-of-way and the light program: link states, lane successors, the vehicle's approach to a junction, and the leader that decides who goes first | `eclipse-sumo/sumo`, https://github.com/eclipse-sumo/sumo, read live 2026-10-21 (OBSERVED) | EPL-2.0 (`LICENSE`, OBSERVED) - copyleft: patterns only, no bytes, no link | NV5 (its row text already says "SUMO is EPL-2.0, patterns only"; this row adds the two files worth reading if that donor stays) | `src/microsim/MSLink.cpp` (link state and right-of-way), `src/microsim/MSBaseVehicle.cpp` (junction approach and the following model), `src/microsim/MSLeaderInfo.cpp` (who leads) | S | If we are ever tempted to link or copy SUMO code (we are not: patterns only); or if the right-of-way rule we take cannot be written in integers | RESEARCH-ONLY (EPL-2.0: read the rule, write our own code) |
| 10 | A raycast-wheel vehicle: each wheel casts a ray down its suspension axis, the spring force is `k * (rest - length) - d * velocity`, and tyre friction is split into forward and sideways impulses with a per-step impulse cap | `bulletphysics/bullet3`, https://github.com/bulletphysics/bullet3, read live 2026-10-21 (OBSERVED) | zlib (`LICENSE.txt`: "The files in this repository are licensed under the zlib license", OBSERVED) | UC19 (physics: military vehicles - the D1 raycast vehicle extended) plus a new row "cart: two wheels and a shaft on the Roman road" | `src/BulletDynamics/Vehicle/btRaycastVehicle.cpp` and `src/BulletDynamics/Vehicle/btRaycastVehicle.h` - the suspension spring/damper and the tyre impulse split; we write our own integer solver | M | If the cart cannot hold the same position over 600 ticks twice (hash equal); or if the suspension needs a float | PORT (CORRECTION: Bullet was mined on 2026-10-20 for soft-body clusters and filter groups; the vehicle files are new) |
| 11 | How to measure a crowd: cell density, flow across a line, speed distribution and a density profile, so a crowd gets a number instead of a look | `PedestrianDynamics/PedPy`, https://github.com/PedestrianDynamics/PedPy, read live 2026-10-21 (OBSERVED) | MIT (`LICENSE`, OBSERVED) | CT64 (content: reacting-crowd staging toward the C3 20k card) - the row has no density or flow bar today | `pedpy/methods/density_calculator.py`, `pedpy/methods/flow_calculator.py`, `pedpy/methods/speed_calculator.py`, `pedpy/methods/profile_calculator.py` - port the formulas into a small integer metric over the tick's positions | S | If the metric cannot be computed from the tick's integer positions; or if the gate crowd reads under the bar at the arena's own agent count | PORT (measurement only, S) |
| 12 | The flock credit in our tree points at a path that does not exist: at the cited commit the flock example is a scene file `examples/flock.lunaris.json` (110 KB), and there is no `examples/flock/flock.rs` | `AizenKai1001/lunaris` at sha `56cc7e30cbd21ece2314c155498a66f69fb36535`, https://github.com/AizenKai1001/lunaris, read live 2026-10-21 (OBSERVED: the `examples` listing and the root listing at that sha) | MIT OR Apache-2.0 (`LICENSE-MIT` and `LICENSE-APACHE` at the root of that commit, OBSERVED) | No board row; propose a new row "crowd behaviour at the gates: a neighbour feed for the flock and ORCA solvers" (the flock is called with an empty neighbour list at `crates/ai/src/depth/steer.rs:235`) | `examples/flock.lunaris.json` (the scene that shows the flock's own parameters) - read it, then write our own feed; the file `crates/ai/src/depth/steer.rs` cites is corrected to the path that exists | S | If the flock path is still called with an empty neighbour list after the row, the row has not moved | PORT (CORRECTION: our citation is stale; the donor file to read is the scene, not Rust source) |

## Rejections

Checked and dropped, so nobody re-mines them:

- `bevyengine/bevy` flock example - excluded by the wave rules (Bevy already mined).
- `jinleili/simuverse` (Rust + wgpu D2Q9 LBM GPU fluid) - mined by lane 02 on 2026-10-20 (PHX-2); not repeated.
- `InteractiveComputerGraphics/splashsurf` - mined by lane 02 (surface reconstruction from a particle pool); not repeated.
- `richardhozak/fppe` (fixed-point rigid bodies in Rust) - mined by lane 02 (PHX-5); not repeated.
- `matthias-research/pages` `tenMinutePhysics/14-cloth.html` - mined by lane 11 on 2026-10-20; row 4 uses the different file `10-softBodies.html`.
- `matthias-research/pages` `tenMinutePhysics/16-GPUCloth.py` and `18-flip.html` - GPU grid work; the tick stays on the CPU, and FLIP needs a grid plus advection we do not have.
- `matthias-research/pages` `tenMinutePhysics/17-fluidSim.html` - a CPU grid fluid (density grid, pressure in N/m); weaker than row 8 and PX32 is parked, so no row.
- `PedestrianDynamics/jupedsim` - LGPL-3.0 copyleft; a research read at best, and rows 5 and 6 already cover the two shapes we need.
- `PedestrianDynamics/vadere` - Java, research licence; dropped.
- `yuxiang-gao/PySocialForce` (MIT, Python) - the same model as row 6; one idea, one row.
- `ElsevierSoftwareX/SOFTX-D-21-00163` (SWANS, 1-D shallow-water runup on inclined beaches) - LGPL-3.0 and MATLAB; kept as a reading reference only, and the swash shape comes from row 7.
- `pozapas/awesome-crowdynamics` - a pointer list, not a donor; used only to find rows 6 and 11.
- `dgreenheck/tidewater` `src/ocean/OceanFFT.js`, `WakeSim.js`, `Caustics.js`, `UnderwaterLighting.js` - rendering and wake effects, not tick behaviour; dropped.
- `carla-simulator/carla` `Traffic/TrafficLightManager.cpp` as its own row - the light phase table we want is in CityFlow (row 1) and CARLA is Unreal-bound; dropped as a duplicate.
- `InteractiveComputerGraphics/PositionBasedDynamics` `Simulation/DistanceFieldCollisionDetection.cpp` - the cloth side of that donor is mined by lane 11; dropped.
- `snape/RVO2` `src/RVO.h` (the agent API) and `src/KdTree.cc` as separate rows - the KD-tree
  neighbour feed and the agent loop are already in `crates/nav/src/orca.rs` and
  `crates/ai/tests/crowd_orca_bench.rs`; only the obstacle half is new (row 2).

## Top 3

1. **CityFlow** (`cityflow-project/cityflow`, Apache-2.0, read live 2026-10-21) -
   https://github.com/cityflow-project/cityflow - closes the new row "road traffic: carts follow
   the road, the war gates are phased lights" and gives NV5 a donor whose licence lets us read the
   code, not only the pattern. Port target: `src/vehicle/vehicle.cpp`, `src/roadnet/trafficlight.cpp`,
   `src/engine/engine.cpp`.
2. **snape/RVO2** (`snape/RVO2`, Apache-2.0, read live 2026-10-21) -
   https://github.com/snape/RVO2 - closes the new row "gate-mouth obstacle segments in the
   near-field crowd" and corrects two stale facts in our tree: the citation says `Agent.cpp` (the
   file is `src/Agent.cc`) and the obstacle half was never ported. Port target: `src/Obstacle.cc`,
   `src/RVOSimulator.cc`, `src/Agent.cc` (`insertObstacleNeighbor`, `linearProgram3`).
3. **dgreenheck/tidewater** (`dgreenheck/tidewater`, MIT, read live 2026-10-21) -
   https://github.com/dgreenheck/tidewater - closes the new row "shore tile: wetness, foam and flow
   on the sand" plus the swash front, which is the first donor in the tree for the coast that
   `docs/plan/frontier.md:83` still marks COLD. Port target: `src/ocean/ShoreSim.js` and
   `src/ocean/ShoreWaves.js`.

Numbers note: no runtime number in this file is ours. Every figure quoted is the donor's own value,
read from the donor's source today: the shore tile's dry time 28 s, swash foam life 4.5 s, surf
foam life 2.6 s and residue life 5.0 s, the swash uprush 0.4 and backwash 0.55 of the wave period,
CityFlow's recommended tick interval of 0.2 s to 1.5 s. Our runtime is unmeasured.
