//! particle_draw_gpu: staged draw bytes and device builders for the flipbook particle draw.
//!
//! Owner: render lane; split from `particles_gpu`, which keeps the pool, the draw
//! state and the submitter. Window build only. Units: bytes in the WGSL layout,
//! meters in the packed fields. Determinism: fixed field order, no wall clock.
//! Errors: none here; device validation surfaces through the submitter.
//! Budget: stack bytes for packing; the builders run once per draw state.

use super::{
    DrawRecord, PARTICLE_DRAW_WGSL, POOL_CAP_HARD, SOFT_FADE_RANGE_M, STRETCH_GAIN, STRETCH_MAX,
};
use crate::vfx_soft::{vfx_depth_layout, vfx_pack_frame, VfxFrame, VFX_FRAME_BYTES};

// ---------------------------------------------------------------------------
// Staged draw bytes (behind `window`): the CPU image of
// `shaders/particle.wgsl`, one instance per [`DrawRecord`], plus its
// per-frame camera uniform. The packing plus the device builders live with
// the frame pass in `present::gpu_present::submit`; this module owns the
// layout contract and the draw state itself.
// ---------------------------------------------------------------------------

/// Draw-uniform byte size for `shaders/particle.wgsl` (`DrawU`): one `mat4`
/// view-projection, the camera right and up axes, the atlas grid and the
/// fade plus stretch bars (112 bytes: WGSL puts every `vec3` member on a
/// 16-byte bound, so the fields that add up to 108 pad out), then the 32-byte
/// [`VfxFrame`] block (depth constants, depth-valid word, light).
pub const PARTICLE_DRAW_UNIFORM_BYTES: usize = 144;

/// Byte offset of the [`VfxFrame`] block inside the draw uniform.
pub(super) const PARTICLE_VFX_AT: usize = PARTICLE_DRAW_UNIFORM_BYTES - VFX_FRAME_BYTES;

/// [`PARTICLE_VFX_AT`] and the depth-valid word eight bytes in, as buffer
/// addresses for the queue writes (a test pins both to the layout).
pub(super) const PARTICLE_VFX_ADDR: u64 = 112;
pub(super) const PARTICLE_DEPTH_ON_ADDR: u64 = 120;

/// One packed draw instance: the `DrawP` image of `shaders/particle.wgsl`
/// (position, size, velocity, alpha, the two flipbook cells, the blend
/// weight, the emitter kind, then the tint on a 16-byte bound).
pub const PARTICLE_DRAW_INSTANCE_BYTES: usize = 64;

/// Instances one staged batch carries at most; the pool's hard bound sizes
/// it, so the instance buffer is sized once and never resizes on a frame,
/// and no tier (up to the hard cap) can outgrow it.
pub(super) const PARTICLE_DRAW_MAX_INSTANCES: usize = POOL_CAP_HARD;

/// Flipbook atlas grid the draw uniform binds. The Kenney sprite set is
/// descriptors only (`content/vfx`); the shipped stand-in atlas is one white
/// texel, so every cell samples 1.0 and the per-kind Kenney grids land with
/// the sprite bytes.
const PARTICLE_ATLAS_GRID: [u32; 2] = [1, 1];

/// Append one little-endian `f32` at `*at`; stack bytes only.
fn particle_put_f32(out: &mut [u8], at: &mut usize, v: f32) {
    out[*at..*at + 4].copy_from_slice(&v.to_le_bytes());
    *at += 4;
}

/// Append one little-endian `u32` at `*at`; stack bytes only.
fn particle_put_u32(out: &mut [u8], at: &mut usize, v: u32) {
    out[*at..*at + 4].copy_from_slice(&v.to_le_bytes());
    *at += 4;
}

/// Pack the `DrawU` image: the frame view-projection in column-major order
/// (the image the forward camera uniform carries), the billboard axes, the
/// atlas grid and the fade plus stretch bars from [`SOFT_FADE_RANGE_M`],
/// [`STRETCH_GAIN`] and [`STRETCH_MAX`], then the [`VfxFrame`] block the soft
/// contact and the lit sprites read. `soft_on` packs as 0: the staged
/// [`DrawRecord::fade`] rides the instance alpha, and the scene-depth contact
/// is the shader's own (`depth_on` in the block).
/// Deterministic: fixed field order, no wall clock.
pub(super) fn particle_pack_uniform(
    view_proj: [[f32; 4]; 4],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    vfx: &VfxFrame,
) -> [u8; PARTICLE_DRAW_UNIFORM_BYTES] {
    let mut out = [0u8; PARTICLE_DRAW_UNIFORM_BYTES];
    let mut at = 0usize;
    for col in view_proj {
        for v in col {
            particle_put_f32(&mut out, &mut at, v);
        }
    }
    for v in cam_right {
        particle_put_f32(&mut out, &mut at, v);
    }
    particle_put_f32(&mut out, &mut at, SOFT_FADE_RANGE_M);
    for v in cam_up {
        particle_put_f32(&mut out, &mut at, v);
    }
    particle_put_f32(&mut out, &mut at, STRETCH_GAIN);
    particle_put_u32(&mut out, &mut at, PARTICLE_ATLAS_GRID[0]);
    particle_put_u32(&mut out, &mut at, PARTICLE_ATLAS_GRID[1]);
    particle_put_f32(&mut out, &mut at, STRETCH_MAX);
    particle_put_u32(&mut out, &mut at, 0);
    // The `soft_on` word closes the 112-byte head; the block starts on 16.
    debug_assert_eq!(at, 112);
    vfx_pack_frame(vfx, &mut out[PARTICLE_VFX_AT..]);
    out
}

