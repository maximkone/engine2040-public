//! upscale_gpu: the live frame's spatial upscale, FSR 1.0 EASU then an optional RCAS sharpen.
//!
//! Owner: render lane. Consumer: the live window frame (`live.rs`), which draws the scene at a
//! render size below the output, lets the blit tone-map it into [`UpscaleSized::lo_view`], and
//! calls [`UpscaleSized::encode`] to land the output-size picture. Units: pixels, a sharpness in
//! `0..=1` (0 skips the sharpen pass). Determinism: a fixed shader, uniform bytes that are a pure
//! function of the two sizes and the sharpness. Presentation only, never hashed.
//!
//! The kernel is `shaders/post/fsr1_upscale.wgsl`; `post::fsr1_easu` is its CPU statement and the
//! tests here hold the two together on a fixture image. Both pass entries come in a plain and a
//! `*_hw_srgb` form, picked by the target format like the blit does.

use super::super::OUTPUT_FORMAT;
use std::sync::{Mutex, PoisonError};

#[cfg(test)]
mod tests;

/// The upscale shader: vertex stage plus the EASU and RCAS fragment entries.
pub(crate) const FSR1_UPSCALE_WGSL: &str = include_str!("../../../shaders/post/fsr1_upscale.wgsl");

/// Bytes of the one uniform both passes read: `con0` then the sharpen params.
const UNIFORM_BYTES: u64 = 32;

/// Pipelines of one entry pair, one per target format, built on the first ask.
type PipeCache = Mutex<Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>>;

/// The pass objects shared by every size: shader, bind layout and the per-format pipelines.
pub(crate) struct UpscalePass {
    layout: wgpu::BindGroupLayout,
    module: wgpu::ShaderModule,
    easu: PipeCache,
    rcas: PipeCache,
}

/// Which fragment pair a pipeline runs.
#[derive(Clone, Copy)]
enum Stage {
    Easu,
    Rcas,
}

impl UpscalePass {
    /// Compile the shader and build the bind layout. Cold path only.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fsr1-upscale-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_BYTES),
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fsr1-upscale"),
            source: wgpu::ShaderSource::Wgsl(FSR1_UPSCALE_WGSL.into()),
        });
        Self {
            layout,
            module,
            easu: Mutex::new(Vec::new()),
            rcas: Mutex::new(Vec::new()),
        }
    }

    /// The EASU pipeline for `format` (`fs_easu`, or `fs_easu_hw_srgb` for a `*Srgb` target).
    pub(crate) fn easu_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        self.pipeline(device, format, Stage::Easu)
    }

    /// The RCAS pipeline for `format`.
    pub(crate) fn rcas_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        self.pipeline(device, format, Stage::Rcas)
    }

    fn pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        stage: Stage,
    ) -> wgpu::RenderPipeline {
        let cache = match stage {
            Stage::Easu => &self.easu,
            Stage::Rcas => &self.rcas,
        };
        let mut kept = cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, pipeline)) = kept.iter().find(|(f, _)| *f == format) {
            return pipeline.clone();
        }
        let pipeline = self.build(device, format, stage);
        kept.push((format, pipeline.clone()));
        pipeline
    }

    fn build(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        stage: Stage,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline_warmup::CountedPipelines as _;
        let entry = match (stage, format.is_srgb()) {
            (Stage::Easu, false) => "fs_easu",
            (Stage::Easu, true) => "fs_easu_hw_srgb",
            (Stage::Rcas, false) => "fs_rcas",
            (Stage::Rcas, true) => "fs_rcas_hw_srgb",
        };
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fsr1-upscale-pipe-layout"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Resources for one (render size, output size, sharpness) triple.
    pub(crate) fn sized(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        in_size: (u32, u32),
        out_size: (u32, u32),
        sharpness: f32,
        want_final: bool,
    ) -> UpscaleSized {
        let sharpness = if sharpness.is_finite() {
            sharpness.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let make =
            |label: &'static str, (width, height): (u32, u32), extra: wgpu::TextureUsages| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: width.max(1),
                        height: height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: OUTPUT_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | extra,
                    view_formats: &[],
                })
            };
        let lo = make("fsr1-lo", in_size, wgpu::TextureUsages::COPY_DST);
        let lo_view = lo.create_view(&wgpu::TextureViewDescriptor::default());
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fsr1-upscale-uniform"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, &uniform_bytes(in_size, out_size, sharpness));
        let bind = |label: &'static str, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            })
        };
        let easu_bind = bind("fsr1-easu-bind", &lo_view);
        let mid = (sharpness > 0.0).then(|| {
            let texture = make("fsr1-mid", out_size, wgpu::TextureUsages::empty());
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let rcas_bind = bind("fsr1-rcas-bind", &view);
            Mid {
                _texture: texture,
                view,
                bind: rcas_bind,
            }
        });
        let fin = want_final.then(|| {
            let texture = make("fsr1-final", out_size, wgpu::TextureUsages::empty());
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Final {
                _texture: texture,
                view,
            }
        });
        UpscaleSized {
            in_size,
            out_size,
            sharpness,
            want_final,
            #[cfg(test)]
            lo: lo.clone(),
            lo_view,
            _uniform: uniform,
            easu_bind,
            mid,
            fin,
        }
    }
}

