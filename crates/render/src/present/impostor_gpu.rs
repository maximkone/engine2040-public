//! impostor_gpu: the device half of the octahedral impostors, behind `gpu`.
//!
//! Owner: render lane; the CPU math is `crate::impostor`, the shaders are
//! `shaders/impostor_bake.wgsl` (bake and downsample) and
//! `shaders/impostor.wgsl` (the instanced quad draw). Units: meters for the
//! mesh and the instance centres, radians for the instance yaw, linear colour
//! for tints, pixels for the atlas. Determinism: the same mesh and layout
//! bake the same atlas bytes; draws read no clock. Errors: typed
//! [`ImpostorGpuError`], nothing panics. Budget: the potato atlas is 8 by 8
//! frames of 64 px in 2 MiB (albedo plus normal); the bake runs once per mesh
//! id and layout, the draw is one quad batch per baked mesh and costs no
//! allocation per frame.
//!
//! Bake: every frame direction of the atlas renders the mesh with an
//! orthographic camera fitted to its bounding sphere (`bake_basis`, the axes
//! the runtime quad shares) at 2x into an albedo target (alpha is coverage)
//! and a world-normal target (`n * 0.5 + 0.5`, tunic mask in alpha), then a
//! box-filter pass writes the atlas. Draw: [`ImpostorSubmitter`] records one
//! `draw(0..6, 0..instances)` per baked mesh inside the caller's pass.

use std::fmt;

use glam::Vec3;

use crate::impostor::{
    bake_basis, frame_direction, AtlasLayout, ImpostorError, TEXEL_BYTES_ALBEDO_NORMAL,
};
use crate::present::{DEPTH_FORMAT, HDR_FORMAT, MOTION_FORMAT};

/// Frames per atlas side of the potato budget.
pub const POTATO_FRAMES_PER_SIDE: u32 = 8;
/// Atlas byte budget of the potato tier: albedo plus normal in 2 MiB.
pub const POTATO_BUDGET_BYTES: u64 = 2 << 20;
/// Atlas edge cap of the potato tier in pixels.
pub const POTATO_EDGE_PX: u32 = 512;
/// The bake renders at this many times the atlas edge, then box-filters down.
pub const BAKE_SUPERSAMPLE: u32 = 2;
/// Albedo atlas format: linear colour stored sRGB, alpha is coverage.
pub const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Normal atlas format: `n * 0.5 + 0.5` in rgb, the tunic mask in alpha.
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Bytes of one staged instance (`Instance` in `impostor.wgsl`).
pub const IMPOSTOR_INSTANCE_BYTES: usize = 32;
/// Bytes of the per-mesh item uniform (`ImpostorItem` in `impostor.wgsl`).
const ITEM_BYTES: usize = 32;
/// Stride of one frame's matrix in the bake uniform (the dynamic offset step).
const FRAME_STRIDE: u64 = 256;
/// Bytes of one interleaved bake vertex: position, normal, uv.
const BAKE_VERTEX_BYTES: u64 = 32;
/// Margin on the fitted sphere so silhouettes never touch the frame edge.
const SPHERE_MARGIN: f32 = 1.03;
/// Alpha below this is cut out of the draw.
const CUTOFF: f32 = 0.5;
/// Most instances one baked mesh stages.
pub const MAX_IMPOSTOR_INSTANCES: usize = 65_536;

/// Bake or stage failure; every variant names what to fix.
#[derive(Debug)]
pub enum ImpostorGpuError {
    /// The impostor math refused the layout or a direction.
    Math(ImpostorError),
    /// The mesh has no triangles to bake.
    EmptyMesh,
    /// One mesh part is malformed (attribute lengths, an index, a value).
    BadPart(usize),
    /// One part texture is malformed (size or byte count).
    BadTexture(usize),
    /// More instances than one batch holds; request and cap carried.
    TooManyInstances {
        /// Instances requested.
        requested: usize,
        /// Instances one batch holds.
        allowed: usize,
    },
    /// The device reported a validation error while baking.
    Validation(String),
}

impl fmt::Display for ImpostorGpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Math(err) => write!(f, "impostor math: {err}"),
            Self::EmptyMesh => f.write_str("impostor mesh has no triangles"),
            Self::BadPart(i) => write!(f, "impostor mesh part {i} is malformed"),
            Self::BadTexture(i) => write!(f, "impostor mesh part {i} texture is malformed"),
            Self::TooManyInstances { requested, allowed } => {
                write!(
                    f,
                    "impostor batch holds {allowed} instances, got {requested}"
                )
            }
            Self::Validation(text) => write!(f, "impostor device validation: {text}"),
        }
    }
}

