---
name: render-pipeline-rules
description: Use when you add or change a render pass, a GPU readback, fog, shadow cascades, draw sort order, normal maps, a temporal effect or a WGSL shader in crates/render, before you call the pass live or the frame faster.
version: 0.1.0
author: skillworks
---

# Render pipeline rules

Use when you change the frame pipeline in `crates/render`: a pass, a readback, fog, shadow cascades, draw order, normal maps, a temporal effect or a shader file. These are pipeline rules. For matrix and color math, use `render-math-checklist`.

1. A pass counts only if the picture changes when it is on versus off. Zero validation errors proves nothing. [crates/render/src/sky.rs::bind_main]
2. Never map, poll or wait on a GPU result inside a frame. If the CPU needs it, read it two frames later with a non-blocking map. If only the GPU needs it, keep it in a texture. [gems1 9.6, 11.3-11.4; gems2 2.3]
3. Fog is a per-fragment function of the view vector and a fog plane. Put a small epsilon in the denominator. Sky pixels use the infinite-distance form. Fog color equals the sky horizon. [gems3 2.2, 3.2; found2 8.5.2; gems1 16.3]
4. Tone map once, in the last pass. AO scales the ambient term, never the final color. [found2 5.3, 10.5.1; geaen 10.3.2]
5. Allocate GPU buffers once, at max size. Write them with `queue.write_buffer`. Skin on the GPU from a joint palette, one draw per character. [gems1 19.2; gems2 17.2, 5.7]
6. A shadow cascade is a view with its own cull and draw list. Size it from the camera-space frustum diagonal, snap its origin to the texel grid, and clamp depth. [found2 8.3.3-8.3.4; gems1 10.4]
7. Sort opaque draws by pipeline, then texture, then front to back. A one-pit arena needs no scene graph and no GPU culling. [geaen 10.2.8.3, 10.2.8.5]
8. Renormalize mipped normals for diffuse light. Keep the raw, shorter mipped normal for specular. [found2 7.5-7.6; gems3 2.4]
9. Temporal effects (TAA, accumulated AO or shadows) need per-pixel velocity that includes skinning. Without it, fighters ghost. [gems2 7.2; gems1 17.2; found2 10.7]
10. Keep one shader file with entry-point variants. Never build WGSL with string replace on another shader, and never copy the BRDF into a second file. [crates/render/src/twin_shadow.rs; crates/render/src/v1_skin.rs]

## Repo facts

- `crates/render/src/present/gpu_present/submit.rs::map_readback_f32`: the function blocks on `poll(wait_indefinitely)`. Rule 2 forbids a wait like this in a frame.
- `crates/render/src/sky.rs::bind_main`: the bind probe `dst[i] = src[i] + look.exposure * 0.0` only copies a buffer. It is not a live pass (rule 1). The same shape is in `crates/render/src/fog.rs::bind_main`.
- `crates/render/src/v1.rs::MeshData`: it holds positions, normals, uvs, indices plus tangents (`tangents`, built by `compute_tangents` from positions plus UVs). Rule 8 needs the tangent on the GPU path; check the carry before adding normal maps.
- `crates/game/src/bin/arena.rs::LOOK_PROBE_EVERY` = 120 and `crates/game/src/bin/arena.rs::LIVE_FOG_EVERY` = 4 set how often the live window runs probes and fog.

Search more: `node tools/books.mjs search "render pass pipeline"`
