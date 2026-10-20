# Proposed rows: physics, VFX and destruction (2026-10-20)

The lane answer first, because every row below hangs on it: **VFX is planned in
this engine, and a lot of it is already written, but the picture half is not
wired up.** `docs/FOCUS.md` (HARDEN) names gore/VFX, networked physics,
destruction, ballistics/siege and a 20k GPU crowd; `docs/plan/render-look.md`
puts "GPU particles vs depth (RP2) -> decals (RP3)" into the frame graph and
owns `crates/render/src/particles_gpu.rs` and `crates/render/src/decals.rs`;
`docs/plan/body-and-break.md` owns the float `crates/physfx` layer and the
rule that blood, dust, cloth tear and GPU debris are presentation, never
hashed. What exists today: a seeded CPU particle pool with dust, flame, sparks
and blood kinds, a flipbook draw state and a soft depth fade
(`crates/render/src/particles_gpu.rs`, `crates/render/src/particle_draw_gpu.rs`),
a CPU hanabi-style effect layer (`crates/render/src/vfx_hanabi.rs`, a port of
the CPU half of bevy_hanabi only), deferred blood decals with depth reject
(`crates/render/src/decals.rs`), a CPU blood-and-water droplet pool
(`crates/render/src/fluids_gpu.rs`), octahedral crowd impostors and VAT
(`crates/render/src/crowd_impostor.rs`, `crates/render/src/shaders/crowd_vat.wgsl`,
`crates/assets/src/vat.rs`), and in `crates/physfx` a float Rapier world with
ragdoll, hinge joints, per-joint PD motors that fade after a hit, blast, impact,
sever and tether (`crates/physfx/src/fx/`). What is missing: (1) no GPU compute
particle simulation and no GPU sort, because `crates/render/src/vfx_hanabi.rs`
and `crates/render/src/shaders/particles_hanabi.wgsl` are CPU and
vertex/fragment only; (2) the live frame draws neither particles nor decals,
because `ForwardLook::live` in
`crates/render/src/present/gpu_present/submit.rs:147` sets `particles: false`
and only the probe path sets it (`crates/render/src/present/gpu_present/live.rs:1538`),
while the live frame runs forward plus blit only
(`crates/render/src/present/gpu_present/live.rs:1308` and `:1376`), and the soft
fade reads a zeroed 64-byte block
(`crates/render/src/particle_draw_gpu.rs:40`,
`crates/render/src/particles_gpu.rs:1501`); (3) `VX5` and `VX10`: there is no
GPU blood, fire or smoke at all, `crates/render/src/fluids_gpu.rs` is a CPU
droplet pool and the VX10 check file `crates/render/src/fluids_check.rs` does
not exist; (4) `UC11` has no file, `crates/world/src/smoke.rs` is missing;
(5) `NS11` is half done, `crates/game/src/break_look.rs:494` hands one
`BreakDust` to the render feed and nothing in `crates/render` consumes it;
(6) `LF6` and `LF7` have pools but the window never spawns dust, sparks or a
blood decal; (7) `DX7` has no tool, `crates/editor/src/fracture_tool.rs` does
not exist, and `RZ4` still draws every chunk as a box; (8) `C10` has no
`body.toml`, no `fx.*` door verbs in `crates/endpoint` and no soft bodies in
`crates/physfx`; (9) `PX9` is still depth-only, because
`crates/physics/src/ragdoll.rs:1109` sets the swing limit from the joint depth
and not from a per-joint table; (10) `RZ10` has a Node harness
(`tools/physfx-backends.mjs`) but no Rust Jolt binding. One board note is
stale and worth fixing: `VX5` still says it is frozen "until the first real
Bevy frame lands", but D8 and D9 deleted the Bevy viewer and FOCUS D7 now
reads "our own renderer", so that gate can never pass as written. That finding
is its own row below (`PHX-13`).

## The rows

