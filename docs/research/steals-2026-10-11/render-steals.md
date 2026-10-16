# Render steals for engine2040 (research, 2026-10-11)

Method: every URL was opened with WebFetch on 2026-10-11 (default branch). "Verified" = file exists and the named functions were read from it. "Listing" = path exists in a directory listing, contents not read. "UNVERIFIED" = not confirmed. Line counts are estimates except XeGTAO (854 lines, measured).

Porting gotcha: Bevy's shaders are now `.wesl`, not `.wgsl`. Porting means removing `package::` / `#import` lines and flattening each packet into one WGSL file, then mapping bindings to crates/render's own bind groups.

## Top 12 in build order

| # | steal | repo + file URL | what to port | est. lines | visible gain on arena frame | depends on |
|---|---|---|---|---|---|---|
| 1 | ACES fitted tonemap | https://github.com/TheRealMJP/BakingLab/blob/master/BakingLab/ACES.hlsl | ACESInputMat, ACESOutputMat, RRTAndODTFit, ACESFitted | 50 | Sun-lit sand and armor stop hard-clipping; whole-frame contrast | HDR scene target, exposure |
| 2 | Anisotropic, Charlie sheen, clear-coat BRDF terms | https://github.com/google/filament/blob/main/shaders/src/surface_brdf.fs | D_GGX_Anisotropic, V_SmithGGXCorrelated_Anisotropic, D_Charlie, distributionCloth, distributionClearCoat, visibilityClearCoat | ~120 (3 packets) | Armor streaks and clear coat; sheen on tunics | tangent frame, material params |
| 3 | Cascade setup (PSSM split, stabilization, texel snap) | https://github.com/TheRealMJP/Shadows/blob/master/Shadows/SetupShadows.hlsl | SetupCascades, PlaneFromPoints, OrthographicProjection, bounding-sphere stabilization | ~200 (2 packets) | Sun shadows stop swimming as the camera turns | camera, sun dir, shadow atlas |
| 4 | Cascade select and blend | https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/render/shadows.wesl | get_cascade_index, world_to_directional_light_local, sample_directional_cascade, fetch_directional_shadow | ~150 | Removes cascade seams on sand and walls | row 3 |
| 5 | Soft shadow filter (Castano-13 PCF, then PCSS) | https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/render/shadow_sampling.wesl | sample_shadow_map_castano_thirteen, sample_shadow_map_jimenez_fourteen, search_for_blockers_in_shadow_map, sample_shadow_map_pcss (directional only) | ~150 (2 packets) | Fighter shadows: hard at contact, soft far from it | row 4, comparison sampler |
| 6 | Dual-filter bloom | https://github.com/bevyengine/bevy/blob/main/crates/bevy_post_process/src/bloom/bloom.wesl | downsample_first (karis_average, 13-tap), downsample, upsample (3x3 tent) | ~180 (2 packets) | Torch and sun-glint halos | row 1 (HDR) |
| 7 | Height fog | https://github.com/google/filament/blob/main/shaders/src/surface_fog.fs | fog(): density with fogHeightFalloff | ~40 | Haze band at sand level; far wall separates from floor | depth, camera |
| 8 | IBL: GGX prefilter, cosine irradiance, DFG LUT, SH consumer | https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/light_probe/environment_filter.wesl ; https://github.com/google/filament/blob/main/shaders/src/surface_light_indirect.fs ; https://github.com/JoeyDeVries/LearnOpenGL/tree/master/src/6.pbr/2.2.2.ibl_specular_textured | generate_radiance_map, generate_irradiance_map; 2.2.2.brdf.fs for DFG; Irradiance_SphericalHarmonics (consumer) | ~250 (3 packets) | Metal and skin pick up sky color; armor reflections | environment cubemap (sky or HDRI) |
| 9 | GTAO-style ambient occlusion | https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/ssao/ssao.wesl (XeGTAO-based, verified); reference https://github.com/GameTechDev/XeGTAO/blob/master/Source/Rendering/Shaders/XeGTAO.hlsli (archived 2024, read-only) | ssao compute main pass, preprocess_depth.wesl, spatial_denoise.wesl | ~300 (3 packets) | Contact darkening at feet and wall-to-sand joints | depth + normal prepass |
| 10 | TAA with neighborhood clamp | https://github.com/bevyengine/bevy/blob/main/crates/bevy_anti_alias/src/taa/taa.wesl | taa fragment, clip_towards_aabb_center, sample_history (Catmull-Rom), closest-motion pick | ~150 (2 packets) | Stable armor and sand edges, little shimmer | row 0 (motion vectors), jitter |
| 11 | RCAS sharpen | https://github.com/bevyengine/bevy/blob/main/crates/bevy_anti_alias/src/contrast_adaptive_sharpening/robust_contrast_adaptive_sharpening.wesl ; alt https://github.com/GPUOpen-Effects/FidelityFX-FSR/blob/master/ffx-fsr/ffx_fsr1.h (FsrRcasF) | fragment: cross 3x3, lobe clamp (FSR_RCAS_LIMIT), sharpness | ~60 | Crisp wall texture after TAA | row 10 |
| 12 | Skin: separable SSS and wrap/transmission | https://github.com/iryoku/separable-sss/blob/master/SeparableSSS.h ; https://github.com/google/filament/blob/main/shaders/src/surface_shading_model_subsurface.fs | SSSSBlurVS/SSSSBlurPS (two passes), SSSSTransmittance; surfaceShading wrap + thickness | ~200 (2 packets) | Faces and forearms stop reading as plastic; warm terminator | skin mask, row 5 |

