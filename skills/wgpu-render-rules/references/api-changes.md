# wgpu changes by version (as the learn-wgpu news pages record them)

Condensed from sotrh/learn-wgpu at commit d90b300 (MIT, Copyright (c) 2020 Benjamin Hansen), reworded, short code lines kept. Use it to repair code written for an older wgpu: find the old form in the left column, write the form in the right column. The source page of each row is `docs/news/<version>/readme.md` unless a path is given.

| Version | Old form | Form to write now |
|---|---|---|
| 0.12 | uniform struct not aligned to the shader (it used to run) | pad it; a mismatch panics at draw time: "Buffer is bound with size 28 where the shader expects 32" (docs/news/0.12/readme.md) |
| 0.12 | `[[block]] struct Camera { .. }` in WGSL | `struct Camera { .. }` (no attribute) |
| 0.13 | `buffer_slice.map_async(MapMode::Read).await` | `map_async(MapMode::Read, move |result| { .. })` with a callback (for example send on a channel), then `device.poll(..)` |
| 0.14 | `SurfaceConfiguration` without `alpha_mode` | add `alpha_mode: wgpu::CompositeAlphaMode::Auto` (or the first of `surface_caps.alpha_modes`) |
| 0.15 | `surface.get_supported_formats(&adapter)` | `surface.get_capabilities(&adapter)`; use `.formats`, `.present_modes`, `.alpha_modes` |
| 0.15 | no `view_formats` | `view_formats: &[]` on `TextureDescriptor`, `view_formats: vec![]` on `SurfaceConfiguration` |
| 0.16 | `NonZeroU32::new(4 * width)` for `bytes_per_row` | `bytes_per_row: Some(4 * width)` and `rows_per_image: Some(height)` |
| 0.18 | `RenderPassDescriptor` with 3 fields | add `occlusion_query_set: None` and `timestamp_writes: None`; `ComputePassDescriptor` gets `timestamp_writes` |
| 22 | shader stage without options; no pipeline cache | `compilation_options: Default::default()` on every stage, `cache: None` on pipelines, `memory_hints: Default::default()` on `DeviceDescriptor` |
| 24 | `entry_point: "vs_main"` | `entry_point: Some("vs_main")`; with exactly one `@vertex` (or `@fragment`) function the entry point can be inferred; same for compute pipelines |
| 25 | `adapter.request_device(&desc, None).await` | `request_device(&desc).await?` with `trace: wgpu::Trace::Off` inside `DeviceDescriptor` |
| 25 | `device.poll(wgpu::Maintain::Wait)` | `device.poll(wgpu::PollType::wait_indefinitely())?` |
| 27 | `DeviceDescriptor` without experimental flag | add `experimental_features: wgpu::ExperimentalFeatures::disabled()` |
| 27 | unit variant `PollType::Wait` | `PollType::Wait` has fields `submission_index` and `timeout` (the submission index comes from `Queue::submit`); use `PollType::wait_indefinitely()` |
| 28 | `mipmap_filter: wgpu::FilterMode::Nearest` | `mipmap_filter: wgpu::MipmapFilterMode::Nearest` |
| 28 | `multiview: None` on `RenderPipelineDescriptor` | `multiview_mask: None` (it is now a bit mask of layers) |
| 28 | push constants (`push_constant_ranges: &[]`) | `immediate_size: 0` on `PipelineLayoutDescriptor` |
| 29 | `bind_group_layouts: &[&layout]` | `bind_group_layouts: &[Some(&layout)]`; the `Option` lets a pipeline keep group numbers stable (camera always `@group(0)`, material always `@group(1)`) even when its shader does not use one of them |
| 29 | `match surface.get_current_texture() { Ok(t) => .., Err(SurfaceError::Outdated) => .. }` | `match` on the enum `wgpu::CurrentSurfaceTexture`: `Success(t)`, `Suboptimal(t)`, `Timeout`, `Occluded`, `Validation`, `Outdated`, `Lost` |
| 29 | `InstanceDescriptor { backends, ..Default::default() }` | no `Default`; set `display` (`None` or a handle): `flags`, `memory_budget_thresholds`, `backend_options`, `display`; or call `InstanceDescriptor::new_without_display_handle()`, `new_with_display_handle(..)` and the two `_from_env` variants |
| 30 | `SurfaceTexture::present()` called on the acquired frame | `queue.present(output)` after `queue.submit(..)` (`SurfaceTexture::present()` was removed) |
| 30 | `buffers: &[Vertex::desc(), Instance::desc()]` | `buffers: &[Some(Vertex::desc()), Some(Instance::desc())]`; `None` keeps a slot empty so a buffer can stay in the same slot across pipelines |
| 30 | adapter and surface without limit bucketing or colour space | the adapter request takes a limit-bucketing flag and the surface config takes `color_space: wgpu::SurfaceColorSpace::Auto` |

Spellings that the pinned pages disagree on. The 30.0 news page names the adapter flag `apply_limit_bucketing` and the colour option `wgpu::ColorSpaces::Auto`; the code in docs/beginner/tutorial2-surface/README.md writes `apply_limit_buckets: true` in `RequestAdapterOptions` and `color_space: wgpu::SurfaceColorSpace::Auto` in `SurfaceConfiguration`. Trust the tutorial code, and confirm either spelling on docs.rs before relying on it.

Pages that still show older numbers. `docs/beginner/tutorial5-textures/README.md` and a few showcase pages print `wgpu = "28.0"` in their Cargo snippets, and some prose still talks of `request_adapter` returning `None`; the code of tutorials 1 to 3 (`wgpu = "30.0"`, `.await?`) is the reference.
