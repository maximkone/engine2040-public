//! The device half of the auto-exposure meter: one compute dispatch over
//! the pre-exposure HDR target, 64 atomic bins, and a one-frame-late
//! async readback the next frame ticks.
//!
//! Owner: render lane, consumer the live presenter frame and the
//! `post::auto_exposure` state. Units: linear HDR texels in, bin counts
//! out. Determinism: the kernel mirrors the CPU oracle sample for sample
//! (same stride pattern, same Rec.709 luma, same 0.3125 stops per bin),
//! so one frame bins to the same counts on either side.
//!
//! The readback is one frame late on purpose: the live frame never waits
//! on the GPU. The dispatch of a frame lands in the bins of that frame,
//! the map of those bins is picked up by the frame after it, and the
//! exposure of that frame rides the meter of the frame before it. A
//! frame whose map has not landed keeps the stops it had; the meter
//! never guesses.
//!
//! Provenance: engine-owned code (no port markers). The donor shape
//! (histogram plus temporal adaptation) is the Bevy auto-exposure
//! method, read as a method only; no donor bytes.

use crate::post::auto_exposure::{hist_stride, ExposureHistogram, AUTO_HIST_BINS};
use crate::present::PresentError;

/// The histogram kernel: 64 log-luma bins over -12..+8 EV, one atomic
/// add per sampled texel.
pub(super) const AUTO_EXPOSURE_HIST_WGSL: &str =
    include_str!("../../shaders/auto_exposure_hist.wgsl");

/// Bytes of the bin buffer: 64 u32 counters.
const BIN_COUNT: u64 = 64;
/// Bytes of one bin: one u32 counter.
const BIN_BYTES: u64 = 4;
/// Bytes of the histogram uniform: size (2 u32), stride (u32), pad (u32).
const UNIFORM_BYTES: u64 = 16;
/// Texels per workgroup side in the kernel.
const WORKGROUP: u32 = 8;

/// Bin buffer bytes for the 64 counters.
fn bin_bytes() -> u64 {
    BIN_COUNT * BIN_BYTES
}

/// The device meter: the kernel, its buffers and the in-flight map.
pub(super) struct AutoExposureGpu {
    /// The device the kernel and the buffers live on.
    device: wgpu::Device,
    /// The queue the uniform rides.
    queue: wgpu::Queue,
    /// The histogram compute pipeline over the HDR target.
    pipe: wgpu::ComputePipeline,
    /// Bind layout: uniform, HDR texture, bin storage.
    layout: wgpu::BindGroupLayout,
    /// 64 atomic counters, cleared before every dispatch.
    bins: wgpu::Buffer,
    /// The dispatch uniform: frame size and sample stride.
    uniform: wgpu::Buffer,
    /// The readback buffer the bins are copied into.
    staging: wgpu::Buffer,
    /// Bind group over the HDR view, rebuilt when the frame size moves.
    bind: Option<wgpu::BindGroup>,
    /// The frame size the bind group was built for.
    bind_size: (u32, u32),
    /// The map in flight; `None` when the staging buffer is free.
    pending: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

impl AutoExposureGpu {
    /// Build the kernel, its layout and the three buffers. No frame state:
    /// the buffers size once and the bind group rides the frame size.
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ra1-auto-exposure-hist"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        // The kernel reads texels, never samples: an
                        // unfilterable float binding takes the half-float
                        // HDR target and the full-float test target alike.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ra1-auto-exposure-hist"),
            source: wgpu::ShaderSource::Wgsl(AUTO_EXPOSURE_HIST_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ra1-auto-exposure-hist"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ra1-auto-exposure-hist"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("hist_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-auto-exposure-bins"),
            size: bin_bytes(),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-auto-exposure-u"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-auto-exposure-readback"),
            size: bin_bytes(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipe,
            layout,
            bins,
            uniform,
            staging,
            bind: None,
            bind_size: (0, 0),
            pending: None,
        }
    }

