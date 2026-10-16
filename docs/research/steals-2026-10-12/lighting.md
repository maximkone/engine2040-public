STEAL SCOUT: LIGHTING AND GLOBAL ILLUMINATION (2026-10-12)
Area: sunlit stone Colosseum at several times of day, torch/fire light at night, potato fallback.
All reads dated 2026-10-12. Every steal below is PRESENTATION (floats and GPU are fine; nothing feeds the tick;
time of day is one integer minute-of-day in the match setup, the renderer turns it into a sun vector).
Bake tools are cook-time presentation: bytes x2 on the same machine, never in the tick.

1. WE HAVE (code first, then what the plan says next)
- Live shading is one sun plus fixed-HDRI ambient. forward.wgsl:397-420 `fs`: one directional light (direct_light 333-353),
  split-sum IBL (147-176), height fog per fragment (364-387). No point lights. The "cascade" only dims the sun 15 percent
  by view distance (402-409); it is not a shadow. `occlusion` is the constant 1.0 (415), so all ambient is full sky.
- The ambient is NOT the arena sky: presenter.rs:384-389 bakes ONE studio HDRI (studio_small_09) once; the sky behind it
  comes from arena_day.rs:22 / presenter.rs:370. Sun height cannot change the light on the sand. sky.rs:138-248 already
  takes a time-of-day SkyParams (sun_dir, turbidity); sky.rs:226-234 computes sun transmittance but never exports a sun colour.
- Sun depth map renders but nothing samples it: submit.rs:461-478 (comment "Nothing samples it yet"). Rows EG11/EG11b/EG11c
  (SPRINT-18:199-233) own that.
- GTAO exists (gtao.rs:30-40: half res, 4 slices x 3 taps, 0.8 ms bar) but the live AO texture stays 1.0 (submit.rs:236-263)
  and blit.wgsl:49-50 multiplies the final colour, so AO would also darken the sun. Row LF3 (SPRINT-18:1412).
- Baked GI is a unit-tested toy: baked_bake.rs:491-507 bakes a flat 20 x 20 m floor fixture; lightmap 64x64, cap 256 (21-29);
  texels are Reinhard tone-mapped to a 16-entry warm-gray palette WITH albedo baked in (gi.rs:10-29, 176-197, 240-244;
  upload Rgba8Unorm baked_gi.rs:311, 627). gi.rs:145-173 `direct()` adds sky + sun + torches with NO visibility ray, so a
  probe under the stands sees the whole sky and the sun is counted twice once the live sun is on. Probes: 6x3x6 at 4 m
  (baked_bake.rs:123-131), 9 vec4 = 144 B each, CPU nearest-probe pick (baked_gi.rs:79-130). The tunnel leak fix is one
  hand-placed SealedBox (baked_bake.rs:67-100, 511). FORWARD_GI_WGSL (baked_gi.rs:435-467) is never included by forward.wgsl.
- Torches: torch_cluster.rs (~870 lines: CPU clusters plus a temporal reservoir) feeds only shadow_cluster/twin/post, nothing
  in present/ or forward.wgsl. The arena kit has 2 torch actors (content/arena/placement.json). Plan rows: VS7 (SPRINT-01:313)
  and EG12(b) "up to 8 point lights in a uniform array" (SPRINT-18:234-243).
- gi_dyn.rs is an analytic-SDF cascade spike, off by default (gi_dyn.rs:29, 156). tier.rs:57-62 gates Rt/Ultra on ray_query +
  accel; accel.rs:1-14 is a CPU BLAS/TLAS refit (1.5 ms bar). There is no low tier yet (potato.md:13); GS10 quality table
  Potato/Comfort/High is planned (potato.md:85).
- Plan: render-look.md:22-39 (status table), 260-265 (hard problem 4, baked light without leaks), RL5 frozen at 386-389,
  "will not build" 303-309 (ray tracing is RT rows). potato.md:68 forbids SDFGI and Solari-class RT. VISUAL-TARGET.md:64-68
  tier table (Base = baked lightmaps or probe GI + GTAO). The 2026-10-11 scouts (render-steals.md, engine-landscape.md:32,86-90)
  named scathanna2, SDFGI, Solari cache and radiance cascades without numbers; this file adds numbers and newer donors.
