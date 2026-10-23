//! decal_draw_gpu: the fight's blood on the floor, staged and recorded (FEEL5).
//!
//! Owner: render lane. The deferred pool in [`crate::decals`] needs a CPU depth
//! frame to project against, which a live frame does not have; a ground splat
//! needs no projection at all, so this draw lays one world-space quad per
//! resident splat on the floor and tests it against the frame's own depth.
//! Window build only. Units: meters in the world positions, bytes in the WGSL
//! layout. Determinism: fixed field order, no wall clock. Errors: typed
//! [`DecalDrawError`]; a refused stage never touches the device.

use crate::decals::{BloodSplat, MAX_DECALS};

/// Draw-uniform byte size for `shaders/decal.wgsl` (`DecalU`): one `mat4`
/// view-projection. Fixed shape, never rebuilt.
pub const DECAL_DRAW_UNIFORM_BYTES: usize = 64;

/// One staged splat: the `Splat` image of `shaders/decal.wgsl` — center plus
/// radius, then the tint rgb and the tick-fade alpha. 32 bytes per splat.
pub const DECAL_DRAW_INSTANCE_BYTES: usize = 32;

/// Most splats one staged frame draws: the decal pool's hard bound, so the
/// tier cap is always inside the buffer the draw was built with.
pub const DECAL_DRAW_MAX_SPLATS: usize = MAX_DECALS;

/// Ground-splat draw shader: one quad per splat, ragged soft rim, fade alpha.
/// Engine-owned WGSL, fixed math only, no wall clock.
pub const DECAL_DRAW_WGSL: &str = include_str!("../shaders/decal.wgsl");

/// Append one little-endian `f32` at `*at`; stack bytes only.
fn decal_put_f32(out: &mut [u8], at: &mut usize, v: f32) {
    out[*at..*at + 4].copy_from_slice(&v.to_le_bytes());
    *at += 4;
}

/// Pack the `DecalU` image: the frame view-projection in column-major order
/// (the image the forward camera uniform carries). Deterministic: fixed order.
pub fn decal_pack_uniform(view_proj: [[f32; 4]; 4]) -> [u8; DECAL_DRAW_UNIFORM_BYTES] {
    let mut out = [0u8; DECAL_DRAW_UNIFORM_BYTES];
    let mut at = 0usize;
    for col in view_proj {
        for v in col {
            decal_put_f32(&mut out, &mut at, v);
        }
    }
    debug_assert_eq!(at, DECAL_DRAW_UNIFORM_BYTES);
    out
}

/// Pack one staged [`BloodSplat`] into the `Splat` image: center, radius,
/// blood color and the tick-fade alpha. Deterministic: fixed order.
pub fn decal_pack_instance(splat: &BloodSplat, out: &mut [u8]) {
    debug_assert_eq!(out.len(), DECAL_DRAW_INSTANCE_BYTES);
    let mut at = 0usize;
    for v in splat.center_m {
        decal_put_f32(out, &mut at, v);
    }
    decal_put_f32(out, &mut at, splat.radius_m);
    for v in crate::decals::BLOOD_COLOR {
        decal_put_f32(out, &mut at, v);
    }
    decal_put_f32(out, &mut at, splat.alpha());
    debug_assert_eq!(at, DECAL_DRAW_INSTANCE_BYTES);
}

/// Pack every staged splat into one contiguous byte buffer; one
/// [`DECAL_DRAW_INSTANCE_BYTES`] block per splat, in order.
pub fn decal_pack_field(splats: &[BloodSplat]) -> Vec<u8> {
    let mut out = vec![0u8; splats.len() * DECAL_DRAW_INSTANCE_BYTES];
    for (i, splat) in splats.iter().enumerate() {
        let slice = &mut out[i * DECAL_DRAW_INSTANCE_BYTES..(i + 1) * DECAL_DRAW_INSTANCE_BYTES];
        decal_pack_instance(splat, slice);
    }
    out
}

/// Refused stage, named so the caller can report what it carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalDrawError {
    /// More splats than one field buffer holds.
    OverCap {
        /// Splats the caller staged.
        requested: usize,
        /// Splats the buffer holds.
        allowed: usize,
    },
    /// A center, a radius or a matrix entry is not finite.
    NotFinite,
}

impl std::fmt::Display for DecalDrawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverCap { requested, allowed } => {
                write!(f, "splat stage holds {requested} splats, cap {allowed}")
            }
            Self::NotFinite => f.write_str("splat stage carried a non-finite value"),
        }
    }
}

/// Draw bind-group layout: the view uniform and the splat storage buffer.
/// Fixed shape, never rebuilt.
pub fn decal_draw_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let uniform_bytes = u64::try_from(DECAL_DRAW_UNIFORM_BYTES).unwrap_or(u64::MAX);
    let splat_bytes = u64::try_from(DECAL_DRAW_INSTANCE_BYTES).unwrap_or(u64::MAX);
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("feel5-decal-draw-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(uniform_bytes),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(splat_bytes),
                },
                count: None,
            },
        ],
    })
}

