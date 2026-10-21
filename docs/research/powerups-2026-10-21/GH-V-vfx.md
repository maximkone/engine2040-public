# GH-V — Game VFX donors a wgpu renderer can port (2026-10-21)

## Answer

The cheap VFX wins for engine2040 are shapes, not engines. Four permissive donors carry
almost everything we still miss: a GPU-resident particle pool in Rust + wgpu that keeps its
own alive/dead lists and draws indirectly (`kevinraymond/fosfora`, MIT OR Apache-2.0), a WebGPU
particle step that never reads back to the CPU (`m2-md/webgpu-compute-particles`, MIT), a WGSL
particle update that already knows emitters, flow maps, attractors and sprite sheets, plus the
hit-burst recipe (`BabylonJS/Babylon.js`, Apache-2.0), and a decal / ribbon-trail pair
(`OGRECave/ogre-next`, MIT). A screen-space shockwave is one small MIT file
(`pmndrs/postprocessing`). Our tree already owns the CPU half: a seeded pool capped at 60,000
(`crates/render/src/particles.rs:19`), a hanabi-style CPU effect layer
(`crates/render/src/vfx_hanabi.rs`), deferred blood decals (`crates/render/src/decals.rs`), a
12-frame trail ring (`crates/anim/src/trails.rs`). What is missing is exactly what the donors carry: no particle compute pass exists, the
live frame draws no particles and no decals, the weapon-trail strip is never drawn, and there is
no shockwave, no dust flow field, no projected decal on walls and no screen-space burst.
We vendor nothing: every row below is a shape we re-implement in our own WGSL and Rust (D5).
Two lanes stay unclosed: screen-space fluid rendering (`kuriishi/PositionBasedFluids` has no
LICENSE file, so there is no grant) and the smoke/fluid rows `VX5`/`VX10`, which are still frozen
by owner decision D7.

### Tree check (OBSERVED, `git -C C:\engine2040 grep`, 2026-10-21)

- `ComputePipeline` hits 14 files under `crates/render/src/` (gtao, fog, decals, sky, lens,
  material_layer, gpu_cull, baked_gi, aa_scale, post) and **zero** in `particles.rs`,
  `particles_gpu.rs`, `particle_draw_gpu.rs`, `vfx_hanabi.rs` → a GPU particle sim is a real gap.
- `trail_strip_verts` is called only from its own test (`crates/render/src/draw.rs:1846`) → the
  LF14 live strip draw is missing, the geometry exists.
- `git grep -i shockwave -- crates/` hits only `crates/audio/src/gunfire.rs` (sonic boom) → a
  visual shockwave is a real gap.
- `git grep -i "flow_map|attractor|ring_emitter|sub_emitter|spatial_hash" -- crates/` = 0 hits.
- `crates/render/src/decals.rs` (71 kB) already has the deferred blood decal, the atlas, the
  depth reject and `BLOOD_ROUGH_WET`/`BLOOD_ROUGH_DRY` → decal rows must add layers, not the
  first decal.
- `crates/render/src/particles.rs:19` = `MAX_PARTICLES_60K = 60_000` (OBSERVED). The 2026-10-20
  physics/VFX file says "4096" for the CPU pool; that number is wrong, this one is read off the
  file today. (CORRECTION.)

