# wgpu 30.0 code shapes

Condensed from sotrh/learn-wgpu at commit d90b300 (MIT, Copyright (c) 2020 Benjamin Hansen), reworded, short code kept. Each section names its source page (a path in the learn-wgpu repo).

## Device, queue and surface (docs/beginner/tutorial2-surface/README.md)

```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::PRIMARY, // Backends::GL on wasm32
    flags: Default::default(),
    memory_budget_thresholds: Default::default(),
    backend_options: Default::default(),
    display: None,
});
let surface = instance.create_surface(window.clone()).unwrap(); // window: Arc<Window>
let adapter = instance
    .request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
        apply_limit_buckets: true,
    })
    .await?;
let (device, queue) = adapter
    .request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::empty(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        required_limits: wgpu::Limits::default(), // downlevel_webgl2_defaults() on wasm32
        memory_hints: Default::default(),
        trace: wgpu::Trace::Off,
    })
    .await?;
```

- `request_adapter` and `request_device` are futures: native code uses `pollster::block_on`.
- Pick the surface format from `surface.get_capabilities(&adapter).formats` (first sRGB one); the shaders of the tutorial assume an sRGB surface, otherwise colours come out darker.
- `resize` sets `config.width` and `config.height` and calls `surface.configure(&device, &config)` only when both are above 0. WebGL caps a dimension at 2048.

## One frame (docs/beginner/tutorial2-surface/README.md, docs/news/30.0/readme.md)

```rust
let output = match self.surface.get_current_texture() {
    wgpu::CurrentSurfaceTexture::Success(t) => t,
    wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
    wgpu::CurrentSurfaceTexture::Timeout
    | wgpu::CurrentSurfaceTexture::Occluded
    | wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
    wgpu::CurrentSurfaceTexture::Outdated => {
        self.surface.configure(&self.device, &self.config);
        return Ok(());
    }
    wgpu::CurrentSurfaceTexture::Lost => anyhow::bail!("Lost device"),
};
let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") });
{
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Render Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view, resolve_target: None, depth_slice: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 }), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
    // set_pipeline, set_bind_group, set_vertex_buffer, draw ..
}
self.queue.submit(std::iter::once(encoder.finish()));
self.queue.present(output);
```

## Pipeline, vertices, indices (docs/beginner/tutorial3-pipeline/README.md, docs/beginner/tutorial4-buffer/README.md)

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex { position: [f32; 3], tex_coords: [f32; 2] }
impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &Self::ATTRIBS }
    }
}
let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("Render Pipeline Layout"), bind_group_layouts: &[Some(&texture_layout), Some(&camera_layout)], immediate_size: 0,
});
let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("Render Pipeline"), layout: Some(&layout),
    vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[Some(Vertex::desc())], compilation_options: wgpu::PipelineCompilationOptions::default() },
    fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })], compilation_options: wgpu::PipelineCompilationOptions::default() }),
    primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, strip_index_format: None, front_face: wgpu::FrontFace::Ccw, cull_mode: Some(wgpu::Face::Back), polygon_mode: wgpu::PolygonMode::Fill, unclipped_depth: false, conservative: false },
    depth_stencil: None,
    multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
    multiview_mask: None,
    cache: None,
});
```

- Triangles that face the camera have counter-clockwise vertices (`FrontFace::Ccw`); `Cw` flips it.
- A vertex attribute can be at most `Float32x4` (also `Uint32x4`, `Sint32x4`); a bigger value takes several attributes, as an instance matrix takes four at `shader_location` 5 to 8 with `VertexStepMode::Instance` (docs/beginner/tutorial7-instancing/README.md).
- The index buffer uses `BufferUsages::INDEX`; only one can be set; `draw` ignores it, so call `draw_indexed` with the index count.

## Texture, sampler, bind group (docs/beginner/tutorial5-textures/README.md)

```rust
let texture = device.create_texture(&wgpu::TextureDescriptor {
    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::Rgba8UnormSrgb,
    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    label: Some("diffuse_texture"), view_formats: &[],
});
let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
    address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, address_mode_w: wgpu::AddressMode::ClampToEdge,
    mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Nearest, mipmap_filter: wgpu::MipmapFilterMode::Nearest,
    ..Default::default()
});
```

- Layout entries (binding 0 and 1, visibility `ShaderStages::FRAGMENT`): `BindingType::Texture { multisampled: false, view_dimension: TextureViewDimension::D2, sample_type: TextureSampleType::Float { filterable: true } }` and `BindingType::Sampler(SamplerBindingType::Filtering)`; the sampler type must match `filterable`.
- Bind group entries: `BindingResource::TextureView(&view)` and `BindingResource::Sampler(&sampler)`.
- WGSL side: `@group(0) @binding(0) var t_diffuse: texture_2d<f32>;`, `@group(0) @binding(1) var s_diffuse: sampler;`, `textureSample(t_diffuse, s_diffuse, in.tex_coords)`.
- With `copy_buffer_to_texture` (instead of `write_texture`) `bytes_per_row` must be a multiple of 256.

## Camera uniform (docs/beginner/tutorial6-uniforms/README.md, docs/showcase/alignment/README.md)

```rust
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform { view_proj: [[f32; 4]; 4] }
// buffer: usage UNIFORM | COPY_DST, contents bytemuck::cast_slice(&[camera_uniform])
// layout entry: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, visibility VERTEX
// bind group entry: resource: camera_buffer.as_entire_binding()
// each frame: queue.write_buffer(&camera_buffer, 0, bytemuck::cast_slice(&[camera_uniform]));
```

WGSL: `@group(1) @binding(0) var<uniform> camera: CameraUniform;` for the second layout of the pipeline layout, bound with `render_pass.set_bind_group(1, &camera_bind_group, &[])`.

A light with two `vec3<f32>` members needs `_padding: u32` after each `[f32; 3]` (32 bytes). Alignment table: scalar 4/4, `vec2` 8/8, `vec3` 16/12, `vec4` 16/16; struct alignment is the largest member alignment.

## Depth (docs/beginner/tutorial8-depth/README.md)

`Texture::DEPTH_FORMAT = wgpu::TextureFormat::Depth32Float`; depth texture size equals the surface (width and height at least 1), usage `RENDER_ATTACHMENT | TEXTURE_BINDING`; a depth sampler that will be rendered needs `compare: Some(wgpu::CompareFunction::LessEqual)`. Pipeline: `depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: wgpu::StencilState::default(), bias: wgpu::DepthBiasState::default() })`. The pass gets the depth view in `depth_stencil_attachment`.

## Readback (docs/showcase/windowless/README.md, docs/showcase/gifs/README.md)

```rust
let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
let padded_bytes_per_row = unpadded_bytes_per_row + (align - unpadded_bytes_per_row % align) % align;
// copy_texture_to_buffer with bytes_per_row: padded_bytes_per_row, then:
buffer_slice.map_async(wgpu::MapMode::Read, move |result| { tx.send(result).unwrap(); });
device.poll(wgpu::PollType::wait_indefinitely())?;
let data = buffer_slice.get_mapped_range()?;
```

Crop each padded row to `unpadded_bytes_per_row` when you use the data, and call `unmap()` when done.
