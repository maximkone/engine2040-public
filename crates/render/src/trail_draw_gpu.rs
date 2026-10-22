//! trail_draw_gpu: staged draw bytes and device builders for the sword ribbon trail.
//!
//! Owner: render lane; split from `trail`, which keeps the CPU ring. Window build only.
//! Units: bytes in the WGSL layout, meters in the vert positions. Determinism: fixed
//! field order, no wall clock. Errors: none here; device validation surfaces through
//! the submitter. Budget: stack bytes for packing; the builders run once per draw state.

use crate::draw::TrailStripVert;

// ---------------------------------------------------------------------------
// Staged draw bytes (behind `window`): the CPU image of
// `shaders/trail.wgsl`, one uniform per frame plus the strip storage buffer.
// The packing plus the device builders live with the frame pass in
// `present::gpu_present::submit`; this module owns the layout contract.
// ---------------------------------------------------------------------------

/// Draw-uniform byte size for `shaders/trail.wgsl` (`TrailU`): one `mat4`
/// view-projection, the camera right and up axes, and the width scale.
/// WGSL puts every `vec3` member on a 16-byte bound, so the image is 96 bytes
/// where the fields add up to 88.
pub const TRAIL_DRAW_UNIFORM_BYTES: usize = 96;

/// One staged vert: the `StripVert` image of `shaders/trail.wgsl` (position
/// plus alpha). 16 bytes per vert, so the storage buffer is 16 * vert_count.
pub const TRAIL_DRAW_VERT_BYTES: usize = 16;

/// Kenney ribbon draw shader over the fixed 16-sample ring; pool bounds and
/// the seeded stream are untouched. The draw path is the `shaders/trail.wgsl`
/// file; `TrailSubmitter` stages the live strip and the frame records it.
/// Engine-owned WGSL, fixed math only, no wall clock.
pub const TRAIL_DRAW_WGSL: &str = include_str!("shaders/trail.wgsl");

/// Append one little-endian `f32` at `*at`; stack bytes only.
fn trail_put_f32(out: &mut [u8], at: &mut usize, v: f32) {
    out[*at..*at + 4].copy_from_slice(&v.to_le_bytes());
    *at += 4;
}

/// Pack the `TrailU` image: the frame view-projection in column-major order
/// (the image the forward camera uniform carries), the camera right and up
/// axes, and the width scale. Deterministic: fixed field order, no wall clock.
pub fn trail_pack_uniform(
    view_proj: [[f32; 4]; 4],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    width_scale: f32,
) -> [u8; TRAIL_DRAW_UNIFORM_BYTES] {
    let mut out = [0u8; TRAIL_DRAW_UNIFORM_BYTES];
    let mut at = 0usize;
    for col in view_proj {
        for v in col {
            trail_put_f32(&mut out, &mut at, v);
        }
    }
    for v in cam_right {
        trail_put_f32(&mut out, &mut at, v);
    }
    trail_put_f32(&mut out, &mut at, width_scale);
    for v in cam_up {
        trail_put_f32(&mut out, &mut at, v);
    }
    // Pad to 16-byte alignment (vec3 is 16 bytes in WGSL).
    trail_put_f32(&mut out, &mut at, 0.0);
    debug_assert_eq!(at, TRAIL_DRAW_UNIFORM_BYTES);
    out
}

/// Pack one staged [`TrailStripVert`] into the `StripVert` image: `pos` and
/// `alpha`. Deterministic: fixed order.
pub fn trail_pack_vert(vert: &TrailStripVert, out: &mut [u8]) {
    debug_assert_eq!(out.len(), TRAIL_DRAW_VERT_BYTES);
    let mut at = 0usize;
    for v in vert.pos_m {
        trail_put_f32(out, &mut at, v);
    }
    trail_put_f32(out, &mut at, vert.alpha);
    debug_assert_eq!(at, TRAIL_DRAW_VERT_BYTES);
}

/// Pack every staged vert into a contiguous byte buffer; one [`TRAIL_DRAW_VERT_BYTES`]
/// block per vert, in order. Deterministic: fixed order.
pub fn trail_pack_strip(verts: &[TrailStripVert]) -> Vec<u8> {
    let mut out = vec![0u8; verts.len() * TRAIL_DRAW_VERT_BYTES];
    for (i, vert) in verts.iter().enumerate() {
        let slice = &mut out[i * TRAIL_DRAW_VERT_BYTES..(i + 1) * TRAIL_DRAW_VERT_BYTES];
        trail_pack_vert(vert, slice);
    }
    out
}

/// Draw bind-group layout: the view uniform and the strip storage buffer.
/// Fixed shape, never rebuilt.
pub fn trail_draw_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = || wgpu::BufferBindingType::Storage { read_only: true };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rl53-trail-draw-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(TRAIL_DRAW_UNIFORM_BYTES as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: storage(),
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(TRAIL_DRAW_VERT_BYTES as u64),
                },
                count: None,
            },
        ],
    })
}

/// Draw pipeline layout: one bind-group layout (the uniform and the strip).
pub fn trail_draw_pipeline_layout(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rl53-trail-draw-pipeline-layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    })
}

/// Draw render pipeline: triangle strip topology, one vertex shader reading
/// the strip storage, one fragment shader outputting `vec4<f32>(1.0, 1.0, 1.0, alpha)`.
pub fn trail_draw_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rl53-trail-draw-pipeline"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("trail_vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("trail_fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
#[cfg(feature = "window")]
pub fn trail_draw_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trail_pack_uniform_is_deterministic() {
        let view_proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let cam_right = [1.0, 0.0, 0.0];
        let cam_up = [0.0, 1.0, 0.0];
        let width_scale = 1.5;
        let a = trail_pack_uniform(view_proj, cam_right, cam_up, width_scale);
        let b = trail_pack_uniform(view_proj, cam_right, cam_up, width_scale);
        assert_eq!(a, b, "the same inputs pack the same uniform bytes");
        assert_eq!(a.len(), TRAIL_DRAW_UNIFORM_BYTES);
    }

    #[test]
    fn trail_pack_strip_is_deterministic() {
        let verts = vec![
            TrailStripVert {
                pos_m: [0.0, 0.0, 0.0],
                alpha: 0.0,
            },
            TrailStripVert {
                pos_m: [1.0, 0.0, 0.0],
                alpha: 1.0,
            },
        ];
        let a = trail_pack_strip(&verts);
        let b = trail_pack_strip(&verts);
        assert_eq!(a, b, "the same verts pack the same strip bytes");
        assert_eq!(a.len(), verts.len() * TRAIL_DRAW_VERT_BYTES);
    }
}
