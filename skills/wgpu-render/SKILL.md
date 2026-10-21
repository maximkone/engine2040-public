---
name: wgpu-render
description: "Use when you write or fix wgpu 30.0 code in crates/render: instance, surface, pipelines, buffers, bind groups, uniform alignment, readback, a validation error, resize or device loss. Older wgpu names from memory will not compile."
---

# wgpu 30.0 in crates/render

The workspace pins `wgpu = "30.0.1"` and `winit = "0.30"` (`Cargo.toml`). The rules follow the learn-wgpu tutorial (sotrh/learn-wgpu @ d90b300, MIT). wgpu renames fields almost every release, so code from memory is usually for an older version and does not compile. Use these names. For one not listed, look it up on docs.rs. Never guess.

- Old form to new form by version: `references/api-changes.md`.
- Full code shapes (device, frame, pipeline, texture, uniform, depth, readback): `references/render-recipes.md`.
- Credit and licence: `references/sources.md`.
- Pass, math and lighting rules live in skills `render-pipeline-math` and `render-lighting`.

## 1. Setup and frame

- Cargo: `wgpu`, `winit = "0.30"`, `pollster = "0.3"`, `env_logger = "0.10"`. Call `env_logger::init()`. wgpu reports the real error only through the log crate, so without it a failure looks silent.
- Instance: `wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, flags: Default::default(), memory_budget_thresholds: Default::default(), backend_options: Default::default(), display: None })`. It has no `Default` any more and is passed by value. `InstanceDescriptor::new_without_display_handle()` and `new_with_display_handle(..)` fill `display`.
- `instance.create_surface(window.clone())` with an `Arc<Window>` gives a `Surface<'static>`.
- Adapter: `request_adapter(&RequestAdapterOptions { power_preference, compatible_surface: Some(&surface), force_fallback_adapter: false, apply_limit_buckets: true }).await?`.
- Device and queue take one argument: `adapter.request_device(&wgpu::DeviceDescriptor { label, required_features, experimental_features: wgpu::ExperimentalFeatures::disabled(), required_limits, memory_hints, trace: wgpu::Trace::Off }).await?`.
- Surface config: `surface.get_capabilities(&adapter)`, prefer an sRGB format, then `SurfaceConfiguration { usage: RENDER_ATTACHMENT, format, width, height, present_mode, alpha_mode, view_formats: vec![], desired_maximum_frame_latency: 2, color_space: wgpu::SurfaceColorSpace::Auto }`. Never configure width or height 0.
- `RenderPassDescriptor`: `label`, `color_attachments: &[Some(RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops })]`, `depth_stencil_attachment`, `occlusion_query_set: None`, `timestamp_writes: None`, `multiview_mask: None`.
- Show the frame with `queue.present(output)` after `queue.submit(..)`. `SurfaceTexture::present()` was removed in 30.

### Acquire outcomes (stated once)

`surface.get_current_texture()` returns the enum `wgpu::CurrentSurfaceTexture`, not a `Result`.

| Outcome | Tutorial loop | Engine presenter (`present/surface.rs::surface_action`) |
|---|---|---|
| `Success` | render | `Render` |
| `Suboptimal` | render | `RenderThenReconfigure` (present, then fix) |
| `Outdated` | configure, skip | `ReconfigureThenRetry` |
| `Lost` | fatal | `ReconfigureThenRetry`; the fault code is `RENDER_DEVICE_LOST` with backoff |
| `Timeout` | skip | `SkipRetryNext` |
| `Occluded` | skip | `SkipKeepLastGood` |
| `Validation` | skip | `AbortKeepLastGood`: no present, last-good kept |

The engine never drops the pipeline or the targets on Outdated, Lost or Validation.

## 2. Pipelines

- `create_pipeline_layout(&PipelineLayoutDescriptor { label, bind_group_layouts: &[Some(&a), Some(&b)], immediate_size: 0 })`. Each layout is an `Option`. `immediate_size` replaced push constants.
- `RenderPipelineDescriptor`: `vertex: VertexState { module, entry_point: Some("vs_main"), buffers, compilation_options: Default::default() }`, `fragment: Some(FragmentState { .. targets: &[Some(ColorTargetState { format, blend: Some(BlendState::REPLACE), write_mask: ColorWrites::ALL })] .. })`, `primitive`, `depth_stencil`, `multisample`, `multiview_mask: None`, `cache: None`. Entry points are `Option`. Vertex and fragment functions need different names.
- `buffers` is `&[Option<VertexBufferLayout>]`. `None` keeps a slot empty. A layout has `array_stride`, `step_mode` and `attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2]`. Bind with `set_vertex_buffer(slot, buffer.slice(..))`.
- Depth: a `Depth32Float` texture with usage `RENDER_ATTACHMENT | TEXTURE_BINDING`. `DepthStencilState { format, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: StencilState::default(), bias: DepthBiasState::default() }`. Rebuild the depth texture on resize.

## 3. Buffers, textures, bind groups