| id | Title | What it is (2 lines) | Donor + exact file path + licence + date read | Row it closes | Owned paths | Accept (command and the one line it must print) | Kill line | Cost |
|---|---|---|---|---|---|---|---|---|
| PHX-1 | The GPU half of a hanabi particle system: compute update, GPU merge sort, indirect draw | A GPU particle system whose renderer lives entirely in compute shaders: one pass initialises and updates the pool, one mergesorts it, one draws it indirectly. Our tree has only the CPU batch layer. | `djeedai/bevy_hanabi` (Apache-2.0 + MIT, `LICENSE-APACHE2` and `LICENSE-MIT` read live 2026-10-20). Files: `src/render/mod.rs` (315191 B), `src/render/sort.rs` (63551 B), `src/render/vfx_update.wgsl` (7740 B), `src/render/vfx_init.wgsl` (7992 B), `src/render/vfx_sort.wesl` (18987 B), `src/render/vfx_indirect.wesl` (4571 B), `src/render/vfx_prefix_sum.wesl` (1831 B), `src/render/aligned_buffer_vec.rs` (46136 B). We write our own WGSL and our own buffers, zero bytes vendored. | `NS11` (also unblocks `LF6`, `RL52`) | `crates/render/src/particles_gpu.rs` (compute half), `crates/render/src/shaders/particles_hanabi.wgsl`, a new `crates/render/src/shaders/particles_sort.wgsl`, `crates/render/src/particle_draw_gpu.rs` (indirect draw args) | `node tools/gh-check.mjs cargo test -p engine-render --lib particles_gpu` -> `NS11 gpu compute pool: 4096 particles, one sort pass, indirect instance_count=4096` | Kill if the CPU pool of 4096 with the flipbook draw already holds the 1.0 ms particle budget on the RTX 3050 and the `LF6`/`LF7` captures pass, then the GPU compute path is not built | L |
| PHX-2 | A Rust and wgpu grid fluid: D2Q9 LBM on GPU compute, plus PBD cloth, plus a wasm build script | A whole fluid solver in the language and the graphics API we already use: a D2Q9 lattice-Boltzmann grid on compute shaders, a position-based cloth, and a wasm build. Our `fluids_gpu.rs` is a CPU droplet pool with no grid. | `jinleili/simuverse` (MIT, `LICENSE.MIT` read live 2026-10-20). Files: `simuverse/src/fluid/d2q9_node.rs` (12604 B), `simuverse/src/fluid/fluid_simulator.rs` (8615 B), `simuverse/src/fluid/lattice.rs` (3479 B), `simuverse/src/pbd/cloth.rs` (14024 B), `simuverse/src/pbd/gen_cloth_constraints.rs` (11550 B), `build-wasm.sh`. Shape only, our own Rust. | `VX5` (and gives `VX10` its check target) | `crates/render/src/fluids_gpu.rs` (the grid half), a new `crates/render/src/shaders/fluid_grid.wgsl`, `crates/render/src/fluids_check.rs` | `node tools/gh-check.mjs cargo test -p engine-render --lib fluids` -> `VX5 grid fluid: 32^3 lattice, 2 Jacobi steps, sim bytes x2 identical` | Kill if the grid cannot hold the `VX10` budget of 1.0 ms on the RTX 3050, or if the owner keeps `VX5` frozen, then nothing is ported | M |
| PHX-3 | A GPU Eulerian fire and smoke solver: 128-voxel cube, vorticity confinement, Jacobi pressure, blackbody ray march | A fire and smoke simulator that runs entirely in compute shaders on a voxel grid, then ray marches the density with Planck blackbody radiation for the flame colour. The shape of `VX5` fire and smoke and of `VX2` volumetrics. | `olliep24/firesim` (NO LICENCE FILE in the repo root on 2026-10-20 and no `license` key in `Cargo.toml`, so there is no grant: read the shape, write our own WGSL, never copy a line). Files: `src/advect_scalars.wgsl` (2485 B), `src/advect_velocity.wgsl` (2173 B), `src/compute_divergence.wgsl` (2480 B), `src/compute_pressure.wgsl` (2784 B), `src/subtract_pressure_gradient.wgsl` (2723 B), `src/add_vorticity_confinement_force.wgsl` (3343 B), `src/compute_curl.wgsl` (2509 B), `src/render_shader.wgsl` (6624 B), `src/compute_step.rs` (3149 B), `src/ping_pong.rs` (1159 B). | `VX10` (and the fire half of `VX5`, `VX2`) | `crates/render/src/shaders/fire_grid.wgsl` (new), `crates/render/src/shaders/fire_raymarch.wgsl` (new), `crates/render/src/fluids_gpu.rs` | `node tools/gh-check.mjs cargo test -p engine-render --lib fire` -> `VX10 fire grid: 32^3 voxels, vorticity on, sim bytes x2 identical` | Kill if no fire cell ever appears in the arena and the owner keeps `VX5` frozen, then the fire solver is not written | M |
| PHX-4 | Runtime fracture authoring and shard meshes: plane slicer, fragmenter, constrained triangulation, inside material, edge UV remap | A fracture package that cuts a mesh at runtime, fragments the pieces, and gives each shard a real inside surface with remapped UVs. Exactly the "every chunk is a box" gap in `RZ4` and the missing editor tool in `DX7`. | `dgreenheck/OpenFracture` (MIT, `LICENSE.md` read live 2026-10-20). Files: `Runtime/Scripts/Fracture.cs` (10257 B), `Runtime/Scripts/Slicers/PlaneSlicer.cs` (2352 B), `Runtime/Scripts/Fragment/MeshSlicer.cs` (16645 B), `Runtime/Scripts/Fragment/ConstrainedTriangulator.cs` (29073 B), `Runtime/Scripts/Fragment/Fragmenter.cs` (12084 B), `Runtime/Scripts/Fragment/FragmentData.cs` (11104 B), `Runtime/Scripts/Slice.cs` (5222 B), `Runtime/Scripts/Prefracture.cs` (5363 B). C# to Rust, shape only. | `RZ4` (and the shape of `DX7`) | `crates/destruction/src/shards3d.rs` (the inside surface), a new `crates/editor/src/fracture_tool.rs` | `node tools/gh-check.mjs cargo test -p engine-destruction --lib shards` -> `RZ4 shard mesh: 12 chunks, inside faces present, UV remapped` | Kill if after one landing `RZ4` still draws every chunk as a box and the capture shows no rubble, then the shard mesh work stops | M |
| PHX-5 | A fixed-point rigid body engine in Rust, deterministic on every machine | A whole rigid body solver written in fixed-point arithmetic, so it gives the same bits on every CPU and OS. The alternative cost oracle for `C15` next to the already named `box3d-rust` and `fixed3d`. | `richardhozak/fppe` (MIT OR Apache-2.0, `LICENSE-MIT` read live 2026-10-20 and the `license` field of `Cargo.toml` read live 2026-10-21). Files: `src/lib.rs` (34622 B, the `I32F32` unit type and the fixed-point body solver), `src/vec.rs` (4487 B). It is a port of tinyphysicsengine over the `fixed` and `cordic` crates. | `C15` (read only: the number it sets for a fully integer solver) | `docs/research/powerups-2026-10-20/proposed-rows-02-physics-vfx-destruction.md` (this file) | `node tools/gh-check.mjs cargo test -p engine-physics --lib fixed` -> `C15 integer solver cost read from the donor table, no port` | Kill if the port of `fixed3d` already prices the integer solver, then this is reading only. The donor README says the library is incomplete with probable bugs, so it can never be the oracle itself | S |
| PHX-6 | Safe Rust bindings to Jolt Physics 5.3.0 over JoltC: body interface, narrow phase, physics system | A Rust wrapper over Jolt so the second-backend comparison can run inside our own harness instead of a Node script that re-implements the scene. | `SecondHalfGames/jolt-rust` (MIT OR Apache-2.0, `LICENSE-MIT` read live 2026-10-20). Files: `crates/rolt/src/body_interface.rs` (3952 B), `crates/rolt/src/narrow_phase.rs` (7513 B), `crates/rolt/src/physics_system.rs` (5724 B), `crates/rolt/src/traits.rs` (17358 B), `crates/rolt/src/reference.rs` (3816 B), `crates/joltc-sys`. | `RZ10` | `crates/physfx/Cargo.toml` (a `jolt` feature), a new `crates/physfx/src/jolt.rs` (the adapter), `tools/physfx-backends.mjs` (reads the Rust table) | `node tools/gh-check.mjs cargo test -p engine-physfx --features rapier,jolt --lib jolt` -> `RZ10 Jolt arm: 318 bodies, mean 241 us, p95 402 us beside Rapier 198 us` | Kill if the mirror run of `tools/physfx-backends.mjs` already prints both rows and Rapier holds the `RZ9` bar at every tier, then no Rust Jolt binding is built. Also kill on the first day without a C++ toolchain on the runner, because JoltC must compile | M |
| PHX-7 | Long-range attachment constraints, bending constraints, SDF collision and a spatial hash for cloth | A cloth solver whose constraint set is the part our verlet cloth lacks: attachments that hold a fold to a shoulder, bending, and a neighbour search. | `vitalight/Velvet` (MIT, `LICENSE.txt` read live 2026-10-20). Files: `Velvet/SpatialHashGPU.cu` (5059 B), `Velvet/SpatialHashGPU.hpp` (4144 B), `Velvet/SpatialHashCPU.hpp` (2650 B), `Velvet/Common.cuh` (2152 B), `Velvet/ParticleInstancedRenderer.hpp` (2189 B). Shape only, CUDA to our CPU verlet. | `E81` (hair and rope cloth, presentation only) | `crates/physics/src/cloth.rs` (the attachment and bending constraints) | `node tools/gh-check.mjs cargo test -p engine-physics --lib cloth` -> `E81 hair rope: Q16 verlet x2 equal, 0.5 ms bar` | Kill if `crates/physics/src/cloth.rs` holds the 0.5 ms bar for hair and rope without long-range attachments, then nothing is ported | M |
| PHX-8 | The recovered rules of a shipped structural destruction game: support removal, per material break behaviour, fracture response | A working reconstruction of a commercial game's destruction rules, with a written rule reference. It is the rules text we would compare our own integer load leg against, not code we would port. | `DevelopmentStatus/rfg-physics-sandbox` (MIT, `LICENSE` read live 2026-10-20). Files: `Docs/destruction_reference.md`, `Research/evidence_registry.md` (26330 B), `ReverseEngineering/`, `Crates/`. It needs the reader's own copy of the game for its assets, which we do not use. | `DX17` (the impact-keyed fracture table) | `docs/research/powerups-2026-10-20/proposed-rows-02-physics-vfx-destruction.md` (this file) | `node tools/gh-check.mjs cargo test -p engine-destruction --lib impact_table` -> `DX17 impact table read against the donor rules, no port` | Kill if our own integer load leg in `crates/destruction/src/structure.rs` already covers every rule the doc lists, then the doc is not read again | S |
| PHX-9 | A cell based fire model with fuel, heat, burn stages and wind | A cell model of fire spread over a grid, with a fuel and burn stage per cell. The shape of the smoke and fire grid `UC11` wants, read as rules. | `MadeInShineA/forest-fire-simulation` (NO LICENCE FILE in the repo root on 2026-10-20, so there is no grant: read the shape, write our own code). Files: `forest_fire_simulation_model.md` (8556 B, the cell model), `src` (the Rust simulation core). | `UC11` | `docs/research/powerups-2026-10-20/proposed-rows-02-physics-vfx-destruction.md` (this file) | `node tools/gh-check.mjs cargo test -p engine-world --lib smoke` -> `UC11 smoke cell: fuel and heat integers, spread replays x2` | Kill if the owner keeps the smoke grid out of the sim, then the model is not read again | S |
| PHX-10 | Crowd as one tick for the whole crowd: group decisions, distance LOD tiers, vertex-animation clips, batch spawn | A crowd system that gives thousands of units one tick and a per-distance representation, instead of a per-unit cost. The shape behind `RL54` and `QA10`. | `DigiLogicLabs/dll-mass-units` (Apache-2.0, `LICENSE` 11350 B in the root and the README states Apache-2.0, read live 2026-10-20; the host is Unreal Engine 5, so this is research only and no code is ported). Files: `Source`, `docs/INTEGRATION_GUIDE.md`, `CHANGELOG.md`. | `RL54` (and `QA10`) | `docs/research/powerups-2026-10-20/proposed-rows-02-physics-vfx-destruction.md` (this file) | `node tools/gh-check.mjs cargo test -p engine-render --lib crowd` -> `RL54 crowd tiers read from the donor, no port` | Kill if `crates/render/src/crowd_inst.rs` and `crates/render/src/crowd_seats.rs` already seat 20k spectators inside the 1.5 ms bar, then no crowd LOD rewrite is started | S |
| PHX-11 | A surface reconstruction from a particle pool: density map, marching cubes, halfedge cleanup | A library that turns SPH particles into a triangle mesh, so a blood pool or a water sheet is a surface and not a cloud of sprites. The missing half of the blood picture in `LF7` and of `VX5`. | `InteractiveComputerGraphics/splashsurf` (MIT, `LICENSE` read live 2026-10-20). Files: `splashsurf_lib/src/marching_cubes.rs` (14017 B), `splashsurf_lib/src/density_map.rs` (31707 B), `splashsurf_lib/src/halfedge_mesh.rs` (22318 B), `splashsurf_lib/src/postprocessing.rs` (33000 B), `splashsurf_lib/src/neighborhood_search.rs` (29060 B), `splashsurf_lib/src/reconstruction.rs` (7108 B). | `LF7` (and the surface half of `VX5`) | `crates/render/src/fluids_gpu.rs` (surface from the pool), a new `crates/render/src/fluid_surface.rs` | `node tools/gh-check.mjs cargo test -p engine-render --lib fluid_surface` -> `LF7 blood surface: 2048 particles, 1 mesh, bytes x2` | Kill if the pool stays a sprite-and-decal presentation and no row ever asks for a fluid surface, then the crate is not added | S |
| PHX-12 | Per-instance animation offset for instanced meshes, and a vertex animation texture bake | A library that bakes vertex positions per frame into a texture and overrides the animation frame per instance, which is what the stands need so twenty thousand spectators are not all on frame one. | `codewriter-packages/Mesh-Animation` (MIT, `LICENSE.md` read live 2026-10-21; the host is Unity, so research only). Files: `Runtime/Scripts` (the per-instance override path), `Runtime/Shaders` (the vertex texture read). Shape only. | `QA10` (and `VG9`) | `crates/render/src/crowd_inst.rs` (the per-instance phase offset) | `node tools/gh-check.mjs cargo test -p engine-render --lib crowd_inst` -> `QA10 per-instance phase: 20000 instances, 20000 distinct phases` | Kill if `crates/render/src/crowd_inst.rs` cannot bind its seats in the presenter and `QA10` never gets a capture, then the per-instance offset is not added | S |
| PHX-13 | The `VX5` freeze names a gate that no longer exists | `VX5` still carries the LATER line "frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands". D8 (2026-10-06) and D9 (2026-10-07) deleted the Bevy viewer, and FOCUS.md D7 now reads "Frozen until the first real frame from our own renderer (crates/render, D8, D9) and the one world (scene drives the match) land". No Bevy frame will ever land, so the gate can never pass as written and the row is stuck forever. | Not a donor: the board text itself, `docs/SPRINT-22-web-realism.md` row `VX5`, read live 2026-10-21, next to `docs/FOCUS.md` D7, D8 and D9. | `VX5` (the freeze text) | `docs/SPRINT-22-web-realism.md` (the lead merges it; this file only names the change) | `node tools/board-slice.mjs VX5` -> the row prints the corrected freeze line, no "Bevy frame" left in it | Kill if the owner says `VX5` stays frozen on purpose, then the text is right and this row dies | S |

