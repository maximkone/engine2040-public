use super::PostError;

// One owner for these two helpers: `aa_scale::gpu` holds the bodies, and the
// public path `post::gpu::max_abs_diff` stays here as a re-export.
use crate::aa_scale::gpu::pack_pixels;
pub use crate::aa_scale::gpu::max_abs_diff;

/// TAA resolve chunk (WGSL): bilinear history reproject, 3x3 box clamp
/// around the current texel, velocity rejection, blend.
pub const TAA_RESOLVE_WGSL: &str = include_str!("../shaders/post/taa_resolve.wgsl");

/// Post chain chunk (WGSL): soft-knee bloom add, exposure, Narkowicz
/// ACES with white normalization, contrast/saturation grading.
pub const POST_CHAIN_WGSL: &str = include_str!("../shaders/post/post_chain.wgsl");

/// History color target descriptor: `Rgba32Float` render-attachment
/// plus sampler binding for the resolve pass.
pub fn history_desc(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("rp1-taa-history"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }
}

/// Post output target descriptor: `Rgba16Float` HDR staging target,
/// sampler-readable for the chain pass.
pub fn post_target_desc(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("rp1-post-target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }
}

/// Allocate the TAA history target on `device`. Real GPU work for the
/// presenter post pass; callers keep last-good on error.
pub fn allocate_history(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<wgpu::Texture, PostError> {
    check_history_dims(width, height)?;
    Ok(device.create_texture(&history_desc(width, height)))
}

/// Validate the history size before allocation: zero suspends (invalid),
/// over-bound degrades with `BudgetExceeded`.
pub fn check_history_dims(width: u32, height: u32) -> Result<(), PostError> {
    if width == 0 || height == 0 {
        return Err(PostError::Invalid("post GPU history size is zero".into()));
    }
    if width > super::MAX_POST_DIM || height > super::MAX_POST_DIM {
        return Err(PostError::BudgetExceeded {
            requested: width.max(height),
            allowed: super::MAX_POST_DIM,
        });
    }
    Ok(())
}

/// Posted CPU/GPU parity tolerances (halved-precision-free f32 path:
/// TAA is pure `+ - * /` plus `sqrt` for speed, so 1-ulp drift only;
/// the post chain adds the f64->f32 bloom accumulation drift).
/// Ghost/shimmer gates still apply unchanged on GPU pixels.
pub const GPU_PARITY_TOL_TAA: f32 = 3e-4;
/// Posted CPU/GPU parity tolerance for the full post chain output.
pub const GPU_PARITY_TOL_POST: f32 = 5e-3;
/// Posted GPU submit budget at test sizes in ms (evidence only).
pub const GPU_SUBMIT_BUDGET_MS: f64 = 16.0;

/// TAA compute shader: motion-vector reprojection with the
/// current-frame jitter offset, bilinear history sample, 3x3 box clamp
/// around the current texel, velocity rejection, blend. Mirrors
/// [`super::taa_resolve_jittered`] texel for texel.
pub const TAA_COMPUTE_WGSL: &str = include_str!("../shaders/post/taa_compute.wgsl");

/// Temporal upsample shader: clamp-to-edge bilinear from the render
/// size to the presentation size. Mirrors [`super::taa_upsample`].
pub const UPSAMPLE_COMPUTE_WGSL: &str = include_str!("../shaders/post/upsample_compute.wgsl");

/// Bloom extract shader: Kino soft-knee weight times the HDR texel.
/// Mirrors [`super::apply_bloom`] extract (`bloom_weight`).
pub const BRIGHT_COMPUTE_WGSL: &str = include_str!("../shaders/post/bright_compute.wgsl");

/// Downsample shader: one 2x2 average step (ceil halves), Karis-weighted
/// for the first mip and plain box after. Mirrors `downsample_karis` and
/// `downsample_box` (f32 accumulation; the CPU box uses f64, inside
/// [`GPU_PARITY_TOL_POST`]).
pub const DOWNSAMPLE_COMPUTE_WGSL: &str = include_str!("../shaders/post/downsample_compute.wgsl");

/// Composite shader: bilinear add-back of every halo mip (the caller
/// leaves the 2x2 and 1x1 veil out) with even intensity split, then
/// exposure, Narkowicz ACES with white
/// normalization, contrast/saturation grading. Mirrors
/// [`super::apply_bloom`] add-back plus [`super::tonemap_aces`] plus
/// [`super::apply_grade`]. All mip levels live in ONE concatenated
/// storage buffer (`mipsN = (w, h, texel_offset, 0)`); with the source
/// and the target that is three storage bindings total, inside every
/// native minimum. The six slots are unrolled under the `count` guard.
pub const COMPOSITE_COMPUTE_WGSL: &str = include_str!("../shaders/post/composite_compute.wgsl");

/// One GPU TAA resolve: pixels plus the validation outcome plus ms.
#[derive(Clone, Debug)]
pub struct GpuResolve {
    /// Resolved HDR pixels, row-major `width * height`.
    pub pixels: Vec<[f32; 3]>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode plus submit plus readback in ms.
    pub submit_ms: f64,
}

/// One GPU post frame: display pixels plus the frame receipt plus the
/// CPU/GPU parity figure. The receipt pass names match the RA2 graph
/// (`taa`, `taau`, `bloom`, `exposure`, `aces`, `grade`).
#[derive(Clone, Debug)]
pub struct GpuFrame {
    /// Display-space pixels, row-major `width * height`.
    pub ldr: Vec<[f32; 3]>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Per-pass receipts plus the RL1/RL2 light ms.
    pub receipt: super::FramePostReceipt,
    /// Max per-channel abs diff against the CPU [`super::run_frame`].
    pub parity_max_diff: f32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for the whole GPU frame in ms (evidence only).
    pub submit_ms: f64,
}

/// Reusable RP1 device state for the WIN1 match loop (and the ED1
/// viewport on the same shape): the five compute pipelines plus their
/// bind layouts. Built once; per-submit buffers are driver resources,
/// same as the RA1 twin, and sit outside any reuse guarantee. Consumes
/// [`super::build_rp1_graph`] on every full-frame submit so pass order
/// stays RA2-owned.
pub struct Rp1Submitter {
    taa_pipe: wgpu::ComputePipeline,
    up_pipe: wgpu::ComputePipeline,
    bright_pipe: wgpu::ComputePipeline,
    down_pipe: wgpu::ComputePipeline,
    comp_pipe: wgpu::ComputePipeline,
    taa_layout: wgpu::BindGroupLayout,
    up_layout: wgpu::BindGroupLayout,
    bright_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    comp_layout: wgpu::BindGroupLayout,
}

fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    wgsl: &str,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipe_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn storage_layout(device: &wgpu::Device, label: &str, counts: &[u32]) -> wgpu::BindGroupLayout {
    let mut entries = Vec::with_capacity(counts.len());
    for (i, kind) in counts.iter().enumerate() {
        // kind 0 = uniform, 1 = read-only storage, 2 = read-write storage.
        let ty = match kind {
            0 => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            1 => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            _ => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        });
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

fn pack_motion(flow: &[[f32; 2]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(flow.len() * 16);
    for m in flow {
        out.extend_from_slice(&m[0].to_le_bytes());
        out.extend_from_slice(&m[1].to_le_bytes());
        out.extend_from_slice(&0f32.to_le_bytes());
        out.extend_from_slice(&0f32.to_le_bytes());
    }
    out
}

fn storage_buf(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    })
}

/// Fixed-size little-endian uniform packer: the RP1 uniforms are constant
/// byte layouts, so they pack into a caller stack array instead of a heap
/// `Vec`. Pushing past `N` drops the excess, which leaves the block short
/// of its declared layout and fails loudly at bind.
struct Pack<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Pack<N> {
    fn new() -> Self {
        Self {
            bytes: [0u8; N],
            at: 0,
        }
    }

    fn u32(&mut self, value: u32) {
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            if let Some(slot) = self.bytes.get_mut(self.at + i) {
                *slot = *byte;
            }
        }
        self.at += 4;
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.at.min(N)]
    }
}

