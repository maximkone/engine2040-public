### 2040 tier (operator 2026-09-24, Finish C; dispatch after Finish line rows)

Look (RT, VG, VX), online (ON) and world (WD) rows from `FOCUS.md` and
`VISUAL-TARGET.md` V5. Presentation rows never feed the tick; golden
`d57b5c8ae3096614` stays x2 on every tier. Patterns come from papers and
public talks; engine source from Unreal, Unity or Flax is never read.

Frontier spikes (FU) from `docs/plan/futures.md` (inbox S36): one
cpu-light test per GAP/COLD topic in the owning crate, bytes x2,
golden unchanged; kill bars are gates.

- [ ] FU1 futures: audio-middleware spike (DSP graph beside the bus mixer).
  Tier B spike. Size S. Owns spike hunks beside `crates/audio/src/dsp.rs`
  (`Bus`, RMS gate frozen). Accept: `cargo test --locked -p engine-audio dsp`
  green; 16-voice bus mix posts ms + RMS baseline; 0 audio-thread allocs;
  bytes x2; golden unchanged. needs: SN6.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] FU2 futures: AI-native editor spike (scene diff + merge window).
  Tier A spike. Size S. Owns spike hunks beside `crates/editor/src/commands.rs`
  (journal API frozen). Accept: `cargo test --locked -p engine-editor commands`
  green; 2000-object diff posts ms + byte count; undo still whole-batch x2;
  golden unchanged. needs: EV3.
- [ ] FU3 futures: open-prompt eval spike (prompt pack on the A6 tasks).
  Tier A spike. Size S. Owns spike hunks beside `crates/endpoint/src/discover.rs`
  (twenty-tool gate frozen). Accept: `cargo test --locked -p engine-endpoint discover`
  green; completion + tokens delta posted; manifest bytes x2; golden unchanged.
  needs: EV3.
- [ ] FU4 futures: learned-physics rollout spike (offline advisor only).
  Tier C spike. Size S. Owns spike hunks beside `crates/physics/src/lib.rs`
  (tick step frozen). Accept: `cargo test --locked -p engine-physics learned`
  green; 128-body rollout posts error vs analytic + ms; tick code untouched;
  golden unchanged. Kill: any learned branch in tick. needs: PX1.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] FU5 futures: differentiable-rendering spike (offline gradients only).
  Tier C spike. Size S. Owns spike hunks beside `crates/render/src/gi.rs`
  (lookup-only runtime frozen). Accept: `cargo test --locked -p engine-render diffr`
  green; 64x64 gradient image posts loss + ms on O5 with the adapter line;
  no present-loop hook; golden unchanged. needs: RA1.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] FU6 futures: AI-material offline-encode spike (bake only).
  Tier C spike. Size S. Owns spike hunks beside `crates/assets/src/texture.rs`
  (cook bounds frozen). Accept: `cargo test --locked -p engine-assets matgen`
  green; 256x256 bake posts bytes + ms; cook x2 identical; golden unchanged.
  needs: CT1.
- [ ] FU7 futures: engine-comparison rubric pin (docs only, no code).
  Tier A spike. Size S. Owns `docs/research/COMPARE.md` table cells
  (harness shape frozen). Accept: fixed task set + O5 line +
  version/source/date per compared system; UNKNOWN stays UNKNOWN;
  `node C:/empire/center/vision-check.mjs engine2040` PASS for
  this file's map row; golden unchanged. needs: EV3.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] FU8 futures: facial lip-sync map spike (phoneme to viseme bake).
  Tier B spike. Size S. Owns spike hunks beside `crates/anim/src/lipsync.rs`
  (bake refusal frozen). Accept: `cargo test --locked -p engine-anim lipsync`
  green; 10-cue bake posts key count + ms; runtime inference still refused;
  bytes x2; golden unchanged. needs: CT7.
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.

- [ ] RT2 render: ray-traced shadows — sun and area lights with soft
  contact-hardening penumbra; cascades stay the Base fallback. Tier C.
  Size M. Owns `crates/render/src/rt_shadow.rs`. Accept: penumbra widens
  with blocker distance in fixtures; per-pass timing on the reference GPU.
  needs: RT1, GX2.
  LATER: parked wave67 RT2 verdict — unpark only when RT7 fixtures + GX13/Q1 real-arena captures + PF2 table + ray-query pinned; Tier C sequencing.
- [ ] RT3 render: many-light direct light — ReSTIR DI over every torch,
  brazier and fire (MegaLights class), shadowed. Tier C. Size L. Owns
  `crates/render/src/restir_di.rs`. Accept: a night arena with 250
  shadowed torches holds the RT-tier budget (ReSTIR DI 2.0 ms at 250
  torches); Base falls back to clusters. Consumer is the WIN1 match.
  needs: RT1, GX2.
  LATER: parked wave67 RT3 verdict — unpark only when RT2 ticked + RT7 fixtures green + ref-GPU ReSTIR re-spike RMSE <=10%; immediate 250-torch work rides RT15.
- [ ] RT4 render: dynamic GI with no bake — ReSTIR GI plus a world
  radiance cache (Solari and id Tech 8 patterns, from their public talks).
  Tier C. Size L. Owns `crates/render/src/restir_gi.rs`. Accept: a
  day-to-night sun sweep updates bounce light with no bake step (ReSTIR
  GI + cache 2.5 ms); the gate tunnel leak test passes; Base keeps RL5
  or RT8. Consumer is the WIN1 match. needs: RT1, GX3.
  LATER: parked wave67 RT4 verdict — unpark only when GX3 ticked + RT7 fixtures + Q1/GX13 captures + PF2 table + ray-query pinned + ReSTIR re-spike ADOPTs.