- `device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label, contents: bytemuck::cast_slice(DATA), usage })` needs `use wgpu::util::DeviceExt;` and `#[repr(C)]` types that derive `bytemuck::Pod` and `Zeroable`.
- Nine `u16` indices are 18 bytes. The buffer pads to 20 (4-byte alignment).
- Index draw: `set_index_buffer(slice, IndexFormat::Uint16)` then `draw_indexed(0..num_indices, 0, 0..instances)`.
- Pixels go in with `queue.write_texture(TexelCopyTextureInfo { texture, mip_level, origin, aspect }, &rgba, TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) }, size)`. Format `Rgba8UnormSrgb`, usage `TEXTURE_BINDING | COPY_DST`.
- A sampler's `mipmap_filter` is a `MipmapFilterMode`, not a `FilterMode`.
- Layout entries: a texture is `BindingType::Texture { multisampled: false, view_dimension: D2, sample_type: Float { filterable: true } }`, its sampler is `Sampler(SamplerBindingType::Filtering)`. A uniform is `Buffer { ty: Uniform, has_dynamic_offset: false, min_binding_size: None }`. Update a uniform buffer (usage `UNIFORM | COPY_DST`) with `queue.write_buffer`.
- The position of a layout in `bind_group_layouts` is the WGSL `@group(n)` and the first argument of `render_pass.set_bind_group(n, &group, &[])`.
- WGSL alignment in uniform and storage buffers (align and size): scalar 4 and 4, `vec2` 8 and 8, `vec3` 16 and 12, `vec4` 16 and 16. A struct aligns to its largest member.
- Pad Rust structs by hand (`_padding: u32` after each `[f32; 3]`). `#[repr(C, align(16))]` fails with bytemuck `Pod` because of implicit padding. A size mismatch panics at draw time ("Buffer is bound with size 28 where the shader expects 32").
- cgmath matrices need `OPENGL_TO_WGPU_MATRIX` because wgpu clip z runs 0 to 1. glam feeds wgpu directly (see skill `render-pipeline-math` for the convention). Texture coordinates run down from the top left, so flip y.

## 4. Readback and compute

