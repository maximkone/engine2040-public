//! Windowed proof of the KAM1b sprite layer.
//!
//! Draws the KAM1a diamond field, three infantry walking at different
//! depths, one tank hull and a turret that follows a moving target. Hulls
//! and turrets are separate painter layers, and every team band is tinted
//! through the mask atlas. The frame is read back, checked against the CPU
//! tint maths and saved as a PNG.
//!
//! The turret aim is deterministic: frame 12 of a fixed target circle. A
//! still image therefore shows the aim that the CPU gate expects.
//!
//! [`run_proof_with_hud`] adds one `engine-ui` HUD frame through
//! [`crate::hud_pass`], so the KAM1c receipt carries the scene and the HUD.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine_ui::HudFrame;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::hud_pass::{self, HudSurface};
use crate::kam2_iso::{IsoCamera, TILE_COUNT, TILE_WORLD};
use crate::kam2_sprite_atlas::{
    hull_team_pixel, infantry_team_pixel, marker_centre_pixel, turret_barrel_tip, SpriteArt,
    CELL_H, CELL_W, ROWS, ROW_HULL, ROW_INFANTRY, ROW_MARKER, ROW_TURRET,
};
use crate::kam2_sprites::{
    depth_order, facing_from_direction, team_tint, turret_facing, Facing, Sprite, SpriteAtlas,
};

/// Window width in logical pixels.
pub const WINDOW_W: u32 = 1280;
/// Window height in logical pixels.
pub const WINDOW_H: u32 = 720;
/// `copy_texture_to_buffer` rows must be 256-byte aligned.
const ROW_ALIGN: u32 = 256;
/// Frames drawn before the receipt frame; the moving target changes the aim.
const CAPTURE_FRAME: u32 = 12;
/// How many surface frames may be skipped before the proof gives up.
const MAX_EMPTY_FRAMES: u32 = 120;
/// Sprite instances the frame may submit.
const MAX_SPRITES: usize = 16;
/// Instance stride in bytes: fourteen floats plus eight bytes of padding.
const INSTANCE_STRIDE: usize = 64;
/// Light grass from the style-lock palette.
const GRASS: [u8; 3] = [0x8F, 0xA8, 0x4E];
/// Light dirt from the style-lock palette.
const DIRT: [u8; 3] = [0xC9, 0xA2, 0x6B];
/// Ambient from the style-lock palette, used as the clear color.
const AMBIENT: [u8; 3] = [0x3A, 0x42, 0x50];
/// Team blue from the style-lock palette.
const TEAM_BLUE: [u8; 3] = [0x2B, 0x6F, 0xD6];
/// Team red from the style-lock palette.
const TEAM_RED: [u8; 3] = [0xC6, 0x37, 0x2E];
/// No team: the mask leaves these pixels untouched.
const TEAM_NONE: [u8; 3] = [255, 255, 255];
/// Offset from the projected ground pixel to a ground sprite's top-left.
const GROUND_TOP_LEFT: [f32; 2] = [-24.0, -38.0];
/// Offset from the projected ground pixel to a turret cell's top-left; the
/// turret art is centred 20 pixels above the tank's ground point.
const TURRET_TOP_LEFT: [f32; 2] = [-24.0, -40.0];
/// Tank ground position, world units.
const TANK_GROUND: [f32; 2] = [-5.0, 0.0];
/// The tank hull faces east in every frame.
const TANK_FACING: Facing = Facing::E;
/// Centre of the target circle, world units.
const TARGET_CENTRE: [f32; 2] = [0.5, 0.0];
/// Radius of the target circle, world units.
const TARGET_RADIUS: f32 = 4.0;
/// Radians the target advances per frame.
const TARGET_STEP: f32 = 0.18;

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