- Finding to keep: scathanna2 has no license file (GitHub license API 404) and its last commit is 2023-01-31, so it is
  IDEA-ONLY and not a 2024+ donor. Godot's lightmapper below replaces it.

2. STEALS (ranked by value for the Rome duel per hour of work)
1. Sky-driven IBL and sun colour for time of day | bevyengine/bevy@c9c21c2b7af92a396b5f93ecfa1536a8a2afcc68:crates/bevy_pbr/src/atmosphere/environment.wesl (+ light_probe/generate.rs) (read 2026-10-12) | MIT OR Apache-2.0 | PRESENTATION | lands in crates/render/src/ibl.rs, arena_day.rs, present/gpu_present/presenter.rs | M
   Pattern: Bevy turns the sky LUT into a cube in a 38-line compute pass, then filters it. We do the CPU twin: split ibl::bake (ibl.rs:711) so
   it takes an EnvMap built from SkyCapture, bake 5 sun keyframes at load on the worker, lerp the 9 SH terms per frame and swap/crossfade the cube.
   Also export sun_color = SUN_INTENSITY * exp(-(beta_R+beta_M)*airmass) from sky.rs:226-234 (own formula, no donor bytes) so dawn goes orange.
   Number: ambient source today 1 fixed studio HDRI, sky/ambient mismatch unmeasured -> SH L0 luminance within FOG_SKY_RAMP_TOL (0.02) of the sky capture
   mean at 5 sun elevations; load +5 bakes on the worker (measure, kill at 3 s); per-frame CPU under 20 us. Proof: new `cargo test -p engine-render --lib sky_ibl`
   + EG8 capture pair dawn vs noon differs on shadow-side sand. Risk: 5 bakes slow on weak CPUs. Kill: drop to 3 keyframes or 32px cube; kill if pair is identical.
2. Occlusion-aware ambient: multi-bounce AO plus specular occlusion | google/filament@87bdf96449df07e7b459b60d2c9f703ca03b836d:shaders/src/surface_ambient_occlusion.fs and @3cbd4e1c2426055662be040a4a7a936c597eb658:shaders/src/surface_light_indirect.fs (read 2026-10-12) | Apache-2.0 | PRESENTATION | lands in shaders/forward.wgsl:410-416, blit.wgsl:49-50 | S
   Pattern: AO only scales diffuse and specular IBL (never the sun); diffuse gets an albedo-dependent multi-bounce lift (gtaoMultiBounce), specular gets
   specularAO (Lagarde fit, or cone version with a bent normal). This fixes the "all ambient is full sky" flat look under stands and in the tunnel.
   Number: occlusion const 1.0 -> AO source (baked sky visibility from steal 5, or GTAO after LF3) applied to ambient only; wall-base / stand-underside pixels
   at least 25 percent darker, open sand within 2 percent; cost ALU only, under 0.05 ms. Proof: EG8 on/off masks + `arena per-pass`.
   Risk: AO also needs a depth prepass to apply to ambient only (LF3 second landing). Kill: if open sand moves over 2 percent, keep blit AO off.
3. Torch point lights: sphere light, range window, seeded flicker | bevyengine/bevy@c9c21c2b…:crates/bevy_pbr/src/render/pbr_lighting.wesl (point_light, getDistanceAttenuation, range falloff; ~L127-132, 348, 597-650) and render/clustered_forward.wesl (read 2026-10-12) | MIT OR Apache-2.0 | PRESENTATION | lands in shaders/forward.wgsl (new `point_light` beside direct_light, same file we already ported v0.15.0 from), present/gpu_present/submit.rs light feed | M
   Pattern: inverse square times (saturate(1 - (d^2/range^2)^2))^2 so every torch has a hard finite range (cull-friendly); the Karis representative point widens the
   specular lobe by radius/(2 d) so a flame gives a real highlight on armor and blades. Flicker is own design (IDEA-ONLY): 3-octave value noise at 6-12 Hz keyed by
   (torch id, frame index) so captures repeat; +/-15 percent intensity, 1900-2200 K colour. Burning props from the sim fire state become lights through the snapshot (read-only).
   Number: 0 point lights -> 8 in a uniform array now, 32 via a storage buffer with CPU nearest-N cull; VS7 accept (mean luminance within 2 m of a torch at least 2x far sand);
   cost estimate 0.6 ms for 8 lights, 1.5 ms for 32 at 1080p on the 3050 (ALU estimate, not measured). Proof: `arena per-pass` lights=, EG8 on/off.
   Risk: shadowless torches glow through walls. Kill: over 1.2 ms for 8 lights; drop to nearest 4 and bake the rest (steal 12).
