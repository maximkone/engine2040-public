//! gpu: the viewmodel pass on the GPU.
//!
//! The rim draws in its own depth range on its own depth target: the scene
//! draws first into the world's depth target, then the rim draws into a
//! second target of its own, cleared and never touched by the world. A wall
//! five centimetres in front of the eye therefore cannot cover the rim.
//!
//! The capture helper draws the scene and the rim, reads both frames back
//! and writes the proof PNG, so the receipt is the file itself.
//!
//! Native only: it blocks on the GPU and writes the capture PNG to disk.

use super::{
    apply_decals, build_rim_mesh, viewmodel_view_proj, worn_decals, AperturePreset, DepthRange,
    RimLight, RimMesh, ViewmodelError, VIEWMODEL_DEPTH, VIEWMODEL_WGSL,
};
use crate::lens::LensPreset;
use wgpu::util::DeviceExt;

/// The vertex stride of the rim and the scene: position(3) + normal(3)
/// + color(3) + roughness(1) floats.
const VERTEX_STRIDE: u64 = 40;
/// `copy_texture_to_buffer` rows must be 256-byte aligned.
const ROW_ALIGN: u32 = 256;
/// The color target of the capture.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The vertical field of view of the capture, radians.
const CAPTURE_FOV_Y_RAD: f32 = 1.05;
/// Where the wall of the capture stands, in front of the eye, in meters:
/// nearer than the rim's nearest point (54 mm), so a shared depth buffer
/// would hide the rim behind the wall.
const CAPTURE_WALL_Z_M: f32 = 0.05;
/// The wall's right edge, in the frame's half width.
const WALL_EDGE_HALF: f32 = -0.12;
/// The wall's cover as a share of the picture's width, in percent: the
/// wall covers the left 44 percent of the picture. The edge above and
/// this number are one edge in two units, and the test holds them
/// together.
const WALL_COVER_PERCENT: u32 = 44;
/// How far the sky and the sand stand behind the eye, in meters.
const SCENE_Z_M: f32 = 60.0;
/// The world's own depth range for the scene behind the rim.
const WORLD_DEPTH: DepthRange = DepthRange {
    near_m: 0.001,
    far_m: 200.0,
};

/// One vertex of the scene or the rim: the same layout for both.
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    roughness: f32,
}

/// What one viewmodel frame needs.
pub struct FrameSpec<'a> {
    /// The rim mesh to draw.
    pub mesh: &'a RimMesh,
    /// The scene light the rim takes.
    pub light: RimLight,
    /// The vertical field of view, radians.
    pub fov_y_rad: f32,
    /// The width over height.
    pub aspect: f32,
    /// The frame width, pixels.
    pub width: u32,
    /// The frame height, pixels.
    pub height: u32,
    /// How far the wall of the scene stands in front of the eye, in
    /// meters. The wall is the proof: the rim must survive it.
    pub wall_z_m: f32,
}

/// One frame the pass produced: the picture with the rim and the
/// picture without it.
pub struct ViewmodelFrame {
    /// The frame with the rim on top, row-major RGBA8.
    pub pixels: Vec<[u8; 4]>,
    /// The same frame without the rim: the scene alone.
    pub scene_pixels: Vec<[u8; 4]>,
    /// The frame width in pixels.
    pub width: u32,
    /// The frame height in pixels.
    pub height: u32,
    /// The wgpu validation errors the pass saw; green means 0.
    pub validation_errors: u32,
}

/// One capture: the file written and the pixels read back from it.
pub struct HelmetCapture {
    /// The preset the capture shows.
    pub preset: AperturePreset,
    /// The PNG file path.
    pub path: String,
    /// The pixels read back from the PNG, row-major.
    pub pixels: Vec<[u8; 4]>,
    /// The pixels the rim covers.
    pub rim_pixels: usize,
    /// The rim pixels over the wall five centimetres in front.
    pub rim_pixels_over_wall: usize,
    /// The mean luma of the capture, 0..1.
    pub mean_luma: f32,
    /// The wgpu validation errors the pass saw; green means 0.
    pub validation_errors: u32,
}

/// The viewmodel pipeline: the rim shader over one uniform.
pub struct ViewmodelPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

/// The textures one frame needs: the color target and the two depth
/// targets, the world's and the rim's own.
struct FrameTargets {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    world_depth: wgpu::TextureView,
    rim_depth: wgpu::TextureView,
    width: u32,
    height: u32,
}

