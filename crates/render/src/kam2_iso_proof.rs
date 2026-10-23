//! Windowed proof of the fixed 2.5D ground field.
//!
//! Opens a real winit window over a wgpu surface, draws one instanced diamond
//! per grid tile with the [`IsoCamera`] basis, reads the presented frame back
//! and writes it as a PNG. Two tile centers are sampled against the locked
//! palette, so a frame that lost its geometry fails instead of shipping a
//! blank image.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::kam2_iso::{IsoCamera, TILE_COUNT, TILE_WORLD};
use crate::kam2_proof_util::{count_near, linear};

/// Window width in logical pixels.
const WINDOW_W: u32 = 1280;
/// Window height in logical pixels.
const WINDOW_H: u32 = 720;
/// `copy_texture_to_buffer` rows must be 256-byte aligned.
const ROW_ALIGN: u32 = 256;
/// How many surface frames may be skipped before the proof gives up.
const MAX_EMPTY_FRAMES: u32 = 60;
/// Light grass from the style-lock palette.
const GRASS: [u8; 3] = [0x8F, 0xA8, 0x4E];
/// Light dirt from the style-lock palette.
const DIRT: [u8; 3] = [0xC9, 0xA2, 0x6B];
/// Ambient from the style-lock palette, used as the clear color.
const AMBIENT: [u8; 3] = [0x3A, 0x42, 0x50];

const SHADER: &str = r#"
struct Cam {
    screen_right: vec3<f32>,
    pad0: f32,
    screen_up: vec3<f32>,
    pad1: f32,
    center: vec2<f32>,
    viewport: vec2<f32>,
    world_per_px: f32,
    tile_world: f32,
    y_scale: f32,
    pad2: f32,
    grass: vec3<f32>,
    pad3: f32,
    dirt: vec3<f32>,
    pad4: f32,
};
@group(0) @binding(0) var<uniform> cam: Cam;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    let tx = ii % 16u;
    let ty = ii / 16u;
    let half = 8.0;

    // Zigzag strip: top, left, right, bottom. Triangles (top,left,right) and
    // (left,right,bottom) cover both halves of the diamond.
    var corner = vec2<f32>(-0.5, -0.5);
    if (vi == 1u) {
        corner = vec2<f32>(-0.5, 0.5);
    } else if (vi == 2u) {
        corner = vec2<f32>(0.5, -0.5);
    } else if (vi == 3u) {
        corner = vec2<f32>(0.5, 0.5);
    }

    let tile = vec2<f32>(
        f32(tx) + 0.5 - half,
        f32(ty) + 0.5 - half,
    ) * cam.tile_world;
    let world_x = tile.x + corner.x * cam.tile_world;
    let world_z = tile.y + corner.y * cam.tile_world;
    let rel = vec3<f32>(world_x - cam.center.x, 0.0, world_z - cam.center.y);
    let pixel = cam.viewport * 0.5
        + vec2<f32>(dot(rel, cam.screen_right), -dot(rel, cam.screen_up) * cam.y_scale)
            / cam.world_per_px;
    let ndc = vec2<f32>(
        pixel.x / cam.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / cam.viewport.y * 2.0,
    );

    var out: VOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = select(cam.dirt, cam.grass, ((tx + ty) & 1u) == 0u);
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

/// One windowed proof run: tile count, PNG path and PNG byte size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// Ground tiles submitted in the draw.
    pub tiles: u32,
    /// PNG written under the repo target directory.
    pub png: PathBuf,
    /// PNG file size in bytes.
    pub bytes: usize,
}

/// Why the windowed proof failed.
#[derive(Debug)]
pub enum ProofError {
    /// The winit event loop could not start or run.
    EventLoop(winit::error::EventLoopError),
    /// The OS refused the window.
    Window(winit::error::OsError),
    /// The window surface could not be created.
    Surface(wgpu::CreateSurfaceError),
    /// No adapter can present to the surface.
    Adapter(wgpu::RequestAdapterError),
    /// The adapter refused the device.
    Device(wgpu::RequestDeviceError),
    /// The surface offers no 8-bit sRGB format, so palette bytes cannot be exact.
    NoSrgbFormat,
    /// Mapping, polling or row stripping failed.
    Readback(String),
    /// A sampled tile center did not carry its palette color.
    Pixel(String),
    /// The PNG could not be encoded.
    Png(String),
    /// The PNG could not be written.
    Io(std::io::Error),
    /// No surface frame arrived, or every frame was retried away.
    NoFrame,
}

impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(error) => write!(f, "{error}"),
            Self::Window(error) => write!(f, "window: {error}"),
            Self::Surface(error) => write!(f, "gpu init failed: surface: {error}"),
            Self::Adapter(error) => write!(f, "gpu init failed: adapter: {error}"),
            Self::Device(error) => write!(f, "gpu init failed: device: {error}"),
            Self::NoSrgbFormat => write!(f, "surface has no 8-bit sRGB format"),
            Self::Readback(error) => write!(f, "gpu readback: {error}"),
            Self::Pixel(error) => write!(f, "pixel gate: {error}"),
            Self::Png(error) => write!(f, "png encode: {error}"),
            Self::Io(error) => write!(f, "png write: {error}"),
            Self::NoFrame => write!(f, "no surface frame was captured"),
        }
    }
}

impl std::error::Error for ProofError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
            Self::Surface(error) => Some(error),
            Self::Adapter(error) => Some(error),
            Self::Device(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Open the real window, draw the 256-tile field, write `output` and return
/// its receipt. The event loop exits after the first captured frame.
pub fn run_proof(output: PathBuf) -> Result<Proof, ProofError> {
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).map_err(ProofError::Io)?;
    }
    let event_loop = EventLoop::new().map_err(ProofError::EventLoop)?;
    let mut app = ProofApp {
        output,
        state: None,
        result: None,
        empty_frames: 0,
    };
    event_loop
        .run_app(&mut app)
        .map_err(ProofError::EventLoop)?;
    match app.result {
        Some(result) => result,
        None => Err(ProofError::NoFrame),
    }
}

struct ProofApp {
    output: PathBuf,
    state: Option<IsoWindow>,
    result: Option<Result<Proof, ProofError>>,
    empty_frames: u32,
}

impl ProofApp {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: ProofError) {
        self.result = Some(Err(error));
        event_loop.exit();
    }
}