The freeze line should read, in place of the Bevy sentence: **"LATER: tier C, frozen by owner decision D7 (docs/FOCUS.md) until the first real frame from our own renderer (crates/render, D8, D9) and the one world (scene drives the match) land; the loop skips it; unpark by deleting this line."** That is the FOCUS.md D7 wording, so the board stops naming a frame that no longer exists. `D7` in FOCUS.md also freezes "tier C spikes", and `VX5` is Tier C, so the freeze itself stands; only its wording is wrong.

## The render lane, first: the order the VFX picture must be wired

LF6 and LF7 have pools that nothing spawns, and RL52 needs the draw. The order
is not free: the draw needs a staged batch, the staged batch needs particles
in the pool, and the particles need a cue. So:

1. **Spawn.** The cue turns into pool particles: a footfall, a block, a parry, a
   knockdown, a break. This is the `LF6` and `LF7` work and it lives in
   `crates/game/src/fighter_look.rs` (the cue side) and
   `crates/game/src/break_look.rs` (the break side), writing into the pool the
   presenter already owns. Until this lands, every later step draws nothing.
2. **Draw.** The live frame must draw the staged batch. Today
   `ForwardLook::live` sets `particles: false`
   (`crates/render/src/present/gpu_present/submit.rs:147`) and only the probe
   path sets it (`crates/render/src/present/gpu_present/live.rs:1538`), so
   `crates/render/src/present/gpu_present/live.rs:1300` (the live
   `encode_forward`) never calls `encode_particle_pass`
   (`crates/render/src/present/gpu_present/submit.rs:1164`). The live path
   needs the pool stepped and staged once per frame, then the same
   `encode_particle_pass` the probe path uses.
