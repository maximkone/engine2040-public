# Physics, VFX and destruction: the donor hunt (2026-10-20)

## The answer first

VFX is planned in this engine, and a lot of it is already written, but the
picture half is not wired up. `docs/FOCUS.md` (HARDEN) names gore/VFX,
networked physics, destruction, ballistics/siege and a 20k GPU crowd;
`docs/plan/render-look.md` puts "GPU particles vs depth (RP2) -> decals (RP3)"
into the frame graph and owns `crates/render/src/particles_gpu.rs` and
`crates/render/src/decals.rs`; `docs/plan/body-and-break.md` owns the float
`crates/physfx` layer and the rule that blood, dust, cloth tear and GPU debris
are presentation, never hashed. What exists today: a seeded CPU particle pool
with dust, flame, sparks and blood kinds, a flipbook draw state and a soft
depth fade (`crates/render/src/particles_gpu.rs`,
`crates/render/src/particle_draw_gpu.rs`), a CPU hanabi-style effect layer
(`crates/render/src/vfx_hanabi.rs`, a port of the CPU half of bevy_hanabi
only), deferred blood decals with depth reject (`crates/render/src/decals.rs`),
a CPU blood-and-water droplet pool (`crates/render/src/fluids_gpu.rs`),
octahedral crowd impostors and VAT (`crates/render/src/crowd_impostor.rs`,
`crates/render/src/crowd_vat.wgsl`, `crates/assets/src/vat.rs`), and in
`crates/physfx` a float Rapier world with ragdoll, hinge joints, per-joint PD
motors that fade after a hit, blast, impact, sever and tether
(`crates/physfx/src/fx/`). What is missing: (1) no GPU compute particle
simulation and no GPU sort, because `crates/render/src/vfx_hanabi.rs` and
`crates/render/src/shaders/particles_hanabi.wgsl` are CPU and vertex/fragment
only; (2) the live frame draws neither particles nor decals, because
`ForwardLook::live` in `crates/render/src/present/gpu_present/submit.rs` sets
`particles: false` and only the probe path sets it
(`crates/render/src/present/gpu_present/live.rs:1538`), while the live frame
runs forward plus blit only (`live.rs:1308` and `live.rs:1376`); the soft
depth fade reads a zeroed 64-byte block
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
reads "our own renderer", so that gate can never pass as written.

