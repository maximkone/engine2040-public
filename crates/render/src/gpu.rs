//! gpu: winit + wgpu sprite lane behind `window` feature. Headless builds skip it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const SHADER: &str = r#"
struct Globals {
    time: f32,
    count: f32,
    aspect: f32,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> g: Globals;

fn hash(n: u32) -> u32 {
    var x = n;
    x = (x ^ 61u) ^ (x >> 16u);
    x = x + (x << 3u);
    x = x ^ (x >> 4u);
    x = x * 0x27d4eb2du;
    x = x ^ (x >> 15u);
    return x;
}
fn rnd(n: u32) -> f32 {
    return f32(hash(n)) / 4294967295.0;
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );
    let r1 = rnd(ii * 7u + 1u);
    let r2 = rnd(ii * 7u + 2u);
    let r3 = rnd(ii * 7u + 3u);
    let r4 = rnd(ii * 7u + 4u);
    let speed = 0.05 + r4 * 0.25;
    let x = fract(r1 + g.time * speed) * 2.0 - 1.0;
    let y = fract(r2 + g.time * speed * 0.9) * 2.0 - 1.0;
    let size = 0.002 + r3 * 0.004;
    let c = corners[vi];
    var out: VOut;
    out.pos = vec4<f32>(x / g.aspect + c.x * size, y + c.y * size, 0.0, 1.0);
    out.color = vec3<f32>(0.2 + r1 * 0.8, 0.2 + r2 * 0.8, 0.2 + r3 * 0.8);
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

/// Open a window, draw `sprite_count` procedural quads, print fps each second,
/// exit after `seconds`. Errors when window/GPU setup fails or no frame was
/// submitted, so a smoke run cannot pass without real GPU work.
pub fn run_window(sprite_count: u32, seconds: u64) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = App::new(sprite_count, seconds);
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if let Some(failure) = app.failure {
        return Err(failure);
    }
    if app.frames == 0 {
        return Err("no frame was submitted and presented".into());
    }
    Ok(())
}

struct GpuState {
    window: Arc<Window>,
    adapter: wgpu::AdapterInfo,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GpuState {
    fn new(window: Arc<Window>, sprite_count: u32) -> Result<Self, String> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Startup pick is resolved here for the wgpu backends; the binary
        // logs `pick.startup_line()` with `choice_reason` (no stdout in lib).
        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let desc = wgpu::InstanceDescriptor {
            backends: crate::backend::wgpu_backends(pick.primary),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        };
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| format!("surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("engine2040-render"),
            ..Default::default()
        }))
        .map_err(|e| format!("device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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
            label: Some("sprite-wgsl"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals-layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bind"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite-layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite-pipeline"),
            layout: Some(&layout),
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
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let _ = sprite_count;
        Ok(Self {
            window,
            adapter: adapter.get_info(),
            surface,
            device,
            queue,
            config,
            pipeline,
            globals,
            bind_group,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Returns whether a frame was submitted and presented.
    fn render(&mut self, time_s: f32, sprite_count: u32) -> bool {
        use wgpu::CurrentSurfaceTexture;
        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(f) => f,
            CurrentSurfaceTexture::Suboptimal(f) => {
                self.surface.configure(&self.device, &self.config);
                f
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Validation => return false,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let aspect = self.config.width as f32 / self.config.height as f32;
        let data = [time_s, sprite_count as f32, aspect, 0.0];
        let mut bytes = Vec::with_capacity(16);
        for v in data {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.queue.write_buffer(&self.globals, 0, &bytes);

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.02,
                            b: 0.04,
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
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.bind_group, &[]);
            rp.draw(0..6, 0..sprite_count);
        }
        self.queue.submit(Some(enc.finish()));
        self.queue.present(frame);
        true
    }
}

struct App {
    sprite_count: u32,
    max_secs: u64,
    state: Option<GpuState>,
    start: Instant,
    last_report: Instant,
    /// Frames actually submitted and presented.
    frames: u64,
    failure: Option<String>,
}

impl App {
    fn new(sprite_count: u32, max_secs: u64) -> Self {
        let now = Instant::now();
        Self {
            sprite_count,
            max_secs,
            state: None,
            start: now,
            last_report: now,
            frames: 0,
            failure: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, failure: String) {
        eprintln!("{failure}");
        self.failure = Some(failure);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("engine2040 render proof")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => return self.fail(event_loop, format!("window: {e}")),
        };
        match GpuState::new(Arc::clone(&window), self.sprite_count) {
            Ok(state) => {
                let info = &state.adapter;
                println!(
                    "render up: sprites={} adapter=\"{}\" backend={:?} type={:?} driver=\"{} {}\"",
                    self.sprite_count,
                    info.name,
                    info.backend,
                    info.device_type,
                    info.driver,
                    info.driver_info
                );
                self.state = Some(state);
                window.request_redraw();
            }
            Err(e) => self.fail(event_loop, format!("gpu init failed: {e}")),
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
                let elapsed = self.start.elapsed();
                if let Some(state) = self.state.as_mut() {
                    if state.render(elapsed.as_secs_f32(), self.sprite_count) {
                        self.frames += 1;
                    }
                }
                if self.last_report.elapsed() >= Duration::from_secs(1) {
                    let fps = self.frames as f64 / self.start.elapsed().as_secs_f64();
                    println!(
                        "sprites={} frames={} avg_fps={:.1}",
                        self.sprite_count, self.frames, fps
                    );
                    self.last_report = Instant::now();
                }
                if elapsed >= Duration::from_secs(self.max_secs) {
                    let fps = self.frames as f64 / elapsed.as_secs_f64();
                    println!(
                        "done sprites={} frames={} secs={} avg_fps={:.1}",
                        self.sprite_count,
                        self.frames,
                        elapsed.as_secs(),
                        fps
                    );
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}