/// The buffers one frame needs: the scene, the rim and the two uniforms.
struct FrameBuffers {
    scene_vertices: wgpu::Buffer,
    scene_indices: wgpu::Buffer,
    scene_count: u32,
    rim_vertices: wgpu::Buffer,
    rim_indices: wgpu::Buffer,
    rim_count: u32,
    world_bind: wgpu::BindGroup,
    rim_bind: wgpu::BindGroup,
}

/// The scene mesh: the vertices and the triangle indices.
struct SceneMesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl ViewmodelPass {
    /// Compile the viewmodel pipeline on `device`.
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewmodel-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewmodel"),
            source: wgpu::ShaderSource::Wgsl(VIEWMODEL_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewmodel"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewmodel"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 36,
                            shader_location: 3,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    /// Draw the scene and the rim into one frame and read both back.
    ///
    /// The scene draws first into the world's depth target. The rim draws
    /// after it into its own depth target, cleared and never touched by
    /// the world, so the wall five centimetres in front of the eye cannot
    /// cover the rim.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spec: &FrameSpec<'_>,
    ) -> Result<ViewmodelFrame, ViewmodelError> {
        let (width, height) = (spec.width, spec.height);
        if width == 0 || height == 0 {
            return Err(ViewmodelError::Invalid(
                "the viewmodel frame is empty".into(),
            ));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let scene = scene_mesh(spec.fov_y_rad, spec.aspect, spec.wall_z_m);
        let targets = FrameTargets::new(device, width, height);
        let world_proj = viewmodel_view_proj(WORLD_DEPTH, spec.fov_y_rad, spec.aspect)?;
        let rim_proj = viewmodel_view_proj(VIEWMODEL_DEPTH, spec.fov_y_rad, spec.aspect)?;
        let buffers = FrameBuffers::new(device, spec, &scene, &self.layout, world_proj, rim_proj);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewmodel"),
        });
        let scene_read = self.encode_world(device, &mut encoder, &targets, &buffers);
        let rim_read = self.encode_rim(device, &mut encoder, &targets, &buffers);
        queue.submit(Some(encoder.finish()));
        finish_scope(device, scope)?;
        let scene_pixels = read_pixels(device, scene_read, width, height)?;
        let pixels = read_pixels(device, rim_read, width, height)?;
        Ok(ViewmodelFrame {
            pixels,
            scene_pixels,
            width,
            height,
            validation_errors: 0,
        })
    }

    /// The world pass: the scene into the world's depth target, then the
    /// copy of the color target for the readback.
    fn encode_world(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        targets: &FrameTargets,
        buffers: &FrameBuffers,
    ) -> wgpu::Buffer {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewmodel-world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.45,
                            g: 0.58,
                            b: 0.74,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.world_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &buffers.world_bind, &[]);
            pass.set_vertex_buffer(0, buffers.scene_vertices.slice(..));
            pass.set_index_buffer(buffers.scene_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..buffers.scene_count, 0, 0..1);
        }
        readback_buffer(
            device,
            encoder,
            &targets.color,
            targets.width(),
            targets.height(),
        )
    }

    /// The rim pass: the rim into its own depth target, then the copy of
    /// the color target for the readback.
    fn encode_rim(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        targets: &FrameTargets,
        buffers: &FrameBuffers,
    ) -> wgpu::Buffer {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewmodel-rim"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.rim_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &buffers.rim_bind, &[]);
            pass.set_vertex_buffer(0, buffers.rim_vertices.slice(..));
            pass.set_index_buffer(buffers.rim_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..buffers.rim_count, 0, 0..1);
        }
        readback_buffer(
            device,
            encoder,
            &targets.color,
            targets.width(),
            targets.height(),
        )
    }
}

impl FrameTargets {
    /// The color target and the two depth targets of one frame.
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewmodel-color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let world_depth = depth_texture(device, "viewmodel-world-depth", width, height);
        let rim_depth = depth_texture(device, "viewmodel-rim-depth", width, height);
        Self {
            color,
            color_view,
            world_depth: world_depth.create_view(&wgpu::TextureViewDescriptor::default()),
            rim_depth: rim_depth.create_view(&wgpu::TextureViewDescriptor::default()),
            width,
            height,
        }
    }

    /// The width of the frame, pixels.
    fn width(&self) -> u32 {
        self.width
    }

    /// The height of the frame, pixels.
    fn height(&self) -> u32 {
        self.height
    }
}

