---
name: wgpu-validation-fix
description: Use when a crates/render GPU submit reports a wgpu validation error, a receipt shows validation_errors above 0, or a present frame returns a PresentError.
---

# wgpu validation fix in crates/render

Every GPU submit here runs inside `device.push_error_scope(wgpu::ErrorFilter::Validation)`, then submits, polls, and pops the scope. A hit becomes `"<module> bind/submit validation: {err:?}"` and the receipt records `validation_errors: 1` (green is `0`, see `is_clean()`). Copy the shape at `crates/render/src/baked_gi.rs::GiSubmitter::submit`.

## Read the error first

1. Copy the full scope text. Never paraphrase it away.
2. Name the pass label and the first call after the scope push (bind set, dispatch, copy, submit).
3. Fix the first error only, then re-run. Same text twice: stop and report BLOCKED.

## Fix map

| Text says | Cause | Fix |
|---|---|---|
| bind group incompatible, missing binding | layout vs shader mismatch | copy nearest `create_bind_group_layout` site in `crates/render/src`; match entries 1:1 |
| usage does not include | wrong `BufferUsages` or `TextureUsages` | add the used flag at creation (`accel.rs`, `crates/render/src/gpu.rs::GpuState::new`) |
| bytes_per_row, stride, alignment | unpadded row copy | pad stride to copy alignment (the test `crates/render/src/baked_gi.rs::gi_lightmap_upload_roundtrips` shows the padded copy) |
| mip_level_count, view dimension | bad mip chain or view desc | set `mip_level_count: levels` at creation (`crates/render/src/ibl.rs::prefilter_cube_desc`) |
| buffer size too small | stale sized buffer | resize in `ensure_buffers` on shape change (`crates/render/src/baked_gi.rs::GiSubmitter::ensure_buffers`) |

## Surface errors are not validation errors

Match `get_current_texture` like `crates/render/src/gpu.rs::GpuState::render`: `Outdated|Lost` reconfigures and the frame is skipped; `Timeout|Occluded|Validation` skips the frame. Map `PresentError` with `recovery()` in `crates/render/src/present/surface.rs::PresentError::recovery`: Validation keeps last-good, Lost/Outdated reconfigures, Timeout skips, Device re-acquires, Invalid* fixes caller input.

## Prove it in pwsh

Run `cargo test -p engine-render --lib <filter>` for the module, then `cargo test -p engine-render --lib` for the crate. Green means the receipt `is_clean()` with `validation_errors == 0`.