struct GroundOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_ground(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> GroundOut {
    let tx = ii % 16u;
    let ty = ii / 16u;
    let half = 8.0;
    var corner = vec2<f32>(-0.5, -0.5);
    if (vi == 1u) {
        corner = vec2<f32>(-0.5, 0.5);
    } else if (vi == 2u) {
        corner = vec2<f32>(0.5, -0.5);
    } else if (vi == 3u) {
        corner = vec2<f32>(0.5, 0.5);
    }
    let tile = vec2<f32>(f32(tx) + 0.5 - half, f32(ty) + 0.5 - half) * cam.tile_world;
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
    var out: GroundOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = select(cam.dirt, cam.grass, ((tx + ty) & 1u) == 0u);
    return out;
}

@fragment
fn fs_ground(in: GroundOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}

struct SpriteIn {
    @location(0) ground: vec2<f32>,
    @location(1) top_left: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) uv_min: vec2<f32>,
    @location(4) uv_size: vec2<f32>,
    @location(5) team: vec4<f32>,
};

struct SpriteOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) team: vec3<f32>,
};

@vertex
fn vs_sprite(@builtin(vertex_index) vi: u32, item: SpriteIn) -> SpriteOut {
    let corner = strip_corner(vi);
    let rel = vec3<f32>(item.ground.x - cam.center.x, 0.0, item.ground.y - cam.center.y);
    let ground_px = cam.viewport * 0.5
        + vec2<f32>(dot(rel, cam.screen_right), -dot(rel, cam.screen_up) * cam.y_scale)
            / cam.world_per_px;
    let px = ground_px + item.top_left + corner * item.size;
    let ndc = vec2<f32>(
        px.x / cam.viewport.x * 2.0 - 1.0,
        1.0 - px.y / cam.viewport.y * 2.0,
    );
    var out: SpriteOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
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

/// One windowed proof run: sprite count, HUD receipt, PNG path and byte size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteProof {
    /// Sprite quads submitted in the frame (hulls plus turrets).
    pub sprites: u32,
    /// HUD draw batches composited on the receipt frame; `0` without a HUD.
    pub hud_draws: u32,
    /// HUD pixels blended on the receipt frame; `0` without a HUD.
    pub hud_pixels: usize,
    /// PNG written under the repo target directory.
    pub png: PathBuf,
    /// PNG file size in bytes.
    pub bytes: usize,
}

/// Why the windowed sprite proof failed.
#[derive(Debug)]
pub enum SpriteProofError {
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
    /// A sampled sprite pixel did not carry the colour the CPU maths predicts.
    Pixel(String),
    /// The PNG could not be encoded.
    Png(String),
    /// The PNG could not be written.
    Io(std::io::Error),
    /// The HUD frame did not fit the readback surface.
    Hud(crate::hud_pass::HudPassError),
    /// No surface frame arrived, or every frame was retried away.
    NoFrame,
}

impl fmt::Display for SpriteProofError {
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
            Self::Hud(error) => write!(f, "hud pass: {error}"),
            Self::NoFrame => write!(f, "no surface frame was captured"),
        }
    }
}

impl std::error::Error for SpriteProofError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
            Self::Surface(error) => Some(error),
            Self::Adapter(error) => Some(error),
            Self::Device(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Hud(error) => Some(error),
            _ => None,
        }
    }
}

/// Open the real window, draw the field and the sprites, write `output` and
/// return its receipt. The event loop exits after the receipt frame.
pub fn run_proof(output: PathBuf) -> Result<SpriteProof, SpriteProofError> {
    run_proof_with_hud(output, None)
}

