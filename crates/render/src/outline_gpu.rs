//! outline_gpu: the device half of SW150, the passes `shaders/outline_mask.wgsl`
//! and `shaders/outline_edge.wgsl` compile into. The CPU twin in [`super`]
//! (mask, Sobel, two blurs, composite, the colours and caps) is the reference;
//! this module owns the textures, pipelines and encoders that run it on the GPU.
//!
//! Shape (three.js `OutlinePass.render`): one instanced mask draw per selected
//! target into a half-res mask over the caller's depth prefill, the edge pass
//! over that mask, thickness blur X then Y, glow blur X then Y at the wide
//! kernel, and one composite into [`OutlineGpu::out`]. Every fullscreen pass
//! binds the same four textures and picks what it reads by entry point, so one
//! layout serves all of them; the ping-pong order is
//! `mask -> a -> b -> a (thickness) -> b -> c (glow) -> out`.
//!
//! The mask pass draws each selected target's bounding box as a unit cube
//! scaled and moved by the target; feeding the skinned fighter mesh to it is the
//! next landing, and the CPU oracle already models a target as its sphere.
//!
//! Units: metres, texels, linear colour. Determinism: fixed pass order, no
//! clock, so the same camera, prefill and selection write the same pixels.

use wgpu::{
    BufferUsages, ColorTargetState, ColorWrites, FragmentState, LoadOp, MultisampleState,
    Operations, PipelineCompilationOptions, PrimitiveState, RenderPassColorAttachment,
    RenderPassDescriptor, ShaderStages, StoreOp, TextureFormat, TextureUsages,
    TextureViewDescriptor, VertexState,
};

use super::{
    kernel_radius_px, mat_mul, outline_plan, OutlineTarget, OutlineView, EDGE_STRENGTH,
    EDGE_THICKNESS, MAX_EDGE_GLOW, MAX_EDGE_THICKNESS,
};
use crate::quality::Quality;

/// The mask shader the mask pipeline compiles.
pub const MASK_WGSL: &str = include_str!("shaders/outline_mask.wgsl");
/// The edge, blur and composite shader.
pub const EDGE_WGSL: &str = include_str!("shaders/outline_edge.wgsl");

/// The colour format every outline target uses: presentation only, so 8 bits
/// per channel holds a mask of 0/1 and a band that saturates at 1.
pub const OUTLINE_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// The most selected targets one mask pass draws: a fighter mesh's worth.
pub const MAX_MASK_DRAWS: usize = 64;

/// One cube corner in local space, the mask pass's stand-in for a fighter mesh.
const CUBE_CORNERS: [[f32; 3]; 8] = [
    [-0.5, -0.5, -0.5],
    [0.5, -0.5, -0.5],
    [0.5, 0.5, -0.5],
    [-0.5, 0.5, -0.5],
    [-0.5, -0.5, 0.5],
    [0.5, -0.5, 0.5],
    [0.5, 0.5, 0.5],
    [-0.5, 0.5, 0.5],
];

/// The cube's twelve triangles.
const CUBE_INDICES: [u16; 36] = [
    0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6, 3, 0,
    4, 3, 4, 7,
];

/// The uniforms one mask draw reads, `shaders/outline_mask.wgsl` `MaskGlobals`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaskGlobals {
    /// The donor's `0.5 bias * projection * view`, row-major.
    pub texture_matrix: [[f32; 4]; 4],
    /// The frame's `projection * view`, row-major.
    pub view_proj: [[f32; 4]; 4],
    /// Depth-prefill size in texels.
    pub prefill_size: [f32; 2],
    /// Near and far plane in metres.
    pub near_far: [f32; 2],
}

impl MaskGlobals {
    /// The bytes the uniform buffer takes: matrices column-major (wgsl), then
    /// the two vec2s, padded to a 16-byte multiple.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut values = [0.0f32; 40];
        let mut at = 0;
        for m in [&self.texture_matrix, &self.view_proj] {
            for col in 0..4 {
                for row in 0..4 {
                    values[at] = m[row][col];
                    at += 1;
                }
            }
        }
        values[32] = self.prefill_size[0];
        values[33] = self.prefill_size[1];
        values[34] = self.near_far[0];
        values[35] = self.near_far[1];
        f32_bytes(&values)
    }
}

