//! LF5c: the live TAA resolve, the pass between the forward and the blit.
//!
//! Owner: render lane (LF5c row), consumer the B9 TAA capture pair that
//! flips this switch and the LF5b jitter switch together. One full-screen
//! fragment pass reads this frame's HDR plus the motion target and last
//! frame's resolved HDR, reprojects the history by the motion vector,
//! clamps it to the current texel's 3x3 YCoCg neighbourhood, then blends
//! ten percent current (history weight [`crate::post::DEFAULT_HISTORY_BLEND`]).
//! The history pair and its bind groups are built with the targets, never
//! per frame; a target rebuild (resize, surface loss) drops the history and
//! the next frame follows the current image. `reset_taa_history` lets the
//! game drop it on a camera cut. Presentation only, never hashed.
//!
//! The resolve stays off until the switch is on: the off path records no
//! pass and the blit keeps reading `hdr_view`, so every earlier frame stays
//! byte-identical. With 4x MSAA on, the motion target is cleared (LF5 has
//! no 4x motion twin), so the resolve holds the history where nothing moves;
//! the accept pair runs on the default 1x path.

/// The live resolve shader.
const TAA_WGSL: &str = include_str!("../../shaders/taa_present.wgsl");
/// Two history images: the resolve reads one and writes the other, then the
/// pair swaps, so no copy ever feeds the next frame.
const HISTORY_IMAGES: usize = 2;

/// One-line receipt for the capture pair: switch, history state, passes and
/// the live frame scope's late validation count.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TaaResolveReceipt {
    /// The resolve switch.
    pub enabled: bool,
    /// The history holds a resolved frame (a reset or rebuild drops it).
    pub history_valid: bool,
    /// Resolve passes recorded since the presenter opened.
    pub passes: u64,
    /// History weight the shader mixes (0.9 keeps ten percent current).
    pub blend: f32,
    /// The frame scope's deferred validation errors, always 0 on a clean run.
    pub validation_errors: u64,
}

impl TaaResolveReceipt {
    /// One line for the capture receipt, stable field order.
    pub fn line(&self) -> String {
        format!(
            "taa={} history={} passes={} blend={:.2} validation_errors={}",
            if self.enabled { "on" } else { "off" },
            if self.history_valid {
                "valid"
            } else {
                "dropped"
            },
            self.passes,
            self.blend,
            self.validation_errors,
        )
    }
}

/// The blit resources the resolved-view binds mirror: same layout and
/// sampler the HDR blit bind uses, over each TAA output instead of `hdr_view`.
pub(crate) struct BlitFeed<'a> {
    /// The presenter's blit bind group layout.
    pub(crate) layout: &'a wgpu::BindGroupLayout,
    /// The presenter's linear blit sampler.
    pub(crate) sampler: &'a wgpu::Sampler,
    /// This target's fog factor view.
    pub(crate) fog: &'a wgpu::TextureView,
    /// This target's contact-AO view.
    pub(crate) ao: &'a wgpu::TextureView,
    /// This target's bloom halo view (half-res, rung 0).
    pub(crate) bloom: &'a wgpu::TextureView,
    /// This target's fog color plus size uniform.
    pub(crate) fog_col: &'a wgpu::Buffer,
}

/// The resolve's per-target resources, shared by both history binds.
struct ResolveFeed<'a> {
    layout: &'a wgpu::BindGroupLayout,
    current: &'a wgpu::TextureView,
    motion: &'a wgpu::TextureView,
    history_sampler: &'a wgpu::Sampler,
    motion_sampler: &'a wgpu::Sampler,
    uniform: &'a wgpu::Buffer,
}

/// Sized history pair plus the binds that read each image: built with the
/// targets, dropped with them.
struct TaaSized {
    views: [wgpu::TextureView; HISTORY_IMAGES],
    /// Resolve binds: entry `i` reads image `i` as history.
    resolve_binds: [wgpu::BindGroup; HISTORY_IMAGES],
    /// Blit binds over image `i`, for the frame that wrote it.
    blit_binds: [wgpu::BindGroup; HISTORY_IMAGES],
}

/// The live resolve state: switch, history validity, ping-pong parity and the
/// device resources (layout, pipeline, two samplers, one uniform).
pub(crate) struct TaaResolve {
    enabled: bool,
    history_valid: bool,
    /// True when the pass went into this frame's encoder.
    ran: bool,
    /// The history validity the last recorded pass read.
    last_frame_valid: bool,
    /// Index of the image holding the last resolved frame.
    last: usize,
    passes: u64,
    blend: f32,
    threshold: f32,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    history_sampler: wgpu::Sampler,
    motion_sampler: wgpu::Sampler,
    sized: Option<TaaSized>,
}