/// Pack one staged [`DrawRecord`] into the `DrawP` image: `pos`, `size_m`,
/// both flipbook cells, `tint` and the blend weight cross over; the alpha slot
/// carries the staged [`DrawRecord::fade`], so the soft fade the CPU staged
/// reaches the blend, and the blend weight is `blend_b` (`mix(a, b, blend_b)`
/// equals `a * blend_a + b * blend_b` once the pair sums to one). The velocity
/// slot carries the particle's velocity (the shader streaks sparks and blood
/// along it), and the word after the blend weight is the emitter kind the
/// shader shapes the sprite by. Deterministic: fixed order.
pub(super) fn particle_pack_instance(record: &DrawRecord, out: &mut [u8]) {
    debug_assert_eq!(out.len(), PARTICLE_DRAW_INSTANCE_BYTES);
    let mut at = 0usize;
    for v in [record.pos[0], record.pos[1], record.pos[2], record.size_m] {
        particle_put_f32(out, &mut at, v);
    }
    for v in record.vel {
        particle_put_f32(out, &mut at, v);
    }
    particle_put_f32(out, &mut at, record.fade);
    particle_put_u32(out, &mut at, record.frame_a);
    particle_put_u32(out, &mut at, record.frame_b);
    particle_put_f32(out, &mut at, record.blend_b);
    particle_put_u32(out, &mut at, u32::from(record.kind));
    for v in record.tint {
        particle_put_f32(out, &mut at, v);
    }
    debug_assert_eq!(at, PARTICLE_DRAW_INSTANCE_BYTES);
}

/// Draw bind-group layout: the view uniform, the instance array and the
/// flipbook atlas plus its filtering sampler. The scene depth rides group 1
/// (`vfx_soft::vfx_depth_layout`) so the fragment fades against it.
pub(super) fn particle_draw_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = || wgpu::BufferBindingType::Storage { read_only: true };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rp2-particle-draw-layout"),
        entries: &[
            // The fragment entry reads `draw.light` and `draw.depth_on` too.
            particle_draw_buffer_entry(
                0,
                wgpu::ShaderStages::VERTEX_FRAGMENT,
                wgpu::BufferBindingType::Uniform,
            ),
            particle_draw_buffer_entry(1, wgpu::ShaderStages::VERTEX, storage()),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// The flipbook pipeline: [`PARTICLE_DRAW_WGSL`] `draw_vs` plus `draw_fs`, no
/// vertex buffers (the shader reads the instance array), one HDR colour target
/// blended premultiplied (each kind picks how much of its light adds) and no
/// depth attachment, so no pass shape can mismatch: the shader fades against
/// the stored depth itself (group 1, `vfx_soft`). The shader lays the quad out
/// as corners 0..3, so the strip closes it with two triangles.
pub(super) fn particle_draw_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("rp2-particle-draw"),
        source: wgpu::ShaderSource::Wgsl(PARTICLE_DRAW_WGSL.into()),
    });
    let depth = vfx_depth_layout(device);
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rp2-particle-draw-pipe-layout"),
        bind_group_layouts: &[Some(layout), Some(&depth)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rp2-particle-draw-pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("draw_vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("draw_fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::present::HDR_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// One device buffer with `usage` plus the copy destination the batch uploads
/// need.
pub(super) fn particle_draw_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Buffer bind-group layout entry at `binding` for `stage`.
fn particle_draw_buffer_entry(
    binding: u32,
    stage: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// The stand-in flipbook atlas: one white texel, so every cell samples 1.0
/// until the Kenney sprite bytes land (`content/vfx` holds descriptors only),
/// with the view the bind group samples.
pub(super) fn particle_draw_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rp2-particle-draw-atlas"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &atlas,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255u8; 4],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
    (atlas, view)
}