    /// Encode the histogram dispatch over the HDR target: clear the bins,
    /// run the kernel on the frame's sample pattern, copy the bins into
    /// the readback buffer. Returns false when the frame is skipped: a
    /// map is still in flight, or the frame has no texels.
    pub(super) fn encode(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> bool {
        if self.pending.is_some() || width == 0 || height == 0 {
            return false;
        }
        let stride = hist_stride(width, height);
        if self.bind.is_none() || self.bind_size != (width, height) {
            let bind = self.bind_for(view);
            self.bind = Some(bind);
            self.bind_size = (width, height);
        }
        let Some(bind) = self.bind.as_ref() else {
            return false;
        };
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&width.to_le_bytes());
        bytes[4..8].copy_from_slice(&height.to_le_bytes());
        bytes[8..12].copy_from_slice(&stride.to_le_bytes());
        self.queue.write_buffer(&self.uniform, 0, &bytes);
        enc.clear_buffer(&self.bins, 0, Some(bin_bytes()));
        let groups_x = width.div_ceil(stride).div_ceil(WORKGROUP);
        let groups_y = height.div_ceil(stride).div_ceil(WORKGROUP);
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ra1-auto-exposure-hist"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }
        enc.copy_buffer_to_buffer(&self.bins, 0, &self.staging, 0, bin_bytes());
        true
    }

    /// Build the bind group over the HDR view. Cold path only: the group
    /// is rebuilt when the frame size moves, never per frame.
    fn bind_for(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ra1-auto-exposure-hist-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.bins.as_entire_binding(),
                },
            ],
        })
    }

    /// Start the map of the copied bins. Called after the submit that
    /// carried the copy; the map lands on its own, without a wait.
    pub(super) fn begin_map(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        self.staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.pending = Some(receiver);
    }

    /// The histogram of the frame before this one, when its map has
    /// landed. `None` while the map is in flight or when it failed: the
    /// frame keeps the stops it had.
    pub(super) fn take_ready(&mut self) -> Result<Option<ExposureHistogram>, PresentError> {
        let Some(receiver) = self.pending.as_ref() else {
            return Ok(None);
        };
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| PresentError::Readback(format!("auto-exposure poll: {error}")))?;
        match receiver.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.pending = None;
                return Err(PresentError::Readback(format!(
                    "auto-exposure map: {error}"
                )));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                return Ok(None);
            }
        }
        let counts = self.read_counts()?;
        self.pending = None;
        self.staging.unmap();
        ExposureHistogram::from_gpu_counts(&counts)
            .map(Some)
            .map_err(|error| PresentError::Readback(format!("auto-exposure histogram: {error}")))
    }

    /// The 64 counters of the mapped readback buffer.
    fn read_counts(&self) -> Result<[u32; AUTO_HIST_BINS], PresentError> {
        let mut counts = [0u32; AUTO_HIST_BINS];
        let mapped = self
            .staging
            .slice(..)
            .get_mapped_range()
            .map_err(|error| PresentError::Readback(format!("auto-exposure range: {error}")))?;
        for (slot, chunk) in mapped.chunks_exact(4).enumerate() {
            if slot >= AUTO_HIST_BINS {
                break;
            }
            counts[slot] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        Ok(counts)
    }
}