/// Timestamp slots for one span over the chain: `begin` opens it on the first pass, `end` closes
/// it on the last. Either may be absent (an untimed frame).
#[derive(Default)]
pub(crate) struct SpanStamps<'a> {
    /// Writes the span's start.
    pub(crate) begin: Option<wgpu::RenderPassTimestampWrites<'a>>,
    /// Writes the span's end.
    pub(crate) end: Option<wgpu::RenderPassTimestampWrites<'a>>,
}

/// The EASU output when a sharpen follows: output size, read by the RCAS pass.
struct Mid {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

/// The output-size target the overlays and the look pass continue on, when a look table is
/// bound and the surface itself cannot be their source.
struct Final {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// The pass resources of one size triple.
pub(crate) struct UpscaleSized {
    in_size: (u32, u32),
    out_size: (u32, u32),
    sharpness: f32,
    want_final: bool,
    #[cfg(test)]
    lo: wgpu::Texture,
    lo_view: wgpu::TextureView,
    _uniform: wgpu::Buffer,
    easu_bind: wgpu::BindGroup,
    mid: Option<Mid>,
    fin: Option<Final>,
}

impl UpscaleSized {
    /// The render-size display-byte target the blit writes and EASU reads.
    pub(crate) fn lo_view(&self) -> &wgpu::TextureView {
        &self.lo_view
    }

    /// The texture behind [`Self::lo_view`] (tests upload a fixture into it).
    #[cfg(test)]
    pub(crate) fn lo_texture(&self) -> &wgpu::Texture {
        &self.lo
    }

    /// True when this triple matches (a steady frame rebuilds nothing).
    pub(crate) fn matches(
        &self,
        in_size: (u32, u32),
        out_size: (u32, u32),
        sharpness: f32,
        want_final: bool,
    ) -> bool {
        self.in_size == in_size
            && self.out_size == out_size
            && self.sharpness.to_bits() == sharpness.to_bits()
            && self.want_final == want_final
    }

    /// The output-size target for the overlays and the look pass, when one was asked for.
    pub(crate) fn final_view(&self) -> Option<&wgpu::TextureView> {
        self.fin.as_ref().map(|f| &f.view)
    }

    /// Record the passes: EASU from the `lo` target into `dst` (or into the mid target when a
    /// sharpen follows, which then writes `dst`). `stamps` carries the timer span over the chain.
    pub(crate) fn encode(
        &self,
        pass: &UpscalePass,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        dst: &wgpu::TextureView,
        dst_format: wgpu::TextureFormat,
        stamps: SpanStamps<'_>,
    ) {
        match &self.mid {
            None => {
                let pipeline = pass.easu_pipeline(device, dst_format);
                let both = merge_stamps(stamps.begin, stamps.end);
                draw(enc, "fsr1-easu", &pipeline, &self.easu_bind, dst, both);
            }
            Some(mid) => {
                let easu = pass.easu_pipeline(device, OUTPUT_FORMAT);
                draw(
                    enc,
                    "fsr1-easu",
                    &easu,
                    &self.easu_bind,
                    &mid.view,
                    stamps.begin,
                );
                let rcas = pass.rcas_pipeline(device, dst_format);
                draw(enc, "fsr1-rcas", &rcas, &mid.bind, dst, stamps.end);
            }
        }
    }
}

/// One pass over the whole target; every texel is written, the clear only keeps a valid load op.
fn draw(
    enc: &mut wgpu::CommandEncoder,
    label: &'static str,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    dst: &wgpu::TextureView,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) {
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: dst,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(pipeline);
    rp.set_bind_group(0, bind, &[]);
    rp.draw(0..3, 0..1);
}

/// One pass carrying both ends of the span: the begin slot of `begin`, the end slot of `end`.
fn merge_stamps<'a>(
    begin: Option<wgpu::RenderPassTimestampWrites<'a>>,
    end: Option<wgpu::RenderPassTimestampWrites<'a>>,
) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
    match (begin, end) {
        (Some(b), Some(e)) => Some(wgpu::RenderPassTimestampWrites {
            query_set: b.query_set,
            beginning_of_pass_write_index: b.beginning_of_pass_write_index,
            end_of_pass_write_index: e.end_of_pass_write_index,
        }),
        (one, None) | (None, one) => one,
    }
}

/// The 32 uniform bytes: `con0` (source texels per output pixel, then `0.5 * scale - 0.5`) and
/// the sharpen params. Written exactly like the CPU `fsr_easu_con`, so both sides see one value.
fn uniform_bytes(in_size: (u32, u32), out_size: (u32, u32), sharpness: f32) -> [u8; 32] {
    let dim = |n: u32| f32::from(u16::try_from(n.max(1)).unwrap_or(u16::MAX));
    let sx = dim(in_size.0) / dim(out_size.0);
    let sy = dim(in_size.1) / dim(out_size.1);
    let words = [
        sx,
        sy,
        0.5 * sx - 0.5,
        0.5 * sy - 0.5,
        sharpness,
        0.0,
        0.0,
        0.0,
    ];
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[cfg(test)]
pub(crate) fn uniform_bytes_for_tests(
    in_size: (u32, u32),
    out_size: (u32, u32),
    sharpness: f32,
) -> [u8; 32] {
    uniform_bytes(in_size, out_size, sharpness)
}