## Ranked table

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | A GPU-resident particle pool in Rust + wgpu: ping-pong storage buffers, alive/dead index lists, indirect draw, an obstacle texture the particles bounce off, a spatial-hash neighbour grid, and a 16-bit counting sort for depth order. Our pool is CPU-side; this is the shape of the GPU one. | `kevinraymond/fosfora`, https://github.com/kevinraymond/fosfora, read live 2026-10-21 (58 stars, OBSERVED in the repo search) | MIT OR Apache-2.0 — root `LICENSE-MIT` (1075 B, SHA 0b1d0141) and `LICENSE-APACHE` (11229 B), both read live 2026-10-21 (OBSERVED) | `NS11` (also `LF6`, `VX10`) | `crates/fosfora-app/src/gpu/particle/system.rs` (header reads "GPU compute particle system with ping-pong storage buffers, alive/dead index lists, and indirect draw for GPU-driven rendering", OBSERVED); `crates/fosfora-app/src/gpu/particle/obstacle.rs` (obstacle texture, alpha threshold, bounce / stick / flow-around, OBSERVED); `crates/fosfora-app/src/gpu/particle/spatial_hash.rs` (count / scatter / prefix-sum grid, target ~16 particles per cell, OBSERVED); `crates/fosfora-app/src/gpu/particle/splat_sort.rs` (16-bit counting sort, 4 dispatches, OBSERVED). We write our own WGSL in `crates/render/src/shaders/`. | L | Kill if the CPU pool of 60,000 with the flipbook draw already holds the 1.0 ms particle budget on the RTX 3050 and LF6/LF7 captures pass — then no GPU particle compute is written. Also kill if the pool needs a per-frame readback, because VX10 wants seeded sim bytes identical twice and a per-frame readback would sit in the frame. | PORT |
| 2 | A WebGPU particle system that stays on the GPU: one storage buffer, a WGSL compute step that updates position and velocity in place, the vertex stage reads the same buffer, and only 48 bytes of uniform go up per frame. Seeded init (mulberry32), a bounds guard for the last workgroup, dispatch sized in workgroups. This is the browser half of NS11. | `m2-md/webgpu-compute-particles`, https://github.com/m2-md/webgpu-compute-particles, read live 2026-10-21 | MIT — `LICENSE` (1071 B, SHA 6658874627ea50949e9c99cf44a4853cc88935d9, "Copyright (c) 2026 Mustafa Morbel"), read live 2026-10-21 (OBSERVED) | `NS11` (also `VX10`, and the web leg of `WEB5`) | `src/sim.wgsl.ts` (the compute step, in-place update, bounds guard, bounce, OBSERVED); `src/gpu-particles.ts` (the buffer carries the STORAGE, COPY_DST and COPY_SRC usages, so the render pass reads the same buffer the compute pass writes, OBSERVED); `src/particles.ts` (seeded init, mulberry32, OBSERVED); `src/dispatch.ts` (workgroup count, `Math.ceil`, OBSERVED). Shape only: our own WGSL and our own buffers. | S | Kill if the browser build cannot give a storage buffer vertex-stage read access on the WebGPU adapter we support, or if the arena never needs more live particles than the CPU pool holds inside the 1.0 ms budget — then the compute path is not written. | PORT |
| 3 | The particle update math a game needs: emit counts, life and size ranges, colour gradients, angle and angular-speed gradients, a sprite-sheet cell index, noise, a flow map and up to N attractors. A flow map is how dust and smoke drift; an attractor with a positive or negative strength is the swirl of a shockwave. | `BabylonJS/Babylon.js`, https://github.com/BabylonJS/Babylon.js, read live 2026-10-21 | Apache-2.0 — `license.md` read live 2026-10-21 ("Apache License Version 2.0", OBSERVED) | `LF6` (also `NS11`) | `packages/dev/core/src/ShadersWGSL/gpuUpdateParticles.compute.fx` (the WGSL; the `FLOWMAP`, `ATTRACTORS`, `ANIMATESHEET`, colour-gradient and size-gradient blocks are all visible in the file, OBSERVED); `packages/dev/core/src/Particles/flowMap.ts` (a flow field that pushes particles along a direction, OBSERVED); `packages/dev/core/src/Particles/attractor.ts` (force falls off with distance squared, sign of strength picks pull or push, OBSERVED). Our own WGSL in `crates/render/src/shaders/`. | M | Kill if the CPU pool with the flipbook draw holds the 1.0 ms particle budget and LF6's capture shows dust and sparks near the blade — then the compute update is not written. | PORT |
| 4 | A particle that spawns other particles: a sub emitter is attached to a particle and fires when that particle dies, optionally handing over part of the parent's velocity. With the emitter shape set (point, cone, box, cylinder, sphere, hemispheric, mesh) this is the whole hit-spark recipe: on a hit, one short-lived particle at the contact point dies and a cone of sparks is born from it. | `BabylonJS/Babylon.js`, https://github.com/BabylonJS/Babylon.js, read live 2026-10-21 | Apache-2.0 (same `license.md`, read live 2026-10-21, OBSERVED) | `LF6` (also `NS11`) | `packages/dev/core/src/Particles/subEmitter.ts` (`SubEmitterType.END` = "Created when the particle dies", `inheritedVelocityAmount`, OBSERVED); `packages/dev/core/src/Particles/EmitterTypes/coneParticleEmitter.ts` and `pointParticleEmitter.ts` (the shapes, OBSERVED). Our own spawn code in `crates/render/src/particles.rs` and the spawn hunk in `crates/game/src/fighter_look.rs`. | S | Kill if LF6's capture after a block already shows spark pixels near the blade from a direct spawn — then the two-stage burst is not built. | PORT |
| 5 | A decal built as a mesh and projected onto the triangles of the source mesh, with a size limit, an angle limit and a per-decal normal, so blood and impact marks sit on the geometry instead of on a screen-space box. | `BabylonJS/Babylon.js`, https://github.com/BabylonJS/Babylon.js, read live 2026-10-21 | Apache-2.0 (same `license.md`, read live 2026-10-21, OBSERVED) | `QL11` (also `LF7`) | `packages/dev/core/src/Meshes/Builders/decalBuilder.pure.ts` (builds the decal mesh from the source mesh, position, normal, size and angle, OBSERVED). Our own code in `crates/render/src/decals.rs`. | M | Kill if the deferred screen-space decal already passes LF7 (decal pixels under the victims, decal pass under 0.5 ms) and the arena has no wall or prop that needs a projected decal — then no decal mesh is built. | PORT |
| 6 | A decal as a first-class scene object: diffuse, normal-map and emissive layers, each globally switchable so an unused layer costs nothing, a decal-space transform, and per-decal metalness and roughness (a wet blood patch is glossy, dry blood is matte). | `OGRECave/ogre-next`, https://github.com/OGRECave/ogre-next, read live 2026-10-21 | MIT — root `COPYING` ("OGRE … is made available under the MIT License", Copyright Torus Knot Software) and the same MIT header in `OgreDecal.h` / `OgreDecal.cpp`, read live 2026-10-21 (OBSERVED) | `LF7` (also `QL11`) | `OgreMain/include/OgreDecal.h` and `OgreMain/src/OgreDecal.cpp` (per-layer diffuse / normal / emissive with global switches, `mIgnoreDiffuseAlpha`, per-decal `mMetalness` / `mRoughness`, SHA f065ba9c47ee82395bebe8bd96c6ea92927c723f, OBSERVED). Our own decal atlas entries in `crates/render/src/decals.rs`. | M | Kill if LF7's capture already shows blood going from wet to dry with the roughness we already carry (`BLOOD_ROUGH_WET` 0.15, `BLOOD_ROUGH_DRY` 0.65) — then the extra layers are not added. | PORT |
| 7 | Screen-space deferred decals in OpenGL: the decal box is drawn with the decal view-projection, the fragment stage rebuilds the world position from the depth buffer, tests it against the decal box, and writes albedo, a tangent-space normal and roughness. This is the maths our decal file needs if it ever goes back to a screen-space path or wants normal-mapped impact marks. | `diharaw/deferred-decals`, https://github.com/diharaw/deferred-decals, read live 2026-10-21 | MIT — `LICENSE` (1073 B, SHA f9661c7e34f08f1cee3988c622d8a49dd08fe012, "Copyright (c) 2019 Dihara Wijetunga"), read live 2026-10-21 (OBSERVED) | `LF7` | `src/shader/decals_vs.glsl` (the decal-space to clip transform, SHA c0a20221d7fb6a4edd8bb4d63a0adf99d32e53fb) and `src/shader/decals_fs.glsl` (`world_position_from_depth`, the TBN from the G-buffer tangent, the normal-map write, SHA d4e71c9200688c2af6ea83a860fc182d216994f2), both read live 2026-10-21 (OBSERVED). Our own WGSL. | S | Kill if the deferred decals we already ship pass LF7 on their own — then this file is not read again; it only pays if we add normal-mapped impact marks or need the depth-rejection maths spelled out. | PORT |
| 8 | A ribbon trail: a chain of nodes is watched, every frame the trail takes a segment from the chain, the segments become a camera-facing ribbon with per-segment texture coordinates, and old segments fade and are recycled. | `OGRECave/ogre-next`, https://github.com/OGRECave/ogre-next, read live 2026-10-21 | MIT (same root `COPYING` and per-file MIT header, read live 2026-10-21, OBSERVED) | `LF14` | `OgreMain/include/OgreRibbonTrail.h` (a BillboardChain that "automatically leaves a trail behind one or more Node instances", OBSERVED) and `OgreMain/src/OgreRibbonTrail.cpp` (the segment chain and the controller that samples the node each frame, SHA 6c3ef550bcf78152b4d3f5068996e529f70a15c4, OBSERVED). Our own in `crates/anim/src/trails.rs` plus the missing present/** strip draw. | S | Kill if the mid-swing capture shows the strip from the existing `trail_strip_verts` output — then the Ogre ribbon shape is not ported; what LF14 lacks is the live draw, not the geometry. | PORT |
| 9 | The old-school pooled particle vocabulary from a 2002-era engine: a manager owns many particle systems and steps them on a frame counter, each system has its own emitters, and the renderers are named after what they draw — a sparkle renderer is a camera-facing spark with a birth radius and a death radius, a ring emitter is an expanding ring, which is a shockwave. | `panda3d/panda3d`, https://github.com/panda3d/panda3d, read live 2026-10-21 | BSD-3-Clause — root `LICENSE` ("Copyright (c) 2008, Carnegie Mellon University", the 3-clause BSD text, SHA d23c5832664b476e90998de8e281330797201f64), read live 2026-10-21 (OBSERVED) | `LF6` (also `NS11`: a per-system cap is the per-event budget) | `panda/src/particlesystem/particleSystemManager.h` (attach / detach systems, `do_particles`, frame stepping, SHA 5bfbe09ec79c7bed25c10546e18c10487b7d02b4, OBSERVED); `panda/src/particlesystem/sparkleParticleRenderer.h` (camera-facing spark, birth and death radius, SHA dc6ed4b95a987dadb8b46d6af94907085a28fc9d, OBSERVED); `panda/src/particlesystem/ringEmitter.cxx` (the expanding ring). Our own code in `crates/render/src/particles.rs`; no C++ is vendored. | S | Kill if the hit spark pool and the ring shockwave never appear in a capture and LF6 stays at dust and sparks only — then the ring emitter is not written. | PORT |
| 10 | A screen-space shockwave: the world point is projected to a screen centre, the fragment stage pulls the UV in along the radial direction inside a moving band, with a cosine wave, a distance decay and an amplitude, aspect corrected so the ring stays round. | `pmndrs/postprocessing`, https://github.com/pmndrs/postprocessing, read live 2026-10-21 | MIT — root `LICENSE.md` ("Copyright © 2015 Raoul van Rüschen", MIT terms), read live 2026-10-21 (OBSERVED) | no row, propose a new row: **LF6b hit shockwave** (a screen-space ring on a heavy hit) | `src/effects/ShockWaveEffect.js` (the uniforms: center, radius, maxRadius, waveSize, amplitude, cameraDistance, size; it also tracks the world position of the wave, SHA f1e30275694fa477ef71d82a23ec3da12866de42, OBSERVED) and `src/effects/glsl/shock-wave.frag` (the band maths: aspect correction, cosine wave, `max(extent - distance*distance, 0) / extent` decay, SHA 6efeaa51d9d26697c85a4a908f78d815bc67f0d, OBSERVED). Our own WGSL in `crates/render/src/post.rs`. | S | Kill if the arena never has a heavy hit worth a screen-space ring, or if the ring costs more than 0.2 ms of post time on the reference GPU — then no shockwave pass is added. | PORT |
| 11 | Screen-space fluid rendering: particle depth and thickness are rendered first, the surface is rebuilt from the depth buffer and shaded with absorption and foam, so a blood pool or a water trough reads as a liquid surface with no mesh. | `kuriishi/PositionBasedFluids`, https://github.com/kuriishi/PositionBasedFluids, read live 2026-10-21 | **No LICENSE file** in the repo root — the listing read live 2026-10-21 shows only CMakeLists.txt, README.md, README_zh.md, imgui.ini, include, resource, showcase, src — so there is no grant. Read it, write our own code. (OBSERVED) | `VX5` (also `VX10`) — both frozen by owner decision D7 ("until the first real Bevy frame lands", SPRINT-12); the Bevy viewer was deleted, so that gate can never pass as written (DERIVED from SPRINT-12 and the 2026-10-20 physics/VFX file) | `src/` (the OpenGL screen-space fluid passes; the van der Laan surface reconstruction the README names). Nothing ported now. | M | Kill if QL11 keeps blood pools in the RL49 ground map as its own text says, then no screen-space fluid is written; also kill if the owner never unpicks VX5 / VX10 from the D7 freeze. | RESEARCH-ONLY (no licence, and both target rows are frozen) |
| 12 | Selective bloom: a depth mask pass keeps only the chosen objects, bloom runs on that mask and the result is added back, so a hit flash can bloom hard while the sand does not. | `pmndrs/postprocessing`, https://github.com/pmndrs/postprocessing, read live 2026-10-21 | MIT (same root `LICENSE.md`, read live 2026-10-21, OBSERVED) | no row, propose a new row: **LF6c hit flash** (one bright sprite that the bloom pass turns into a readable burst of light) | `src/effects/SelectiveBloomEffect.js` (a `DepthPass` plus a `DepthMaskMaterial` shader pass plus a clear pass, then `BloomEffect` on the mask, OBSERVED). Our own WGSL mask pass beside the landed bloom in `crates/render/src/post.rs`. | M | Kill if a bright emissive flash sprite with the landed bloom (LF4, ticked 99e624b) already reads as a burst of light in a capture — then no mask pass is built. | RESEARCH-ONLY (a new render pass on top of a landed bloom; read first, port later) |

