---
name: ibl-light-checks
description: "Use when a builder or judge wires image-based lighting, normal maps, gloss maps, env cubes or the IBL bake in crates render or assets before calling the light live."
version: 0.1.0
author: skillworks
---

# IBL light checks

Nine checks for an image-based lighting diff. Name the crate in each finding (render, assets). PASS needs a pixel or bake proof per item, never a promise.

1. **Normal map decode.** Ask: does `forward.wgsl` decode normals with bias and unit length, rebuilding z when only xy is stored?
   Flag: raw RGB used as a vector, or `crates/assets/src/mesh_import.rs::fallback_tangents` skipped so mirrored UVs invert. (render)
2. **Tangent handedness.** Ask: is tangent.w kept as +-1 from `recompute_tangents` in `crates/assets/src/import_repair.rs` through `cooked.rs::tangents` to the shader?
   Flag: w dropped or averaged, so mirrored islands wash out and seams show. (assets)
3. **Gloss per texel.** Ask: do k_s and alpha ride per texel through ORM sampling in `forward.wgsl`, not one material scalar?
   Flag: `AMBIENT_FLOOR` hides gloss, so wet and dry sand read the same. (render)
4. **Env cube sampling.** Ask: does `ibl.rs::prefilter_cube_desc` build a cube at infinity, sampled by the ray reflected about the normal?
   Flag: a spherical map used near the poles, where one texel smears the pole. (render)
5. **Diffuse from SH.** Ask: does `shade_ibl` in `crates/render/src/ibl.rs` take diffuse from `gi.rs::sh_irradiance` SH, divided by pi for non-metals?
   Flag: ambient added after tone map, or a metal keeps diffuse. (render)
6. **Split-sum specular.** Ask: does specular pick the prefiltered level by roughness plus `brdf_integrate` scale and bias over `BRDF_LUT_SIZE`?
   Flag: one mip sampled for all roughness, so rough and mirror match. (render)
7. **Bake inputs pinned.** Ask: is the bake cooked from `content/arena/hdri/*.hdr` with fixed roughness steps `IBL_CUBE_SIZE`, bytes x2 equal?
   Flag: a live HDRI swap or drifting roughness steps, so bakes differ run to run. (render)
8. **Probe and IBL agree.** Ask: does `baked_gi.rs::fighter_irradiance` agree with `eval_sh` on the same normal?
   Flag: the probe path and `FORWARD_IBL_INPUTS` path diverge, so fighters pop. (render)
9. **Live bind proves pixels.** Ask: are `ibl_prefilter`, `ibl_brdf` and `ibl_sh` bound in the forward group built by `Presenter::forward_bind` in `crates/render/src/present/gpu_present/presenter.rs`, with on vs off above the EG8 epsilon?
   Flag: the bake exists but `fs` still reads `AMBIENT_FLOOR`, so frames are identical. (render)

## Do / Don't
- Do: bind the bake and prove on-vs-off pixels differ. Don't: land a bake no live pass samples.

source: Gregory, Game Engine Architecture (2nd ed.), ch. 10.3.1