impl std::error::Error for ImpostorGpuError {}

impl From<ImpostorError> for ImpostorGpuError {
    fn from(err: ImpostorError) -> Self {
        Self::Math(err)
    }
}

/// The potato atlas layout: 8 by 8 frames of 64 px, 2 MiB, 512 px edge.
///
/// # Errors
/// Never fails for the shipped constants; the math error is carried.
pub fn potato_layout() -> Result<AtlasLayout, ImpostorError> {
    AtlasLayout::fit(
        POTATO_FRAMES_PER_SIDE,
        TEXEL_BYTES_ALBEDO_NORMAL,
        POTATO_BUDGET_BYTES,
        POTATO_EDGE_PX,
    )
}

/// One RGBA8 base-colour image (sRGB bytes), `width * height * 4` long.
#[derive(Clone, PartialEq, Debug)]
pub struct BakeTexture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA8 bytes.
    pub rgba: Vec<u8>,
}

/// One part of the baked mesh: posed vertices, a factor and an optional map.
#[derive(Clone, PartialEq, Debug)]
pub struct BakePart {
    /// Positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals, one per position.
    pub normals: Vec<[f32; 3]>,
    /// First UV set, one per position.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// Linear base-colour factor.
    pub base_color: [f32; 3],
    /// Base-colour map the factor multiplies; `None` bakes the factor alone.
    pub texture: Option<BakeTexture>,
    /// True when the instance tunic tint recolours this part.
    pub tunic: bool,
}

/// A baked figure: every part in one pose.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct BakeMesh {
    /// The parts, drawn in order.
    pub parts: Vec<BakePart>,
}

impl BakeMesh {
    /// Check every part before a bake trusts it.
    ///
    /// # Errors
    /// [`ImpostorGpuError::EmptyMesh`], [`ImpostorGpuError::BadPart`] or
    /// [`ImpostorGpuError::BadTexture`] naming the first bad part.
    pub fn validate(&self) -> Result<(), ImpostorGpuError> {
        let mut tris = 0usize;
        for (i, part) in self.parts.iter().enumerate() {
            let n = part.positions.len();
            let sized = part.normals.len() == n && part.uvs.len() == n;
            let finite = part.positions.iter().flatten().all(|c| c.is_finite())
                && part.normals.iter().flatten().all(|c| c.is_finite())
                && part.uvs.iter().flatten().all(|c| c.is_finite())
                && part.base_color.iter().all(|c| c.is_finite());
            let in_range = part
                .indices
                .iter()
                .all(|index| usize::try_from(*index).is_ok_and(|k| k < n));
            if !sized || !finite || !in_range || part.indices.len() % 3 != 0 {
                return Err(ImpostorGpuError::BadPart(i));
            }
            if let Some(tex) = &part.texture {
                let want = usize::try_from(tex.width)
                    .ok()
                    .zip(usize::try_from(tex.height).ok())
                    .map(|(w, h)| w.saturating_mul(h).saturating_mul(4));
                if tex.width == 0 || tex.height == 0 || want != Some(tex.rgba.len()) {
                    return Err(ImpostorGpuError::BadTexture(i));
                }
            }
            tris += part.indices.len() / 3;
        }
        if tris == 0 {
            return Err(ImpostorGpuError::EmptyMesh);
        }
        Ok(())
    }

    /// Positions of the parts that have triangles: what the bake draws.
    fn drawn_positions(&self) -> impl Iterator<Item = &[f32; 3]> {
        self.parts
            .iter()
            .filter(|part| !part.indices.is_empty())
            .flat_map(|part| part.positions.iter())
    }

    /// Bounding sphere `(centre, radius)`: the box centre and the farthest
    /// vertex from it. Zero radius only for a one-point mesh.
    #[must_use]
    pub fn bounding_sphere(&self) -> ([f32; 3], f32) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in self.drawn_positions() {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if lo[0] > hi[0] {
            return ([0.0; 3], 0.0);
        }
        let centre = [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ];
        let far2 = self
            .drawn_positions()
            .map(|p| {
                let d = [p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            })
            .fold(0.0f32, f32::max);
        (centre, far2.sqrt())
    }
}

/// A baked atlas: the two textures plus the sphere it was fitted to.
pub struct ImpostorAtlas {
    /// Frame grid, edge and bytes.
    pub layout: AtlasLayout,
    /// Bounding-sphere centre in the mesh's own space, meters.
    pub centre_m: [f32; 3],
    /// Fitted sphere radius in meters (what the quad spans).
    pub radius_m: f32,
    /// Wall milliseconds the bake took, submit to idle (0 on the web).
    pub bake_ms: f64,
    albedo: wgpu::Texture,
    normal: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    normal_view: wgpu::TextureView,
}

impl ImpostorAtlas {
    /// The albedo atlas (readback and receipts).
    #[must_use]
    pub fn albedo_texture(&self) -> &wgpu::Texture {
        &self.albedo
    }

