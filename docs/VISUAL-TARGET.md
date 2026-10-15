# VISUAL-TARGET — turn “Flax-level” into reviewable evidence

Status: target specification, not a parity claim. Flax is a visual/workflow
reference; use licensed assets and public feature descriptions, not its source.
Matching one scene is a vertical-slice achievement, not engine-wide parity.

Operator ambition (2026-09-23): Unity/Unreal-quality graphics and animation in
the reference arena. Treat those as art-direction references, not a promise of
engine-wide features. Pin a reference renderer/version, content rights, camera,
lighting, resolution and motion sequence before comparing. Locomotion's concrete
course and ownership are in `docs/plan/animation.md` (LOC1/LOC2); still images
cannot establish animation quality.

## Reference project

Build one reusable art-directed environment: the Roman arena of the M3 slice.
A sunlit sand floor and stands connect to a dim gate tunnel (exterior to
interior), with an animated gladiator, banners, torches, dust and a crowd
backdrop. Add a water trough or glass lamp only when the material chart needs
it. Use the same content in editor, packaged game and automated captures.
Include a material chart and isolated lighting tests; impressive scenery can
hide bugs. The district, siege field and ludus maps (M6) reuse the same chart,
cameras and receipts; the arena stays the first reference. Content sourcing and provenance follow the `ROADMAP.md` content track.

Record asset provenance, scene version, camera/exposure, time-of-day, resolution,
quality settings and reference renderer version. Use fixed-camera stills and
moving-camera clips; judge silhouette, material response, lighting, shadowing,
motion, stability and composition. Reference images are review targets, not
byte-identical GPU goldens or substitutes for licensed source assets.

## Render milestones

| Stage | Required implementation | Acceptance evidence |
|---|---|---|
| V1: correct 3D | Mesh/texture upload, indexed draws, camera/depth, normals/tangents, linear/sRGB handling, metallic-roughness PBR, directional light, exposure/tonemap | Actual imported mesh; material/normal/depth diagnostic views; no validation errors; resize/minimize/recover tests |
| V2: coherent lighting | Directional cascaded shadows, bounded local lights, IBL/environment maps, reflection probes, baked indirect light/lightmaps or probes, AO | Interior/exterior shots; controlled roughness/metalness/exposure charts; no obvious light leaks; shadow transitions reviewed in motion |
| V3: playable quality | Skinning, animation blending, stable temporal AA or measured alternative, motion vectors where required, transparency sorting, foliage/terrain, decals, particles, fog, restrained bloom/color grading | Character/camera-motion clips; disocclusion, ghosting, shimmering and shadow acne tests; slice performance capture |
| V4: scale | Frustum/occlusion culling, LOD, streaming/residency, pipeline cache/warmup, resource pooling; advanced geometry only if profiles justify it | Large-scene trace with before/after CPU/GPU/VRAM; equivalent quality; stable load/unload and device recovery |

Choose a straightforward forward+/clustered or deferred implementation first,
with a decision record and target-device measurements. Render-graph passes
need declared inputs/outputs, resource lifetimes and synchronization; “passes
are reorderable data” does not mean arbitrary reordering is valid.

V1–V4 do not need meshlets, a visibility buffer or ray tracing, so the first
good scene does not wait for them. V5 is the operator's 2040 bar
(2026-09-24) and does need them, on the tiers below.

**Truth rule (2026-09-24).** A look stage counts only when its passes submit
on a real device in the window build. CPU reference math and llvmpipe parity
are merge gates, never look gates. RL1, RL2, RL3 and RP1 were ticked while
every GPU submit returned `Unsupported`; GX1–GX5 on the board make them live
and GX6 makes the proof gate catch this next time.

| Stage | Required implementation | Acceptance evidence |
|---|---|---|
| V5: 2040 look | GPU-driven culling + meshlet virtual geometry (visibility buffer, two-pass Hi-Z); hardware ray tracing through wgpu ray queries: RT shadows, many-light direct light (ReSTIR DI, 250+ shadowed torches), dynamic GI with no bake (ReSTIR GI + world cache), RT reflections; engine denoiser; path-traced reference; froxel volumetric fog and clouds; skin SSS, eyes, hair cards; cinematic camera (physical exposure, bokeh DoF, per-object motion blur, HDR10 output); 20k+ GPU crowd | Day→night clip with no bake step; 250-torch night arena; tunnel leak test; path-traced vs RT-tier stills on the material chart within perceptual tolerance; 100M-source-triangle arena with no LOD pop in motion; per-pass timings per tier on named GPUs |