## Rejections

- `djeedai/bevy_hanabi` — already row 1 of `docs/research/powerups-2026-10-20/02-physics-vfx-destruction-steals.md`
  (the GPU compute, sort and indirect half is still unported). Not repeated here.
- `jinleili/simuverse`, `olliep24/firesim`, `InteractiveComputerGraphics/splashsurf`,
  `dgreenheck/OpenFracture` — all already in the 2026-10-20 physics/VFX file. Not repeated.
- `paulrobello/par-particle-life` — GPU particle life in Rust with wgpu, but AGPL-3.0 and it is a
  rule-set toy, not an emitter. REJECT on the copyleft (same call as the 2026-10-20 file).
- `id-Software/Quake` — read `QW/client/r_part.c` live 2026-10-21: GPL-2.0 header, `#define
  MAX_PARTICLES 2048`, the 8-entry brightness ramps. That is the 1996 spark pool our "2005 MMO"
  brief is chasing, but GPL-2.0 forbids copying it and our own pool already carries a cap
  (`MAX_PARTICLES_60K = 60_000`). The client-side spark spawner was not located by the code index
  today. RESEARCH-ONLY, no row.
- `MdIqubal/Matrix-Shockwave-Shader` — MIT, but the ripple is a tessellated surface shader and
  neither WebGPU nor wgpu has a tessellation stage, so the shape cannot be ported as written.
  REJECT: use the screen-space ring (row 10) or a mesh ring instead.