## Ranked table

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | The GPU half of a hanabi-style particle system: compute update, GPU merge sort, indirect draw. Our tree has only the CPU batch port | `djeedai/bevy_hanabi`, https://github.com/djeedai/bevy_hanabi, read live 2026-10-20 | MIT OR Apache-2.0 (`LICENSE-MIT` read live, 1070 bytes; `LICENSE-APACHE2` in root) | `NS11` (also `LF6`, `RL52`) | `src/render/mod.rs` (315191 B, the compute renderer), `src/render/sort.rs` (63551 B, the GPU merge sort), `src/render/vfx_update.wgsl` (7740 B), `src/render/vfx_init.wgsl` (7992 B), `src/render/vfx_sort.wesl` (18987 B), `src/render/vfx_indirect.wesl` (4571 B), `src/render/vfx_prefix_sum.wesl` (1831 B), `src/render/aligned_buffer_vec.rs` (46136 B). We write our own WGSL and our own buffers | L | Kill if the CPU pool of 4096 with the flipbook draw already holds the 1.0 ms particle budget on the RTX 3050 and `LF6`/`LF7` captures pass, then the GPU compute path is not built | PORT (CORRECTION: the donor was cited on 2026-10-11 and 2026-10-12, but only the CPU half was ported) |
| 2 | A Rust plus wgpu grid fluid: D2Q9 LBM on GPU compute, plus PBD cloth, plus a wasm build script | `jinleili/simuverse`, https://github.com/jinleili/simuverse, read live 2026-10-20 | MIT (`LICENSE.MIT` read live, 1066 bytes, Jinlei Li 2023) | `VX5` (also `VX10`) | `simuverse/src/fluid/d2q9_node.rs` (12604 B, the LBM compute node), `simuverse/src/fluid/fluid_simulator.rs` (8615 B, the curl and particle update nodes), `simuverse/src/fluid/lattice.rs` (3479 B), `simuverse/src/pbd/cloth.rs` (14024 B), `simuverse/src/pbd/gen_cloth_constraints.rs` (11550 B), `build-wasm.sh`. Shape only, our own code | M | Kill if the grid cannot hold the VX10 budget of 1.0 ms on the RTX 3050, or if `crates/render/src/fluids_gpu.rs` stays a CPU droplet pool and the row is parked by the owner, then nothing is ported | PORT |
| 3 | A GPU Eulerian fire and smoke solver: 128 voxel cube, vorticity confinement, Jacobi pressure, blackbody ray march | `olliep24/firesim`, https://github.com/olliep24/firesim, read live 2026-10-20 | No license file in the repo root (root listing read live has no LICENSE, and `Cargo.toml` has no `license` field) so there is no grant: read only, write our own code | `VX10` (also `VX5`) | `src/advect_scalars.wgsl` (2485 B), `src/advect_velocity.wgsl` (2173 B), `src/compute_divergence.wgsl` (2480 B), `src/compute_pressure.wgsl` (2784 B), `src/subtract_pressure_gradient.wgsl` (2723 B), `src/add_vorticity_confinement_force.wgsl` (3343 B), `src/compute_curl.wgsl` (2509 B), `src/render_shader.wgsl` (6624 B), `src/compute_step.rs` (3149 B, the ping-pong compute step), `src/ping_pong.rs` (1159 B) | M | Kill if no fire cell ever appears in the arena and the owner keeps `VX5` frozen, then the fire solver is not written | RESEARCH-ONLY (no license, so no bytes) |
| 4 | Runtime fracture authoring and shard meshes: plane slicer, fragmenter, constrained triangulation, inside material, edge UV remap | `dgreenheck/OpenFracture`, https://github.com/dgreenheck/OpenFracture, read live 2026-10-20, file paths re-read live 2026-10-21 | MIT (`LICENSE.md` read live, 1073 bytes, Daniel Greenheck 2021) | `RZ4` (also `DX7`) | `Runtime/Scripts/Fracture.cs` (10257 B, the trigger and re-fracture options), `Runtime/Scripts/Slicers/PlaneSlicer.cs` (2352 B), `Runtime/Scripts/Fragment/MeshSlicer.cs` (16645 B), `Runtime/Scripts/Fragment/ConstrainedTriangulator.cs` (29073 B), `Runtime/Scripts/Fragment/Fragmenter.cs` (12084 B), `Runtime/Scripts/Fragment/FragmentData.cs` (11104 B), `Runtime/Scripts/Slice.cs` (5222 B), `Runtime/Scripts/Prefracture.cs` (5363 B). C# to Rust, shape only | M | Kill if after one landing `RZ4` still draws every chunk as a box and the capture shows no rubble, then the shard mesh work stops | PORT |
| 5 | Surface reconstruction from a particle pool: density map plus marching cubes plus halfedge cleanup | `InteractiveComputerGraphics/splashsurf`, https://github.com/InteractiveComputerGraphics/splashsurf, read live 2026-10-20 | MIT (`LICENSE` read live, 1078 bytes, Fabian Loschner 2020) | no row, propose a new row: "LF7b blood and water pool surface from the particle pool" | `splashsurf_lib/src/marching_cubes.rs` (14017 B), `splashsurf_lib/src/density_map.rs` (31707 B), `splashsurf_lib/src/halfedge_mesh.rs` (22318 B), `splashsurf_lib/src/postprocessing.rs` (33000 B), `splashsurf_lib/src/neighborhood_search.rs` (29060 B), `splashsurf_lib/src/reconstruction.rs` (7108 B), `splashsurf_lib/src/sph_interpolation.rs` (10865 B) | S | Kill if the pool stays a sprite-and-decal presentation and no row ever asks for a fluid surface, then the crate is not added | PORT |
| 6 | A fixed-point rigid body engine in Rust, deterministic on every machine | `richardhozak/fppe`, https://github.com/richardhozak/fppe, read live 2026-10-20 | MIT OR Apache-2.0 (`LICENSE-MIT` read live, 1036 bytes; `Cargo.toml` license field read live 2026-10-21) | `C15` | `src/lib.rs` (34622 B, the `I32F32` unit type and the fixed-point body solver), `src/vec.rs` (4487 B). Read for the fixed-unit shape and the `fixed` plus `cordic` choice | S | Kill if the port of `fixed3d` already prices the integer solver, then this is reading only. Note the donor README says the library is incomplete with probable bugs, so it can never be the oracle itself | RESEARCH-ONLY |
| 7 | Safe Rust bindings to Jolt Physics 5.3.0 over JoltC, with a body interface, narrow phase and physics system | `SecondHalfGames/jolt-rust`, https://github.com/SecondHalfGames/jolt-rust, read live 2026-10-20 | MIT OR Apache-2.0 (`LICENSE-MIT` read live, 1082 bytes, Second Half Games 2024; `LICENSE-APACHE` in root) | `RZ10` | `crates/rolt/src/body_interface.rs` (3952 B), `crates/rolt/src/narrow_phase.rs` (7513 B), `crates/rolt/src/physics_system.rs` (5724 B), `crates/rolt/src/traits.rs` (17358 B), `crates/rolt/src/reference.rs` (3816 B), `crates/joltc-sys` | M | Kill if the mirror run of `tools/physfx-backends.mjs` already prints both rows and Rapier holds the `RZ9` bar at every tier, then no Rust Jolt binding is built. Also kill on the first day without a C++ toolchain on the runner, because JoltC must compile | PORT |
| 8 | Per-instance animation offset for instanced meshes, and a vertex-animation texture bake | `codewriter-packages/Mesh-Animation`, https://github.com/codewriter-packages/Mesh-Animation, read live 2026-10-21 | MIT (`LICENSE.md` read live, 1071 bytes, Vlad Vanifatov 2020) | `QA10` (also `VG9`, `RL54`) | `Runtime/Scripts` (the per-instance override path) and `Runtime/Shaders` (the vertex texture read). Shape only: one phase offset per instance in our instance-step buffer | S | Kill if `crates/render/src/crowd_inst.rs` cannot bind its seats in the presenter and `QA10` never gets a capture, then the per-instance offset is not added | RESEARCH-ONLY (Unity host) |
| 9 | Long-range attachment constraints, bending constraints, SDF collision and a spatial hash for cloth | `vitalight/Velvet`, https://github.com/vitalight/Velvet, read live 2026-10-20 | MIT (`LICENSE.txt` read live, 1065 bytes, vitalight 2022) | `E81` | `Velvet/SpatialHashGPU.cu` (5059 B), `Velvet/SpatialHashGPU.hpp` (4144 B), `Velvet/SpatialHashCPU.hpp` (2650 B), `Velvet/Common.cuh` (2152 B), `Velvet/ParticleInstancedRenderer.hpp` (2189 B). Shape only, in our own Q16 verlet cloth | M | Kill if `crates/physics/src/cloth.rs` holds the 0.5 ms bar for hair and rope without long-range attachments, then nothing is ported | RESEARCH-ONLY (CUDA, no web target) |
| 10 | The recovered rules of a shipped structural destruction game: support removal, per material break behaviour, fracture response | `DevelopmentStatus/rfg-physics-sandbox`, https://github.com/DevelopmentStatus/rfg-physics-sandbox, read live 2026-10-20 | MIT (`LICENSE` read live, 1089 bytes, RFG Physics Sandbox contributors 2026) | `DX17` | `Docs/destruction_reference.md` and `Research/evidence_registry.md` (26330 B). Rules text only, no code | S | Kill if our own integer load leg in `crates/destruction/src/structure.rs` already covers every rule the doc lists, then the doc is not read again | RESEARCH-ONLY |
| 11 | Crowd as one tick for the whole crowd: group decisions, distance LOD tiers, vertex-animation clips, batch spawn | `DigiLogicLabs/dll-mass-units`, https://github.com/DigiLogicLabs/dll-mass-units, read live 2026-10-20 | Apache-2.0 (`LICENSE` 11350 B in root; README states Apache-2.0) but the host is Unreal Engine 5, so research only | `RL54` (also `VG4`, `VG9`) | `Source` and `docs/INTEGRATION_GUIDE.md`. Shape only | S | Kill if `crates/render/src/crowd_inst.rs` and `crates/render/src/crowd_seats.rs` already seat 20k spectators inside the 1.5 ms bar, then no crowd LOD rewrite is started | RESEARCH-ONLY (Unreal host) |
| 12 | A cell based fire model with fuel, heat, burn stages and wind, written as a streaming integer grid | `MadeInShineA/forest-fire-simulation`, https://github.com/MadeInShineA/forest-fire-simulation, read live 2026-10-20 | No license file in the repo root (root listing read live has no LICENSE) so there is no grant: read only, write our own code | `UC11` | `forest_fire_simulation_model.md` (8556 B, the cell model), `src` (the Rust simulation core). Rules text only, our own integer grid in `crates/world/src/smoke.rs` | S | Kill if the owner keeps the smoke grid in the world crate out of the sim, then the model is not read again | RESEARCH-ONLY (no license) |