- [ ] RT5 render: ray-traced reflections — glossy and mirror (wet sand,
  polished armor, water trough) with a screen-space first hit. Tier C.
  Size M. Owns `crates/render/src/rt_reflect.rs`. Accept: a roughness
  sweep on the material chart matches the RT7 reference within tolerance.
  needs: RT4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT6 render: engine denoiser — spatiotemporal (SVGF and ReBLUR shape,
  reimplemented from papers) for RT2–RT5; vendor denoisers only as
  optional adapters after a license row. Tier C. Size L. Owns
  `crates/render/src/denoise.rs`. Accept: a moving-camera clip with no
  boiling at the RT-tier sample count; the ghosting test passes. needs:
  RT2, GX4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT7 render: path-traced reference — progressive, same materials and
  lights; the ground truth for Q captures and photo mode. Tier C. Size L.
  Owns `crates/render/src/pathtrace.rs`. Accept: converged stills of the
  material chart; RT-tier stills within perceptual tolerance of it.
  Deep 3D pass 2026-09-25: fixtures chart, night (24 of 250 torches) and tunnel; seeds 7 and 17 at 64 to 4,096 spp; converged when 1,024 vs 4,096 spp FLIP is at most 0.02; an ablation with NEE off must score clearly worse on night and tunnel, else the reference is rejected. needs: GX8, RT1, RL6.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT8 render: probe-grid GI for Base — cascaded light grids with
  runtime relight (id Tech 8 shape), so GPUs without ray queries still get
  moving bounce light. Tier C. Size M. Owns
  `crates/render/src/probe_grid.rs`. Accept: a sun sweep relights probes
  within budget on llvmpipe and the reference GPU. needs: GX3.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG1 render: GPU-driven culling — instance buffers, indirect draws,
  two-pass Hi-Z occlusion. Tier C. Size M. Owns
  `crates/render/src/gpu_cull.rs`. Accept: an occluded-crowd fixture draws
  under 10% of instances; CPU draw cost stays flat as instances grow.
  needs: GX1, RA3.
  LATER: re-aim at renderer 2026-10-06 (owner dropped viewer, D7 Bevy gate dead); CPU oracle seed at target/leftover-2026-10-06/engine-gameplay; planner re-seats into this row's home; unpark by deleting this line.
- [ ] VG2 assets: meshlet cook — clusters plus a simplification DAG
  (meshoptimizer patterns, license read first), bytes identical x2.
  Tier C. Size M. Owns `crates/assets/src/meshlet.rs`. Accept: arena
  architecture cooks to clusters within the error bound; cook x2
  identical. needs: CT2.
  LATER: re-aim at renderer 2026-10-06 (owner dropped viewer, D7 Bevy gate dead); CPU oracle seed at target/leftover-2026-10-06/engine-gameplay; planner re-seats into this row's home; unpark by deleting this line.
- [ ] VG3 render: virtual geometry — cluster LOD cut per view, visibility
  buffer, compute raster for tiny triangles. Tier C. Size L. Owns
  `crates/render/src/meshlet.rs`. Accept: 100M source triangles of arena
  stone at budget (virtual geometry 3.0 ms); no LOD pop in a moving clip.
  Consumer is the WIN1 match. Deep 3D pass: gate triple for VG1/VG3/VG5: 3.0 ms at 100M triangles, under 10% of drawn clusters occluded, no LOD pop in a camera fly-through (meshoptimizer MIT and voidin MIT patterns; Bevy meshlets ideas only). needs: VG1, VG2.
  LATER: re-aim at renderer 2026-10-06 (owner dropped viewer, D7 Bevy gate dead); CPU oracle seed at target/leftover-2026-10-06/engine-gameplay; planner re-seats into this row's home; unpark by deleting this line.
- [ ] VG4 render: GPU crowd — 20k–50k spectators with vertex-animation
  textures and impostors, reacting to AU2 crowd events. Tier C. Size M.
  Owns `crates/render/src/crowd.rs`. Accept: 20k spectators within 1.5 ms
  on the reference GPU; a cheer wave follows a kill event. Consumer is
  the WIN1 match. needs: VG1,
  AU2.
  LATER: re-aim at renderer 2026-10-06 (owner dropped viewer, D7 Bevy gate dead); CPU oracle seed at target/leftover-2026-10-06/engine-gameplay; planner re-seats into this row's home; unpark by deleting this line.
- [ ] VX1 render: froxel volumetric fog, light shafts and local fog
  volumes (sand dust, torch smoke), lit by the sun and RT3 lights.
  Tier C. Size M. Owns `crates/render/src/volumetric.rs`. Accept: shafts
  through the gate arch; stable in motion; 1.0 ms at 1080p on the
  reference GPU. Consumer is the WIN1 match. needs: GX7, GX2, RL4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX2 render: volumetric clouds and weather sky — rain wetness drives
  the RL6 wet layer. Tier C. Size M. Owns `crates/render/src/clouds.rs`.
  Accept: a day-to-night clip with moving clouds; wet sand darkens and
  reflects. needs: GX7, GX8, RL4, RL6.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX3 render: characters up close — skin subsurface scattering, eye
  shading, hair cards with anisotropic shading. Tier C. Size L. Owns
  `crates/render/src/character.rs`. Accept: blind review of the hero
  close-up against the RT7 reference; budget per fighter posted. needs: GX8,
  RL6.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX4 render: cinematic camera — physical exposure with histogram
  auto-exposure, bokeh depth of field, per-object motion blur, film grain,
  HDR10 output. Tier C. Size M. Owns `crates/render/src/camera_fx.rs`.
  Accept: exposure stays stable through the tunnel exit (cinematic
  1.0 ms); HDR10 swapchain on the reference display or an honest refusal.
  Consumer is the WIN1 match. needs: GX4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX5 render: GPU blood, fire and smoke sims — presentation only,
  seeded pools. Tier C. Size M. Owns `crates/render/src/fluids_gpu.rs`.
  Accept: identical sim bytes x2 from a seed on one device; 1.0 ms budget.
  needs: GX9, RP2.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX6 game: photo mode over the RT7 path tracer (LIGHT). Tier C.
  Size S. Owns `crates/game/src/photo.rs`. Accept: pause, free camera, a
  converged still saved with its manifest. needs: RT7, VX4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX7 render: Gaussian-splat backdrops — licensed captures of ruins or
  skyline composited with depth, presentation only (LIGHT). Tier C.
  Size M. Owns `crates/render/src/splat.rs`. Accept: the backdrop sorts
  against arena depth; a license row per capture. LATER: after WD3.
  needs: GX1.
