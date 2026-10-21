//! The window's GPU leg: two instanced pipelines, sprite atlases, readback.
//!
//! Consumer: the `kam2play` shell, which owns the match and the camera and asks
//! this module to draw one [`Scene`] and one [`HudFrame`]. The shape pass draws
//! the ground quads; the sprite pass draws atlas cells with the team tint
//! through the mask texture, one bind group for the soldier sheet and one per
//! house atlas slot: the cottage in slot zero and one slot per house kind. The
//! frame is a plain sRGB surface; a capture frame is copied into a readback
//! buffer, the HUD is composited on the CPU and the result becomes a PNG.
//!
//! Determinism: no clock and no sim here; the pixels are a pure function of the
//! scene and the HUD. Failure modes are typed in [`crate::PlayError`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine_render::hud_pass::{self, HudSurface};
use engine_ui::HudFrame;
use kam2_game::{Part, Scene, ShapeQuad, SpriteQuad, HOUSE_SLOTS};
use winit::window::Window;

use crate::art::Sheet;
use crate::PlayError;

/// `copy_texture_to_buffer` rows must be 256-byte aligned.
const ROW_ALIGN: u32 = 256;
/// Shape instances the frame may submit: 192 ground diamonds.
const MAX_SHAPES: usize = 512;
/// Unit sprite instances the frame may submit.
const MAX_SPRITES: usize = 64;
/// House sprite instances the frame may submit: the 16x12 field holds less.
const MAX_HOUSES: usize = 256;
/// Instance stride in bytes: twelve floats per shape or sprite.
const INSTANCE_STRIDE: usize = 48;
/// Ambient from the style-lock palette, used as the clear colour.
const AMBIENT: [u8; 3] = [0x3A, 0x42, 0x50];

const SHADER: &str = r#"
struct View {
    viewport: vec2<f32>,
    pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var mask_tex: texture_2d<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;

fn strip_corner(vi: u32) -> vec2<f32> {
    if (vi == 1u) {
        return vec2<f32>(1.0, 0.0);
    }
    if (vi == 2u) {
        return vec2<f32>(0.0, 1.0);
    }
    if (vi == 3u) {
        return vec2<f32>(1.0, 1.0);
    }
    return vec2<f32>(0.0, 0.0);
}

fn to_ndc(pixel: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(
        pixel.x / view.viewport.x * 2.0 - 1.0,
        1.0 - pixel.y / view.viewport.y * 2.0,
        0.0,
        1.0,
    );
}

struct ShapeIn {
    @location(0) c0: vec2<f32>,
    @location(1) c1: vec2<f32>,
    @location(2) c2: vec2<f32>,
    @location(3) c3: vec2<f32>,
    @location(4) color: vec4<f32>,
};

struct ShapeOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_shape(@builtin(vertex_index) vi: u32, item: ShapeIn) -> ShapeOut {
    var pixel = item.c0;
    if (vi == 1u) {
        pixel = item.c1;
    } else if (vi == 2u) {
        pixel = item.c2;
    } else if (vi == 3u) {
        pixel = item.c3;
    }
    var out: ShapeOut;
    out.pos = to_ndc(pixel);
    out.color = item.color;
    return out;
}

@fragment
fn fs_shape(in: ShapeOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct SpriteIn {
    @location(0) top_left: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_min: vec2<f32>,
    @location(3) uv_size: vec2<f32>,
    @location(4) team: vec4<f32>,
};

struct SpriteOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) team: vec3<f32>,
};

@vertex
fn vs_sprite(@builtin(vertex_index) vi: u32, item: SpriteIn) -> SpriteOut {
    let corner = strip_corner(vi);
    var out: SpriteOut;
    out.pos = to_ndc(item.top_left + corner * item.size);
    out.uv = item.uv_min + corner * item.uv_size;
    out.team = item.team.xyz;
    return out;
}

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_sprite(in: SpriteOut) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas_tex, atlas_sampler, in.uv);
    if (texel.a < 0.02) {
        discard;
    }
    let coverage = textureSample(mask_tex, atlas_sampler, in.uv).a;
    var rgb = texel.rgb;
    if (coverage > 0.0) {
        let product = rgb * in.team;
        let base_luma = dot(rgb, LUMA);
        let product_luma = dot(product, LUMA);
        var tinted = product;
        if (product_luma > 0.000001) {
            tinted = product * (base_luma / product_luma);
        }
        rgb = mix(rgb, clamp(tinted, vec3<f32>(0.0), vec3<f32>(1.0)), coverage);
    }
    return vec4<f32>(srgb_to_linear(rgb), texel.a);
}
"#;

