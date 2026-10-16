//! sun_shadow: single arena sun depth map from live draws.
//! Owner: render lane via submit slice 2. Units: meters, texels, 0..1 depths.
use std::fmt;

use crate::arena_day::ARENA_DAY_SUN_DIR;

/// Sun shadow failures. Bad matrix or shape fails loudly.
#[derive(Clone, PartialEq, Debug)]
pub enum SunShadowError {
    Invalid(String),
}
impl fmt::Display for SunShadowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "sun shadow input: {e}"),
        }
    }
}
impl std::error::Error for SunShadowError {}
/// Sun map edge in texels: 2048 over the pit.
pub const SUN_SHADOW_SIZE: u32 = 2048;
/// Float edge matching [`SUN_SHADOW_SIZE`] for UV math.
pub const SUN_SHADOW_SIZE_F32: f32 = 2048.0;
/// Depth bias: `frag - BIAS <= stored` is lit.
pub const SUN_SHADOW_BIAS: f32 = 0.002;
/// Ortho box center in meters: the pit center.
pub const SUN_SHADOW_CENTER_M: [f32; 3] = [0.0, 0.0, 0.0];
/// Ortho half-extent in meters: 30 m span over the pit.
pub const SUN_SHADOW_HALF_EXTENT_M: f32 = 15.0;
/// Sun camera near in meters from the sun eye.
pub const SUN_SHADOW_NEAR_M: f32 = 0.5;
/// Sun camera far in meters from the sun eye.
pub const SUN_SHADOW_FAR_M: f32 = 60.0;
fn sun_light_basis(sun: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let fwd = [-sun[0], -sun[1], -sun[2]];
    let helper: [f32; 3] = if fwd[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let mut t = [
        helper[1] * fwd[2] - helper[2] * fwd[1],
        helper[2] * fwd[0] - helper[0] * fwd[2],
        helper[0] * fwd[1] - helper[1] * fwd[0],
    ];
    let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-12);
    t = [t[0] / tl, t[1] / tl, t[2] / tl];
    let u = [
        fwd[1] * t[2] - fwd[2] * t[1],
        fwd[2] * t[0] - fwd[0] * t[2],
        fwd[0] * t[1] - fwd[1] * t[0],
    ];
    (t, u, fwd)
}