/// The uniforms one edge-chain pass reads, `shaders/outline_edge.wgsl`
/// `EdgeGlobals`.
#[derive(Clone, Copy, Debug, Default)]
pub struct EdgeGlobals {
    /// Source size in texels.
    pub tex_size: [f32; 2],
    /// Blur direction, `(1, 0)` across or `(0, 1)` down.
    pub direction: [f32; 2],
    /// Kernel radius in texels.
    pub kernel_radius: f32,
    /// Composite strength, the donor's `edgeStrength`.
    pub edge_strength: f32,
    /// Composite glow weight, the donor's `edgeGlow`.
    pub edge_glow: f32,
    /// Cosine breathing, the donor's `pulsePeriod`.
    pub pulse: f32,
}

impl EdgeGlobals {
    /// The bytes the uniform buffer takes: eight f32 in declaration order.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        f32_bytes(&[
            self.tex_size[0],
            self.tex_size[1],
            self.direction[0],
            self.direction[1],
            self.kernel_radius,
            self.edge_strength,
            self.edge_glow,
            self.pulse,
        ])
    }

    /// The values one edge or blur pass carries: black edge colour choice is the
    /// shader's, the strength belongs to the composite only.
    #[must_use]
    fn pass(tex_size: [f32; 2], direction: [f32; 2], kernel_radius: f32) -> Self {
        Self {
            tex_size,
            direction,
            kernel_radius,
            edge_strength: 0.0,
            edge_glow: 1.0,
            pulse: 1.0,
        }
    }
}

/// The frame one outline encode runs for.
#[derive(Clone, Copy, Debug)]
pub struct OutlineFrame<'a> {
    /// The camera the mask is projected with.
    pub view: &'a OutlineView,
    /// The frame's depth prefill, sampled at `prefill_size`.
    pub prefill: &'a wgpu::TextureView,
    /// The prefill's size in texels, so the mask fragment can index it.
    pub prefill_size: [u32; 2],
    /// The selected targets: one mask draw each, zero draws on an empty list.
    pub targets: &'a [OutlineTarget],
}

/// One target's model matrix: the unit cube scaled to its bounding sphere and
/// moved to its centre.
#[must_use]
fn target_model(target: &OutlineTarget) -> [[f32; 4]; 4] {
    let s = target.radius_m.max(1e-3);
    [
        [s, 0.0, 0.0, target.center[0]],
        [0.0, s, 0.0, target.center[1]],
        [0.0, 0.0, s, target.center[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// The bind groups and pipeline layouts both pipeline sets bind.
struct Layouts {
    globals: wgpu::BindGroupLayout,
    draws: wgpu::BindGroupLayout,
    edge: wgpu::BindGroupLayout,
    mask_pipeline: wgpu::PipelineLayout,
    edge_pipeline: wgpu::PipelineLayout,
}

/// Build the two layouts and the two pipeline layouts, once per chain.
fn layouts(device: &wgpu::Device) -> Layouts {
    let uniform_entry = |binding: u32, visibility: ShaderStages| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sw150-globals-layout"),
        entries: &[uniform_entry(0, ShaderStages::VERTEX_FRAGMENT)],
    });
    let draws = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sw150-draws-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let edge = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sw150-edge-layout"),
        entries: &[
            uniform_entry(0, ShaderStages::FRAGMENT),
            texture_entry(1),
            texture_entry(2),
            texture_entry(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let mask_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sw150-mask-pl"),
        bind_group_layouts: &[Some(&globals), Some(&draws)],
        immediate_size: 0,
    });
    let edge_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sw150-edge-pl"),
        bind_group_layouts: &[Some(&edge)],
        immediate_size: 0,
    });
    Layouts {
        globals,
        draws,
        edge,
        mask_pipeline,
        edge_pipeline,
    }
}

