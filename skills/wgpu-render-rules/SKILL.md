---
name: wgpu-render-rules
description: "Use when writing or fixing Rust code for wgpu 30.0 (the learn-wgpu tutorial version): instance, surface and frame loop, pipelines, vertex and index buffers, textures, bind groups, uniform alignment, buffer readback; an older wgpu name or signature from memory will not compile"
version: 0.1.0
author: skillworks
license: "MIT (derived from sotrh/learn-wgpu @ d90b300)"
---

# wgpu 30.0 render rules

Pinned to wgpu 30.0 and winit 0.30 as taught by sotrh/learn-wgpu at commit d90b300 (MIT). wgpu renames fields and changes signatures almost every release, so code from memory is usually for an older version and does not compile. Use these names; for one not listed, look it up on docs.rs instead of guessing. Each rule ends with its source page, a path in the learn-wgpu repo. Changes by version: `references/api-changes.md`. Code shapes: `references/render-recipes.md`. Credit and licence: `references/sources.md`.

## Setup and frame

- Cargo: `wgpu = "30.0"`, `winit = "0.30"`, `pollster = "0.3"`, `env_logger = "0.10"`. Call `env_logger::init()`: wgpu reports the real error only through the log crate, so without it a failure looks silent. [docs/beginner/tutorial1-window/README.md]
- Instance: `wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, flags: Default::default(), memory_budget_thresholds: Default::default(), backend_options: Default::default(), display: None })`. It has no `Default` any more, and it is passed by value. The helpers `InstanceDescriptor::new_without_display_handle()` and `new_with_display_handle(..)` fill `display`. [docs/news/29.0/readme.md] [docs/beginner/tutorial2-surface/README.md]
- `instance.create_surface(window.clone())` with an `Arc<Window>` gives a `Surface<'static>`. Then `request_adapter(&RequestAdapterOptions { power_preference, compatible_surface: Some(&surface), force_fallback_adapter: false, apply_limit_buckets: true }).await?`. [docs/beginner/tutorial2-surface/README.md]
- Device and queue: one argument, `adapter.request_device(&wgpu::DeviceDescriptor { label, required_features, experimental_features: wgpu::ExperimentalFeatures::disabled(), required_limits, memory_hints, trace: wgpu::Trace::Off }).await?`. [docs/news/25.0/readme.md] [docs/beginner/tutorial2-surface/README.md]
- Surface config: `surface.get_capabilities(&adapter)`, prefer an sRGB format, then `SurfaceConfiguration { usage: RENDER_ATTACHMENT, format, width, height, present_mode, alpha_mode, view_formats: vec![], desired_maximum_frame_latency: 2, color_space: wgpu::SurfaceColorSpace::Auto }`. Never configure width or height 0. [docs/beginner/tutorial2-surface/README.md]
- `surface.get_current_texture()` returns the enum `wgpu::CurrentSurfaceTexture`, not a `Result`. `Success(t)` and `Suboptimal(t)` render; `Timeout`, `Occluded` and `Validation` skip the frame; `Outdated` calls `surface.configure(&device, &config)` and skips; `Lost` is fatal. [docs/news/29.0/readme.md] [docs/beginner/tutorial2-surface/README.md]
- Show the frame with `queue.present(output)` after `queue.submit(..)`. `SurfaceTexture::present()` was removed in 30. [docs/news/30.0/readme.md]
- `RenderPassDescriptor` has `label`, `color_attachments: &[Some(RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops })]`, `depth_stencil_attachment`, `occlusion_query_set: None`, `timestamp_writes: None`, `multiview_mask: None`. [docs/beginner/tutorial2-surface/README.md] [docs/beginner/tutorial3-pipeline/README.md]

## Pipelines

