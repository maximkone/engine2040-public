//! KAM10: the play window draws every house kind from its own atlas.
//!
//! Opens one 1280x720 window, plays the demo village at a fixed 10 Hz, draws
//! the field, the houses from the building atlas of their own kind and both
//! armies from the packed soldier atlas, and saves a PNG receipt on F12 and
//! again at frame 120, then exits by itself. The demo village is fixed: one
//! road, eleven houses and four swordsmen, so one receipt shows all five
//! house kinds. The procedural stand-in is drawn only when the soldier atlas
//! file is missing; the receipt names the atlas every house kind drew from.
//!
//! Controls: W/S/D and the arrow keys pan; A pans left and, on the press, orders
//! the nearest side-0 swordsman onto its nearest enemy (the same key carries
//! both the listed pan and the listed A order); 1..5 pick warehouse, farm, mill,
//! bakery and woodcutter; a left click places the selected house on the clicked
//! tile; U fields a swordsman for side 0 and E one for side 1; space pauses.
//! The camera never rotates, as the style lock asks.

mod art;
mod gpu;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use engine_render::kam2_iso::IsoCamera;
use engine_ui::HudFrame;
use kam2_game::{hud_frame, scene, screen_to_tile, script, MAP_H, MAP_W};
use kam2_sim::{Command, HouseKind, Refusal, Sim, UnitKind};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use art::{slot_of, Art, BuildingArts, HOUSE_KINDS};
use gpu::{Gpu, Outcome};

/// Window width in logical pixels.
const WINDOW_W: u32 = 1280;
/// Window height in logical pixels.
const WINDOW_H: u32 = 720;
/// Frame that triggers the automatic receipt, as the order asks.
const AUTO_CAPTURE_FRAME: u32 = 120;
/// One simulation step: the fixed 10 Hz tick.
const TICK: Duration = Duration::from_millis(100);
/// Steps one frame may catch up on after a stall; the rest is dropped.
const MAX_CATCH_UP: u32 = 4;
/// Surface frames that may be skipped in a row before the window gives up.
const MAX_EMPTY_FRAMES: u32 = 240;
/// Camera pan speed in world units per second.
const PAN_SPEED: f32 = 6.0;
/// Row of the one east-west road of the demo village.
const ROAD_ROW: i32 = 6;
/// The demo village: side, kind and top-left tile of every house. Five kinds on
/// each side, so one receipt shows all five kinds; every footprint touches the
/// road, so every house is linked to its own warehouse from tick zero.
const DEMO_HOUSES: [(u8, HouseKind, i32, i32); 11] = [
    (0, HouseKind::Warehouse, 0, 3),
    (0, HouseKind::Farm, 3, 4),
    (0, HouseKind::Mill, 5, 4),
    (0, HouseKind::Bakery, 7, 4),
    (0, HouseKind::Woodcutter, 9, 4),
    (1, HouseKind::Warehouse, 0, 7),
    (1, HouseKind::Farm, 4, 7),
    (1, HouseKind::Mill, 6, 7),
    (1, HouseKind::Bakery, 8, 7),
    (1, HouseKind::Woodcutter, 10, 7),
    (1, HouseKind::Farm, 12, 7),
];
/// The demo armies: side and tile of every swordsman.
const DEMO_UNITS: [(u8, i32, i32); 4] = [(0, 12, 5), (0, 14, 5), (1, 12, 9), (1, 14, 9)];
/// Ticks the demo village plays before the first frame, so every house is past
/// its build time and draws its finished cell.
const STARTUP_TICKS: u32 = 60;

/// Why the play window failed.
#[derive(Debug)]
enum PlayError {
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
    /// The surface offers no 8-bit sRGB format.
    NoSrgbFormat,
    /// Mapping, polling or row stripping failed.
    Readback(String),
    /// The composited frame failed a pixel requirement.
    Pixel(String),
    /// The PNG could not be encoded.
    Png(String),
    /// The HUD frame did not fit the readback surface.
    Hud(engine_render::hud_pass::HudPassError),
    /// A sprite atlas disagrees with its manifest.
    Art(String),
    /// An atlas PNG did not decode.
    Image(String),
    /// The demo village no longer fits its fixed layout.
    Match(Refusal),
    /// The PNG could not be written.
    Io(std::io::Error),
    /// The surface never presented a frame.
    NoFrame,
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            Self::Hud(error) => write!(f, "hud pass: {error}"),
            Self::Art(error) => write!(f, "sprite art: {error}"),
            Self::Image(error) => write!(f, "atlas png: {error}"),
            Self::Match(refusal) => write!(f, "demo village refused: {refusal:?}"),
            Self::Io(error) => write!(f, "io: {error}"),
            Self::NoFrame => write!(f, "no surface frame was presented"),
        }
    }
}