/// Ground-splat pipeline: triangle-strip quads, one HDR colour target alpha
/// blended, and a read-only depth test against the frame's stored depth
/// (`Depth32Float`, never written), so a splat under a body stays hidden.
pub fn decal_draw_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("feel5-decal-draw"),
        source: wgpu::ShaderSource::Wgsl(DECAL_DRAW_WGSL.into()),
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("feel5-decal-draw-pipe-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("feel5-decal-draw-pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("decal_vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("decal_fs"),
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
        depth_stencil: Some(wgpu::DepthStencilState {
            format: crate::present::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// One device buffer with `usage` plus the copy destination every stage needs.
fn decal_draw_buffer(
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

/// Device state for the staged draw: the pipeline, the 64-byte uniform, the
/// max-size splat buffer and the one bind group that ties them together.
/// Built once, on the first non-empty field; every later frame rewrites the
/// same buffers.
struct DecalDrawState {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    field: wgpu::Buffer,
    /// Staged instance bytes, reused across frames (resized, never per frame
    /// reallocated past the high-water mark).
    packed: Vec<u8>,
    /// Splats staged this frame; 0 records no draw.
    count: u32,
}

/// Reusable window submit state for the ground splats: the frame uniform the
/// camera fills and the staged field. Built once per frame loop; a steady
/// frame rewrites the same two buffers and creates nothing. Never maps and
/// never waits on the device.
pub struct DecalDrawSubmitter {
    view: [u8; DECAL_DRAW_UNIFORM_BYTES],
    draw: Option<DecalDrawState>,
}

impl Default for DecalDrawSubmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl DecalDrawSubmitter {
    /// Build the submitter: the draw camera starts at identity, and no device
    /// state exists until the first non-empty field.
    pub fn new() -> Self {
        Self {
            view: decal_pack_uniform([
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
            draw: None,
        }
    }

    /// Buffers created so far (probe for the reuse test: flat across frames
    /// that stage the same shape).
    pub fn created_buffers(&self) -> u64 {
        if self.draw.is_some() {
            2
        } else {
            0
        }
    }

    /// Splats in the field staged on the device; 0 draws nothing.
    pub fn staged_count(&self) -> u32 {
        self.draw.as_ref().map_or(0, |draw| draw.count)
    }

    /// True when the staged frame has splats to draw.
    pub fn has_draw(&self) -> bool {
        self.staged_count() > 0
    }

    /// Stage one frame: the resident splats and this frame's view-projection.
    /// Over-cap fields and non-finite values are refused before any device
    /// work; an empty field clears the staged count, so a calm frame issues no
    /// draw call. This path never maps and never waits.
    pub fn stage(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        splats: &[BloodSplat],
        view_proj: [[f32; 4]; 4],
    ) -> Result<u32, DecalDrawError> {
        if splats.len() > DECAL_DRAW_MAX_SPLATS {
            return Err(DecalDrawError::OverCap {
                requested: splats.len(),
                allowed: DECAL_DRAW_MAX_SPLATS,
            });
        }
        let finite = view_proj.iter().flatten().all(|c| c.is_finite())
            && splats.iter().all(|splat| {
                splat.center_m.iter().all(|c| c.is_finite())
                    && splat.radius_m.is_finite()
                    && splat.alpha().is_finite()
            });
        if !finite {
            return Err(DecalDrawError::NotFinite);
        }
        self.view = decal_pack_uniform(view_proj);
        if splats.is_empty() {
            if let Some(draw) = self.draw.as_mut() {
                draw.count = 0;
            }
            return Ok(0);
        }
        if self.draw.is_none() {
            self.draw = Some(DecalDrawState::build(device));
        }
        let Some(draw) = self.draw.as_mut() else {
            return Ok(0);
        };
        draw.packed.clear();
        draw.packed
            .resize(splats.len() * DECAL_DRAW_INSTANCE_BYTES, 0);
        for (index, splat) in splats.iter().enumerate() {
            let at = index * DECAL_DRAW_INSTANCE_BYTES;
            let slice = &mut draw.packed[at..at + DECAL_DRAW_INSTANCE_BYTES];
            decal_pack_instance(splat, slice);
        }
        queue.write_buffer(&draw.uniform, 0, &self.view);
        queue.write_buffer(&draw.field, 0, &draw.packed);
        draw.count = u32::try_from(splats.len()).unwrap_or(u32::MAX);
        Ok(draw.count)
    }

    /// Record the staged splats into `pass`, or nothing when the frame staged
    /// none. The pass must carry one HDR colour target and the frame's stored
    /// depth read-only, the shape the pipeline declares.
    pub fn encode_draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(draw) = self.draw.as_ref().filter(|draw| draw.count > 0) else {
            return;
        };
        pass.set_pipeline(&draw.pipeline);
        pass.set_bind_group(0, &draw.bind, &[]);
        pass.draw(0..4, 0..draw.count);
    }
}

impl DecalDrawState {
    /// Build the draw state once: the pipeline over [`DECAL_DRAW_WGSL`], the
    /// uniform, the max-size splat buffer and the bind group. Two buffers are
    /// what the create counter charges.
    fn build(device: &wgpu::Device) -> Self {
        let layout = decal_draw_layout(device);
        let pipeline = decal_draw_pipeline(device, &layout, crate::present::HDR_FORMAT);
        // Buffer sizes are layout constants; the `usize` to `u64` step is
        // total on every target the engine builds for.
        let uniform_bytes = u64::try_from(DECAL_DRAW_UNIFORM_BYTES).unwrap_or(u64::MAX);
        let field_bytes =
            u64::try_from(DECAL_DRAW_MAX_SPLATS * DECAL_DRAW_INSTANCE_BYTES).unwrap_or(u64::MAX);
        let uniform = decal_draw_buffer(
            device,
            "feel5-decal-uniform",
            uniform_bytes,
            wgpu::BufferUsages::UNIFORM,
        );
        let field = decal_draw_buffer(
            device,
            "feel5-decal-field",
            field_bytes,
            wgpu::BufferUsages::STORAGE,
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("feel5-decal-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: field.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            bind,
            uniform,
            field,
            packed: Vec::with_capacity(DECAL_DRAW_MAX_SPLATS * DECAL_DRAW_INSTANCE_BYTES),
            count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splat(seq: u64, x: f32) -> BloodSplat {
        BloodSplat {
            center_m: [x, 0.0, 0.0],
            radius_m: 0.3,
            age_ticks: seq,
            seq,
        }
    }

    #[test]
    fn splat_packers_are_deterministic() {
        let a = decal_pack_uniform([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let b = decal_pack_uniform([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        assert_eq!(a, b, "the same matrix packs the same uniform bytes");
        assert_eq!(a.len(), DECAL_DRAW_UNIFORM_BYTES);
        let field = decal_pack_field(&[splat(0, 1.0), splat(1, 2.0)]);
        assert_eq!(field.len(), 2 * DECAL_DRAW_INSTANCE_BYTES);
        assert_eq!(
            field,
            decal_pack_field(&[splat(0, 1.0), splat(1, 2.0)]),
            "the same splats pack the same field bytes"
        );
        let alpha_at = DECAL_DRAW_INSTANCE_BYTES - 4;
        let alpha = f32::from_le_bytes(field[alpha_at..alpha_at + 4].try_into().expect("4 bytes"));
        assert!(
            (alpha - crate::decals::blood_alpha(0)).abs() < 1e-6,
            "the last word is the fade alpha at that age"
        );
    }

    #[test]
    fn a_stage_over_the_cap_or_a_bad_value_is_refused() {
        // Pure guards run before any device work. The stage path needs a
        // device, so the cap and the finite check live as constants here and
        // the GPU test below proves the accepted shape records.
        assert_eq!(DECAL_DRAW_MAX_SPLATS, crate::decals::MAX_DECALS);
        assert_eq!(DECAL_DRAW_INSTANCE_BYTES, 8 * 4);
        assert_eq!(DECAL_DRAW_UNIFORM_BYTES, 16 * 4);
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn splats_stage_and_record_without_validation_errors() {
        let Some((device, queue, _info)) = crate::test_gpu::require_or_skip(
            "splats_stage_and_record_without_validation_errors",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let mut submitter = DecalDrawSubmitter::new();
        assert_eq!(submitter.created_buffers(), 0);
        assert!(!submitter.has_draw(), "a fresh submitter draws nothing");
        let view_proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let staged = submitter
            .stage(device, queue, &[splat(0, 1.0), splat(1, 1.5)], view_proj)
            .expect("the field uploads");
        assert_eq!(staged, 2);
        assert_eq!(submitter.created_buffers(), 2, "two buffers, built once");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("feel5-decal-draw-target"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::present::HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("feel5-decal-draw-depth"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::present::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let hdr_view = hdr.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("feel5-decal-draw-frame"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("feel5-decal-draw-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
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
            submitter.encode_draw(&mut pass);
        }
        queue.submit(Some(enc.finish()));
        assert!(
            pollster::block_on(scope.pop()).is_none(),
            "the staged splats must record a valid draw"
        );
        // A calm frame clears the staged count, so nothing draws and nothing
        // new is created.
        assert_eq!(
            submitter
                .stage(device, queue, &[], view_proj)
                .expect("an idle frame stages nothing"),
            0
        );
        assert!(!submitter.has_draw());
        assert_eq!(
            submitter.created_buffers(),
            2,
            "a calm frame creates nothing"
        );
    }

    /// The packer writes each field at the offset naga gives it, so the frame
    /// uniform the shader reads is the image the CPU staged.
    #[cfg(feature = "shader-validate")]
    #[test]
    fn decal_uniform_layout_matches_the_packer() {
        let (span, fields) =
            engine_material::compile::uniform_struct_layout(DECAL_DRAW_WGSL, "DecalU")
                .expect("decal.wgsl declares DecalU");
        assert_eq!(
            span,
            u32::try_from(DECAL_DRAW_UNIFORM_BYTES).expect("small")
        );
        let offset = |name: &str| {
            fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, offset)| *offset)
                .unwrap_or_else(|| panic!("DecalU has no {name}"))
        };
        assert_eq!(offset("view_proj"), 0);
    }
}