- [ ] RT9 render: RT tier gate — tier-aware graph wiring with per-pass
  budgets, tier stamped in every RA8 manifest. Tier C. Size M. Owns
  `crates/render/src/rt_tier_gate.rs`. Accept: llvmpipe reports Base and
  runs the same graph; forced RT on llvmpipe refused loudly; per-pass
  budget table posted; reference-GPU tier + timing (reference GPU).
  needs: GX1, RT0.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT10 render: proxy validation — CT2 proxy deviation gate for BLAS
  builds. Tier C. Size S. Owns `crates/render/src/proxy_check.rs`.
  Accept: proxy-to-mesh deviation charted within the posted bound; build
  inputs identical x2. needs: RT1, CT2.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT11 render: ReSTIR validation — reservoir convergence against the
  RT7 reference on torch and bounce fixtures. Tier C. Size M. Owns
  `crates/render/src/restir_check.rs`. Accept: night-torch and tunnel
  fixtures within perceptual tolerance of RT7 at the RT sample count.
  Deep 3D pass: visibility-weighted reservoirs (2506.05930) and one MCMC mutation per frame (2211.00166) as ablations; kill a variant with under 10% variance gain or over 0.5 ms. needs: RT3, RT7.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT12 render: denoiser-to-graph wiring — the RT6 denoiser output
  feeds the GX4 post chain through the RA2 aliasing pool with history
  validation, reusing the landed RT9 budget gate. Tier C. Size M. Owns
  `crates/render/src/denoise_wire.rs`. Accept: moving-camera clip shows
  no boiling and passes the ghosting test; history buffers ride RA2
  transients; llvmpipe parity within tolerance; denoise 1.5ms on the
  reference GPU. needs: RT6, GX4, RT9. (PLN render-2040, reviewed
  2026-09-26; waits RT6/RT9.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT13 render: path-reference convergence lock — the RT7
  1,024-vs-4,096 spp FLIP-at-most-0.02 gate plus the NEE-off ablation
  become permanent fixtures chart, night and tunnel at seeds 7 and 17.
  Tier C. Size S. Owns `crates/render/src/pathtrace_gate.rs`. Accept:
  converged stills of the material chart; RT-tier stills within
  perceptual tolerance of RT7; NEE-off scores clearly worse on night
  and tunnel or the reference is rejected. needs: RT7, GX8. (PLN
   render-2040, reviewed 2026-09-26; waits RT7.)
   LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] RT19 render: Bevy-SSR screen-space reflection march port - port the
  `ScreenSpaceReflections` settings shape plus plugin plus uniform bind
  shape from `bevy_pbr-0.20.0-rc.2/src/ssr/mod.rs` and the depth-march
  plus bisection refine from `src/ssr/raymarch.wesl` (MIT OR Apache-2.0,
  Bevy licence notice kept, zero donor bytes pasted, own WGSL) as new
  presentation-only `crates/render/src/rt_reflect.rs` owning the RT5
  screen-space half beside frozen `post.rs`, never in tick; the ray-query
  mirror half stays parked behind RT4. Tier C. Size S. Owns
  `crates/render/src/rt_reflect.rs`. Accept: `cargo test -p engine-render
  --lib rt_reflect` green plus roughness sweep on the material chart
  within RT7 tolerance with the march inside the 1.0 ms reflections
  ceiling at 1080p on the RTX 3050, present x2 identical, golden
  `d57b5c8ae3096614` unchanged. F2P: the rt_reflect test fails before
  (no module on master). P2P: rt_reflect green x2 plus golden held.
  needs: RL6, GX4. (RES-STEAL 2026-10-05 bevy-ssr r13/r14/r16/r19/r21/r23
   deduped to one row; D8 re-seated at crates/render per FOCUS D8/D9.)
