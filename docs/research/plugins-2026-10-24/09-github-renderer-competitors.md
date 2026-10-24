# 09 - GitHub renderer competitors: low-end paths and cheap looks

Research date: 2026-10-24. Scope: how competitor engines and renderers get good looks cheaply, read from their GitHub source and docs, for engine2040 (Rust/wgpu) on a potato PC (integrated GPU, 2-4 GB VRAM, 8 GB RAM) and in the browser. Licence does not matter: ideas and code are ports.

## How to read this

- Eleven engine groups were read by read-only research passes (GitHub through gh; docs through WebFetch). The lead re-checked release tags and a set of key source lines in this pass. Items marked [LC] were checked by the lead. Everything else is as reported by the research pass and carries its link. Treat unmarked claims as leads until a port reads the code.
- Tags: (seen) = the repo or folder appears in C:\engine2040\docs\research\hunt-2026-10-21\donors-seen.txt (line number given where known). (seen: FILE #N) = the same technique already sits in an owner note under docs/research/plugins-2026-10-24/. new = neither.
- Numbers: only numbers the project itself publishes are quoted, with the page or file. No millisecond figure here was measured in this pass. Where a published figure comes from a desktop or non-iGPU device, it says so.
- Engine2040 status (section 4) is a file-name check under C:\engine2040\crates (Glob), not a code read. A name hit means the module exists. Quality and behaviour are not checked.
- No quotes are reproduced. Code is paraphrased.

## 1. Release tags and repeats

Release tags (gh, 2026-10-24):

| Engine | Repo | Latest release | Default branch |
|---|---|---|---|
| Bevy | bevyengine/bevy | v0.20.0 [LC] | main |
| Godot 4 | godotengine/godot | 4.7.2-stable [LC] | master |
| Filament | google/filament | v1.78.1 [LC] | main |
| three.js | mrdoob/three.js | r186 [LC] | dev |
| Babylon.js | BabylonJS/Babylon.js | 9.32.0 [LC] | master |
| Flax | FlaxEngine/FlaxEngine | 1.13.7012 [LC] | master |
| Fyrox | FyroxEngine/Fyrox | v1.0.0 [LC] | master |
| Stride | stride3d/stride | releases/4.4.1, as gh returns it [LC] | master |
| O3DE Atom | o3de/o3de | 2605.0 [LC] | development |
| Unity URP | Unity-Technologies/Graphics | no GitHub release (404); URP 17.6.0 from package.json | master |
| rend3, kajiya, wgpu | BVE-Reborn/rend3, EmbarkStudios/kajiya, gfx-rs/wgpu | not checked | trunk, main, trunk |

Already seen (donors-seen.txt, line numbers): bevyengine/bevy 258; bevy_anti_alias/src 256; godotengine/godot 860; servers/rendering 1760; google/filament 866; filament/src 685; GameTechDev/XeGTAO 769; aniso/Charlie 155; o3de/o3de 1345; ShaderLib/Atom 1772; stride3d/stride 1925; FyroxEngine/Fyrox 753; mrdoob/three.js 1279; bhouston/three-ntc 260; BabylonJS/Babylon.js 215; playcanvas 831 and 529; gfx-rs/wgpu 794; BVE-Reborn/rend3 326; EmbarkStudios/kajiya 625; iryoku/smaa 1004; TheRealMJP/Shadows 1976; sobotka/AgX 1834; h3r2tic/tony-mc-mapface 906; KhronosGroup/ToneMapping 1062; Bruneton/Hillaire 314; ebruneton/precomputed_atmospheric_scattering 605; GPUOpen-Effects/FidelityFX-FSR 877; shadow/cascade 1775; shadow/csm.rs 1776; light_probe/generate.rs 1102; effect/fxaa.rs 611; volumetric_fog/mod.rs 2116; ssr/mod.rs 1878; meshlet/mod.rs 1222.

Not in donors-seen: FlaxEngine/FlaxEngine (only Forge/old-Flax 726 and two FlaxMCP repos), Unity-Technologies/Graphics, godot-docs, drivers/gles3.

Owner notes that already cover renderer-adjacent ideas: 04 #1 bgfx (one submit API, backend switch), 04 #3 Slang, 04 #4 FidelityFX-SDK (FSR, radiance-caching preview), 04 #5 splat viewer (worker sort); 05 #1 Animation Budget Allocator, #2 Mass representation tiers, #12 Lumen low-end ladder, #13 Nanite fallback, #23 TSR render scale; 06 #9 GPU Instancer Pro, #17 Unity Animation Rigging, #20 Weather FX Free, #22 Toony Colors Pro 2, #23 Bakery GPU lightmapper, #26 Mesh Baker, #27 UnityMeshSimplifier; 03 Dynamic Fog and Mist 2, Amplify Impostors, Automatic LOD; 07 #25 Godot splat raster path, #26 UnityGaussianSplatting, #27 gsplat.

## 2. Engine by engine

Each section gives the low-end path (what is turned off or replaced), the cheap techniques with source, any published numbers, and what was not verified.

### 2.1 Bevy (bevyengine/bevy v0.20.0, main). Repo seen 258.

Crate paths moved in this release: bloom and AA are under crates/bevy_post_process and crates/bevy_anti_alias, and cascades and lights are under crates/bevy_light.

Low-end path:
- WebGL2 build: the root webgl2 feature maps to bevy_internal's webgl feature (Cargo.toml line 679; crates/bevy_internal/Cargo.toml 199-209). Many gates also test wasm32, so native builds keep the desktop path.
- SSAO is off on WebGL2 and WebGPU [LC: crates/bevy_pbr/src/ssao/mod.rs says so in its doc comment]. SSR is off on WebGL2 and WebGPU (crates/bevy_pbr/src/ssr/mod.rs line 75).
- Shadows on WebGL: one directional light and one cascade per light (crates/bevy_pbr/src/render/light.rs 227-244; crates/bevy_light/src/cascade.rs 143-151).
- Per-light environment probes are off on WebGL2 (crates/bevy_pbr/src/light_probe/environment_map.rs 34-36); view-attached environment maps still work. Irradiance volumes without binding arrays use only the nearest volume (irradiance_volume.rs 65-68).
- Clusters: non-clustered fallback under three storage-buffer bindings (crates/bevy_light/src/cluster/mod.rs 195-199). A cluster config of None disables GPU clustering (line 48).
- Deferred: WebGL2 packs a tighter G-buffer (crates/bevy_pbr/src/deferred/functions.wesl). The default opaque method is forward (crates/bevy_material/src/opaque.rs 23-25).
- Auto-off: atmosphere without compute or storage textures (crates/bevy_pbr/src/atmosphere/mod.rs 137-157); SSAO without five storage textures per stage (ssao/mod.rs line 68).
- Meshlets need int64 texture atomics, subgroups, depth-clip control and MSAA off (crates/bevy_pbr/src/meshlet/mod.rs 94-126, 259-261). They are excluded by feature set, not by a WebGL cfg. Not a potato option.
- Device switch: WgpuSettingsPriority::WebGL2 uses downlevel WebGL2 limits (crates/bevy_render/src/settings.rs 20-26, 95-97, 330); the environment variable WGPU_SETTINGS_PRIO=webgl2 selects it.
- AgX and TonyMcMapface need the tonemapping_luts feature; without it the LUT is a placeholder (crates/bevy_core_pipeline/src/tonemapping/mod.rs 56-84, 247-263). TonyMcMapface is the documented default (crates/bevy_render/src/view/mod.rs 312), per the research pass.

Techniques:

| Technique | Cheap variant | Cost on iGPU (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| Horizon AO, XeGTAO-derived: depth prep, AO, spatial denoise | Low = 4 samples per pixel [LC: ssao/mod.rs, Low => (1, 2), 4 spp]; High = 18 | three passes | sample counts only | crates/bevy_pbr/src/ssao | new; XeGTAO seen 769 |
| Bloom: 13-tap downsample, firefly-reduced first level, 3x3 tent upsample | lower max mip dimension (default 512 gives 8 mips) | about two passes per mip; half-res first mip | none | crates/bevy_post_process/src/bloom [LC: bloom.wesl, downsampling_pipeline.rs, upsampling_pipeline.rs, settings.rs, mod.rs] | new |
| Cascaded shadows and filters: up to 4 cascades; Gaussian, Hardware2x2 or Temporal filter | 1 cascade, 1024 map, Hardware2x2 | one depth pass per cascade per light | none | crates/bevy_light/src/cascade.rs | seen 1775; TheRealMJP/Shadows seen 1976 (related) |
| AA ladder: FXAA, SMAA, TAA | FXAA sensitivity low; SMAA preset low | FXAA one pass; SMAA three; TAA adds prepasses and history; TAA needs MSAA off | none (taa/mod.rs line 88 says cost scales with resolution) | crates/bevy_anti_alias/src | folder seen 256; SMAA seen 1004 |
| Contrast-adaptive sharpening | strength 0 disables it | one full-screen pass | none | crates/bevy_anti_alias/src/contrast_adaptive_sharpening | new |
| Atmosphere LUTs (Hillaire 2020): transmittance, multiscatter, sky-view, aerial-view | LookupTexture mode (default) with small LUTs | four compute LUT passes plus sky; self-disables without compute | none (0.16 post says low cost, no figure) | crates/bevy_pbr/src/atmosphere | seen 314, 605 |
| Tonemap LUT (AgX, TonyMcMapface): 3D LUT from KTX2 | Reinhard or fitted ACES (analytic, no LUT) | one 3D fetch per pixel | none | crates/bevy_core_pipeline/src/tonemapping; enum in crates/bevy_render/src/view/mod.rs | seen 1834, 906, 125 |
| Light probes: specular and diffuse cubemaps; voxel irradiance in 3D textures | view-attached env map only; one volume | per-pixel sampling; filter pass when generated | none | crates/bevy_pbr/src/light_probe | seen 1102 |

Published numbers (Bevy): the 0.16 release post, read through a WebFetch summary and not re-read, reports a GPU-driven scene at 10.16 ms (0.16) against 33.55 ms (0.15), on a mobile RTX 4090 under Vulkan on Linux. That is a desktop GPU, not an iGPU figure.

Not verified: the meshlet WebGL exclusion is inferred from the feature list; SSAO half-res and denoise radius; FXAA and CAS internals; atmosphere LUT update cadence; Tony LUT size; whether default builds enable tonemapping_luts.

Ideas only, from rend3, kajiya and wgpu (folders, no numbers):
- rend3 (trunk): rend3-routine/src has forward.rs, skybox.rs, tonemapping.rs, clear.rs and pbr/ and common/ folders. There is no routine-level shadow or culling file. https://github.com/BVE-Reborn/rend3/tree/trunk/rend3-routine/src (new; repo seen 326).
- kajiya (main): GI folders ssgi/, ircache/, wrc/, rtdgi/, rtr/. The last four hold ray-generation shaders and need ray tracing. The cheapest candidate is ssgi/: https://github.com/EmbarkStudios/kajiya/tree/main/assets/shaders/ssgi (new; repo seen 625).
- wgpu examples (trunk): shadow, skybox and mip-pass examples: https://github.com/gfx-rs/wgpu/tree/trunk/examples/features/src/shadow (new; repo seen 794).

### 2.2 Godot 4 (godotengine/godot 4.7.2-stable, master; docs godotengine/godot-docs master)

Repo seen 860; servers/rendering seen 1760. godot-docs and drivers/gles3 are not seen.

Low-end path:
- Renderer choice: forward_plus is the default [LC: main/main.cpp, rendering_method set to forward_plus at line 2601; the mobile branch is at line 2609]. The web default is Compatibility, and the docs describe an automatic fallback to Compatibility when RenderingDevice is missing (godot-docs tutorials/rendering/renderers.rst).
- Mobile (servers/rendering/renderer_rd/forward_mobile/render_forward_mobile.cpp): the docs remove SSAO, SSIL, SSR, SDFGI, VoxelGI, volumetric fog, TAA, FSR and contact shadows. The SDFGI path has a low-end skip comment. Kept: MSAA, FXAA, SMAA, glow, fog, tonemap, DOF, debanding, and 256 lights per view.
- Compatibility (drivers/gles3/rasterizer_scene_gles3.cpp): RGBA8 colour with no internal HDR, 24-bit depth without reverse-Z, no compute, no FXAA, SMAA, debanding, TAA or DOF. Shadowed lights are multi-pass. Kept: SSAO (in drivers/gles3/shaders/effects/post.glsl), glow, fog, tonemap, 3D MSAA, baked LightmapGI, reflection probes (two per mesh), and eight omni and eight spot lights per mesh. This is the web default.
- Shadow atlas size and a 16-bit directional option: servers/rendering/renderer_rd/storage_rd/light_storage.cpp.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| Compatibility path: single-pass forward, no HDR, no compute | make it the potato and web default | low base | none | drivers/gles3/rasterizer_scene_gles3.cpp | seen: 07 #25 (path idea); feature matrix new |
| SSAO on GLES3: quality, half-size, adaptive, blur and fade knobs | half size, lowest quality, no blur | one post pass plus blur passes | none | drivers/gles3/shaders/effects/post.glsl | new |
| Baked LightmapGI | bake on desktop and ship | near zero at runtime (docs) | none | godot-docs tutorials/3d/global_illumination/using_lightmap_gi.rst | seen: 06 #23 (idea) |
| Reflection probe, Once update mode | renders once per move | one cubemap render per move | none | godot-docs tutorials/3d/global_illumination/reflection_probes.rst | new |
| Tonemap with FXAA and debanding in one pass | linear plus FXAA | one full-screen pass, mostly ALU | none | servers/rendering/renderer_rd/shaders/effects/tonemap.glsl | new |
| SMAA | FXAA (one pass) instead | three passes, no temporal | none | servers/rendering/renderer_rd/effects/smaa.cpp | seen 1004 |
| Bilinear and FSR1 scaling | bilinear at lower render scale | one upscale pass | none | servers/rendering/renderer_rd/effects/fsr.cpp | render scale seen: 05 #23; FSR1 new |
| 16-bit directional shadow maps | 16-bit map, smaller atlas | half the depth memory (qualitative) | none | servers/rendering/renderer_rd/storage_rd/light_storage.cpp | new |

Published numbers (Godot): none found. The docs describe costs qualitatively.

Not verified: Forward+ gating for SSR and SDFGI (docs table and keyword counts only). drivers/gles3/environment/gi.cpp mentions voxel GI many times, yet the docs say Compatibility has no VoxelGI, so it is probably a stub. The 16-bit default, and the FSR1 and SMAA internals.

### 2.3 Filament (google/filament v1.78.1, main)

Seen: google/filament 866; filament/src 685; GameTechDev/XeGTAO 769 (cited by the GTAO path); aniso/Charlie 155.

Low-end path:
- Feature levels (filament/backend/include/backend/DriverEnums.h; filament/include/filament/Engine.h): FL1 (GLES 3.0) is the default. Devices without GLES 3.0 drop to FL0, where post-processing is off (filament/include/filament/View.h). FL2 adds ES 3.1 features, cube arrays and 16 texture units; FL3 has 31.
- Dynamic resolution is off unless the backend reports frame timing (filament/src/details/View.cpp).
- Upscaler tier (filament/src/PostProcessManager.cpp [LC: the SGSR include is at line 60 and the SGSR1 call is near line 3377]): LOW is bilinear, MEDIUM is SGSR1, and HIGH and ULTRA are FSR1. Translucent views are forced to LOW. [LC: RELEASE_NOTES.md lines 1220-1221 confirm that dynamic-resolution MEDIUM, HIGH and ULTRA use FSR1.]
- Defaults (filament/include/filament/View.h): FXAA on, TAA off, MSAA off. HDR colour at LOW or MEDIUM is an opaque R11G11B10F buffer (filament/include/filament/Options.h).
- The fog shader (shaders/src/surface_fog.fs) overlaps 03 Dynamic Fog and Mist 2.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| Feature-level tiers FL0 to FL3 | ship FL1 with LOW AO and bloom, FXAA only | tier choice, no runtime cost | none | filament/backend/include/backend/DriverEnums.h | seen: 05 #12 (tier ladder) |
| Dynamic resolution with three upscalers: bilinear, SGSR1, FSR1 with RCAS | SGSR1 as the middle tier | one upscale pass each | FSR1 "about 2 ms at 4K after a 4.6x speedup" [LC: RELEASE_NOTES.md line 1220; device not stated]. No SGSR figure | filament/src/PostProcessManager.cpp | seen 685; tiers seen: 05 #23; SGSR tier new |
| SAO ambient occlusion: half-resolution AO buffer by default; GTAO and SSCT options | LOW quality, no low-pass | one AO pass plus blur passes | none (code constants: 7, 11, 16 samples) | filament/src/materials/ssao/ssao.cpp | seen 685; GTAO path cites XeGTAO (seen 769) |
| Bloom: fixed-size bloom buffer and mip chain | LOW quality, 3 to 4 levels | several down and up passes | none; docs say HIGH is much slower on mobile | filament/src/PostProcessManager.cpp | seen 685 |
| IBL spherical-harmonic irradiance | 2 bands; roughest prefiltered mip for specular | SH is ALU; the roughest-mip read is one fetch | docs advise 2 bands for performance; no ms | shaders/src/surface_light_indirect.fs | seen 866 |
| Shadows: per-light map, cascades, PCF, VSM, PCSS | PCF, 1024 map, one cascade | one depth pass per shadowed light and cascade | none | filament/include/filament/LightManager.h | seen 866; avoid VSM (research pass: ELVSM roughly doubles shadow-map memory) |
| Specular occlusion and cloth | specular AO SIMPLE; cloth only where used | ALU per pixel | none | shaders/src/surface_ambient_occlusion.fs | seen 155 (Charlie term) |

Published numbers (Filament pages): docs/Filament.md.html gives 60 fps at 2048x1440 with 4x MSAA on a Nexus 9 (Tegra K1), a 2014 device. RELEASE_NOTES.md gives FSR1 at about 2 ms at 4K after a 4.6x speedup (device not stated) and DoF at about 4 ms on a Pixel 4 (research pass).

Not verified: FL2 and FL3 per-feature gating; the default shadow type; ULTRA AO values; the GTAO quality mapping; specular-AA shader code (only a material option, in filament/src/MaterialDefinition.h); FSR1 and SGSR internals. The Filament.md source is not in the tree; only the built docs page was read.

### 2.4 Flax (FlaxEngine/FlaxEngine 1.13.7012, master)

Not in donors-seen.

Low-end path (research pass):
- One enum drives the tiers: Quality Low, Medium, High, Ultra (Source/Engine/Graphics/Enums.h 693-712). Each effect reads its own global.
- AO: Low uses QualityLevel 1, two blur passes and SkipHalfPixels; Ultra uses QualityLevel 3 and three blur passes (Source/Engine/Renderer/Lighting/AmbientOcclusionPass.cpp 249-270).
- SSR: 40, 55 and 70 samples for Low, Medium and High (default 60); SDF step scale 1.5, 1.0 and 0.8 (Source/Engine/Renderer/Reflections/ScreenSpaceReflectionsPass.cpp 192-209).
- GI (DDGI): 96, 128, 192 and 256 rays per probe from Low to Ultra; probe spacing clamped to 10-1000 (Source/Engine/Renderer/GI/DynamicDiffuseGlobalIllumination.cpp 417-432).
- Shadows: bias raised at lower tiers (Source/Engine/Renderer/Lighting/ShadowsPass.cpp 769-817). Docs (WebFetch summary): PCF 1x1, 2x2 and 3x3 for Low, Medium and High; 32-tap PCSS at Ultra.
- Editor bake mode drops AO, shadows, AA, bloom and SSR (Source/Engine/Renderer/Renderer.cpp 247-262).
- Web target: none seen; the README names platforms only.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| DDGI probe GI | 96 rays; up to 4 cascades | high | Docs (WebFetch summary): probe spacing 200-500 world units advised; software reflections 0.5-1 ms and specular 0.5-0.9 ms on an RTX 2080 Ti; DDGI 200-300 MB. Desktop device. A search snippet gave 100-150 units, which conflicts with the fetched 200-500 | Source/Engine/Renderer/GI/DynamicDiffuseGlobalIllumination.cpp | new; ladder seen: 05 #12 |
| Tiered AO | QualityLevel 1, skip half pixels | low-medium | none | Source/Engine/Renderer/Lighting/AmbientOcclusionPass.cpp | ladder seen: 05 #12; values new |
| SSR sample ladder | 40 samples, SDF step 1.5 | medium | none | Source/Engine/Renderer/Reflections/ScreenSpaceReflectionsPass.cpp | new |
| Cascades plus static shadow atlas | static tiles cached; defrag threshold 0.7; max 4 cascades (Source/Engine/Renderer/Config.h line 109); PCF 1x1 | medium | docs example atlas 4096 (WebFetch) | Source/Engine/Renderer/Lighting/ShadowsPass.cpp | new |
| AA chain with sharpening | FXAA on low | low | none | Source/Engine/Renderer/Renderer.cpp 98-175 | new |
| Upscale | MultiScaler: plain and bilateral upscale | low | none | Source/Engine/Renderer/Utils/MultiScaler.h 21-22 | new |

Not verified: the Ultra value of the enum member; the GraphicsSettings asset path (guessed, missing); AO, SSR and DDGI shader code not read; docs numbers are WebFetch summaries.

### 2.5 O3DE Atom (o3de/o3de 2605.0, development)

Seen: o3de/o3de 1345; ShaderLib/Atom 1772.

Pass inventory (Gems/Atom/Feature/Common/Assets/Passes/): SSAO and GTAO (SsaoCompute, GtaoCompute); SSR (ReflectionScreenSpace*); bloom; tone mapping (DisplayMapper; ACES code under Code/Source/ACES); TAA; SMAA; MSAA resolve; cascaded shadows (CascadedShadowmaps); exponential shadows (EsmShadowmaps); projected shadows; diffuse GI (DiffuseGlobalFullscreen).

Low-end path:
- iOS and Android default to the low-end pipeline (docs, WebFetch; repo LowEndRenderPipeline.azasset). It is selected by the LowEndPipeline material tag and registered in material_pipelines.setreg (line 7).
- LowEndPipeline.pass: no SSAO, SSR, bloom, TAA or SMAA, DoF or deferred fog. It keeps MSAA resolve, light adaptation, skinning and UI.
- LowEndForward.pass: exponential shadow maps for directional and projected lights.
- Docs matrix: 12 features supported and 11 not (AO, SSR, DoF, bloom, SMAA, TAA, deferred fog, SSS, RTX GI, PBR sky, terrain). The main pipeline supports all 23.
- Passes/Mobile/: a separate pipeline with a mobile DisplayMapper; mobile SSR is in ReflectionScreenSpaceMobile.pass.
- Web: not documented. An Emscripten bootstrap trait mentions MobileRenderPipeline; shipping status unverified.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| Whole low-end pipeline: pass list swapped per material tag | drops AO, SSR, bloom, TAA and SMAA | low | docs: none. Blog (WebFetch summary) claims up to 400% mobile gain in 24.09.0, with no scene or device given | Gems/Atom/Feature/Common/Assets/Passes/LowEndPipeline.pass | ladder seen: 05 #12; the pass-list swap new |
| Exponential shadows | replaces CSM and PCF on low | low-medium | none | Passes/LowEndForward.pass | new |
| Mobile SSR | separate SSR and blur passes | medium (pass count) | none | Passes/ReflectionScreenSpaceMobile.pass | new |
| Probe-grid GI | off on low | high | none | Gems/DiffuseProbeGrid/Code/Source/Render/DiffuseProbeGrid.cpp | new; skip on potato |

Not verified: web shipping status; setreg contents beyond the line read; no shader code read; no GI numbers. The 400% figure has no scene or device.

### 2.6 Stride (stride3d/stride, master; latest release tag releases/4.4.1)

Seen: stride3d/stride 1925.

Low-end path:
- The default compositor is gated by GraphicsProfile. At Level_10_0 or higher it adds shadow-map renderers and clustered point and spot lights. Below that it has only ambient, directional, skybox, point and spot group renderers (sources/engine/Stride.Engine/Rendering/Compositing/GraphicsCompositorHelper.cs 60-104). With post effects on, everything is disabled except colour transforms, which hold ToneMap (lines 36-49).
- The Level-9 template (samples/NewGame/NewGame/MyTemplate.Game/Assets/GraphicsCompositor.LDR.sdgfxcomp) has ShadowMapRenderer null, LightProbes false, PostEffects null, LightShafts null and MSAA None.
- Profile enum: sources/engine/Stride.Foundation/Graphics/GraphicsProfile.cs.
- Shadow map size = 1024 base times a factor (/8, /4, /2, x1, x2) (sources/engine/Stride.Rendering/Rendering/Lights/LightShadowMapSize.cs 13-48). Cascades are 1, 2 or 4 (LightShadowMapCascadeCount.cs).
- Web: no WebAssembly mention in the README; iOS and Android only.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| Profile-gated default compositor | Level-9 drops shadows, probes and post | low | none | GraphicsCompositorHelper.cs | ladder seen: 05 #12; gating new |
| Shadow size ladder | XSmall or Small factor | low-medium | docs (WebFetch): base 1024 | LightShadowMapSize.cs | new |
| Cascade enum 1, 2, 4 | 1 cascade | medium | none | LightShadowMapCascadeCount.cs | new |
| AO sample count (default 13) | fewer samples, one blur | medium | code comment: more samples means less noise and more cost | sources/engine/Stride.Rendering/Rendering/Images/AmbientOcclusion/AmbientOcclusion.cs | new |

Not verified: clustered light code and FXAA or TAA not read; the full default post chain not traced; docs numbers from a WebFetch summary.

### 2.7 Fyrox (FyroxEngine/Fyrox v1.0.0, master)

Seen: FyroxEngine/Fyrox 753.

Low-end path [LC: fn ultra at settings.rs line 179, high at 215, medium at 256, low at 297]:
- Four presets in fyrox-impl/src/renderer/settings.rs: ultra, high, medium, low. The default is high(), so a potato build must pick low explicitly.
- low(): shadow maps 1 px and disabled; SSAO, light scatter, FXAA, bloom and parallax off (lines 297-330).
- medium(): 512 shadow maps at half precision, no soft shadows; CSM 512 without PCF; SSAO and FXAA on (256-294).
- high(): 1024 shadow maps; CSM 2048 with PCF (215-253).
- Deferred for opaque, forward for transparent (ARCHITECTURE.md 37-38).
- Web target: yes; the README (line 31) says the demos run in a web browser.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| Preset ladder: four tier structs | low() turns every extra off | low | none | fyrox-impl/src/renderer/settings.rs | ladder seen: 05 #12; values new |
| Cascaded shadows: 3 cascades; size per tier; PCF toggle | 512 map, no PCF | medium | none | fyrox-impl/src/renderer/shadow/csm.rs (seen 753); fyrox-impl/src/scene/light/directional.rs line 55 (seen 753) | new |
| SSAO: 32-sample kernel, 4x4 noise, blur | off on low | medium | none | fyrox-impl/src/renderer/ssao/mod.rs | new |
| HDR exposure: auto adaptation chain or fixed manual value | manual exposure | low | none | fyrox-impl/src/renderer/hdr/mod.rs 343-348 | new |

Not verified: the tone-map operator shader was not read; the lightmapper location (ARCHITECTURE.md lines 100 and 320) was not traced; no published numbers found.

### 2.8 three.js (mrdoob/three.js r186, dev)

Seen: mrdoob/three.js 1279; bhouston/three-ntc 260.

Low-end path [LC: WebGPURenderer.js line 41 documents forceWebGL and line 57 is the switch]:
- Backend: WebGPU when present, otherwise WebGL 2 with a warning. Switch: forceWebGL (src/renderers/webgpu/WebGPURenderer.js 22-24, 41, 55-69).
- Adapter: powerPreference passes through to requestAdapter, so a low-power hint works (WebGPUBackend.js 216, 221).
- Shadows: PCFSoft is deprecated since r186 and remapped to PCF (src/constants.js 73; src/renderers/webgl/WebGLShadowMap.js 99-102). VSM adds horizontal and vertical blur passes (same file, 415, 424).
- Post: pass targets follow the composer size (drawing buffer times pixel ratio) and default to half-float (examples/jsm/postprocessing/EffectComposer.js 61, 69, 152, 342). Switch: composer pixel ratio.
- Tone-mapping enum (src/constants.js 423-483), consumed by examples/jsm/postprocessing/OutputPass.js.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| WebGPU with WebGL 2 fallback | forceWebGL; low-power hint | backend choice; no per-frame cost | none (the r184 notes claim 3.0x compile speed, not frame time) | src/renderers/webgpu/WebGPURenderer.js | seen 1279; backend idea seen: 04 #1 |
| Shadow map ladder: basic, PCF (default), PCFSoft (deprecated), VSM | basic or PCF, sun only | basic cheapest; PCF one filtered pass; VSM two blur passes | none | src/renderers/webgl/WebGLShadowMap.js | new |
| Tone curve choice: Reinhard, Cineon, ACES, AgX, Neutral in one output pass | Neutral or Reinhard | ALU only; AgX and ACES add matrix work | none | src/renderers/shaders/ShaderChunk/tonemapping_pars_fragment.glsl.js | new |
| Post pass set: GTAOPass, SAOPass, SSAOPass, SMAAPass, UnrealBloomPass, SSRPass | SMAA plus half-res bloom; drop AO and SSR | GTAO, SAO and SSAO need normal and AO targets plus blur; SSR several full-size targets; SMAA three full-size passes | none | examples/jsm/postprocessing/ | new |
| Composer resolution scale | pixel ratio below 1; 8-bit targets where HDR is not needed | fill and bandwidth, linear in pixels | none | examples/jsm/postprocessing/EffectComposer.js | new |
| SceneOptimizer batching (experimental) | merge static meshes into a BatchedMesh at load | CPU and draw-call saver | docs page has no numbers | examples/jsm/utils/SceneOptimizer.js | new |

Published numbers: the r184 release notes claim a 3.0x compile-time gain, not a frame-time gain (https://github.com/mrdoob/three.js/releases/tag/r184).

Not verified: nothing measured; PCF tap counts; the UnrealBloom size step; SSRPass resolution; the r180 quality setting (#31576) was not traced; no mobile-specific path found.

### 2.9 Babylon.js (BabylonJS/Babylon.js 9.32.0, master)

Seen: BabylonJS/Babylon.js 215; playcanvas 831 and 529 (related).

Low-end path:
- SceneOptimizer presets Low, Moderate and High [LC: sceneOptimizer.ts defines the optimization classes, and LowDegradationAllowed is at line 523]. OptimizeAsync uses Moderate (research pass: lines 841-842).
- Cut order (research pass): merge meshes, shadows, lens flares, post-processes, particles, texture cap (1024, 512, 256), then render targets and hardware scaling (caps 2 and 4). Low omits the last two.
- Shadow filter defaults to none; quality defaults to high (packages/dev/core/src/Lights/Shadows/shadowGenerator.ts 417, 579, 1923-1932).
- SSAO2 runs at a ratio (docs example 0.5 for AO, 1.0 for blur), with 8 samples by default. bypassBlur skips the denoise (packages/dev/core/src/PostProcesses/RenderPipeline/Pipelines/ssao2RenderingPipeline.pure.ts 117, 206, 329).
- Energy conservation and SH irradiance default on; specular AA defaults off (packages/dev/core/src/Materials/PBR/pbrBRDFConfiguration.ts 31, 44; pbrBaseMaterial.pure.ts 801, 1770).
- Docs (WebFetch): device-ratio adaptation is off by default and opt-in through the Engine's fourth argument. Snapshot rendering is WebGPU-only.

Techniques:

| Technique | Cheap variant | Cost (qualitative) | Published | Source | Tag |
|---|---|---|---|---|---|
| SceneOptimizer ladder | Low preset | CPU governor; each cut saves GPU work | 60 FPS target and 2000 ms check interval (doc page, WebFetch summary) | packages/dev/core/src/Misc/sceneOptimizer.ts | seen 215; new |
| Shadow filter ladder: none, four exponential variants, Poisson, PCF, PCSS | none filter with low quality | none cheapest; PCF and PCSS take more taps; blurred variants add passes | none | shadowGenerator.ts | new |
| SSAO2 at a fraction of the buffer | ratio 0.5, 8 samples, bypassBlur | half-size AO is the cheap look; blur adds passes | none (doc summary advises values under 1.0; unverified) | ssao2RenderingPipeline.pure.ts | new |
| PBR flag set: energy conservation, SH irradiance, specular AA, multi-scatter lobe | energy conservation off; keep SH; specular AA off | small per-pixel ALU; SH is a fixed cost | source comment: 24 multiply-adds for SH (not timed) | packages/dev/core/src/Shaders/ShadersInclude/harmonicsFunctions.fx | new |
| WebGPU snapshot rendering | fast mode on static scenes | CPU saving; GPU roughly unchanged | none; the page claims a large fast-mode gain (WebFetch summary) | doc page | new |
| CPU freezes and skips: freeze materials, world matrices, active meshes; priority modes | intermediate priority mode | CPU; GPU little change | multiview roughly 2x (XR only, page claim) | optimize_your_scene doc (WebFetch) | new |

Not verified: the WebGPU adapter request (code-search hit, not read); no WebGPU-to-WebGL fallback claim found; preset priorities; SSAO2 default ratio; the 60 FPS and 2000 ms values come from a WebFetch summary; no separate mobile path.

### 2.10 Unity URP (Unity-Technologies/Graphics master; URP 17.6.0)

Not in donors-seen (grep: no match). Used here as the target list: a commercial engine's full look-feature set plus its low-end settings.

Low-end settings (Runtime/Data/UniversalRenderPipelineAsset.cs; defaults from code):
- Render scale, default 1.0. Docs example: 0.7, 0.9 and 0.7 by memory tier (WebFetch summary).
- HDR on by default; HDR precision 32-bit by default, 64-bit as an option.
- MSAA off by default (2x, 4x, 8x).
- Main-light shadows: cascades default 1 (max 4); distance default 50; map size default 2048.
- Soft shadows off by default (quality Medium).
- Additional lights per-pixel by default, per-vertex or off; per-object cap 4; additional-light shadows off. Docs say Forward+ ignores these settings.
- Depth and opaque textures both off; the opaque copy is a 2x downsample.
- SRP Batcher on by default; dynamic batching off.
- Colour grading LDR by default; LUT size 32.
- Upscaling filter default Auto (Linear, Point, FSR, STP).
- Light probes: legacy by default; probe volumes as an option; APV SH bands L1; memory budget medium; streaming off.
- Reflection-probe blending and box projection off; LOD cross-fade on.

Target list (one line each; research-pass paths):
- Forward (default): Runtime/UniversalRenderer.cs; Runtime/ForwardLights.cs. Cheapest with few lights.
- Forward+: tile light culling, no per-object light cap: Runtime/Tiling/TilingJob.cs; ShaderLibrary/Clustering.hlsl. Pays off with many lights.
- Deferred: Runtime/DeferredLights.cs; Runtime/Passes/GBufferPass.cs; Shaders/Utils/StencilDeferred.shader. Costly (G-buffer bandwidth).
- 2D renderer: Runtime/2D/. Not for 3D.
- Depth prepass and copies: Runtime/Passes/DepthOnlyPass, DepthNormalOnlyPass, CopyDepthPass, CopyColorPass. Extra scene draw and full-screen copies.
- Main-light shadows: Runtime/Passes/MainLightShadowCasterPass.cs (one cascade by default).
- Additional-light shadows: Runtime/Passes/AdditionalLightsShadowCasterPass.cs (off by default; costly per light).
- Screen-space shadows (main light only, two passes): Runtime/RendererFeatures/ScreenSpaceShadows.cs; Shaders/Utils/ScreenSpaceShadows.shader. One full-screen pass.
- Contact shadows: none in the listings.
- Motion vectors: Runtime/Passes/MotionVectorRenderPass.cs. TAA, STP and motion blur need it.
- SSAO raster: Runtime/Passes/ScreenSpaceAmbientOcclusion/AAOPass.cs (downsample; 4, 8 or 12 samples; 1-3 blur passes); Shaders/Utils/ScreenSpaceAmbientOcclusion.shader; ShaderLibrary/SSAO.hlsl. Cheap at Low with downsample.
- SSAO GTAO: Runtime/Passes/ScreenSpaceAmbientOcclusion/GTAOPass.cs; Shaders/Utils/GTAO.compute. Compute cost, not measured.
- Screen-space reflections: linear or Hi-Z; full, half or quarter res: Runtime/RendererFeatures/ScreenSpaceReflectionRendererFeature/; Shaders/Utils/ComputeScreenSpaceReflection.shader. Linear advised for low-end.
- Reflection probes: Runtime/ReflectionProbeManager.cs.
- Adaptive Probe Volumes: Packages/com.unity.render-pipelines.core/Runtime/Lighting/ProbeVolume/ProbeVolume.hlsl. Docs: 8 probes per pixel, 64 per brick, spacing of 1, 3, 9 or 27 m. No cost figures.
- Surface-cache GI: RendererFeatures/SurfaceCacheGIRendererFeature/. Heavy; define-gated; not a potato option.
- Decals, DBuffer: Runtime/Decal/DBuffer/DBufferRenderPass.cs. Costly on tile GPUs (its own tooltip says so).
- Decals, screen-space: Runtime/Decal/ScreenSpace/DecalScreenSpaceRenderPass.cs. Cheaper; no per-surface blend.
- Bloom: Runtime/Passes/PostProcess/BloomPostProcessPass.cs; Shaders/PostProcessing/Bloom.shader. Gaussian, Dual and Kawase; mip pyramid up to 16.
- Tonemapping: Runtime/Overrides/Tonemapping.cs; Shaders/PostProcessing/Common.hlsl. None, Neutral and ACES; per-pixel in the uber pass.
- Uber post: Runtime/Passes/PostProcess/UberPostProcessPass.cs; Shaders/PostProcessing/UberPost.shader. Grain, dither, chromatic aberration, distortion, bloom composite and tonemap in one pass.
- Colour grading LUT: Runtime/Passes/ColorGradingLutPass.cs; Shaders/PostProcessing/LutBuilderLdr.shader. Per-frame LUT, cheap.
- DOF and motion blur: Runtime/Passes/PostProcess/DepthOfFieldGaussianPostProcessPass.cs; DepthOfFieldBokehProcessPass.cs; MotionBlurPostProcessPass.cs. Gaussian is lighter; Bokeh is usually heavier.
- Lens flare and Panini: Runtime/Passes/PostProcess/LensFlareDataDrivenPostProcessPass.cs; LensFlareScreenSpacePostProcessPass.cs; PaniniProjectionPostProcessPass.cs. Moderate.
- FXAA and SMAA: FXAA keyword in Shaders/PostProcessing/FinalPost.shader with FXAA3_11.hlsl; Runtime/Passes/PostProcess/SmaaPostProcessPass.cs. FXAA cheap; SMAA moderate.
- TAA: Runtime/Passes/PostProcess/TemporalAntiAliasingPostProcessPass.cs; Runtime/History/TaaHistory.cs. Moderate; needs motion vectors.
- STP: Runtime/Passes/PostProcess/StpPostProcessPass.cs (the code marks it obsolete from 6000.3 in favour of STPIUpscaler). Costly; docs say it does not run with dynamic resolution.
- FSR 1 (EASU, RCAS) and upscaler selection: Runtime/Passes/PostProcess/Fsr1UpscalePostProcessPass.cs; Shaders/PostProcessing/EdgeAdaptiveSpatialUpsampling.shader; Runtime/Passes/PostProcess/UpscalerPostProcessPass.cs. Cheap; spatial.
- Render scale: Runtime/Data/UniversalRenderPipelineAsset.cs. The biggest single lever.
- Dynamic resolution: core handler (Packages/com.unity.render-pipelines.core/Runtime/Common/DynamicResolutionHandler.cs). Unresolved for URP: docs and code disagree (a comment in UpscalerPostProcessPass.cs near line 60 says URP has no DRS).
- Fog: built in (ShaderLibrary/Fog.hlsl); no fog volume in the overrides. Cheap.
- GPU Resident Drawer: asset fields only; code not read.
- On-tile post (mobile): Runtime/RendererFeatures/OnTilePostProcessFeature.cs; OnTileUberPost.shader. Built for tile GPUs; not read.

Cheapest looks with code (research pass):
- Raster SSAO at low samples, Kawase blur, downsample on. AAOPass.cs maps the volume settings to downsample, samples and blur; ScreenSpaceAmbientOcclusion.shader does the work. Passes: AO, one Kawase blur, cleanup; half res when downsampled. Docs (6.6, WebFetch summary): 4 to 8 samples roughly doubles compute; downsample cuts processed pixels to a quarter. Port note: rebuild normals from depth so the normals prepass is skipped.
- Kawase bloom (BloomPostProcessPass.cs): mip count is log2 of the size minus 1, capped by Max Iterations (default 6). Docs say the quarter-res start is best for performance (WebFetch summary). Port note: quarter-res start, cap at 4 to 6 mips.
- Uber post (UberPost.shader): keyword variants cover distortion, chromatic aberration, bloom, tonemap, grain and dither in one fragment program. Port note: one shader with keyword variants for every per-pixel look.
- FSR 1 upscale plus RCAS sharpen (Fsr1UpscalePostProcessPass.cs; sharpen keyword in FinalPost.shader). Docs list FSR 1 for URP. Port note: render at 0.7 to 0.9 and upscale; spatial, so no motion vectors needed.
- Linear SSR at half or quarter res (ScreenSpaceReflectionRendererFeature folder). The in-repo design note gives targets, not measurements: a midrange mobile target of 4 ms at half or quarter res, and a PS5 target of 2.7 ms at full res; linear is recommended for low-end (Runtime/RendererFeatures/ScreenSpaceReflectionRendererFeature/.001-screen-space-reflection-decisions.md).

Published numbers (URP): none measured. The SSAO and SSR figures above are docs or design-note targets.

Not verified (URP): the asset reference page returned 404 at three URLs, so field defaults come from code; dynamic resolution is unresolved; APV runtime cost; lightmaps (ShaderLibrary/GlobalIllumination.hlsl located, not read); name-only items (OnTile post, GPU Resident Drawer code, decal Entities path, light cookies, MipGen, the reflection probe manager, DOF and lens-flare internals); Forward+ requirements; GTAO and surface-cache GI costs. A third-party SSAO figure (1-3 ms on desktop) was excluded: it is not Unity and it is not measured.

## 3. Comparison table: technique by engine

Key: Y = present; L = present with a low-end or cheaper variant; N = absent or off on low; ? = not verified in this pass.

| Technique | Bevy | Godot 4 | Filament | Flax | O3DE | Stride | Fyrox | three.js | Babylon | URP |
|---|---|---|---|---|---|---|---|---|---|---|
| Tonemap curve choice (AgX, Neutral, ACES) | Y (LUT path; fitted ACES) | Y (five curves; names not checked) | ? | ? | Y (ACES) | Y (ToneMap) | Y (operator not read) | Y (AgX, Neutral, ACES) | ? | Y (None, Neutral, ACES) |
| Half-res or tiered AO | L (Low 4 spp) | L (half size on GLES3) | L (SAO half res) | L (QualityLevel 1) | N on low pipeline | L (13 samples default) | N on low (SSAO off) | Y (GTAO, SAO, SSAO) | L (ratio 0.5) | L (downsample and samples) |
| Bloom | Y | Y (kept on Mobile and Compatibility) | Y (LOW 3-4 levels) | ? | N on low pipeline | ? | N on low | Y (UnrealBloom) | Y (cut by SceneOptimizer) | Y (Kawase, Dual, Gaussian) |
| FXAA | Y | Y (Mobile; folded into tonemap) | Y (default on) | L (FXAA on low) | N on low (no TAA or SMAA) | ? | N on low | ? | ? | Y (keyword in final pass) |
| SMAA or TAA | Y (TAA needs MSAA off) | Y (SMAA; no TAA on Mobile or Compatibility) | Y (TAA optional, off by default) | Y | Y (removed on low) | ? | ? | Y (SMAAPass) | ? | Y |
| Upscaler and render scale | N (no FSR; CAS only) | L (bilinear; FSR1 Forward+ only) | L (bilinear, SGSR1, FSR1 tiers) | Y (MultiScaler) | ? | ? | ? | N (pixel ratio only) | Y (hardware scaling step) | Y (render scale, FSR1, STP) |
| Cascaded shadows | L (one cascade on WebGL2) | Y (directional atlas) | Y (cascades, PCF, VSM, PCSS) | Y (up to 4, static atlas) | Y (CSM) | L (1, 2 or 4) | L (3 cascades; off on low) | Y (basic, PCF, VSM) | L (filters; none on low) | L (one cascade by default) |
| Exponential or variance shadows | ? | ? | Y (VSM, ELVSM; avoid) | ? | Y (ESM on low) | ? | ? | Y (VSM) | Y (exponential variants) | ? |
| Screen-space reflections | N (off on WebGL and WebGPU) | N on Mobile and Compatibility; Y on Forward+ | ? | Y (40, 55, 70 samples) | Y (mobile SSR pass) | ? | ? | Y (SSRPass) | ? | Y (linear or Hi-Z; quarter res advised) |
| IBL spherical-harmonic irradiance | ? | ? | Y (2 bands advised) | ? | ? | ? | ? | ? | Y (default on; 24 multiply-adds) | ? |
| Baked lightmaps | ? | Y (LightmapGI; baked on desktop) | ? | ? | ? | ? | L (lightmapper not traced) | ? | ? | Y (lightmaps; Bakery seen) |
| Real-time GI | N | Y (SDFGI, VoxelGI; Forward+ only) | ? | Y (DDGI, 96-256 rays) | Y (probe grid; off on low) | ? | ? | N | N | Y (APV; surface-cache GI is heavy) |
| Sky atmosphere LUTs | Y (Hillaire; off without compute) | ? | ? | ? | ? | ? | ? | ? | ? | ? |
| Fog | Y (volumetric; seen 2116) | Y (Forward+ only; removed on Mobile) | Y (analytic surface fog) | ? | ? | ? | ? | ? | ? | Y (built-in fog) |
| Forward or deferred | Y (forward by default) | Y (Forward+, Mobile, Compatibility) | ? | ? | Y (forward on low; deferred main) | ? | Y (deferred opaque, forward transparent) | ? | ? | Y (Forward, Forward+, Deferred) |
| Web or WebGL low path | Y (webgl2 feature) | Y (Compatibility is the web default) | ? | N (none seen) | ? (Emscripten trait) | N (no WebAssembly) | Y (demos in browser) | Y (WebGPU with WebGL 2 fallback) | Y (WebGPU; fallback not claimed) | ? |
| Dynamic resolution or frame governor | ? | ? | L (only with frame timing) | ? | ? | ? | ? | ? | Y (SceneOptimizer) | ? (unresolved) |
| Batching, instancing, freezing | ? | ? | ? | ? | ? | ? | ? | Y (BatchedMesh, experimental) | Y (freeze, merge) | Y (SRP Batcher on) |

## 4. TOP 15: port this next, ranked by look per millisecond on an iGPU

Method: the rank is a judgement of look gain against pass count and bandwidth. Nothing here is measured. The engine2040 column is a file-name check only, under C:\engine2040\crates, in this pass.

Engine2040 name checks, present: crates/render/src/shaders/ssao.wgsl; crates/render/src/gtao.rs; crates/render/src/present/gpu_present/gtao_live.rs; crates/render/src/tonemap.rs; crates/render/src/shaders/tonemap.wgsl; crates/render/src/post/fsr1.rs; crates/render/src/shaders/post/fsr1_upscale.wgsl; crates/render/src/bloom_dual.rs; crates/render/src/ibl.rs; crates/render/src/sky.rs; crates/render/src/spec_lobe.rs; crates/render/src/shadow_cascade.rs; crates/render/src/shadow_cluster.rs; crates/render/src/dynres_gov.rs; crates/render/src/quality.rs; crates/render/src/tier.rs; crates/render/src/present/fog_fwd.rs; crates/web/src/tier_view.rs; crates/render/src/temporal.rs; crates/render/src/aa_scale.rs; crates/render/src/present/impostor_gpu.rs; crates/render/src/particles_gpu.rs.
Engine2040 name checks, absent: no file name containing ssr, lightmap, fxaa, atmos, esm or sgsr.

1. **Fold FXAA and debanding into the final tonemap pass**, so the potato tier gets a cheap AA by default. Look: medium (edge shimmer on iGPU). Cost: saves a pass compared with a separate FXAA pass; ALU only. Port from: Godot servers/rendering/renderer_rd/shaders/effects/tonemap.glsl (research pass); URP Shaders/PostProcessing/FinalPost.shader with FXAA3_11.hlsl. Engine2040: tonemap.rs and tonemap.wgsl present; no FXAA file name. Status: NEW (smaa seen 1004).

2. **Add AgX or Khronos Neutral as an analytic curve in the existing tonemap pass**, with no LUT. Look: high (highlight roll-off and colour). Cost: near zero (ALU). Port from: three.js src/renderers/shaders/ShaderChunk/tonemapping_pars_fragment.glsl.js (research pass). Keep the Bevy LUT path off the potato tier. Engine2040: tonemap.rs present; check which curves it has. Status: CHECK (seen 1834 AgX, 1062 Khronos ToneMapping, 906 Tony).

3. **Quarter-res linear SSR with a capped sample count.** Look: medium to high on wet or glossy floors and metal. Cost: medium, the most expensive item here, so keep it off the potato default. Port from: Flax Source/Engine/Renderer/Reflections/ScreenSpaceReflectionsPass.cpp (40 samples at Low); URP Runtime/RendererFeatures/ScreenSpaceReflectionRendererFeature/ (linear advised for low-end). Engine2040: no SSR file name. Status: NEW (Bevy ssr/mod.rs, seen 1878, is off on WebGL).

4. **Baked lightmaps for the static city geometry.** Look: high on static surfaces. Cost: near zero at runtime (one texture sample); the bake runs offline. Port from: Godot docs tutorials/3d/global_illumination/using_lightmap_gi.rst (documentation only); URP ShaderLibrary/GlobalIllumination.hlsl (located, not read). Engine2040: no lightmap file name. Status: NEW (06 #23 Bakery is a bake tool, seen as an idea).

5. **SGSR1 as the middle upscale tier**: bilinear at LOW, SGSR1 at MEDIUM, FSR1 with RCAS at HIGH. Look: keeps detail at a lower render scale for one pass. Cost: one upscale pass. Port from: Filament filament/src/PostProcessManager.cpp, the SGSR1 path with its sgsr.h include [LC]. Engine2040: fsr1.rs and fsr1_upscale.wgsl present; no SGSR name. Status: NEW tier (FidelityFX-FSR seen 877; fsr_easu_rcas.wgsl seen 1521).

6. **Exponential (ESM) soft shadows as the low-tier filter**, in place of PCF. Look: medium (soft edges). Cost: blur passes at shadow resolution, cheaper than PCSS. Port from: O3DE Gems/Atom/Feature/Common/Assets/Passes/LowEndForward.pass (seen 1345); Babylon shadowGenerator.ts exponential variants (research pass). Engine2040: shadow_cascade.rs present; no ESM name. Status: NEW filter option.

7. **Shadow tier ladder**: one cascade, 1024 map and PCF 2x2 on low; cascade count 1, 2 or 4. Look: medium. Cost: one depth pass per cascade. Port from: Stride sources/engine/Stride.Rendering/Rendering/Lights/LightShadowMapCascadeCount.cs and LightShadowMapSize.cs (seen 1925); Fyrox fyrox-impl/src/renderer/shadow/csm.rs (seen 753) and the low() preset [LC: settings.rs line 297]. Engine2040: shadow_cascade.rs present; the tier table is the new part. Status: TUNE.

8. **Two-band SH irradiance for IBL diffuse**, with the roughest prefiltered mip for specular. Look: medium on ambient light. Cost: tiny. Port from: Filament shaders/src/surface_light_indirect.fs (seen 866); Babylon packages/dev/core/src/Shaders/ShadersInclude/harmonicsFunctions.fx (research pass). Engine2040: ibl.rs present; the SH band count is not checked. Status: CHECK.

9. **Ordered cut ladder under a frame-time governor**: merge meshes, shadows, lens flares, post-processes, particles, texture cap, render targets, then hardware scale, stopping at the target frame time. Look: keeps the frame rate on the potato tier. Cost: CPU governor. Port from: Babylon packages/dev/core/src/Misc/sceneOptimizer.ts [LC: class list and LowDegradationAllowed at line 523]. Engine2040: dynres_gov.rs and quality.rs present; the cut order across effects is the new part. Status: EXTEND (seen 215).

10. **Sky from precomputed transmittance and sky-view LUTs**, updated at a low rate, with a gradient fallback for the potato tier. Look: high for outdoor scenes. Cost: LUT compute at a low rate plus one lookup per pixel. Port from: Bevy crates/bevy_pbr/src/atmosphere (research pass; self-disables without compute); Bruneton/Hillaire (seen 314) and ebruneton precomputed scattering (seen 605). Engine2040: sky.rs present; no atmosphere name; the method is unknown. Status: CHECK; the LUT method is new.

11. **Bloom tier caps**: quarter or half-res start, 4 to 6 mips, and a firefly-reduced first level. Look: medium. Cost: fewer passes on the low tier. Port from: URP Runtime/Passes/PostProcess/BloomPostProcessPass.cs (quarter-res start; Max Iterations default 6); Bevy crates/bevy_post_process/src/bloom/downsampling_pipeline.rs (13-tap and firefly-reduced first level, research pass). Engine2040: bloom_dual.rs present. Status: TUNE.

12. **Single-pass forward web default path**: no HDR, RGBA8, no compute, baked lightmaps, per-mesh light caps. Look: enables the browser tier. Cost: lowest. Port from: Godot drivers/gles3/rasterizer_scene_gles3.cpp (research pass). Engine2040: crates/web/src/tier_view.rs present. Status: TUNE (idea seen: 07 #25).

13. **Analytic height fog for the low tier**, with no volumetric pass. Look: medium (depth cue; hides draw distance). Cost: one per-pixel formula. Port from: Filament shaders/src/surface_fog.fs (overlaps 03 Dynamic Fog and Mist 2); URP ShaderLibrary/Fog.hlsl. Engine2040: present/fog_fwd.rs present. Status: CHECK (seen: 03 Dynamic Fog and Mist 2).

14. **Energy-conservation and SH flags on, specular AA off, multi-scatter lobe skipped on iGPU.** Look: small to medium. Cost: small ALU. Port from: Babylon packages/dev/core/src/Materials/PBR/pbrBRDFConfiguration.ts (lines 31, 44) and Shaders/ShadersInclude/harmonicsFunctions.fx. Engine2040: spec_lobe.rs present; energy compensation not checked. Status: CHECK (seen 215).

15. **Static batching and freezing of static props at load** (CPU and draw calls). Look: more props at the same frame cost; GPU effect small. Port from: three.js examples/jsm/utils/SceneOptimizer.js (BatchedMesh, experimental); Babylon freeze and merge (doc summary). Engine2040: present/impostor_gpu.rs and particles_gpu.rs present (instancing-related); no static batching name. Status: NEW for static props (06 #26 Mesh Baker seen for the atlas idea).

Already in engine2040 by name (no port needed for the core feature; the list above covers the gaps and upgrades): SSAO and GTAO (ssao.wgsl, gtao.rs), FSR1 (post/fsr1.rs), dual bloom (bloom_dual.rs), cascaded shadows (shadow_cascade.rs), tonemap (tonemap.rs), IBL (ibl.rs), sky (sky.rs), dynamic-resolution governor (dynres_gov.rs), fog (fog_fwd.rs).

## 5. Excluded for the potato tier

- Meshlets (Bevy): need int64 atomics, subgroups, depth-clip control and MSAA off.
- Nanite and Lumen hardware ray tracing: see 05 #12 and #13 (seen).
- kajiya GI (rtdgi, rtr, wrc, ircache): ray tracing.
- URP surface-cache GI and Flax DDGI: heavy per-probe rays; the only figures are desktop ones.
- Godot SDFGI and VoxelGI: Forward+ only.
- Unity APV: no cost figures; memory per brick; not measured.

## 6. Process notes

- Research passes covered Bevy (with rend3, kajiya and wgpu as ideas), Godot 4 and Filament, Flax, O3DE Atom, Stride, Fyrox, three.js and Babylon.js, and Unity URP. Each pass was read-only: no files written, no git, no clones.
- Four parallel sub-agent launches were refused by a concurrency cap, so the passes ran one at a time.
- One research pass opened and closed a browser pane by mistake. No files were touched.
- Only items marked [LC] were checked by the lead. The rest is reported and should be verified by reading the code before a port.