impl ApplicationHandler for ProofApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("engine2040 2.5D ground proof")
            .with_inner_size(winit::dpi::LogicalSize::new(
                f64::from(WINDOW_W),
                f64::from(WINDOW_H),
            ));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(event_loop, ProofError::Window(error)),
        };
        match IsoWindow::new(window) {
            Ok(state) => {
                self.state = Some(state);
                if let Some(state) = self.state.as_ref() {
                    state.window.request_redraw();
                }
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = self.state.as_mut() {
                    state.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if self.result.is_some() {
                    return;
                }
                let outcome = match self.state.as_mut() {
                    Some(state) => state.capture(&self.output),
                    None => return,
                };
                match outcome {
                    Ok(Some(proof)) => {
                        self.result = Some(Ok(proof));
                        event_loop.exit();
                    }
                    Ok(None) => {
                        self.empty_frames += 1;
                        if self.empty_frames >= MAX_EMPTY_FRAMES {
                            self.fail(event_loop, ProofError::NoFrame);
                        }
                    }
                    Err(error) => self.fail(event_loop, error),
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.result.is_none() {
            if let Some(state) = self.state.as_ref() {
                state.window.request_redraw();
            }
        }
    }
}

struct IsoWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    camera: IsoCamera,
    bgra: bool,
}

impl IsoWindow {
    fn new(window: Arc<Window>) -> Result<Self, ProofError> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = crate::backend::new_instance(crate::backend::wgpu_backends(pick.primary));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(ProofError::Surface)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(ProofError::Adapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("kam2-iso-proof"),
            ..Default::default()
        }))
        .map_err(ProofError::Device)?;

        let caps = surface.get_capabilities(&adapter);
        let format = [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ]
        .into_iter()
        .find(|candidate| caps.formats.contains(candidate))
        .or_else(|| {
            caps.formats
                .iter()
                .copied()
                .find(|candidate| candidate.is_srgb())
        })
        .ok_or(ProofError::NoSrgbFormat)?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kam2-iso-wgsl"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kam2-iso-camera"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kam2-iso-camera-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kam2-iso-camera-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kam2-iso-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kam2-iso-pipeline"),
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
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            uniform,
            bind_group,
            camera: IsoCamera::new(width, height),
            bgra,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.camera.resize(width, height);
    }

    fn pack_uniform(&self) -> [u8; 96] {
        let basis = self.camera.basis();
        let center = self.camera.world_center();
        let viewport = self.camera.viewport();
        let grass = linear(GRASS);
        let dirt = linear(DIRT);
        let values = [
            basis.right[0],
            basis.right[1],
            basis.right[2],
            0.0,
            basis.up[0],
            basis.up[1],
            basis.up[2],
            0.0,
            center[0],
            center[1],
            viewport[0],
            viewport[1],
            self.camera.world_per_px(),
            TILE_WORLD,
            self.camera.vertical_stretch(),
            0.0,
            grass[0],
            grass[1],
            grass[2],
            1.0,
            dirt[0],
            dirt[1],
            dirt[2],
            1.0,
        ];
        let mut bytes = [0u8; 96];
        for (index, value) in values.into_iter().enumerate() {
            let offset = index * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// Draw one frame. `None` means the surface was not ready and the caller
    /// may try the next one.
    fn capture(&mut self, output: &Path) -> Result<Option<Proof>, ProofError> {
        let width = self.config.width;
        let height = self.config.height;
        if width < 64 || height < 64 {
            return Ok(None);
        }
        use wgpu::CurrentSurfaceTexture;
        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(None);
            }
            CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Validation => return Ok(None),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.queue
            .write_buffer(&self.uniform, 0, &self.pack_uniform());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kam2-iso-frame"),
            });
        {
            let clear = linear(AMBIENT);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kam2-iso-ground"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear[0]),
                            g: f64::from(clear[1]),
                            b: f64::from(clear[2]),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..4, 0..TILE_COUNT);
        }

        let bytes_per_row = width * 4;
        let padded = bytes_per_row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kam2-iso-readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &frame.texture,
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
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| ProofError::Readback(format!("poll: {error}")))?;
        receiver
            .recv()
            .map_err(|error| ProofError::Readback(format!("map channel: {error}")))?
            .map_err(|error| ProofError::Readback(format!("map: {error}")))?;

        let row_bytes = usize::try_from(bytes_per_row)
            .map_err(|_| ProofError::Readback("row bytes overflow".into()))?;
        let stride = usize::try_from(padded)
            .map_err(|_| ProofError::Readback("row stride overflow".into()))?;
        let mut rgba = vec![
            0u8;
            row_bytes
                * usize::try_from(height)
                    .map_err(|_| ProofError::Readback("height overflow".into()))?
        ];
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|error| ProofError::Readback(format!("mapped range: {error}")))?;
            for (row, destination) in rgba.chunks_exact_mut(row_bytes).enumerate() {
                let offset = row * stride;
                destination.copy_from_slice(&mapped[offset..offset + row_bytes]);
            }
        }
        readback.unmap();
        if self.bgra {
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }

        // A full 16x16 field covers 128 diamonds per color, 1024 pixels each.
        // A clipped or half-drawn tile field loses far more than this margin.
        let grass_count = count_near(&rgba, GRASS);
        let dirt_count = count_near(&rgba, DIRT);
        if grass_count < 130_000 || dirt_count < 130_000 {
            return Err(ProofError::Pixel(format!(
                "field coverage grass={grass_count} dirt={dirt_count}"
            )));
        }

        expect_pixel(
            &rgba,
            width,
            height,
            self.camera.project(IsoCamera::tile_center(7, 7)),
            GRASS,
            "grass",
        )?;
        expect_pixel(
            &rgba,
            width,
            height,
            self.camera.project(IsoCamera::tile_center(8, 7)),
            DIRT,
            "dirt",
        )?;

        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| ProofError::Png(error.to_string()))?;
        }
        std::fs::write(output, &png).map_err(ProofError::Io)?;
        Ok(Some(Proof {
            tiles: TILE_COUNT,
            png: output.to_path_buf(),
            bytes: png.len(),
        }))
    }
}

fn expect_pixel(
    rgba: &[u8],
    width: u32,
    height: u32,
    at: [f32; 2],
    want: [u8; 3],
    name: &str,
) -> Result<(), ProofError> {
    let x = at[0].round();
    let y = at[1].round();
    if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
        return Err(ProofError::Pixel(format!("{name} sample outside frame")));
    }
    let row =
        usize::try_from(width).map_err(|_| ProofError::Pixel("frame width overflow".into()))?;
    let index = (usize::try_from(y as u32)
        .map_err(|_| ProofError::Pixel("pixel row overflow".into()))?
        * row
        + usize::try_from(x as u32)
            .map_err(|_| ProofError::Pixel("pixel column overflow".into()))?)
        * 4;
    let got = [rgba[index], rgba[index + 1], rgba[index + 2]];
    if got
        .iter()
        .zip(want)
        .any(|(value, expected)| value.abs_diff(expected) > 3)
    {
        return Err(ProofError::Pixel(format!(
            "{name} pixel {got:?}, want {want:?}"
        )));
    }
    Ok(())
}
