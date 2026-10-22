# Render: five steals from real donor source (2026-10-23)

Question this note answers: which four GPU passes in filament, bevy, godot and
kajiya are worth porting into `crates/render`, and what does engine2040 already
have? Every donor claim below has a pinned sha and a quoted line. Every local
claim has a `file:line`. Nothing here was built, tested or captured.

## Method

Commit shas came from each repo's `commits/<branch>.atom` feed on 2026-10-23,
then each file was fetched at that sha and read line by line.

| Repo | Branch | Pinned sha | Read |
|---|---|---|---|
| google/filament | main | `479ee3548410f21c038e50f1fe9335fe09eb433b` | `filament/src/Froxelizer.cpp` (1062 L), `Froxelizer.h` |
| bevyengine/bevy | main | `84e2d2cc5adaeff7547e44f434a0d0d7a20345b8` | `crates/bevy_pbr/src/ssr/mod.rs`, `ssr/raymarch.wesl` (556 L), `volumetric_fog/mod.rs` |
| godotengine/godot | master | `9c2e39242e20846f84d990fe457a5e146b77fa36` | `servers/rendering/renderer_rd/shaders/effects/bokeh_dof.glsl`, `effects/ssao.glsl` |
| EmbarkStudios/kajiya | main | `9fdec0f074f9f822d395f9cfd041b58749e32303` | `assets/shaders/motion_blur/motion_blur.hlsl`, shader dir listing |
| mrdoob/three.js | dev | `0cef1806db67a3fd7de4f29fb91797b03aedb324` | `examples/jsm/tsl/display/SSGINode.js` |

All five are permissive (Apache-2.0 / MIT / MIT-or-Apache). AGENTS rule 4 says
donor code may be ported; these notes still ask for shape, not a vendored file,
because every local pass carries a CPU oracle and a receipt.

## engine2040 today

| Pass | Where | Honest state |
|---|---|---|
| Sun cascades | `crates/render/src/shadow_cascade.rs:960` (`blend_weights_partition`) | Real: 1-4 cascades, 1024-4096 px edge, seam blend pinned by test |
| GTAO | `crates/render/src/gtao.rs:28-54` | Real: half res, 4 slices x 3 taps, f64 CPU oracle, 0.002 parity, 0.8 ms budget |
| TAA | `crates/render/src/post/taa.rs:10-22` | Real: 3x3 min/max history clamp, history-valid flag, pixel motion cutoff |
| Bloom, exposure, ACES | `crates/render/src/post/chain.rs:4-10` | Real, CPU chain plus device pass |
| IBL | `crates/render/src/shaders/forward.wgsl:110-113`, `:295` (`ibl_specular`) | Real: prefilter chain, BRDF LUT, SH irradiance. The only reflection source we have |
| Torch lights | `crates/render/src/torch_light.rs:24` (`TORCH_MAX = 8`), `shaders/forward.wgsl:56` (`array<TorchU, 8>`) | Capped at 8 uniform slots, summed linearly per fragment |
| Cluster assign, CPU | `crates/render/src/shadow_cluster.rs:30-34` | Real CPU pass: 16 px tiles, 256 lights, 24 per cell |
| Cluster assign, GPU | `crates/render/src/shaders/cluster.wgsl:28` (`cell_counts[cell] = light_cells[cell % 8u];`) | Stub. No pipeline compiles it; only a brace-balance test reads it (`shadow_cascade.rs:979`) |
| SSAO shader | `crates/render/src/shaders/ssao.wgsl:31` | Stub: a 3-tap depth blur, no hemisphere samples. Not compiled |
| Height fog | `crates/render/src/fog.rs:1` | Real CPU height fog plus contact shadows. Never reads the sun shadow map |
| Volumetric fog shader | `crates/render/src/shaders/fog_vol.wgsl:93` (`vol_main`) | Written, but no Rust file names `fog_vol` (grep 0 hits): dead file |
| Meshlets | `crates/render/src/meshlet.rs:12` (`MESHLET_IDEAS_ENABLED_DEFAULT = false`) | Ideas gate off, no draw path |
| GPU cull | `crates/render/src/gpu_cull.rs:10-14` | Proof path only, its own doc says "No draw path uses it yet" |
| ReSTIR DI | `crates/render/src/restir_di.rs:1` | CPU reservoir oracle plus denoise cost. Not a GPU pass |
| GI probes | `crates/render/src/gi_probe.rs:1` | Its own first line: "dynamic GI probe stub" |
| SSR | `crates/render/src/quality.rs:249` | The name sits in `CUT_ORDER`. No pass, no shader, no knob |
| Motion blur | `crates/render/src/quality.rs:248` | Ditto: `CUT_ORDER` only |
| DOF, SSGI, froxel | none | grep `bokeh`, `depth_of_field`, `ssgi`, `froxel` over `crates/` = 0 hits each |

