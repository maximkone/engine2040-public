//! v1_gpu: wgpu offscreen V1 forward-PBR behind the `window` feature.
//!
//! Same scene contract as [`crate::v1`]: validated mesh upload, glam camera,
//! one directional light, metallic-roughness PBR, exposure plus ACES, depth
//! test. Renders offscreen (no window/surface needed) into a texture, copies
//! back to the CPU and returns PNG bytes plus a [`crate::v1::Manifest`] that
//! names the real adapter/driver and carries the validation error count.
//! Targets are rebuilt per capture size, which is the resize/recreate path;
//! the headless [`crate::v1::SwapState`] transitions are unit-tested without
//! a GPU, and the windowed swapchain in [`crate::gpu`] already reconfigures
//! on resize and on `Outdated`/`Lost`.
//!
//! WGSL math is ported from Bevy Engine's MIT/Apache-2.0 renderer at
//! `v0.15.0`; each ported construct carries a
//! `PORT(bevyengine/bevy@v0.15.0:<path>)` marker with its Bevy source file.
//! The orchestrator records the matching `EVIDENCE.md` row.

use crate::v1::{self, CaptureError, Manifest, Scene};
use std::fmt;

// ---------------------------------------------------------------------------
// Forward PBR shader: one file, `shaders/forward.wgsl`, with the display
// transform (`shaders/tonemap.wgsl`) prepended at compile time. BRDF core
// ported from Bevy Engine (MIT OR Apache-2.0) at `v0.15.0`; the PORT
// markers live in those files. This offscreen capture path runs the
// `fs_display` fragment entry (one light plus the baked ambient, exposure,
// ACES, sRGB written straight to an 8-bit target); the window frame runs
// `fs` into a linear HDR target and tone-maps in its blit pass.
// ---------------------------------------------------------------------------

/// The forward shader source: tonemap library plus `forward.wgsl`. Entry
/// points: `vs`, `vs_skinned`, `vs_depth`, `vs_sky`, `fs`, `fs_sky`,
/// `fs_display`, `fs_skinned`.
pub const FORWARD_PBR_WGSL: &str = concat!(
    include_str!("shaders/tonemap.wgsl"),
    include_str!("shaders/forward.wgsl")
);

// ---------------------------------------------------------------------------
// GPU errors: typed, no panics on device/IO data.
// ---------------------------------------------------------------------------