## Rejections

- `ext-sakamoro/ALICE-Physics`, https://github.com/ext-sakamoro/ALICE-Physics,
  read live 2026-10-20: 128-bit fixed point, bit-exact replay, forkable
  snapshots, the right shape for `C15`, but the root holds `LICENSE-AGPL` plus
  `LICENSE-COMMERCIAL.md`, and `docs/research/finish-line-2026-10-17/02-github.md`
  already lists it as AGPL-3.0 and do not copy. REJECT.
- `InteractiveComputerGraphics/SPlisHSPlasH`, https://github.com/InteractiveComputerGraphics/SPlisHSPlasH,
  read live 2026-10-20: MIT, the best SPH reference there is, but `PX32`
  already names its donor (the flax-mcp `SphSolver.cs`), and the Rust binding
  path `Project-Splines/SPlisHSPlasH` 404s today, so there is no Rust donor to
  point at. REJECT as a donor, keep as a solver reference.
- `dgreenheck/ez-tree`: the same author as row 4, and it is already closed in
  `docs/archive/2026-10-10/plans/steals-closed.md` for tree scattering. Not
  re-mined.
- `PaulRobello/par-particle-life`, https://github.com/paulrobello/par-particle-life,
  read live 2026-10-20: GPU particle life with spatial hashing, but the
  license is AGPL-3.0 or later, and it is a rule-set toy rather than an
  emitter. REJECT on the copyleft.