/// Like [`run_proof`], with one HUD frame composited over the readback frame
/// through [`crate::hud_pass`]: the PNG carries the scene and the HUD, and a
/// HUD that paints no pixels fails the pixel gate. The event loop exits after
/// the receipt frame.
///
/// # Errors
///
/// Returns [`SpriteProofError`] for window, GPU, readback, pixel-gate or PNG
/// failures, including a HUD surface that does not fit the frame.
pub fn run_proof_with_hud(
    output: PathBuf,
    hud: Option<HudFrame>,
) -> Result<SpriteProof, SpriteProofError> {
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).map_err(SpriteProofError::Io)?;
    }
    let event_loop = EventLoop::new().map_err(SpriteProofError::EventLoop)?;
    let mut app = SpriteProofApp {
        output,
        hud,
        state: None,
        result: None,
        empty_frames: 0,
    };
    event_loop
        .run_app(&mut app)
        .map_err(SpriteProofError::EventLoop)?;
    match app.result {
        Some(result) => result,
        None => Err(SpriteProofError::NoFrame),
    }
}

struct SpriteProofApp {
    output: PathBuf,
    hud: Option<HudFrame>,
    state: Option<SpriteWindow>,
    result: Option<Result<SpriteProof, SpriteProofError>>,
    empty_frames: u32,
}

impl SpriteProofApp {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: SpriteProofError) {
        self.result = Some(Err(error));
        event_loop.exit();
    }
}

impl ApplicationHandler for SpriteProofApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("engine2040 KAM1b sprite proof")
            .with_inner_size(winit::dpi::LogicalSize::new(
                f64::from(WINDOW_W),
                f64::from(WINDOW_H),
            ));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(event_loop, SpriteProofError::Window(error)),
        };
        match SpriteWindow::new(window, self.hud.take()) {
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
                    Some(state) => state.render(&self.output),
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
                            self.fail(event_loop, SpriteProofError::NoFrame);
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

/// One sprite quad, laid out for the sprite vertex buffer.
#[derive(Clone, Copy, Debug, Default)]
struct SpriteInstance {
    /// Ground anchor in world units, x east and z south.
    ground: [f32; 2],
    /// Screen offset from the ground pixel to the cell's top-left.
    top_left: [f32; 2],
    /// Quad size in screen pixels.
    size: [f32; 2],
    /// Atlas UV origin.
    uv_min: [f32; 2],
    /// Atlas UV size.
    uv_size: [f32; 2],
    /// Team colour in sRGB 0..1; the fourth slot is unused.
    team: [f32; 4],
}

const SPRITE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x2,
    3 => Float32x2,
    4 => Float32x2,
    5 => Float32x4,
];

/// One infantry seed: where it starts and its step per frame.
#[derive(Clone, Copy, Debug)]
struct WalkerSeed {
    start: [f32; 2],
    step: [f32; 2],
}

/// Three walkers at different depths, each with its own ground direction.
const WALKERS: [WalkerSeed; 3] = [
    WalkerSeed {
        start: [-6.5, 2.0],
        step: [0.03, 0.02],
    },
    WalkerSeed {
        start: [3.0, -3.0],
        step: [0.02, -0.03],
    },
    WalkerSeed {
        start: [5.0, 4.5],
        step: [-0.03, 0.0],
    },
];

/// Everything one frame draws, plus the values the pixel gates re-derive.
struct Scene {
    walkers: [Sprite; 3],
    tank: Sprite,
    turret: Facing,
    target: [f32; 2],
    instances: Vec<SpriteInstance>,
}