/// Offscreen renderer failures.
#[derive(Debug)]
pub enum GpuError {
    /// No usable adapter (headless/CI without a GPU skips, never fails).
    NoAdapter,
    /// Device/request failure, message carried.
    Device(String),
    /// A wgpu validation error was observed inside the capture scope.
    Validation(String),
    /// Readback mapping/poll failure.
    Readback(String),
    /// Scene/capture-side failure.
    Capture(CaptureError),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no wgpu adapter available"),
            Self::Device(e) => write!(f, "gpu device: {e}"),
            Self::Validation(e) => write!(f, "gpu validation: {e}"),
            Self::Readback(e) => write!(f, "gpu readback: {e}"),
            Self::Capture(e) => write!(f, "gpu capture: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

// ---------------------------------------------------------------------------
// Offscreen forward renderer.
// ---------------------------------------------------------------------------

/// Byte size of the camera uniform (64 B view-proj + 16 B pos/pad).
pub(crate) const CAMERA_UNIFORM_BYTES: u64 = 80;
/// Byte size of the light uniform (16 B dir/pad + 16 B color/intensity).
pub(crate) const LIGHT_UNIFORM_BYTES: u64 = 32;
/// Byte size of the material uniform (16 B base/metallic + 16 B factors).
const MATERIAL_UNIFORM_BYTES: u64 = 32;
/// Interleaved vertex stride: pos(3) + normal(3) + uv(2) + tangent(4) f32.
pub(crate) const VERTEX_STRIDE: u64 = 48;
/// `copy_texture_to_buffer` rows must be 256-byte aligned.
const ROW_ALIGN: u32 = 256;

/// Shared uniform-buffer helper: single owner for the `UNIFORM | COPY_DST`
/// init pattern used by the forward and skinned offscreen passes.
pub(crate) fn create_uniform_buffer(
    device: &wgpu::Device,
    label: &str,
    bytes: &[u8],
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// The studio HDRI behind the baked ambient of every offscreen forward
/// capture (the window presenter bakes the same picture), embedded once.
pub(crate) const STUDIO_HDRI_BYTES: &[u8] =
    include_bytes!("../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

/// Offscreen V1 forward renderer: pipeline compiled once, color/depth
/// targets rebuilt per capture size (the recreate path). The `fs_display`
/// entry lights with the baked ambient, so the studio bake lives here and
/// binds at 9-12 on every capture.
pub struct GpuForward {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    adapter_driver: String,
    pipeline: wgpu::RenderPipeline,
    camera_layout: wgpu::BindGroupLayout,
    ibl_prefilter: wgpu::Texture,
    ibl_brdf: wgpu::Texture,
    ibl_sh_buf: wgpu::Buffer,
    ibl_sampler: wgpu::Sampler,
}

impl GpuForward {
    /// Bring up a high-performance adapter and compile the forward pipeline.
    /// Returns [`GpuError::NoAdapter`] when no GPU is present; callers
    /// testing on headless machines treat that as skip, not failure.
    pub fn init() -> Result<Self, GpuError> {
        // Test builds only: cap the GPU tests that hold a device at once.
        #[cfg(test)]
        crate::test_gpu::hold_gpu_slot();
        let instance = crate::backend::new_instance(
            // Test builds skip the GL backend: its driver DLL loads from a
            // helper thread and can wedge the process loader lock.
            if cfg!(test) {
                wgpu::Backends::PRIMARY
            } else {
                wgpu::Backends::all()
            },
        );
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|_| GpuError::NoAdapter)?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("engine2040-r1-forward"),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(format!("request_device: {e}")))?;

        // The capture entry samples the baked ambient like the window frame,
        // so the studio bake is uploaded once here. A failed bake fails init
        // loudly, never a silent dark frame.
        let (ibl_bake, _) = crate::ibl::bake(STUDIO_HDRI_BYTES)
            .map_err(|e| GpuError::Device(format!("look ibl bake: {e}")))?;
        let (ibl_prefilter, ibl_brdf, ibl_sh_buf) =
            crate::ibl::gpu::upload_live_bake(&device, &queue, &ibl_bake)
                .map_err(|e| GpuError::Device(format!("live ibl bake: {e}")))?;
        let ibl_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("r1-ibl-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("r1-forward-pbr"),
            source: wgpu::ShaderSource::Wgsl(FORWARD_PBR_WGSL.into()),
        });
        let mut layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        layout_entries.extend(crate::ibl::gpu::forward_ibl_layout_entries());
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("r1-uniforms"),
            entries: &layout_entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("r1-forward-layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        // Linear-space working target: the shader already sRGB-encodes, so a
        // plain UNORM target avoids a double conversion.
        const TARGET: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("r1-forward-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 3,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_display"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
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
        });

        Ok(Self {
            device,
            queue,
            adapter_name: info.name.clone(),
            adapter_driver: format!("{} {}", info.driver, info.driver_info),
            pipeline,
            camera_layout,
            ibl_prefilter,
            ibl_brdf,
            ibl_sh_buf,
            ibl_sampler,
        })
    }

    fn uniform_buffer(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        create_uniform_buffer(&self.device, label, bytes)
    }

    /// The capture's camera, light and material uniform buffers, built once
    /// per capture (a cold, one-shot path: the byte images are small `Vec`s).
    fn capture_uniform_buffers(
        &self,
        cam_u: &v1::CameraUniform,
        light_u: &v1::LightUniform,
        mat_u: &v1::MaterialUniform,
    ) -> [wgpu::Buffer; 3] {
        let cam = cam_u.to_bytes();
        let light = light_u.to_bytes();
        let mat = mat_u.to_bytes();
        debug_assert_eq!(cam.len() as u64, CAMERA_UNIFORM_BYTES);
        debug_assert_eq!(light.len() as u64, LIGHT_UNIFORM_BYTES);
        debug_assert_eq!(mat.len() as u64, MATERIAL_UNIFORM_BYTES);
        [
            self.uniform_buffer("r1-u-camera", &cam),
            self.uniform_buffer("r1-u-light", &light),
            self.uniform_buffer("r1-u-material", &mat),
        ]
    }

    /// Render `scene` offscreen at `width` x `height`, read the frame back
    /// and return PNG bytes plus a manifest naming the real adapter/driver.
    /// A validation scope wraps the whole capture: any wgpu validation error
    /// fails the capture instead of silently producing pixels.
    pub fn render_capture(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, Manifest), GpuError> {
        if width == 0 || height == 0 {
            return Err(GpuError::Capture(CaptureError::ZeroArea));
        }
        if width > v1::MAX_CAPTURE_DIM || height > v1::MAX_CAPTURE_DIM {
            return Err(GpuError::Capture(CaptureError::TooLarge { width, height }));
        }
        let mut camera = scene.camera.clone();
        camera.aspect = width as f32 / height as f32;
        let (cam_u, light_u, mat_u) =
            v1::build_uniforms(&camera, camera.aspect, &scene.light, &scene.material).map_err(
                |e| match e {
                    v1::UniformError::Camera(e) => GpuError::Capture(CaptureError::Camera(e)),
                    v1::UniformError::Light(e) => GpuError::Capture(CaptureError::Light(e)),
                    v1::UniformError::Material(e) => GpuError::Capture(CaptureError::Material(e)),
                },
            )?;

        // The whole capture runs inside one validation scope: zero tolerance.
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        // Recreate path: fresh color + depth targets for this exact size.
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("r1-capture-color"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("r1-capture-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

        use wgpu::util::DeviceExt;
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("r1-mesh-vb"),
                contents: &scene.mesh.vertex_bytes(),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("r1-mesh-ib"),
                contents: &scene.mesh.index_bytes(),
                usage: wgpu::BufferUsages::INDEX,
            });
        let [cam_buf, light_buf, mat_buf] = self.capture_uniform_buffers(&cam_u, &light_u, &mat_u);
        let ibl_prefilter_view = self
            .ibl_prefilter
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("r1-ibl-prefilter-view"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let ibl_brdf_view = self.ibl_brdf.create_view(&wgpu::TextureViewDescriptor {
            label: Some("r1-ibl-brdf-view"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("r1-uniforms"),
            layout: &self.camera_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cam_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: mat_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&ibl_prefilter_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&ibl_brdf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&self.ibl_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: self.ibl_sh_buf.as_entire_binding(),
                },
            ],
        });

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("r1-capture"),
            });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("r1-forward"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.03,
                            g: 0.035,
                            b: 0.06,
                            a: 1.0,
                        }),
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
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, vb.slice(..));
            rp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..scene.mesh.indices.len() as u32, 0, 0..1);
        }

        let bytes_per_row = width * 4;
        let padded = bytes_per_row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("r1-readback"),
            size: (padded as u64) * (height as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit(Some(enc.finish()));

        // Map and strip row padding.
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(format!("poll: {e}")))?; // alloc-ok: cold readback failure, never a steady frame
        rx.recv()
            .map_err(|e| GpuError::Readback(format!("map channel: {e}")))? // alloc-ok: cold readback failure, never a steady frame
            .map_err(|e| GpuError::Readback(format!("map: {e}")))?; // alloc-ok: cold readback failure, never a steady frame
        let mut rgba = vec![0u8; (bytes_per_row as usize) * (height as usize)]; // alloc-ok: offscreen capture staging, not the window steady frame
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| GpuError::Readback(format!("mapped range: {e}")))?; // alloc-ok: cold readback failure, never a steady frame
            for (dst, src) in rgba.chunks_exact_mut(bytes_per_row as usize).enumerate() {
                let off = dst * padded as usize;
                src.copy_from_slice(&mapped[off..off + bytes_per_row as usize]);
            }
        }
        readback.unmap();

        // Pop the validation scope after all GPU work completed.
        let scope_err = pollster::block_on(scope.pop());
        if let Some(err) = scope_err {
            return Err(GpuError::Validation(format!("{err:?}"))); // alloc-ok: cold validation failure, never a steady frame
        }

        let png = v1::encode_png_rgba8(&rgba, width, height).map_err(GpuError::Capture)?;
        let manifest = Manifest::new(&self.adapter_name, &self.adapter_driver, width, height, 0);
        Ok((png, manifest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R1 capture smoke: real wgpu forward-PBR offscreen render with the
    /// procedural cube, PNG + manifest out, zero validation errors. Skips
    /// (passes) when no adapter exists — headless CI has no GPU — and the
    /// receipt run on the reference machine must actually render.
    /// Exercises the recreate path by capturing at two sizes in a row.
    #[test]
    fn gpu_offscreen_capture_is_validation_clean() {
        let gpu = match GpuForward::init() {
            Ok(g) => g,
            Err(GpuError::NoAdapter) => {
                eprintln!("SKIPPED gpu_offscreen_capture: no wgpu adapter on this machine");
                return;
            }
            Err(e) => panic!("gpu init failed (must fail loudly, never fake green): {e}"),
        };
        for (w, h) in [(160u32, 120u32), (96u32, 96u32)] {
            let scene = v1::default_scene(w, h).expect("default scene is valid");
            let (png, manifest) = gpu
                .render_capture(&scene, w, h)
                .expect("gpu capture renders clean");
            assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
            let img = image::load_from_memory(&png).expect("gpu capture is a valid PNG");
            assert_eq!((img.width(), img.height()), (w, h));
            assert_eq!((manifest.width, manifest.height), (w, h));
            assert!(
                manifest.is_clean(),
                "validation errors must be 0, got {:?}",
                manifest
            );
            assert!(!manifest.adapter.is_empty(), "manifest names the adapter");
            assert!(!manifest.driver.is_empty(), "manifest names the driver");
            // The lit cube must cover pixels above the clear color (7, 8, 15
            // from the 0.03/0.035/0.06 clear).
            let rgba = img.to_rgba8();
            let mut covered = 0usize;
            for p in rgba.pixels() {
                let off = (p[0] as f32 - 7.0)
                    .abs()
                    .max((p[1] as f32 - 8.0).abs())
                    .max((p[2] as f32 - 15.0).abs());
                if off > 5.0 {
                    covered += 1;
                }
            }
            assert!(
                covered > 100,
                "gpu cube must cover pixels at {w}x{h}, got {covered}"
            );
        }
        // The GPU light provably shades: same scene, light off, real energy
        // difference. This also re-exercises the recreate path once more.
        let mut dark = v1::default_scene(96, 96).expect("default scene is valid");
        dark.light.intensity = 0.0;
        let lit_scene = v1::default_scene(96, 96).expect("default scene is valid");
        let (lit_png, _) = gpu
            .render_capture(&lit_scene, 96, 96)
            .expect("lit gpu capture renders");
        let (dark_png, _) = gpu
            .render_capture(&dark, 96, 96)
            .expect("unlit gpu capture renders");
        let lit = image::load_from_memory(&lit_png)
            .expect("lit png decodes")
            .to_rgba8();
        let dark = image::load_from_memory(&dark_png)
            .expect("dark png decodes")
            .to_rgba8();
        let mut energy = 0u64;
        for (a, b) in lit.pixels().zip(dark.pixels()) {
            energy += (a[0] as i32 - b[0] as i32).unsigned_abs() as u64
                + (a[1] as i32 - b[1] as i32).unsigned_abs() as u64
                + (a[2] as i32 - b[2] as i32).unsigned_abs() as u64;
        }
        assert!(energy > 500, "gpu light must shade pixels, energy {energy}");
        // Persist the receipt outside the repo (temp dir, never git dirt).
        let scene = v1::default_scene(320, 200).expect("default scene is valid");
        let (png, manifest) = gpu
            .render_capture(&scene, 320, 200)
            .expect("receipt renders");
        let dir = std::env::temp_dir().join("engine2040-r1");
        std::fs::create_dir_all(&dir).expect("temp receipt dir");
        std::fs::write(dir.join("r1-capture.png"), &png).expect("receipt png writes");
        std::fs::write(dir.join("r1-manifest.json"), manifest.to_json())
            .expect("receipt json writes");
        eprintln!(
            "R1 receipt: {} + manifest {}",
            dir.join("r1-capture.png").display(),
            manifest.to_json()
        );
    }
}
