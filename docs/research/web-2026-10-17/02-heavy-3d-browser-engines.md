# Heavy 3D browser engines: what to copy

Scout notes, 2026-10-17. Each idea was read at its primary source unless marked "search summary" or GUESS. We run Rust and wgpu, so copy designs and numbers, not code.

## Ideas

| Idea | Source + URL | Licence | Copy or do | Row | Effort |
|---|---|---|---|---|---|
| Compile every pipeline before frame 1 | three.js Renderer docs `compileAsync` (threejs.org/docs/pages/Renderer.html); Godot stutter guide (docs.godotengine.org/en/stable/tutorials/performance/pipeline_compilations.html) | MIT, MIT | Build every pipeline the arena and 8 fighters use behind the loader; print count and ms. Godot's web advice matches | NEW: compile all pipelines behind the loader | M |
| Ordered quality ladder | Babylon.js SceneOptimizer (doc.babylonjs.com/features/featuresDeepDive/scene/sceneOptimizer/) | Apache-2.0 | On fps miss, degrade in this order: merge meshes, shadows, lens flares, post and particles, texture size, render targets, hardware scaling last. Target 60 fps. Standard mode never restores | WEB8, RZ9 | S |
| Adapter benchmark to tier 0-3 | detect-gpu (github.com/pmndrs/detect-gpu) | MIT | Probe fps at startup and map to a tier. Its gfxbench data stopped updating in Dec 2025, so self-host it or write our own probe | WEB8 | S |
| KTX2 cook, transcoded at load | PlayCanvas texture-compression manual (developer.playcanvas.com/user-manual/optimization/texture-compression/); glTF-Transform (github.com/donmccurdy/glTF-Transform) | MIT; MIT; Basis Universal Apache-2.0 | One KTX2 per texture, transcoded to the device format (manual: 202 KB to 46 KB). BC, ETC2 and ASTC are optional WebGPU features, so choose at load. Normal maps need `basisu -normal_map -swizzle gggr -renorm` (PlayCanvas forum, search summary) | VR2 | M |
| Meshopt geometry and LOD chains | meshoptimizer (github.com/zeux/meshoptimizer) | MIT | Cook kit meshes with meshopt compression (decodes at 3-6 GB/s on a desktop CPU); build LOD chains with its simplify functions; store them in the WEB6 bundle | WEB6; NEW: LOD chains for kit meshes | M |
| Coarse-first streaming | PlayCanvas engine v2.23.2 notes (github.com/playcanvas/engine/releases) | MIT | Load the lowest splat LOD across the whole view first, then refine. Order WEB6 bundle blobs coarse first | WEB6 | M |
| Isolation headers from a service worker | Godot web export doc (docs.godotengine.org/en/stable/tutorials/export/exporting_for_web.html) | MIT | Single-thread export is the default since 4.3. Threads need COOP and COEP headers; Godot's PWA option sets them from a service worker. Copy that for WEB10 and ship single-thread first | WEB10 | S |
| Gaussian splats as glTF | KHR_gaussian_splatting (github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_gaussian_splatting) | Khronos glTF (not read) | Its README says "Ratified". Stores position, rotation, scale, opacity, SH and a point fallback; compression is a separate extension. Hero props only | NEW: splat hero props | L |
| Browser realism checklist | Babylon.js specifications (babylonjs.com/specifications/) | Apache-2.0 | Lists PBR, HDR IBL, cascaded shadows (PCF, PCSS), bloom, decals, CPU and GPU particles, lightmap textures, reflection probes. Does not list SSAO or TAA | VR1 | S |

## Top 5 this week

1. Fix the VR3 limits table (see Traps) so the gate tests real defaults. S.
2. Warm-up pass: compile every pipeline behind the loader and print count and ms. M.
3. WEB8 ladder: Babylon's order on fps miss, tier from a startup probe, `tier=` printed with each cut. S.
4. KTX2 and meshopt cook for the kit, with the bundle in coarse-first order. M.
5. WEB10: service-worker isolation headers, single-thread first. S.

## Traps

- VR3 has two wrong defaults. MDN's default column gives `maxUniformBufferBindingSize` 65536 (64 KB, not 16 KB) and `maxVertexBufferArrayStride` 2048 (not 256 B). The rest match, but `maxBindGroups` is pipeline-wide, not per stage.
- Godot's web export is WebGL 2 Compatibility only in the docs read; WebGPU is not supported. Do not cite it as WebGPU precedent.
- Unreal has no UE5 HTML5 export (forum posts, not an Epic statement). Epic's browser route is Pixel Streaming, a server GPU per player.
- Unity's docs recommend Brotli over gzip; name the codec in WEB11's bar. The 2 GB heap figure is from one blog, unverified.
- Khronos lists KHR_meshopt_compression; meshoptimizer's README uses EXT_meshopt_compression. Check which one the loader reads.
- Needle Engine: no licence found (npm page 403, FAQ silent). Wonderland: "start with a free license", pricing unread. Read licences before porting code.
- Spark (MIT) is WebGL2-only, so it gives no WebGPU splat path. Babylon's Node Render Graph was alpha in March 2025 (status unverified); its WebGPU optimisation page returned 404.
- Three.js: an undated forum thread says meshopt geometry failed on WebGPU (search summary). Test on WEB5 before trusting it.
- Splat budgets from third-party news are not in the release page read: GUESS. Virtual shadow maps appear in no browser feature list read: GUESS, no browser path.
- `createRenderPipelineAsync` needs a secure context (MDN). GitHub Pages is HTTPS, so that holds.