The pattern: the quality ladder advertises cuts for two passes that do not
exist, and the light path assigns 256 lights on the CPU while the shader shades
8.

## The steals, ranked

### 1. Froxel light assignment, then shade from the cell list
Donor: filament, `froxelizePointAndSpotLight`.
https://github.com/google/filament/blob/479ee3548410f21c038e50f1fe9335fe09eb433b/filament/src/Froxelizer.cpp#L929-L1001

```cpp
if (UTILS_LIKELY(iz != zcenter)) {
    cz = spherePlaneIntersection(s, (iz < zcenter) ? planesZ[iz + 1] : planesZ[iz]);
}
if (cz.w > 0) { // intersection of light with this plane (slice)
    // the sphere (light) intersects this slice's plane, and we now have a new smaller
    // sphere centered there. Now, find x & y slices that contain the sphere's center
```

What it buys: exact light-in-cell assignment for 250 torches, written as one
byte per (light, cell) record (`Froxelizer.h#L163`, `using RecordBufferType =
uint8_t;`), so the fragment shader reads a short contiguous list instead of
looping every light. The trick worth taking is the shrinking sphere: each z
plane clips the sphere, and the smaller sphere is reused for the y and x tests,
so a big torch near the camera costs a handful of plane tests, not a
light-by-cell brute force.

Local gap: `torch_light.rs:24` caps the frame at 8 lights and
`forward.wgsl:56` is `array<TorchU, 8>`. The CPU already assigns 24 lights per
cell (`shadow_cluster.rs:34`), so today 16 of those 24 never reach a pixel.
`cluster.wgsl:28` is the placeholder that should do this work.

Size M. Risk: needs a storage buffer plus a new bind group in the forward pass
(`forward.wgsl` currently has group 0 bindings 0-19 and group 1 binding 0 only),
and a prefix sum over cell counts. The ladder already holds a *CPU cost* cut on
this shape (CANDIDATE ACCEPTED, `docs/research/LADDER.md:164`, range-tightened
z-span). This steal is the half it does not cover: the shading read.

**Landed (headless half)** `e3ed8c819`: `shadow_cluster/cluster_mask.rs` packs
the finished CPU assignment into Filament's light-group shape, one `u32` per 32
lights per cell (`mask_groups(MAX_TORCH_LIGHTS)` = 8 words a cell), and
`cluster.wgsl` is now the real reader (`cell_of`, `mask_word`, `light_lit`,
`lights_in`, `resolve`) instead of the `cell % 8u` stub. Proof:
`node tools/gh-check.mjs cargo test -p engine-render --lib cluster_mask` 4
passed; `... --features shader-validate cluster_wgsl` 1 passed (naga, no GPU);
`cargo clippy -p engine-render --all-targets -- -D warnings` pass. Left: no
frame binds the table, and `forward.wgsl` still reads its 8-slot torch uniform,
so no pixel is different yet. That half needs a device (a capture, not a read).

### 2. Motion blur from a tile-max velocity buffer
Donor: kajiya, `motion_blur.hlsl`.
https://github.com/EmbarkStudios/kajiya/blob/9fdec0f074f9f822d395f9cfd041b58749e32303/assets/shaders/motion_blur/motion_blur.hlsl#L42-L60

```hlsl
// Scramble tile coordinates to diffuse the tile quantization in noise
...
const float2 tile_velocity = blur_scale * tile_velocity_tex[clamp(velocity_tile_coord, int2(0, 0), int2(depth_tex_size.xy) - 1) / 16];
```

What it buys: a scatter-as-gather blur whose kernel length comes from the max
velocity in a 16 px tile (`velocity_reduce_x/y` build it), plus a dithered
sample position so the tap pattern does not band. The depth and spread weights
(`sample_weight`, L23-27) keep a sharp foreground from smearing into a fast
background.

Local gap: no pass. `quality.rs:248` names `motion_blur` in the cut order and
nothing implements it. We do have per-pixel flow already
(`post_jitter_motion.rs:85`, "`motion = current_pos - history_pos`"), which is
camera reprojection; a fighter's own velocity needs the previous instance
matrix, which the draw path can carry beside `draw_id.rs`.

Size S-M: two small reductions plus one full-screen pass on the HDR target.
Risk: v1 blurs camera motion only. Say so in the row, and do not call it
per-object until the instance velocity exists.