- `JujuAdams/Shockwave` — MIT (GameMaker project), a material that writes a distortion direction
  into RG for a surface shader to consume. It needs a surface-shader host we do not have, and the
  screen-space version (row 10) is the portable one. REJECT as a donor, keep as a reference.
- `mohan-bee/GPU-Particle-System` — no LICENSE file in the root listing and no `license` field in
  `particle-system-gpu/Cargo.toml` (both read live 2026-10-21), so no grant; also a beginner
  project. RESEARCH-ONLY, and rows 1-3 cover the same ground.
- `Sieluna/Tianshi` — no LICENSE file and no `license` field in `Cargo.toml` (read live
  2026-10-21): a wgpu + SPIR-V particle study, no grant. RESEARCH-ONLY.
- `ttnghia/RealTimeFluidRendering` — no LICENSE file in the root listing (only
  `FluidRendering.licenseheader`), so no grant, and it is a narrower-range-filter screen-space
  fluid than row 11. RESEARCH-ONLY.
- `kevinraymond/fosfora` `splat_sort.rs` — same donor as row 1, and the 16-bit counting sort is
  written for 3DGS splats; the GPU particle sort idea is already owned by the bevy_hanabi row of
  the 2026-10-20 file. Not a separate row.
- `loganzartman/wgpu-particles`, `snakeru/wgpu-particles` — tiny, licences not read, and rows 1-3
  cover the same ground.
