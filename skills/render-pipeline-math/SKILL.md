---
name: render-pipeline-math
description: "Use when you add or change a render pass, readback, fog, cascade, draw order, MSAA, TAA, WGSL shader, or do matrix, quaternion, normal, tangent, depth, shadow-bias or color math, or a frame shows mirrored, dark, flickering or vanishing geometry."
---

# Render pipeline rules and math checks

Use when you change the frame pipeline in `crates/render` or write matrix, quaternion, depth or color code. Three parts: pipeline rules, ten math checks, MSAA. wgpu names and API shapes are in skill `wgpu-render`. Lights, sky and AO are in skill `render-lighting`. Culling and VRAM are in skill `render-cull-vram`.

## 1. Pipeline rules

1. A pass counts only if the picture changes when it is on versus off. Zero validation errors proves nothing. Prove it with `docs/book/ref-render.md` section 4 (the pass-truth gate, `crates/render/src/pass_truth.rs`).
2. Never map, poll or wait on a GPU result inside a frame. If the CPU needs it, read it two frames later with a non-blocking map. If only the GPU needs it, keep it in a texture.
3. Fog is a per-fragment function of the view vector and a fog plane. Put a small epsilon in the denominator. Sky pixels use the infinite-distance form (`fog_factor_infinite` in `crates/render/src/shaders/forward.wgsl`). Fog color equals the sky horizon.
4. Tone map once, in the last pass. AO scales the ambient term, never the final color.
5. Allocate GPU buffers once, at max size. Write them with `queue.write_buffer`. Skin on the GPU from a joint palette, one draw per character.
6. A shadow cascade is a view with its own cull and draw list. Size it from the camera-space frustum diagonal, snap its origin to the texel grid, and clamp depth.
7. Sort opaque draws by pipeline, then texture, then front to back. A one-pit arena needs no scene graph and no GPU culling.
8. Renormalize mipped normals for diffuse light. Keep the raw, shorter mipped normal for specular.
9. Temporal effects (TAA, accumulated AO or shadows) need per-pixel velocity that includes skinning. Without it, fighters ghost.
10. Keep one shader file with entry-point variants. Never build WGSL with string replace on another shader. Never copy the BRDF into a second file. See `crates/render/src/twin_shadow.rs` and `crates/render/src/v1_skin.rs`.

### Repo facts

- `crates/render/src/present/gpu_present/submit.rs::map_readback_f32` blocks on `poll(wait_indefinitely)`. Rule 2 forbids a wait like this in a frame.
- The bind probe `dst[i] = src[i] + look.exposure * 0.0` in `crates/render/src/sky.rs` and `crates/render/src/fog.rs` only copies a buffer. It is not a live pass (rule 1).
- `crates/render/src/v1.rs::MeshData` holds positions, normals, uvs, indices and `tangents`. The tangents come from `recompute_tangents` in `crates/assets/src/import_repair.rs`. `engine-assets` owns them, there is no local mirror. Rule 8 needs the tangent on the GPU path: check the carry before adding normal maps.
- `crates/game/src/bin/arena.rs::LOOK_PROBE_EVERY` (120) and `LIVE_FOG_EVERY` (4) set how often the live window runs probes and fog.

## 2. Math checks

Ten checks. Each has a symptom and a quick test (a unit test or a one-frame capture). The stack is glam (column vectors, column-major) feeding wgpu (depth 0..1).

1. **One written convention.** Column vectors, `M2 * M1 * v` (last applied is leftmost), column-major storage. Handedness, up axis and NDC depth range are stated once in the render docs.
   Symptom: mirrored scene, wrong rotation direction, translation in the wrong place, matrix transposed in the shader.
   Test: `translate(5,0,0) * rotate_z(90deg)` on (1,0,0) gives (5,1,0). Upload a matrix with translation (7,8,9) and read it back in the shader.
2. **Mirrors flip winding.** A negative determinant of the upper 3x3 changes handedness, so front faces become back faces.
   Symptom: a mesh with negative scale or a mirrored import vanishes or is lit from inside.
   Test: draw `scale(-1,1,1)` of a cube. Flip the front-face rule for any draw whose determinant is negative.
3. **Drift.** Long products and deep hierarchies lose orthogonality. Rebuild from position, quaternion and scale each frame, or re-orthonormalize (Gram-Schmidt).
   Symptom: slow skew or size creep.
   Test: after 10k composed frames, `M^T M` stays near identity (about 1e-4 in f32).
4. **Quaternions.** Keep them unit length: normalize after every multiply or integration chain. `q` and `-q` are the same rotation, so negate one operand when dot < 0 before lerp or slerp, then renormalize.
   Symptom: a character shrinks or skews. A blend spins the long way round.
   Test: debug-assert `|q|^2` is within 1e-4 of 1. Blend identity with a 350-degree z-rotation at t=0.5 and expect about -5 degrees, not 175.