fn zero_storage(device: &wgpu::Device, label: &str, texels: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: texels * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

impl Rp1Submitter {
    /// Compile the five RP1 compute pipelines plus their bind layouts.
    /// No frame-size state: buffers size per submit.
    pub fn new(device: &wgpu::Device) -> Self {
        let taa_layout = storage_layout(device, "rp1-taa-layout", &[0, 1, 1, 1, 2]);
        let up_layout = storage_layout(device, "rp1-up-layout", &[0, 1, 2]);
        let bright_layout = storage_layout(device, "rp1-bright-layout", &[0, 1, 2]);
        let down_layout = storage_layout(device, "rp1-down-layout", &[0, 1, 2]);
        let comp_layout = storage_layout(device, "rp1-comp-layout", &[0, 1, 1, 2]);
        let taa_pipe = compute_pipeline(device, "rp1-taa", TAA_COMPUTE_WGSL, &taa_layout);
        let up_pipe = compute_pipeline(device, "rp1-up", UPSAMPLE_COMPUTE_WGSL, &up_layout);
        let bright_pipe =
            compute_pipeline(device, "rp1-bright", BRIGHT_COMPUTE_WGSL, &bright_layout);
        let down_pipe = compute_pipeline(device, "rp1-down", DOWNSAMPLE_COMPUTE_WGSL, &down_layout);
        let comp_pipe = compute_pipeline(device, "rp1-comp", COMPOSITE_COMPUTE_WGSL, &comp_layout);
        Self {
            taa_pipe,
            up_pipe,
            bright_pipe,
            down_pipe,
            comp_pipe,
            taa_layout,
            up_layout,
            bright_layout,
            down_layout,
            comp_layout,
        }
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffers: &[&wgpu::Buffer],
    ) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rp1-bind"),
            layout,
            entries: &entries,
        })
    }

    /// GPU TAA resolve over one frame: motion-vector reprojection with
    /// the jitter offset, bilinear history sample, 3x3 box clamp,
    /// velocity rejection, blend. One compute dispatch plus readback
    /// inside a validation scope; 0 scope hits means green. This path
    /// has no `Device` refusal variant gap: bad input fails loudly,
    /// device work reports pixels.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    pub fn resolve(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        current: &[[f32; 3]],
        history: &[[f32; 3]],
        motion: &super::MotionField,
        width: u32,
        height: u32,
        params: &super::TaaParams,
        jitter_px: [f32; 2],
    ) -> Result<GpuResolve, PostError> {
        params.validate()?;
        super::check_image(current, width, height, "taa current")?;
        super::check_image(history, width, height, "taa history")?;
        if motion.width != width || motion.height != height {
            return Err(PostError::Invalid(format!(
                "taa motion {}x{} differs from frame {width}x{height}",
                motion.width, motion.height
            )));
        }
        if !jitter_px.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid("taa jitter is not finite".into()));
        }
        if jitter_px[0].abs() > 1.0 || jitter_px[1].abs() > 1.0 {
            return Err(PostError::Invalid(format!(
                "taa jitter {jitter_px:?} outside +-1px"
            )));
        }
        let texels = width as usize * height as usize;
        let started = std::time::Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut uni = Vec::with_capacity(48);
        uni.extend_from_slice(&width.to_le_bytes());
        uni.extend_from_slice(&height.to_le_bytes());
        uni.extend_from_slice(&0u32.to_le_bytes());
        uni.extend_from_slice(&0u32.to_le_bytes());
        uni.extend_from_slice(&params.effective_blend().to_le_bytes());
        uni.extend_from_slice(&params.motion_threshold_px.to_le_bytes());
        uni.extend_from_slice(&(if params.use_clamp { 1f32 } else { 0f32 }).to_le_bytes());
        uni.extend_from_slice(&0f32.to_le_bytes());
        uni.extend_from_slice(&jitter_px[0].to_le_bytes());
        uni.extend_from_slice(&jitter_px[1].to_le_bytes());
        uni.extend_from_slice(&0f32.to_le_bytes());
        uni.extend_from_slice(&0f32.to_le_bytes());
        let (u_buf, cur_buf, hist_buf, mot_buf, dst_buf) = (
            crate::v1_gpu::create_uniform_buffer(device, "rp1-taa-u", &uni),
            storage_buf(device, "rp1-taa-cur", &pack_pixels(current)),
            storage_buf(device, "rp1-taa-hist", &pack_pixels(history)),
            storage_buf(device, "rp1-taa-mot", &pack_motion(&motion.flow)),
            zero_storage(device, "rp1-taa-dst", texels as u64),
        );
        let bind = self.bind(
            device,
            &self.taa_layout,
            &[&u_buf, &cur_buf, &hist_buf, &mot_buf, &dst_buf],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-taa"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp1-taa-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.taa_pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| PostError::Device(format!("poll: {e}")))?;
        let scope_err = pollster::block_on(scope.pop());
        // Copy out through a second encoder so the scope above covers
        // exactly the resolve dispatch.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-taa-copy"),
        });
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rp1-taa-readback"),
            size: texels as u64 * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&dst_buf, 0, &back, 0, texels as u64 * 16);
        queue.submit(Some(encoder.finish()));
        let slice = back.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| PostError::Device(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| PostError::Device(format!("map channel: {e}")))?
            .map_err(|e| PostError::Device(format!("map: {e}")))?;
        if let Some(err) = scope_err {
            return Err(PostError::Validation(format!("{err:?}")));
        }
        let mut pixels = Vec::with_capacity(texels);
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| PostError::Device(format!("mapped range: {e}")))?;
            for chunk in mapped.chunks_exact(16) {
                let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                let z = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return Err(PostError::Validation(
                        "gpu taa pixels are not finite".into(),
                    ));
                }
                pixels.push([x, y, z]);
            }
        }
        back.unmap();
        Ok(GpuResolve {
            pixels,
            width,
            height,
            validation_errors: 0,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Full RP1 frame on the device in compiled graph order: TAA at
    /// the render size, TAAU to the presentation size, bloom
    /// extract plus box-blur mip chain, then the composite
    /// (bloom add, exposure, ACES, grade). Consumes
    /// [`super::compile_rp1_for_present`] first so the six recorded
    /// dispatches always match the RA2 order; then compares against
    /// the CPU [`super::run_frame`] for the parity figure. One
    /// validation scope covers every dispatch; any hit fails loudly.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    pub fn run_frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        current: &[[f32; 3]],
        history: &[[f32; 3]],
        motion: &super::MotionField,
        render_w: u32,
        render_h: u32,
        present_w: u32,
        present_h: u32,
        taa: &super::TaaParams,
        chain: &super::PostChain,
        lights: &super::PostLights,
        jitter_px: [f32; 2],
    ) -> Result<GpuFrame, PostError> {
        taa.validate()?;
        chain.validate()?;
        let compiled =
            super::compile_rp1_for_present(render_w, render_h, present_w, present_h, chain)?;
        if compiled.order().len() != 6 {
            let passes = compiled.order().len();
            let reason = format!("rp1 compiled passes {passes} differ from 6"); // alloc-ok: cold refusal, never a steady frame
            return Err(PostError::Invalid(reason));
        }
        super::check_image(current, render_w, render_h, "taa current")?;
        super::check_image(history, render_w, render_h, "taa history")?;
        if motion.width != render_w || motion.height != render_h {
            let (mw, mh) = (motion.width, motion.height);
            let reason = format!("taa motion {mw}x{mh} differs from render {render_w}x{render_h}"); // alloc-ok: cold refusal, never a steady frame
            return Err(PostError::Invalid(reason));
        }
        let mult = chain.exposure.mult()?;
        let render_n = render_w as usize * render_h as usize;
        let present_n = present_w as usize * present_h as usize;
        let started = std::time::Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        // Stage 1 (taa): render-size resolve.
        let taa_started = std::time::Instant::now();
        let mut taa_uni = Pack::<48>::new();
        taa_uni.u32(render_w);
        taa_uni.u32(render_h);
        taa_uni.u32(0);
        taa_uni.u32(0);
        taa_uni.f32(taa.effective_blend());
        taa_uni.f32(taa.motion_threshold_px);
        taa_uni.f32(if taa.use_clamp { 1.0 } else { 0.0 });
        taa_uni.f32(0.0);
        taa_uni.f32(jitter_px[0]);
        taa_uni.f32(jitter_px[1]);
        taa_uni.f32(0.0);
        taa_uni.f32(0.0);
        let (taa_u, cur_buf, hist_buf, mot_buf, resolved_buf) = (
            crate::v1_gpu::create_uniform_buffer(device, "rp1-u-taa", taa_uni.as_slice()),
            storage_buf(device, "rp1-cur", &pack_pixels(current)),
            storage_buf(device, "rp1-hist", &pack_pixels(history)),
            storage_buf(device, "rp1-mot", &pack_motion(&motion.flow)),
            zero_storage(device, "rp1-resolved", render_n as u64),
        );
        let taa_bind = self.bind(
            device,
            &self.taa_layout,
            &[&taa_u, &cur_buf, &hist_buf, &mot_buf, &resolved_buf],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-taa"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp1-taa-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.taa_pipe);
            pass.set_bind_group(0, &taa_bind, &[]);
            pass.dispatch_workgroups(render_w.div_ceil(8), render_h.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        let taa_ms = taa_started.elapsed().as_secs_f64() * 1000.0;
        // Stage 2 (taau): render -> present bilinear.
        let up_started = std::time::Instant::now();
        let sx = render_w as f32 / present_w as f32;
        let sy = render_h as f32 / present_h as f32;
        let mut up_uni = Pack::<32>::new();
        up_uni.u32(render_w);
        up_uni.u32(render_h);
        up_uni.u32(present_w);
        up_uni.u32(present_h);
        up_uni.f32(sx);
        up_uni.f32(sy);
        up_uni.f32(0.0);
        up_uni.f32(0.0);
        let (up_u, up_buf) = (
            crate::v1_gpu::create_uniform_buffer(device, "rp1-u-up", up_uni.as_slice()),
            zero_storage(device, "rp1-upsampled", present_n as u64),
        );
        let up_bind = self.bind(device, &self.up_layout, &[&up_u, &resolved_buf, &up_buf]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-taau"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp1-taau-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.up_pipe);
            pass.set_bind_group(0, &up_bind, &[]);
            pass.dispatch_workgroups(present_w.div_ceil(8), present_h.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        let up_ms = up_started.elapsed().as_secs_f64() * 1000.0;
        // Stage 3 (bloom): bright extract then box-blur mip chain.
        let bloom_started = std::time::Instant::now();
        let mut bright_uni = Pack::<32>::new();
        bright_uni.f32(chain.bloom.threshold);
        bright_uni.f32(chain.bloom.knee);
        bright_uni.f32(0.0);
        bright_uni.f32(0.0);
        bright_uni.u32(present_w);
        bright_uni.u32(present_h);
        bright_uni.u32(0);
        bright_uni.u32(0);
        let (bright_u, bright_buf) = (
            crate::v1_gpu::create_uniform_buffer(device, "rp1-u-bright", bright_uni.as_slice()),
            zero_storage(device, "rp1-bright", present_n as u64),
        );
        let bright_bind = self.bind(
            device,
            &self.bright_layout,
            &[&bright_u, &up_buf, &bright_buf],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-bright"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp1-bright-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bright_pipe);
            pass.set_bind_group(0, &bright_bind, &[]);
            pass.dispatch_workgroups(present_w.div_ceil(8), present_h.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        // Box-blur mip chain: each level halves the previous bright mip,
        // mirroring `downsample_box` (ceil halves, stop at 1x1). The
        // chain vector owns every buffer: index 0 is the bright extract
        // (dropped before the composite), the rest are the sampled mips
        // in order. Borrows end at bind-group creation, so pushing the
        // next dst after each submit is borrow-clean.
        let plan = super::BloomPassPlan::for_frame(present_h, &chain.bloom)?;
        let mut chain_bufs: Vec<wgpu::Buffer> = vec![bright_buf]; // alloc-ok: run_frame probe staging with readback, not the window steady frame
        let mut chain_sizes: Vec<(u32, u32)> = vec![(present_w, present_h)]; // alloc-ok: run_frame probe staging with readback, not the window steady frame
        for level in 0..plan.depth {
            let (lw, lh) = chain_sizes[chain_sizes.len() - 1];
            let nw = lw.div_ceil(2).max(1);
            let nh = lh.div_ceil(2).max(1);
            // The first downsample is Karis-weighted (mode 1), later
            // levels are plain box averages (mode 0).
            let karis = u32::from(level == 0);
            let mut down_uni = Pack::<32>::new();
            down_uni.u32(lw);
            down_uni.u32(lh);
            down_uni.u32(nw);
            down_uni.u32(nh);
            down_uni.u32(karis);
            down_uni.u32(0);
            down_uni.u32(0);
            down_uni.u32(0);
            let down_u =
                crate::v1_gpu::create_uniform_buffer(device, "rp1-u-down", down_uni.as_slice());
            let dst = zero_storage(device, "rp1-mip", nw as u64 * nh as u64);
            {
                let src = &chain_bufs[chain_bufs.len() - 1];
                let down_bind = self.bind(device, &self.down_layout, &[&down_u, src, &dst]);
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rp1-down"),
                });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("rp1-down-pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.down_pipe);
                    pass.set_bind_group(0, &down_bind, &[]);
                    pass.dispatch_workgroups(nw.div_ceil(8), nh.div_ceil(8), 1);
                }
                queue.submit(Some(encoder.finish()));
            }
            chain_bufs.push(dst);
            chain_sizes.push((nw, nh));
            if nw == 1 && nh == 1 {
                break;
            }
        }
        // Drop the bright extract: the CPU add-back samples the chain
        // mips only, never the extract itself.
        chain_bufs.remove(0);
        chain_sizes.remove(0);
        // The add-back skips the veil: a mip at or below 2x2 covers the
        // whole frame and would lift far darks, so only the halo mips
        // (some side above 2) reach the composite, which splits the
        // intensity over their count. Sizes only shrink along the
        // chain, so the halo mips are a prefix.
        let halo = chain_sizes
            .iter()
            .take_while(|(w, h)| *w > 2 || *h > 2)
            .count();
        chain_bufs.truncate(halo);
        chain_sizes.truncate(halo);
        let mip_bufs = chain_bufs;
        let mip_sizes = chain_sizes;
        let bloom_ms = bloom_started.elapsed().as_secs_f64() * 1000.0;
        // Stage 4..6 (exposure/aces/grade): single composite dispatch.
        // The mip levels are concatenated into one storage buffer first
        // (offsets in texels, packed as `mipsN = (w, h, off, 0)`), so
        // the composite binds three storage buffers total: source,
        // mips, target.
        let comp_started = std::time::Instant::now();
        let mut mip_total: u64 = 0;
        let mut mip_offs: Vec<u64> = Vec::with_capacity(mip_bufs.len()); // alloc-ok: run_frame probe staging with readback, not the window steady frame
        for (w, h) in mip_sizes.iter() {
            mip_offs.push(mip_total);
            mip_total += *w as u64 * *h as u64;
        }
        let mips_buf = zero_storage(device, "rp1-mips", mip_total.max(1));
        let mut full_sizes = [(1u32, 1u32, 0u64); 6];
        for (i, s) in mip_sizes.iter().enumerate().take(6) {
            full_sizes[i] = (s.0, s.1, mip_offs[i]);
        }
        let count = mip_sizes.len().min(6) as u32;
        let mut comp_uni = Pack::<160>::new();
        comp_uni.u32(present_w);
        comp_uni.u32(present_h);
        comp_uni.u32(count);
        comp_uni.u32(0);
        for (w, h, off) in full_sizes {
            comp_uni.u32(w);
            comp_uni.u32(h);
            comp_uni.u32(off as u32);
            comp_uni.u32(0);
        }
        comp_uni.f32(chain.bloom.intensity);
        comp_uni.f32(mult);
        comp_uni.f32(chain.aces_white);
        comp_uni.f32(chain.grade.contrast);
        comp_uni.f32(chain.grade.saturation);
        comp_uni.f32(chain.grade.lift[0]);
        comp_uni.f32(chain.grade.lift[1]);
        comp_uni.f32(chain.grade.lift[2]);
        comp_uni.f32(chain.grade.gain[0]);
        comp_uni.f32(chain.grade.gain[1]);
        comp_uni.f32(chain.grade.gain[2]);
        comp_uni.f32(0.0);
        let (comp_u, ldr_buf) = (
            crate::v1_gpu::create_uniform_buffer(device, "rp1-u-comp", comp_uni.as_slice()),
            zero_storage(device, "rp1-ldr", present_n as u64),
        );
        let comp_bind = self.bind(
            device,
            &self.comp_layout,
            &[&comp_u, &up_buf, &mips_buf, &ldr_buf],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-comp"),
        });
        // Stage the mip levels into the concatenated buffer first;
        // encoder order is execution order, so the pass below reads
        // settled bytes.
        for (buf, off) in mip_bufs.iter().zip(mip_offs.iter()) {
            let bytes = buf.size();
            encoder.copy_buffer_to_buffer(buf, 0, &mips_buf, off * 16, bytes);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp1-comp-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.comp_pipe);
            pass.set_bind_group(0, &comp_bind, &[]);
            pass.dispatch_workgroups(present_w.div_ceil(8), present_h.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| PostError::Device(format!("poll: {e}")))?; // alloc-ok: cold device-loss path, never a steady frame
        let scope_err = pollster::block_on(scope.pop());
        if let Some(err) = scope_err {
            return Err(PostError::Validation(format!("{err:?}"))); // alloc-ok: cold validation failure, never a steady frame
        }
        let comp_ms_total = comp_started.elapsed().as_secs_f64() * 1000.0;
        // Single readback of the LDR buffer.
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rp1-ldr-readback"),
            size: present_n as u64 * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp1-ldr-copy"),
        });
        encoder.copy_buffer_to_buffer(&ldr_buf, 0, &back, 0, present_n as u64 * 16);
        queue.submit(Some(encoder.finish()));
        let slice = back.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| PostError::Device(format!("poll: {e}")))?; // alloc-ok: cold device-loss path, never a steady frame
        rx.recv()
            .map_err(|e| PostError::Device(format!("map channel: {e}")))? // alloc-ok: cold device-loss path, never a steady frame
            .map_err(|e| PostError::Device(format!("map: {e}")))?; // alloc-ok: cold device-loss path, never a steady frame
        let mut ldr = Vec::with_capacity(present_n); // alloc-ok: run_frame probe staging with readback, not the window steady frame
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| PostError::Device(format!("mapped range: {e}")))?; // alloc-ok: cold device-loss path, never a steady frame
            for chunk in mapped.chunks_exact(16) {
                let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                let z = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return Err(PostError::Validation(
                        "gpu ldr pixels are not finite".into(),
                    ));
                }
                ldr.push([x, y, z]);
            }
        }
        back.unmap();
        // CPU parity: the oracle runs on the same inputs.
        let cpu = super::run_frame(
            current, history, motion, render_w, render_h, present_w, present_h, taa, chain, lights,
        )?;
        let mut parity_max_diff = 0f32;
        for (a, b) in cpu.ldr.iter().zip(ldr.iter()) {
            for c in 0..3 {
                parity_max_diff = parity_max_diff.max((a[c] - b[c]).abs());
            }
        }
        let submit_ms = started.elapsed().as_secs_f64() * 1000.0;
        let render_px = render_w as u64 * render_h as u64;
        let present_px = present_w as u64 * present_h as u64;
        let third = comp_ms_total / 3.0;
        Ok(GpuFrame {
            width: present_w,
            height: present_h,
            ldr,
            receipt: super::FramePostReceipt {
                taa: super::PassReceipt {
                    pass: "taa",
                    pixels: render_px,
                    ms: taa_ms,
                },
                upsample: super::PassReceipt {
                    pass: "taau",
                    pixels: present_px,
                    ms: up_ms,
                },
                bloom: super::PassReceipt {
                    pass: "bloom",
                    pixels: present_px,
                    ms: bloom_ms,
                },
                exposure: super::PassReceipt {
                    pass: "exposure",
                    pixels: present_px,
                    ms: third,
                },
                aces: super::PassReceipt {
                    pass: "aces",
                    pixels: present_px,
                    ms: third,
                },
                grade: super::PassReceipt {
                    pass: "grade",
                    pixels: present_px,
                    ms: third,
                },
                sun_ms: lights.sun_ms,
                cluster_ms: lights.cluster_ms,
                ibl_ms: lights.ibl_ms,
            },
            parity_max_diff,
            validation_errors: 0,
            submit_ms,
        })
    }
}