impl Scene {
    fn at(frame: u32) -> Self {
        let travel = frame as f32;
        let mut walkers = [Sprite::new([0.0, 0.0], Facing::S); 3];
        for (slot, seed) in walkers.iter_mut().zip(WALKERS) {
            let ground = [
                seed.start[0] + seed.step[0] * travel,
                seed.start[1] + seed.step[1] * travel,
            ];
            let facing = facing_from_direction(seed.step).unwrap_or(Facing::S);
            *slot = Sprite::new(ground, facing);
        }
        let tank = Sprite::new(TANK_GROUND, TANK_FACING);
        let phase = travel * TARGET_STEP;
        let target = [
            TARGET_CENTRE[0] + TARGET_RADIUS * phase.cos(),
            TARGET_CENTRE[1] + TARGET_RADIUS * phase.sin(),
        ];
        let turret = turret_facing(tank.ground, target).unwrap_or(tank.facing);

        let layout = SpriteAtlas {
            cell_w: CELL_W,
            cell_h: CELL_H,
            rows: ROWS,
        };
        let mut hulls: Vec<(Sprite, u32, [u8; 3])> = walkers
            .iter()
            .map(|sprite| (*sprite, ROW_INFANTRY, TEAM_BLUE))
            .collect();
        hulls.push((tank, ROW_HULL, TEAM_RED));
        hulls.push((Sprite::new(target, Facing::S), ROW_MARKER, TEAM_NONE));
        let turrets = [(Sprite::new(tank.ground, turret), ROW_TURRET, TEAM_RED)];

        let mut instances = Vec::with_capacity(hulls.len() + turrets.len());
        let hull_sprites: Vec<Sprite> = hulls.iter().map(|hull| hull.0).collect();
        for index in depth_order(&hull_sprites) {
            let (sprite, row, team) = hulls[index];
            push_instance(&mut instances, &sprite, row, team, GROUND_TOP_LEFT, layout);
        }
        for (sprite, row, team) in turrets {
            push_instance(&mut instances, &sprite, row, team, TURRET_TOP_LEFT, layout);
        }
        Self {
            walkers,
            tank,
            turret,
            target,
            instances,
        }
    }

    fn sprites(&self) -> u32 {
        self.instances.len() as u32
    }
}

fn push_instance(
    out: &mut Vec<SpriteInstance>,
    sprite: &Sprite,
    row: u32,
    team: [u8; 3],
    top_left: [f32; 2],
    layout: SpriteAtlas,
) {
    let [uv_min, uv_size] = layout
        .cell_uv(row, sprite.facing)
        .expect("fixed atlas holds every row and facing");
    out.push(SpriteInstance {
        ground: sprite.ground,
        top_left,
        size: [CELL_W as f32, CELL_H as f32],
        uv_min,
        uv_size,
        team: [
            f32::from(team[0]) / 255.0,
            f32::from(team[1]) / 255.0,
            f32::from(team[2]) / 255.0,
            1.0,
        ],
    });
}

fn instance_bytes(instances: &[SpriteInstance]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(instances.len() * INSTANCE_STRIDE);
    for instance in instances {
        for value in instance
            .ground
            .iter()
            .chain(instance.top_left.iter())
            .chain(instance.size.iter())
            .chain(instance.uv_min.iter())
            .chain(instance.uv_size.iter())
            .chain(instance.team.iter())
        {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.resize(bytes.len() + (INSTANCE_STRIDE - 14 * 4), 0);
    }
    bytes
}

struct SpriteWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    ground: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    atlas_bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    camera: IsoCamera,
    art: SpriteArt,
    hud: Option<HudFrame>,
    frame: u32,
    bgra: bool,
}