const SHAPE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x2,
    3 => Float32x2,
    4 => Float32x4,
];

const SPRITE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x2,
    3 => Float32x2,
    4 => Float32x4,
];

/// One written capture: PNG path, file size and the HUD batches composited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// PNG under the repo target directory.
    pub png: PathBuf,
    /// PNG file size in bytes.
    pub bytes: usize,
    /// HUD draw batches composited on the frame.
    pub hud_draws: u32,
}

/// What one draw call did.
#[derive(Debug)]
pub enum Outcome {
    /// The frame was drawn and presented.
    Drawn,
    /// The surface was not ready; the frame was skipped.
    Skipped,
    /// The frame was drawn, read back and written as a PNG.
    Captured(Capture),
}

/// The window surface and the two pipelines.
pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    shapes_pipeline: wgpu::RenderPipeline,
    sprites_pipeline: wgpu::RenderPipeline,
    view_buffer: wgpu::Buffer,
    view_bind: wgpu::BindGroup,
    atlas_bind: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    house_binds: Vec<Option<wgpu::BindGroup>>,
    shapes: wgpu::Buffer,
    sprites: wgpu::Buffer,
    houses: wgpu::Buffer,
    bgra: bool,
}

/// One opened surface plus the device that presents to it.
struct OpenedSurface {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    format: wgpu::TextureFormat,
    bgra: bool,
}

/// The viewport uniform buffer, its bind group and the layout both pipelines use.
struct ViewBinding {
    buffer: wgpu::Buffer,
    bind: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
}

/// Open the window surface, pick an sRGB format and request the device.
fn open_surface(window: Arc<Window>) -> Result<OpenedSurface, PlayError> {
    let size = window.inner_size();
    let width = size.width.max(1);
    let height = size.height.max(1);

    let (choice, _) = engine_render::backend::resolve_from_env(None);
    let pick = engine_render::backend::pick_backend(choice, None);
    let instance =
        engine_render::backend::new_instance(engine_render::backend::wgpu_backends(pick.primary));
    let surface = instance
        .create_surface(Arc::clone(&window))
        .map_err(PlayError::Surface)?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(PlayError::Adapter)?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kam2play"),
        ..Default::default()
    }))
    .map_err(PlayError::Device)?;

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
    .ok_or(PlayError::NoSrgbFormat)?;
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
    let bgra = matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    Ok(OpenedSurface {
        surface,
        device,
        queue,
        config,
        format,
        bgra,
    })
}