// ---------------------------------------------------------------------------
// Tests: the device histogram against the CPU oracle, and the readback
// contract (the meter never blocks and never guesses).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::auto_exposure::histogram_log_luma;

    /// The live-frame fixture size in pixels.
    const SIDE: u32 = 128;

    /// The device the meter runs on; no adapter maps to a routed skip.
    fn test_device() -> Result<(wgpu::Device, wgpu::Queue), crate::test_gpu::NoAdapter> {
        crate::test_gpu::hold_gpu_slot();
        let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|error| {
            crate::test_gpu::NoAdapter(format!("no wgpu adapter on this machine: {error}"))
        })?;
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("auto-exposure-test"),
            ..Default::default()
        })) {
            Ok(pair) => Ok(pair),
            Err(error) => panic!("test device requests succeed when the adapter exists: {error}"),
        }
    }

    /// The fixture: a 64x64 frame whose lumas are exact powers of two
    /// that never land on a bin edge (a luma whose scaled position is a
    /// whole number would let a one-ulp difference move a sample from
    /// one bin to the next).
    fn fixture() -> (Vec<[f32; 3]>, u32, u32) {
        let (width, height) = (64u32, 64u32);
        let levels = [0.0625f32, 0.125, 0.5, 1.0, 2.0, 4.0, 16.0, 32.0];
        let mut frame = Vec::with_capacity(
            usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0),
        );
        for y in 0..height {
            for x in 0..width {
                let step = (x / 8 + y / 8) % u32::try_from(levels.len()).unwrap_or(1);
                let luma = levels[usize::try_from(step).unwrap_or(0)];
                frame.push([luma, luma, luma]);
            }
        }
        (frame, width, height)
    }

    /// The device histogram over the HDR target bins the frame exactly
    /// like the CPU oracle: the same samples, the same bins, the same
    /// trimmed mean.
    #[test]
    fn auto_exposure_gpu_histogram_matches_the_cpu_oracle() {
        let Some((device, queue)) = crate::test_gpu::require_or_skip(
            "auto_exposure_gpu_histogram_matches_the_cpu_oracle",
            test_device(),
        ) else {
            return;
        };
        let (frame, width, height) = fixture();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("auto-exposure-hist-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut bytes = Vec::with_capacity(
            usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0) * 16,
        );
        for p in &frame {
            for c in p {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            bytes.extend_from_slice(&1.0f32.to_le_bytes());
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut meter = AutoExposureGpu::new(&device, &queue);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("auto-exposure-hist"),
        });
        assert!(
            meter.encode(&mut enc, &view, width, height),
            "the first dispatch runs"
        );
        queue.submit(Some(enc.finish()));
        meter.begin_map();
        let hist = loop {
            match meter.take_ready() {
                Ok(Some(hist)) => break hist,
                Ok(None) => {
                    device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .expect("the map lands");
                }
                Err(error) => panic!("auto-exposure readback: {error}"),
            }
        };
        let cpu = histogram_log_luma(&frame, width, height).expect("histograms");
        assert_eq!(
            hist.bins, cpu.bins,
            "the device bins the frame like the oracle"
        );
        assert_eq!(hist.texels, cpu.texels, "the same samples were binned");
        assert_eq!(
            meter.bind_size,
            (width, height),
            "the bind group rides the frame size"
        );
    }

    /// The meter never blocks and never guesses: with no map in flight
    /// it has nothing to say, and a second dispatch is skipped while the
    /// map of the first one is still open.
    #[test]
    fn auto_exposure_meter_waits_for_its_map_instead_of_guessing() {
        let Some((device, queue)) = crate::test_gpu::require_or_skip(
            "auto_exposure_meter_waits_for_its_map_instead_of_guessing",
            test_device(),
        ) else {
            return;
        };
        let (frame, width, height) = fixture();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("auto-exposure-hist-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut meter = AutoExposureGpu::new(&device, &queue);
        assert!(
            meter.take_ready().expect("no map in flight").is_none(),
            "a meter with no map has nothing to report"
        );
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("auto-exposure-hist"),
        });
        assert!(
            meter.encode(&mut enc, &view, width, height),
            "the dispatch runs"
        );
        queue.submit(Some(enc.finish()));
        meter.begin_map();
        assert!(meter.pending.is_some(), "the map is open after the submit");
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("auto-exposure-hist"),
        });
        assert!(
            !meter.encode(&mut enc, &view, width, height),
            "a frame is skipped while the map of the last one is open"
        );
        let _ = frame;
    }

    /// The live frame: the meter histograms the HDR target the forward
    /// wrote, the readback lands a frame later, and the adapted stops
    /// ride the frame that follows.
    #[test]
    fn auto_exposure_live_frame_meters_the_hdr_target() {
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(crate::present::PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(error) => panic!("presenter init failed (must fail loudly): {error}"),
        };
        let Some(mut presenter) = crate::test_gpu::require_or_skip(
            "auto_exposure_live_frame_meters_the_hdr_target",
            probe,
        ) else {
            return;
        };
        presenter.set_auto_exposure_enabled(true);
        presenter
            .upload_mesh(mesh_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(SIDE, SIDE).expect("valid size applies");
        let list = bright_list();
        for _ in 0..12 {
            presenter.render_live_offscreen(&list).expect("live frame");
        }
        let hist = presenter
            .auto_exposure_histogram()
            .expect("the meter read a frame");
        assert!(hist.texels > 0.0, "the histogram holds samples");
        let ev = presenter.auto_exposure_ev();
        assert!(
            ev != 0.0 && (-8.0..=8.0).contains(&ev),
            "the meter moved the stops off zero, got {ev}"
        );
    }

    /// The mesh key of the live-frame fixture.
    fn mesh_key() -> crate::present::MeshKey {
        crate::present::MeshKey::new(0xA0E1, [0xA0; 32])
    }

    /// A bright cube on a dark field: the meter reads the frame and the
    /// stops move away from the fixed exposure.
    fn bright_list() -> crate::present::DrawList {
        let scene = crate::v1::default_scene(SIDE, SIDE).expect("default scene is valid");
        crate::present::DrawList {
            camera: crate::v1::Camera::new(
                [0.0, 0.0, 40.0],
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                40.0f32.to_radians(),
            ),
            light: scene.light,
            items: vec![crate::present::DrawItem {
                mesh: mesh_key(),
                material: crate::v1::PbrMaterial {
                    base_color: [1.0, 1.0, 1.0],
                    metallic: 0.0,
                    roughness: 0.8,
                    exposure: 6.0,
                },
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        }
    }
}