    /// The normal atlas (readback and receipts).
    #[must_use]
    pub fn normal_texture(&self) -> &wgpu::Texture {
        &self.normal
    }
}

fn pad_f32(v: u32) -> f32 {
    f32::from(u16::try_from(v).unwrap_or(u16::MAX))
}

fn put(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// View-projection (column-major) of frame `i`: an orthographic camera on the
/// frame direction, looking at the sphere centre, spanning the radius.
fn frame_view_proj(
    n: u32,
    i: u32,
    centre: [f32; 3],
    radius: f32,
) -> Result<[f32; 16], ImpostorGpuError> {
    dir_view_proj(frame_direction(n, i)?, centre, radius)
}

/// View-projection (column-major) of an orthographic camera on `dir`
/// (object toward camera) with the sprite axes `bake_basis` gives: the matrix
/// of a baked frame, and of any live view the quad is checked against.
fn dir_view_proj(
    dir: [f32; 3],
    centre: [f32; 3],
    radius: f32,
) -> Result<[f32; 16], ImpostorGpuError> {
    let (_right, up) = bake_basis(dir)?;
    let d = Vec3::from(dir);
    let eye = Vec3::from(centre) + d * (radius * 2.0);
    let view = glam::camera::rh::view::look_to_mat4(eye, -d, Vec3::from(up));
    let proj = glam::camera::rh::proj::directx::orthographic(
        -radius,
        radius,
        -radius,
        radius,
        radius * 0.5,
        radius * 3.5,
    );
    Ok((proj * view).to_cols_array())
}

/// Half-size mip: 2 by 2 box average per channel (bytes, sRGB-approximate).
fn halve(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = ((width / 2).max(1), (height / 2).max(1));
    let w = usize::try_from(width).unwrap_or(1);
    let mut out = Vec::with_capacity(usize::try_from(nw * nh * 4).unwrap_or(0));
    for y in 0..nh {
        for x in 0..nw {
            let x0 = usize::try_from(x * 2).unwrap_or(0);
            let y0 = usize::try_from(y * 2).unwrap_or(0);
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(usize::try_from(height).unwrap_or(1) - 1);
            for c in 0..4 {
                let at = |xx: usize, yy: usize| u32::from(rgba[(yy * w + xx) * 4 + c]);
                let sum = at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1);
                out.push(u8::try_from((sum + 2) / 4).unwrap_or(u8::MAX));
            }
        }
    }
    (nw, nh, out)
}

/// The GPU side of one part: buffers, its uniform and its bind group.
struct PartGpu {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    count: u32,
    bind: wgpu::BindGroup,
}

/// Vertex, index and uniform buffers of one part.
fn part_buffers(
    device: &wgpu::Device,
    part: &BakePart,
) -> (wgpu::Buffer, wgpu::Buffer, wgpu::Buffer) {
    use wgpu::util::DeviceExt as _;
    let mut verts = Vec::with_capacity(part.positions.len() * 32);
    for ((p, n), uv) in part
        .positions
        .iter()
        .zip(part.normals.iter())
        .zip(part.uvs.iter())
    {
        for c in p.iter().chain(n.iter()).chain(uv.iter()) {
            put(&mut verts, *c);
        }
    }
    let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rl64-bake-vertex"),
        contents: &verts,
        usage: wgpu::BufferUsages::VERTEX,
    });
    let mut idx = Vec::with_capacity(part.indices.len() * 4);
    for i in &part.indices {
        put_u32(&mut idx, *i);
    }
    let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rl64-bake-index"),
        contents: &idx,
        usage: wgpu::BufferUsages::INDEX,
    });
    let mut uniform = Vec::with_capacity(32);
    for c in part.base_color {
        put(&mut uniform, c);
    }
    put(&mut uniform, if part.tunic { 1.0 } else { 0.0 });
    put(&mut uniform, if part.texture.is_some() { 1.0 } else { 0.0 });
    for _ in 0..3 {
        put(&mut uniform, 0.0);
    }
    let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rl64-bake-part"),
        contents: &uniform,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    (vertex, index, ubuf)
}