3. **Soft fade.** The fade reads a constant zero gap:
   `crates/render/src/particle_draw_gpu.rs:40` fixes the buffer at 64 bytes and
   `crates/render/src/particles_gpu.rs:1501` writes 64 zero bytes into it. The
   frame's real depth must reach that buffer, so a particle behind a wall
   fades instead of drawing through it. This needs the depth copy the forward
   pass already makes, so it is a bind, not a new pass.
4. **Decals before TAA.** The blood decal must be in the frame before the TAA
   resolve, or the resolve smears it. `crates/render/src/decals.rs` has the
   pool and `upload_instances` (`:917`) but no draw pass: the decals are a
   bind probe (`:985`). The pass belongs next to the particle pass in
   `crates/render/src/present/gpu_present/submit.rs`, before the resolve.
5. **The GPU sort.** Last, because it only pays off once the draw carries
   thousands of blended quads: the sort and the indirect draw are `PHX-1`.

## For the render lane

Owner: `crates/render`. Not touched by this packet: `crates/render` is held by
the GS44 landing agent. These are the exact patches the render lane owes, so
they can be applied as soon as the hold lifts.

**Spec R1: the live path spawns and draws particles.**

- File: `crates/render/src/present/gpu_present/live.rs`.
- Function: `present_frame_live` (`:1187`).
- Change: after `let sky_timed = self.encode_forward(...)` (`:1300`), the live
  path must not rely on `ForwardLook::live` having set `particles: false`
  (`crates/render/src/present/gpu_present/submit.rs:147`). Instead the live
  path steps the pool once per frame and stages it, then passes
  `ForwardLook::live` a `particles: true` when the staged count is above
  zero.
