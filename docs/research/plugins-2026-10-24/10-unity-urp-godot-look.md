# Unity URP and Godot 4 look on a potato iGPU: picks for engine2040 (2026-10-24)

Scope: how Unity URP and Godot 4 get their look on weak GPUs, the asset-store packs and Godot addons that sell that look cheaply, a parity check against engine2040's render crate, and a ranked top 15. Skips what `03-market-sweep.md`, `06-unity-godot-itch.md`, `04-github-awesome.md` and `donors-seen.txt` already cover (see Overlap check).

## How to read this

- [V] = fetched in this session. Facts come from the fetched page.
- [S] = found in a search result. Facts come from the search summary, a store capture with empty feature text, or a third-party mirror. Re-check before relying on it.
- [U] = not verified this pass.
- Costs are relative labels from Unity and Godot docs, not milliseconds. No source publishes an iGPU ms figure for any pick. Where engine2040 already has a budget for the same kind of pass, its number is used (`arena_day.rs`, `aa_scale.rs`, `accel.rs`). Anything else must be measured on the potato reference before it is trusted.
- The three Kronnect store captures had empty feature sections. Their price and version are [V]. Their features are [S].

## Overlap check

- `donors-seen.txt` (2026-10-21) already lists these, so they are not picked again: XeGTAO, SMAA (`iryoku/smaa`), FidelityFX FSR, Khronos PBR Neutral (`KhronosGroup/ToneMapping`), AgX, Tony McMapface, Hillaire and Bruneton atmospheric scattering, SSRT3, TheRealMJP shadows, separable SSS, the bevy outline crate (`komadori/bevy_mod_outline`), the octahedral impostor component, Unity Animation-Instancing, Unity CsReference, PhotoMode (Unity and DarknessFX), godot-camera-shake, the Godot trail system, the Godot sword-slash shader, the Godot VAT plugin, and the godotshaders.com site.
- `06` covers Toony Colors Pro 2, Crest Water 5, Weather FX Free, Bakery, GPU Instancer Pro, Mesh Baker, Automatic LOD, and Rawteous. `03` covers Amplify Impostors and Dynamic Fog & Mist 2. None is repeated here.
- Today's look audit (`docs/research/look-audit-2026-10-24.md`) lists the visible defects. Most are content or art (flat boxes, untextured dunes, HUD text, black slabs), not missing render features. Two are render-side and are used below: hard-edged blood and dust squares (defect 8) and the flat hit flash (defect 11).

## engine2040 today (what the parity table measures against)

From the module docs and the shader list under `crates/render/src`:

- Tiers (`quality.rs`): Potato (smallest scale, one cascade, tightest caps), Mobile (one cascade, no AO), Web (two cascades, half-res AO, no TAA), Comfort (three cascades), High (four cascades). `tier_rules.rs`: an integrated GPU is Potato; a discrete GPU under 4 GB is Potato; a discrete GPU at 4 GB or more is Comfort; High is never picked by rule.
- Lighting: Forward+ core (`core.rs`). The forward pass keeps no normal or albedo target (`buffers.rs`), so there is no deferred G-buffer. Baked arena lightmaps and SH probes (`baked_gi.rs`, `ibl.rs`). RT sun shadows and TLAS, tier-gated (`rt_shadows.rs`, `accel.rs`, refit budget 1.5 ms).
- Shadows: cascades up to four (`shadow_cascade.rs`, `shadow_post.rs`), PCF taps (`v1_ibl_shadow.rs`), contact shadows (`contact_shadow.rs`, `contact_feet.rs`). No PCSS found.
- AO, fog, sky: half-res GTAO (`gtao.rs`), height fog (`fog.rs`, 0.2 ms bar in `arena_day.rs`), a `fog_vol.wgsl` shader (role not checked), physical sky (`sky.rs`, 0.4 ms bar).
- Post: dual-filter bloom (`bloom_dual.rs`), hue-keeping tonemap (`tonemap.rs`, `shaders/tonemap.wgsl`), `.cube` LUT grade (`post/lut.rs`, `lut_present.wgsl`), lens pass with barrel distortion, chromatic aberration, vignette and film grain (`lens.rs`, `post/lens.wgsl`), auto exposure (`auto_exposure_hist.wgsl`), a composite compute (`composite_compute.wgsl`).
- AA and resolution: TAA resolve (`post/taa.rs`, `taa_resolve.wgsl`), SMAA 1x fallback (`aa_scale.rs`; the file's own note says full SMAA T2x costs about 1 ms), EASU plus RCAS upscale (`upscale_easu.rs`, `fsr_easu_rcas.wgsl`), dynamic scale governor (`dynres_gov.rs`). No FXAA pass found.
- Content: decal pool with depth reject (`decals.rs`), octahedral impostors (`impostor.rs`, `crowd_impostor.rs`), instanced crowds (`crowd_inst.rs`, `crowd_vat.wgsl`), outline (`outline.wgsl`), LOD dither (`lod_dither.wgsl`), terrain draw path with no splat shading (`terrain.rs`), particles (`particles_gpu.rs`, `particle.wgsl`).
- Named but not built: `motion_blur` and `ssr` appear only as knob names in `quality.rs` (lines 295-296 and 958-959). No depth of field, light cookie, box-projected probe, planar reflection, water, or foliage wind was found in the render shaders. The word "wind" only matched "window".

## 1. Unity URP: what it has and what Unity says each costs

**Rendering paths and light caps**
- Forward, Forward+, Deferred. Deferred needs Shader Model 4.5; OpenGL and OpenGL ES builds fall back to Forward. Later 6.x manuals also list Deferred+ [S].
- Per-camera cap for extra real-time lights in Forward and Deferred: 256 on desktop and console, 32 on mobile. Forward+ has no per-object cap [S, Unity 6.0 table; the alpha docs host failed to fetch].

**Shadows** [V: shadows-optimization, configure-for-better-performance]
- Cascade count: each cascade is more render passes. Unity gives no per-cascade figure.
- Soft shadows: "might have a significant performance impact" on tile-based GPUs (mobile, untethered XR). Unity says profile it.
- Point-light shadows: a point light's shadow map is six views, "roughly on par with six spot lights". On mobile they use a significant share of the frame; reduce or drop them.
- Max distance: fewer objects in the shadow pass.
- Shadowmask: at most four overlapping mixed lights; medium and lower mobile tiers may use it.

**Ambient occlusion** [V: SSAO reference, with Unity's cost labels]
- Method: interleaved gradient noise (static, insignificant cost) or blue noise (animated).
- Radius: High cost (smaller is faster). Samples: High cost, 12 / 8 / 4 for high / medium / low; going from 4 to 8 doubles GPU load.
- Downsample (half resolution): Very high cost, though a quarter of the pixels.
- Blur: Very high cost. Bilateral (3 passes) / Gaussian (2) / Kawase (1).
- After Opaque: Medium cost; Unity says it helps tile-based mobile GPUs.

**Decals** [V: decal reference, configure page]
- DBuffer adds a DepthNormal prepass, which is less efficient on tile-based GPUs. DBuffer does not run on OpenGL or OpenGL ES, or on particles and terrain details.
- Screen Space decals draw after opaques with normals rebuilt from depth. Unity recommends Screen Space for tile-based mobile.
- Max Draw Distance is a camera-distance cutoff. Normal Blend Low, Medium and High use 1, 3 and 5 depth samples.
- Unity's configure page: decals "create an additional render pass", so use them sparingly.

**Probes, lightmaps, APV**
- Reflection probes: Unity's mobile advice is to turn off Probe Blending and Box Projection [V]. Box projection needs a matching URP asset option [S].
- Light cookies: turn off if unused, or shrink the cookie atlas [V].
- Adaptive Probe Volumes [V: probevolumes-concept]: a volume is a group of light probes, placed automatically in 3D bricks of 64 probes (4x4x4). Default spacing is 1, 3, 9 or 27 m. Each pixel samples the eight nearest probes. A scene must sit in the one active Baking Set. Streaming serves large open worlds. No performance figure is published. The memory note (the active set stays in RAM) is [S].
- Progressive Lightmapper and URP lightmap settings: [U], not fetched this pass.

**Post stack** [V: EffectList, names only]
- The URP effect list: Bloom, Tonemapping, Color Adjustments, Color Curves, Color Lookup (LUT), Lift Gamma Gain, Split Toning, White Balance, Channel Mixer, Chromatic Aberration, Vignette, Film Grain, Lens Distortion, Motion Blur, Depth of Field, Panini Projection, Screen Space Lens Flare, Shadows Midtones Highlights. The page gives no per-effect cost.
- No URP screen-space reflections yet. Unity's SSR is in preview (alpha 6000.6.0a7 or later); a forum post targets Unity 6.7 [S].

**Anti-aliasing and upscaling** [V: anti-aliasing, STP] [S: resolution-scale]
- FXAA: the least resource-intensive option; Unity recommends it for mobile.
- SMAA: much sharper than FXAA; no cost stated.
- TAA: can ghost in extreme cases. Incompatible with MSAA, camera stacking and dynamic resolution.
- MSAA: more resource-intensive than other AA on most hardware. Cheaper on tiled GPUs when no post AA is used.
- STP: a URP upscaling filter. At render scale 1.0 it still runs its TAA stage. Incompatible with dynamic resolution; render scale is the only scaler it accepts.
- Upscaling filters in URP: Bilinear, Nearest, STP, FSR 1 [S]. FSR 2, TAAU, Catmull-Rom, CAS and DLSS are HDRP options [S].

**Mobile checklist** [V: configure-for-better-performance]
- HDR off unless needed. MSAA lower or off. Depth texture off unless a shader needs it. Opaque texture off unless needed. Depth priming disabled on mobile. Store actions Auto or Discard on mobile. Native RenderPass on where supported. SRP Batcher on. LOD cross-fade off on mobile (it uses alpha testing). Baked Lit for static objects, Simple Lit for dynamic ones on mobile. Complex Lit: skip where possible; Clear Coat off.
- Each quality level can point at its own URP asset [S].

## 2. Picks

Each pick: link, what it is, the technique, the low-end cost where a source gives one, and a one-line steal with the engine2040 pass it touches.

### 2a. Unity URP built-in features and samples (P1 to P17)

**P1. URP SSAO renderer feature** [V]
- Link: https://docs.unity3d.com/Manual/urp/ssao-renderer-feature-reference.html
- What: screen-space ambient occlusion as a URP renderer feature.
- Technique: noise-based sampling (interleaved gradient or blue noise), half-resolution option, one of three blurs (Kawase, Gaussian, bilateral).
- Low-end cost: Radius High; Samples High (4 to 8 doubles GPU load); Downsample and Blur Very high; Intensity insignificant.
- Steal (`gtao.rs`, Potato): keep half-res GTAO, use the one-pass blur and 4 samples as the Potato setting, keep contact AO on.

**P2. Shadow cascades and soft shadows** [V]
- Link: https://docs.unity3d.com/Manual/shadows-optimization.html
- What: URP main-light shadow settings and their cost notes.
- Technique: cascade splits, soft-shadow filtering, shadow max distance, per-light shadow budgets.
- Low-end cost: each cascade adds passes; soft shadows heavy on tile GPUs; a point-light shadow is six spot-light shadows.
- Steal (`shadow_cascade.rs`, `shadow_post.rs`): keep one cascade and PCF on Potato; add a per-tier caster distance cap; no point-light shadows on Potato (`torch_cluster.rs`).

**P3. Decal renderer feature** [V]
- Link: https://docs.unity.com/en-us/engine/6000.7/manual/visual-effects/decals/renderer-feature-decal/reference
- What: decal projection with DBuffer or screen-space modes.
- Technique: DBuffer overlays decals in the opaque pass; screen-space draws after opaques with depth-rebuilt normals.
- Low-end cost: DBuffer needs a DepthNormal prepass (worse on tile GPUs, no GLES); Normal Blend uses 1, 3 or 5 depth samples; a max-distance cutoff is available.
- Steal (`decals.rs`, `decal_draw_gpu.rs`): screen-space only, a max-draw-distance cutoff per tier, no prepass-style decal path.

**P4. Adaptive Probe Volumes** [V]
- Link: https://docs.unity3d.com/Manual/urp/probevolumes-concept.html (streaming page: https://docs.unity3d.com/Manual/urp/probevolumes-streaming.html [S])
- What: probe-based GI placed automatically in bricks and sampled per pixel.
- Technique: 64-probe bricks (4x4x4) sized by geometry density; eight-probe per-pixel lookup; baking sets and lighting scenarios; streaming by cell.
- Low-end cost: no published figure. The active baking set's lighting stays in RAM [S].
- Steal (`baked_bake.rs`, `baked_gi.rs`): place bake-time probe bricks by geometry density and stream by cell; the runtime lookup stays on the existing SH path.

**P5. Reflection probes and box projection** [V for the mobile advice; link [S]]
- Link: https://docs.unity.com/en-us/engine/6000.7/manual/lighting-overview/lighting/reflection-probes/introduction [S]
- What: cubemap reflections with parallax correction through a box volume.
- Technique: box projection with a blend-distance ramp; geometry outside every volume falls back to the skybox [S].
- Low-end cost: Unity says turn off Probe Blending and Box Projection on mobile [V].
- Steal (`ibl.rs`, `ibl_arena_env.rs`, Comfort and up only): one baked box-projected cubemap for the arena floor and walls; off on Potato.

**P6. Light cookies** [V note]
- Link: https://docs.unity3d.com/Manual/urp/configure-for-better-performance.html
- What: projected textures on spot and point lights.
- Technique: cookie atlas sampled per light.
- Low-end cost: turn off if unused, or shrink the atlas [V]. Godot's Compatibility renderer has no projector textures [V].
- Steal (`spot.rs`): skip on Potato; use baked projected decals for window light.

**P7. URP anti-aliasing options** [V]
- Link: https://docs.unity3d.com/Manual/urp/anti-aliasing.html
- What: FXAA, SMAA, TAA and MSAA choices.
- Technique: FXAA single-pass edge filter; SMAA edge detect plus blend; TAA history reprojection; MSAA hardware multisampling.
- Low-end cost: FXAA is the least costly and is recommended for mobile; MSAA is costly on most hardware; TAA ghosts in extreme cases.
- Steal (`aa_scale.rs`): Potato gets a single-pass FXAA-class option instead of SMAA 1x; TAA stays on Comfort and up; no MSAA on Potato.

**P8. STP upscaling filter** [V]
- Link: https://docs.unity3d.com/6000.4/Documentation/Manual/urp/stp/stp-enable.html
- What: URP's temporal upscaler.
- Technique: temporal resolve with its own TAA stage, which still runs at render scale 1.0.
- Low-end cost: no figure stated; incompatible with dynamic resolution.
- Steal (`post/taa.rs`, the present-path `taa_upsample`): no new code; keep TAA running at scale 1.0 on Comfort and up.

**P9. URP upscaling filters (Bilinear, Nearest, STP, FSR 1)** [S]
- Link: https://docs.unity3d.com/Manual/resolution-scale-introduction.html
- What: the URP upscaler list. FSR 1 is the spatial filter; FSR 2 and TAAU are HDRP options.
- Technique: FSR 1 is an edge-adaptive spatial upscale (EASU) plus a sharpening pass (RCAS).
- Low-end cost: not stated.
- Steal (`upscale_easu.rs`, `post/upscale.rs`): EASU plus RCAS is the URP FSR 1 equivalent. Set the Potato render-scale knob (`quality.rs`) to a value chosen by measurement.

**P10. URP effect list (post stack)** [V, names only]
- Link: https://docs.unity3d.com/Manual/urp/EffectList.html
- What: the URP post overrides, each set in a Volume.
- Technique: per-effect overrides; the page does not describe the maths.
- Low-end cost: none stated on the page. The configure page says Volume Update Mode can be scripted so volumes are not updated every frame [V].
- Steal (`post/chain.rs`, `composite_compute.wgsl`): check whether the composite pass already folds tonemap, LUT, vignette and CA; on Potato it should be one dispatch (`lens.wgsl` already does CA, vignette and grain in one pass).

**P11. Forward+ and light caps** [S]
- Link: https://docs.unity3d.com/Manual/urp/rendering/forward-rendering-paths.html (Unity 6.4)
- What: Unity's rendering-path comparison.
- Technique: per-camera light caps (256 desktop, 32 mobile, for Forward and Deferred); Forward+ has no per-object cap.
- Low-end cost: not stated.
- Steal (`core.rs`, `quality.rs`): keep the per-camera light cap as a tier knob, with 32 as the Potato reference.

**P12. URP screen-space reflections (preview)** [S]
- Link: https://discussions.unity.com/t/preview-of-screen-space-reflections-for-urp/1721494
- What: Unity's first-party URP SSR, in preview (alpha 6000.6.0a7 or later; the post targets Unity 6.7). Off-screen rays fall back to a probe or the sky.
- Technique: screen-space ray march.
- Low-end cost: not stated.
- Steal (none now): not for Potato. If glossy floors are needed later, one planar reflection on Comfort and up.

**P13. Boat Attack (Unity URP sample)** [V]
- Link: https://github.com/unity-technologies/boatattack
- What: a boat-racing slice built to teach URP. It targets Unity 2019.3, so it is an old reference.
- Technique: custom ScriptableRenderPass for water FX and caustics; planar reflections through SRP callbacks; Gerstner waves (WIP); buoyancy in the C# Job System; Shader Graph shaders; v3 post stack.
- Low-end cost: the README says it is mobile-focused (low-poly art, LODs, no compute shaders). No numbers.
- Steal (new pass, no caustic pass found in `crates/render`): a projected caustic texture on sand as one cheap pass.

**P14. Fontainebleau demo** [V]
- Link: https://github.com/Unity-Technologies/FontainebleauDemo
- What: a forest showcase for photogrammetry and the LayeredLit shader. The README gives a PS4 base target at 1080p30, Unity 2019.3 or later, and Git LFS (the first open can take over an hour).
- Technique: a lighting-condition swap (day, sunset, night), a lightmap switching tool, lightmapped LODs, reflection probes and probe volumes. The README also lists deferred rendering, volumetric fog, planar and screen-space reflections, micro shadowing, contact shadows and a local fog volume.
- Low-end cost: not stated. The README does not name a pipeline; LayeredLit is an HDRP shader, so treat the demo as an HDRP-class reference (my inference).
- Steal (`arena_day.rs`, `baked_bake.rs`): day, sunset and night as three baked lighting sets with a lightmap swap; no runtime cost.

**P15. Gigaya (Unity sample, GDC 2022)** [S]
- Link: https://80.lv/articles/gigaya-an-upcoming-sample-game-by-unity
- What: a stylised 3D platformer sample built on URP, Shader Graph and VFX Graph, focused on a modular character controller. A free download was promised; release status not confirmed.
- Technique: stylised look from URP and graphs.
- Low-end cost: not stated.
- Steal: look reference only.

**P16. Happy Harvest (URP 2D sample)** [S]
- Link: https://discussions.unity.com/t/new-urp-2d-sample-project-happy-harvest-download-now/922428
- What: a top-down 2D farm sample for Unity 2022 LTS.
- Technique: shadow and volume data kept in auxiliary textures; shadows kept out of sprite art.
- Low-end cost: not stated.
- Steal: none for a 3D renderer. Low relevance.

**P17. URP 3D Sample and lighting deep dive** [S]
- Link: https://unity.com/vn/resources/urp-3d-sample-deep-dive-lighting-techniques
- What: a May 2024 webinar that runs lighting demos in the URP 3D sample project; covers probe types and the Rendering Debugger, and previews APV.
- Technique: lighting workflow reference.
- Low-end cost: not stated.
- Steal: watch for probe trade-offs before any APV-style bake work (see P4).

### 2b. Unity asset-store packs and tools (P18 to P33)

**P18. Volumetric Fog & Mist 2 (Kronnect)** [V price and version; features S]
- Link: https://assetstore.unity.com/packages/slug/162694
- Store capture: $69.99, version 31.4.5 (Oct 23, 2026), Built-in and URP, no HDRP, Unity 2022.3.24f1 listed. Description empty.
- Technique [S]: fog volumes with directional-light shadows including cascades; spot and point shadows capped at 8 under Forward and 64 or more under Forward+; reduced-resolution fog upscaled with a bilateral filter; GI from APV on Unity 2023.1+ URP.
- Low-end cost: reduced-resolution rendering is the stated speed lever [S].
- Steal (`fog.rs`, `fog_vol.wgsl`): half-res fog with a bilateral upscale inside the existing 0.2 ms height-fog bar; no new pass.

**P19. Shiny SSR 2 (Kronnect)** [V price and version; features S]
- Link: https://assetstore.unity.com/packages/slug/188638
- Store capture: $51.99, version 32.4.2, Built-in and URP, not HDRP, Unity 2022.3.24f1 listed. Description empty.
- Technique [S]: real-time glossy screen-space reflections. Gaps per the search summary: VR, orthographic cameras, forward-path terrain reflections.
- Low-end cost: not stated.
- Steal (none for Potato): a reference for glossy tuning only.

**P20. Radiant Global Illumination (Kronnect)** [V price and version; technique S]
- Link: https://assetstore.unity.com/packages/slug/225934
- Store capture: $25.99 (50% off $51.99), version 32.3 (Aug 24, 2026), Built-in and URP, not HDRP. Description empty.
- Technique [S]: screen-space indirect light. Tags: ssgi, light bounce, indirect lighting, raytracing.
- Low-end cost: not stated. Screen-space GI has known failure modes (P22).
- Steal (`baked_gi.rs`): none for Potato; keep baked GI.

**P21. HTrace SSGI (URP edition)** [S]
- Link: https://discussions.unity.com/t/htrace-screen-space-global-illumination-released/874485
- What: screen-space GI with URP and HDRP editions; the older H-Trace GI and Occlusion asset is deprecated.
- Low-end cost: not stated.
- Steal: none for Potato.

**P22. Open-source screen-space GI for URP** [S]
- Link: https://discussions.unity.com/t/open-source-screen-space-global-illumination-for-urp/952599
- What: an open-source URP package, tested on Unity 6 only; issues go to GitHub.
- Failure notes from the forum thread: breaks with camera motion in play mode and at close range; a reply blames screen-space limits; a reflection probe is offered as a fallback.
- Steal (`gi.rs`): use the failure list as a test list for any screen-space pass. No port.

**P23. Lumina GI (ARTnGAME)** [S]
- Link: https://discussions.unity.com/t/save-600-bundle-offer-lumina-gi-real-time-true-voxel-based-global-illumination-for-urp-hdpr/887593
- What: voxel-based real-time GI for URP and HDRP, described as building on the SEGI idea.
- Low-end cost: voxel tracing is the expensive end; not measured.
- Steal: none for Potato.

**P24. Enviro 3 Sky and Weather** [S]
- Link: https://discussions.unity.com/t/released-enviro-3-sky-and-weather-system/930064
- What: dual-layer volumetric clouds, dual-layer exponential height fog, volumetric lighting, a preset weather system, and an Additional Weather Pack with 20 more types. The BIRP, URP and HDRP compatibility claims are the developer's. The Asset Store listing was not located.
- Low-end cost: not stated.
- Steal (`arena_day.rs`, `sky.rs`, `fog.rs`): weather as data presets, with the sky baked once per preset. Skip the cloud pass on Potato.

**P25. Beautify 3 (Kronnect)** [S]
- Link: https://assetstore.unity.com/packages/vfx/shaders/fullscreen-camera-effects/beautify-hdrp-165411 (the HDRP edition; per the search summary it sends Built-in and URP users to Beautify 3)
- What: a post pack with bloom, anamorphic flares, outline, chromatic aberration, depth of field and sharpening, per readme copies [S]. Its optional ACES and AgX tonemaps are said to be MIT [S]; AgX is already on the donor list.
- Low-end cost: not stated.
- Steal (`bloom_dual.rs`): an anamorphic streak as one extra pass on Comfort and up.

**P26. MicroSplat (terrain shading)** [S]
- Link: https://discussions.unity.com/t/free-microsplat-a-modular-terrain-shading-system-for-unity-terrains/673704 and https://assetstore.unity.com/packages/tools/terrain/microsplat-global-texturing-96482
- What: the free core shades Unity terrains with up to 32 texture layers in one pass; PBR with per-layer tiling, tint and strength; linear or height-based blend. Paid add-ons add runtime procedural texturing, terrain-to-mesh and decals. Triplanar was not confirmed.
- Low-end cost: one pass; add-ons not measured.
- Steal (`terrain.rs`): a four-layer splat with height-based blend in one pass for ground and sand.

**P27. Flat Kit (toon)** [S]
- Link: https://assetstore.unity.com/packages/vfx/shaders/flat-kit-toon-shading-and-water-143368
- What: toon shading for Built-in and URP. Parametric cel banding (single, stepped or curve); lightmap support; edge detection from depth, normals or colour; stylised water (URP only). Price conflicts in search results ($40 on one blog, $20 on a reseller copy); not verified.
- Low-end cost: not stated.
- Steal (`forward.wgsl`): a three-band cel ramp as one fixed shader variant for units and props, which keeps the variant count low.

**P28. Toon Shaders Pro for URP (Daniel Ilett)** [S]
- Link: https://danielilett.itch.io/toon-shaders-pro-for-unity-urp
- What: URP toon shading and outline through a renderer feature; Forward+ and Deferred; four outline modes; from $15.99.
- Low-end cost: the outline is one pass.
- Steal (`outline.wgsl`): compare its four outline modes with ours; keep one outline pass.

**P29. Cartoon FX Remaster (Free)** [S]
- Link: store page not retrieved. A readme copy (version R 1.0, 2023-03-13) is on third-party mirrors only.
- What: a free VFX pack for Built-in and URP. Its soft particles need the depth texture on. The free build carries the same licence terms as the paid packs, which permit commercial use.
- Low-end cost: sprite-based, low; not measured.
- Steal (`particle.wgsl`, `particle_draw_gpu.rs`): radial alpha falloff plus a depth fade for the puffs, blood and dust. This is the fix for audit defect 8 (hard-edged squares).

**P30. Stylized Vegetation Wind Shader URP 2019** [S]
- Link: https://assetstore.unity.com/packages/3d/vegetation/stylized-vegetation-wind-shader-urp-2019-153080
- What: a URP-only stylised set with wind for fields, lawns and clearings. Unity 2019.5 or later; $7; last release April 2020 (search summary).
- Technique: a vertex wind offset driven by a noise texture.
- Low-end cost: vertex-only; not measured.
- Steal (`forward.wgsl`, vertex stage): wind offset on grass and tree meshes with a per-vertex mask; no alpha test needed.

**P31. Stylized Vegetation Wind Shader URP 2025 (Fab)** [S]
- Link: https://www.fab.com/listings/1a592c1a-49c8-4a95-bf2e-674b95b239f0
- What: cell-shaded grass, trees and bushes with wind; a global wind controller script; alpha-channel control of wind strength; noise-texture inspection tools. Unity 2025.4 or later.
- Steal (`forward.wgsl`): one global wind controller as data feeding the same vertex stage as P30.

**P32. Lux Instant Vegetation** [S]
- Link: https://marketplace.unity.com/packages/vfx/shaders/lux-instant-vegetation-286644
- What: procedural wind on tree and bush meshes; URP and HDRP, not Built-in; Unity 2022.3.20 or later.
- Steal (`forward.wgsl`): the same vertex wind as P30. Pick one, not both.

**P33. Kino and KinoMotion (keijiro)** [S]
- Link: https://github.com/keijiro/Kino (umbrella repo, not fetched; a mirror was observed); KinoMotion observed at https://gittrend.io/repo/keijiro/KinoMotion
- What: Kino bundles post effects, including a streak bloom. KinoMotion is MIT-licensed motion blur using a velocity-tile method. Sources disagree on the target pipeline (HDRP, Post Processing Stack v2, URP). KinoMotion needs motion vectors in RGHalf, which is weak on mobile. Last pushes: umbrella about Nov 2020; KinoMotion about Sep 2017.
- Low-end cost: moderate, with a mobile weak spot.
- Steal (`post/chain.rs`): not for Potato. Motion blur is named in `quality.rs` but has no pass.

### 2c. Godot 4 renderers, addons and shaders (P34 to P40)

**P34. Godot Compatibility renderer (renderer overview)** [V, Godot 4.7 stable]
- Link: https://docs.godotengine.org/en/stable/tutorials/rendering/renderers.html
- What: the OpenGL renderer for older and low-end hardware and for the web.
- Technique: forward single-pass lighting; lights with shadows use multiple passes with less accurate blending.
- Feature table (Compatibility, Mobile, Forward+):
  - Yes on Compatibility: depth and height fog, glow, tonemapping, adjustments, SSAO, MSAA 3D, SSAA, depth texture, fullscreen custom post.
  - No on Compatibility: decals, SSR, volumetric fog, SDFGI, VoxelGI, SSIL, TAA, FSR 2, FXAA, SMAA, compute shaders, light projector textures, PCSS on directional lights, CompositorEffects.
  - Limits on Compatibility: 8 OmniLights and SpotLights per mesh; 2 reflection probes per mesh; RGBA8 colour; 24-bit depth without reverse Z.
  - Cost label: "Low base cost, but high scaling cost".
- Steal (`quality.rs`, Potato row): use the Compatibility column as the Potato checklist. Potato must not rely on SSR, volumetric fog, TAA, decals or light projectors, and reflection probes stay at two per mesh.

**P35. Godot LightmapGI (lightmap tutorial)** [V]
- Link: https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_lightmap_gi.html
- What: baked global illumination with optional directional lightmaps.
- Low-end cost: "baked lightmaps work fine on low-end PCs and mobile devices, as they consume almost no resources at runtime". Directional lightmaps are "slightly more expensive to render". The static bake mode "provides the best performance". Large textures "can require a lot of video memory". Godot renders at most 8 LightmapGI nodes at once. Compatibility renders baked lightmaps, but baking needs RenderingDevice [V, renderers page].
- Steal (`baked_gi.rs`, `baked_bake.rs`): lightmaps are the only GI on Potato. Cap the atlas size. Directional lightmaps off on Potato.

**P36. Godot Forward+-only features** [V]
- Link: https://docs.godotengine.org/en/stable/tutorials/rendering/renderers.html
- What: the features that only Forward+ has: SDFGI, VoxelGI, SSIL, SSR, volumetric fog, TAA, FSR 2, subsurface scattering, PCSS on directional lights, compute shaders.
- Low-end cost: Forward+ has the "highest base cost, and low scaling cost".
- Steal: the do-not-port list for the Potato tier. Do not chase these on the iGPU floor.

**P37. Terrain3D (TokisanGames)** [V]
- Link: https://github.com/TokisanGames/Terrain3D
- What: a Godot 4 terrain addon. MIT. The page shows no release number.
- Technique: up to 32 texture sets; up to 10 LOD levels for the terrain mesh and for foliage; foliage instancing with a shadow impostor.
- Low-end cost: not measured. LOD and instancing are the levers.
- Steal (`terrain.rs`, foliage instancing): the 32-texture cap and the 10-level LOD as targets; the shadow impostor for trees on Potato.

**P38. FXAA for Godot 4 Compatibility renderer** [S]
- Link: https://bluzi.itch.io/fxaa-for-godot-4-compatibility-renderer
- What: an FXAA shader for the Compatibility renderer, which has no FXAA or SMAA (see P34).
- Low-end cost: one pass; not measured.
- Steal (`aa_scale.rs`): the same Potato AA fallback as P7.

**P39. Shaders Showcase - Toon (Urar Studios)** [S]
- Link: https://urar-studios.itch.io/shaders-showcase-toon
- What: a toon shader pack with an outline built as a Next Pass. The listing claims Compatibility and Forward+ support; $10 or more.
- Forum evidence [S]: an outline shader from the Godot Shaders site showed artifacts under Forward+ and no visible effect under Compatibility (https://forum.godotengine.org/t/strange-artifacting-on-outline-shader/131362).
- Steal (`outline.wgsl`): test the outline style on Compatibility first, because the forum reports say outlines break there.

**P40. Godot Ultimate Shader Pack 2 (phonix-cool)** [S]
- Link: https://phonix-cool.itch.io/ultimate-shader-pack2
- What: a toon shader and an outline shader with adjustable colour and width. The licence allows commercial use without credit. The renderer is not named and no price was shown.
- Steal: skip unless tested in a Compatibility-only project.

## 3. URP parity checklist (engine2040 vs URP look)

Status is what the render crate shows. "Unknown" means not checked this pass.

| # | URP look feature | engine2040 today (file) | Have it? | Cheapest match on an iGPU |
|---|---|---|---|---|
| 1 | Forward+ clustered lights | `core.rs` (headless Forward+, tile assign, light cull) | yes | keep the per-camera light cap as a tier knob |
| 2 | Deferred G-buffer | `buffers.rs`: forward keeps no normal or albedo target | no | do not add; screen-space decals only |
| 3 | Shadow cascades (1 to 4) | `shadow_cascade.rs`, `shadow_post.rs` (MAX_CASCADES 4), tiers | yes | Potato stays at one cascade |
| 4 | Soft shadows | PCF taps (`v1_ibl_shadow.rs`, `aa_scale.rs`); no PCSS | partial | keep PCF; skip PCSS |
| 5 | Shadow max distance | not checked | unknown | per-tier caster distance cap |
| 6 | Point and spot light shadows | `spot.rs`, `shadow_cluster/torch_cluster.rs` | partial | none on Potato |
| 7 | Light cookies | none found | no | baked projected decals (P6) |
| 8 | Contact shadows | `contact_shadow.rs`, `contact_feet.rs` | yes | keep |
| 9 | Ambient occlusion | `gtao.rs` (half-res GTAO), `shaders/ssao.wgsl` | yes | half-res with one-pass blur (P1) |
| 10 | Reflection probes with box projection | no box projection; `ibl.rs` SH | partial | one baked box cubemap, Comfort and up (P5) |
| 11 | Adaptive Probe Volumes | `baked_gi.rs` lightmaps plus SH probes | partial | bake-time brick placement (P4) |
| 12 | Baked lightmaps | `baked_gi.rs`, `baked_bake.rs` (validity mask) | yes | keep; the only GI on Potato (P35) |
| 13 | Screen-space GI | none; `gi.rs` runtime GI is gated | no | skip; bake instead (P22) |
| 14 | Ray-traced sun shadows | `rt_shadows.rs`, `accel.rs` | yes, tier-gated | off on Potato |
| 15 | Decals | `decals.rs`, `decal_draw_gpu.rs`, `decal.wgsl` | yes | max-distance cutoff, screen space only (P3) |
| 16 | Height and depth fog | `fog.rs` (0.2 ms bar in `arena_day.rs`) | yes | half-res with bilateral upscale (P18) |
| 17 | Volumetric fog | `shaders/fog_vol.wgsl` (role not checked) | partial | half-res fog volume (P18) |
| 18 | Physical sky | `sky.rs` (0.4 ms bar) | yes | bake once per preset (P24) |
| 19 | Bloom | `bloom_dual.rs` (dual filter, half-res halo) | yes | half-res on Potato |
| 20 | Tonemap | `tonemap.rs`, `shaders/tonemap.wgsl` | yes | keep |
| 21 | Colour grading LUT | `post/lut.rs` (.cube), `lut_present.wgsl` | yes | fold into the composite (P10) |
| 22 | Vignette, grain, CA, barrel | `lens.rs`, `post/lens.wgsl` | yes | vignette only on Potato |
| 23 | Auto exposure | `auto_exposure_hist.wgsl` | yes | keep |
| 24 | Motion blur | named in `quality.rs` (lines 295-296); no pass | no | skip on Potato (P33) |
| 25 | Depth of field | none found | no | skip |
| 26 | TAA | `post/taa.rs`, `taa_resolve.wgsl`, tier knob | yes | Comfort and up only |
| 27 | FXAA | none found | no | single-pass FXAA-class option for Potato (P7, P38) |
| 28 | SMAA | `aa_scale.rs` (SMAA 1x fallback) | yes | keep 1x only |
| 29 | MSAA | `present/gpu_present/msaa.rs`, tier knob | yes | off on Potato |
| 30 | Render scale with FSR 1-class upscale | `upscale_easu.rs` (EASU plus RCAS), `post/upscale.rs` | yes | Potato render-scale knob (P9) |
| 31 | Dynamic resolution | `dynres_gov.rs`, `aa_scale.rs` ScaleController | yes | keep |
| 32 | Temporal upscale (STP-like) | present-path `taa_upsample` | yes | keep; no new code (P8) |
| 33 | Foliage wind | none in shaders ("wind" matched only "window") | no | vertex wind with one global controller (P30 to P32) |
| 34 | Water | no water shader in the shader list | no | one scrolling-normal plane, no reflection on Potato |
| 35 | Terrain splat | `terrain.rs` draw path; no splat shading found | no | four-layer height blend (P26) |
| 36 | Particle soft edges | `particle.wgsl`; audit defect 8 (hard squares) | no | radial alpha plus depth fade (P29) |
| 37 | Outline | `outline.wgsl` | yes | keep |
| 38 | Cel ramp | not checked in `forward.wgsl` | unknown | three-band ramp variant (P27) |
| 39 | Impostors and far LOD | `impostor.rs`, `crowd_impostor.rs` | yes | keep |
| 40 | Instanced crowds | `crowd_inst.rs`, `crowd_vat.wgsl` | yes | keep |
| 41 | LOD cross-fade | `lod_dither.wgsl` | yes | keep dither (Unity says avoid alpha-test fades on mobile) |

## 4. TOP 15 (look gained per millisecond on an iGPU)

Ranking basis: near-zero or cost-saving steals first, then visible gain. Cost basis is the engine's own budget where one exists, otherwise the doc label. Nothing here is measured on an iGPU.

| Rank | Pick | Steal and where | Look gain (1 to 5) | Cost basis |
|---|---|---|---|---|
| 1 | P29 Cartoon FX soft particles | radial alpha and depth fade in `particle.wgsl` | 4 | an alpha change; near zero |
| 2 | P35 Godot LightmapGI rule | lightmaps as the only Potato GI in `baked_gi.rs` | 4 | "almost no resources at runtime" [V] |
| 3 | P30 to P32 vertex wind | `forward.wgsl` vertex stage, one global controller | 4 | vertex-only; no doc label; measure |
| 4 | P26 MicroSplat idea | four-layer height blend in `terrain.rs` | 4 | one pass; measure |
| 5 | P9 EASU plus RCAS at a Potato render scale | `upscale_easu.rs`, `quality.rs` render-scale knob | 3 | saves pixel cost; the scale is set by measurement |
| 6 | P7 and P38 single-pass FXAA-class AA on Potato | `aa_scale.rs` | 3 | cheaper than SMAA 1x; SMAA T2x about 1 ms per the file's note |
| 7 | P18 half-res fog with bilateral upscale | `fog.rs` | 3 | inside the 0.2 ms bar in `arena_day.rs` |
| 8 | P1 half-res GTAO with one-pass blur | `gtao.rs` | 3 | Unity labels Downsample and Blur as the big costs |
| 9 | P27 three-band cel ramp variant | `forward.wgsl` | 3 | ALU only |
| 10 | P10 one composite pass | `post/chain.rs`, `composite_compute.wgsl` | 2 | saves dispatches |
| 11 | P24 weather presets as data | `arena_day.rs`, `sky.rs` | 2 | sky baked once per preset; no per-frame cost |
| 12 | P25 anamorphic bloom streak (Comfort and up) | `bloom_dual.rs` | 2 | one extra pass; measure |
| 13 | P5 baked box-projected arena reflection (Comfort and up) | `ibl.rs` | 3 on Comfort, 0 on Potato | one sample; Unity says off on mobile |
| 14 | P3 decal max distance, screen space only | `decals.rs` | 2 | removes prepass-style cost |
| 15 | P6 baked projected decals in place of cookies | `spot.rs` | 2 | baked; no runtime light cost |

## 5. Dropped and not verified

- Waterways (Arnklit): the asset-library listings say Godot 3.2 and the Godot 4 status is not confirmed. Dropped.
- Stylized Water 2: not located in this pass (`06` also failed to locate it).
- Toon Kit 2: $30, URP only per a compatibility table [S]. Not needed; not picked.
- Beautify LUT pack ($15, 220 LUTs): found only on a third-party readme and a reseller page. Not picked.
- The Gumroad "Beautify3" page is a reseller, not Kronnect. Not used.
- Godot 3D wind shaders on godotshaders.com (Wandering Clipmap, Pixel Art Wind Sway, Stylized Cartoon Grass): not picked. The site is already in `donors-seen.txt`, and the wind claims were not checked. Stylized Cartoon Grass showed no wind in its excerpt.
- Dynamic 2D Grass: 2D only.
- Happy Harvest: kept as a 2D reference (P16) only.
- Catlike Coding SRP tutorials: not searched in this pass.
- Unity Progressive Lightmapper page: [U], not fetched.
- Unity's mobile post-processing page is linked from the configure page but was not retrieved. The mobile default list in a search summary was not used.
- Unity URP asset-settings page (`urp-asset-additional-settings`): covers only the advanced-properties UI, no costs. Not used.
- Enviro Lite: already dropped in `06`.
- Unity URP SSR: preview only; not usable.

## 6. Gaps and caveats

- No source gives an iGPU millisecond figure for any pick. The top 15 uses doc cost labels and engine budgets as proxies. Measure on the potato reference (iGPU, 2 to 4 GB VRAM, 8 GB RAM) before committing any change.
- The Kronnect store captures had empty feature sections. Their features are [S].
- The Unity 6.0 table (caps, upscaler list) comes from search summaries. The alpha docs host failed to fetch.
- Unity documentation URLs differ between docs.unity3d.com and docs.unity.com across 6.x versions. The cost statements matched across the pages fetched.
- Fontainebleau's README names no pipeline; the HDRP reading is my inference from the shader it uses.
- Boat Attack is a Unity 2019.3 sample. Its newer branches are not confirmed here.
- Godot's renderer table is from the 4.7 stable page. Re-check it when the engine pins a Godot version.

## Sources

Unity (fetched [V]): https://docs.unity3d.com/Manual/urp/ssao-renderer-feature-reference.html ; https://docs.unity3d.com/Manual/shadows-optimization.html ; https://docs.unity3d.com/Manual/urp/configure-for-better-performance.html ; https://docs.unity3d.com/Manual/urp/anti-aliasing.html ; https://docs.unity3d.com/6000.4/Documentation/Manual/urp/stp/stp-enable.html ; https://docs.unity3d.com/Manual/urp/probevolumes-concept.html ; https://docs.unity3d.com/Manual/urp/EffectList.html ; https://docs.unity.com/en-us/engine/6000.7/manual/visual-effects/decals/renderer-feature-decal/reference ; https://github.com/unity-technologies/boatattack ; https://github.com/Unity-Technologies/FontainebleauDemo ; https://assetstore.unity.com/packages/slug/162694 ; https://assetstore.unity.com/packages/slug/188638 ; https://assetstore.unity.com/packages/slug/225934

Unity (search-only [S]): https://docs.unity3d.com/Manual/urp/probevolumes-streaming.html ; https://docs.unity3d.com/Manual/resolution-scale-introduction.html ; https://docs.unity3d.com/Manual/urp/rendering/forward-rendering-paths.html ; https://discussions.unity.com/t/preview-of-screen-space-reflections-for-urp/1721494 ; https://docs.unity.com/en-us/engine/6000.7/manual/lighting-overview/lighting/reflection-probes/introduction ; https://80.lv/articles/gigaya-an-upcoming-sample-game-by-unity ; https://discussions.unity.com/t/new-urp-2d-sample-project-happy-harvest-download-now/922428 ; https://unity.com/vn/resources/urp-3d-sample-deep-dive-lighting-techniques ; https://discussions.unity.com/t/htrace-screen-space-global-illumination-released/874485 ; https://discussions.unity.com/t/open-source-screen-space-global-illumination-for-urp/952599 ; https://discussions.unity.com/t/save-600-bundle-offer-lumina-gi-real-time-true-voxel-based-global-illumination-for-urp-hdpr/887593 ; https://discussions.unity.com/t/released-enviro-3-sky-and-weather-system/930064 ; https://assetstore.unity.com/packages/vfx/shaders/fullscreen-camera-effects/beautify-hdrp-165411 ; https://discussions.unity.com/t/free-microsplat-a-modular-terrain-shading-system-for-unity-terrains/673704 ; https://assetstore.unity.com/packages/tools/terrain/microsplat-global-texturing-96482 ; https://assetstore.unity.com/packages/vfx/shaders/flat-kit-toon-shading-and-water-143368 ; https://danielilett.itch.io/toon-shaders-pro-for-unity-urp ; https://assetstore.unity.com/packages/3d/vegetation/stylized-vegetation-wind-shader-urp-2019-153080 ; https://www.fab.com/listings/1a592c1a-49c8-4a95-bf2e-674b95b239f0 ; https://marketplace.unity.com/packages/vfx/shaders/lux-instant-vegetation-286644 ; https://github.com/keijiro/Kino ; https://gittrend.io/repo/keijiro/KinoMotion

Godot (fetched [V]): https://docs.godotengine.org/en/stable/tutorials/rendering/renderers.html ; https://docs.godotengine.org/en/stable/tutorials/3d/global_illumination/using_lightmap_gi.html ; https://github.com/TokisanGames/Terrain3D

Godot (search-only [S]): https://bluzi.itch.io/fxaa-for-godot-4-compatibility-renderer ; https://urar-studios.itch.io/shaders-showcase-toon ; https://forum.godotengine.org/t/strange-artifacting-on-outline-shader/131362 ; https://phonix-cool.itch.io/ultimate-shader-pack2