/// The base-colour map (a white texel when the part has none) with its mip
/// chain written, as a view.
fn base_color_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    map: Option<&BakeTexture>,
) -> wgpu::TextureView {
    let white = BakeTexture {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
    };
    let tex = map.unwrap_or(&white);
    let mut levels = vec![(tex.width, tex.height, tex.rgba.clone())];
    loop {
        let next = match levels.last() {
            Some((w, h, data)) if *w > 1 || *h > 1 => halve(*w, *h, data),
            _ => break,
        };
        levels.push(next);
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rl64-bake-base-color"),
        size: wgpu::Extent3d {
            width: tex.width,
            height: tex.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: u32::try_from(levels.len()).unwrap_or(1),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (w, h, data)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: u32::try_from(level).unwrap_or(0),
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(*h),
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn make_part(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    part: &BakePart,
) -> PartGpu {
    let (vertex, index, ubuf) = part_buffers(device, part);
    let view = base_color_view(device, queue, part.texture.as_ref());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rl64-bake-part-bind"),
        layout,
        entries: &[
            buffer_entry(0, &ubuf),
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    PartGpu {
        vertex,
        index,
        count: u32::try_from(part.indices.len()).unwrap_or(0),
        bind,
    }
}

fn target_texture(
    device: &wgpu::Device,
    label: &str,
    edge: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn uniform_ty(dynamic: bool, bytes: u64) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: dynamic,
        min_binding_size: wgpu::BufferSize::new(bytes),
    }
}

fn texture_ty() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        multisampled: false,
        view_dimension: wgpu::TextureViewDimension::D2,
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
    }
}

fn sampler_ty() -> wgpu::BindingType {
    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
}

/// The bake pipelines and layouts, built once per bake (and per reference
/// frame in the tests): the mesh pass writes albedo plus normal, the blit
/// box-filters the supersampled targets down.
struct BakeKit {
    frame_layout: wgpu::BindGroupLayout,
    part_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    mesh_pipe: wgpu::RenderPipeline,
    blit_pipe: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

/// The two bake targets every bake pass writes: albedo then normal.
fn bake_targets() -> [Option<wgpu::ColorTargetState>; 2] {
    let state = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    [state(ALBEDO_FORMAT), state(NORMAL_FORMAT)]
}

/// The frame, part and blit bind-group layouts of the bake.
fn bake_layouts(
    device: &wgpu::Device,
) -> (
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
) {
    let frag = wgpu::ShaderStages::FRAGMENT;
    let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rl64-bake-frame-layout"),
        entries: &[layout_entry(
            0,
            wgpu::ShaderStages::VERTEX,
            uniform_ty(true, 64),
        )],
    });
    let part = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rl64-bake-part-layout"),
        entries: &[
            layout_entry(0, frag, uniform_ty(false, 32)),
            layout_entry(1, frag, texture_ty()),
            layout_entry(2, frag, sampler_ty()),
        ],
    });
    let blit = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rl64-bake-blit-layout"),
        entries: &[
            layout_entry(1, frag, texture_ty()),
            layout_entry(2, frag, texture_ty()),
            layout_entry(3, frag, sampler_ty()),
        ],
    });
    (frame, part, blit)
}

/// The mesh pass pipeline: one interleaved vertex stream, depth tested.
fn mesh_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    frame_layout: &wgpu::BindGroupLayout,
    part_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rl64-bake-mesh-layout"),
        bind_group_layouts: &[Some(frame_layout), Some(part_layout)],
        immediate_size: 0,
    });
    let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rl64-bake-mesh"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_mesh"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: BAKE_VERTEX_BYTES,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attrs,
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_mesh"),
            targets: &bake_targets(),
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
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
    })
}

/// The downsample pipeline: one fullscreen triangle into both targets.
fn blit_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    blit_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rl64-bake-blit-pipe-layout"),
        bind_group_layouts: &[Some(blit_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rl64-bake-blit"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_blit"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_blit"),
            targets: &bake_targets(),
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl BakeKit {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rl64-impostor-bake"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/impostor_bake.wgsl").into()),
        });
        let (frame_layout, part_layout, blit_layout) = bake_layouts(device);
        let mesh_pipe = mesh_pipeline(device, &shader, &frame_layout, &part_layout);
        let blit_pipe = blit_pipeline(device, &shader, &blit_layout);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rl64-bake-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            frame_layout,
            part_layout,
            blit_layout,
            mesh_pipe,
            blit_pipe,
            sampler,
        }
    }

    /// One bind group over the frame-matrix buffer (a dynamic offset steps it).
    fn frame_bind(&self, device: &wgpu::Device, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rl64-bake-frame-bind"),
            layout: &self.frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(64),
                }),
            }],
        })
    }

    /// The parts of `mesh` on the device.
    fn parts(&self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: &BakeMesh) -> Vec<PartGpu> {
        // A part with no triangles has nothing to draw, and a zero-size buffer
        // cannot be sliced, so it never reaches the device.
        mesh.parts
            .iter()
            .filter(|part| !part.indices.is_empty())
            .map(|part| make_part(device, queue, &self.part_layout, &self.sampler, part))
            .collect()
    }
}