### 3. Volumetric fog that samples the sun shadow map
Donor: bevy, `volumetric_fog`.
https://github.com/bevyengine/bevy/blob/84e2d2cc5adaeff7547e44f434a0d0d7a20345b8/crates/bevy_pbr/src/volumetric_fog/mod.rs#L18-L24

```
//! post]. It uses raymarching in screen space, transformed into shadow map
//! space for sampling and combined with physically-based modeling of absorption
//! and scattering. Bevy employs the widely-used [Henyey-Greenstein phase
```

What it buys: shafts. Light beams through the arena arches at dawn and torch
glow in the dark are the identity of the Rome scene, and they need exactly one
thing our fog lacks: a shadow-map lookup per march step. Bevy also names the
cheap part of the model, Henyey-Greenstein phase, which `fog_vol.wgsl:72`
(`vol_glow`) already approximates.

Local gap: `fog.rs:1` is height fog plus contact shadows with no sun visibility.
`shaders/fog_vol.wgsl:93` does have a per-pixel march and a HG-ish glow, but no
Rust file includes it, so it has never run. The sun shadow map is already bound
in the forward pass (`forward.wgsl:122`), so the sample is in reach.

Size M, because the first step is wiring a dead shader (bind group, pass
registration, one GPU gate) before the shadow tap.
Risk: the march must stay at a fixed step count and must not touch the golden
hash (presentation only). If a per-step depth tap costs more than the fog
budget, fall back to a shadow lookup at the halfway point.

### 4. Depth of field with a signed circle of confusion
Donor: godot, `bokeh_dof.glsl`.
https://github.com/godotengine/godot/blob/9c2e39242e20846f84d990fe457a5e146b77fa36/servers/rendering/renderer_rd/shaders/effects/bokeh_dof.glsl#L212-L228

```glsl
if (bokeh.a < color.a) {
    mix_amount = min(1.0, max(0.0, max(abs(color.a), abs(bokeh.a)) - DEPTH_GAP));
} else {
    mix_amount = min(1.0, max(0.0, abs(color.a) - DEPTH_GAP));
}
color.rgb = mix(color.rgb, bokeh.rgb, mix_amount); //blend between hires and lowres
```

What it buys: the composite rule that makes DOF safe. Alpha carries a *signed*
CoC (negative near, positive far, L47-58), so a sharp fighter in front of a
blurred crowd is never overdrawn by the blurred buffer. Godot also ships the
physical CoC form beside the artist one, and a golden-angle circular gather
(L193-205) that keeps the bokeh disc round at 16 taps.

Local gap: no DOF (grep 0). The post chain stops at bloom, exposure, ACES,
grading (`post/chain.rs:4`), and `blit.wgsl` tone-maps once.
Size M: CoC pass, one half-res blur, composite. Risk: low. The camera is a
chase cam, so near blur must be off or the fighter blurs; start far-only.