Row 0 (prerequisite for row 10): https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/prepass/prepass.wesl writes motion_vector under MOTION_VECTOR_PREPASS (current minus previous clip position). Verified.

## Per-item notes (shaders, inputs, proving test)

1. ACES: input linear HDR. Test: arena capture with ACES vs current clamp; pixel count at 255 on sun-lit sand should drop, armor hue should not shift. Alt: Tony McMapface https://github.com/h3r2tic/tony-mc-mapface/tree/main/shader (tony_mc_mapface.hlsl plus .dds LUT). AgX GLSL reference: https://github.com/mrdoob/three.js/blob/dev/src/renderers/shaders/ShaderChunk/tonemapping_pars_fragment.glsl.js (AgXToneMapping, agxDefaultContrastApprox, matrices inside the function). The sobotka/AgX repo https://github.com/sobotka/AgX has LUTs and config.ocio only, no shader code (verified).
2. BRDF terms: inputs tangent frame, anisotropy, clear-coat weight and roughness, sheen roughness. Test: armor with anisotropy on vs off; highlight aspect ratio stretches along the tangent. Cloth model: https://github.com/google/filament/blob/main/shaders/src/surface_shading_model_cloth.fs (surfaceShading; calls distribution and visibility from surface_brdf.fs).
3. Cascade setup: inputs view matrix, fov, aspect, near/far, sun dir, shadow map size. Test: pan the camera 10 degrees with stabilization on vs off; a static fighter's shadow edge should move under one texel.
4. Cascade select: inputs view-space depth and cascade far bounds. Test: cascade_debug_visualization on vs off; seams should blend across the overlap band.
5. Filter: inputs shadow depth texture, comparison sampler, light size for PCSS. Test: fighter foot contact stays hard, shadow tip 3 m away softens (measure edge width). PCF alternative: https://github.com/TheRealMJP/Shadows/blob/master/Shadows/PCFKernels.hlsl (FilterSize_ defaults to 2 and is undefined there; set 3/5/7/9; 64 Poisson taps).
6. Bloom: inputs HDR target and mip chain. Test: torch and sun-glint halo radius on vs off; threshold at max should give no bloom.
7. Height fog: inputs depth reconstruction, camera position, fogDensity, fogHeightFalloff. Test: luminance at the knee-height floor band and the far wall base on vs off. Bevy's https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/render/fog.wesl is distance-only (no height term), so take Filament's.
8. IBL: inputs env cubemap mip chain, 9 SH coefficients, DFG LUT. Test: armor with IBL vs flat ambient; reflections gain detail and the shadow side takes sky tint. Filament's IBLPrefilterContext.cpp has IrradianceFilter and SpecularFilter but no SH or DFG generation (verified); SH and DFG source are UNVERIFIED.
9. AO: inputs prepass depth, normals, hilbert LUT (Bevy reads no motion vectors here). Test: AO on vs off; darkening at both fighters' feet and the wall base.
10. TAA: inputs current color, history, depth (5-tap closest motion), motion vectors, jitter. Test: static camera diff between two frames on sand edges; moving fighter ghost length must stay short.
11. RCAS: inputs TAA output and sharpness. Test: Laplacian energy of the wall texture up; overshoot at fighter silhouettes under about 10 percent.
12. SSS: inputs skin mask (material channel), shadow or thickness for transmittance, depth. Test: face terminator with SSS on vs off; warmer, softer red edge only inside the skin mask.