/// The 16-byte viewport uniform: width, height and two pads.
fn view_binding(device: &wgpu::Device) -> ViewBinding {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kam2play-view"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("kam2play-view-layout"),
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
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("kam2play-view-bind"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    ViewBinding {
        buffer,
        bind,
        layout,
    }
}

/// The atlas bind-group layout: base texture, mask texture and the sampler.
fn make_atlas_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("kam2play-atlas-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Upload the atlas and the mask, then bind them with the nearest sampler.
fn atlas_binding(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sheet: &Sheet,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::BindGroup {
    let atlas = sprite_texture(device, queue, sheet, false);
    let mask = sprite_texture(device, queue, sheet, true);
    let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
    let mask_view = mask.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("kam2play-atlas-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("kam2play-atlas-bind"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&mask_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

/// The one shader module both pipelines share.
fn shader_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("kam2play-wgsl"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    })
}

/// The flat shape pipeline: screen-space quads in one instanced strip draw.
fn shape_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    view_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("kam2play-shape-layout"),
        bind_group_layouts: &[Some(view_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("kam2play-shape-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_shape"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: INSTANCE_STRIDE as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &SHAPE_ATTRIBUTES,
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_shape"),
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
    })
}

/// The sprite pipeline: atlas cells with the team mask tint over the field.
fn sprite_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    view_layout: &wgpu::BindGroupLayout,
    atlas_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("kam2play-sprite-layout"),
        bind_group_layouts: &[Some(view_layout), Some(atlas_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("kam2play-sprite-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_sprite"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: INSTANCE_STRIDE as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &SPRITE_ATTRIBUTES,
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_sprite"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// One instance buffer sized for `capacity` quads.
fn instance_buffer(device: &wgpu::Device, label: &str, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * INSTANCE_STRIDE) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Gpu {
    /// Bring up the surface, the device and both pipelines for `window`.
    ///
    /// The soldier atlas goes in with the sheet; the house atlases come later,
    /// one per slot, through [`Gpu::set_house_sheet`].
    ///
    /// # Errors
    ///
    /// Returns [`PlayError`] for a refused surface, adapter or device and for a
    /// surface without an 8-bit sRGB format.
    pub fn new(window: Arc<Window>, sheet: &Sheet) -> Result<Self, PlayError> {
        let OpenedSurface {
            surface,
            device,
            queue,
            config,
            format,
            bgra,
        } = open_surface(window)?;
        let view = view_binding(&device);
        let atlas_layout = make_atlas_layout(&device);
        let atlas_bind = atlas_binding(&device, &queue, sheet, &atlas_layout);
        let shader = shader_module(&device);
        let shapes_pipeline = shape_pipeline(&device, format, &view.layout, &shader);
        let sprites_pipeline =
            sprite_pipeline(&device, format, &view.layout, &atlas_layout, &shader);
        let shapes = instance_buffer(&device, "kam2play-shapes", MAX_SHAPES);
        let sprites = instance_buffer(&device, "kam2play-sprites", MAX_SPRITES);
        let houses = instance_buffer(&device, "kam2play-houses", MAX_HOUSES);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            shapes_pipeline,
            sprites_pipeline,
            view_buffer: view.buffer,
            view_bind: view.bind,
            atlas_bind,
            atlas_layout,
            house_binds: vec![None; HOUSE_SLOTS],
            shapes,
            sprites,
            houses,
            bgra,
        })
    }

    /// Upload one house atlas into its slot: the cottage in slot zero and one
    /// slot per house kind. A slot outside the registry is ignored.
    pub fn set_house_sheet(&mut self, slot: usize, sheet: &Sheet) {
        let bind = atlas_binding(&self.device, &self.queue, sheet, &self.atlas_layout);
        if let Some(cell) = self.house_binds.get_mut(slot) {
            *cell = Some(bind);
        }
    }

    /// Surface width in pixels.
    pub fn width(&self) -> u32 {
        self.config.width
    }

    /// Surface height in pixels.
    pub fn height(&self) -> u32 {
        self.config.height
    }

    /// Follow a window resize; a zero side is ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Draw one frame. `capture` writes that same frame to a PNG after the HUD
    /// pass and returns its receipt; without it the frame only presents.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError`] for a lost frame buffer, a readback failure, a HUD
    /// surface that does not fit, a PNG write failure, or a house whose atlas
    /// slot has no sheet.
    pub fn draw(
        &mut self,
        scene: &Scene,
        hud: &HudFrame,
        capture: Option<&Path>,
    ) -> Result<Outcome, PlayError> {
        let width = self.config.width;
        let height = self.config.height;
        if width < 64 || height < 64 {
            return Ok(Outcome::Skipped);
        }
        self.upload(scene, width, height);

        use wgpu::CurrentSurfaceTexture;
        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(Outcome::Skipped);
            }
            CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Validation => return Ok(Outcome::Skipped),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kam2play-frame"),
            });
        self.encode_scene(&mut encoder, &view, scene)?;

        let padded = (width * 4).div_ceil(ROW_ALIGN) * ROW_ALIGN;
        let readback = capture.map(|_| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("kam2play-readback"),
                size: u64::from(padded) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        if let Some(readback) = readback.as_ref() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &frame.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: readback,
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
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);

        let Some(path) = capture else {
            return Ok(Outcome::Drawn);
        };
        let Some(readback) = readback else {
            return Err(PlayError::Readback("no readback buffer".into()));
        };
        self.finish_capture(&readback, hud, path, width, height, padded)
    }

    /// One render pass: clear to ambient, then the field and the billboards.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError::Art`] when a house names an atlas slot that has no
    /// sheet, so a house is never dropped from the frame.
    fn encode_scene(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        scene: &Scene,
    ) -> Result<(), PlayError> {
        let clear = engine_assets::bc::srgb_byte_to_linear(AMBIENT[0]);
        let green = engine_assets::bc::srgb_byte_to_linear(AMBIENT[1]);
        let blue = engine_assets::bc::srgb_byte_to_linear(AMBIENT[2]);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("kam2play-scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear),
                        g: f64::from(green),
                        b: f64::from(blue),
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
        self.record(&mut pass, scene)
    }

    /// Read the captured frame back, composite the HUD and write the PNG.
    fn finish_capture(
        &self,
        readback: &wgpu::Buffer,
        hud: &HudFrame,
        path: &Path,
        width: u32,
        height: u32,
        padded: u32,
    ) -> Result<Outcome, PlayError> {
        let mut rgba = read_pixels(&self.device, readback, width, height, padded, self.bgra)?;
        let hud_draws = {
            let mut surface = HudSurface::new(width, height, &mut rgba).map_err(PlayError::Hud)?;
            let receipt = hud_pass::draw(hud, &mut surface);
            if receipt.drawn_pixels == 0 {
                return Err(PlayError::Pixel("hud drew no pixels".into()));
            }
            u32::try_from(receipt.draws).unwrap_or(u32::MAX)
        };
        let png = encode_png(&rgba, width, height)?;
        std::fs::write(path, &png).map_err(PlayError::Io)?;
        Ok(Outcome::Captured(Capture {
            png: path.to_path_buf(),
            bytes: png.len(),
            hud_draws,
        }))
    }

    /// Copy the frame's quads into the instance buffers and the viewport uniform.
    fn upload(&mut self, scene: &Scene, width: u32, height: u32) {
        let mut shape_bytes = Vec::with_capacity(scene.ground.len() * INSTANCE_STRIDE);
        for quad in &scene.ground {
            push_shape(&mut shape_bytes, quad);
        }
        let mut house_bytes = Vec::with_capacity(scene.houses.len() * INSTANCE_STRIDE);
        for quad in &scene.houses {
            push_sprite(&mut house_bytes, quad);
        }
        let mut sprite_bytes = Vec::with_capacity(scene.units.len() * INSTANCE_STRIDE);
        for quad in &scene.units {
            push_sprite(&mut sprite_bytes, quad);
        }
        if !shape_bytes.is_empty() {
            self.queue.write_buffer(&self.shapes, 0, &shape_bytes);
        }
        if !house_bytes.is_empty() {
            self.queue.write_buffer(&self.houses, 0, &house_bytes);
        }
        if !sprite_bytes.is_empty() {
            self.queue.write_buffer(&self.sprites, 0, &sprite_bytes);
        }
        let mut view = [0u8; 16];
        for (slot, value) in [width as f32, height as f32, 0.0, 0.0]
            .into_iter()
            .enumerate()
        {
            let offset = slot * 4;
            view[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        self.queue.write_buffer(&self.view_buffer, 0, &view);
    }

    /// Record the ground, then the billboard order: house cells and unit cells
    /// alternate as the depth keys say, one draw and one atlas bind per run. A
    /// house run is split where the atlas slot changes, so every house draws
    /// from the atlas of its own kind.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError::Art`] when a house names an atlas slot that has no
    /// sheet: the frame never drops a house to keep going.
    fn record(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene) -> Result<(), PlayError> {
        let ground = u32::try_from(scene.ground.len()).unwrap_or(0);
        pass.set_pipeline(&self.shapes_pipeline);
        pass.set_bind_group(0, &self.view_bind, &[]);
        pass.set_vertex_buffer(0, self.shapes.slice(..));
        if ground > 0 {
            pass.draw(0..4, 0..ground);
        }
        let mut index = 0usize;
        while index < scene.order.len() {
            match scene.order[index] {
                Part::House(first) => {
                    let run = contiguous(&scene.order, index, first);
                    let mut offset = 0usize;
                    while offset < run {
                        let slot = scene
                            .house_atlases
                            .get(first + offset)
                            .copied()
                            .ok_or_else(|| {
                                PlayError::Art(format!(
                                    "house {} has no atlas slot in the scene",
                                    first + offset
                                ))
                            })?;
                        let length = same_slot(scene, first + offset, run - offset);
                        let bind = self
                            .house_binds
                            .get(slot)
                            .and_then(Option::as_ref)
                            .ok_or_else(|| {
                                PlayError::Art(format!("house atlas slot {slot} has no sheet"))
                            })?;
                        let start = u32::try_from(first + offset).unwrap_or(0);
                        let end = start + u32::try_from(length).unwrap_or(0);
                        pass.set_pipeline(&self.sprites_pipeline);
                        pass.set_bind_group(0, &self.view_bind, &[]);
                        pass.set_bind_group(1, bind, &[]);
                        pass.set_vertex_buffer(0, self.houses.slice(..));
                        pass.draw(0..4, start..end);
                        offset += length;
                    }
                    index += run;
                }
                Part::Unit(first) => {
                    let run = contiguous(&scene.order, index, first);
                    let start = u32::try_from(first).unwrap_or(0);
                    let end = start + u32::try_from(run).unwrap_or(0);
                    pass.set_pipeline(&self.sprites_pipeline);
                    pass.set_bind_group(0, &self.view_bind, &[]);
                    pass.set_bind_group(1, &self.atlas_bind, &[]);
                    pass.set_vertex_buffer(0, self.sprites.slice(..));
                    pass.draw(0..4, start..end);
                    index += run;
                }
            }
        }
        Ok(())
    }
}

/// How many houses from `start` share the atlas slot of the first one, up to
/// `remaining` houses.
fn same_slot(scene: &Scene, start: usize, remaining: usize) -> usize {
    let Some(slot) = scene.house_atlases.get(start).copied() else {
        return 1;
    };
    let mut length = 1usize;
    while length < remaining {
        match scene.house_atlases.get(start + length).copied() {
            Some(next) if next == slot => length += 1,
            _ => break,
        }
    }
    length
}

/// How many consecutive parts of one kind follow `index`, with run indices:
/// a house run continues while each next part is the next house.
fn contiguous(order: &[Part], index: usize, first: usize) -> usize {
    let houses = matches!(order[index], Part::House(_));
    let mut run = 1usize;
    while index + run < order.len() {
        let next = match order[index + run] {
            Part::House(next) if houses => next,
            Part::Unit(next) if !houses => next,
            _ => break,
        };
        if next != first + run {
            break;
        }
        run += 1;
    }
    run
}

/// One shape instance: four pixel corners, then linear RGB and alpha one.
fn push_shape(bytes: &mut Vec<u8>, quad: &ShapeQuad) {
    for corner in quad.corners {
        for value in corner {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for channel in quad.color {
        bytes.extend_from_slice(&channel.to_le_bytes());
    }
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
}

/// One sprite instance: cell rect, atlas UV rect and the team colour.
fn push_sprite(bytes: &mut Vec<u8>, quad: &SpriteQuad) {
    for value in quad
        .top_left
        .iter()
        .chain(quad.size.iter())
        .chain(quad.uv_min.iter())
        .chain(quad.uv_size.iter())
    {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for channel in quad.team {
        bytes.extend_from_slice(&(f32::from(channel) / 255.0).to_le_bytes());
    }
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
}

/// Upload one atlas or mask texture from the loaded sheet.
fn sprite_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sheet: &Sheet,
    mask: bool,
) -> wgpu::Texture {
    let label = if mask {
        "kam2play-mask"
    } else {
        "kam2play-atlas"
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: sheet.width(),
            height: sheet.height(),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes = if mask { sheet.mask() } else { sheet.base() };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(sheet.width() * 4),
            rows_per_image: Some(sheet.height()),
        },
        wgpu::Extent3d {
            width: sheet.width(),
            height: sheet.height(),
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Map the readback buffer, strip its row padding and return straight RGBA8.
fn read_pixels(
    device: &wgpu::Device,
    readback: &wgpu::Buffer,
    width: u32,
    height: u32,
    padded: u32,
    bgra: bool,
) -> Result<Vec<u8>, PlayError> {
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| PlayError::Readback(format!("poll: {error}")))?;
    receiver
        .recv()
        .map_err(|error| PlayError::Readback(format!("map channel: {error}")))?
        .map_err(|error| PlayError::Readback(format!("map: {error}")))?;

    let row_bytes =
        usize::try_from(width * 4).map_err(|_| PlayError::Readback("row bytes overflow".into()))?;
    let stride =
        usize::try_from(padded).map_err(|_| PlayError::Readback("row stride overflow".into()))?;
    let rows =
        usize::try_from(height).map_err(|_| PlayError::Readback("height overflow".into()))?;
    let mut rgba = vec![0u8; row_bytes * rows];
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| PlayError::Readback(format!("mapped range: {error}")))?;
        for (row, destination) in rgba.chunks_exact_mut(row_bytes).enumerate() {
            let offset = row * stride;
            destination.copy_from_slice(&mapped[offset..offset + row_bytes]);
        }
    }
    readback.unmap();
    if bgra {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(rgba)
}

/// Encode straight RGBA8 bytes as a PNG.
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, PlayError> {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| PlayError::Png(error.to_string()))?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_render::kam2_iso::IsoCamera;
    use kam2_game::{build_with, HouseManifest, Units};

    fn cottage() -> HouseManifest {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("content")
            .join("kam2")
            .join("house")
            .join("manifest.json");
        HouseManifest::parse(&std::fs::read_to_string(path).expect("the cottage manifest reads"))
            .expect("the cottage manifest parses")
    }

    fn quad() -> ShapeQuad {
        ShapeQuad {
            corners: [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]],
            color: [0.5, 0.25, 1.0],
        }
    }

    fn sprite() -> SpriteQuad {
        SpriteQuad {
            top_left: [0.0, 0.0],
            size: [64.0, 64.0],
            uv_min: [0.0, 0.0],
            uv_size: [0.25, 0.25],
            team: [255, 255, 255],
        }
    }

    #[test]
    fn a_shape_instance_is_twelve_floats() {
        let mut bytes = Vec::new();
        push_shape(&mut bytes, &quad());
        assert_eq!(bytes.len(), INSTANCE_STRIDE);
        assert_eq!(bytes[0..4], 0.0f32.to_le_bytes());
        assert_eq!(bytes[32..36], 0.5f32.to_le_bytes());
        assert_eq!(bytes[44..48], 1.0f32.to_le_bytes());
    }

    #[test]
    fn a_sprite_instance_carries_the_team_colour() {
        let quad = SpriteQuad {
            top_left: [10.0, 20.0],
            size: [48.0, 40.0],
            uv_min: [0.75, 0.0],
            uv_size: [0.125, 0.25],
            team: [43, 111, 214],
        };
        let mut bytes = Vec::new();
        push_sprite(&mut bytes, &quad);
        assert_eq!(bytes.len(), INSTANCE_STRIDE);
        assert_eq!(bytes[0..4], 10.0f32.to_le_bytes());
        assert_eq!(bytes[8..12], 48.0f32.to_le_bytes());
        let blue = f32::from(214u8) / 255.0;
        assert_eq!(bytes[40..44], blue.to_le_bytes());
    }

    #[test]
    fn runs_only_merge_contiguous_indices() {
        let order = [
            Part::House(0),
            Part::House(1),
            Part::Unit(0),
            Part::House(2),
            Part::Unit(1),
        ];
        assert_eq!(contiguous(&order, 0, 0), 2);
        assert_eq!(contiguous(&order, 3, 2), 1);
        assert_eq!(contiguous(&order, 2, 0), 1);
        assert_eq!(contiguous(&order, 4, 1), 1);
    }

    #[test]
    fn a_house_run_splits_where_the_atlas_slot_changes() {
        let scene = Scene {
            ground: Vec::new(),
            houses: vec![sprite(); 4],
            house_atlases: vec![0, 2, 2, 3],
            units: Vec::new(),
            order: vec![
                Part::House(0),
                Part::House(1),
                Part::House(2),
                Part::House(3),
            ],
        };
        assert_eq!(same_slot(&scene, 0, 4), 1, "the cottage stands alone");
        assert_eq!(same_slot(&scene, 1, 3), 2, "the two farms share one atlas");
        assert_eq!(same_slot(&scene, 3, 1), 1, "the last house stands alone");
    }

    #[test]
    fn the_upload_plan_keeps_ground_before_the_billboards() {
        let camera = IsoCamera::new(1280, 720);
        let sim = kam2_game::match_start().expect("start village");
        let scene = build_with(&sim, &camera, &Units::StandIn, &cottage(), 0);
        let mut shapes = Vec::new();
        for quad in &scene.ground {
            push_shape(&mut shapes, quad);
        }
        assert_eq!(
            shapes.len(),
            scene.ground.len() * INSTANCE_STRIDE,
            "the shape buffer holds the ground only"
        );
        assert!(shapes.len() <= MAX_SHAPES * INSTANCE_STRIDE);
        assert_eq!(scene.houses.len(), sim.houses().len());
        assert!(scene.houses.len() <= MAX_HOUSES);
        let mut houses = Vec::new();
        for quad in &scene.houses {
            push_sprite(&mut houses, quad);
        }
        assert_eq!(houses.len(), scene.houses.len() * INSTANCE_STRIDE);
        assert!(scene.units.len() <= MAX_SPRITES);
        assert_eq!(scene.order.len(), scene.houses.len() + scene.units.len());
    }
}