/// The four pipelines: the mask, the edge, the separable blur and the composite.
struct Pipelines {
    mask: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

/// Build the four pipelines over the two modules, once per chain.
fn pipelines(
    device: &wgpu::Device,
    layouts: &Layouts,
    mask_module: &wgpu::ShaderModule,
    edge_module: &wgpu::ShaderModule,
) -> Pipelines {
    let targets = || {
        vec![Some(ColorTargetState {
            format: OUTLINE_FORMAT,
            blend: None,
            write_mask: ColorWrites::ALL,
        })]
        .into_boxed_slice()
    };
    let mask = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sw150-mask"),
        layout: Some(&layouts.mask_pipeline),
        vertex: VertexState {
            module: mask_module,
            entry_point: Some("vs_mask"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
        },
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            module: mask_module,
            entry_point: Some("fs_mask"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &targets(),
        }),
        multiview_mask: None,
        cache: None,
    });
    let full_pass = |label: &'static str, entry: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layouts.edge_pipeline),
            vertex: VertexState {
                module: edge_module,
                entry_point: Some("vs_full"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: edge_module,
                entry_point: Some(entry),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &targets(),
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    Pipelines {
        mask,
        edge: full_pass("sw150-edge", "fs_edge"),
        blur: full_pass("sw150-blur", "fs_blur"),
        composite: full_pass("sw150-composite", "fs_composite"),
    }
}

/// The device-owned outline chain at one half-res size.
pub struct OutlineGpu {
    width: u32,
    height: u32,
    pipelines: Pipelines,
    mask_globals: wgpu::Buffer,
    mask_draws: wgpu::Buffer,
    /// One uniform buffer per edge-chain pass: every write lands at submit, so a
    /// shared buffer would let the last pass's values reach all of them.
    edge_uniforms: [wgpu::Buffer; 6],
    mask_texture: wgpu::Texture,
    edge_a: wgpu::Texture,
    edge_b: wgpu::Texture,
    edge_c: wgpu::Texture,
    /// The composite target the test reads back; the frame path blends the same
    /// band over the HDR frame instead.
    out: wgpu::Texture,
    layouts: Layouts,
    sampler: wgpu::Sampler,
    cube_vertices: wgpu::Buffer,
    cube_indices: wgpu::Buffer,
}

impl OutlineGpu {
    /// Build the chain for a frame at `frame_width` by `frame_height`: every
    /// target runs at `ceil(frame / 2)`. `None` when the tier draws nothing or
    /// the frame has no area, so the potato floor allocates nothing.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        quality: Quality,
        frame_width: u32,
        frame_height: u32,
    ) -> Option<Self> {
        let plan = outline_plan(quality, frame_width, frame_height, 1);
        if !plan.enabled {
            return None;
        }
        let width = plan.mask_size[0];
        let height = plan.mask_size[1];
        let mask_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sw150-outline-mask"),
            source: wgpu::ShaderSource::Wgsl(MASK_WGSL.into()),
        });
        let edge_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sw150-outline-edge"),
            source: wgpu::ShaderSource::Wgsl(EDGE_WGSL.into()),
        });
        let layouts = layouts(device);
        let pipelines = pipelines(device, &layouts, &mask_module, &edge_module);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let target = |label: &'static str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OUTLINE_FORMAT,
                // COPY_SRC so a test (and a debug capture) can read a target back.
                usage: TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let uniform = |label: &'static str, bytes: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let mask_draws = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sw150-mask-draws"),
            size: u64::try_from(MAX_MASK_DRAWS * 64).unwrap_or(4096),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let edge_uniforms = [
            uniform("sw150-edge-u", 32),
            uniform("sw150-blurx-u", 32),
            uniform("sw150-blury-u", 32),
            uniform("sw150-glowx-u", 32),
            uniform("sw150-glowy-u", 32),
            uniform("sw150-composite-u", 32),
        ];
        let cube_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sw150-cube-vertices"),
            size: u64::try_from(CUBE_CORNERS.len() * 12).unwrap_or(96),
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cube_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sw150-cube-indices"),
            size: u64::try_from(CUBE_INDICES.len() * 2).unwrap_or(72),
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Self {
            width,
            height,
            pipelines,
            mask_globals: uniform("sw150-mask-globals", 160),
            mask_draws,
            edge_uniforms,
            mask_texture: target("sw150-mask"),
            edge_a: target("sw150-edge-a"),
            edge_b: target("sw150-edge-b"),
            edge_c: target("sw150-edge-c"),
            out: target("sw150-out"),
            layouts,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("sw150-sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            }),
            cube_vertices,
            cube_indices,
        })
    }

    /// The mask target's size in texels: `ceil(frame / 2)` on both axes.
    #[must_use]
    pub fn mask_size(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    /// Upload the cube the mask pass draws. Once per device, not per frame.
    pub fn upload_mesh(&self, queue: &wgpu::Queue) {
        let mut verts = Vec::with_capacity(CUBE_CORNERS.len() * 3);
        for corner in CUBE_CORNERS {
            verts.extend_from_slice(&corner);
        }
        queue.write_buffer(&self.cube_vertices, 0, &f32_bytes(&verts));
        queue.write_buffer(&self.cube_indices, 0, &u16_bytes(&CUBE_INDICES));
    }

    /// Encode one frame's outline: the mask draw per selected target, the edge
    /// pass, the four blur passes and the composite into [`Self::out`]. Zero
    /// targets encodes nothing, so an unlocked frame costs no draw.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        frame: &OutlineFrame<'_>,
    ) {
        let Some(target_count) = u32::try_from(frame.targets.len()).ok().filter(|n| *n > 0) else {
            return;
        };
        let _ = target_count;
        let mask = self
            .mask_texture
            .create_view(&TextureViewDescriptor::default());
        let a = self.edge_a.create_view(&TextureViewDescriptor::default());
        let b = self.edge_b.create_view(&TextureViewDescriptor::default());
        let c = self.edge_c.create_view(&TextureViewDescriptor::default());
        self.encode_mask(device, queue, enc, frame, &mask);
        self.encode_chain(device, queue, enc, &mask, &a, &b, &c);
        let composite = EdgeGlobals {
            tex_size: [u32_to_f32(self.width), u32_to_f32(self.height)],
            direction: [1.0, 0.0],
            kernel_radius: 1.0,
            edge_strength: EDGE_STRENGTH,
            edge_glow: 1.0,
            pulse: 1.0,
        };
        queue.write_buffer(&self.edge_uniforms[5], 0, &composite.to_bytes());
        let group = edge_group(
            device,
            &self.layouts.edge,
            &self.edge_uniforms[5],
            &self.sampler,
            &mask,
            &a,
            &c,
        );
        let out = self.out.create_view(&TextureViewDescriptor::default());
        clear_then(enc, &out, |rp| {
            rp.set_pipeline(&self.pipelines.composite);
            rp.set_bind_group(0, &group, &[]);
            rp.draw(0..3, 0..1);
        });
    }

    /// The mask pass: the selected targets white, the visibility channel from
    /// the depth compare against the prefill.
    fn encode_mask(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        frame: &OutlineFrame<'_>,
        mask: &wgpu::TextureView,
    ) {
        let globals = MaskGlobals {
            texture_matrix: frame.view.texture_matrix(),
            view_proj: mat_mul(&frame.view.projection(), &frame.view.view_matrix()),
            prefill_size: [
                u32_to_f32(frame.prefill_size[0]),
                u32_to_f32(frame.prefill_size[1]),
            ],
            near_far: [frame.view.near_m, frame.view.far_m],
        };
        queue.write_buffer(&self.mask_globals, 0, &globals.to_bytes());
        let mut draws = Vec::with_capacity(MAX_MASK_DRAWS * 16);
        for target in frame.targets.iter().take(MAX_MASK_DRAWS) {
            let m = target_model(target);
            for col in 0..4 {
                for row in 0..4 {
                    draws.push(m[row][col]);
                }
            }
        }
        queue.write_buffer(&self.mask_draws, 0, &f32_bytes(&draws));
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sw150-mask-bg"),
            layout: &self.layouts.globals,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.mask_globals.as_entire_binding(),
            }],
        });
        let draws_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sw150-draws-bg"),
            layout: &self.layouts.draws,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.mask_draws.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(frame.prefill),
                },
            ],
        });
        let index_count = u32::try_from(CUBE_INDICES.len()).unwrap_or(36);
        let instances = u32::try_from(MAX_MASK_DRAWS).unwrap_or(1);
        let target_count = u32::try_from(frame.targets.len())
            .unwrap_or(instances)
            .min(instances);
        clear_then(enc, mask, |rp| {
            rp.set_pipeline(&self.pipelines.mask);
            rp.set_bind_group(0, &globals_group, &[]);
            rp.set_bind_group(1, &draws_group, &[]);
            rp.set_vertex_buffer(0, self.cube_vertices.slice(..));
            rp.set_index_buffer(self.cube_indices.slice(..), wgpu::IndexFormat::Uint16);
            rp.draw_indexed(0..index_count, 0, 0..target_count);
        });
    }

    /// The edge pass, the thickness pair and the glow pair, each reading what
    /// the pass before it wrote.
    fn encode_chain(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        mask: &wgpu::TextureView,
        a: &wgpu::TextureView,
        b: &wgpu::TextureView,
        c: &wgpu::TextureView,
    ) {
        let size = [u32_to_f32(self.width), u32_to_f32(self.height)];
        let thickness = u32_to_f32(kernel_radius_px(EDGE_THICKNESS, MAX_EDGE_THICKNESS));
        let glow = u32_to_f32(MAX_EDGE_GLOW);
        let chain = [
            (
                &self.pipelines.edge,
                &self.edge_uniforms[0],
                mask,
                a,
                EdgeGlobals::pass(size, [1.0, 0.0], 1.0),
            ),
            (
                &self.pipelines.blur,
                &self.edge_uniforms[1],
                a,
                b,
                EdgeGlobals::pass(size, [1.0, 0.0], thickness),
            ),
            (
                &self.pipelines.blur,
                &self.edge_uniforms[2],
                b,
                a,
                EdgeGlobals::pass(size, [0.0, 1.0], thickness),
            ),
            (
                &self.pipelines.blur,
                &self.edge_uniforms[3],
                a,
                b,
                EdgeGlobals::pass(size, [1.0, 0.0], glow),
            ),
            (
                &self.pipelines.blur,
                &self.edge_uniforms[4],
                b,
                c,
                EdgeGlobals::pass(size, [0.0, 1.0], glow),
            ),
        ];
        for (pipeline, uniforms, source, to, pass) in chain {
            queue.write_buffer(uniforms, 0, &pass.to_bytes());
            let group = edge_group(
                device,
                &self.layouts.edge,
                uniforms,
                &self.sampler,
                mask,
                source,
                source,
            );
            clear_then(enc, to, |rp| {
                rp.set_pipeline(pipeline);
                rp.set_bind_group(0, &group, &[]);
                rp.draw(0..3, 0..1);
            });
        }
    }

    /// Read the composite target back as RGBA8, row-major, `mask_size` texels.
    /// The proof a test reads; a frame path never calls it.
    #[must_use]
    pub fn read_out(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<[u8; 4]> {
        let row_bytes = self.width * 4;
        let bytes_per_row = row_bytes.div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sw150-readback"),
            size: u64::from(bytes_per_row) * u64::from(self.height),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sw150-readback-copy"),
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.out,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(enc.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let _ = rx.recv();
        let data = slice
            .get_mapped_range()
            .expect("the readback buffer maps after the wait");
        let columns = usize::try_from(self.width).unwrap_or(0);
        let rows = usize::try_from(self.height).unwrap_or(0);
        let stride = usize::try_from(bytes_per_row).unwrap_or(0);
        let mut out = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            let line = &data[row * stride..];
            for col in 0..columns {
                let p = &line[col * 4..col * 4 + 4];
                out.push([p[0], p[1], p[2], p[3]]);
            }
        }
        drop(data);
        readback.unmap();
        out
    }
}