- The pool lives on the presenter: `self.particles` is stepped in
  `crates/render/src/present/gpu_present/submit.rs:339`
  (`self.particles.step(particles_gpu::TICK_DT)`) and staged through
  `ParticlePool::stage_pool` (`crates/render/src/particles_gpu.rs:1353`), which
  binds the camera with `set_draw_view` (`:1320`) and returns a
  `ParticleGpuReceipt`. The live path must call `stage_pool` with the frame's
  `frame.built_view_proj` and the camera right and up axes, then let
  `encode_particle_pass` (`crates/render/src/present/gpu_present/submit.rs:1164`)
  draw the staged batch.
- New helper: `fn live_particle_batch(&mut self, frame: &Frame, enc: &mut wgpu::CommandEncoder, targets: &Targets, timer: Option<(&GpuTimer, &mut PassesTimed)>) -> bool`,
  next to `present_frame_live`, so the probe path and the live path share one
  shape. Return `true` when a batch was drawn.
- Test: `crates/render/src/particles_gpu.rs` tests, file
  `crates/render/src/particles_gpu/tests.rs` (or the crate's existing test
  module), named
  `the_live_frame_stages_and_draws_the_particle_batch`: with a pool that has
  64 live sparks, the live path reports `staged_count() == 64` and the frame
  receipt carries a particle pass time above zero.
- Proof: `node tools/gh-check.mjs cargo test -p engine-render --features window --lib particles_gpu`
  -> `the_live_frame_stages_and_draws_the_particle_batch ... ok`. The window
  feature needs a GPU, so this line runs on the PC (the helper sends GPU and
  window lines to the PC itself).
- On/off pair: `crates/render/src/pass_truth.rs` gains a `particles` toggle
  (the probe toggles already exist) so a capture pair proves the pass changes
  pixels.

**Spec R2: the decal draw before the TAA resolve.**

- File: `crates/render/src/present/gpu_present/submit.rs`.
- New function: `fn encode_decal_pass(&mut self, enc: &mut wgpu::CommandEncoder, targets: &Targets, timer: Option<wgpu::RenderPassTimestampWrites<'_>>) -> bool`,
  beside `encode_particle_pass` (`:1164`).
- The decal data is already on the CPU side: `DecalPool::upload_instances`
  (`crates/render/src/decals.rs:917`) gives the instance bytes and
  `DecalPool::place` (`:766`) puts one down. The pass needs a pipeline over
  `crates/render/src/shaders/` (a new `decal.wgsl`), an instance buffer fed
  by `upload_instances`, and the frame's depth for the depth reject the pool
  already computes in `crates/render/src/decals.rs:416`.
- Order: the decal pass must be encoded BEFORE the TAA resolve, and after the
  forward pass, so the resolve sees the decal. The live frame's resolve is
  encoded in `present_frame_live`; the probe frame's in `present_frame_probed`
  (`crates/render/src/present/gpu_present/live.rs:1469`).
- Test: `crates/render/src/decals.rs` tests, named
  `the_decal_pass_draws_the_pooled_decals`: after one `place`, the pass draws
  `pool.used()` instances.
- Proof: `node tools/gh-check.mjs cargo test -p engine-render --features window --lib decals`
  -> `the_decal_pass_draws_the_pooled_decals ... ok`.