impl FrameBuffers {
    /// The scene, the rim and the two uniforms of one frame.
    fn new(
        device: &wgpu::Device,
        spec: &FrameSpec<'_>,
        scene: &SceneMesh,
        layout: &wgpu::BindGroupLayout,
        world_proj: [f32; 16],
        rim_proj: [f32; 16],
    ) -> Self {
        let scene_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewmodel-scene-vertices"),
            contents: &pack_vertices(&scene.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let scene_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewmodel-scene-indices"),
            contents: &pack_indices(&scene.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let rim_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewmodel-rim-vertices"),
            contents: &pack_vertices(&rim_vertices_of(spec.mesh)),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let rim_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewmodel-rim-indices"),
            contents: &pack_indices(&spec.mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let world_uniform =
            uniform_buffer(device, "viewmodel-world-uniform", &world_proj, spec.light);
        let rim_uniform = uniform_buffer(device, "viewmodel-rim-uniform", &rim_proj, spec.light);
        let world_bind = bind(device, layout, &world_uniform);
        let rim_bind = bind(device, layout, &rim_uniform);
        Self {
            scene_vertices,
            scene_indices,
            scene_count: u32::try_from(scene.indices.len()).unwrap_or(0),
            rim_vertices,
            rim_indices,
            rim_count: u32::try_from(spec.mesh.indices.len()).unwrap_or(0),
            world_bind,
            rim_bind,
        }
    }
}

/// The rim mesh as the pass's own vertices: one layout for the scene and
/// the rim, so one byte writer serves both.
fn rim_vertices_of(mesh: &RimMesh) -> Vec<Vertex> {
    mesh.vertices
        .iter()
        .map(|vertex| Vertex {
            position: vertex.position_m,
            normal: vertex.normal,
            color: vertex.color,
            roughness: vertex.roughness,
        })
        .collect()
}

/// The uniform block of one pass: the projection, the sun and the ambient.
fn uniform_buffer(
    device: &wgpu::Device,
    label: &str,
    view_proj: &[f32; 16],
    light: RimLight,
) -> wgpu::Buffer {
    let bytes = pack_uniform(view_proj, &light);
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

/// The bind group of one uniform buffer.
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewmodel-bind"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    })
}

/// A depth texture for one pass: the world's or the rim's own.
fn depth_texture(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// The scene behind the rim: a far sky, a tree line and sand, plus the
/// wall that stands in front of the eye.
fn scene_mesh(fov_y_rad: f32, aspect: f32, wall_z_m: f32) -> SceneMesh {
    let half_height = SCENE_Z_M * (fov_y_rad * 0.5).tan();
    let half_width = half_height * aspect;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    // The far bands: sky, tree line, sand.
    let bands = [
        (1.0f32, 0.12f32, [0.45f32, 0.58, 0.74]),
        (0.12, -0.02, [0.14, 0.16, 0.12]),
        (-0.02, -1.0, [0.58, 0.47, 0.28]),
    ];
    for (top, bottom, color) in bands {
        let top_y = top * half_height;
        let bottom_y = bottom * half_height;
        let base = u32::try_from(vertices.len()).unwrap_or(0);
        vertices.push(Vertex {
            position: [-half_width, bottom_y, SCENE_Z_M],
            normal: [0.0, 0.0, 1.0],
            color,
            roughness: 0.9,
        });
        vertices.push(Vertex {
            position: [half_width, bottom_y, SCENE_Z_M],
            normal: [0.0, 0.0, 1.0],
            color,
            roughness: 0.9,
        });
        vertices.push(Vertex {
            position: [half_width, top_y, SCENE_Z_M],
            normal: [0.0, 0.0, 1.0],
            color,
            roughness: 0.9,
        });
        vertices.push(Vertex {
            position: [-half_width, top_y, SCENE_Z_M],
            normal: [0.0, 0.0, 1.0],
            color,
            roughness: 0.9,
        });
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // The wall: five centimetres in front of the eye, over the left of
    // the picture.
    let wall_half_height = wall_z_m * (fov_y_rad * 0.5).tan();
    let wall_half_width = wall_half_height * aspect;
    let wall_right = WALL_EDGE_HALF * wall_half_width;
    let base = u32::try_from(vertices.len()).unwrap_or(0);
    let stone = [0.40f32, 0.38, 0.34];
    vertices.push(Vertex {
        position: [-wall_half_width, -wall_half_height, wall_z_m],
        normal: [0.0, 0.0, 1.0],
        color: stone,
        roughness: 0.85,
    });
    vertices.push(Vertex {
        position: [wall_right, -wall_half_height, wall_z_m],
        normal: [0.0, 0.0, 1.0],
        color: stone,
        roughness: 0.85,
    });
    vertices.push(Vertex {
        position: [wall_right, wall_half_height, wall_z_m],
        normal: [0.0, 0.0, 1.0],
        color: stone,
        roughness: 0.85,
    });
    vertices.push(Vertex {
        position: [-wall_half_width, wall_half_height, wall_z_m],
        normal: [0.0, 0.0, 1.0],
        color: stone,
        roughness: 0.85,
    });
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    SceneMesh { vertices, indices }
}

/// The vertex bytes: position, normal, color and roughness.
fn pack_vertices(vertices: &[Vertex]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertices.len() * 40);
    for vertex in vertices {
        for value in vertex.position {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.normal {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.color {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&vertex.roughness.to_le_bytes());
    }
    out
}

/// The index bytes: every index as four little-endian bytes.
fn pack_indices(indices: &[u32]) -> Vec<u8> {
    indices
        .iter()
        .flat_map(|index| index.to_le_bytes())
        .collect()
}

/// The uniform block: the projection, the sun and the ambient.
fn pack_uniform(view_proj: &[f32; 16], light: &RimLight) -> Vec<u8> {
    let mut out = Vec::with_capacity(112);
    for value in view_proj {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in light.direction {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&light.intensity.to_le_bytes());
    for value in light.color {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0f32.to_le_bytes());
    for value in light.ambient {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0f32.to_le_bytes());
    out
}

/// Wait for the device and read the validation scope: no error means the
/// pass is clean.
fn finish_scope(device: &wgpu::Device, scope: wgpu::ErrorScopeGuard) -> Result<(), ViewmodelError> {
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|err| ViewmodelError::Device(format!("poll: {err}")))?;
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(ViewmodelError::Validation(format!("{err:?}")));
    }
    Ok(())
}

/// A readback buffer and the copy of the color target into it.
fn readback_buffer(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::Texture,
    width: u32,
    height: u32,
) -> wgpu::Buffer {
    let bytes_per_row = width * 4;
    let padded = bytes_per_row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewmodel-readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    buffer
}

/// Map a readback buffer and strip the row padding.
fn read_pixels(
    device: &wgpu::Device,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
) -> Result<Vec<[u8; 4]>, ViewmodelError> {
    let bytes_per_row = usize::try_from(width).unwrap_or(0) * 4;
    let padded = u32::try_from(bytes_per_row)
        .unwrap_or(0)
        .div_ceil(ROW_ALIGN)
        * ROW_ALIGN;
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|err| ViewmodelError::Device(format!("poll: {err}")))?;
    rx.recv()
        .map_err(|err| ViewmodelError::Device(format!("map channel: {err}")))?
        .map_err(|err| ViewmodelError::Device(format!("map: {err}")))?;
    let mut out = Vec::with_capacity(bytes_per_row * usize::try_from(height).unwrap_or(0));
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|err| ViewmodelError::Device(format!("mapped range: {err}")))?;
        for row in 0..usize::try_from(height).unwrap_or(0) {
            let start = row * usize::try_from(padded).unwrap_or(0);
            for chunk in mapped[start..start + bytes_per_row].chunks_exact(4) {
                out.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
        }
    }
    buffer.unmap();
    Ok(out)
}

/// Run the pass for every preset and write one PNG per preset into
/// `dir`. The pixels are read back from the written file, so the
/// receipt is the file itself.
pub fn capture_presets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    dir: &std::path::Path,
    presets: &[AperturePreset],
    width: u32,
    height: u32,
) -> Result<Vec<HelmetCapture>, ViewmodelError> {
    let pass = ViewmodelPass::new(device);
    let mut out = Vec::with_capacity(presets.len());
    for preset in presets {
        out.push(capture_one(
            &pass, device, queue, *preset, dir, width, height,
        )?);
    }
    Ok(out)
}

/// One capture: the scene with the rim, the rim's pixel count and the
/// PNG on disk.
fn capture_one(
    pass: &ViewmodelPass,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    preset: AperturePreset,
    dir: &std::path::Path,
    width: u32,
    height: u32,
) -> Result<HelmetCapture, ViewmodelError> {
    let mut mesh = build_rim_mesh(preset)?;
    apply_decals(&mut mesh, &worn_decals());
    let light = RimLight::scene();
    let spec = FrameSpec {
        mesh: &mesh,
        light,
        fov_y_rad: CAPTURE_FOV_Y_RAD,
        aspect: aspect_of(width, height),
        width,
        height,
        wall_z_m: CAPTURE_WALL_Z_M,
    };
    let frame = pass.render(device, queue, &spec)?;
    // The rim's pixels: the ones the rim moved. The wall covers the
    // left of the picture and stands nearer than the rim, so the rim
    // pixels over it prove the rim survives a wall five centimetres in
    // front of the eye.
    let wall_columns = wall_columns(width);
    let row_width = usize::try_from(width).unwrap_or(1);
    let mut rim_pixels = 0usize;
    let mut rim_pixels_over_wall = 0usize;
    for (index, (with_rim, without)) in frame
        .pixels
        .iter()
        .zip(frame.scene_pixels.iter())
        .enumerate()
    {
        if with_rim != without {
            rim_pixels += 1;
            if index % row_width < wall_columns {
                rim_pixels_over_wall += 1;
            }
        }
    }
    // The helmet lens rides the frame: the same bodycam look the
    // first-person view takes.
    let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
    for pixel in &frame.pixels {
        rgba.extend_from_slice(pixel);
    }
    let profile = LensPreset::Helmet.profile();
    let lensed = crate::lens::apply_bytes(&profile, 0, &rgba, width, height)
        .map_err(|err| ViewmodelError::Invalid(format!("viewmodel lens: {err}")))?;
    let png = crate::v1::encode_png_rgba8(&lensed, width, height)
        .map_err(|err| ViewmodelError::Io(format!("viewmodel png: {err}")))?;
    std::fs::create_dir_all(dir)
        .map_err(|err| ViewmodelError::Io(format!("viewmodel mkdir: {err}")))?;
    let path = dir.join(format!("{}.png", preset.name()));
    std::fs::write(&path, &png)
        .map_err(|err| ViewmodelError::Io(format!("viewmodel write: {err}")))?;
    let bytes =
        std::fs::read(&path).map_err(|err| ViewmodelError::Io(format!("viewmodel read: {err}")))?;
    let pixels = read_png(&bytes)?;
    Ok(HelmetCapture {
        preset,
        path: path.display().to_string(),
        pixels,
        rim_pixels,
        rim_pixels_over_wall,
        mean_luma: crate::lens::mean_luma(&lensed),
        validation_errors: frame.validation_errors,
    })
}

/// The width over height of a frame.
fn aspect_of(width: u32, height: u32) -> f32 {
    f32::from(u16::try_from(width).unwrap_or(u16::MAX))
        / f32::from(u16::try_from(height.max(1)).unwrap_or(u16::MAX))
}

/// The columns the wall covers: the left 44 percent of the picture.
fn wall_columns(width: u32) -> usize {
    let cover = width * WALL_COVER_PERCENT / 100;
    usize::try_from(cover).unwrap_or(0)
}

/// The pixels of a PNG, row-major.
fn read_png(bytes: &[u8]) -> Result<Vec<[u8; 4]>, ViewmodelError> {
    let image = image::load_from_memory(bytes)
        .map_err(|err| ViewmodelError::Io(format!("viewmodel png decode: {err}")))?;
    let rgba = image.to_rgba8();
    let mut out = Vec::with_capacity(
        usize::try_from(rgba.width()).unwrap_or(0) * usize::try_from(rgba.height()).unwrap_or(0),
    );
    for pixel in rgba.pixels() {
        out.push([pixel[0], pixel[1], pixel[2], pixel[3]]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wall's edge and its cover are one edge in two units: the
    /// capture counts the columns under the wall, so the two must agree.
    #[test]
    fn wall_edge_and_its_cover_are_one_edge() {
        let covered = (0.5 + WALL_EDGE_HALF * 0.5) * 100.0;
        assert!(
            (covered - WALL_COVER_PERCENT as f32).abs() < 0.5,
            "the wall edge {WALL_EDGE_HALF} and the cover {WALL_COVER_PERCENT} percent are one edge"
        );
        // The capture counts the columns under the wall the same way.
        assert_eq!(wall_columns(320), 140);
        assert_eq!(wall_columns(100), 44);
    }
}