## Beyond the top 12 (verified paths)

- Sky: https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/atmosphere (render_sky.wesl samples transmittance, sky-view and aerial-view LUTs; functions.wesl). Hillaire reference: https://github.com/sebh/UnrealEngineSkyAtmosphere/tree/master/Resources (RenderSkyCommon.hlsl: UvToSkyViewLutParams, SkyViewLutParamsToUv, sampleMediumRGB, CornetteShanksMiePhaseFunction, GetSunLuminance; RenderSkyRayMarching.hlsl listed). Bruneton: https://github.com/ebruneton/precomputed_atmospheric_scattering/tree/master/atmosphere (functions.glsl, listing).
- Volumetric light shafts: https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/volumetric_fog/volumetric_fog.wesl (henyey_greenstein; per-pixel raymarch sampling directional shadow maps; no froxels).
- Contact shadows: https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/contact_shadows.rs (defaults: linear_steps 16, thickness 0.1, length 0.3). Its shader file is UNVERIFIED. Bend's screen-space shadows: https://www.bendstudio.com/blog/inside-bend-screen-space-shadows/ (code zip linked there, NOT downloaded; needs your approval).
- Decals: https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/decal/forward.wesl (get_forward_decal_info, depth fade from prepass). Clustered: https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/decal (clustered.wesl listed).
- Particles (dust, sparks): https://github.com/djeedai/bevy_hanabi/tree/main/src/render (WGSL: vfx_init, vfx_update, vfx_render, vfx_sort). Too large to port whole; take update and render for one emitter type.
- Clustered lights: https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/render (clustered_forward.wesl listed, contents not read).
- Film grain and vignette: Filament inline_vignette.fs and inline_dithering.fs exist (listing: https://github.com/google/filament/tree/main/shaders/src, contents not read).
- Sharpen alt: https://github.com/GPUOpen-Effects/FidelityFX-CAS/blob/master/ffx-cas/ffx_cas.h (CasSetup, CasFilter; listing plus names).

## UNVERIFIED and gaps

- Film grain code: no WGSL or GLSL repo found. The FidelityFX Lens manual https://gpuopen.com/manuals/fidelityfx_sdk/techniques/lens/ names vignette, film grain and chromatic aberration but links no code repo.
- Pre-integrated skin: no GLSL or WGSL repo found. Lux_2.0 https://github.com/larsbertram69/Lux_2.0 exists (Unity; shader path not visible). Penner's GPU Pro 2 article is not on GitHub.
- Contact-shadow shader: Bevy's contact_shadows.rs holds parameters only; keijiro's https://github.com/keijiro/ContactShadows (Unity) exists but its shader paths are not visible.
- Filament SH irradiance and DFG generation source; the file that defines Bevy's tone_mapping() (imported as package::tonemapping, not in the tonemapping directory).
- Fire and sparks beyond Hanabi; rim light (math only, no repo); Bend screen-space shadow code (not downloaded).
- Line counts other than XeGTAO (854) and ACES (50) are estimates.
- Bevy paths are as of its default branch on 2026-10-11; later commits may move files (anti-alias and post-process crates were recently split out).