- Kill line: if the decal pool never receives a decal from a hit cue (the
  `LF7` spawn is not landed), the pass is dead weight: hold the pass until the
  spawn lands.

**Spec R3: the soft fade reads the real depth.**

- Files: `crates/render/src/particle_draw_gpu.rs` and
  `crates/render/src/particles_gpu.rs`.
- Change 1: `PARTICLE_DRAW_SCENE_DEPTH_BYTES` (`:40`) is a fixed 64 bytes, and
  `particles_gpu.rs:1501` writes `[0u8; 64]` into it every frame, so
  `scene_depth` in `crates/render/src/shaders/particle.wgsl:34` is always zero
  and the fade is off. The buffer must be sized from the frame's resolution
  (one `f32` per texel of a half-res depth, plus a header with the width and
  height) and filled with the frame's depth.
- Change 2: the fade is computed in `ParticlePool::draw_records`, which today
  takes a closure `|_| 0.0` (`crates/render/src/particles_gpu.rs:1369`). The
  closure must return the real depth gap: the particle's view depth against the
  depth of the texel it covers.
- Change 3: the depth copy. The forward pass already writes the frame depth
  (`live.rs:1286` writes the AO field from it) and the fog feed copies it, so
  the copy exists; the particle pass needs its own read of the same texture
  (a `texture_depth_2d` binding) rather than a storage buffer of floats, which
  is cheaper and needs no copy at all.
- Test: `crates/render/src/particle_draw_gpu.rs` tests, named
  `the_soft_fade_reads_the_real_depth`: a particle behind the scene depth
  fades to zero and a particle in front of it keeps its alpha.
- Proof: `node tools/gh-check.mjs cargo test -p engine-render --features window --lib particle_draw_gpu`
  -> `the_soft_fade_reads_the_real_depth ... ok`.
- Kill line: if the depth texture cannot be read in the particle pass without
  a copy that costs more than the 1.0 ms particle bar, the fade rides the
  record alpha (as today) and this spec dies.

**Spec R4: the GPU sort and the indirect draw (`PHX-1`).**

- Files: `crates/render/src/shaders/particles_hanabi.wgsl` (the compute half),
  a new `crates/render/src/shaders/particles_sort.wgsl`,
  `crates/render/src/particle_draw_gpu.rs` (the indirect args).
- The compute pass needs a ping-pong pair of particle buffers, a radix or
  merge sort keyed on the view depth, a prefix sum of the alive counts and an
  indirect `instance_count`. The donor shapes are
  `src/render/sort.rs` and `src/render/vfx_sort.wesl` of bevy_hanabi, read
  live 2026-10-20; we write our own WGSL.
- Test: `crates/render/src/particles_gpu.rs` tests, named
  `the_gpu_sort_orders_the_particles_back_to_front`: 4096 particles at random
  depths come out of the sort in non-decreasing depth order.
- Proof: `node tools/gh-check.mjs cargo test -p engine-render --features window --lib particles_gpu`
  -> `the_gpu_sort_orders_the_particles_back_to_front ... ok`.
- Kill line: the CPU pool of 4096 with the flipbook draw holds the 1.0 ms
  particle budget on the RTX 3050: then no GPU sort is built.

## For the endpoint lane

Owner: `crates/endpoint`. Held by the AI-native lane; this is the spec, not
code.

**Spec E1: the four `fx.*` door verbs and the C10 receipt shape.**

- File: a new `crates/endpoint/src/packs/fx.rs`, registered in
  `crates/endpoint/src/packs/mod.rs` next to `physics.rs` and `break_.rs`.
- Verbs: `fx.ragdoll`, `fx.limb`, `fx.break`, `fx.blast`. Each takes the
  authority event and returns a receipt. The receipt is a flat table of
  integers and strings, never a float, because it is printed and read by an
  agent, not hashed.
- Receipt shape (one table, four kinds, the kind named in a `kind` field):
  - `kind` (`ragdoll`, `limb`, `break`, `blast`)
  - `slot` (the fighter slot, for `ragdoll`)
  - `part` (the limb part id, for `limb`)
  - `bodies` (how many bodies the physical layer now holds)
  - `shards` (how many pieces a break admitted, for `break`)
  - `pushed` (how many bodies a blast shoved, for `blast`)
  - `gore` (`off`, `reduced`, `full`, the gore level the receipt was drawn at)
  - `muscle` (the muscle strength the body description gave, in thousandths)
  - `receipt` (the one line the physical layer prints, for example
    `physfx: bodies=14 contacts=0 us=41`)
- The verb must not accept a float, and must not write to the sim: it reads an
  authority event and asks the physical layer for a receipt. The physical layer
  is float and outside the hash, so the door verb is presentation only and
  `not_wired` is the honest answer when the physical layer is off.
- Test: `crates/endpoint/src/packs/fx.rs` tests, named
  `the_fx_verbs_answer_a_receipt_for_a_seeded_event`: all four verbs answer
  their receipt table, and `fx.break` on a wall with no shards answers
  `not_wired` rather than a zero.
- Proof: `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib fx`
  -> `the_fx_verbs_answer_a_receipt_for_a_seeded_event ... ok`.
- Kill line: if the physical layer is not on the `app` leg of `engine-game`
  (it is, since RZ4 landed it), the verbs answer `not_wired` and the row
  waits.

