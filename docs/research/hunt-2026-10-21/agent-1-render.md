# Hunt 1: the lit frame and the look (agent-1-render)

Date: 2026-10-21. Scope: gap B7 (finish the lit frame), B8 (machine eyes), B9 (temporal
artifacts), rows LF1-LF8, EG11-EG13, RL46/RL52/RL53, plus the board rows that the
campaign opened for the same work (PW01 velocity target, PW03 depth+normal prepass,
NS11w landing 2). Read-only hunt: no build, no edit under `crates/`.

## Method

- Search API through `gh api` (repo search + code search). Code search by `filename:*.wgsl`
  was the sharpest tool: it finds WGSL files nobody writes about.
- Every file named below was fetched and read. Nothing is proposed on the strength of a
  repo description.
- Cross-checked against `docs/research/steals-2026-10-11|-12|-18|-20/` and
  `hunt-2026-10-21/donors-seen.txt`.
  - `kiss3d`, `pbrt-r4`, `Helio`, `ZirconEngine` were **never read** before
    (`steals-2026-10-18/06-new-engines.md` line 33: "kiss3d ... name-checked by metadata
    only, no directory read"), so they are fresh.
  - `panxinmiao/myth` was LISTING only, so its `blue_noise.wgsl` is unread, but myth loses
    on size (84 KB per file) so it is not ranked.
  - Do-not-mine list respected: bevy, godot, o3de, Filament, Falcor, WickedEngine, Dawn,
    renderling, Daedalus, meshoptimizer, XeGTAO, rend3, kajiya, Granite, zenfg, NVIDIA-RTX,
    KhronosGroup, wgpu, naga, wesl-rs, rust-av/ssimulacra2, NVlabs/flip.
- Our tree checked first. What we already own (so no donor is proposed for it):
  - `crates/render/src/gtao.rs` (half-res GTAO compute + bilateral upsample + a **CPU oracle
    twin** with a parity test), `crates/render/src/post/taa.rs` + `shaders/post/taa_resolve.wgsl`
    (history target + a **plain min/max box clamp** + mix, no YCoCg, no Catmull-Rom, no
    disocclusion, no jitter reconstruction), `crates/render/src/post/chain.rs`
    (bloom/exposure/grade/tonemap on the CPU), `crates/render/src/post/auto_exposure.rs`
    + `present/gpu_present/auto_exposure_gpu.rs` (GPU histogram),
    `crates/render/src/decals.rs` (CPU-placed projected decals with depth reject + atlas,
    **no scene draw**), `crates/render/src/particles_gpu.rs` + `particle_draw_gpu.rs`
    (CPU pool + draw records, **not drawn in the scene**), `crates/render/src/gpu_cull.rs`
    + `shaders/cull.wgsl` (Hi-Z, wall-only), `crates/render/src/shadow_cluster.rs`
    (clustered shadow on the CPU), `crates/render/src/core.rs` (headless Forward+ on the CPU).
  - `crates/material/src/compile.rs` + `twin.rs` + `skin.rs` (one BRDF, Charlie sheen,
    terminator LUT) and `crates/metric/src/metric.rs` (`metric::flip`) already exist.

---

## 1. Ranked steals (best first)

| # | repo (owner/repo) | stars | lic | donor file path | what we take | our crate + row/gate | size | risk |
|---|---|---|---|---|---|---|---|---|
| 1 | Siroko/kansei | 28 | MIT | `rust/kansei-core/src/shaders/taa_resolve.wgsl` (10.9K, 231 lines) + `motion_vectors.wgsl` (1.8K) | A complete Karis TAA **and** TAAU resolve in WGSL: Karis reversible tonemap working space, YCoCg, **Playdead clip-to-box** variance clip, **Catmull-Rom history from 5 bilinear taps**, **closest-depth velocity dilation** (thin edges keep their motion), **disocclusion by history view-depth mismatch** (the history's alpha carries last frame's view depth), **jitter-aware 3x3 Gaussian reconstruction** (`exp(-2.29 d^2)`, Blackman-Harris fit) and **temporal upsampling** (clip box widens by the upscale factor, the disocclusion test accepts any of the 4 rendered pixels around the output pixel, the current frame's share scales with the reconstruction weight). `motion_vectors.wgsl` gives the mesh-side rule: current clip minus previous clip in uv units, with the previous bone/instance transform beside the world matrix. | `crates/render/src/shaders/post/taa_resolve.wgsl` + `crates/render/src/post/taa.rs` + `crates/render/src/present/gpu_present/submit.rs`. Rows LF5c (resolve), LF5b (jitter), LF5 (motion vectors), PW01. Gate G3 (TAA LIVE), B9 | M | Our resolve is a **fragment** post pass, kansei's is a **compute** pass writing a storage texture. Port the math, keep our bind layout. Two things to re-derive: our history is `rgba16float` with no room for depth (alpha is free - use it), and our TAA currently has no velocity target at all, so LF5/LF5b are hard prerequisites. Our `TaaParams` CPU twin must mirror the new uniform or the parity test breaks. |
| 2 | pannapudi/voidin | 77 | MIT | `shaders/reproject.wgsl` (1.4K), `shaders/visibility.wgsl` (2.6K), `shaders/emit_draws.wgsl` (1.8K), `shaders/taa.wgsl` (3.4K) | A 180-blob **GPU-driven wgpu renderer** with the three passes we are missing: (a) **velocity from a depth prepass with jitter removed**: `velocity = (curr_ndc + camera.jitter) - (prev_ndc + camera.prev_jitter)`, and it stores a **disocclusion flag in .z** (`limits` = the previous position was inside the screen) that the TAA shader reads as `mix(1.0, 1.0/12.0, velocity.z)` to drop history; (b) a **G-buffer prepass** that writes octahedral-encoded world normals + packed UVs to one `u32` target and a flat material id to a second target; (c) **GPU frustum cull + indirect draw**: one compute thread per instance writes a `DrawIndexedIndirect` with `instance_count = 0` when the bounding sphere is outside the frustum. Its TAA is a YCbCr variance clip with a Mitchell-Netravali-filtered centre. | `crates/render/src/shaders/velocity.wgsl` (new), `crates/render/src/shaders/prepass.wgsl` (new), `crates/render/src/shaders/cull.wgsl`, `crates/render/src/present/gpu_present/submit.rs`. Rows **PW01** (velocity target), **PW03** (depth + normal prepass, AO on ambient only), LF3, B7 | M | voidin is 2D-ish (its Y is up, its world matrix conventions differ) and its octahedral pack must match our `ogdakke`/`encoding` conventions. Its depth is a plain depth texture; our frame has an MSAA path (EG13) where a depth **texture** attachment is unavailable - we need the resolved depth. The `DrawIndexedIndirect` struct layout must match wgpu's `IndirectDrawArgs`. |
| 3 | dimforge/kiss3d | 1790 | BSD-3-Clause | `src/builtin/clustered_build.wgsl` (2.5K), `clustered_cull.wgsl` (2.8K), `hdr_bloom.wgsl` (4.4K), `tonemap_ops.wgsl` (3.9K), `auto_exposure_meter.wgsl` (1.1K), `auto_exposure_adapt.wgsl` (1.7K) | A clean **Forward+ in WGSL in a Rust engine**: one thread per froxel builds the view-space AABB from the inverse projection (screen tile corners x exponential depth slice, `z = -near * (far/near)^(k/Z)`), one thread per froxel tests every light's bounding sphere against that AABB and writes a contiguous slice of a global light-index list plus `(offset, count)`. No atomics (fixed 256-light slice per cluster). Also the dual-filter **Kawase** bloom (soft-knee prefilter, 13-tap down, 9-tap tent up) and a geometric-mean **auto-exposure** meter (32x32 log-luminance grid into a 1x1 R16Float) with a dt-correct exponential adaptation pass. | `crates/render/src/shadow_cluster.rs` + `crates/render/src/core.rs` (make the Forward+ cull live on the GPU), `crates/render/src/post/gpu.rs` (bloom), `crates/render/src/post/auto_exposure.rs`. Rows **EG12** (ambient and torches), LF4, LF2; gates G2 (speed), G9 (potato) | S-M | kiss3d is Rust + wgpu, so the port is nearly verbatim; but its cluster layout gives each cluster a **fixed 256-light slice**, which is 1 MB per 8x8x24 grid - we want a shared list with an atomic counter, or the GPU VRAM bar (G9) breaks. Our forward+ is a **CPU** headless path with a visible hash (`core.rs`); the GPU path must not change the hash, so keep the CPU path and add the GPU one beside it. Our bloom already exists (LF4 landed 99e624b) - take the 13-tap/9-tap weights only if our current bloom is visibly wrong. |
| 4 | mewhhaha/pen | 0 | MIT | `crates/pen-render/src/shaders/decals.wgsl` (2.5K, 71 lines) | The **box-projected instanced decal draw** in 71 WGSL lines: one instance per decal, 36-vertex cube, `decal.model` transforms it, the fragment reads the **depth buffer**, reconstructs the world position with `inverse_view_projection`, projects it into decal-local space, discards outside the box, samples the **decal atlas** with `local.xz * 0.5 + 0.5`, and offers two blend modes: `fragment` (multiplied over the surface, so the surface's lighting and shadow show through) and `paint` (premultiplied, in its own colour). The `local.y` fade (`1 - smoothstep(0.6, 1.0, abs(local.y))`) stops the decal bleeding onto walls. | `crates/render/src/shaders/decal.wgsl` (new) + `crates/render/src/decals.rs` + `crates/render/src/present/gpu_present/submit.rs`. Rows **LF7** (blood on the sand), NS11w landing 2 (the decal draw hunk), B7 | S | Our `decals.rs` already places decals on the CPU with a depth-reject footprint and owns a `DecalAtlas`; this donor replaces the **placement** with a **draw**, so the CPU pool becomes a GPU draw list (one storage buffer of decal transforms). Needs the depth texture of the frame, which on the MSAA path must be the resolved depth. Multiply-vs-paint decides whether blood darkens the sand or glows: blood is paint (premultiplied), soot is multiply. |
| 5 | 5unekku/Lunar | 0 | Apache-2.0 | `crates/lunar-render-3d/src/motion_vector.wgsl` (2.1K), `crates/lunar-render-3d/src/decal.wgsl` (3.5K) | The smallest honest **camera-only motion vector** pass (61 lines): depth -> world via `inv_view_proj` -> previous clip via `prev_view_proj` -> `(cur_ndc - prev_ndc)` into an `Rg16Float`. Plus the canonical **box-projected deferred decal** (Wili "Deferred Decals" ShaderX7; MacDonald "Practical Decals" GDC 2012) with the cube built in the vertex shader from `vertex_index` (no vertex buffer) and the edge fade `1 - smoothstep(0.4, 0.5, max(abs(local.x), abs(local.z)))`. | `crates/render/src/shaders/motion_vector.wgsl` (new) + `crates/render/src/decals.rs`. Rows **LF5** (motion vectors), LF7; gates G3, G10 | S | Camera-only motion: correct for the static arena, wrong for the 8 skinned fighters, which need per-vertex previous-clip (take kansei #1 for that half). Lunar is a 4 KB crate with no README, so its conventions are undocumented - read the Rust side before trusting the WGSL. Its decal is the same technique as #4 but 1.4K bigger and with no atlas; take #4 for the draw, Lunar only for the citation trail. |
| 6 | leighteg/helmer | 39 | MIT | `helmer_render/src/graphics/shaders/hiz_cull.wgsl` (4.2K, 132 lines), `ssgi_upsample.wgsl` (2.6K) | A **temporal Hi-Z occlusion cull** in one compute pass: 8 corners of the world AABB -> previous *and* current clip -> a conservative screen rect -> the **mip level chosen from the rect's largest dimension** -> the 4 corner texels of the Hi-Z at that mip -> `min` of the 4 -> `occluded = max_depth + bias < hiz_depth` -> writes a visibility flag. This is a real replacement for our wall-only `hiz_hidden_wall`. The second file is a **joint bilateral upsample** (5x5, depth sigma 0.01, normal sigma 0.1, `acos(dot)` for the normal term) that lifts a half-res field to full res with no bleeding across silhouettes - the exact shape our GTAO upsample needs. | `crates/render/src/shaders/cull.wgsl` (replace `hiz_hidden_wall`), `crates/render/src/gpu_cull.rs`, `crates/render/src/gtao.rs` (upsample). Row **B7** (live cull), LF3; gate G2 | S-M | The temporal part needs **last frame's** rect, so the cull result is one frame stale - fine for a standing crowd, wrong for a fighter crossing a doorway. Our Hi-Z is built per frame from the depth prepass; if PW03 lands first, the mip chain is already there. The bilateral upsample needs the **normal** target, which is PW03's other half - so #6 is downstream of #2. |
| 7 | redcamel/RedGPU | 383 | MIT | `src/display/instancingMesh/shader/instanceCullingCompute.wgsl` (3.5K), `src/light/clusterLight/pass/light/PassClustersLight.wgsl` (5.3K) | One compute pass that does **frustum cull + distance LOD select + atomic indirect draw args**: a sphere-vs-6-planes test, `calculateLODLevel` from a distance list, then `aliveIndex = atomicAdd(&indirectDrawBuffer[lodLevel].instanceCount, 1u)` and a visibility buffer slot per live instance. The per-LOD indirect args mean one draw call per LOD, and the instance count is the live count, so the CPU never touches the draw list. | `crates/render/src/gpu_cull.rs` + `crates/render/src/present/gpu_present/submit.rs` + `crates/render/src/shaders/cull.wgsl`. Row **B7** (cull, sort and instancing in the live draw loop), QW10 (draw-call receipt); gate G2 | M | RedGPU is a TypeScript WebGPU library; the WGSL is portable but the host side is TS, so we write our own Rust submitter. The `atomicAdd` into `instanceCount` means the args buffer must be **cleared every frame** (a clear-dispatch or a write of zero) - a classic source of one-frame ghosts. Our `DrawItem`/`draw_id.rs` layout has to change to an indirect path. |
| 8 | greggman/webgpu-driving | 3 | **none** | `src/shaders/post/taa.wgsl` (3.6K), `src/shaders/clusters.wgsl` (1.8K) | Two compact reference shaders worth reading and re-deriving: a TAA with **velocity dilation to the closest-depth neighbour**, a **mirror-glass escape hatch** (a velocity sentinel `> 32.0` means "no history"), Karis tonemapped neighbourhood statistics in YCoCg, AABB clip toward the mean, and an alpha that rises with the screen-space speed; and a **16x9x24 froxel cluster build** in 58 lines with the light list packed as `[count, index...]` per froxel. | algorithm reference only; the code that lands comes from #1 and #3. Rows LF5c, EG12 | S | **No LICENSE file** (GitHub reports `none`): all rights reserved. Treat as reading, not lifting. Eric Lengyel is the author of the clustered-forward literature, so the *method* is safe to re-derive from the papers. |
| 9 | DoubleStyx/Renderide | 80 | MIT | `crates/renderide/shaders/modules/post/gtao_math.wgsl` (3.5K), `passes/post/gtao_denoise.wgsl` (4.2K), `passes/post/gtao_main.wgsl` (14.2K), `passes/backend/gtao_view_normals.wgsl` (1.6K), `passes/post/agx_tonemap.wgsl` (2.7K), `passes/compute/auto_exposure_histogram.wgsl` (4.4K) | A **real GTAO**, not our fixed-slice reimplementation: `multi_bounce_fit` (the Jimenez multi-bounce curve `(2.0404a - 0.3324)`, `(4.7951a - 0.6417)`, `(2.7552a + 0.6903)`), `calculate_view_normal` from the four screen-space edges with an acceptance mask, a **Hilbert-index spatio-temporal noise** (64x64 Hilbert, 64-frame cycle) for the per-pixel rotation, a depth-mip-aware `sample_mip_for_offset`, and a depth-aware **denoiser** with edge symmetricisation, an edge-leak step and diagonal weights derived from the cardinal edges. `gtao_view_normals.wgsl` is exactly the **world-normal prepass** PW03 asks for (world normal transformed into the GTAO view convention). `agx_tonemap.wgsl` is a 2.7K AgX curve. | `crates/render/src/gtao.rs` + `crates/render/src/shaders/gtao_*.wgsl` (new) + `crates/render/src/post/chain.rs`. Rows **LF3** (landing 2: AO on ambient only), **PW03** (normal target), LF2; gate G3 | L | The shaders are **WESL** (`#import renderide::post::gtao_math`) - naga does not read WESL, so every module is rewritten as a plain WGSL include we own. Our `gtao.rs` keeps a **f64 CPU oracle twin with a parity test**; a real GTAO doubles that proof work (the oracle must be re-derived too). Largest item on this list; do it after LF5/PW03, not before. |
| 10 | swedishembedded/brain | 44 | Apache-2.0 | `crates/kernels/wgsl/l1ssim_partials_h.wgsl` (1.6K), `l1ssim_map_v.wgsl` (3.5K), `l1ssim_moments_h.wgsl`, `l1ssim_moments_v.wgsl`, `l1ssim_grad_v.wgsl` | **L1-SSIM in WGSL**: the nine SSIM partial planes (mu_x, mu_y, sigma_x, sigma_y, sigma_xy), an 11-tap Gaussian window `exp(-i^2/4.5) * 0.266011725` (the same window FLIP uses), a separable map and a gradient pass. This is the machinery to compute a **perceptual image metric on the GPU**, so the browser page can score its own frame against the native frame instead of shipping PNGs to the host. | `crates/render/src/shaders/l1ssim.wgsl` (new) + `crates/metric/src/metric.rs` (the CPU twin) + `crates/web/src/arena_view.rs`. Row **LF15** (FLIP regression score), gap **B8**, row **WEB5** (browser frame beside the native frame); gate G4, G10 | M | This is a **new capability**, not a replacement: `metric::flip` (LF15, 061667e3) already scores native-to-native and stays the gate. The L1-SSIM kernels must agree with our CPU `metric::flip` within a tolerance or the two numbers disagree in the same report. Licence Apache-2.0 with a per-file SPDX header - keep the header if we lift a file. |
| 11 | electronicarts/pbmpm | 280 | BSD-3-Clause | `shaders/particleEmit.wgsl` (4.8K), `shaders/particleRender.wgsl` (2.9K), `shaders/setIndirectArgs.wgsl` (0.7K) | Two things for the live particle path: (a) a **GPU particle pool with an atomic free list** (`atomicSub` on a free-index stack, `atomicAdd` on the count when the free list is empty) so the pool is persistent and never grows; (b) an **instanced quad draw** where `instance_index` indexes the storage buffer of particles, the quad is scaled by the particle radius, and the fragment alpha is `smoothstep(0, 1, 1 - (dist - radius) / (onePixel * 500 * radius))` - a resolution-independent soft edge that never shows a square. `setIndirectArgs.wgsl` writes the draw args from the live particle count. | `crates/render/src/particles_gpu.rs` + `crates/render/src/particle_draw_gpu.rs` + `crates/render/src/shaders/particle.wgsl`. Rows **RL52** (particles drawn, soft, lit, flipbook-blended), **LF6** (dust and sparks); gate G3 (particles LIVE) | M | pbmpm is a 2D MPM fluid demo; take the pool and the draw, not the simulation. Our pool is **CPU-seeded and deterministic** (SplitMix64, LF6's Accept says "one tape twice gives equal particle counts per frame"); a GPU-owned pool breaks that unless the emitter schedule stays on the CPU and the GPU only integrates. The 500-pixel AA width is a good default for us but it makes big particles vanish at 720p - clamp it. |
| 12 | maxwag9/rusty_skylines | 4 | **none** | `data/Rusty Skylines Mod/shaders/compute/blue_noise_init.wgsl` (0.8K), `blue_noise_step.wgsl` (2.3K), `blue_noise_output.wgsl` | A **GPU void-and-cluster blue noise generator**: per-texel Gaussian energy over the binary pattern, an `atomicMax` on the bit pattern of the energy to find the extremum, an `atomicMin` on the index to break ties, then kill that point and write its rank. Three kernels total, so the whole generator is about 4K of WGSL. | `crates/render/src/shaders/blue_noise.wgsl` (new) + a cook-time bake into `content/`. Rows LF5b (jitter), LF3 (GTAO rotation), LF4; gate G4 (no banding) | S-M | **No licence file.** It is also an offline multi-iteration kernel, not a frame-time thing, so it belongs in the cook, not in the frame. The value is the *recipe*, and the bigger value is sharing one texture across six passes (see Wild ideas #1). |

### Also read, not ranked

- `zmactep/patinae` (216, BSD-3-Clause, Rust+wgpu): `crates/patinae-render/src/shaders/compute/ssao.wgsl`
  (6.7K) - 32-sample Crytek-style depth-only SSAO with the **hemisphere kernel baked into the
  uniform** and a temporal `frame_phase` jitter; `ssao_blur.wgsl`; `ssao_compose.wgsl`. Our GTAO
  is better, so this only wins as the **potato-tier AO** (G9) where the GTAO quality is not
  affordable. Size M.
- `kevinraymond/fosfora` (58, Apache-2.0): `assets/shaders/builtin/particle_prepare_indirect.wgsl`
  (0.8K) and `particle_prepare_indirect_world.wgsl` (1.1K) - a tiny prepare-indirect-args pair
  for particles, a good template for RL52's draw count.
- `B4rtekk1/Minerust` (6, MIT, Rust): `src/shaders/cull.wgsl` (5.7K) - GPU indirect rendering for
  a voxel world; a good second look when the arena gets real chunk streaming.
- `hlavacs/Vienna-WebGPU-Engine` (23, MIT): `resources/shaders/light_clustering.wgsl`
  (8.3K) - a C++ engine's clustered light build, richer than kiss3d's but in a different engine.
- `Lander-Rust/bevy-instancing` (18, Apache-2.0): `src/compute/shaders/indirect_offsets.wgsl`
  (1.1K) and `indirect_struct.wgsl` - the minimal indirect-args struct for Rust/wgpu.
- `panxinmiao/myth` (224, Apache-2.0): `crates/myth_render/.../blue_noise.wgsl` - unread before
  (LISTING only), but 84 KB per file, so it loses to #12.

---

## 2. Wild ideas

1. **One blue-noise texture, six passes, one hash.** Bake a 64x64 void-and-cluster blue noise
   once (cook time) and give the *same* texture to: the TAA jitter rotation, the GTAO
   per-pixel kernel rotation, the shadow PCF rotation, the decal atlas dither, the post-chain
   ordered dither, and the L1-SSIM window offsets. One asset, one hash, one golden. No GitHub
   engine ships a single shared blue-noise blueprint across TAA+AO+shadow+post - each ships
   its own. For us it is also the cheapest possible way to kill the banding our pass-truth
   report keeps seeing.
2. **Fighter-aware TAA.** Store the view depth in the TAA history's alpha (kansei) and, in the
   spare bits, the **object id** of the surface. Then the resolve can tell "the history was the
   other fighter" from "the history was this fighter one frame ago" and reset harder in the
   first case. In an 8-fighter brawl the classic ghost is exactly the other fighter's arm.
   Nothing in the wild does fighter-aware TAA because nobody has our cast.
3. **Camera-only motion as the potato path.** The velocity target can come entirely from the
   depth prepass (Lunar, 61 lines). The 8 fighters then add per-vertex velocity on top. So the
   static arena gets correct TAA at 0.6x internal resolution with TAAU for almost no money -
   which is the cheapest single win on the G9 potato gate.
4. **Skinning-aware decals for free.** Our decal box is world-space, so blood on a body slides
   off when the body moves. Give the decal record a **bone index** plus last frame's bone
   matrices and evaluate the box in bone space (the same trick the skinning vertex path
   already uses). Then blood rides the skin, and a corpse's blood stays where the corpse fell.
5. **Clustered culling that carries the shadow slot.** kiss3d's cluster cull writes (offset,
   count) per froxel. Add the shadow-map slot to the light record and the fragment loop then
   samples one shadow map per light instead of N. For an arena with torches this turns
   "every light, every pixel" into "2 lights, every pixel" and makes EG11's sun shadow plus
   torch shadows affordable at 8 fighters.
6. **A render-complexity number the engine prints.** Every pass declares how many WGSL lines
   and how many bytes of transient texture it costs; the product check prints "the lit frame is
   N lines of WGSL we own, M KB of transient VRAM". It makes the engine's complexity a
   machine-checkable claim, not a vibe, and it makes the render crate's own growth measurable.
   No GitHub engine can print that number.
7. **A live flicker number, not a per-run regression.** Run the L1-SSIM window in a compute
   shader every frame against the previous accepted frame and print the per-frame flicker of
   the sand edge. LF5c's Accept ("sand-edge luminance flicker drops at least 50% vs off")
   becomes a number the frame itself prints, so TAA tuning is a live dial, not a capture
   comparison run.
8. **Deterministic GPU particles with a CPU schedule.** Keep the emitter schedule, the seed
   and the per-tick spawn counts on the CPU (hash-seeded, so the tape decides), and let the
   GPU own only the **integration** and the **draw**. The particle pool becomes a deterministic
   function of the tape while the draw is GPU-side with a free list (pbmpm). LF6's Accept
   ("one tape twice gives equal particle counts per frame") survives.
9. **The browser as a first-class metric peer.** With the L1-SSIM kernels in WGSL, the web page
   computes the perceptual metric against the native frame itself and the door verb
   `view.compare` returns a number the browser produced. The browser stops being a picture we
   screenshot and becomes a machine that grades itself - and gap B8 gets a real gate.
10. **A shadow atlas that clusters itself.** Our `shadow_cluster.rs` already clusters torch
    shadows. Extend it to the **sun**: a 2048 atlas with one tile per cluster and a per-tile
    light list, so one sun shadow map serves an arena of many stands instead of a cascade
    that swims. Then the sun shadow (EG11) stops needing CSM and starts needing an atlas -
    a smaller change than it sounds and a much better look.

---

## 3. Rejected

1. **patriciogonzalezvivo/lygia** (3469 stars) - **Prosperity Public License 3.0**: free for
   noncommercial, commercial use is a 30-day trial only. A commercial engine cannot ship it.
   Also each file is a 0.5-2K snippet, so it is a lookup table, not a technique. Its
   `lighting/common/gtaoMultiBounce.wgsl` (0.6K) and `lighting/ssao.wgsl` (5K) are the only
   interesting parts and Renderide/patinae cover them with a permissive licence.
2. **greggman/webgpu-driving** - no LICENSE file, all rights reserved. Its TAA and cluster
   shaders are the best compact reference I found, so they stay on the list as *reading*, and
   the code that lands comes from kansei and kiss3d.
3. **maxwag9/rusty_skylines** - no LICENSE file. Kept on the list only as a recipe for the
   void-and-cluster blue noise (idea #1); do not lift the file.
4. **NVIDIAGameWorks/Falcor** and **GameTechDev/XeGTAO** - on the do-not-mine list, and
   independently the wrong shape: Falcor's GTAO is Slang with a 3000-line pass, while our
   `gtao.rs` carries a f64 CPU oracle twin with a parity test, so a Slang port would mean
   rewriting the shader *and* re-proving the oracle. Renderide (#9) gives the same algorithm
   in WGSL we can actually lift.
5. **Kunelinmc/IgnisRenderer** (2 stars, TypeScript) - `src/shaders/webgpu/scene/decal.wgsl` is
   **42K in one file** and `lighting/clusteredLightingCull.wgsl` is 22.6K. Porting either is
   3000 lines we cannot test. mewhhaha/pen (#4) gives the same decal in 71 lines.
6. **electronicarts/pbmpm's MPM solver** - the repo is a position-based MPM sand and snow
   demo with a 2D fluid, not a game renderer. We take the free list and the instanced quad
   and nothing else.
7. **kishimisu/WebGPU-Radix-Sort** (101) - no LICENSE, and a 4-way radix sort over 256-bit
   blocks. We have 4096 particles: a CPU sort of 4096 keys is under 0.1 ms and keeps the
   draw deterministic. Overkill.
8. **leighteg/helmer's SSGI** - the raymarch is 6.7K of shader plus a 27K Rust pass and a 19K
   denoiser, which does not fit G2's 8.3 ms on a 3050 with 8 fighters. Take only the
   joint-bilateral upsample idea (#6) and only if SSGI is ever attempted.
9. **DoubleStyx/Renderide as a whole** - a full renderer built on WESL `#import` modules with
   multiview, its own render graph and its own bind-group conventions. Only the GTAO math, the
   AgX curve and the auto-exposure histogram are worth lifting; the rest is a rewrite.
10. **moonbit-community/mgstudio, Sunet-Studio/Sundown, takahirox/three-rs-wasm** - all have
    the right file names (`gtao_main.wgsl`, `decal.wgsl`, `taa.wgsl`), but they are ports of
    Bevy/Three with the Bevy/Three conventions baked in (material ids, `#include` glue, a
    JavaScript host). Reading them teaches the same thing kansei and voidin teach better.

---

## 4. Verdict

**Siroko/kansei's `rust/kansei-core/src/shaders/taa_resolve.wgsl` is the single best steal** -
it is the only donor in the whole hunt that is a complete, real TAA/TAAU resolve in plain
WGSL inside a Rust engine, and it is the only one that carries the three things our
`taa_resolve.wgsl` lacks and our row LF5c demands: disocclusion handling (the history's alpha
holds last frame's view depth), jitter-aware neighbourhood reconstruction (the Blackman-Harris
fit `exp(-2.29 d^2)`), and temporal upscaling - 231 lines we can port nearly verbatim, which
turns LF5 from "add a box clamp" into "add the real thing".