/// One bind group for an edge-chain pass: the uniforms and the three textures it
/// may read, bound in the layout's order.
#[expect(clippy::too_many_arguments, reason = "one pass's five bindings")]
fn edge_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    mask: &wgpu::TextureView,
    edge1: &wgpu::TextureView,
    edge2: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sw150-edge-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(mask),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(edge1),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(edge2),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Clear `to` and record one draw inside that pass.
fn clear_then(
    enc: &mut wgpu::CommandEncoder,
    to: &wgpu::TextureView,
    record: impl FnOnce(&mut wgpu::RenderPass<'_>),
) {
    let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
        label: Some("sw150-pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: to,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    record(&mut rp);
}

/// f32 slice to bytes, little-endian, for a buffer upload.
fn f32_bytes(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// u16 slice to bytes, little-endian, for an index upload.
fn u16_bytes(values: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// A texel count as f32; frame sizes stay far below 2^24.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "texel counts fit f32")]
fn u32_to_f32(v: u32) -> f32 {
    v as f32
}

/// The depth a point `z_m` metres in front of the eye writes into a wgpu
/// `0..=1` depth buffer with `near` and `far`: the inverse of the shader's
/// `depth_to_view_z`, used to fill a prefill texture from the CPU in a test.
#[must_use]
pub fn depth_for(z_m: f32, near: f32, far: f32) -> f32 {
    far * (z_m - near) / (z_m * (far - near))
}

#[cfg(test)]
#[path = "outline_gpu_tests.rs"]
mod tests;