/// Pack the per-frame matrices at the dynamic-offset stride.
fn frame_matrix_bytes(n: u32, centre: [f32; 3], radius: f32) -> Result<Vec<u8>, ImpostorGpuError> {
    let frames = n * n;
    let mut bytes = vec![0u8; usize::try_from(u64::from(frames) * FRAME_STRIDE).unwrap_or(0)];
    for i in 0..frames {
        let vp = frame_view_proj(n, i, centre, radius)?;
        let at = usize::try_from(u64::from(i) * FRAME_STRIDE).unwrap_or(0);
        for (k, c) in vp.iter().enumerate() {
            bytes[at + k * 4..at + k * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    Ok(bytes)
}

fn clear_attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    })
}

/// Record the mesh pass: every cell of an `n` by `n` grid of `cell_px`
/// squares, each with its own matrix, over `parts`.
fn record_frames(
    rp: &mut wgpu::RenderPass<'_>,
    kit: &BakeKit,
    frame_bind: &wgpu::BindGroup,
    parts: &[PartGpu],
    n: u32,
    cell_px: u32,
) {
    rp.set_pipeline(&kit.mesh_pipe);
    let cell = pad_f32(cell_px);
    for i in 0..n * n {
        let (px_x, px_y) = (cell_px * (i % n), cell_px * (i / n));
        rp.set_viewport(pad_f32(px_x), pad_f32(px_y), cell, cell, 0.0, 1.0);
        rp.set_scissor_rect(px_x, px_y, cell_px, cell_px);
        let offset = u32::try_from(u64::from(i) * FRAME_STRIDE).unwrap_or(0);
        rp.set_bind_group(0, frame_bind, &[offset]);
        for part in parts {
            rp.set_bind_group(1, &part.bind, &[]);
            rp.set_vertex_buffer(0, part.vertex.slice(..));
            rp.set_index_buffer(part.index.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..part.count, 0, 0..1);
        }
    }
}

/// The supersampled bake targets and the atlas textures they filter into.
struct BakeTargets {
    hi_albedo: wgpu::TextureView,
    hi_normal: wgpu::TextureView,
    hi_depth: wgpu::TextureView,
    albedo: wgpu::Texture,
    normal: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    normal_view: wgpu::TextureView,
}

impl BakeTargets {
    fn new(device: &wgpu::Device, layout: AtlasLayout) -> Self {
        let hi_px = layout.atlas_px * BAKE_SUPERSAMPLE;
        let render_use =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let hi_albedo = target_texture(device, "rl64-hi-albedo", hi_px, ALBEDO_FORMAT, render_use);
        let hi_normal = target_texture(device, "rl64-hi-normal", hi_px, NORMAL_FORMAT, render_use);
        let hi_depth = target_texture(
            device,
            "rl64-hi-depth",
            hi_px,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let atlas_use = render_use | wgpu::TextureUsages::COPY_SRC;
        let edge = layout.atlas_px;
        let albedo = target_texture(device, "rl64-atlas-albedo", edge, ALBEDO_FORMAT, atlas_use);
        let normal = target_texture(device, "rl64-atlas-normal", edge, NORMAL_FORMAT, atlas_use);
        Self {
            hi_albedo: view(&hi_albedo),
            hi_normal: view(&hi_normal),
            hi_depth: view(&hi_depth),
            albedo_view: view(&albedo),
            normal_view: view(&normal),
            albedo,
            normal,
        }
    }
}

/// Record the box-filter pass from the supersampled targets into the atlas.
fn record_downsample(
    enc: &mut wgpu::CommandEncoder,
    device: &wgpu::Device,
    kit: &BakeKit,
    targets: &BakeTargets,
) {
    let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("rl64-bake-blit-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let view = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rl64-bake-blit-bind"),
        layout: &kit.blit_layout,
        entries: &[
            view(1, &targets.hi_albedo),
            view(2, &targets.hi_normal),
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&blit_sampler),
            },
        ],
    });
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("rl64-bake-downsample"),
        color_attachments: &[
            clear_attachment(&targets.albedo_view),
            clear_attachment(&targets.normal_view),
        ],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(&kit.blit_pipe);
    rp.set_bind_group(0, &blit_bind, &[]);
    rp.draw(0..3, 0..1);
}