5. **Normals use the inverse-transpose.** Positions and tangents use `M`. Normals use the inverse-transpose of the upper 3x3, then renormalize. Plain `M` is only safe for rotation plus uniform scale.
   Symptom: lighting wrong on stretched meshes.
   Test: with scale (2,1,1) and a 45-degree face, `dot(n', M t)` must be 0.
6. **Tangent frame (stated once here).** Orthonormalize t, b, n after interpolation. Store the handedness sign in `tangent.w`, bitangent = `w * (n x t)`. Blend two normal maps by converting to slopes (divide xy by z), adding, renormalizing. Never average raw normals.
   Symptom: bumps inverted on mirrored UV halves, seams, washed-out detail maps.
   Test: one quad, bump map, light from +x. The lit side faces the light on mirrored and unmirrored islands.
7. **Reverse-Z.** With a standard projection half the depth range sits between near and 2x near. Use a float depth buffer, infinite far, clear 0, compare Greater. Switch all four together (also the shadow bias sign).
   Symptom: distant z-fighting, or everything vanishes after a half-done switch.
   Test: two surfaces 1 cm apart at 500 m resolve. Clear value and compare op are grepped next to the projection.
8. **Shadow bias.** Constant offset (about 2^-19 in the shadow projection) plus slope-scaled offset (about 3x the largest depth slope) clamped (about 1/128). Signs flip for a reversed projection.
   Symptom: acne stripes, or detached shadows when the bias is too big.
   Test: sun at 80 degrees over a flat floor shows no stripes. A thin wall's shadow touches its base.
9. **Color is linear inside.** Albedo and emissive use sRGB formats. Data maps (normal, roughness, masks) stay linear. Light, blend and filter in linear. Encode once at output.
   Symptom: dark and contrasty, or washed out when encoded twice.
   Test: linear 0.5 must land near byte 188. A black/white checker blurred in linear reads about 188, not 128.
10. **Float hygiene.** No `==` on transformed values. Test length > eps before normalizing (zero length gives NaN). Clamp `acos` and `asin` inputs to [-1,1]. Draw camera-relative far from the origin.
    Symptom: NaN black pixels, shimmer far from origin.
    Test: shift every position by 1e5, no shimmer. Assert uploaded matrices are finite.

Do and don't:

- Do transform normals with the inverse-transpose and renormalize. Don't multiply normals by the model matrix because uniform-scale meshes looked fine.
- Do tag color maps sRGB, data maps linear, and encode once. Don't add `pow(x, 2.2)` on top of an sRGB format, or load color maps as plain Unorm.
- Do change projection, depth clear, depth compare and shadow bias sign together for reverse-Z. Don't swap only the projection and tune near and far until it looks right.

## 3. MSAA (4x)

Eight lines. A pass needs an edge-pixel test plus frame-split ms, never a picture alone.

1. Four samples, not a 4x wider frame. One shading run per pixel. Coverage and depth run per subsample.
2. Color, depth and stencil all grow 4x. A 4x color buffer beside a 1x depth buffer is a bug.
3. Resolve before post and blit: the pass has a `resolve_target`, the average to screen size happens before tone map and present. A resolve after present is a bug.
4. Edges blend and interiors hold. Edge pixels show in-between values with MSAA on and flat steps with it off. Whole-frame softening is a bug.
5. Keep one off switch that picks the fallback path (`crates/render/src/present/aa.rs::AaChoice`), so an A/B capture is possible.
6. Cost is bandwidth, not shading. Frame-split p50 rises at most 2 ms at 1080p. Report before and after numbers (`crates/game/src/bin/arena_frame/mod.rs::frame_split_line`).
7. MSAA stays apart from the temporal and SMAA fallback. Do not wire it through `crates/render/src/post.rs::taa_resolve` or the scale fallback.
8. Where the sample count lives: `sample_count` in `crates/render/src/present/gpu_present/presenter.rs`, `crates/render/src/present/gpu_present/submit.rs` and `crates/render/src/v1_gpu.rs`. Grep finds `sample_count: 1` for the targets in `v1_gpu.rs` and `v1_skin.rs`, so 4x MSAA is not wired yet.

Sources: Lengyel, Foundations of Game Engine Development Vol 1 (ch. 1.3, 1.6, 2.1, 2.7, 3.2) and Vol 2 (ch. 5.3-5.4, 6.3, 7.5-7.6, 8.3.4); Game Engine Gems 1 (ch. 9.6, 11.3-11.4, 16.3, 17.2, 19.2), Gems 2 (ch. 2.3, 5.7, 7.2, 17.2), Gems 3 (ch. 2.2, 2.4, 3.2); Lengyel Vol 2 (ch. 8.3.3-8.3.4, 10.5.1, 10.7, 7.5-7.6, 8.5.2); Gregory, Game Engine Architecture 2nd ed. (ch. 10.2.7, 10.2.8.3, 10.2.8.5, 10.3.2). Search more: `node tools/books.mjs search "render pass pipeline"`.