4. Lightmap storage fix: linear HDR irradiance, no albedo, no sun, BC6H | bevyengine/bevy@c9c21c2b…:crates/bevy_pbr/src/lightmap/mod.rs (format and UV rules) + hasenbanck/block_compression@f81ea7551b8fff47fb4e16d02a3077c0aba20a90 (BC1-BC7 incl. BC6H on DX12/Metal/Vulkan; README read) | MIT OR Apache-2.0 / MIT | PRESENTATION | lands in crates/render/src/gi.rs:10-29,176-197,240-244, baked_gi.rs:311,627, shaders/forward.wgsl (VIn gets uv1, forward.wgsl:178-182) | S
   Pattern: store sky + bounce irradiance only (the live sun and the live torches stay real-time, otherwise they count twice); the shader multiplies by the textured
   albedo. Bevy needs a second UV set, atlas, linear sampler, and warns that bicubic lightmap sampling can leak. xatlas padding is already 4 (geo/src/uv.rs:35-36).
   Number: 16 palette grays + LDR + albedo baked -> RGB9E5 or BC6H; at 1024x1024 BC6H is 1 MB per keyframe (RGBA8 would be 4 MB); distinct output levels in a gradient fixture 16 -> over 1024.
   Proof: `cargo test -p engine-render --lib baked_gi::tests::bake_identical_bytes_twice` stays green + new gradient-levels test. Risk: RL5 is frozen (render-look.md:386), lead must lift it for this row.
   Kill: if BC6H shows blocking on smooth gradients, ship RGB9E5 (4 B/texel).
5. Sky visibility + bent normal bake (time-of-day-ready ambient) | google/filament@3cbd4e1c…:shaders/src/surface_light_indirect.fs (diffuse uses the bent normal when present) + GameTechDev/XeGTAO@a5b1686c7ea37788eeb3576b5be47f7c03db532c:Source/Rendering/Shaders/XeGTAO.hlsli (bent normals behind XE_GTAO_COMPUTE_BENT_NORMALS; README: about +25 percent cost); idea from Unity APV sky occlusion docs (per-probe static sky occlusion, live sky colour) (read 2026-10-12) | Apache-2.0 / MIT | PRESENTATION | lands in crates/render/src/gi.rs `direct()`/`Lightmap::bake`/`ProbeGrid::bake`, baked_bake.rs | M
   Pattern: bake once, per texel and per probe, a visibility fraction and a bent normal against the kit; at runtime ambient = sky_SH(bent normal) * visibility. The sky SH (incl. warm ground bounce
   from the sky capture) changes with time of day for free, so ONE bake serves all hours for the sky part. First slice traces the 8 placement.json proxy boxes with a brute-force slab test: no BVH, no new dependency.
   Number: gi.rs:145-173 has no visibility -> tunnel-fixture sky visibility inside <= 0.35 vs open sand >= 0.95 with NO SealedBox; bake <= 3 s at 1024^2 x 64 rays x 8 boxes (estimate ~1 s); bytes x2.
   Proof: new `cargo test -p engine-render --lib baked_gi::tests::sky_visibility` + x2. Risk: proxy boxes are coarser than the arches. Kill: if arches look flat after slice 1, move to steal 9.