impl std::error::Error for PlayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EventLoop(error) => Some(error),
            Self::Window(error) => Some(error),
            Self::Surface(error) => Some(error),
            Self::Adapter(error) => Some(error),
            Self::Device(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::NoSrgbFormat
            | Self::Readback(_)
            | Self::Pixel(_)
            | Self::Png(_)
            | Self::Art(_)
            | Self::Image(_)
            | Self::Match(_)
            | Self::NoFrame => None,
        }
    }
}

/// One automatic receipt: what the captured frame drew and where it went.
#[derive(Clone, Debug)]
struct Receipt {
    /// House kinds the frame drew.
    kinds: usize,
    /// House cells the frame drew.
    houses: usize,
    /// The PNG the frame was written to.
    png: PathBuf,
    /// Size of that PNG in bytes.
    bytes: usize,
    /// The atlas every house kind drew from.
    sources: String,
}

/// Which pan directions are held right now; screen space, y down.
#[derive(Clone, Copy, Debug, Default)]
struct Pan {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl Pan {
    /// Pan direction in world units: x east, z south, one unit long per axis.
    fn vector(&self) -> [f32; 2] {
        [axis(self.right, self.left), axis(self.down, self.up)]
    }
}

/// One axis of a held-key pan: `1`, `-1`, or `0` when both or neither is held.
fn axis(positive: bool, negative: bool) -> f32 {
    match (positive, negative) {
        (true, false) => 1.0,
        (false, true) => -1.0,
        (true, true) | (false, false) => 0.0,
    }
}

/// The live window: match, camera, input state and the GPU leg.
struct Play {
    window: Arc<Window>,
    gpu: Gpu,
    sim: Sim,
    art: Art,
    arts: BuildingArts,
    camera: IsoCamera,
    paused: bool,
    selected: HouseKind,
    cursor: [f32; 2],
    pan: Pan,
    notice: String,
    last_tick: Duration,
    last_frame: Duration,
    frame: u32,
    f12_pending: bool,
    empty_frames: u32,
    auto_png: PathBuf,
    f12_png: PathBuf,
}

impl Play {
    /// Build the demo village, load the soldier and house art and open the
    /// surface.
    ///
    /// # Errors
    ///
    /// Returns [`PlayError`] when an atlas is missing or broken, when the demo
    /// village no longer fits its fixed layout, or when the GPU refuses the
    /// window.
    fn new(
        window: Arc<Window>,
        auto_png: PathBuf,
        f12_png: PathBuf,
        art_dir: &Path,
        house_dir: &Path,
        buildings_dir: &Path,
    ) -> Result<Self, PlayError> {
        let art = Art::load(art_dir)?;
        let mut arts = BuildingArts::new(house_dir, buildings_dir);
        arts.load_all()?;
        let mut gpu = Gpu::new(Arc::clone(&window), art.sheet())?;
        for slot in 0..kam2_game::HOUSE_SLOTS {
            if let Some(sheet) = arts.sheet(slot) {
                gpu.set_house_sheet(slot, sheet);
            }
        }
        let mut sim = demo_village()?;
        for _ in 0..STARTUP_TICKS {
            sim.tick();
        }
        let size = window.inner_size();
        let camera = IsoCamera::new(size.width.max(1), size.height.max(1));
        let now = engine_clock::now();
        Ok(Self {
            window,
            gpu,
            sim,
            art,
            arts,
            camera,
            paused: false,
            selected: HouseKind::Warehouse,
            cursor: [0.0, 0.0],
            pan: Pan::default(),
            notice: String::new(),
            last_tick: now,
            last_frame: now,
            frame: 0,
            f12_pending: false,
            empty_frames: 0,
            auto_png,
            f12_png,
        })
    }

    /// Follow a window resize; the camera keeps its world center and zoom.
    fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        self.camera.resize(width, height);
    }