/// Bake `mesh` into a fresh atlas of `layout`: one render pass over every
/// frame direction at [`BAKE_SUPERSAMPLE`]x, one box-filter pass into the
/// atlas, then wait for the device (native) and read any validation error.
///
/// # Errors
/// A bad mesh, a bad direction, or a device validation error.
pub fn bake_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mesh: &BakeMesh,
    layout: AtlasLayout,
) -> Result<ImpostorAtlas, ImpostorGpuError> {
    use wgpu::util::DeviceExt as _;
    mesh.validate()?;
    let started = engine_clock::now();
    let (centre, raw_radius) = mesh.bounding_sphere();
    let radius = (raw_radius * SPHERE_MARGIN).max(1e-3);
    let n = layout.frames_per_side;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let frame_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("rl64-bake-frames"),
        contents: &frame_matrix_bytes(n, centre, radius)?,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let kit = BakeKit::new(device);
    let parts = kit.parts(device, queue, mesh);
    let frame_bind = kit.frame_bind(device, &frame_buf);
    let targets = BakeTargets::new(device, layout);

    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rl64-bake"),
    });
    {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl64-bake-frames-pass"),
            color_attachments: &[
                clear_attachment(&targets.hi_albedo),
                clear_attachment(&targets.hi_normal),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.hi_depth,
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
        record_frames(
            &mut rp,
            &kit,
            &frame_bind,
            &parts,
            n,
            layout.frame_px * BAKE_SUPERSAMPLE,
        );
    }
    record_downsample(&mut enc, device, &kit, &targets);
    queue.submit(Some(enc.finish()));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(ImpostorGpuError::Validation(err.to_string()));
        }
    }
    #[cfg(target_arch = "wasm32")]
    drop(scope);
    let bake_ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    Ok(ImpostorAtlas {
        layout,
        centre_m: centre,
        radius_m: radius,
        bake_ms,
        albedo: targets.albedo,
        normal: targets.normal,
        albedo_view: targets.albedo_view,
        normal_view: targets.normal_view,
    })
}

// ---------------------------------------------------------------------------
// Draw: instances, the per-mesh batch and the pass recorder.
// ---------------------------------------------------------------------------

/// One staged impostor: where it stands and how it is dressed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ImpostorInstance {
    /// Sphere centre in world meters (the baked centre, moved to the seat).
    pub centre_m: [f32; 3],
    /// Uniform scale of the sprite, 1 keeps the baked size.
    pub scale: f32,
    /// Linear tunic colour for the parts baked with the tunic mask.
    pub tint: [f32; 3],
    /// Yaw about +y in radians the baked figure is turned by.
    pub yaw_rad: f32,
}

impl ImpostorInstance {
    fn pack(&self, out: &mut Vec<u8>) {
        for c in self.centre_m {
            put(out, c);
        }
        put(out, self.scale);
        for c in self.tint {
            put(out, c);
        }
        put(out, self.yaw_rad);
    }

    fn finite(&self) -> bool {
        self.centre_m.iter().all(|c| c.is_finite())
            && self.tint.iter().all(|c| c.is_finite())
            && self.scale.is_finite()
            && self.yaw_rad.is_finite()
    }
}

/// The pass shape a draw records into; the pipeline follows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PassShape {
    /// The 1x forward pass: HDR, id and motion targets plus depth.
    Forward1x,
    /// The 4x forward pass: one HDR target plus depth.
    ForwardMsaa,
    /// One colour target of this format plus depth (tests and twins).
    Single(wgpu::TextureFormat),
}

/// The frame buffers the draw reads: the presenter owns them.
#[derive(Clone, Copy)]
pub struct FrameBuffers<'a> {
    /// Jittered camera: view-projection then eye (80 bytes).
    pub cam: &'a wgpu::Buffer,
    /// Sun direction and radiance (32 bytes).
    pub light: &'a wgpu::Buffer,
    /// Height fog (128 bytes).
    pub fog: &'a wgpu::Buffer,
    /// Baked-ambient SH (144 bytes).
    pub sh: &'a wgpu::Buffer,
    /// Previous frame's view-projection (64 bytes).
    pub prev_cam: &'a wgpu::Buffer,
    /// This frame's un-jittered view-projection (64 bytes).
    pub steady_cam: &'a wgpu::Buffer,
}

struct Batch {
    key: u64,
    atlas: ImpostorAtlas,
    lit: bool,
    item: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
    bind: Option<wgpu::BindGroup>,
    scratch: Vec<u8>,
}