**Spec E2: the `body.toml` shape the door can author.**

- The body description is DATA, in a `body.toml` the editor and the door can
  write, with four parts: the bone shapes (one row per bone: name, parent,
  radius, half height), the muscle strength (per joint or one number for the
  body), the gore level (`off`, `reduced`, `full`) and the cut table (which
  joints can be cut and at what energy).
- The door verb that reads it is `body.read` in the same pack: it answers the
  parsed table and refuses a bad field with a typed code, never a zero.
- Test: `crates/endpoint/src/packs/fx.rs` tests, named
  `the_body_description_answers_its_table_and_refuses_a_bad_field`.
- Proof: `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib fx`
  -> `the_body_description_answers_its_table_and_refuses_a_bad_field ... ok`.

## For the world lane

Owner: `crates/world`. Held by the auto-making lane; this is the spec.

**Spec W1: `crates/world/src/smoke.rs`, the coarse integer smoke grid.**

- File: a new `crates/world/src/smoke.rs`, declared in
  `crates/world/src/lib.rs` next to `cells.rs` and `terrain.rs`.
- The grid is INTEGERS, in the world crate, because the sim owns it: a cell
  holds a smoke density and a fire heat, both thousandths. `UC11` says the
  grid blocks AI sight, so a cell above a threshold is opaque to the `world`
  line of sight, and the check is the `UW14` verifier.
- Shape:
  - `pub struct SmokeCell { pub density: u16, pub heat: u16 }` (thousandths).
  - `pub struct SmokeGrid { cells: Vec<SmokeCell>, width: u32, height: u32 }`.
  - `pub fn SmokeGrid::new(width: u32, height: u32) -> Self`.
  - `pub fn SmokeGrid::add(&mut self, at: (u32, u32), density: u16, heat: u16)`:
    one cell, clamped, refused above the cap.
  - `pub fn SmokeGrid::step(&mut self)`: heat decays, density follows the heat,
    spread walks to the four neighbours in a fixed order (north, east, south,
    west) so the same input gives the same bytes.
  - `pub fn SmokeGrid::blocks_sight(&self, at: (u32, u32)) -> bool`: a cell over
    the density threshold is opaque.
  - `pub fn SmokeGrid::state_bytes(&self) -> Vec<u8>`: the canonical bytes, so
    a spread replays x2.
- The grid is a world fact, so it is in the world crate and hashed with the
  world. The GPU picture of the same grid belongs to `VX5` and `VX1` and reads
  the world grid, never the other way round.
- Test: `crates/world/src/smoke.rs` tests, named
  `the_smoke_grid_spreads_and_replays_byte_equal`: one fire cell spreads to
  its neighbours over 60 steps, `state_bytes()` is equal twice, and a dense
  cell blocks sight while an empty one does not.
- Proof: `node tools/gh-check.mjs cargo test -p engine-world --lib smoke`
  -> `the_smoke_grid_spreads_and_replays_byte_equal ... ok`.
- Kill line: if the owner keeps the smoke grid out of the sim (`UC11` is
  Tier C), the file is not written and `VX1`/`VX5` draw their own.

## For the destruction lane

Owner: `crates/destruction` and `crates/editor`. Held by the bug-fix agent and
the editor lane; this is the spec.

**Spec D1: `RZ4` shard meshes, so a broken wall is not a pile of boxes.**

- File: `crates/destruction/src/shards3d.rs` (the shard mesh builder) plus a
  new `crates/destruction/src/shard_mesh.rs` (the mesh bytes).
- What is missing: the authority `BreakEvent` already carries the pieces
  (`crates/destruction/src/fracture.rs`, `shards3d.rs`), and
  `crates/game/src/break_look.rs` draws every piece as a box. The shard mesh
  is the piece geometry with an inside face and a remapped UV, so the rubble
  reads as broken stone.
- Shape:
  - `pub fn shard_mesh(shard: &Shard, material: &MaterialRow) -> ShardMesh`,
    where `ShardMesh { positions: Vec<[f32; 3]>, normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>, indices: Vec<u32> }`.
  - The inside face is the piece's own back faces, with the material's
    interior colour; the outside keeps the original UVs.
  - `pub fn shard_mesh_bytes(mesh: &ShardMesh) -> Vec<u8>`: the canonical
    bytes, so the same shard gives the same mesh twice.
- The mesh is presentation, so it is float and never hashed; the authority
  piece list stays the truth.
- Test: `crates/destruction/src/shard_mesh.rs` tests, named
  `a_shard_mesh_has_an_inside_face_and_stable_bytes`: the mesh has twice the
  triangles of the box, the inside face is not the outside face, and the
  bytes are equal twice.
- Proof: `node tools/gh-check.mjs cargo test -p engine-destruction --lib shard_mesh`
  -> `a_shard_mesh_has_an_inside_face_and_stable_bytes ... ok`.
- Kill line: if the arena wall still reads as a pile of boxes after one
  landing and the capture shows no rubble, the shard mesh work stops.

**Spec D2: `DX7` fracture authoring in the editor.**

- File: a new `crates/editor/src/fracture_tool.rs`, declared in
  `crates/editor/src/lib.rs` next to `gizmo.rs` and `commands/`.
- What the tool does: paint bonds on a mesh, preview the strength, and
  replay a break, all through the command service with undo, so an agent and
  a human author the same way.
