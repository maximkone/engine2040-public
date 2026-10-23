//! bloom_dual (device): the live frame's dual-filter bloom on the GPU.
//!
//! Owner: render lane, consumer `present_frame_live` (one compute pass after
//! the forward, before the blit). Shape: a Karis-weighted bright mip at half
//! the frame size, then `levels - 1` 13-tap downsample dispatches under it,
//! then `levels - 1` 3x3 tent upsample dispatches that fold each smaller rung
//! back onto the larger one, smallest first. The blit samples rung 0 at half
//! size and adds it before the frame's one tone map. The shaders and the CPU
//! oracle (`crate::bloom_dual::oracle`, test builds) run the same math and a
//! device test pins their parity.
//!
//! Units: linear HDR radiance, every mip `Rgba16Float` (11-bit significand),
//! which stays inside `GPU_PARITY_TOL_POST` against the f32 oracle.
//! Determinism: fixed tap order, no clock, no readback; presentation only,
//! never the tick hash. Sizing is cold path (with the targets): one texture
//! per mip and one 48-byte uniform per dispatch, all written once. A steady
//! frame allocates nothing, writes no uniform and records one pass.
//! Errors: a frame the plan cannot size gets a 1x1 zero halo and no dispatch,
//! so the blit still binds a texture and adds nothing.
//! Budget: `BLOOM_DUAL_BUDGET_MS`, held by the timing test on this PC.

use crate::bloom_dual::DualBloomPlan;
use crate::decals::compute_uniform_entry;

/// Bytes of the chain uniform (`BloomU` in all three shaders).
const BLOOM_UNIFORM_BYTES: u64 = 48;

/// Workgroup edge of every dispatch, in pixels.
const GROUP_EDGE: u32 = 8;

/// Device-ms budget of the whole 720p chain; the timing test holds it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the timing test holds this budget")
)]
pub(crate) const BLOOM_DUAL_BUDGET_MS: f64 = 0.5;

/// Mip 0: the Karis-weighted bright extract.
const BRIGHT_WGSL: &str = include_str!("../../shaders/post/bloom_dual_bright.wgsl");

/// One 13-tap downsample step.
const DOWN_WGSL: &str = include_str!("../../shaders/post/bloom_dual_down.wgsl");

/// One 3x3 tent upsample step with the half blend.
const UP_WGSL: &str = include_str!("../../shaders/post/bloom_dual_up.wgsl");

/// Which pipeline one dispatch runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// Bright extract into mip 0.
    Bright,
    /// 13-tap downsample into the next smaller mip.
    Down,
    /// Tent upsample into the next larger rung.
    Up,
}

/// One dispatch of the chain: the pipeline, its bind group and its grid.
struct Step {
    stage: Stage,
    bind: wgpu::BindGroup,
    groups: (u32, u32),
}

/// The device half of the live bloom: the three compute pipelines, their
/// layouts and the one clamp-to-edge linear sampler every step shares.
pub(crate) struct BloomDualGpu {
    bright: wgpu::ComputePipeline,
    down: wgpu::ComputePipeline,
    up: wgpu::ComputePipeline,
    bright_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    up_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Frame-size resources of the chain, built with the targets they read and
/// sampled by the blit bind's halo slot.
pub(crate) struct BloomDualSized {
    /// The half-res halo the blit samples (rung 0, or the bright mip when the
    /// ladder holds one rung).
    pub(crate) halo_view: wgpu::TextureView,
    /// Index into `textures` of the halo's own texture.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "names the halo texture the parity test reads")
    )]
    halo_index: usize,
    /// The dispatches in encode order.
    steps: Vec<Step>,
    /// Owns every mip texture the steps and `halo_view` read.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "owns the mip textures the steps read")
    )]
    textures: Vec<wgpu::Texture>,
    /// Owns every uniform buffer the steps read.
    _uniforms: Vec<wgpu::Buffer>,
}

impl BloomDualSized {
    /// A 1x1 zero halo for a frame the plan cannot size: the blit still binds
    /// a texture and adds nothing.
    fn empty(device: &wgpu::Device) -> Self {
        let (texture, view) = mip_texture(device, "bloom-dual-empty", (1, 1));
        Self {
            halo_view: view,
            halo_index: 0,
            steps: Vec::new(),
            textures: vec![texture],
            _uniforms: Vec::new(),
        }
    }