- `sharpen3d/openvat`, https://github.com/sharpen3d/openvat, read live
  2026-10-20: a good VAT encoder, but GPL-3.0 and our VAT bake already landed
  in `crates/assets/src/vat.rs` under `GS20`. REJECT.
- `keijiro/HdrpVatExample`, https://github.com/keijiro/HdrpVatExample, read
  live 2026-10-20: public domain and shows soft, rigid, fluid and sprite VAT,
  but our VAT bake is already in the tree, so the remaining gap is the draw
  path, not the bake. REJECT.
- `dgreenheck/OpenFracture` for `DX5`: `DX5` is already ticked and
  `crates/destruction/src/fracture.rs` holds the runtime Voronoi cut, so the
  donor is aimed at `RZ4` and `DX7` instead.
- `NVlabs` and `nvpro` VFX-Jolt: `https://github.com/nvpro/VFX-Jolt` 404s
  today and a GitHub repo search for VFX-Jolt returns only
  `ljramones/DynamisPhysics` (0 stars, Java). No NVIDIA VFX-Jolt repo to read.
  Note the NVIDIA RTX SDKs stay excluded.
- `Audit of the rows named in the brief`: `DX13`, `PX10`, `PX16`, `PX17`,
  `GS46` and `QW1` are all ticked or archived, and `board-slice.mjs` refuses
  them, so no donor is aimed at them. `QW4`, `QW5`, `QW6`, `QW9` and `QW10`
  are web bundle and size rows with no physics or VFX content. `PX9`, `RZ4`,
  `RZ6`, `RZ10`, `LF6`, `LF7`, `NS11`, `VX5`, `VX10`, `UC11`, `DX6`, `DX7`,
  `DX17`, `E81`, `QA10`, `RL54`, `QA8`, `RL52`, `RL57`, `AM18` and `CT25`
  are open and are the targets above.
- `box3d-rust` and `fixed3d`: named in the brief as already mined and named
  in `docs/research/finish-line-2026-10-17/02-github.md` rows 1 and 5. Row 6
  adds the only fresh third option, and `C15` still owns the spike.

## Top 3

1. `djeedai/bevy_hanabi`, https://github.com/djeedai/bevy_hanabi, MIT OR
   Apache-2.0. Closes `NS11` (and unblocks `LF6`, `LF52`). Cost L. The GPU
   compute, sort and indirect half is still unported.
2. `jinleili/simuverse`, https://github.com/jinleili/simuverse, MIT. Closes
   `VX5` with a Rust and wgpu GPU grid fluid, and gives `VX10` its check
   target. Cost M.
3. `dgreenheck/OpenFracture`, https://github.com/dgreenheck/OpenFracture, MIT.
   Closes `RZ4` with real shard meshes and inside material, and gives `DX7`
   its authoring shape. Cost M.