- Shape:
  - `pub fn paint_bond(document: &mut Document, at: [f32; 3], strength: u16) -> Result<EditReceipt, EditorError>`:
    one bond, strength in thousandths, refused above the bond cap.
  - `pub fn preview_break(document: &Document, at: [f32; 3]) -> BreakPreview`:
    the pieces a hit there would make, from the same `fracture.rs` the sim
    uses, so the preview cannot lie.
  - `pub fn replay_break(document: &Document, at: [f32; 3]) -> BreakPreview`:
    the same preview with the break stepped, so the editor shows the motion.
  - Every edit goes through the command service with undo, like every other
    editor edit.
- Test: `crates/editor/src/fracture_tool.rs` tests, named
  `the_fracture_tool_paints_bonds_previews_a_break_and_undoes`: a painted
  bond is in the document, the preview lists pieces, undo takes the bond back.
- Proof: `node tools/gh-check.mjs cargo test -p engine-editor --lib fracture_tool`
  -> `the_fracture_tool_paints_bonds_previews_a_break_and_undoes ... ok`.
- Kill line: if the editor cannot reach the command service for a mesh
  edit, the tool waits for that and does not land a second write path.

## The bevy_hanabi extra check: what is portable without the compute layer

The donor is a CPU and vertex-fragment implementation on our side, and the
compute half is ours to write. Concretely, from the live read on 2026-10-20:

- **Portable without any GPU compute** (shape only, our own Rust): the effect
  descriptor (spawner plus init plus update plus gradient),
  `src/asset.rs`, `src/spawn.rs` and `src/gradient.rs`, which our
  `crates/render/src/vfx_hanabi.rs` already ports as a CPU layer; the batch
  shape (`src/render/batch.rs`) and the effect cache (`src/render/effect_cache.rs`),
  which our file also ports; and the billboarding, the two-frame flipbook
  blend and the velocity stretch, which live in `src/render/vfx_render.wgsl`
  and are already in our `crates/render/src/shaders/particle.wgsl` and
  `crates/render/src/particles_gpu.rs`.
- **Needs our own GPU work** (nothing to copy, the shape only): the compute
  update (`src/render/vfx_update.wgsl`), the compute init
  (`src/render/vfx_init.wgsl`), the GPU merge sort (`src/render/sort.rs` and
  `src/render/vfx_sort.wesl` with its fill, copy and merge passes), the
  prefix sum (`src/render/vfx_prefix_sum.wesl`), the indirect draw
  (`src/render/vfx_indirect.wesl`) and the aligned buffer bookkeeping
  (`src/render/aligned_buffer_vec.rs`). We write our own WGSL for all of it.
- **The honest cost**: `PHX-1` stays **L**, not M. The donor gives us the
  shape of the passes and the layout of the buffers, but every line of the
  compute path is ours, and the sort and the ping-pong buffers are the hard
  part. The cheap half of the row is the CPU pool plus the flipbook draw,
  which is what `LF6` and `LF7` need first, and that is S to M. So the row
  should be read as two landings: the draw first (M, this is what the arena
  needs to show dust and blood), then the GPU compute and sort (L, only when
  the particle count is big enough to need it).

## What the tree already has (checked 2026-10-21, so nobody re-mines it)

- `crates/render/src/particles_gpu.rs`: a seeded CPU pool, four emitter kinds
  (dust, flame, sparks, blood), the flipbook pair, the soft fade
  (`soft_fade`, `:962`), the velocity stretch, `hanabi_dispatch` (`:764`) and
  `stage_pool` (`:1353`).
- `crates/render/src/particle_draw_gpu.rs`: the staged draw bytes, the
  instance layout, the flipbook pipeline and the scene-depth binding
  (`:40`).
- `crates/render/src/vfx_hanabi.rs`: the CPU hanabi layer, a port of the CPU
  half of bevy_hanabi.
- `crates/render/src/decals.rs`: deferred blood decals, depth reject, the
  wet-to-dry roughness curve, the pool and `upload_instances` (`:917`).
- `crates/render/src/fluids_gpu.rs`: a seeded CPU pool of blood and water
  droplets, no grid and no compute.
- `crates/render/src/crowd_impostor.rs`, `crowd_inst.rs`, `crowd_seats.rs`,
  `crates/render/src/shaders/crowd_vat.wgsl` and `crates/assets/src/vat.rs`:
  the crowd staging and the VAT bake.
- `crates/physfx/src/fx/`: the float Rapier world with ragdoll spawn, hinge
  joints, the PD motor that fades after a hit, blast, impact, sever and
  tether, and the `RagdollReceipt` with its one printed line.
- `crates/physics/src/joint_limits.rs` and `crates/physics/src/ragdoll.rs`:
  the per-joint degree table (spine 20/35, head 35/45, arm 85/70, hip 40/30,
  knee and elbow hinge 145) and the spawn that reads it. `PX9` wants the
  contacts and the torque motors on top of that table.
- `crates/destruction/src/fracture.rs`, `shards3d.rs`, `impact_table.rs`,
  `debris_lod.rs`, `bond_grade.rs`, `structure.rs`: the runtime Voronoi
  fracture, the shard list, the impact table, the debris LOD, the graded bonds
  and the integer load leg.
- `crates/physics/src/cloth.rs`: the CPU verlet cloth, the cape drape.