- [ ] RT20 render: Bevy contact-shadow resolve for the RT2 raster fallback
  - port `ContactShadows` (linear_steps 16, thickness, length) plus the
  uniform upload from `bevy_pbr-0.20.0-rc.2/src/contact_shadows.rs`
  (MIT OR Apache-2.0, licence notice kept, zero donor bytes pasted, own
  WGSL raymarch over the depth prepass) beside the frozen cascade path in
  `crates/render/src/shadow_cluster.rs`, presentation-only never in tick.
  Tier A. Size S. Owns `crates/render/src/shadow_cluster.rs`. Accept:
  `cargo test -p engine-render --lib shadow_cluster` green plus
  fighter-feet contact darkening stable in motion inside the 1.0 ms
  ceiling at 1080p on the RTX 3050 with present x2 identical and golden
  unchanged. F2P: contact-darkening fixture absent before. P2P: fixture
  green x2 plus golden held. needs: RT1, GX2. (RES-STEAL 2026-10-05
  bevy-contact-shadows r17; D7-frozen until BV3.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG5 render: visibility-buffer resolve + Hi-Z pyramid wired into the
  forward pass. Tier C. Size L. Owns `crates/render/src/vis_buffer.rs`.
  Accept: occluded-crowd fixture draws under 10% of instances; no LOD pop
  in a moving clip; 100M source triangles at budget on the reference GPU.
  needs: VG1, VG2.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG6 render: crowd LOD + VAT validation with AU2 cheer coupling.
  Tier C. Size M. Owns `crates/render/src/crowd_lod.rs`. Accept: 20k
  spectators within 1.5 ms on the reference GPU; a cheer wave follows a
  kill event. needs: VG4, AU2.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG10 assets: meshlet cook-to-render contract proof — every VG2
  cook names exactly the VG3 cluster cut it feeds with the posted
  error bound, proven x2 on arena stone. Tier C. Size S. Owns
  contract test + per-cook cut records (no new system). Accept: exact
  cut sets; error bound held; cook bytes x2; 100M source triangles at
  the 3.0ms budget on the reference GPU. needs: VG2, VG3, CT2. (PLN
  render-2040, reviewed 2026-09-26; waits VG2/VG3.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG11 render: indirect-draw production wiring — the VG8 compute
  cull writes indirect draws the RA1 presenter executes, over the RA2
  aliasing pool with typed no-device refusal. Tier C. Size M. Owns
  the presenter hunks beside `crates/render/src/gpu_cull.rs` (cull
  API frozen). Accept: occluded crowd under 10% drawn; CPU flat to
  20k instances; llvmpipe parity; reference-GPU timing. needs: VG8,
  RA2. (PLN render-2040, reviewed 2026-09-26; waits VG8.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG7 assets: meshlet clusterize + simplify cook — bounded clusters +
  error DAG for 100M-tri arena stone, bytes x2 (meshoptimizer clusterize /
  simplify / overdraw ideas, MIT, reimplemented std-only, no vendoring).
  Tier C. Size M. Owns `crates/assets/src/meshlet.rs`. Accept: arena
  architecture within the posted error bound, cook x2 identical, llvmpipe
  parity on bytes. needs: CT2, VG2.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG8 render: GPU indirect cull + Hi-Z two-pass — compute frustum cull
  to indirect draws (voidin bvh + indirect shape, MIT, reimplemented on
  current wgpu; RA2 aliasing pool; typed no-device refusal). Tier C.
  Size M. Owns `crates/render/src/gpu_cull.rs`. Accept: occluded crowd
  under 10% drawn, CPU flat to 20k, llvmpipe parity + reference-GPU
  timing. needs: GX1, RA3, VG1.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG9 render: instanced crowd phase-buffer + VAT bake spike —
  many_foxes-pattern rings (Bevy, MIT, ideas reimplemented) to VAT plus
  AU2 cheer coupling. Tier C. Size M. Owns `crates/render/src/crowd.rs`.
  Accept: 20k in 1.5 ms on the reference GPU, cheer follows kill, VAT
  bytes x2, else killed with numbers. needs: VG1, VG4, AU2, CT1.
   LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG13 render: Bevy-meshlet cluster build with LOD cut — port
  `MeshletMesh::from_mesh` cluster build plus `BvhNode`/`Meshlet`/
  `MeshletCullData` layouts plus BVH traversal LOD select
  (`project_aabb`, `lod_error_is_imperceptible`, `aabb_in_frustum`)
  from `bevy_pbr-0.20.0-rc.2/src/meshlet/` (MIT OR Apache-2.0, Bevy
  licence notice kept, zero donor bytes pasted, own WGSL) as new
  presentation-only `crates/render/src/meshlet.rs` beside frozen
  `draw.rs`, never in tick; visibility-buffer cull follows as a second
  piece. Tier C. Size S. Owns `crates/render/src/meshlet.rs`. Accept:
  `cargo test -p engine-render --lib meshlet` green plus 100M-triangle
  arena stone inside the 3.0 ms virtual-geometry ceiling on the RTX 3050
  with under 10% drawn clusters occluded and no LOD pop in a moving
  clip, digest x2, golden `d57b5c8ae3096614` unchanged. F2P: the meshlet
  test fails before (no module on master). P2P: meshlet green x2 plus
  golden held. needs: VG1, VG2. (RES-STEAL 2026-10-05 bevy-meshlet
   r1-r11 deduped; D7-frozen until BV3.)
   LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VG14 render: visibility-buffer cull second piece over the VG13
  meshlet build - port two-pass instance/BVH/cluster cull
  (`project_aabb`, `lod_error_is_imperceptible`, `aabb_in_frustum`,
  `should_occlusion_cull_aabb`, 64px software-raster split, hardware
  raster plus resolve) as presentation-only hunks in
  `crates/render/src/meshlet.rs`, Bevy licence notice kept, zero donor
  bytes pasted, own WGSL, never in tick. Tier C. Size S. Owns
  `crates/render/src/meshlet.rs`. Accept: `cargo test -p engine-render
  --lib meshlet` green plus 100M-triangle arena stone inside the 3.0 ms
  virtual-geometry ceiling on the RTX 3050 with under 10 percent drawn
  clusters occluded and no LOD pop in a moving clip, digest x2, golden
  unchanged. F2P: visbuf-cull fixture fails before. P2P: meshlet green x2
  plus golden held. needs: VG13. (RES-STEAL 2026-10-05
  bevy-meshlet-visbuf r15; D7-frozen until BV3.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX8 render: froxel + cloud integration — RT3 light injection and
  rain-wet coupling into the RL6 wet layer. Tier C. Size M. Owns
  `crates/render/src/vol_integrate.rs`. Accept: shafts through the gate
  arch stable in motion; wet sand darkens and reflects; 1.0 ms fog budget
  on the reference GPU. needs: GX2, RL4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX9 render: cinematic + HDR10 calibration — histogram auto-exposure,
  bokeh, motion blur, grain and swapchain output. Tier C. Size M. Owns
  `crates/render/src/cine_hdr.rs`. Accept: tunnel-exit exposure stable;
  HDR10 on the reference display or honest refusal. Deep 3D pass: AgX against ACES A/B, histogram exposure, 13-tap bloom with Karis firefly rejection (Bevy v0.16 MIT, PORT row); HDR10 or an honest refusal. needs: GX4, VX4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX10 render: fluid determinism — seeded blood, fire and smoke sim
  bytes x2 on one device. Tier C. Size S. Owns
  `crates/render/src/fluids_check.rs`. Accept: identical sim bytes x2
  from a seed; 1.0 ms budget on the reference GPU. needs: RP2, VX5.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX11 game: photo + splat gate — converged stills with manifests and
  licensed backdrops sorted against depth. Tier C. Size S. Owns
  `crates/game/src/photo_splat.rs`. Accept: pause + free camera saves a
  converged still with manifest; backdrop sorts against arena depth; a
  license row per capture. needs: RT7, VX6.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX12 render: weather coupling lock — one contract pins WD5 cycle
  outputs to VX2 clouds, VX8 shafts and the RL6 wet layer so the three
  can never drift. Tier C. Size S. Owns
  `crates/render/src/weather_lock.rs`. Accept: day-to-night clip with
  moving clouds and wet sand; golden hash unchanged with weather on
  asserted x2; 1.0ms fog budget on the reference GPU. needs: VX8,
  RL6, WD5. (PLN render-2040, reviewed 2026-09-26; waits VX8/WD5.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX13 render: character budget lock into PF1 — post the VX3
  per-close-up-fighter 1.0ms cost into PF1 workloads with a kill bar
  against the RT7 hero close-up. Tier C. Size S. Owns perf baseline
  hunks (no new pass code). Accept: a seeded character regression
  fails the gate; blind review of the hero close-up against RT7;
  golden unchanged. needs: VX3, PF1, GX8. (PLN render-2040, reviewed
  2026-09-26; waits VX3.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX14 render: arena air — cheap seeded dust motes plus torch-glow
  sprites, presentation only, ahead of full VX1 froxel. Tier C. Size S.
  Owns `crates/render/src/arena_air.rs`. Accept: capture pair before
  `target/showcase/2026-09-27-q1/fight/frame_000015.png` vs after on
  the reference GPU scores higher by this seat on atmosphere + motion
  with 0 validation errors; golden unchanged. needs: RP2, GX9.
  (LOOK-ART 2026-09-27.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX15 render: stands + gate-tunnel backdrop seal — crowd-card stands
  ring plus a dark gate-tunnel mouth against the sunlit sand as
  presentation-only backdrop geometry in a new
  `crates/render/src/arena_backdrop.rs`, ahead of the CT25 kit and full
  VX1 volumetrics (`arena_air.rs` frozen). Tier C. Size S. Owns
  `crates/render/src/arena_backdrop.rs`. Accept: capture pair before
  `target/pilot-gx13/owner-arena/frame_000015.png` vs after on the
  reference GPU scores higher by this seat on atmosphere + composition
  with 0 validation errors; golden unchanged. needs: CT8, GX9.
  (LOOK-ART 2026-09-28.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX16 render: banner wave + dust drift — presentation-only cloth wave
  on the CT53 dressing banners plus seeded dust drift, ahead of full
  VX1 froxel and VG4 crowd. Tier C. Size S. Owns
  `crates/render/src/arena_banners.rs` (`arena_backdrop.rs`,
  `arena_air.rs`, `twin_grade.rs` frozen). Accept: capture pair before
  `target/pilot-owner-view/r3-20260929/editor-a3/viewport.png` vs after
  on the reference GPU scores higher by this seat on atmosphere +
   motion with 0 validation errors; golden unchanged. needs: GX9, RP2.
   (LOOK-ART 2026-09-29.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] VX17 render: Bevy volumetric-fog froxel port — port
  `VolumetricFogPlugin` plus `init_volumetric_fog_pipeline` plus
  `VolumetricFogPipeline` plus `volumetric_fog.wesl` screen-space
  raymarch with Henyey-Greenstein phase (settings `VolumetricFog`
  step_count 64 plus jitter, `VolumetricLight` on the sun, `FogVolume`
  boxes; own WGSL, zero donor bytes pasted, Bevy licence notice kept)
  from `bevy_pbr-0.20.0-rc.2/src/volumetric_fog/` plus
  `bevy_light-0.20.0-rc.2/src/volumetric.rs` (MIT OR Apache-2.0) as new
  presentation-only `crates/render/src/volumetric.rs` beside frozen
  `fog.rs`, never in tick; shafts-only slice (sun through gate arch),
  torch volumes deferred. Tier C. Size S. Owns
  `crates/render/src/volumetric.rs`. Accept: `cargo test -p
  engine-render --lib volumetric` green plus gate-arch shafts stable in
  motion at 1.0 ms at 1080p on the RTX 3050 with present x2 identical and
  golden `d57b5c8ae3096614` unchanged. F2P: the volumetric test fails
  before (no module on master). P2P: volumetric green x2 plus golden
  held. needs: GX7, GX2, RL4. (RES-STEAL 2026-10-05 bevy-volfog r2-r11
  deduped; D7-frozen until BV3.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] ON6 net: sessions — party, server list, matchmaking-lite by glory
  rank (LIGHT). Tier C. Size M. Owns `crates/net/src/session.rs`.
  Accept: a party of 3 queues and lands in one match; the server list
  refreshes under budget. needs: ON2, MP3.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] ON7 net: networked physics — predicted debris, ragdoll contacts and
  siege projectiles with server authority and client resim. Tier C.
  Size L. Owns `crates/net/src/phys_sync.rs`. Accept: a wall break
  converges on all clients at 150 ms RTT; replay exact. needs: ON1, DX4.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] ON8 audio: proximity voice — Opus (license read first) over ON1
  streams (LIGHT). Tier C. Size M. Owns `crates/audio/src/voice.rs`.
  Accept: two clients talk with under 150 ms added latency; mute works.
  needs: ON1.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] ON12 audio: voice proximity mix — Opus (binding license read first) over
  ON1 streams with jitter buffer and distance gain, presentation only. Tier C.
  Size M. Owns `crates/audio/src/voice_net.rs`. Accept: two QUIC clients talk
  with under 150 ms added latency; mute and deaf flags authoritative; golden
  hash unchanged with voice on. needs: ON1, ON8.
- [ ] ON13 net: invite-and-party hardening — expiring invite codes, version +
  rules-digest refuse, party of 3 into one match, list refresh budget. Tier C.
  Size M. Owns `crates/net/src/party.rs`. Accept: mismatched versions refuse
  loudly with the digest pair; party of 3 lands in one match; list refresh
  under budget on fixtures. needs: ON2, ON6.
- [ ] ON14 net: datagram discipline — never fragment past 1,200 bytes
  (`OverBudget` drops and counts), far and idle poses shed first under
  congestion with 2-3 tick expiry, inputs and combat sent redundantly and
  never expired (RFC 9221 datagram rules). Tier C. Size M. Owns
  `crates/net/src/wan_discipline.rs`. Accept: 64 bots plus AI at 150 ms,
  30 ms jitter, 2% loss and reorder converge; no combat entity starved over
  500 ms; at most 128 kbps per client; hash x2. needs: ON1, ON3, ON4, ON5.
- [ ] ON15 net: per-connection scope gate — room membership + per-user scope
  predicate over `aoi.rs` feeding the ON3 priority queue (naia rooms/scopes,
  MIT OR Apache-2.0, patterns only; naia has no rate-LOD of its own).
  Tier C. Size M. Owns `crates/net/src/interest.rs`. Accept: 64-bot soak
  shows 3x or better saving vs broadcast at 128 kbps/client or less, no
  combat entity starved over 500 ms, zero desync. needs: ON3, MP2.
- [ ] ON16 net: token handshake + soak harness — encrypted short-lived join
  tokens and a replay window feeding `wan_soak.rs` per-fault reports
  (netcode.io STANDARD + soak.c, BSD-3-Clause, patterns reimplemented in
  Rust, no C vendored). Tier C. Size M. Owns `crates/net/src/wan_soak.rs`.
  Accept: ON4 matrix at 16-32 converges on every checkpoint with a
  per-fault report; seeded rejoin converges in 5 s or less. needs: ON1, ON2.
- [ ] ON17 net: per-class resend/memory discipline — unidirectional channel
  configs with resend timers and shed caps behind the Transport trait
  (renet channel shapes, MIT OR Apache-2.0, reimplemented; may fold into
  ON14 acceptance instead of landing separately). Tier C. Size M. Owns
  `crates/net/src/wan_discipline.rs`. Accept: ON4 matrix converges with
  zero   combat starvation, 128 kbps/client or less, hash x2. needs: ON1,
  ON3, ON4.
- [ ] ON18 net: version-stamped WAN resync — CC-OPI (arXiv 2609.19208v1)
  version-based sync + emergency pool reimplemented std-only in wan_soak
  as an alternative to the AH4 path (spike only, no codec change).
  Tier B. Size S. Owns `crates/net/src/wan_soak.rs`. Accept: ON4 matrix
  at 16-32 converges on every checkpoint with per-fault converge-time +
  byte-overhead report; overhead over 10 percent or any miss kills it
  with numbers. needs: ON1, ON3.
- [ ] ON19 net: P2P-sync negative gate — CRDT P2P VR (arXiv 2503.17826v1)
  evaluated and rejected for combat state (non-Byzantine assumption vs
  the ON5 cheat model). Tier B. Size S. Owns a negative-test block in
  `crates/net/src/relay.rs` tests. Accept: test asserts combat snapshots
  never merge peer-to-peer and every state change carries authority +
  tape; a P2P combat path fails the gate. needs: ON1, ON5.
- [ ] ON20 net: delta-maintained scope views — DBSP section-4 Z-set idea
  (arXiv 2203.16684v1, mechanism only, reimplemented std-only, no Feldera
  dep) for ON15 room/scope predicates over aoi.rs. Tier B. Size S. Owns
  `crates/net/src/interest.rs` scope-view block + `fixtures/scope-delta/`.
  Accept: 100/100 equality with full rebuild on the seeded trace,
  one-tick p95 within the ON10 tick budget at 5x or better median
  speedup, else killed with numbers. needs: ON3.
- [ ] ON21 net: rollback prediction from lightyear — `LinkConditionerConfig`
  + `InputBuffer` patterns (MIT/Apache-2.0, reimplemented, no Bevy dep).
  Tier C. Size S. Owns `crates/net/src/rollback.rs`. Accept: 4-client
  headless server with 2-frame delay shows <3 rollback events per 1000
  ticks; hash x2. needs: ON1, MP1.
- [ ] ON22 net: GGRS input-prediction and desync-detection patterns —
  `InputPredictor` trait + checksum desync ported (MIT/Apache-2.0,
  reimplemented; P2P session model NOT taken). Tier C. Size S. Owns
  `crates/net/src/prediction.rs`. Accept: 3-client local simulation with
  2-frame delay shows <2 mis-predictions per 1000 frames; desync detected
  within 1 tick. needs: ON1, MP1.
- [ ] ON23 net: renet_netcode authenticated-transport audit — compare
  `NetcodeServerTransport` token-auth against quinn TLS for ON1 (patterns
  only, no dep). Tier C. Size S. Owns `crates/net/src/transport_audit.rs`.
  Accept: side-by-side connection-setup benchmark posted; quinn wins or
  ties. needs: ON1.
- [ ] ON24 net: ICE conformance hardening — MESSAGE-INTEGRITY on STUN
  binding from invite-ticket creds, candidate-pair nomination,
  consent-freshness keepalives (str0m is-crate shapes, MIT/Apache,
  reimplemented, no dep). Tier C. Size M. Owns `crates/net/src/relay.rs`
  ICE block. Accept: RFC vector fixtures pass; spoofed bindings refused
  100%; punch-or-fallback still within 300 ticks; hash x2. needs: ON2.
- [ ] ON25 net: TURN allocation discipline — Allocate/Refresh lifetimes,
  CreatePermission-before-forward, ChannelBind multiplexing on the relay
  path (webrtc-rs/turn shapes + coturn oracle, reimplemented, no dep).
  Tier C. Size M. Owns `crates/net/src/relay.rs` allocation block.
  Accept: zero unpermitted or stale-allocation forwards in fixtures; ON4
  fallback drill converges; hash x2. needs: ON2.
- [ ] ON26 net: subscription-predicate interest — typed per-connection
  range/view/LOD predicates over aoi.rs with change-driven delta push
  (SpacetimeDB subscription idea, BSL-1.1, reimplemented, no donor code).
  Tier C. Size M. Owns `crates/net/src/interest.rs` scope-predicate
  block. Accept: 64-bot soak ≥3x vs broadcast at ≤128 kbps/client, no
  combat entity starved over 500 ms, hash x2. needs: ON3, ON15.
- [ ] ON27 net: interest-aware relay spike — subscription-scoped
  forwarding on the ON25 allocation path (ideas only, no new donor).
  Tier C. Size S. Owns relay forward-filter test block. Accept: ≥2x
  relay-egress saving on the ON26 fixture with zero unpermitted forwards,
  else killed with numbers. needs: ON25, ON26.
- [ ] ON28 net: soft-collision net prediction — PushPawn-style
  net-predicted organic soft collisions reimplemented (MIT, PORT cite +
  EVIDENCE row). Tier B. Size M. Owns
  `crates/net/src/soft_collision.rs`. Accept: de-sync converges ≤ 3
  frames; p95 input ≤ 50ms. Kill: non-converging desync; p95 over
  budget. needs: ON1, ON3. (RES-STEAL wave 35 Dive 11, reviewed
  2026-09-26; remapped from colliding ON3.)
- [ ] ON29 net: interest production wiring — `session.rs`/`scale.rs`
  call `ConnectionInterest::select` on the live snapshot path with
  the ON3 budgets, keeping the in-test consumer as the oracle. Tier
  B. Size S. Owns call-site hunks in `crates/net/src/session.rs` +
  `scale.rs` (interest API frozen). Accept: 64-bot soak kbps and
  starve numbers equal the in-test oracle x2; no combat entity
  starved over 500ms; hash x2. needs: ON3, ON9. (PLN online,
  reviewed 2026-09-26.)
- [ ] ON30 net: online two-machine played proof — 2 humans + 6 AI over
  QUIC through the ON2 relay converge mid-fight through a scripted
  drop+rejoin; measured loss/latency distributions posted. Tier A.
  Size M. Owns `crates/net/src/quic.rs` + `relay.rs` proof hunks +
  fixtures (no new transport). Accept: played bout converges with
  full checkpoint equality; punch-or-fallback within 300 ticks; hash
  x2 with the MP1 tape. needs: ON1, ON2, MP1. (PLN online, reviewed
  2026-09-26; loss, jitter and reorder are proven on loopback with a
  fault injector (`crates/net/src/wan_soak.rs`); only the real-NAT
  hole-punch leg needs a second network.)
  progress 2026-10-10: loopback only, Accept not met, row stays open. `net-bout on30` (--features quic) PASS: relay and direct bouts converge with full checkpoint equality. Open: real-NAT hole punch across two networks (no relay bin on HEAD yet); the hash x2 with the MP1 tape leg was not run. cargo test -p engine-net --features quic --lib: 261 pass, 1 red: lobby::tests::loopback_two_peers_form_fight_without_typed_addresses (beacon timeout under load; lobby.rs untouched by this work, not chased).
  progress 2026-10-10 (split-claude): real-NAT runner built. `net_relay [--bind 0.0.0.0:7041]` on a host both networks reach (RelayHub: STUN echo, token register, forward from the registered address only). Machine A: `net_bout on30-serve --relay R:7041` prints an invite code; machine B: `net_bout on30-join --relay R:7041 --code CODE`. Both register, learn the public address, trade candidates, punch (bound 300 ticks), then play WAN bouts x2 over the relay and x2 direct when the punch won; each equals its one-process twin. Receipts in `target/on30-nat` (online-bout, checkpoints, drop-rejoin, distributions, punch, real-net). The real leg rides UDP, not QUIC. Three-process loopback smoke: ON30 NAT PASS. Open: one run across two real networks.
  decision 2026-10-10 (owner delegated to Claude): the real-NAT leg counts over the UDP courier; QUIC through the relay is QR1 (LATER, `docs/SPRINT-18-gaps.md`). ON30 ticks on one run across two real networks with the `target/on30-nat` receipts and the hash x2 with the MP1 tape (operator run: `net_relay` on a reachable host, `on30-serve` and `on30-join` on two networks).
- [ ] ON31 net: voice-boundary proof with SN9 — typed tests proving
  `voice.rs` bake-only refusal holds while ON8/ON12 net voice never
  touches the tick, checkpoints or golden. Tier B. Size S. Owns
  boundary test hunks in `crates/audio` + `crates/net` (no new
  codec). Accept: live-synth call refuses with its typed error;
  voice on/off golden identical x2; transport stays ON-owned, bake
  stays audio-owned. needs: ON1, ON8, AU1. (PLN online, reviewed
  2026-09-26; waits ON8.)
- [ ] ON36 net: confirmed-frame discard in P2P rollback — discard
  confirmed frames from the P2P session `history` behind a
  requested-frame guard, bounding memory (spike: 64→11 entries,
  512B→88B on the 64-frame bursty-loss tape, fold/tip/digest
  identical x2, 0 spurious refusals). Size S. Owns discard hunks in
  `crates/net/src/rollback.rs` (window/ring shape frozen). Accept:
  `cargo test -p engine-net --lib rollback` green plus the 64-frame
  bursty-loss tape with history ≤ CAP+pending and fold/tip/digest
  identical x2. Consumer: P2P duel + ON4 soak. needs: ON3.
  (Research-merge wave 90, ON36 spike.)
- [ ] WD3 content: Rome district map — streets, rooftops and a market
  laid out for TV1 traversal fights. Tier C. Size L. Owns
  `content/maps/district/`. Accept: a traversal route crosses the map
  with vault, mantle and climb; AI chases follow nav links. needs: CT9, WD1,
  TV1.
- [ ] WD5 world: day/night and weather cycle driving VX2 and RT4.
  Tier C. Size S. Owns `crates/world/src/cycle.rs`. Accept: the cycle is
  presentation only; the golden hash is unchanged with it on (asserted
  twice with the cycle on); the cycle publishes one `SkyState` per frame
  for render only; sun position feeds the RL4 sky path, never sim.
  needs: VX2.
- [ ] WD7 world: cell publish gate — dual ahead-agent check
  (geometric seam scan + navigational walk) before any cell goes
  live; bad cells refused with the failing check named. Tier C.
  Size S. Owns the gate in `crates/world/src/cells.rs` + fixtures.
  Accept: a cell with a broken seam and a cell with an unreachable
  nav island both refuse before publish; a 9-cell walk holds the
  AS2 budgets. needs: WD1.
- [ ] WD8 world: versioned mesh tiles — per-cell NV1 bakes stitched
  on shared edges with per-tile versions in the mesh bytes, so
  `rebake_mesh_tile` stops returning `Unsupported` and DX1 break
  sets rebake exactly their tiles. Tier C. Size M. Owns tile
  versions in `crates/nav/src/mesh.rs` + `rebake.rs`. Accept: break
  fixtures bump exactly the dirty tiles; untouched tiles keep
  versions; fighters finish then repath on the new version.
  needs: WD1, NV1.
- [ ] WD9 content: district traversal bake — sidecar edge markers
  per district cell baked into NV2 link registries; a route
  fixture crosses the map with vault, mantle and climb while AI
  chases follow nav links. Tier C. Size M. Owns
  `content/maps/district/` sidecars + cook glue. Accept: probe
  fixtures pick TV1-expected actions on district edges; warped
  roots land within 1 cm; replay exact. needs: WD3, TV1.
- [ ] WD11 world: weather presentation contract — rain, wind and fog
  parameters clocked by the WD5 cycle into VX2 clouds, VX5 smoke
  drift and the RL6 wet layer; sim never reads them. Tier C.
  Size S. Owns `crates/world/src/weather.rs`. Accept: a
  day-to-night clip with moving clouds and wet sand; golden hash
  unchanged with weather on, asserted x2. needs: WD5, VX2.
- [ ] WD12 content: map assembly manifests — versioned cell manifest
  bytes (own magic, seed, per-cell asset list) with a provenance
  gate refusing cells whose assets lack `EVIDENCE.md` rows. Tier C.
  Size S. Owns `crates/world/src/manifest.rs`. Accept: ludus cells
  round-trip; one unprovenanced asset refuses the whole cell with
  its path named; manifest bytes identical x2. needs: WD1, CT9.
- [ ] WD13 world: city grammar + map import — flax-mcp road graph,
  grammar interpreter and Overpass parser rewritten in Rust as
  `crates/world` city modules (MIT, PORT cite + EVIDENCE row).
  Tier C. Size M. Owns `crates/world/src/city.rs`. Accept: directed
  2D road graph; interpreter walks rule bodies; OSM JSON parses;
  height cascade resolves. Kill: float in tick; divergence. needs:
  WD1. (RES-STEAL wave 32 flax-mcp, reviewed 2026-09-26.)
- [ ] WD14 world: node-constraint collapse for arena tilings —
  propagate-and-collapse over authored node restrictions (wall /
  rubble / gate / door tiles plus room-adjacency graphs) with a
  fixed backtrack cap, offline at generation time over integer
  seeds (AustinHellerRepo/WaveFunctionCollapse discipline, MIT).
  Tier B. Size S. Owns solver hunks beside `crates/world/src/gen/wfc.rs`
  (builders frozen). Accept: 20 seeds solve >= 18 within the cap at
  p50 <= 100 ms; SP-clean rate at/above builder+repair baseline;
  rerun byte-identical x2; 0 tick code. needs: WD1.
  (PLN47 verdict TAKE; EMPIRE outcome 4.)
- [ ] WD15 world: staged arena-ground bake — ordered seeded passes
  (base → wear → scatter) baking courtyard dirt, rubble scatter and
  worn paths to deterministic tile/prop tables with a preview capture
  (bones-ai/rust-procedural-world discipline, MIT; Bevy/f32 never
  enter). Tier B. Size S. Owns bake hunks beside
  `crates/world/src/ground/` (floor frozen). Accept: 10 bakes
  rerun-identical x2; 0 blocked paths or gates; bake <= 100 ms;
  blind A/B prefers staged over single-pass; 0 tick float.
  needs: WD1, NV3. (PLN47 verdict TAKE; EMPIRE outcome 4.)