impl TaaResolve {
    /// Build the device resources once; the off default records no pass.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        // The CPU oracle's defaults are the one source for the two blends:
        // 0.9 keeps ten percent current, 32 px rejects a teleport's history.
        let params = crate::post::TaaParams::default();
        let layout = resolve_layout(device);
        let (history_sampler, motion_sampler) = resolve_samplers(device);
        Self {
            enabled: false,
            history_valid: false,
            ran: false,
            last_frame_valid: false,
            last: 0,
            passes: 0,
            blend: params.history_blend,
            threshold: params.motion_threshold_px,
            pipeline: resolve_pipeline(device, &layout),
            uniform: resolve_uniform(device),
            layout,
            history_sampler,
            motion_sampler,
            sized: None,
        }
    }

    /// Build the history pair and its binds for this target size. A target
    /// rebuild calls this again, dropping the stale history.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        current: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        width: u32,
        height: u32,
        blit: BlitFeed<'_>,
    ) {
        let views = history_views(device, width, height);
        let feed = ResolveFeed {
            layout: &self.layout,
            current,
            motion,
            history_sampler: &self.history_sampler,
            motion_sampler: &self.motion_sampler,
            uniform: &self.uniform,
        };
        let resolve_binds = std::array::from_fn(|index| resolve_bind(device, &feed, &views[index]));
        let blit_binds =
            std::array::from_fn(|index| target_blit_bind(device, &blit, &views[index]));
        self.sized = Some(TaaSized {
            views,
            resolve_binds,
            blit_binds,
        });
        self.history_valid = false;
        self.ran = false;
        self.last = 0;
    }

    /// Record the resolve for this frame, or nothing when the switch is off
    /// or the targets are not built: one full-screen pass, no readback.
    /// Callers run it after the forward pass and before they borrow a sink
    /// view: the `&mut self` here cannot overlap a `&self` sink borrow.
    pub(crate) fn encode(
        &mut self,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
    ) {
        self.ran = false;
        self.last_frame_valid = false;
        if !self.enabled {
            return;
        }
        let Some(sized) = self.sized.as_ref() else {
            return;
        };
        let history = self.last;
        let output = (history + 1) % HISTORY_IMAGES;
        self.last_frame_valid = self.history_valid;
        queue.write_buffer(
            &self.uniform,
            0,
            &uniform_bytes(
                width,
                height,
                self.blend,
                self.threshold,
                self.history_valid,
            ),
        );
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("taa-resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sized.views[output],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &sized.resolve_binds[history], &[]);
            pass.draw(0..3, 0..1);
        }
        self.last = output;
        self.history_valid = true;
        self.ran = true;
        self.passes = self.passes.saturating_add(1);
    }

    /// Turn the pass on or off; turning it on starts from no history.
    pub(crate) fn set_enabled(&mut self, on: bool) {
        if on && !self.enabled {
            self.history_valid = false;
        }
        self.enabled = on;
        self.ran = false;
    }

    /// Drop the history: a camera cut, a teleport or a scene swap. The next
    /// resolve follows the current frame outright.
    pub(crate) fn invalidate(&mut self) {
        self.history_valid = false;
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// The image this frame's resolve wrote, for the meter that must read it.
    pub(crate) fn resolved_view(&self) -> Option<&wgpu::TextureView> {
        if !self.ran {
            return None;
        }
        self.sized.as_ref().map(|sized| &sized.views[self.last])
    }

    /// The blit bind over the image this frame's resolve wrote.
    pub(crate) fn resolved_blit_bind(&self) -> Option<&wgpu::BindGroup> {
        if !self.ran {
            return None;
        }
        self.sized
            .as_ref()
            .map(|sized| &sized.blit_binds[self.last])
    }

    pub(crate) fn receipt(&self) -> TaaResolveReceipt {
        TaaResolveReceipt {
            enabled: self.enabled,
            history_valid: self.history_valid,
            passes: self.passes,
            blend: self.blend,
            validation_errors: 0,
        }
    }

    /// Whether the last recorded pass read a valid history: the uniform's
    /// validity word. The reset tests prove a cut or a resize drops it.
    #[cfg(test)]
    pub(crate) fn last_frame_valid(&self) -> bool {
        self.last_frame_valid
    }
}

impl crate::present::Presenter {
    /// Turn the live TAA resolve on or off. Off (the default) records no pass
    /// and keeps the blit on `hdr_view`, so the off frame is byte-identical to
    /// the pre-LF5c path. Turning it on drops any stale history.
    pub fn set_taa_resolve_enabled(&mut self, on: bool) {
        self.taa.set_enabled(on);
    }

    /// The resolve switch as the live frame reads it.
    pub fn taa_resolve_enabled(&self) -> bool {
        self.taa.enabled()
    }

    /// Drop the resolve history: call on a camera cut, a teleport or a scene
    /// swap. A resize drops it by itself (the targets rebuild).
    pub fn reset_taa_history(&mut self) {
        self.taa.invalidate();
    }