    /// One key event: pan flags follow the press and release, orders fire once.
    fn key(&mut self, event: &KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        match &event.logical_key {
            Key::Character(text) => {
                let name = text.to_ascii_lowercase();
                match name.as_str() {
                    "w" => self.pan.up = pressed,
                    "s" => self.pan.down = pressed,
                    "d" => self.pan.right = pressed,
                    "a" => {
                        self.pan.left = pressed;
                        if pressed && !event.repeat {
                            self.attack();
                        }
                    }
                    "u" if pressed && !event.repeat => self.spawn(0),
                    "e" if pressed && !event.repeat => self.spawn(1),
                    "1" if pressed => self.selected = HouseKind::Warehouse,
                    "2" if pressed => self.selected = HouseKind::Farm,
                    "3" if pressed => self.selected = HouseKind::Mill,
                    "4" if pressed => self.selected = HouseKind::Bakery,
                    "5" if pressed => self.selected = HouseKind::Woodcutter,
                    _ => {}
                }
            }
            Key::Named(NamedKey::ArrowUp) => self.pan.up = pressed,
            Key::Named(NamedKey::ArrowDown) => self.pan.down = pressed,
            Key::Named(NamedKey::ArrowLeft) => self.pan.left = pressed,
            Key::Named(NamedKey::ArrowRight) => self.pan.right = pressed,
            Key::Named(NamedKey::Space) if pressed && !event.repeat => self.paused = !self.paused,
            Key::Named(NamedKey::F12) if pressed && !event.repeat => self.f12_pending = true,
            _ => {}
        }
    }

    /// Place the selected house on the tile under the cursor.
    fn place(&mut self) {
        let Some((x, y)) = screen_to_tile(&self.camera, self.cursor) else {
            "OFF MAP".clone_into(&mut self.notice);
            return;
        };
        match self
            .sim
            .apply(Command::PlaceHouse {
                kind: self.selected,
                x,
                y,
            })
        {
            Ok(()) => {
                self.notice = format!("PLACED {}", self.selected.name().to_ascii_uppercase());
            }
            Err(refusal) => {
                self.notice = format!("REFUSED {refusal:?}").to_ascii_uppercase();
            }
        }
    }

    /// Field one swordsman for `side` on the nearest free tile.
    fn spawn(&mut self, side: u8) {
        let Some((x, y)) = script::spawn_tile(&self.sim, side) else {
            "NO ROOM".clone_into(&mut self.notice);
            return;
        };
        match self.sim.apply_for(
            side,
            Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side,
                x,
                y,
            },
        ) {
            Ok(()) => self.notice = format!("SPAWN S{side}"),
            Err(refusal) => {
                self.notice = format!("REFUSED {refusal:?}").to_ascii_uppercase();
            }
        }
    }

    /// The A order: the nearest side-0 swordsman attacks its nearest side-1 enemy.
    fn attack(&mut self) {
        let Some((id, target)) = script::nearest_enemy(&self.sim, 0) else {
            "NO TARGET".clone_into(&mut self.notice);
            return;
        };
        match self.sim.apply_for(0, Command::Attack { id, target }) {
            Ok(()) => self.notice = format!("ATTACK {id}->{target}"),
            Err(refusal) => {
                self.notice = format!("REFUSED {refusal:?}").to_ascii_uppercase();
            }
        }
    }

    /// Move the camera by the held pan keys, clamped to the map extents.
    fn pan_camera(&mut self, now: Duration) {
        let seconds = now.saturating_sub(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let [vx, vz] = self.pan.vector();
        if vx == 0.0 && vz == 0.0 {
            return;
        }
        let [cx, cz] = self.camera.world_center();
        let limit_x = MAP_W as f32 * 0.5;
        let limit_z = MAP_H as f32 * 0.5;
        self.camera.pan_to([
            (cx + vx * PAN_SPEED * seconds).clamp(-limit_x, limit_x),
            (cz + vz * PAN_SPEED * seconds).clamp(-limit_z, limit_z),
        ]);
    }

    /// Play the fixed 10 Hz ticks that came due since the last frame.
    fn advance_ticks(&mut self, now: Duration) {
        let mut due = now.saturating_sub(self.last_tick);
        let mut steps = 0u32;
        while due >= TICK && steps < MAX_CATCH_UP {
            self.last_tick += TICK;
            if !self.paused {
                self.sim.tick();
            }
            steps += 1;
            due = now.saturating_sub(self.last_tick);
        }
        if due >= TICK {
            self.last_tick = now;
        }
    }

    /// Draw one frame; a capture frame returns the automatic receipt.
    fn render(&mut self) -> Result<Option<Receipt>, PlayError> {
        let now = engine_clock::now();
        self.pan_camera(now);
        self.advance_ticks(now);
        let tick = self.sim.ticks();
        let frame_scene = scene::build_with_arts(
            &self.sim,
            &self.camera,
            self.art.units(),
            self.arts.registry(),
            tick,
        );
        let hud: HudFrame = hud_frame(
            self.gpu.width(),
            self.gpu.height(),
            &self.sim,
            self.paused,
            self.selected,
            &self.notice,
        );
        let auto = self.frame >= AUTO_CAPTURE_FRAME;
        let f12 = !auto && self.f12_pending;
        let capture_path = if auto {
            Some(self.auto_png.clone())
        } else if f12 {
            Some(self.f12_png.clone())
        } else {
            None
        };
        match self.gpu.draw(&frame_scene, &hud, capture_path.as_deref())? {
            Outcome::Skipped => {
                self.empty_frames = self.empty_frames.saturating_add(1);
                if self.empty_frames >= MAX_EMPTY_FRAMES {
                    return Err(PlayError::NoFrame);
                }
                Ok(None)
            }
            Outcome::Drawn => {
                self.empty_frames = 0;
                self.frame = self.frame.saturating_add(1);
                Ok(None)
            }
            Outcome::Captured(capture) => {
                self.empty_frames = 0;
                if auto {
                    Ok(Some(Receipt {
                        kinds: kinds_in(&self.sim),
                        houses: frame_scene.houses.len(),
                        png: capture.png,
                        bytes: capture.bytes,
                        sources: sources_line(&self.arts),
                    }))
                } else {
                    println!(
                        "KAM10 F12 hud={} units={} unit_source={} png={} bytes={}",
                        capture.hud_draws,
                        frame_scene.units.len(),
                        self.art.source(),
                        capture.png.display(),
                        capture.bytes
                    );
                    self.f12_pending = false;
                    self.frame = self.frame.saturating_add(1);
                    Ok(None)
                }
            }
        }
    }
}