6. Cycles / Eevee reference and first-cut cook | Blender manual, Eevee "Light Probe Volume" page (Validity Threshold, Dilation Threshold and Radius, Normal Bias, View Bias, surfel bake; read 2026-10-12); bake operator signatures NOT read | Blender is GPL: separate cook tool, outputs only (decision D4/D5) | PRESENTATION (cook) | lands in tools/dcc/blender (GEO1 plan; the dir is not on disk today, only content/fighters/tools/*.py use bpy) | S
   Pattern: one headless Blender script bakes the kit lightmap (Cycles) and a probe volume (Eevee) for 3 sun angles + night, and renders path-traced 1080p stills of the same cameras. Those stills are the machine-judge references the Look gate lacks
   (render-look.md:31: zero reference PNGs) and the A/B truth for steals 4-9. Hash the cooked EXR/KTX2, not Blender.
   Number: lighting reference images 0 -> 4 per cook (10:00, 14:00, 18:00, night); our frame vs reference mean dE and SSIM posted per pass. Proof: cook x2 same SHA on the same Blender version.
   Risk: API names unverified (IDEA-ONLY for the script); Cycles seed/threads may break x2. Kill: if x2 differs, pin 1 CPU thread and fixed seed; if still differs, use it only as a reference, never as shipped data.
7. Probe validity, virtual offset, dilation (replace the SealedBox) | turanszkij/WickedEngine@d7e162ded686f71717f6efbb625a1994149946e6:WickedEngine/shaders/ddgi_updateCS.hlsl (probe relocation: keep distance 0.1, offset smoothed 0.01, clamped to half a cell, 18-neighbour voxel nudge) + Unity URP docs probevolumes-light-probe-validity (backface hits mark a probe invalid; Virtual Offset moves it out; Dilation fills from valid neighbours) + Blender doc above (read 2026-10-12) | MIT / docs (idea) | PRESENTATION | lands in crates/render/src/baked_bake.rs:67-100,511-516, baked_gi.rs:79-130 | M
   Pattern: during the bake, rays that hit back faces mark a probe invalid; try to push it out, else fill from valid neighbours; runtime also offsets the lookup point along the normal. Wicked's update shader has no dead-probe test of its own, so the validity rule comes from Unity/Eevee.
   Number: 1 hand-placed box, 108 probes at 4 m -> automatic validity on a 1 m grid (~20x6x20 = 2400 probes; raise PROBE_MAX_COUNT 1024 at baked_bake.rs:25); tunnel bleed fixture still 0.
   Proof: existing `sealed_tunnel_takes_nearest_without_bleed` passes with the box removed. Risk: dilation across a thin gate pillar leaks. Kill: keep the box as an override list.
8. Probe volume as a 3D ambient-cube texture, sampled per fragment (fighters AND the 20-50k crowd) | bevyengine/bevy@c9c21c2b…:crates/bevy_pbr/src/light_probe/irradiance_volume.wesl (~85 lines, Valve ambient cube, 3 hardware-trilinear fetches weighted by N*N, half-texel clamp) and irradiance_volume.rs docs (24 B per voxel, RGB9E5) (read 2026-10-12) | MIT OR Apache-2.0 | PRESENTATION | lands in baked_gi.rs:435-467 (replace FORWARD_GI_WGSL probe part), forward.wgsl | M
   Pattern: pack 6 colours per voxel into one 3D texture (x, 2y, 3z); hardware trilinear does the blend; no CPU pick. Bevy docs weigh ambient cube against SH L1/L2: L1 can go negative (no RGB9E5), packed SH blocks hardware filtering.
   Number: 144 B/probe SSBO + CPU pick (0.5 ms budget for 8 fighters, baked_gi.rs:39) -> 24 B/voxel; 40x12x40 m at 1 m = 19,200 voxels = 460 KB; crowd ambient CPU cost 0 (a CPU pick cannot serve 20k instances).
   Proof: probe GPU-vs-CPU parity test (GI_BIND_PARITY_TOL style). Risk: ambient cube is lower quality than SH9. Kill: keep SH9 for the 2 hero fighters, cube for crowd only.
9. Traced baker over the real kit meshes (bounce, probes, emission) | godotengine/godot@232e6b14abf6e590b41d60f11704e42de22bff72:modules/lightmapper_rd/lm_compute.glsl (modes DIRECT, BOUNCE with cosine rays and Russian roulette, LIGHT_PROBES with 9 SH terms and 4/ray_count scale, emission only via bounce; ray walk is a uniform-grid DDA with cluster boxes, not a BVH) (read 2026-10-12) + dimforge/parry@b3c3584ce9be9bf6edb821fd7548b817087131ff (Apache-2.0; README only, API unread; geo already has parry3d 0.30.2 behind `collision`, geo/Cargo.toml:35, and vendored xatlas) | MIT / Apache-2.0 | PRESENTATION (cook) | lands in new crates/geo/src/bake.rs (D4 "bake" verb) feeding baked_bake.rs ArenaBake (needs a from_bytes; only to_bytes exists, baked_bake.rs:266) | L
   Pattern: same integrator shape as Godot, CPU, per-texel RNG seeds so the thread count does not change the bytes. Sun is NOT baked (live); bounce keyframes per sun angle and a night layer with torches as emitters.
   Number: flat-floor fixture -> 8 kit meshes at 1024^2; budget 1M texels x 256 rays x 2 bounces on 8 threads, target under 60 s (estimate, measure); bytes x2.
   Proof: `cargo test -p engine-geo --features collision bake_identical_bytes_twice`. Risk: scope. Kill: if steal 5 + 6 already read right in the Q1 judge, stop at sky visibility and skip bounce.
10. Bake hygiene: unocclude, dilate, seam blend, JNLM denoise | godotengine/godot@232e6b14…:modules/lightmapper_rd/lm_compute.glsl (MODE_UNOCCLUDE pushes texels off geometry, MODE_DILATE, MODE_DENOISE = JNLM, MODE_PACK_L1_COEFFS) and lm_blendseams.glsl (copies lit colour across UV seams and into chart padding) (read 2026-10-12) | MIT | PRESENTATION (cook) | lands in geo/src/bake.rs | M
   Pattern: this is the light-leak kit for thick stone: texels straddling a wall edge are pushed out and dilated, seams are blended, noise is removed by a built-in JNLM pass (Godot dropped OIDN for it), no runtime or binary dependency.
   Number: leak texels (bright texel next to a dark wall base across a seam) unmeasured -> 0 on the fixture; denoise ~6 dB noise cut (target, measure). Proof: seam fixture + x2. Risk: JNLM blurs stone detail; lightmaps are low-frequency (10 cm texels) so low risk.
11. AO quality: XeGTAO denoise, thin-occluder compensation, bent normals; Blade a-trous as the temporal option | GameTechDev/XeGTAO@a5b1686c…:Source/Rendering/Shaders/XeGTAO.hlsli (denoise = two 3x3 passes, centre + 4 edge + 4 diagonal, edge weights, diagonal weight 0.425; thinOccluderCompensation; archived 2024-04-22) + kvark/blade@56f05651cca73659130e924e497dda633f839d40:blade-render/code/a-trous.wgsl (170 lines WGSL, temporal accumulate + a-trous 3x3 with luminance-variance gate) (read 2026-10-12) | MIT / MIT | PRESENTATION | lands in crates/render/src/gtao.rs:763-896 (WGSL), post.rs | M
   Pattern: our half-res GTAO has bilateral upsample but no denoise; XeGTAO ships a depth-aware denoise and the thin-occluder trick (fighters, pillars do not over-darken). XeGTAO README timings: High 1080p 0.56 ms on RTX 2060, 2.39 ms on an Iris Xe laptop, bent normals about +25 percent.
   Number: gtao.rs bar 0.8 ms half-res (render-look.md:173) -> keep 0.8 ms including denoise; shimmer variance under SMAA_SHIMMER_MAX_VAR. Proof: GX14 boosted run + EG8. Risk: overlaps render-steals.md #9 (same family); this adds the denoise and numbers only. Kill: over 1.0 ms.
12. Layered time-of-day: sun-bounce keyframes plus per-group torch layers | IDEA-ONLY (Quake light styles are GPL; Unity APV Lighting Scenarios docs read: blending covers probe data only, probe positions must stay fixed between bakes) | n/a | PRESENTATION | lands in baked_gi.rs, forward.wgsl | M
   Pattern: light is linear, so bake K layers (sun bounce at 4 angles, night torches by group) and mix with weights; flicker is just the torch-layer weight; time of day lerps two sun layers. Memory: 5 layers x 1 MB BC6H at 1024^2.
   Number: 1 baked state -> 4 day + 1 night layers = 5 MB; blend cost 2-3 fetches per fragment. Proof: layer-sum equals the single combined bake within 1/64 on the fixture. Risk: only useful after steals 4 and 9. Kill: if steal 5's live sky_SH x visibility already looks right across hours, skip layers.
13. Sun shafts through the arches | bevyengine/bevy@c9c21c2b…:crates/bevy_pbr/src/volumetric_fog/volumetric_fog.wesl (~400 lines: per-pixel march from fog AABB front to scene depth, step_count uniform, interleaved-gradient jitter, shadow cascade per step, Henyey-Greenstein) and volumetric_fog/mod.rs (cost scales with directional lights, shadow maps required) (read 2026-10-12) | MIT OR Apache-2.0 | PRESENTATION | lands in crates/render/src/fog.rs + forward.wgsl fog | M
   Pattern: needs EG11b first. Run at quarter res with 24 steps and bilateral upsample, 1 directional light. Number: no shafts -> shafts; cost estimate 0.5-1.0 ms at quarter res on the 3050 (estimate; Bevy gives none). Kill: over 1.0 ms or shimmer over 0.06.
14. RT-tier spike: Solari world cache plus Blade ReSTIR DI/denoise | bevyengine/bevy@c9c21c2b…:crates/bevy_solari/src/realtime/world_cache_query.wesl (spatial hash of quantised position + normal, LOD cell size, checksum, max 3 probe steps, lifetime 10 frames) + kvark/blade@56f05651…:blade-render/code/ray-trace.wgsl (ReSTIR DI, 24-byte reservoir, up to 4 temporal taps, WGSL `rayQuery`) (read 2026-10-12) | MIT OR Apache-2.0 / MIT | PRESENTATION | lands in render/accel.rs consumer, tier.rs Rt path (Tier C, frozen by D7 until the first real frame) | L
   Numbers (jms55 blog, RTX 3080, 1600x900 internal): RT lighting passes 1.2 ms (Cornell) to 5.8 ms (Bistro) before the 6.1 ms DLSS-RR denoiser; world cache update 1.42 -> 0.09 ms. A 3050 laptop is roughly 5x slower (my estimate, not measured), so expect 6-30 ms at 900p: not 60 fps.
   wgpu 30 changelog lists ray tracing for Vulkan and Metal and no DX12 item; Blade lists Windows = Vulkan only. So an Rt tier on this PC means the Vulkan backend. Keep as a one-day measure-only spike; kill if lighting passes exceed 8 ms at 540p on the 3050.

3. BOARD ROWS (top 6; prefix XX, lead numbers; plan owner is render-look.md unless noted)
- [ ] XX1 render: sky-driven IBL and sun colour for time of day — the live ambient is one fixed studio HDRI (presenter.rs:384-389) while the sky comes
  from the arena-day rig (presenter.rs:370), so sun height never changes the sand; split ibl::bake (ibl.rs:711) to take an EnvMap from SkyCapture, bake 5
  keyframes on the worker, lerp SH per frame, export sun colour from sky.rs:226-234. Diff: good. Tier A. Size M. Owns `crates/render/src/ibl.rs`,
  `crates/render/src/arena_day.rs`, the ibl hunk of `crates/render/src/present/gpu_present/presenter.rs`. Accept: `cargo test -p engine-render --lib sky_ibl`
  shows SH L0 luminance within 0.02 of the sky capture mean at 5 elevations, keyframe bytes x2 equal, and the dawn vs noon capture pair differs on shadow-side
  sand. F2P: ambient identical at every sun height today. P2P: golden unchanged, `cargo test -p engine-render --lib ibl` green. needs: LF2. (plan: render-look.md)
- [ ] XX2 render: ambient-only multi-bounce AO and specular occlusion — forward.wgsl:415 `occlusion = 1.0` and the blit AO multiplies the sun too (blit.wgsl:49-50);
  port the Filament multi-bounce and specular-AO functions and apply AO to IBL only. Diff: good. Tier A. Size S. Owns `crates/render/src/shaders/forward.wgsl` AO hunk,
  `crates/render/src/shaders/blit.wgsl`. Accept: EG8 on vs off: wall-base and stand-underside pixels at least 25 percent darker, open sand within 2 percent, `arena per-pass`
  AO cost under 0.05 ms, validation errors 0. F2P: on/off frames identical today. P2P: golden unchanged. needs: LF3. (plan: render-look.md)
- [ ] XX3 render: torch point lights with sphere radius, range window and seeded flicker — forward.wgsl has only the sun (`light` binding 1); EG12(b) plans 8 plain lights;
  port Bevy point_light + range falloff, array of 32 with CPU nearest-N cull, flicker keyed by (torch id, frame index). Diff: good. Tier B. Size M. Owns
  `crates/render/src/shaders/forward.wgsl` light hunk, light feed in `crates/render/src/present/gpu_present/submit.rs`. Accept: at night the mean luminance within 2 m of a torch is at
  least 2x far sand; 8 lights cost at most 0.8 ms and 32 lights at most 1.5 ms in `arena per-pass`; capture x2 equal. F2P: 0 point lights in the live frame today.
  P2P: golden unchanged. needs: EG12, VS7. (plan: render-look.md)
- [ ] XX4 render: lightmap texel is linear HDR irradiance — gi.rs:10-29 quantises to 16 grays after Reinhard and bakes albedo in (gi.rs:176-197, 240-244); switch to
  RGB9E5 or BC6H irradiance without albedo or sun, add the second UV set (forward.wgsl:178-182). Diff: good. Tier B. Size S. Owns `crates/render/src/gi.rs`,
  `crates/render/src/baked_gi.rs`, `crates/render/src/baked_bake.rs`. Accept: `cargo test -p engine-render --lib baked_gi` green with a gradient fixture showing over 1024 distinct
  levels (16 today), bake bytes x2 equal, 1024^2 BC6H keyframe at most 1 MB. F2P: gradient fixture posts 16 levels. P2P: golden unchanged, `bake_identical_bytes_twice` green.
  needs: lead unfreezes RL5 (render-look.md:386). (plan: render-look.md)
- [ ] XX5 render: sky visibility and bent normal in the bake — gi.rs:145-173 `direct()` has no visibility ray, so probes under the stands see the full sky; trace the 8
  placement.json proxy boxes (slab test) per texel and probe, store visibility + bent normal, ambient = sky SH at the bent normal times visibility. Diff: good. Tier B. Size M.
  Owns `crates/render/src/gi.rs`, `crates/render/src/baked_bake.rs`. Accept: `cargo test -p engine-render --lib baked_gi::tests::sky_visibility` shows tunnel-centre visibility at
  most 0.35 and open sand at least 0.95 with no SealedBox, release bake at most 3 s, bytes x2. F2P: tunnel visibility 1.0 today. P2P: golden unchanged. needs: XX4. (plan: render-look.md)
- [ ] XX6 tools: path-traced lighting references from headless Blender — Q1 has zero reference PNGs (render-look.md:31); one cook script bakes the kit lightmap and a probe volume
  and renders 4 stills (10:00, 14:00, 18:00, night) with the same cameras as the arena capture. Diff: regular. Tier B. Size S. Owns `tools/dcc/blender/gi_reference.py`
  (new, GEO1 area). Accept: the cook run x2 gives the same SHA-256 for the 4 stills and the lightmap, and `node tools/finish.mjs` posts our-frame vs reference dE and SSIM per hour.
  F2P: 0 lighting references today. P2P: golden unchanged, no Blender in the shipped build. needs: GEO1. (plan: content.md / render-look.md Q1)

4. PLAN TEXT (render-look.md section 3 "Target design" and section 4 "Hard problems")
Section 3, add "Lighting layers" under the frame data flow:
  ambient = sky_SH(bent_normal) * sky_visibility * multi-bounce AO   (live in time of day: the SH comes from the sky capture of the current sun, not from a fixed HDRI)
          + bounce_layer(sun angle)  (lightmap + 3D ambient-cube volume, baked indirect only, lerped between 2 keyframes)
          + night torch layers (baked, weight = flicker)
  direct  = live sun (cascades) + up to 32 torch point lights (sphere radius, finite range); the live sun and live torches are NEVER baked into the lightmap or probes.
  Rules: the lightmap holds linear HDR irradiance, not albedo, not tone-mapped; time of day is one integer minute in the match setup, the renderer derives floats; torch and fire lights read the
  snapshot (sim fire cells), flicker seeds from (light id, frame index) so captures repeat; none of it feeds the tick.
Section 4, add hard problem 7 "One ambient truth at every hour": sky, ambient and fog read the same sky capture (extends hard problem 1); keyframes of 5 sun angles, 9 SH terms lerped per frame.
Section 4, add hard problem 8 "Light leaks through thick stone", three layers: bake time (unocclude, dilate, back-face validity, seam blend: Godot lightmapper + Unity/Eevee probe rules), storage time (xatlas
  padding 4, linear sampler, no bicubic, validity in alpha), run time (normal offset, bent normal, specular occlusion). Test: seam fixture and tunnel fixture, 0 bleed, bytes x2.
Budgets to add beside section 3 numbers (proposals until measured): torches 0.8 ms for 8 and 1.5 ms for 32, AO application 0.05, sun shafts 0.5 at quarter res, lightmap residency 8 MB for 5 BC6H keyframes at 1024^2.
Potato ladder (GS10 knobs, same baked bytes on every tier so potato is fewer passes, not different data): Potato = lightmap + ambient volume + 1 sun cascade 1024 + 4 torches, no GTAO, no shafts; Comfort (3050) = + GTAO 0.8 ms,
  3 cascades, 32 torches; High = + shafts, XeGTAO bent normals. Precedent: Godot's Compatibility renderer for old hardware renders pre-baked lightmaps but cannot bake them (Godot docs, read). XeGTAO is 2.39 ms on an Iris Xe laptop, so it is off on potato.
Slim: torch_cluster.rs (~870 lines, no forward consumer) is a cut candidate for SL3 once the flat torch array lands; scathanna2 stays out of the donor list (unlicensed, 2023).
Tier C: the RT lighting path (steal 14) stays measure-only until the first real frame lands (D7).

5. NOT WORTH IT (checked, rejected for this game)
- Radiance cascades (2D/3D): the 2025 Holographic RC paper reports 1.85 ms at 512x512 and 7.67 ms at 1024x1024 on an RTX 3080 Laptop, and that is 2D; the 3D repos found are unlicensed (three-rc) or WIP without numbers (unity-urp-radiance-cascades, MIT).
  The arena is static with 2 to about 40 torches, so a bake beats a per-frame solve on a 3050.
- Bevy Solari as the baseline GI: see steal 14 numbers; Bevy 0.20 itself made ReSTIR optional and off by default because it costs a lot for little gain, and its denoiser is NVIDIA-only DLSS-RR. Not a 60 fps path on the 3050.
- DDGI (Wicked ddgi_*.hlsl) as the baseline: 512 rays per probe per frame needs a ray-traceable scene every frame; the arena does not change. Keep only the probe relocation idea (steal 7).
- GPU light clustering (Bevy PR #23036 reports 0.099 ms GPU vs 2.12 ms CPU on its many_lights stress test; search summary only, PR not read): the arena has 2 torch actors today and about 40 at most; a flat array needs no clusters.
- LTC rect/area lights (Bevy rect_light needs LTC LUTs): no rectangular emitters in a torch-lit stone arena; sphere lights cover flames.
- Screen-space GI via visibility bitmask (cdrinmatane/SSRT3, MIT, last commit 2024-10-28, README: about 2 to 3 ms on an RTX 3060 for the whole effect, light passes behind thin surfaces, off-screen light falls back to probes): torches that leave the screen would pop, and the baked bounce already gives the fill light.

Read count: 23 source files + 7 READMEs/repo pages = 30 donor files; also read as articles (not counted): jms55 Solari 0.18 post, Bevy 0.20 news, wgpu CHANGELOG (first 100k chars), Unity URP probe docs, Blender 5.2 Eevee probe page, Godot renderer docs.
IDEA-ONLY steals: 12 (and the Blender script API in 6, the flicker curve in 3). Search-summary-only claims: HRC timings, Bevy PR #23036.
DONE 14 steals, 30 donor files read.
