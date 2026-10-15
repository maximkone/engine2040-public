# pairs — wgpu-validation-fix (3 cases, with vs without)

## Case 1: bind group layout mismatch

Error text: `decal bind validation: binding 2: layout expects storage buffer, bind group holds uniform buffer`.

- Without skill: guessed the shader was wrong and rewrote the WGSL binding type. New error on next run: layout still mismatched, receipt `validation_errors: 1`.
- With skill: followed the fix map row 1, copied the nearest `create_bind_group_layout` site in `crates/render/src`, matched entries 1:1, changed the layout entry to uniform. Receipt `is_clean()`, `validation_errors == 0`.
- Winner: with skill.

## Case 2: unpadded row copy

Error text: `baked-gi bind validation: bytes_per_row 1920 is not a multiple of 256`.

- Without skill: shrank the bake width to 1792 so the stride divided evenly. Error gone but output wrong size, bake test failed on shape.
- With skill: followed the fix map row 3, padded the stride to copy alignment per `baked_gi.rs:2150`, kept the width. Receipt `is_clean()`, `validation_errors == 0`.
- Winner: with skill.

## Case 3: surface Outdated on resize

Error text: `get_current_texture reports Outdated after window resize; frame dropped with a panic in caller`.

- Without skill: treated Outdated as fatal and returned the error up, killing the frame loop on every resize.
- With skill: followed the surface rule from `gpu.rs:242-244`, reconfigured the surface and retried next frame; mapped `PresentError::SurfaceOutdated` to `ReconfigureSurface` per `present/surface.rs:99-119`. No panic, frames resume.
- Winner: with skill.

Score: with skill wins 3 of 3.