- `BabylonJS/Babylon.js` `packages/dev/core/src/Decals/` — that directory does not exist today
  (path error on 2026-10-21); the decal builder lives under `packages/dev/core/src/Meshes/Builders/`.
  Noted so nobody re-checks the wrong path.
- `mrdoob/three.js` `GPUComputationRenderer` — three.js is already in `docs/research/LADDER.md`
  (line 237) for the film-grain shader, and its GPGPU ping-pong idea is the same design as row 2.
  Not repeated.
- Unreal, Unity, Godot, O3DE, Flax, Stride, Filament, Bevy — excluded by the wave rules.
- `OGRECave/ogre-next` `OgreDecal.cpp` also carries a clustered-light request queue line
  (`ForwardPlusBase::MinDecalRq`, OBSERVED). A transient impact light is a real VFX want, but the
  board has no row for it, so it is noted here rather than claimed as a row.

## Top 3

1. `kevinraymond/fosfora`, https://github.com/kevinraymond/fosfora — MIT OR Apache-2.0.
   Closes `NS11` with the GPU-resident pool: ping-pong buffers, alive/dead index lists, indirect
   draw, obstacle texture, spatial hash. Cost L. The compute-update-and-sort half is owned by the
   bevy_hanabi row of the 2026-10-20 file; this row adds the pool discipline and the Rust/wgpu
   binding shape.
2. `BabylonJS/Babylon.js`, https://github.com/BabylonJS/Babylon.js — Apache-2.0.
   Closes `LF6` with two files: `packages/dev/core/src/ShadersWGSL/gpuUpdateParticles.compute.fx`
   (the update maths a game needs: emitters, flow map, attractors, sprite sheet, colour
   gradients) and `packages/dev/core/src/Particles/subEmitter.ts` (a particle that spawns a burst
   when it dies — the hit-spark recipe). Cost M, then S for the burst alone.
3. `OGRECave/ogre-next`, https://github.com/OGRECave/ogre-next — MIT.
   Closes `LF7` and `LF14` with two files: `OgreMain/src/OgreDecal.cpp` (decal layers, per-decal
   roughness for wet-to-dry blood) and `OgreMain/src/OgreRibbonTrail.cpp` (the ribbon trail).
   Cost M for the decal, S for the trail.