    /// One-line plus fields receipt for the capture pair. The validation count
    /// is the live frame scope's, so a clean run prints zero.
    pub fn taa_resolve_receipt(&self) -> TaaResolveReceipt {
        TaaResolveReceipt {
            validation_errors: self.deferred_validation_errors(),
            ..self.taa.receipt()
        }
    }

    /// Whether the last recorded frame ran with valid history: false on the
    /// first frame, after `reset_taa_history` and after a target rebuild.
    /// Test and receipt evidence; it reads the word the frame submitted.
    #[cfg(test)]
    pub(crate) fn taa_last_frame_valid(&self) -> bool {
        self.taa.last_frame_valid()
    }

    /// The blit bind for this frame: the resolve output when the pass ran,
    /// else the target's HDR bind.
    pub(super) fn blit_bind_for<'a>(
        &'a self,
        targets: &'a super::frame::Targets,
    ) -> &'a wgpu::BindGroup {
        self.taa.resolved_blit_bind().unwrap_or(&targets.blit_bind)
    }

    /// The HDR view the exposure meter reads: the resolved image on a frame
    /// where the resolve ran, else the target's own HDR view.
    pub(super) fn meter_hdr_view(&self) -> Option<wgpu::TextureView> {
        match self.taa.resolved_view() {
            Some(view) => Some(view.clone()),
            None => self
                .targets
                .as_ref()
                .map(|targets| targets.hdr_view.clone()),
        }
    }
}

/// The resolve bind group layout: current, history, its sampler, motion, its
/// sampler and the frame uniform.
fn resolve_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("taa-resolve-bind"),
        entries: &[
            texture_entry(0),
            texture_entry(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_entry(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

/// The full-screen pass into one HDR history image, replacement blend.
fn resolve_pipeline(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("taa-resolve"),
        source: wgpu::ShaderSource::Wgsl(TAA_WGSL.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("taa-resolve-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("taa-resolve-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: super::HDR_FORMAT,
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

/// Linear history sampling (the reprojection lands between texels) and
/// nearest motion sampling (a velocity never bleeds from its neighbour).
fn resolve_samplers(device: &wgpu::Device) -> (wgpu::Sampler, wgpu::Sampler) {
    let history = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("taa-history-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let motion = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("taa-motion-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (history, motion)
}

/// The 32-byte resolve uniform, written once per resolved frame.
fn resolve_uniform(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("taa-resolve-uniform"),
        size: 32, // the `TaaU` uniform: size pair, blend, threshold, validity, pads
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Two HDR images the resolve ping-pongs between; the views own them.
fn history_views(device: &wgpu::Device, width: u32, height: u32) -> [wgpu::TextureView; 2] {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    std::array::from_fn(|index| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(if index == 0 {
                "taa-history-a"
            } else {
                "taa-history-b"
            }),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: super::HDR_FORMAT,
            // Sampled as history and by the meter, written as the resolve
            // output.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    })
}

/// One history image's resolve bind: the same current plus motion, this image
/// as history.
fn resolve_bind(
    device: &wgpu::Device,
    feed: &ResolveFeed<'_>,
    history: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("taa-resolve-bind"),
        layout: feed.layout,
        entries: &[
            texture_bind(0, feed.current),
            texture_bind(1, history),
            sampler_bind(2, feed.history_sampler),
            texture_bind(3, feed.motion),
            sampler_bind(4, feed.motion_sampler),
            wgpu::BindGroupEntry {
                binding: 5,
                resource: feed.uniform.as_entire_binding(),
            },
        ],
    })
}

/// The blit bind over one resolved image: HDR slot 0 plus the target's fog
/// pair and bloom halo, exactly the layout the HDR blit bind uses.
fn target_blit_bind(
    device: &wgpu::Device,
    feed: &BlitFeed<'_>,
    resolved: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("taa-blit-bind"),
        layout: feed.layout,
        entries: &[
            texture_bind(0, resolved),
            sampler_bind(1, feed.sampler),
            texture_bind(2, feed.fog),
            texture_bind(3, feed.ao),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: feed.fog_col.as_entire_binding(),
            },
            texture_bind(5, feed.bloom),
        ],
    })
}

/// A filterable float texture entry at `binding` (HDR, history and motion are
/// all filterable formats; the motion sampler itself stays nearest).
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn texture_bind<'a>(binding: u32, view: &'a wgpu::TextureView) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn sampler_bind<'a>(binding: u32, sampler: &'a wgpu::Sampler) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Sampler(sampler),
    }
}

/// The 32-byte resolve uniform: size, history weight, velocity reject in
/// pixels, validity and three pad words.
fn uniform_bytes(width: u32, height: u32, blend: f32, threshold: f32, valid: bool) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&width.to_le_bytes());
    bytes[4..8].copy_from_slice(&height.to_le_bytes());
    bytes[8..12].copy_from_slice(&blend.to_le_bytes());
    bytes[12..16].copy_from_slice(&threshold.to_le_bytes());
    bytes[16..20].copy_from_slice(&f32::from(u8::from(valid)).to_le_bytes());
    bytes
}