/// The demo village: one road, eleven houses of five kinds on each side and
/// four swordsmen.
///
/// # Errors
///
/// Returns the first [`Refusal`] when the fixed layout no longer fits the map
/// or the stock.
fn demo_village() -> Result<Sim, Refusal> {
    let mut sim = Sim::new(MAP_W, MAP_H);
    for x in 0..MAP_W {
        sim.apply(Command::PlaceRoad { x, y: ROAD_ROW })?;
    }
    for (side, kind, x, y) in DEMO_HOUSES {
        sim.apply_for(side, Command::PlaceHouse { kind, x, y })?;
    }
    for (side, x, y) in DEMO_UNITS {
        sim.apply_for(
            side,
            Command::SpawnUnit {
                kind: UnitKind::Swordsman,
                side,
                x,
                y,
            },
        )?;
    }
    Ok(sim)
}

/// How many house kinds the village holds.
fn kinds_in(sim: &Sim) -> usize {
    let mut kinds: Vec<HouseKind> = sim.houses().iter().map(|house| house.kind).collect();
    kinds.sort_by_key(|kind| kind.index());
    kinds.dedup();
    kinds.len()
}

/// The atlas every house kind drew from: the cottage first, then the kinds in
/// registry order.
fn sources_line(arts: &BuildingArts) -> String {
    let mut line = format!("cottage={}", arts.source(0));
    for kind in HOUSE_KINDS {
        line.push(' ');
        line.push_str(kind.name());
        line.push('=');
        line.push_str(arts.source(slot_of(kind)));
    }
    line
}

/// The winit application: one window, one receipt.
struct PlayApp {
    auto_png: PathBuf,
    f12_png: PathBuf,
    art_dir: PathBuf,
    house_dir: PathBuf,
    buildings_dir: PathBuf,
    state: Option<Play>,
    result: Option<Result<Receipt, PlayError>>,
}