### Quality tiers

Each tier is picked at startup from the adapter's reported features (RT0) and
is recorded in every receipt. Lower tiers are fallbacks, not the target.

| Tier | Needs | Lighting | Target |
|---|---|---|---|
| Base | any wgpu adapter | cascades + clusters, IBL, baked lightmaps or probe-grid GI, GTAO, SSR-lite | 1080p60 on the reference GPU (the V1–V4 budget below) |
| RT | ray-query capable GPU (the RTX 3050 qualifies) | RT shadows, ReSTIR DI/GI, world cache, RT reflections, denoiser | 1080p output from 540p–720p internal + TAAU, 60 fps goal on the reference GPU, measured, not promised |
| Ultra | high-end RT GPU (second reference GPU when the operator names one) | RT tier at higher internal resolution + more bounces; path tracer for photo mode | 1440p–4K output; photo mode converges in seconds |

Everything V5 adds is presentation only: it never feeds the tick, so replay
and golden hashes stay tier-independent (`HASH-LAW.md`). Vendor denoisers and
upscalers (DLSS-RR, FSR Ray Regeneration) are optional adapters behind a
feature flag after a license row; the engine's own denoiser and TAAU ship by
default.

Research grounding (accessed 2026-09-24, patterns only, never source): UE 5.7
MegaLights and Nanite Foliage, UE 5.8 MegaLights and Iris production-ready;
Bevy Solari (ReSTIR DI/GI, world cache, simplified RT proxies beside
full-resolution raster) and Bevy virtual geometry (BVH culling); id Tech 8
dynamic GI replacing ~110 GB of baked data (SIGGRAPH 2025); wgpu 30
experimental ray tracing; neural texture compression and cooperative vectors
(DirectX SM 6.9, not in wgpu yet). Rows cite papers, not engine code.

## Proposed desktop budget

Target: 1920×1080 presentation, 60 fps, raster quality tier on the NVIDIA
GeForce RTX 3050 6GB Laptop GPU selected in SPRINT O5. Earlier smoke reported
Vulkan/driver 610.62; each new receipt must record its actual adapter, driver,
CPU, power mode and internal render resolution. These are targets, not measured
performance. Upscaling may use a lower internal resolution only when declared;
R6's actual implementation/license remains to be verified. No 8 GiB device is
assumed. Measure VRAM against the device's reported available budget.

- CPU critical-path frame p95 ≤ 8 ms; GPU frame p95 ≤ 14 ms.
- Presented frame interval p99 ≤ 25 ms after warmup; report stutter count and
  max separately. CPU and GPU overlap, so do not add their times as frame time.
- Simulation 1,000-agent stress fixture p99 ≤ 2 ms on the named CPU, measured
  separately from the content slice and with an explicit workload definition.
- Proposed resident GPU allocation ceiling ≤ 4.5 GiB on this 6 GB tier, leaving
  driver/OS headroom; report transient pools, textures and geometry separately.
  Calibrate with V1 evidence rather than silently raising the ceiling.
- Capture ≥ 2,000 steady-state frames after a documented warmup, plus a separate
  cold-load/first-use trace. Record vsync, upscaling and dynamic resolution state.

If hardware is unavailable, build and CPU shader/math tests can pass; visual
quality and GPU performance stay **unverified**. Software rendering is useful
for smoke/regression checks, not proof of desktop GPU performance.

## Gate types

1. Compile supported backend/feature combinations.
2. Run actual GPU work and verify an adapter/device was created, passes were
   submitted and validation was clean; initialization failures must fail the run.
3. Compare fixed captures within documented image/perceptual tolerances on a
   pinned test environment; human-review intentional reference updates.
4. Review temporal clips and material/lighting calibration scenes.
5. Measure per-pass GPU and CPU timings, VRAM, draw/triangle counts and hitches.
6. Run the same simulation inputs under null and GPU presentation and compare
   simulation checkpoints. Pixel equality is not required across vendors.

For animation, capture the same LOC2 input tape through GM1/R3: starts/stops,
strafing, pivots, slopes/steps, airborne/landing and combat interrupts. Include
foot-contact diagnostics, root/capsule displacement and replay checkpoints.
Declare fixed quality for baseline captures; adaptive runs separately record
resolution/LOD changes. Missing content, skinning or captures remain failed or
unverified, not a lower-quality substitute for the planned gate.

Keep render receipts with scene/content/build hashes and hardware metadata.
`examples/window.rs` currently demonstrates procedural sprites; its FPS and the
CPU lighting helper tests do not establish any V1–V4 exit by themselves.