impl SpriteWindow {
    fn new(window: Arc<Window>, hud: Option<HudFrame>) -> Result<Self, SpriteProofError> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = crate::backend::new_instance(crate::backend::wgpu_backends(pick.primary));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(SpriteProofError::Surface)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(SpriteProofError::Adapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("kam1b-sprite-proof"),
            ..Default::default()
        }))
        .map_err(SpriteProofError::Device)?;

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
        .ok_or(SpriteProofError::NoSrgbFormat)?;
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

        let art = SpriteArt::build();
        let atlas = sprite_texture(&device, &queue, &art, false);
        let mask = sprite_texture(&device, &queue, &art, true);
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
        let mask_view = mask.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kam1b-atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kam1b-wgsl"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kam1b-camera"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kam1b-camera-layout"),
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
        let camera_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kam1b-camera-bind"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kam1b-atlas-layout"),
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
        });
        let atlas_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kam1b-atlas-bind"),
            layout: &atlas_layout,
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
        });

        let ground_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kam1b-ground-layout"),
                bind_group_layouts: &[Some(&camera_layout)],
                immediate_size: 0,
            });
        let sprite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kam1b-sprite-layout"),
                bind_group_layouts: &[Some(&camera_layout), Some(&atlas_layout)],
                immediate_size: 0,
            });
        let ground = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kam1b-ground-pipeline"),
            layout: Some(&ground_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ground"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ground"),
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
        let sprites = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kam1b-sprite-pipeline"),
            layout: Some(&sprite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sprite"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: INSTANCE_STRIDE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &SPRITE_ATTRIBUTES,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kam1b-sprites"),
            size: (MAX_SPRITES * INSTANCE_STRIDE) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            ground,
            sprites,
            uniform,
            camera_bind,
            atlas_bind,
            instances,
            camera: IsoCamera::new(width, height),
            art,
            hud,
            frame: 0,
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

    /// Draw one frame; the receipt frame also reads the pixels back and
    /// writes the PNG. `None` means the frame was drawn but is not the
    /// receipt yet, or the surface was not ready.
    fn render(&mut self, output: &Path) -> Result<Option<SpriteProof>, SpriteProofError> {
        let width = self.config.width;
        let height = self.config.height;
        if width < 64 || height < 64 {
            return Ok(None);
        }
        let scene = Scene::at(self.frame);
        let capture = self.frame >= CAPTURE_FRAME;
        self.queue
            .write_buffer(&self.instances, 0, &instance_bytes(&scene.instances));
        self.queue
            .write_buffer(&self.uniform, 0, &self.pack_uniform());

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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kam1b-frame"),
            });
        {
            let clear = linear(AMBIENT);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kam1b-scene"),
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
            pass.set_pipeline(&self.ground);
            pass.set_bind_group(0, &self.camera_bind, &[]);
            pass.draw(0..4, 0..TILE_COUNT);
            pass.set_pipeline(&self.sprites);
            pass.set_bind_group(0, &self.camera_bind, &[]);
            pass.set_bind_group(1, &self.atlas_bind, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            pass.draw(0..4, 0..scene.sprites());
        }

        let bytes_per_row = width * 4;
        let padded = bytes_per_row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        let readback = capture.then(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("kam1b-readback"),
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

        if !capture {
            self.frame += 1;
            return Ok(None);
        }
        let Some(readback) = readback else {
            return Err(SpriteProofError::Readback("no readback buffer".into()));
        };
        let mut rgba = self.read_pixels(&readback, width, height, padded)?;
        self.gate(&rgba, width, height, &scene)?;
        let (hud_draws, hud_pixels) = match self.hud.as_ref() {
            Some(frame) => {
                let mut surface =
                    HudSurface::new(width, height, &mut rgba).map_err(SpriteProofError::Hud)?;
                let receipt = hud_pass::draw(frame, &mut surface);
                if receipt.drawn_pixels == 0 {
                    return Err(SpriteProofError::Pixel("hud drew no pixels".into()));
                }
                (
                    u32::try_from(receipt.draws).unwrap_or(u32::MAX),
                    receipt.drawn_pixels,
                )
            }
            None => (0, 0),
        };

        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| SpriteProofError::Png(error.to_string()))?;
        }
        std::fs::write(output, &png).map_err(SpriteProofError::Io)?;
        Ok(Some(SpriteProof {
            sprites: scene.sprites(),
            hud_draws,
            hud_pixels,
            png: output.to_path_buf(),
            bytes: png.len(),
        }))
    }

    fn read_pixels(
        &mut self,
        readback: &wgpu::Buffer,
        width: u32,
        height: u32,
        padded: u32,
    ) -> Result<Vec<u8>, SpriteProofError> {
        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| SpriteProofError::Readback(format!("poll: {error}")))?;
        receiver
            .recv()
            .map_err(|error| SpriteProofError::Readback(format!("map channel: {error}")))?
            .map_err(|error| SpriteProofError::Readback(format!("map: {error}")))?;

        let row_bytes = usize::try_from(width * 4)
            .map_err(|_| SpriteProofError::Readback("row bytes overflow".into()))?;
        let stride = usize::try_from(padded)
            .map_err(|_| SpriteProofError::Readback("row stride overflow".into()))?;
        let mut rgba = vec![
            0u8;
            row_bytes
                * usize::try_from(height).map_err(
                    |_| SpriteProofError::Readback("height overflow".into())
                )?
        ];
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|error| SpriteProofError::Readback(format!("mapped range: {error}")))?;
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
        Ok(rgba)
    }

    fn gate(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        scene: &Scene,
    ) -> Result<(), SpriteProofError> {
        let grass_count = count_near(rgba, GRASS);
        let dirt_count = count_near(rgba, DIRT);
        // The full field holds about 131k pixels of each colour; the sprites
        // cover a few thousand. A clipped or half-drawn field loses far more.
        if grass_count < 128_000 || dirt_count < 128_000 || grass_count + dirt_count < 257_000 {
            return Err(SpriteProofError::Pixel(format!(
                "field coverage grass={grass_count} dirt={dirt_count} total={}",
                grass_count + dirt_count
            )));
        }
        let layout = self.art.layout();
        let gate = PixelGate {
            camera: &self.camera,
            art: &self.art,
            layout,
        };
        for (index, walker) in scene.walkers.iter().enumerate() {
            gate.expect_team(
                rgba,
                width,
                height,
                PixelRef {
                    sprite: *walker,
                    top_left: GROUND_TOP_LEFT,
                    row: ROW_INFANTRY,
                    texel: infantry_team_pixel(walker.facing),
                },
                TEAM_BLUE,
                &format!("walker {index}"),
            )?;
        }
        gate.expect_team(
            rgba,
            width,
            height,
            PixelRef {
                sprite: scene.tank,
                top_left: GROUND_TOP_LEFT,
                row: ROW_HULL,
                texel: hull_team_pixel(scene.tank.facing),
            },
            TEAM_RED,
            "tank hull",
        )?;
        gate.expect_texel(
            rgba,
            width,
            height,
            PixelRef {
                sprite: Sprite::new(scene.tank.ground, scene.turret),
                top_left: TURRET_TOP_LEFT,
                row: ROW_TURRET,
                texel: turret_barrel_tip(scene.turret),
            },
            "turret barrel tip",
        )?;
        gate.expect_texel(
            rgba,
            width,
            height,
            PixelRef {
                sprite: Sprite::new(scene.target, Facing::S),
                top_left: GROUND_TOP_LEFT,
                row: ROW_MARKER,
                texel: marker_centre_pixel(),
            },
            "target marker",
        )?;
        Ok(())
    }
}