/// Counters for the frame-split line and the receipts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ImpostorStats {
    /// Baked meshes resident.
    pub meshes: u32,
    /// Instances staged for the coming frame.
    pub instances: u32,
    /// Draw calls the coming frame records.
    pub draws: u32,
    /// Atlas bytes resident (albedo plus normal).
    pub atlas_bytes: u64,
}

/// Baked meshes, their instance batches and the draw pipelines.
#[derive(Default)]
pub struct ImpostorSubmitter {
    batches: Vec<Batch>,
    pipelines: Vec<(PassShape, wgpu::RenderPipeline)>,
    shader: Option<wgpu::ShaderModule>,
    layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
}

impl ImpostorSubmitter {
    /// An empty submitter: nothing baked, nothing drawn.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bake `mesh` under `key` unless an atlas of that key and `layout` is
    /// already resident (cached by mesh id and layout). Returns true when a
    /// bake ran.
    ///
    /// # Errors
    /// The bake's errors.
    pub fn bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        mesh: &BakeMesh,
        layout: AtlasLayout,
        lit: bool,
    ) -> Result<bool, ImpostorGpuError> {
        use wgpu::util::DeviceExt as _;
        if self
            .batches
            .iter()
            .any(|b| b.key == key && b.atlas.layout == layout)
        {
            return Ok(false);
        }
        let atlas = bake_atlas(device, queue, mesh, layout)?;
        let mut item = Vec::with_capacity(ITEM_BYTES);
        put(&mut item, atlas.radius_m);
        put_u32(&mut item, layout.frames_per_side);
        put_u32(&mut item, layout.frame_px);
        put(&mut item, CUTOFF);
        put(&mut item, if lit { 1.0 } else { 0.0 });
        for _ in 0..3 {
            put(&mut item, 0.0);
        }
        let item = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rl64-impostor-item"),
            contents: &item,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let instances = Self::instance_buffer(device, 1);
        let batch = Batch {
            key,
            atlas,
            lit,
            item,
            instances,
            capacity: 1,
            count: 0,
            bind: None,
            scratch: Vec::new(),
        };
        self.batches.retain(|b| b.key != key);
        self.batches.push(batch);
        Ok(true)
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl64-impostor-instances"),
            size: u64::try_from(capacity.max(1) * IMPOSTOR_INSTANCE_BYTES).unwrap_or(0),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Replace the staged instances of the mesh baked under `key`. The buffer
    /// doubles when it must grow and is rewritten in place otherwise; an
    /// empty list clears the batch. An unknown key stages nothing.
    ///
    /// # Errors
    /// More than [`MAX_IMPOSTOR_INSTANCES`] instances.
    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        list: &[ImpostorInstance],
    ) -> Result<(), ImpostorGpuError> {
        if list.len() > MAX_IMPOSTOR_INSTANCES {
            return Err(ImpostorGpuError::TooManyInstances {
                requested: list.len(),
                allowed: MAX_IMPOSTOR_INSTANCES,
            });
        }
        let Some(batch) = self.batches.iter_mut().find(|b| b.key == key) else {
            return Ok(());
        };
        let live: Vec<&ImpostorInstance> = list.iter().filter(|i| i.finite()).collect();
        batch.count = u32::try_from(live.len()).unwrap_or(0);
        if live.is_empty() {
            return Ok(());
        }
        if live.len() > batch.capacity {
            batch.capacity = live.len().next_power_of_two();
            batch.instances = Self::instance_buffer(device, batch.capacity);
            batch.bind = None;
        }
        batch.scratch.clear();
        for inst in live {
            inst.pack(&mut batch.scratch);
        }
        queue.write_buffer(&batch.instances, 0, &batch.scratch);
        Ok(())
    }

    /// Stats for the coming frame.
    #[must_use]
    pub fn stats(&self) -> ImpostorStats {
        ImpostorStats {
            meshes: u32::try_from(self.batches.len()).unwrap_or(u32::MAX),
            instances: self.batches.iter().map(|b| b.count).sum(),
            draws: u32::try_from(self.batches.iter().filter(|b| b.count > 0).count())
                .unwrap_or(u32::MAX),
            atlas_bytes: self.batches.iter().map(|b| b.atlas.layout.bytes).sum(),
        }
    }

    /// True while the coming frame has at least one instance to draw.
    #[must_use]
    pub fn has_draw(&self) -> bool {
        self.batches.iter().any(|b| b.count > 0)
    }

    /// The baked atlas under `key`, if resident.
    #[must_use]
    pub fn atlas(&self, key: u64) -> Option<&ImpostorAtlas> {
        self.batches.iter().find(|b| b.key == key).map(|b| &b.atlas)
    }

    /// Build what the coming pass needs: the shader, the layouts, the
    /// pipeline of `shape` and a bind group per live batch over the frame's
    /// buffers. Cheap and allocation-free once warm.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        bufs: &FrameBuffers<'_>,
        shapes: &[PassShape],
    ) {
        if !self.has_draw() {
            return;
        }
        let shader = self.shader.get_or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rl64-impostor-draw"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/impostor.wgsl").into()),
            })
        });
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let frag = wgpu::ShaderStages::FRAGMENT;
        let layout = self.layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rl64-impostor-layout"),
                entries: &[
                    layout_entry(0, both, uniform_ty(false, 80)),
                    layout_entry(1, frag, uniform_ty(false, 32)),
                    layout_entry(2, both, uniform_ty(false, 128)),
                    layout_entry(3, frag, uniform_ty(false, 144)),
                    layout_entry(4, wgpu::ShaderStages::VERTEX, uniform_ty(false, 64)),
                    layout_entry(5, wgpu::ShaderStages::VERTEX, uniform_ty(false, 64)),
                    layout_entry(6, both, uniform_ty(false, 32)),
                    layout_entry(7, frag, texture_ty()),
                    layout_entry(8, frag, texture_ty()),
                    layout_entry(9, frag, sampler_ty()),
                    layout_entry(
                        10,
                        wgpu::ShaderStages::VERTEX,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(32),
                        },
                    ),
                ],
            })
        });
        let sampler = self.sampler.get_or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("rl64-impostor-sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        });
        for shape in shapes {
            if !self.pipelines.iter().any(|(s, _)| s == shape) {
                self.pipelines
                    .push((*shape, draw_pipeline(device, shader, layout, *shape)));
            }
        }
        for batch in &mut self.batches {
            if batch.count == 0 {
                continue;
            }
            // The frame buffers are the presenter's and may be rebuilt, so the
            // bind group is cheap to remake each frame.
            batch.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl64-impostor-bind"),
                layout,
                entries: &[
                    buffer_entry(0, bufs.cam),
                    buffer_entry(1, bufs.light),
                    buffer_entry(2, bufs.fog),
                    buffer_entry(3, bufs.sh),
                    buffer_entry(4, bufs.prev_cam),
                    buffer_entry(5, bufs.steady_cam),
                    buffer_entry(6, &batch.item),
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&batch.atlas.albedo_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&batch.atlas.normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    buffer_entry(10, &batch.instances),
                ],
            }));
        }
    }

    /// Record every live batch into `pass`: one draw of six vertices per
    /// instance, no vertex buffer. The pass must match `shape` (its colour
    /// targets, a Depth32Float attachment); [`Self::prepare`] ran first.
    pub fn encode_draw(&self, pass: &mut wgpu::RenderPass<'_>, shape: PassShape) {
        let Some((_, pipeline)) = self.pipelines.iter().find(|(s, _)| *s == shape) else {
            return;
        };
        pass.set_pipeline(pipeline);
        for batch in &self.batches {
            let Some(bind) = batch.bind.as_ref().filter(|_| batch.count > 0) else {
                continue;
            };
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..6, 0..batch.count);
        }
    }

    /// Whether the batch under `key` shades with the sun and ambient.
    #[must_use]
    pub fn is_lit(&self, key: u64) -> Option<bool> {
        self.batches.iter().find(|b| b.key == key).map(|b| b.lit)
    }
}

fn draw_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    shape: PassShape,
) -> wgpu::RenderPipeline {
    let single;
    let (formats, entry, samples): (&[wgpu::TextureFormat], &str, u32) = match shape {
        PassShape::Forward1x => (
            &[HDR_FORMAT, wgpu::TextureFormat::R32Uint, MOTION_FORMAT],
            "fs_main_gbuf",
            1,
        ),
        PassShape::ForwardMsaa => (&[HDR_FORMAT], "fs_main", 4),
        PassShape::Single(format) => {
            single = [format];
            (&single, "fs_main", 1)
        }
    };
    let mut slots: [Option<wgpu::ColorTargetState>; 3] = [None, None, None];
    for (slot, format) in slots.iter_mut().zip(formats) {
        *slot = Some(wgpu::ColorTargetState {
            format: *format,
            blend: if *format == wgpu::TextureFormat::R32Uint {
                None
            } else {
                Some(wgpu::BlendState::REPLACE)
            },
            write_mask: wgpu::ColorWrites::ALL,
        });
    }
    let targets = &slots[..formats.len()];
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rl64-impostor-pipe-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rl64-impostor-draw"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            targets,
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
#[path = "impostor_gpu_tests.rs"]
mod impostor_gpu_tests;