### 5. Screen-space reflections with a hybrid root finder
Donor: bevy, `raymarch.wesl` (a port of Tomasz Stachowiak's `raymarch.hlsl`).
https://github.com/bevyengine/bevy/blob/84e2d2cc5adaeff7547e44f434a0d0d7a20345b8/crates/bevy_pbr/src/ssr/raymarch.wesl#L203-L217

```wgsl
if ((*root_finder).use_secant) {
    // Finish with one application of the secant method
    let total_d = min_d.distance + -max_d.distance;
    let mid_t = mix(min_t, max_t, min_d.distance / total_d);
```

What it buys: the cheapest correct march. Linear steps with an exponent that
packs samples near the camera, a binary search to bracket, then one secant step
to land the hit, with a thickness test that lets a ray pass a too-thin
occluder (L313, `res.valid = res.penetration < (*distance_fn).depth_thickness;`).

Local gap: `quality.rs:249` names `ssr` in `CUT_ORDER`; there is no pass. The
only reflection is IBL (`forward.wgsl:295`), which cannot reflect a fighter.
Size L, not M: bevy's own doc says SSR "currently require[s] deferred rendering"
(`ssr/mod.rs:59`), and we are forward-only with a depth prepass. We would
reconstruct normals from depth (`forward.wgsl:426` `vs_depth`) or add a normal
target. That is the real cost, not the march.
Risk: screen-edge dropout and ghosting under TAA. Gate it on `Gtao`-style tier
knobs, not on by default.

## What not to steal

- **three.js SSGI now.** `SSGINode.js:156` needs a constant world-space
  thickness and the node requires `rg11b10ufloat-renderable` (L416); it also
  leans on MRT plus a temporal filter (`spatial_filter.hlsl` /
  `temporal_filter.hlsl` in kajiya's `assets/shaders/ssgi/`). We have baked SH
  GI (`gi.rs`) and IBL. Size L for a look gain SSR would already deliver. Read
  it again after SSR lands.
- **Godot's 32-tap adaptive SSAO as a replacement.** `ssao.glsl:26`
  (`INTELSSAO_MAIN_DISK_SAMPLE_COUNT (32)`) plus depth mips and a tap counter.
  Our GTAO is real, has a CPU oracle and a 0.002 parity gate (`gtao.rs:54`).
  Replacing it would put that gate at risk for no measured win. The one idea
  worth a later look is the depth-mip chain for a large radius.
- **Filament's 3D froxel fog volume.** It costs a 3D transient our RA2 alias
  budget does not have, for a scene whose fog is a height ramp. Do shafts
  (steal 3) before a volume.
- **Virtual shadow maps and virtual geometry.** `meshlet.rs:12` is a gated-off
  sketch and `gpu_cull.rs:14` says no draw path uses it. Fixing the draw path
  comes first.
- **A shadow atlas for all 250 torches.** 24 lights per cell is already the
  cap; shadow maps for 250 cost memory we do not have a budget line for.

## Could not verify

- GitHub's API was rate-limited (`/rate_limit` answered `core remaining: 0`),
  so no `default_branch` or license metadata was read from the API. The shas
  come from each repo's commits feed and the files were fetched at those shas.
  A sha read from a feed is not a signature; treat it as a pin, not a proof.
- Filament's `FROXEL_SLICE_COUNT` value: not present in `Froxelizer.h` under
  that name in my grep. No slice-count number is claimed here.
- Whether wgpu 30 on our adapter allows the storage buffer read in the forward
  fragment stage at 24 records per cell without a new limit. Needs a device
  probe, not a read.
- Ran nothing for this survey: no `cargo check`, no shader compile, no GPU run,
  no capture. Every "real" in the table above is read from source and its own
  doc comment. After the landing (`e3ed8c819`) the mask tests and the naga gate
  ran on a runner, headless; still no GPU, no window, no pixel.

## Cards

Same shape as the head cards in `docs/research/LADDER.md`. Read only; LADDER.md
was not edited.

- CARD | P4 | shade 250 torches from a per-froxel light record list, shrinking-sphere assignment | PORT(google/filament@479ee35), Apache-2.0 read live 2026-10-23 at pinned sha, shape only, no bytes vendored | not run: arena with 250 torches, baseline 8 uniform slots -> froxel record list, lit-light count per cell equal to the CPU oracle, per-fragment light iterations p95 down, pixels differ only on newly lit cells, golden d57b5c8ae3096614 unmoved | spike first (R23-FROX)
- CARD | P4 | tile-max velocity motion blur with dithered gather and depth/spread weights | PORT(EmbarkStudios/kajiya@9fdec0f), MIT-or-Apache read live 2026-10-23 at pinned sha, shape only | not run: Q1 capture of a camera whip, baseline sharp -> blur on, 16 px tile max velocity equals the CPU reduce, a sharp foreground keeps its edge, median ms at 1080p, golden unmoved | spike first (R23-MBLUR)
- CARD | P4 | volumetric fog that samples the sun shadow map per step, so arches cast shafts | PORT(bevyengine/bevy@84e2d2c), MIT read live 2026-10-23 at pinned sha, shape only | not run: dawn arena capture, baseline height fog -> shadowed march, a shaft appears behind an arch and not inside it, fog_vol.wgsl runs with 0 validation errors, ms cost, golden unmoved | spike first (R23-SHAFT)
- CARD | P4 | depth of field with a signed CoC and a far-only composite | PORT(godotengine/godot@9c2e392), MIT read live 2026-10-23 at pinned sha, shape only | not run: chase-cam capture, baseline sharp crowd -> far DOF, the fighter's silhouette edge is unchanged pixel for pixel, bokeh disc round at 16 taps, ms cost, golden unmoved | spike first (R23-DOF)
- CARD | P4 | screen-space reflections with a linear-plus-bisection-plus-secant root finder and a thickness test | PORT(bevyengine/bevy@84e2d2c), MIT read live 2026-10-23 at pinned sha, shape only | not run: marble and armor frame, baseline IBL only -> SSR, a fighter reflects in the polished floor, hit rate on a known probe set, ghosting under TAA measured, ms cost | spike first (R23-SSR), after normals exist