    /// The texture behind `halo_view`; the parity test copies it back.
    #[cfg(test)]
    pub(crate) fn halo_texture(&self) -> Option<&wgpu::Texture> {
        self.textures.get(self.halo_index)
    }
}

/// One filterable float read: the source mip, or the base rung at binding 1.
fn float_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// One filtering sampler, for the bilinear taps of the down and up steps.
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// One `Rgba16Float` storage write: every mip is written once, read later.
fn storage_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

/// One mip texture and the view its passes bind. `Rgba16Float` is filterable
/// and writable as a storage texture; COPY_SRC lets the parity test read the
/// halo back.
fn mip_texture(
    device: &wgpu::Device,
    label: &str,
    (width, height): (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Pack one dispatch's uniform: both sizes and the soft-knee pair. Pure bytes.
fn uniform_bytes(src: (u32, u32), dst: (u32, u32), params: [f32; 4]) -> [u8; 48] {
    let mut out = [0u8; 48];
    for (i, v) in [src.0, src.1, 0, 0, dst.0, dst.1, 0, 0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in params.iter().enumerate() {
        out[32 + i * 4..36 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// One dispatch's uniform buffer, written once at sizing.
fn uniform_buffer(device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8; 48]) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bloom-dual-uniform"),
        size: BLOOM_UNIFORM_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    buffer
}

/// Workgroup grid of one mip.
fn grid((width, height): (u32, u32)) -> (u32, u32) {
    (
        width.max(1).div_ceil(GROUP_EDGE),
        height.max(1).div_ceil(GROUP_EDGE),
    )
}

impl BloomDualGpu {
    /// Compile the three kernels, their layouts and the shared sampler. Cold
    /// path, once per presenter.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        use crate::pipeline_warmup::CountedPipelines as _;
        let layout = |label: &'static str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let bright_layout = layout(
            "bloom-dual-bright-layout",
            &[
                compute_uniform_entry(0),
                float_texture_entry(1),
                storage_texture_entry(2),
            ],
        );
        let down_layout = layout(
            "bloom-dual-down-layout",
            &[
                compute_uniform_entry(0),
                float_texture_entry(1),
                sampler_entry(2),
                storage_texture_entry(3),
            ],
        );
        let up_layout = layout(
            "bloom-dual-up-layout",
            &[
                compute_uniform_entry(0),
                float_texture_entry(1),
                float_texture_entry(2),
                sampler_entry(3),
                storage_texture_entry(4),
            ],
        );
        let module = |label: &'static str, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let pipeline = |label: &'static str,
                        layout: &wgpu::BindGroupLayout,
                        module: &wgpu::ShaderModule,
                        entry: &'static str| {
            let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipe_layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let bright_module = module("bloom-dual-bright", BRIGHT_WGSL);
        let down_module = module("bloom-dual-down", DOWN_WGSL);
        let up_module = module("bloom-dual-up", UP_WGSL);
        Self {
            bright: pipeline(
                "bloom-dual-bright",
                &bright_layout,
                &bright_module,
                "bright_main",
            ),
            down: pipeline("bloom-dual-down", &down_layout, &down_module, "down_main"),
            up: pipeline("bloom-dual-up", &up_layout, &up_module, "up_main"),
            bright_layout,
            down_layout,
            up_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("bloom-dual-sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
        }
    }

    /// Size the chain over one source image: every mip texture, one uniform
    /// per dispatch and the dispatches in encode order. Cold path: runs with
    /// the targets, never per frame.
    pub(crate) fn sized(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::TextureView,
        width: u32,
        height: u32,
        cfg: &crate::post::BloomConfig,
    ) -> BloomDualSized {
        let Some(plan) = DualBloomPlan::for_frame(width, height, cfg).ok() else {
            return BloomDualSized::empty(device);
        };
        let sizes = plan.level_sizes();
        let levels = sizes.len();
        let (down_textures, down_views) = sizes
            .iter()
            .map(|size| mip_texture(device, "bloom-dual-down", *size))
            .unzip::<_, _, Vec<_>, Vec<_>>();
        let up_count = levels.saturating_sub(1);
        let (up_textures, up_views) = sizes
            .iter()
            .take(up_count)
            .map(|size| mip_texture(device, "bloom-dual-up", *size))
            .unzip::<_, _, Vec<_>, Vec<_>>();
        let mut uniforms: Vec<wgpu::Buffer> = Vec::with_capacity(levels.saturating_mul(2));
        let mut steps: Vec<Step> = Vec::with_capacity(levels.saturating_mul(2));
        let first = sizes.first().copied().unwrap_or((1, 1));
        // Mip 0: the bright extract reads the frame at full size.
        let uniform = uniform_buffer(
            device,
            queue,
            &uniform_bytes(
                (width, height),
                first,
                [plan.threshold, plan.knee, 0.0, 0.0],
            ),
        );
        if let Some(down_first) = down_views.first() {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-dual-bright-bind"),
                layout: &self.bright_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(down_first),
                    },
                ],
            });
            steps.push(Step {
                stage: Stage::Bright,
                bind,
                groups: grid(first),
            });
            uniforms.push(uniform);
        }
        self.push_down_steps(
            device,
            queue,
            &sizes,
            &down_views,
            &mut steps,
            &mut uniforms,
        );
        self.push_up_steps(
            device,
            queue,
            &sizes,
            &down_views,
            &up_views,
            &mut steps,
            &mut uniforms,
        );
        let halo = if up_count == 0 {
            down_views.first().cloned()
        } else {
            up_views.first().cloned()
        };
        let Some(halo_view) = halo else {
            return BloomDualSized::empty(device);
        };
        let halo_index = if up_count == 0 { 0 } else { levels };
        let mut textures = down_textures;
        textures.extend(up_textures);
        BloomDualSized {
            halo_view,
            halo_index,
            steps,
            textures,
            _uniforms: uniforms,
        }
    }

    /// The 13-tap rungs below mip 0: `down[i]` from `down[i - 1]`. One bind
    /// group and one uniform per rung, built with the targets.
    fn push_down_steps(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sizes: &[(u32, u32)],
        down: &[wgpu::TextureView],
        steps: &mut Vec<Step>,
        uniforms: &mut Vec<wgpu::Buffer>,
    ) {
        for level in 1..sizes.len() {
            let (Some(src), Some(dst)) = (sizes.get(level - 1), sizes.get(level)) else {
                continue;
            };
            let (Some(src_view), Some(dst_view)) = (down.get(level - 1), down.get(level)) else {
                continue;
            };
            let uniform = uniform_buffer(device, queue, &uniform_bytes(*src, *dst, [0.0; 4]));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-dual-down-bind"),
                layout: &self.down_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(dst_view),
                    },
                ],
            });
            steps.push(Step {
                stage: Stage::Down,
                bind,
                groups: grid(*dst),
            });
            uniforms.push(uniform);
        }
    }

    /// The tent rungs, smallest first: `up[i] = (down[i] + tent(up[i + 1])) / 2`.
    /// Rung `i + 1` is the next up texture, or `down[last]` at the top.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call carries the device, queue, both ladders and the accumulators"
    )]
    fn push_up_steps(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sizes: &[(u32, u32)],
        down: &[wgpu::TextureView],
        up: &[wgpu::TextureView],
        steps: &mut Vec<Step>,
        uniforms: &mut Vec<wgpu::Buffer>,
    ) {
        for level in (0..up.len()).rev() {
            let (Some(src), Some(dst)) = (sizes.get(level + 1), sizes.get(level)) else {
                continue;
            };
            let (Some(base_view), Some(dst_view)) = (down.get(level), up.get(level)) else {
                continue;
            };
            let previous = if level + 1 == up.len() {
                down.get(level + 1)
            } else {
                up.get(level + 1)
            };
            let Some(previous) = previous else {
                continue;
            };
            let uniform = uniform_buffer(device, queue, &uniform_bytes(*src, *dst, [0.0; 4]));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-dual-up-bind"),
                layout: &self.up_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(base_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(previous),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(dst_view),
                    },
                ],
            });
            steps.push(Step {
                stage: Stage::Up,
                bind,
                groups: grid(*dst),
            });
            uniforms.push(uniform);
        }
    }

    /// Record the chain for one frame: one compute pass, the dispatches in
    /// ladder order (bright, downs, ups, smallest first). Dispatches in a pass
    /// are ordered, so each rung reads the rung written before it.
    /// Caller: `present_frame_live`, only when the bloom gate opens.
    pub(crate) fn encode(
        &self,
        enc: &mut wgpu::CommandEncoder,
        sized: &BloomDualSized,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        if sized.steps.is_empty() {
            return;
        }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ra1-bloom-dual"),
            timestamp_writes: timestamps,
        });
        for step in &sized.steps {
            let pipeline = match step.stage {
                Stage::Bright => &self.bright,
                Stage::Down => &self.down,
                Stage::Up => &self.up,
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &step.bind, &[]);
            pass.dispatch_workgroups(step.groups.0, step.groups.1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::submit::map_readback;
    use super::*;
    use crate::bloom_dual::oracle;
    use crate::pass_truth::harness::fixtures::init_or_skip;
    use crate::post::gpu::GPU_PARITY_TOL_POST;
    use crate::post::BloomConfig;
    use crate::present::FrameWaits;
    use crate::present::Presenter;

    /// The seeded palette: `(linear value, f16 bits)`. The device source is
    /// written from the bits and the oracle reads the value, so both sides
    /// start from the same numbers inside `Rgba16Float`. Every value sits
    /// under the knee foot, so the one hot texel is the frame's only bloom
    /// source and the mean leg measures its halo alone.
    const PALETTE: [(f32, u16); 8] = [
        (0.03125, 0x2800),
        (0.0625, 0x2C00),
        (0.125, 0x3000),
        (0.1875, 0x3200),
        (0.25, 0x3400),
        (0.3125, 0x3500),
        (0.375, 0x3600),
        (0.4375, 0x3700),
    ];

    /// One texel at 10000 nits, exact in `Rgba16Float` (2^13 * 1.220703125).
    const HOT: (f32, u16) = (10_000.0, 0x70E2);

    /// `f16` bits back to `f32`: the readback decode. The palette, the hot
    /// texel and every value a float16 can hold below 16 decode exactly.
    fn half_to_f32(h: u16) -> f32 {
        let sign = if h >> 15 == 1 { -1.0 } else { 1.0 };
        let exp = i32::from((h >> 10) & 0x1f);
        let frac = f32::from(h & 0x3ff);
        let magnitude = match exp {
            0 => frac * 2.0f32.powi(-24),
            31 if frac == 0.0 => f32::INFINITY,
            31 => f32::NAN,
            _ => (1.0 + frac / 1024.0) * 2.0f32.powi(exp - 15),
        };
        sign * magnitude
    }

    /// The parity frame: the palette in a fixed pattern with one 10000-nit
    /// texel in the middle. Returns the oracle image and its f16 bytes.
    fn parity_frame(width: u32, height: u32) -> (Vec<[f32; 3]>, Vec<u8>) {
        let mut frame = Vec::with_capacity(width as usize * height as usize);
        let mut bytes = Vec::with_capacity(width as usize * height as usize * 8);
        for y in 0..height {
            for x in 0..width {
                let channels = if x == width / 2 && y == height / 2 {
                    [HOT, HOT, HOT]
                } else {
                    let first = ((x * 7 + y * 3) % 8) as usize;
                    [
                        PALETTE[first],
                        PALETTE[(first + 3) % 8],
                        PALETTE[(first + 5) % 8],
                    ]
                };
                frame.push([channels[0].0, channels[1].0, channels[2].0]);
                for (_value, bits) in channels {
                    bytes.extend_from_slice(&bits.to_le_bytes());
                }
                bytes.extend_from_slice(&0x3C00u16.to_le_bytes());
            }
        }
        (frame, bytes)
    }

    /// One `Rgba16Float` source texture over `bytes`, plus its view.
    fn source_texture(
        presenter: &Presenter,
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = presenter.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bloom-dual-source"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        presenter.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Copy one `Rgba16Float` texture back, row-major `[r, g, b]` per texel.
    fn read_halo(
        presenter: &Presenter,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<[f32; 3]> {
        let row = width * 8;
        let padded = row.div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom-dual-halo-readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bloom-dual-halo-copy"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
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
        presenter.queue.submit(Some(enc.finish()));
        let mut waits = FrameWaits::default();
        let bytes = map_readback(&presenter.device, &mut waits, &buffer, row, padded, height)
            .expect("halo readback maps");
        bytes
            .chunks_exact(8)
            .map(|c| {
                [
                    half_to_f32(u16::from_le_bytes([c[0], c[1]])),
                    half_to_f32(u16::from_le_bytes([c[2], c[3]])),
                    half_to_f32(u16::from_le_bytes([c[4], c[5]])),
                ]
            })
            .collect()
    }

    /// Mean of one linear frame over every channel.
    fn mean(frame: &[[f32; 3]]) -> f32 {
        let total: f32 = frame.iter().map(|p| p[0] + p[1] + p[2]).sum();
        total / (frame.len() as f32 * 3.0)
    }

    /// RL36 accept: the device chain and the CPU oracle resolve the same halo
    /// (inside `GPU_PARITY_TOL_POST`), and one 10000-nit pixel moves the
    /// frame's mean under one percent. Skip without an adapter, never fake
    /// pass.
    #[test]
    fn gpu_bloom_dual_filter_parity() {
        let Some(presenter) = init_or_skip("gpu_bloom_dual_filter_parity") else {
            return;
        };
        let (width, height) = (64u32, 64u32);
        let cfg = BloomConfig::default();
        let plan = DualBloomPlan::for_frame(width, height, &cfg).expect("plan builds");
        let (frame, bytes) = parity_frame(width, height);
        let (_source, source_view) = source_texture(&presenter, width, height, &bytes);
        let gpu = BloomDualGpu::new(&presenter.device);
        let sized = gpu.sized(
            &presenter.device,
            &presenter.queue,
            &source_view,
            width,
            height,
            &cfg,
        );
        let levels = usize::try_from(plan.levels).expect("levels fit usize");
        assert_eq!(
            sized.steps.len(),
            1 + 2 * (levels - 1),
            "bright pass plus both ladders"
        );
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bloom-dual-parity"),
            });
        gpu.encode(&mut enc, &sized, None);
        presenter.queue.submit(Some(enc.finish()));
        let (hw, hh) = plan.level_size(0);
        let texture = sized.halo_texture().expect("halo texture");
        let gpu_halo = read_halo(&presenter, texture, hw, hh);
        let cpu_halo = oracle::halo(&frame, &plan).expect("cpu ladder runs");
        assert_eq!(gpu_halo.len(), cpu_halo.len(), "same halo size");
        let mut max_diff = 0.0f32;
        for (gpu_texel, cpu_texel) in gpu_halo.iter().zip(cpu_halo.iter()) {
            for ch in 0..3 {
                let diff = (gpu_texel[ch] - cpu_texel[ch]).abs();
                max_diff = max_diff.max(diff);
            }
        }
        let peak = cpu_halo
            .iter()
            .flat_map(|t| t.iter())
            .fold(0.0f32, |m, v| m.max(*v));
        eprintln!(
            "bloom dual parity on {}: {hw}x{hh} halo, peak {peak:.5}, max |gpu - cpu| {max_diff:.6}",
            presenter.adapter_name()
        );
        assert!(peak > 0.05, "the 10000-nit texel haloes, peak {peak}");
        assert!(
            max_diff <= GPU_PARITY_TOL_POST,
            "gpu halo matches the cpu oracle: {max_diff} > {GPU_PARITY_TOL_POST}"
        );
        // One 10000-nit pixel must not own the frame: composite the device
        // halo the way the blit does and compare the frame's mean.
        let lit = oracle::composite(&frame, &gpu_halo, &plan);
        let base = mean(&frame);
        let lifted = mean(&lit);
        let shift = (lifted - base).abs() / base;
        eprintln!(
            "one 10000-nit texel moves the frame mean {:.4}% ({base:.6} -> {lifted:.6})",
            shift * 100.0
        );
        assert!(
            shift <= 0.01,
            "one 10000-nit texel moves the frame mean under 1%: {shift}"
        );
        // RL36 accept, third leg: the whole 720p chain inside the budget, so
        // this one command proves parity, the frame mean and the clock.
        if presenter
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            let (ms, dispatches) = measure_720p(&presenter);
            eprintln!(
                "bloom dual 720p on {}: {ms:.4} ms over {dispatches} dispatches (budget {BLOOM_DUAL_BUDGET_MS} ms)",
                presenter.adapter_name()
            );
            assert!(ms > 0.0, "the device clock advanced over the chain");
            assert!(
                ms <= BLOOM_DUAL_BUDGET_MS,
                "the 720p chain stays inside {BLOOM_DUAL_BUDGET_MS} ms: {ms}"
            );
        } else {
            crate::test_gpu::note_skip("bloom_dual_parity_timing", "adapter lacks TIMESTAMP_QUERY");
        }
    }

    /// RL36 accept, third leg: the 720p timing. Build the chain over a
    /// 1280x720 source and measure the best of three timed passes. The shared
    /// GPU sits at its idle clock (315 of 2100 MHz here) between submits, so a
    /// lone pass would measure the idle clock, not the frame: hold the chain
    /// on the queue for a second first, and keep twenty chains ahead of every
    /// timed pass. The timestamps bracket the pass itself, so the queued work
    /// does not enter the number. Returns `(ms, dispatch count)`.
    fn measure_720p(presenter: &Presenter) -> (f64, usize) {
        let (width, height) = (1280u32, 720u32);
        let cfg = BloomConfig::default();
        let (_frame, bytes) = parity_frame(width, height);
        let (_source, source_view) = source_texture(presenter, width, height, &bytes);
        let gpu = BloomDualGpu::new(&presenter.device);
        let sized = gpu.sized(
            &presenter.device,
            &presenter.queue,
            &source_view,
            width,
            height,
            &cfg,
        );
        let warm = |count: usize| {
            for _ in 0..count {
                let mut enc =
                    presenter
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("bloom-dual-warmup"),
                        });
                gpu.encode(&mut enc, &sized, None);
                presenter.queue.submit(Some(enc.finish()));
            }
        };
        warm(400);
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("warm-up finishes");
        let queries = presenter
            .device
            .create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("bloom-dual-timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            });
        let resolve = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom-dual-timestamp-resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom-dual-timestamp-staging"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut best = f64::MAX;
        for _ in 0..3 {
            warm(20);
            let mut enc =
                presenter
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("bloom-dual-timed"),
                    });
            let writes = wgpu::ComputePassTimestampWrites {
                query_set: &queries,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            };
            gpu.encode(&mut enc, &sized, Some(writes));
            enc.resolve_query_set(&queries, 0..2, &resolve, 0);
            enc.copy_buffer_to_buffer(&resolve, 0, &staging, 0, 16);
            presenter.queue.submit(Some(enc.finish()));
            let mut waits = FrameWaits::default();
            let ticks = map_readback(&presenter.device, &mut waits, &staging, 16, 16, 1)
                .expect("timestamp readback");
            let begin = u64::from_le_bytes(ticks[0..8].try_into().expect("begin tick"));
            let end = u64::from_le_bytes(ticks[8..16].try_into().expect("end tick"));
            let ns = (end.saturating_sub(begin)) as f64
                * f64::from(presenter.queue.get_timestamp_period());
            best = best.min(ns / 1.0e6);
        }
        (best, sized.steps.len())
    }

    /// RL36 accept: the whole 720p chain stays inside
    /// [`BLOOM_DUAL_BUDGET_MS`] on the device clock. Skip without an adapter
    /// or timestamp queries, never fake pass.
    #[test]
    fn gpu_bloom_dual_stays_inside_the_half_millisecond() {
        let Some(presenter) = init_or_skip("gpu_bloom_dual_stays_inside_the_half_millisecond")
        else {
            return;
        };
        if !presenter
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            crate::test_gpu::note_skip("bloom_dual_timing", "adapter lacks TIMESTAMP_QUERY");
            return;
        }
        let (best, steps) = measure_720p(&presenter);
        eprintln!(
            "bloom dual 720p on {}: {best:.4} ms over {steps} dispatches (budget {BLOOM_DUAL_BUDGET_MS} ms)",
            presenter.adapter_name()
        );
        assert!(best > 0.0, "the device clock advanced over the chain");
        assert!(
            best <= BLOOM_DUAL_BUDGET_MS,
            "the 720p chain stays inside {BLOOM_DUAL_BUDGET_MS} ms: {best}"
        );
    }
}
