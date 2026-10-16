# Skill-on vs skill-off pairs: ibl-light-checks

Three review situations. "Off" is what a reviewer says from habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it prevents, and the file or pixel proof it asks for.

## P1: constant ambient hides the IBL bake (items 5, 9)

Situation: A builder bakes `ibl.rs` prefilter plus SH plus BRDF LUT from `content/arena/hdri/arena_day.hdr`, but `forward.wgsl::fs` still adds `AMBIENT_FLOOR`. The bake test passes and the builder calls LF2 done.

Off: "The bake is done and the LUT builds, so IBL is live. The constant is a reasonable ambient."

On: "Not live. Item 9: `ibl_prefilter`, `ibl_brdf` and `ibl_sh` must be bound in the forward group built by `Presenter::forward_bind` in `crates/render/src/present/gpu_present/presenter.rs`, with on-vs-off pixels above the EG8 epsilon. Item 5: diffuse must come from `shade_ibl` in `crates/render/src/ibl.rs` via `gi.rs::sh_irradiance`, not the constant. Proof: armor and sand regions differ on vs off, capture x2 equal, `arena per-pass` ibl above 0 ms. Search `node tools/books.mjs search \"image based lighting\"` and cite [geaen 10.3.1]."

What skill-less misses: it treats a passing bake as a live pass. It never checks the bind or the pixels, so identical frames pass as lighting.

## P2: mirrored normal maps invert (items 1, 2)

Situation: A builder adds tangent attribute with Repeat sampler plus normal and ORM sampling on sand and stone. Mirrored UV islands light from the wrong side, but the flat-quad test passes.

Off: "Tangents are in and the quad lights, so normal mapping works. Mirroring is a minor artifact."

On: "Two findings. Item 2: tangent.w must stay +-1 from `recompute_tangents` in `crates/assets/src/import_repair.rs` through `cooked.rs::tangents` to the shader; averaging w flips mirrored islands. Item 1: `forward.wgsl` must decode with bias, renormalize, and rebuild z when only xy is stored. Proof: one quad with mirrored UVs, light from +x, both halves face the light; `mesh_import.rs::fallback_tangents` covered."

What skill-less misses: handedness and decode. A flat unmirrored test hides both, so the bug ships to every mirrored mesh.

## P3: one mip for all roughness (items 6, 7)

Situation: A builder binds the prefilter cube but samples level 0 for every material, with roughness from the ORM map. Rough stone shines like a mirror, and the bake bytes differ between runs.

Off: "The prefilter is bound and roughness is read, so specular varies. Byte drift is noise."

On: "Two findings. Item 6: specular must pick the prefiltered level by roughness plus `brdf_integrate` scale and bias over `BRDF_LUT_SIZE`; one mip makes rough and mirror match. Item 7: the bake must cook from pinned `content/arena/hdri/*.hdr` with fixed `IBL_CUBE_SIZE` steps, bytes x2 equal. Proof: rough vs mirror regions differ, bake x2 identical, `split-sum shape` test in `ibl.rs` green."

What skill-less misses: the roughness-to-level rule and bake determinism. Without them every material shares one shine and reruns drift.