fn sprite_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    art: &SpriteArt,
    mask: bool,
) -> wgpu::Texture {
    let label = if mask { "kam1b-mask" } else { "kam1b-atlas" };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: art.width(),
            height: art.height(),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes = if mask {
        art.mask_bytes()
    } else {
        art.base_bytes()
    };
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
            bytes_per_row: Some(art.width() * 4),
            rows_per_image: Some(art.height()),
        },
        wgpu::Extent3d {
            width: art.width(),
            height: art.height(),
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn cell_local(
    layout: SpriteAtlas,
    row: u32,
    facing: Facing,
    texel: [u32; 2],
) -> Result<[u32; 2], SpriteProofError> {
    let origin = layout
        .cell_origin(row, facing)
        .ok_or_else(|| SpriteProofError::Pixel("atlas cell outside the grid".into()))?;
    Ok([texel[0] - origin[0], texel[1] - origin[1]])
}

/// One pixel the gate checks: sprite, its layer offset, atlas row and the
/// absolute texel inside that cell.
#[derive(Clone, Copy, Debug)]
struct PixelRef {
    sprite: Sprite,
    top_left: [f32; 2],
    row: u32,
    texel: [u32; 2],
}

/// CPU side of the pixel gates: atlas, layout and camera of one frame.
struct PixelGate<'a> {
    camera: &'a IsoCamera,
    art: &'a SpriteArt,
    layout: SpriteAtlas,
}