/// Arena sun view-projection for this frame: ortho camera on the pit.
pub fn arena_sun_view_proj() -> Result<[[f32; 4]; 4], SunShadowError> {
    let sun_dir = ARENA_DAY_SUN_DIR;
    if !sun_dir.iter().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun dir is not finite".into()));
    }
    let len2 = sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2];
    if len2 < 1e-12 {
        return Err(SunShadowError::Invalid("sun dir has ~zero length".into()));
    }
    let len = len2.sqrt();
    let sun = [sun_dir[0] / len, sun_dir[1] / len, sun_dir[2] / len];
    let center = SUN_SHADOW_CENTER_M;
    let half = SUN_SHADOW_HALF_EXTENT_M;
    let span = SUN_SHADOW_FAR_M - SUN_SHADOW_NEAR_M;
    let (t, u, _) = sun_light_basis(sun);
    let dot_c = center[0] * sun[0] + center[1] * sun[1] + center[2] * sun[2];
    let dot_ct = center[0] * t[0] + center[1] * t[1] + center[2] * t[2];
    let dot_cu = center[0] * u[0] + center[1] * u[1] + center[2] * u[2];
    let w3 = (half + dot_c - SUN_SHADOW_NEAR_M) / span;
    let m = [
        [t[0] / half, u[0] / half, -sun[0] / span, 0.0],
        [t[1] / half, u[1] / half, -sun[1] / span, 0.0],
        [t[2] / half, u[2] / half, -sun[2] / span, 0.0],
        [-dot_ct / half, -dot_cu / half, w3, 1.0],
    ];
    if !m.iter().flatten().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun view is not finite".into()));
    }
    Ok(m)
}
/// Depth descriptor: square 2048 `Depth32Float`, attachment plus binding.
pub fn sun_shadow_depth_desc() -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("sun-shadow-depth"),
        size: wgpu::Extent3d {
            width: SUN_SHADOW_SIZE,
            height: SUN_SHADOW_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }
}
#[cfg(test)]
const SUN_SINGLE_DEPTH_WGSL: &str = r#"
struct SunU {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> sun: SunU;
@vertex
fn vs(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return sun.view_proj * vec4<f32>(pos, 1.0);
}
"#;
/// Uniform bytes: 80 bytes, matrix first (64), then `bias` and `scale`.
pub fn sun_shadow_uniform_bytes(vp: [[f32; 4]; 4], bias: f32, scale: f32) -> [u8; 80] {
    let mut out = [0u8; 80];
    for (ci, col) in vp.iter().enumerate() {
        for (ri, c) in col.iter().enumerate() {
            out[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    out[64..68].copy_from_slice(&bias.to_le_bytes());
    out[68..72].copy_from_slice(&scale.to_le_bytes());
    out
}
fn sun_uv_depth(
    view_proj: [[f32; 4]; 4],
    point: [f32; 3],
) -> Result<Option<[f32; 3]>, SunShadowError> {
    if !view_proj.iter().flatten().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun view is not finite".into()));
    }
    if !point.iter().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun point is not finite".into()));
    }
    let mut clip = [0.0; 3];
    for i in 0..3 {
        clip[i] = view_proj[0][i] * point[0]
            + view_proj[1][i] * point[1]
            + view_proj[2][i] * point[2]
            + view_proj[3][i];
    }
    if !clip.iter().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun clip is not finite".into()));
    }
    let u = clip[0] * 0.5 + 0.5;
    let v = clip[1] * 0.5 + 0.5;
    let depth = clip[2];
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) || !(0.0..=1.0).contains(&depth) {
        return Ok(None);
    }
    Ok(Some([u, v, depth]))
}
fn sun_shadow_factor(stored: f32, frag: f32, bias: f32) -> Result<f32, SunShadowError> {
    if !stored.is_finite() || !frag.is_finite() || !bias.is_finite() {
        return Err(SunShadowError::Invalid("sun compare is not finite".into()));
    }
    if frag - bias <= stored {
        Ok(1.0)
    } else {
        Ok(0.0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sun_size_is_2048() {
        assert_eq!(SUN_SHADOW_SIZE, 2048, "sun edge stays 2048");
        assert_eq!(SUN_SHADOW_SIZE_F32, 2048.0, "float edge matches");
        assert_eq!(SUN_SHADOW_HALF_EXTENT_M, 15.0, "ortho spans 30 m");
        let desc = sun_shadow_depth_desc();
        assert_eq!(desc.size.width, 2048, "desc width stays 2048");
        assert_eq!(desc.size.height, 2048, "desc height stays 2048");
        assert_eq!(desc.format, wgpu::TextureFormat::Depth32Float);
    }
    #[test]
    fn sun_single_box_is_nearer_than_plane() {
        let m = arena_sun_view_proj().expect("arena sun builds");
        let n = arena_sun_view_proj().expect("builds twice");
        assert_eq!(m, n);
        let Some([_, _, top]) = sun_uv_depth(m, [2.0, 1.8, 0.0]).expect("box projects") else {
            panic!("box top sits inside the arena map");
        };
        let Some([_, _, ground]) = sun_uv_depth(m, [2.0, 0.0, 0.0]).expect("ground projects")
        else {
            panic!("ground sits inside the arena map");
        };
        assert!(top < ground, "box nearer than plane: {top} vs {ground}");
        let s = sun_shadow_factor(top, ground, SUN_SHADOW_BIAS).expect("behind");
        assert_eq!(s, 0.0);
        let l = sun_shadow_factor(ground, top, SUN_SHADOW_BIAS).expect("above");
        assert_eq!(l, 1.0);
        // Offscreen GPU depth order from the pit camera (EG11 slice 1 proof).
        // Small test size for speed; the 2048 shape is pinned above.
        let Some((device, queue, _)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED sun single-map gpu: no wgpu adapter on this machine");
            return;
        };
        let test_size: u32 = 64;
        let mut matrix_bytes = [0u8; 64];
        for (ci, col) in m.iter().enumerate() {
            for (ri, c) in col.iter().enumerate() {
                matrix_bytes[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4]
                    .copy_from_slice(&c.to_le_bytes());
            }
        }
        let plane: [[f32; 3]; 4] = [
            [-15.0, 0.0, -15.0],
            [15.0, 0.0, -15.0],
            [15.0, 0.0, 15.0],
            [-15.0, 0.0, 15.0],
        ];
        let box_top: [[f32; 3]; 4] = [
            [1.5, 1.8, -0.5],
            [2.5, 1.8, -0.5],
            [2.5, 1.8, 0.5],
            [1.5, 1.8, 0.5],
        ];
        let to_bytes = |verts: &[[f32; 3]; 4]| {
            let mut out = Vec::with_capacity(48);
            for p in verts {
                for c in p {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out
        };
        let plane_bytes = to_bytes(&plane);
        let box_bytes = to_bytes(&box_top);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun-single-uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, &matrix_bytes);
        let make_vert = |bytes: &[u8]| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-single-vert"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, bytes);
            buf
        };
        let plane_vb = make_vert(&plane_bytes);
        let box_vb = make_vert(&box_bytes);
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let mut index_bytes = [0u8; 24];
        for (i, w) in indices.iter().enumerate() {
            index_bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let make_index = || {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-single-index"),
                size: 24,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &index_bytes);
            buf
        };
        let plane_ib = make_index();
        let box_ib = make_index();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun-single-depth"),
            source: wgpu::ShaderSource::Wgsl(SUN_SINGLE_DEPTH_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun-single-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun-single-pipe-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sun-single-pipeline"),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun-single-bind"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun-single-test-depth"),
            size: wgpu::Extent3d {
                width: test_size,
                height: test_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let started = std::time::Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut last: Option<Vec<f32>> = None;
        let (box_u, box_v) = {
            let Some([u, v, _]) = sun_uv_depth(m, [2.0, 1.8, 0.0]).expect("box projects") else {
                panic!("box top sits inside the arena map");
            };
            (u, v)
        };
        let (ground_u, ground_v) = {
            let Some([u, v, _]) = sun_uv_depth(m, [2.0, 0.0, 0.0]).expect("ground projects") else {
                panic!("ground sits inside the arena map");
            };
            (u, v)
        };
        for _ in 0..2 {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sun-single-encode"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sun-single-depth-pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.set_vertex_buffer(0, plane_vb.slice(..));
                pass.set_index_buffer(plane_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
                pass.set_vertex_buffer(0, box_vb.slice(..));
                pass.set_index_buffer(box_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            }
            let padded = test_size.div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-single-readback"),
                size: u64::from(padded) * u64::from(test_size),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(test_size),
                    },
                },
                wgpu::Extent3d {
                    width: test_size,
                    height: test_size,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("readback poll holds");
            rx.recv()
                .expect("readback channel holds")
                .expect("readback maps");
            let w = usize::try_from(test_size).expect("test size fits");
            let mut depths = vec![0.0f32; w * w];
            {
                let mapped = slice.get_mapped_range().expect("mapped range holds");
                let stride = usize::try_from(padded).expect("padded fits");
                for y in 0..w {
                    for x in 0..w {
                        let off = y * stride + x * 4;
                        depths[y * w + x] = f32::from_le_bytes([
                            mapped[off],
                            mapped[off + 1],
                            mapped[off + 2],
                            mapped[off + 3],
                        ]);
                    }
                }
            }
            readback.unmap();
            if let Some(prev) = last.replace(depths.clone()) {
                assert_eq!(prev, depths, "depth bytes x2 equal");
                let sample = |u: f32, v: f32| {
                    let xi = ((u * test_size as f32).floor() as usize).min(w - 1);
                    let yi = ((v * test_size as f32).floor() as usize).min(w - 1);
                    depths[yi * w + xi]
                };
                let box_d = sample(box_u, box_v);
                let ground_d = sample(ground_u, ground_v);
                assert!(
                    box_d < ground_d,
                    "gpu box nearer than plane: {box_d} vs {ground_d}"
                );
                assert!(box_d < 1.0 && ground_d <= 1.0, "map wrote depths");
            }
        }
        let scope_err = pollster::block_on(scope.pop());
        assert!(scope_err.is_none(), "0 validation errors");
        eprintln!(
            "sun single-map offscreen: box ({box_u:.3},{box_v:.3}) nearer than plane, x2 equal, submit_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