- `copy_texture_to_buffer` needs `bytes_per_row` to be a multiple of `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256). Pad each row, crop it when you read.
- Read a buffer with `slice.map_async(wgpu::MapMode::Read, move |result| ..)` (a callback), then `device.poll(wgpu::PollType::wait_indefinitely())?`, then `get_mapped_range()`. `Maintain` is gone. `PollType::Wait` now carries fields.
- A `MAP_READ` buffer can only have `COPY_DST`. Copy a storage buffer into a temporary one first.
- No `map_async` plus blocking poll inside a frame. On the web path no `pollster::block_on` or `device.poll(wait)` at all (see skills `render-pipeline-math` rule 2 and `web-wasm`).
- Compute: `ComputePipelineDescriptor { label, layout: None, module, entry_point: None, compilation_options: Default::default(), cache: Default::default() }`. `layout: None` derives the bind groups from the shader. `entry_point: None` picks the only `@compute` function.
- Run with `begin_compute_pass(&Default::default())` and `dispatch_workgroups(items.div_ceil(64), 1, 1)` for `@workgroup_size(64)`.

## 5. Validation errors in crates/render

Every GPU submit runs inside `device.push_error_scope(wgpu::ErrorFilter::Validation)`, then submits, polls and pops the scope. A hit becomes `"<module> bind/submit validation: {err:?}"`. The receipt records `validation_errors: 1`. Green is `0` (`is_clean()`). Copy the shape at `crates/render/src/baked_gi.rs::GiSubmitter::submit`.

Read the error first:

1. Copy the full scope text. Never paraphrase it away.
2. Name the pass label and the first call after the scope push (bind set, dispatch, copy, submit).
3. Fix the first error only, then rerun. The same text twice: stop and write a `docs/BLOCKERS.md` row.

| Text says | Cause | Fix |
|---|---|---|
| bind group incompatible, missing binding | layout vs shader mismatch | copy the nearest `create_bind_group_layout` site in `crates/render/src`; match entries 1:1 |
| usage does not include | wrong `BufferUsages` or `TextureUsages` | add the used flag at creation (`crates/render/src/accel.rs`, `crates/render/src/gpu.rs::GpuState`) |
| bytes_per_row, stride, alignment | unpadded row copy | pad the stride to copy alignment (`crates/render/src/baked_gi.rs::gi_lightmap_upload_roundtrips` shows the padded copy) |
| mip_level_count, view dimension | bad mip chain or view desc | set `mip_level_count: levels` at creation (`crates/render/src/ibl.rs::prefilter_cube_desc`) |
| buffer size too small | stale sized buffer | resize in `crates/render/src/baked_gi.rs::GiSubmitter::ensure_buffers` on shape change |

Surface errors are not validation errors. Match `get_current_texture` like `crates/render/src/gpu.rs::GpuState::render`. Map a `PresentError` with `crates/render/src/present/surface.rs::PresentError::recovery`:

| `PresentError` | `RecoveryAction` |
|---|---|
| `Validation`, `NoAdapter`, `SurfaceOccluded`, `OutOfMemory` | `KeepLastGood` |
| `Device`, `DeviceLost` | `ReacquireDevice` (drop the presenter, build a new one) |
| `SurfaceLost`, `SurfaceOutdated` | `ReconfigureSurface` |
| `SurfaceTimeout` | `SkipFrame` |
| `Readback` | `RetryFrame` |
| `InvalidSize`, `InvalidDraw`, `InvalidMesh`, `InvalidTexture`, `MissingTexture`, `NoSurface`, `BadDigest` | `FixCallerInput` |

On the web the browser answers error scopes late. `crates/render/src/present/gpu_present/device_init.rs::Presenter::deferred_validation_errors` counts them (skill `web-wasm`).

## 6. Surface resize and device loss (crates/render/src/present)

The presenter keeps the last good pipeline and targets through resize, minimize, surface loss and device loss, then recovers.

1. Zero size suspends and never configures the surface. See `crates/render/src/present/surface.rs::SizeClass::classify`, then `crates/render/src/present/gpu_present/presenter.rs::Presenter::resize`, which calls `SwapState::recreate` in `crates/render/src/v1.rs`.
2. Map each acquire outcome to one action (table in section 1). See `crates/render/src/present/gpu_present/submit.rs::Presenter::render_surface`. Outdated and Lost reconfigure and return an error for a retry.
3. Reconfigure with the stored last-good config, never a new size: `crates/render/src/present/gpu_present/submit.rs::Presenter::reconfigure_surface`.
4. Device loss is a flag the caller sets. Frames then fail with `DeviceLost`. See `Presenter::mark_device_lost` and `Presenter::check_device` in `presenter.rs`, then `crates/render/src/heal.rs::RenderHeal::report_device_lost` for backoff. Grep finds no wgpu device-lost callback in `crates/render/src`, so nothing sets the flag by itself.
5. Map each error once. `PresentError::recovery` says what the caller does. `PresentError::fault_code` says what the supervisor reports. `crates/render/src/present/tests_unit.rs::every_error_names_its_recovery` checks it.
6. Release the live residency caches on every resize, surface loss and device loss. `crates/render/src/present/cache.rs::CpuCache::evict` sits behind `Presenter::evict_texture`. Grep finds no caller outside its own test, so this hook is not wired yet. The belt that first owned this release (`crates/render/src/uploads.rs::StagingBelt::invalidate_all`) is cut; recover it with `git log --follow -- crates/render/src/uploads.rs`.

Do not configure the surface or the targets at zero size. Do not drop the pipeline or the targets on Outdated, Lost or Validation. Do not count a skipped GPU test as green.

## 7. Proof commands

A GPU test with no adapter returns early and passes, so a green run can test nothing.

- No GPU needed: `cargo test -p engine-render --lib every_error_names_its_recovery`, `surface_events_map_to_recovery_actions`, `size_class_covers_minimize_and_reject`. For the heal check use `verified_recovery_restores_full_quality_and_resets` (in `crates/render/src/heal.rs`).
- With a GPU, fail on a missing adapter: `$env:E2040_REQUIRE_GPU = "1"; cargo test --locked -p engine-render --all-features --lib resize_and_minimize_survive_with_last_good`. Repeat with `device_loss_fails_loudly_until_rebuilt`. `crates/render/src/test_gpu.rs::require_or_skip` reads the variable.
- Strict WebGPU: `E2040_STRICT_WEBGPU=1` opens every native instance with `InstanceFlags::STRICT_WEBGPU_COMPLIANCE` (WebGPU features and limits only, `LoadOp::DontCare` refused), so a pass a browser would reject fails on this PC. Make new instances with `crate::backend::new_instance(backends)`, never `Instance::new`. Proof: `cargo test -p engine-render --features window --lib strict_webgpu`.
- Which adapter drew: `Presenter::backend_selection().line()` is one stable line, `backend: source=.. requested=.. selected=".." api=.. kind=.. fallback=no|yes reason=".." remedy=".."` (`crate::adapter::BackendSelection`; the editor snapshot envelope carries it). A fallback is a software adapter or a surface that hides the best adapter. `E2040_REQUIRE_GPU=1` (the GPU tests' switch) makes a fallback a failed bring-up, so a proof job cannot pass on the wrong adapter. Proof: `cargo test -p engine-render --features window --lib presenter_pick::`.
- Does a frame match a golden: `pass_truth::frame_tolerance::frames_match(golden, frame, &FrameTolerance::GOLDEN)`, mean channel error at most 1.5 levels AND at most 0.5 percent of pixels off by more than 12; the verdict holds both numbers. Never write a new local threshold. Its tests nudge a post gain by 0.02 and must fail. A parity test with a tighter level keeps it: `FrameTolerance { over_level: 3, ..FrameTolerance::GOLDEN }`.
- Module then crate: `cargo test -p engine-render --lib <filter>`, then `cargo test -p engine-render --lib`. Green means `is_clean()` with `validation_errors == 0`.
- `node tools/proof.mjs`: the engine-render GPU skip count it prints must be 0 on a GPU machine.
- Heavy test lines go to GitHub: skill `github-check`. GPU and window lines run on the PC by themselves.

Sources: sotrh/learn-wgpu @ d90b300 (MIT), see `references/sources.md`.