impl PixelGate<'_> {
    fn expect_team(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        pixel: PixelRef,
        team: [u8; 3],
        name: &str,
    ) -> Result<(), SpriteProofError> {
        let base = self.art.base_rgba(pixel.texel);
        let mask = self.art.mask_alpha(pixel.texel);
        let want = team_tint([base[0], base[1], base[2]], mask, team);
        let at = self.pixel_at(pixel)?;
        expect_near(rgba, width, height, at, want, name)
    }

    fn expect_texel(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        pixel: PixelRef,
        name: &str,
    ) -> Result<(), SpriteProofError> {
        let texel = self.art.base_rgba(pixel.texel);
        let at = self.pixel_at(pixel)?;
        expect_near(
            rgba,
            width,
            height,
            at,
            [texel[0], texel[1], texel[2]],
            name,
        )
    }

    fn pixel_at(&self, pixel: PixelRef) -> Result<[f32; 2], SpriteProofError> {
        let local = cell_local(self.layout, pixel.row, pixel.sprite.facing, pixel.texel)?;
        Ok(sprite_pixel(
            self.camera,
            &pixel.sprite,
            pixel.top_left,
            local,
        ))
    }
}

/// Screen pixel of one cell texel: the quad origin is fractional, so the
/// containing framebuffer pixel is the floor of the texel's left edge.
fn sprite_pixel(
    camera: &IsoCamera,
    sprite: &Sprite,
    top_left: [f32; 2],
    texel: [u32; 2],
) -> [f32; 2] {
    let ground = camera.project([sprite.ground[0], 0.0, sprite.ground[1]]);
    [
        ground[0] + top_left[0] + texel[0] as f32,
        ground[1] + top_left[1] + texel[1] as f32,
    ]
}

fn expect_near(
    rgba: &[u8],
    width: u32,
    height: u32,
    at: [f32; 2],
    want: [u8; 3],
    name: &str,
) -> Result<(), SpriteProofError> {
    let x = at[0].floor();
    let y = at[1].floor();
    if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
        return Err(SpriteProofError::Pixel(format!(
            "{name} sample outside frame"
        )));
    }
    let row = usize::try_from(width)
        .map_err(|_| SpriteProofError::Pixel("frame width overflow".into()))?;
    let index = (usize::try_from(y as u32)
        .map_err(|_| SpriteProofError::Pixel("pixel row overflow".into()))?
        * row
        + usize::try_from(x as u32)
            .map_err(|_| SpriteProofError::Pixel("pixel column overflow".into()))?)
        * 4;
    let got = [rgba[index], rgba[index + 1], rgba[index + 2]];
    if got
        .iter()
        .zip(want)
        .any(|(value, expected)| value.abs_diff(expected) > 3)
    {
        return Err(SpriteProofError::Pixel(format!(
            "{name} at {x},{y}: pixel {got:?}, want {want:?}"
        )));
    }
    Ok(())
}

fn linear(rgb: [u8; 3]) -> [f32; 3] {
    [
        engine_assets::bc::srgb_byte_to_linear(rgb[0]),
        engine_assets::bc::srgb_byte_to_linear(rgb[1]),
        engine_assets::bc::srgb_byte_to_linear(rgb[2]),
    ]
}

fn count_near(rgba: &[u8], want: [u8; 3]) -> usize {
    rgba.chunks_exact(4)
        .filter(|pixel| {
            pixel[0].abs_diff(want[0]) <= 3
                && pixel[1].abs_diff(want[1]) <= 3
                && pixel[2].abs_diff(want[2]) <= 3
        })
        .count()
}
