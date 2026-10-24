---
name: wgpu-render
description: "Use when you write or fix wgpu 30.0 code in crates/render: instance, surface, pipelines, buffers, bind groups, uniform alignment, readback, a validation error, resize or device loss. Older wgpu names from memory will not compile."
---

# wgpu 30.0 in crates/render

The workspace pins `wgpu = "30.0.1"` and `winit = "0.30"` (`Cargo.toml`). The rules follow the learn-wgpu tutorial (sotrh/learn-wgpu @ d90b300, MIT). wgpu renames fields almost every release, so code from memory is usually for an older version and does not compile. Use these names. For one not listed, look it up on docs.rs. Never guess.

- Old form to new form by version: `references/api-changes.md`.
- Full code shapes (device, frame, pipeline, texture, uniform, depth, readback): `references/render-recipes.md`.
- Field lists, the acquire-outcome and error-recovery tables, resize and device loss: `references/api-notes.md`.
- Credit and licence: `references/sources.md`.
- Pass, math and lighting rules live in skills `render-pipeline-math` and `render-lighting`.

## Look up, do not recall

The pinned versions in `Cargo.lock` are `wgpu 30.0.1`, `winit 0.30.13` and `rapier3d 0.36.0`. A docs page for another version is the wrong answer.
Name the item first (struct, enum variant or method), then read its page: `Invoke-WebRequest https://docs.rs/wgpu/30.0.1/wgpu/struct.SurfaceConfiguration.html`, or WebFetch the same URL, and read the fields table.
Paste the signature you read into the edit, then write the call. A field or method that is not on the pinned page does not exist; never rename it by guessing a neighbour.
No network: grep `references/api-changes.md` and `references/render-recipes.md` first. If the name is in neither, say "unverified" and stop.

## The names that bite (full lists in `references/api-notes.md`)

- `Instance::new` takes an `InstanceDescriptor` by value, no `Default`; make new instances with `crate::backend::new_instance(backends)` so the strict-WebGPU flag applies.
- `request_device` takes one argument. `entry_point` is `Option<&str>`. `bind_group_layouts` are `&[Option<&..>]`. `immediate_size` replaced push constants.
- `create_surface(Arc<Window>)` gives `Surface<'static>`. Show with `queue.present(output)`; `SurfaceTexture::present()` is gone in 30.
- `get_current_texture()` returns `wgpu::CurrentSurfaceTexture` (an enum, not a `Result`): `Success`, `Suboptimal`, `Outdated`, `Lost`, `Timeout`, `Occluded`, `Validation`.
- `TexelCopyTextureInfo`, `TexelCopyBufferLayout`, `MipmapFilterMode`, `PollType::wait_indefinitely()`; `Maintain` is gone.
- Never configure width or height 0. Never drop the pipeline or the targets on Outdated, Lost or Validation.
- Pad uniform structs by hand; `vec3` is 16/12. A size mismatch panics at draw time.
- `copy_texture_to_buffer` rows pad to `COPY_BYTES_PER_ROW_ALIGNMENT` (256).
- No `map_async` plus blocking poll inside a frame; none at all on the web path (skill `web-wasm`).

## Validation errors

Every GPU submit runs inside `device.push_error_scope(wgpu::ErrorFilter::Validation)`, then submits, polls and pops the scope. A hit becomes `"<module> bind/submit validation: {err:?}"`. The receipt records `validation_errors: 1`. Green is `0` (`is_clean()`). Copy the shape at `crates/render/src/baked_gi.rs::GiSubmitter::submit`.

1. Copy the full scope text. Never paraphrase it away.
2. Name the pass label and the first call after the scope push (bind set, dispatch, copy, submit).
3. Fix the first error only, then rerun. The same text twice: stop and write a `docs/BLOCKERS.md` row.

Text-to-cause table and the `PresentError` -> `RecoveryAction` map: `references/api-notes.md`. Surface errors are not validation errors; match `get_current_texture` like `crates/render/src/gpu.rs::GpuState::render`.

## Proof commands

A GPU test with no adapter returns early and passes, so a green run can test nothing.

- No GPU needed: `node tools/gh-check.mjs cargo test -p engine-render --lib <filter>` for `every_error_names_its_recovery`, `surface_events_map_to_recovery_actions`, `size_class_covers_minimize_and_reject`. For the heal check use `verified_recovery_restores_full_quality_and_resets` (in `crates/render/src/heal.rs`).
- With a GPU, fail on a missing adapter: `$env:E2040_REQUIRE_GPU = "1"; node tools/gh-check.mjs cargo test --locked -p engine-render --all-features --lib resize_and_minimize_survive_with_last_good`. Repeat with `device_loss_fails_loudly_until_rebuilt`. `crates/render/src/test_gpu.rs::require_or_skip` reads the variable.
- Strict WebGPU: `E2040_STRICT_WEBGPU=1` opens every native instance with `InstanceFlags::STRICT_WEBGPU_COMPLIANCE` (WebGPU features and limits only, `LoadOp::DontCare` refused), so a pass a browser would reject fails on this PC. Proof: `node tools/gh-check.mjs cargo test -p engine-render --features window --lib strict_webgpu`.
- Which adapter drew: `Presenter::backend_selection().line()` is one stable line, `backend: source=.. requested=.. selected=".." api=.. kind=.. fallback=no|yes reason=".." remedy=".."` (`crate::adapter::BackendSelection`; the editor snapshot envelope carries it). A fallback is a software adapter or a surface that hides the best adapter. `E2040_REQUIRE_GPU=1` makes a fallback a failed bring-up, so a proof job cannot pass on the wrong adapter. Proof: `node tools/gh-check.mjs cargo test -p engine-render --features window --lib presenter_pick::`.
- Frame against a golden: `pass_truth::frame_tolerance::frames_match`, mean channel error at most 1.5 levels and at most 0.5 percent of pixels off by more than 12. Never write a new local threshold (`references/api-notes.md`).
- Module then crate: `node tools/gh-check.mjs cargo test -p engine-render --lib <filter>`, then `node tools/gh-check.mjs cargo test -p engine-render --lib`. Green means `is_clean()` with `validation_errors == 0`.
- `node tools/proof.mjs`: the engine-render GPU skip count it prints must be 0 on a GPU machine.
- Heavy test lines go to GitHub: skill `github-check`. GPU and window lines run on the PC by themselves.

Sources: sotrh/learn-wgpu @ d90b300 (MIT), see `references/sources.md`.