impl PlayApp {
    /// Prepare the target directory the PNGs go to and name the atlas folders.
    fn new() -> Result<Self, PlayError> {
        let root = repo_root();
        let target = root.join("target");
        std::fs::create_dir_all(&target).map_err(PlayError::Io)?;
        let content = root.join("content").join("kam2");
        Ok(Self {
            auto_png: target.join("kam2play_buildings.png"),
            f12_png: target.join("kam2play-f12.png"),
            art_dir: content.join("soldier"),
            house_dir: content.join("house"),
            buildings_dir: content.join("buildings"),
            state: None,
            result: None,
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: PlayError) {
        self.result = Some(Err(error));
        event_loop.exit();
    }
}

impl ApplicationHandler for PlayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("KAM10 play window")
            .with_inner_size(LogicalSize::new(f64::from(WINDOW_W), f64::from(WINDOW_H)));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(event_loop, PlayError::Window(error)),
        };
        match Play::new(
            window,
            self.auto_png.clone(),
            self.f12_png.clone(),
            &self.art_dir,
            &self.house_dir,
            &self.buildings_dir,
        ) {
            Ok(play) => {
                self.state = Some(play);
                if let Some(play) = self.state.as_ref() {
                    play.window.request_redraw();
                }
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(play) = self.state.as_mut() {
                    play.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if self.result.is_some() {
                    return;
                }
                let outcome = match self.state.as_mut() {
                    Some(play) => play.render(),
                    None => return,
                };
                match outcome {
                    Ok(Some(receipt)) => {
                        self.result = Some(Ok(receipt));
                        event_loop.exit();
                    }
                    Ok(None) => {}
                    Err(error) => self.fail(event_loop, error),
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(play) = self.state.as_mut() {
                    play.key(&event);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(play) = self.state.as_mut() {
                    play.cursor = [position.x as f32, position.y as f32];
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(play) = self.state.as_mut() {
                    play.place();
                }
            }
            WindowEvent::Focused(false) => {
                if let Some(play) = self.state.as_mut() {
                    play.pan = Pan::default();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.result.is_none() {
            if let Some(play) = self.state.as_ref() {
                play.window.request_redraw();
            }
        }
    }
}

/// The repo root, two parents above the crate manifest.
fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn main() -> Result<(), PlayError> {
    let event_loop = EventLoop::new().map_err(PlayError::EventLoop)?;
    let mut app = PlayApp::new()?;
    event_loop.run_app(&mut app).map_err(PlayError::EventLoop)?;
    match app.result {
        Some(Ok(receipt)) => {
            println!(
                "KAM10 OK kinds={} houses={} png={} bytes={}",
                receipt.kinds,
                receipt.houses,
                receipt.png.display(),
                receipt.bytes
            );
            println!("KAM10 SOURCES {}", receipt.sources);
            Ok(())
        }
        Some(Err(error)) => Err(error),
        None => Err(PlayError::NoFrame),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kam2_sim::Ware;

    #[test]
    fn the_demo_village_holds_eleven_houses_of_five_kinds() {
        let sim = demo_village().expect("the demo village places");
        assert_eq!(sim.houses().len(), 11);
        assert_eq!(sim.units().len(), 4);
        assert_eq!(kinds_in(&sim), 5, "one receipt shows all five kinds");
        for side in [0u8, 1] {
            let mut kinds: Vec<HouseKind> =
                sim.houses_of(side).map(|house| house.kind).collect();
            kinds.sort_by_key(|kind| kind.index());
            kinds.dedup();
            assert_eq!(kinds.len(), 5, "side {side} shows all five kinds");
        }
        for house in sim.houses() {
            assert!(
                house.link.is_some(),
                "{} has no road link",
                house.kind.name()
            );
        }
    }

    #[test]
    fn the_demo_village_is_paid_for_out_of_the_starting_stock() {
        let sim = demo_village().expect("the demo village places");
        assert_eq!(sim.stock_of(0, Ware::Wood), 4);
        assert_eq!(sim.stock_of(1, Ware::Wood), 0);
        assert_eq!(sim.stock_of(0, Ware::Stone), 15);
        assert_eq!(sim.stock_of(1, Ware::Stone), 15);
    }

    #[test]
    fn every_house_is_built_after_the_startup_ticks() {
        let mut sim = demo_village().expect("the demo village places");
        for _ in 0..STARTUP_TICKS {
            sim.tick();
        }
        assert_eq!(sim.ticks(), STARTUP_TICKS);
        for house in sim.houses() {
            assert!(
                sim.ticks() >= house.built_at,
                "{} is still building",
                house.kind.name()
            );
        }
    }
}