- `create_pipeline_layout(&PipelineLayoutDescriptor { label, bind_group_layouts: &[Some(&a), Some(&b)], immediate_size: 0 })`: each layout is an `Option`, and `immediate_size` replaced push constants. [docs/news/29.0/readme.md] [docs/news/28.0/readme.md]
- `RenderPipelineDescriptor` sets `vertex: VertexState { module, entry_point: Some("vs_main"), buffers, compilation_options: Default::default() }`, `fragment: Some(FragmentState { .. targets: &[Some(ColorTargetState { format, blend: Some(BlendState::REPLACE), write_mask: ColorWrites::ALL })] .. })`, `primitive`, `depth_stencil`, `multisample`, `multiview_mask: None`, `cache: None`. Entry points are `Option`; the vertex and fragment functions need different names. [docs/news/24.0/readme.md] [docs/news/28.0/readme.md] [docs/beginner/tutorial3-pipeline/README.md]
- `buffers` is `&[Option<VertexBufferLayout>]`; `None` keeps a slot empty. A layout has `array_stride`, `step_mode`, and `attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2]`. Bind with `set_vertex_buffer(slot, buffer.slice(..))`. [docs/beginner/tutorial4-buffer/README.md] [docs/news/30.0/readme.md]
- Depth: a `Depth32Float` texture with usage `RENDER_ATTACHMENT | TEXTURE_BINDING`; `DepthStencilState { format, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: StencilState::default(), bias: DepthBiasState::default() }`. Rebuild the depth texture on resize. [docs/beginner/tutorial8-depth/README.md]

## Buffers, textures, bind groups

- `device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label, contents: bytemuck::cast_slice(DATA), usage })` needs `use wgpu::util::DeviceExt;` and `#[repr(C)]` types that derive `bytemuck::Pod` and `Zeroable`. Nine `u16` indices are 18 bytes and the buffer is padded to 20 (4-byte alignment). Use `set_index_buffer(slice, IndexFormat::Uint16)` with `draw_indexed(0..num_indices, 0, 0..instances)`. [docs/beginner/tutorial4-buffer/README.md]
- Pixels go in with `queue.write_texture(TexelCopyTextureInfo { texture, mip_level, origin, aspect }, &rgba, TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) }, size)`; format `Rgba8UnormSrgb`, usage `TEXTURE_BINDING | COPY_DST`. A sampler's `mipmap_filter` is a `MipmapFilterMode`, not a `FilterMode`. [docs/beginner/tutorial5-textures/README.md] [docs/news/0.16/readme.md] [docs/news/28.0/readme.md]
- Layout entries: a texture is `BindingType::Texture { multisampled: false, view_dimension: D2, sample_type: Float { filterable: true } }` and its sampler is `Sampler(SamplerBindingType::Filtering)`; a uniform is `Buffer { ty: Uniform, has_dynamic_offset: false, min_binding_size: None }`. Update a uniform buffer (usage `UNIFORM | COPY_DST`) with `queue.write_buffer`. [docs/beginner/tutorial5-textures/README.md] [docs/beginner/tutorial6-uniforms/README.md]
- The position of a layout in `bind_group_layouts` is both the WGSL `@group(n)` and the first argument of `render_pass.set_bind_group(n, &group, &[])`. [docs/beginner/tutorial6-uniforms/README.md]
- WGSL alignment in uniform and storage buffers: scalar 4 and 4, `vec2` 8 and 8, `vec3` align 16 and size 12, `vec4` 16 and 16; a struct aligns to its largest member. Pad Rust structs by hand (a `_padding: u32` after each `[f32; 3]`); `#[repr(C, align(16))]` fails with bytemuck `Pod` because of implicit padding. [docs/showcase/alignment/README.md]
- cgmath matrices need `OPENGL_TO_WGPU_MATRIX` because wgpu clip z runs 0 to 1; multiply `camera.view_proj * vec4<f32>(position, 1.0)`. Texture coordinates run down from the top left, so flip y. [docs/beginner/tutorial6-uniforms/README.md] [docs/beginner/tutorial5-textures/README.md]

## Readback and compute

- `copy_texture_to_buffer` needs `bytes_per_row` to be a multiple of `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256): pad each row, crop it when you read it. [docs/showcase/gifs/README.md]
- Read a buffer with `slice.map_async(wgpu::MapMode::Read, move |result| ..)` (a callback), then `device.poll(wgpu::PollType::wait_indefinitely())?`, then `get_mapped_range()`. `Maintain` is gone, and `PollType::Wait` now carries fields. A `MAP_READ` buffer can only have `COPY_DST`, so copy a storage buffer into a temporary one first. [docs/news/25.0/readme.md] [docs/news/27.0/readme.md] [docs/compute/introduction/readme.md]
- Compute: `ComputePipelineDescriptor { label, layout: None, module, entry_point: None, compilation_options: Default::default(), cache: Default::default() }`; `layout: None` derives the bind groups from the shader and `entry_point: None` picks its only `@compute` function. Run with `begin_compute_pass(&Default::default())` and `dispatch_workgroups(items.div_ceil(64), 1, 1)` for `@workgroup_size(64)`. [docs/compute/introduction/readme.md]
