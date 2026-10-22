//! scene (SD7 + HC9): the speed demo as one scene in the window.
//!
//! Owns the stage (the dunes or the procedural New York), the movers (flight, web-swing, car),
//! the sword, the cut pieces, the crowd and the speed feel. The window steps it once per sim
//! tick (fixed dt, so a capture repeats), feeds it keys, and asks it for draws and a camera
//! each frame. With no key ever pressed (a `--exit` capture) an autopilot runs the show: on the
//! dunes it flies at the nearest tower and swings; in New York it races down an avenue at
//! street level with the blade through the building bases and the sidewalk, swings on webs,
//! then drives. Presentation only. Units: meters, seconds, radians.

use engine_physfx::Tier;
use engine_render::particles_gpu::EmitterKind;
use engine_render::present::{DrawList, Presenter, MAX_DRAWS_PER_FRAME};
use engine_render::v1::PbrMaterial;
use winit::keyboard::KeyCode;

use super::car::{Car, CarInput};
use super::collide::{push_out, slide};
use super::crowd::{Crowd, SHIRTS, WAKE_MPS};
use super::cut::{box_shell, cut_tower, Slide};
use super::draw::{bar_mesh, blade_mesh, flat_mesh, flat_mesh_rot, item, key, Soup};
use super::flight::{speed_feel, Flight, FlightInput};
use super::hero::{hero_meshes, HeroPose, Pose};
use super::num::{add, centroid, len, rotate, scale, sub, unit_or};
use super::stage::{Look, Stage};
use super::swing::{Swing, SwingInput};
use super::sword::{blade_hits_box, Sword, BLADE_M};
use super::traffic::{Crossing, Lane, Traffic};

/// The world's seed: one demo world, the same every run.
const SEED: u64 = 0x5EED_2040;
/// Mesh keys: falling pieces, the blade, the car, the web, the crowd.
const PIECE_KEY: u64 = 0x5D20_0000;
const BLADE_KEY: u64 = 0x5D30_0000;
const CROWD_KEY: u64 = 0x5D40_0000;
const TRAFFIC_KEY: u64 = 0x5D48_0000;
/// Cabs per lane, and how close a blow must pass a cab to hit it, meters.
const CABS_PER_LANE: usize = 8;
const CAB_REACH_M: f32 = 3.0;
/// A falling piece is dropped after this long, seconds.
const PIECE_LIFE_S: f32 = 14.0;
/// Most pieces falling at once (the oldest goes first).
const MAX_PIECES: usize = 40;
/// Chase camera: behind and above the body, meters; it looks this far ahead.
const CHASE_BACK_M: f32 = 14.0;
const CHASE_UP_M: f32 = 5.5;
const LOOK_AHEAD_M: f32 = 30.0;
/// The camera's far plane: the whole 2 km world.
const FAR_M: f32 = 6000.0;
/// Dust thrown where a blade goes through a tower.
const CUT_DUST: u32 = 60;
const CUT_SPARKS: u32 = 40;
/// Turn rate from the keys, rad/s, and the pitch rate.
const TURN_RAD_S: f32 = 1.6;
const PITCH_RAD_S: f32 = 1.0;
/// The dune autopilot swings when the target tower is this close, meters.
const AUTO_SWING_M: f32 = 40.0;
/// Walkers and buildings around a hit are searched within this reach, meters.
const BODY_REACH_M: f32 = 2.5;
const WALL_REACH_M: f32 = 40.0;
/// Walkers in New York.
const WALKERS: usize = 1600;
/// Blocks this close to the autopilot's avenue get walkers three times as often, meters.
const BUSY_M: f32 = 300.0;
/// New York autopilot: street-level altitude, how far in from the curb it rides, and its
/// minute: fly, then swing, then drive (seconds).
const AUTO_ALT_M: f32 = 4.0;
const AUTO_CURB_M: f32 = 6.0;
const AUTO_FLY_S: f32 = 25.0;
const AUTO_SWING_S: f32 = 45.0;
const AUTO_LOOP_S: f32 = 60.0;
/// The drive leg starts this far from the avenue's middle, so it has the whole avenue to run.
const AVENUE_END_M: f32 = 700.0;
/// Body radii against building walls, meters, and the lamps that light the night.
const BODY_RADIUS_M: f32 = 1.5;
const CAR_RADIUS_M: f32 = 2.5;
const WALL_SEARCH_M: f32 = 40.0;
/// A wall hit slows a flyer to this share of its speed, and the next hit waits this long.
const CRASH_KEEPS: f32 = 0.5;
const CRASH_COOLDOWN_S: f32 = 0.5;
/// Lamps lit in the night preset: how many (the shader block holds eight), how far from the
/// camera they count, their reach, their strength and warm colour.
const LAMPS_LIT: usize = 8;
const LAMP_NEAR_M: f32 = 160.0;
const LAMP_RANGE_M: f32 = 60.0;
const LAMP_INTENSITY: f32 = 80.0;
const LAMP_COLOUR: [f32; 3] = [1.0, 0.72, 0.4];
/// Web timing for the autopilot: the longest hold is twice this, then it flies free, seconds.
const WEB_HOLD_S: f32 = 1.6;
const WEB_FREE_S: f32 = 0.4;

const BLADE: PbrMaterial = PbrMaterial {
    base_color: [0.78, 0.80, 0.84],
    metallic: 1.0,
    roughness: 0.25,
    exposure: 1.0,
};
const CAR_BODY: PbrMaterial = PbrMaterial {
    base_color: [0.65, 0.08, 0.06],
    metallic: 0.6,
    roughness: 0.35,
    exposure: 1.0,
};
const CAB: PbrMaterial = PbrMaterial {
    base_color: [0.95, 0.72, 0.05],
    metallic: 0.3,
    roughness: 0.4,
    exposure: 1.0,
};
const SUIT: PbrMaterial = PbrMaterial {
    base_color: [0.08, 0.16, 0.6],
    metallic: 0.0,
    roughness: 0.55,
    exposure: 1.0,
};
const CAPE: PbrMaterial = PbrMaterial {
    base_color: [0.72, 0.04, 0.05],
    metallic: 0.0,
    roughness: 0.7,
    exposure: 1.0,
};
const SKIN: PbrMaterial = PbrMaterial {
    base_color: [0.85, 0.62, 0.5],
    metallic: 0.0,
    roughness: 0.6,
    exposure: 1.0,
};
const WEB: PbrMaterial = PbrMaterial {
    base_color: [0.92, 0.92, 0.95],
    metallic: 0.0,
    roughness: 0.6,
    exposure: 1.0,
};
/// Shirt colours of the crowd, one mesh each.
const SHIRT_COLOURS: [[f32; 3]; SHIRTS] = [
    [0.55, 0.12, 0.1],
    [0.1, 0.2, 0.45],
    [0.75, 0.68, 0.5],
    [0.12, 0.12, 0.13],
];

/// Fly, swing on webs (New York only) or drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Fly,
    Swing,
    Drive,
}

/// One cut-off top, sliding then falling.
struct Piece {
    shell: engine_destruction::csg::PropMesh,
    pivot: [f32; 3],
    slide: Slide,
    age_s: f32,
    look: Look,
}

/// The New York autopilot's state: the avenue it rides, its direction, its web clock.
#[derive(Clone, Copy, Debug, Default)]
struct Auto {
    /// West and east curb of the avenue, meters.
    avenue: Option<(f32, f32)>,
    south: bool,
    web_s: f32,
}

/// The whole demo.
pub(crate) struct SpeedDemo {
    stage: Stage,
    pieces: Vec<Piece>,
    flight: Flight,
    swing: Swing,
    car: Car,
    sword: Sword,
    crowd: Option<Crowd>,
    traffic: Option<Traffic>,
    /// Cabs knocked over.
    cabs_hit: u32,
    /// Walls the body hit, and the wait before the next counts.
    bumps: u32,
    bump_cd_s: f32,
    /// The night preset is on: street lamps light the frame.
    pub(crate) night: bool,
    mode: Mode,
    held: Vec<KeyCode>,
    /// A key was pressed this run: the autopilot hands over for good.
    touched: bool,
    /// Keys and clicks are ignored (a hands-off capture).
    pub(crate) hands_off: bool,
    auto: Auto,
    pending_fx: Vec<([f32; 3], [f32; 3])>,
    last_tip: Option<[f32; 3]>,
    t_s: f32,
    cuts: u32,
    top_mps: f32,
}

impl SpeedDemo {
    /// Build the world: the dunes, or New York with `nyc`; the flyer above its edge.
    pub(crate) fn new(nyc: bool) -> Result<Self, String> {
        let stage = if nyc {
            Stage::nyc(SEED)
        } else {
            Stage::dunes(SEED)?
        };
        let avenue = stage.city().and_then(|c| c.avenue_near(0.0));
        let crowd = match stage.city() {
            Some(city) => Some(Crowd::new(
                &busy_loops(city.walker_loops(), avenue),
                WALKERS,
                SEED,
                Tier::Comfort,
            )?),
            None => None,
        };
        let (x, z, up) = match avenue {
            Some((_, east)) => (east - AUTO_CURB_M, 700.0, AUTO_ALT_M),
            None => (0.0, 300.0, 60.0),
        };
        let traffic = stage.city().map(|city| {
            let lanes = city
                .lanes()
                .into_iter()
                .map(|(start, end)| Lane { start, end })
                .collect();
            let crossings = city
                .crossings()
                .into_iter()
                .map(|at| Crossing {
                    at,
                    phase_offset_s: 0.0,
                })
                .collect();
            Traffic::new(lanes, crossings, CABS_PER_LANE, SEED)
        });
        let ground = |x: f32, z: f32| stage.height_at(x, z);
        let start = [x, ground(x, z) + up, z];
        let car = Car::new(x, z, car_heading(0.0), true, &ground);
        Ok(Self {
            flight: Flight::new(start),
            swing: Swing::new(start, [0.0; 3]),
            car,
            stage,
            pieces: Vec::new(),
            sword: Sword::default(),
            crowd,
            traffic,
            cabs_hit: 0,
            bumps: 0,
            bump_cd_s: 0.0,
            night: false,
            mode: Mode::Fly,
            held: Vec::new(),
            touched: false,
            hands_off: false,
            auto: Auto {
                avenue,
                ..Auto::default()
            },
            pending_fx: Vec::new(),
            last_tip: None,
            t_s: 0.0,
            cuts: 0,
            top_mps: 0.0,
        })
    }

    /// A key went down or up. Tab cycles the movers, F toggles always-faster, E or a click
    /// swings the sword, Space fires and lets go of the web while swinging; the rest are held
    /// for movement.
    pub(crate) fn key(&mut self, code: KeyCode, down: bool) {
        if self.hands_off {
            return;
        }
        // Only a press hands over: a stray release (the Enter that launched us) must not.
        self.touched |= down;
        self.press(code, down);
    }

    fn press(&mut self, code: KeyCode, down: bool) {
        if down && !self.held.contains(&code) {
            self.held.push(code);
            match code {
                KeyCode::Tab => self.next_mode(),
                KeyCode::KeyF => {
                    self.flight.always_faster = !self.flight.always_faster;
                    self.car.always_faster = !self.car.always_faster;
                }
                KeyCode::KeyE => self.sword.swing(),
                KeyCode::Space if self.mode == Mode::Swing => {
                    let near = self.stage.boxes_near(self.swing.pos, 130.0);
                    self.swing.attach(&near);
                }
                _ => {}
            }
        } else if !down {
            self.held.retain(|k| *k != code);
            if code == KeyCode::Space && self.mode == Mode::Swing {
                self.swing.release();
            }
        }
    }

    /// The mouse button swings the sword.
    pub(crate) fn click(&mut self) {
        if self.hands_off {
            return;
        }
        self.touched = true;
        self.sword.swing();
    }

    /// Fly, then swing (New York), then drive, then fly; each hands its place and speed on.
    fn next_mode(&mut self) {
        let (pos, yaw, vel) = self.body();
        let faster = self.flight.always_faster;
        let stage = &self.stage;
        let ground = |x: f32, z: f32| stage.height_at(x, z);
        self.mode = match self.mode {
            Mode::Fly if self.stage.is_nyc() => {
                self.swing = Swing::new(pos, vel);
                Mode::Swing
            }
            Mode::Fly | Mode::Swing => {
                self.car = Car::new(pos[0], pos[2], car_heading(yaw), faster, &ground);
                Mode::Drive
            }
            Mode::Drive => {
                self.flight = Flight::new([pos[0], pos[1] + 2.0, pos[2]]);
                self.flight.yaw = yaw;
                self.flight.always_faster = self.car.always_faster;
                self.flight.speed = self.flight.speed.max(len(vel));
                Mode::Fly
            }
        };
    }

    fn holding(&self, code: KeyCode) -> f32 {
        if self.held.contains(&code) {
            1.0
        } else {
            0.0
        }
    }

    /// One fixed step of everything.
    pub(crate) fn step(&mut self, dt: f32) {
        self.t_s += dt;
        if !self.touched {
            if self.stage.is_nyc() {
                self.nyc_autopilot(dt);
            } else {
                self.autopilot();
            }
        }
        let before = self.body().0;
        self.move_body(dt);
        self.collide_body(before, dt);
        self.sword.step(dt);
        self.top_mps = self.top_mps.max(self.speed());
        self.cut_what_the_blade_meets();
        self.knock_the_crowd(dt);
        for piece in &mut self.pieces {
            piece.slide.step(dt);
            piece.age_s += dt;
        }
        self.pieces.retain(|p| p.age_s < PIECE_LIFE_S);
    }

    fn move_body(&mut self, dt: f32) {
        let turn = self.holding(KeyCode::KeyD) - self.holding(KeyCode::KeyA);
        let forward = self.holding(KeyCode::KeyW) - self.holding(KeyCode::KeyS);
        let climb = self.holding(KeyCode::Space) - self.holding(KeyCode::ControlLeft);
        let pitch = self.holding(KeyCode::ArrowUp) - self.holding(KeyCode::ArrowDown);
        let space = self.held.contains(&KeyCode::Space);
        let reel = self.held.contains(&KeyCode::KeyW);
        let boost = self.held.contains(&KeyCode::ShiftLeft);
        let stage = &self.stage;
        let ground = |x: f32, z: f32| stage.height_at(x, z);
        match self.mode {
            Mode::Fly => {
                let input = FlightInput {
                    forward,
                    strafe: 0.0,
                    up: climb,
                    yaw_delta: turn * TURN_RAD_S * dt,
                    pitch_delta: pitch * PITCH_RAD_S * dt,
                    boost,
                };
                self.flight.step(&input, dt, &ground);
            }
            Mode::Swing => {
                let input = SwingInput {
                    hold: space,
                    reel,
                    steer: turn,
                };
                self.swing.step(&input, dt, &ground);
            }
            Mode::Drive => {
                let input = CarInput {
                    throttle: self.holding(KeyCode::KeyW),
                    brake: self.holding(KeyCode::KeyS),
                    // The car's +steer turns it left; D turns right like the flyer.
                    steer: -turn,
                    handbrake: space,
                };
                self.car.step(&input, dt, &ground);
            }
        }
    }

    /// Keep the body out of the buildings: it stops on the wall face it came through, loses
    /// the speed that pointed into the wall and, the first time, throws dust and sparks.
    fn collide_body(&mut self, before: [f32; 3], dt: f32) {
        self.bump_cd_s = (self.bump_cd_s - dt).max(0.0);
        if !self.stage.is_nyc() {
            return;
        }
        let (pos, _, _) = self.body();
        let radius = if self.mode == Mode::Drive {
            CAR_RADIUS_M
        } else {
            BODY_RADIUS_M
        };
        let near = self.stage.boxes_near(pos, WALL_SEARCH_M);
        let Some(hit) = push_out(before, pos, radius, &near) else {
            return;
        };
        let speed = self.speed();
        match self.mode {
            Mode::Fly => {
                self.flight.pos = hit.pos;
                if self.bump_cd_s <= 0.0 {
                    self.flight.speed *= CRASH_KEEPS;
                }
            }
            Mode::Swing => {
                self.swing.pos = hit.pos;
                self.swing.vel = slide(self.swing.vel, hit.normal);
            }
            Mode::Drive => self.car.bump(hit.pos, hit.normal),
        }
        if self.bump_cd_s <= 0.0 && speed > WAKE_MPS {
            self.bump_cd_s = CRASH_COOLDOWN_S;
            self.bumps += 1;
            self.pending_fx.push((hit.pos, hit.normal));
        }
    }

    /// Speed of whatever is moving, m/s.
    pub(crate) fn speed(&self) -> f32 {
        match self.mode {
            Mode::Fly => self.flight.speed,
            Mode::Swing => self.swing.speed(),
            Mode::Drive => self.car.speed_mps(),
        }
    }

    /// Position, yaw and velocity of the body carrying the sword.
    fn body(&self) -> ([f32; 3], f32, [f32; 3]) {
        match self.mode {
            Mode::Fly => {
                let dir = self.flight.look();
                (
                    self.flight.pos,
                    self.flight.yaw,
                    scale(dir, self.flight.speed),
                )
            }
            Mode::Swing => {
                let v = self.swing.vel;
                let yaw = if v[0].hypot(v[2]) > 0.5 {
                    v[0].atan2(-v[2])
                } else {
                    self.flight.yaw
                };
                (self.swing.pos, yaw, v)
            }
            Mode::Drive => {
                // The car's own forward is +z; the demo's yaw looks down -z at 0.
                let (pos, rot) = self.car.pose();
                let fwd = rotate(rot, [0.0, 0.0, 1.0]);
                let yaw = fwd[0].atan2(-fwd[2]);
                (pos, yaw, scale([fwd[0], 0.0, fwd[2]], self.car.speed_mps()))
            }
        }
    }

    fn cut_what_the_blade_meets(&mut self) {
        let (pos, yaw, vel) = self.body();
        let blade = self.sword.blade(pos, yaw, vel);
        let Some(plane) = blade.plane else {
            return;
        };
        let reach = BLADE_M + 4.0;
        for i in 0..self.stage.standing.len() {
            let (min, max) = self.stage.standing[i].bounds;
            let dx = (min[0] - pos[0]).max(pos[0] - max[0]);
            let dz = (min[2] - pos[2]).max(pos[2] - max[2]);
            if dx > reach || dz > reach || !blade_hits_box(&blade, min, max) {
                continue;
            }
            let Some(cut) = cut_tower(&self.stage.standing[i].shell, &plane) else {
                continue;
            };
            let pivot = centroid(&cut.falls.positions);
            self.pieces.push(Piece {
                slide: Slide::start(&cut, scale(vel, 0.25)),
                shell: cut.falls,
                pivot,
                age_s: 0.0,
                look: self.stage.standing[i].look,
            });
            self.pending_fx.push((pivot, cut.normal));
            self.stage.cut(i, cut.stays);
            self.cuts += 1;
        }
        if self.pieces.len() > MAX_PIECES {
            let drop = self.pieces.len() - MAX_PIECES;
            self.pieces.drain(..drop);
        }
    }

    /// The body and the blade knock walkers over: whatever passes a walker faster than the
    /// wake speed throws them as a ragdoll; the buildings around become walls they hit.
    fn knock_the_crowd(&mut self, dt: f32) {
        let (pos, yaw, vel) = self.body();
        let blade = self.sword.blade(pos, yaw, vel);
        let tip_vel = match self.last_tip {
            Some(last) if dt > 0.0 => scale(sub(blade.tip, last), 1.0 / dt),
            _ => vel,
        };
        self.last_tip = Some(blade.tip);
        let mid = scale(add(blade.hilt, blade.tip), 0.5);
        let blows = [
            (pos, vel),
            (mid, scale(add(vel, tip_vel), 0.5)),
            (blade.tip, tip_vel),
        ];
        if let Some(traffic) = self.traffic.as_mut() {
            let stage = &self.stage;
            traffic.step(dt, &|x: f32, z: f32| stage.height_at(x, z));
            for (at, v) in blows {
                self.cabs_hit += traffic.hit(at, CAB_REACH_M, v);
            }
        }
        let Some(crowd) = self.crowd.as_mut() else {
            return;
        };
        crowd.step(dt);
        for (at, v) in blows {
            if len(v) >= WAKE_MPS {
                let walls = self.stage.boxes_near(at, WALL_REACH_M);
                crowd.hit(at, BODY_REACH_M, v, &walls);
            }
        }
    }

    /// Dunes, no keys yet: fly at the nearest standing tower so its middle passes the blade,
    /// and swing when close. Holds altitude at the tower's middle height.
    fn autopilot(&mut self) {
        let (pos, yaw, _) = self.body();
        let Some((dist, c)) = self
            .stage
            .standing
            .iter()
            .filter(|s| s.cuts == 0)
            .map(|s| {
                let (min, max) = s.bounds;
                let c = scale(add(min, max), 0.5);
                ((c[0] - pos[0]).hypot(c[2] - pos[2]), c)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
        else {
            return;
        };
        // Aim to pass with the tower at the blade's middle: offset left of the centre.
        let right = [yaw.cos(), 0.0, yaw.sin()];
        let aim = [
            c[0] - right[0] * BLADE_M * 0.6,
            c[1],
            c[2] - right[2] * BLADE_M * 0.6,
        ];
        self.steer_to(aim);
        if dist < AUTO_SWING_M && !self.sword.swinging() {
            self.sword.swing();
        }
    }

    /// Press the keys that turn toward `aim` and hold its height (flying) or the throttle.
    fn steer_to(&mut self, aim: [f32; 3]) {
        let (pos, yaw, _) = self.body();
        let want_yaw = (aim[0] - pos[0]).atan2(-(aim[2] - pos[2]));
        let turn = wrap(want_yaw - yaw);
        let webbing = self.mode == Mode::Swing && self.held.contains(&KeyCode::Space);
        self.held.retain(|k| {
            !matches!(
                k,
                KeyCode::KeyA | KeyCode::KeyD | KeyCode::Space | KeyCode::ControlLeft
            )
        });
        if webbing {
            self.held.push(KeyCode::Space);
        }
        if turn > 0.05 {
            self.held.push(KeyCode::KeyD);
        } else if turn < -0.05 {
            self.held.push(KeyCode::KeyA);
        }
        match self.mode {
            Mode::Fly if pos[1] < aim[1] - 1.5 => self.held.push(KeyCode::Space),
            Mode::Fly if pos[1] > aim[1] + 1.5 => self.held.push(KeyCode::ControlLeft),
            Mode::Drive if !self.held.contains(&KeyCode::KeyW) => self.held.push(KeyCode::KeyW),
            _ => {}
        }
    }

    /// New York, no keys yet: ride the avenue at street level with the blade through the
    /// building bases, turning at the ends; fly, then swing, then drive, every minute.
    fn nyc_autopilot(&mut self, dt: f32) {
        let Some((west, east)) = self.auto.avenue else {
            return;
        };
        let phase = self.t_s.rem_euclid(AUTO_LOOP_S);
        let want = if phase < AUTO_FLY_S {
            Mode::Fly
        } else if phase < AUTO_SWING_S {
            Mode::Swing
        } else {
            Mode::Drive
        };
        if want != self.mode {
            self.next_mode();
            if self.mode == Mode::Drive {
                self.start_drive(west, east);
            }
        }
        let (pos, _, _) = self.body();
        if !self.auto.south && pos[2] < -720.0 {
            self.auto.south = true;
        } else if self.auto.south && pos[2] > 720.0 {
            self.auto.south = false;
        }
        // The blade is on the right: going north that is the east curb, going south the west.
        let (lane_x, ahead) = if self.auto.south {
            (west + AUTO_CURB_M, 80.0)
        } else {
            (east - AUTO_CURB_M, -80.0)
        };
        let ground = self.stage.height_at(lane_x, pos[2]);
        self.steer_to([lane_x, ground + AUTO_ALT_M, pos[2] + ahead]);
        if self.mode == Mode::Swing {
            self.auto_web(dt, lane_x, ahead);
        }
        if !self.sword.swinging() {
            self.sword.swing();
        }
    }

    /// The car cannot turn about at a wall, so its leg starts at the far end of the avenue from
    /// the body, facing the other end, with the whole avenue to run.
    fn start_drive(&mut self, west: f32, east: f32) {
        let (pos, _, _) = self.body();
        let south = pos[2] < 0.0;
        let (x, z, yaw) = if south {
            (west + AUTO_CURB_M, -AVENUE_END_M, std::f32::consts::PI)
        } else {
            (east - AUTO_CURB_M, AVENUE_END_M, 0.0)
        };
        self.auto.south = south;
        let stage = &self.stage;
        let ground = |x: f32, z: f32| stage.height_at(x, z);
        let car = Car::new(x, z, car_heading(yaw), self.car.always_faster, &ground);
        self.car = car;
    }

    /// The autopilot's web: hang it from the building face ahead on the blade side, let go
    /// once past the anchor (or after a while), fly free a moment, fire again. Picking the
    /// anchor here, along the lane, keeps the swing on the avenue.
    fn auto_web(&mut self, dt: f32, lane_x: f32, ahead: f32) {
        self.auto.web_s += dt;
        let pos = self.swing.pos;
        match self.swing.anchor {
            Some(anchor) => {
                let past = (pos[2] - anchor[2]) * ahead.signum() > 8.0;
                if past || self.auto.web_s > WEB_HOLD_S * 2.0 {
                    self.auto.web_s = 0.0;
                    self.swing.release();
                }
            }
            None if self.auto.web_s > WEB_FREE_S => {
                // The blade side's building face: east of the lane going north, west going south.
                let face = lane_x - ahead.signum() * (AUTO_CURB_M + 6.0);
                let at = [face, pos[1], pos[2] + ahead.signum() * 45.0];
                let top = self
                    .stage
                    .boxes_near(at, 4.0)
                    .iter()
                    .map(|b| b.1[1])
                    .fold(f32::MIN, f32::max);
                if top > pos[1] + 10.0 {
                    let anchor = [at[0], top.min(pos[1] + 45.0), at[2]];
                    self.swing.anchor = Some(anchor);
                    self.swing.rope_m = len(sub(anchor, pos));
                    self.auto.web_s = 0.0;
                }
            }
            None => {}
        }
    }

    /// Upload what changed and push this frame's draws: the stage, the movers, the crowd and
    /// the falling pieces last (so a frame over the draw budget sheds pieces first).
    pub(crate) fn push(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
    ) -> Result<(), String> {
        list.items.clear();
        self.stage.push(presenter, list)?;
        self.push_movers(presenter, list)?;
        let eye: [f32; 3] = list.camera.eye.into();
        self.push_crowd(presenter, list, eye)?;
        self.push_traffic(presenter, list)?;
        for (i, piece) in self.pieces.iter().enumerate() {
            let mesh = flat_mesh(
                &piece.shell,
                piece.slide.offset,
                piece.slide.axis,
                piece.slide.angle,
                piece.pivot,
            )?;
            presenter
                .upload_mesh(key(PIECE_KEY, i), &mesh)
                .map_err(|e| format!("piece: {e}"))?;
            list.items
                .push(self.stage.draw(key(PIECE_KEY, i), piece.look));
        }
        list.items.truncate(MAX_DRAWS_PER_FRAME);
        for (at, normal) in self.pending_fx.drain(..) {
            presenter.fight_burst(
                EmitterKind::Dust,
                at,
                [normal[0], normal[1] + 0.5, normal[2]],
                CUT_DUST,
            );
            presenter.fight_burst(EmitterKind::Sparks, at, normal, CUT_SPARKS);
        }
        Ok(())
    }

    /// The hero in the chase cam (flying or swinging; driving shows the car).
    fn push_hero(&self, presenter: &mut Presenter, list: &mut DrawList) -> Result<(), String> {
        let pose = match self.mode {
            Mode::Fly => Pose::Fly,
            Mode::Swing => Pose::Swing,
            Mode::Drive => return Ok(()),
        };
        let (pos, yaw, vel) = self.body();
        let pitch = vel[1].atan2(vel[0].hypot(vel[2]));
        let meshes = hero_meshes(&HeroPose {
            pos,
            yaw,
            pitch,
            pose,
            t_s: self.t_s,
        })?;
        for (i, (mesh, material)) in [
            (meshes.suit, SUIT),
            (meshes.cape, CAPE),
            (meshes.skin, SKIN),
        ]
        .into_iter()
        .enumerate()
        {
            presenter
                .upload_mesh(key(BLADE_KEY, 3 + i), &mesh)
                .map_err(|e| format!("hero: {e}"))?;
            list.items.push(item(key(BLADE_KEY, 3 + i), material));
        }
        Ok(())
    }

    /// The street lamps nearest `eye` as the night's lights.
    pub(crate) fn torch_lights(
        &self,
        eye: [f32; 3],
    ) -> Vec<engine_render::torch_light::TorchLight> {
        let Some(city) = self.stage.city() else {
            return Vec::new();
        };
        let mut near: Vec<(f32, [f32; 3])> = city
            .lamps
            .iter()
            .map(|l| (len(sub(*l, eye)), *l))
            .filter(|(d, _)| *d < LAMP_NEAR_M)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.into_iter()
            .take(LAMPS_LIT)
            .map(|(_, pos)| engine_render::torch_light::TorchLight {
                pos,
                color: LAMP_COLOUR,
                range_m: LAMP_RANGE_M,
                intensity: LAMP_INTENSITY,
            })
            .collect()
    }

    fn push_movers(&self, presenter: &mut Presenter, list: &mut DrawList) -> Result<(), String> {
        self.push_hero(presenter, list)?;
        let (pos, yaw, vel) = self.body();
        let blade = self.sword.blade(pos, yaw, vel);
        presenter
            .upload_mesh(key(BLADE_KEY, 0), &blade_mesh(blade.hilt, blade.tip)?)
            .map_err(|e| format!("blade: {e}"))?;
        list.items.push(item(key(BLADE_KEY, 0), BLADE));
        if self.mode == Mode::Drive {
            let (p, rot) = self.car.pose();
            let body = box_shell([-0.9, 0.3, -2.2], [0.9, 1.4, 2.2]);
            presenter
                .upload_mesh(key(BLADE_KEY, 1), &flat_mesh_rot(&body, p, rot)?)
                .map_err(|e| format!("car: {e}"))?;
            list.items.push(item(key(BLADE_KEY, 1), CAR_BODY));
        }
        if let (Mode::Swing, Some(anchor)) = (self.mode, self.swing.anchor) {
            presenter
                .upload_mesh(key(BLADE_KEY, 2), &bar_mesh(self.swing.pos, anchor, 0.06)?)
                .map_err(|e| format!("web: {e}"))?;
            list.items.push(item(key(BLADE_KEY, 2), WEB));
        }
        Ok(())
    }

    fn push_traffic(&self, presenter: &mut Presenter, list: &mut DrawList) -> Result<(), String> {
        let Some(traffic) = self.traffic.as_ref() else {
            return Ok(());
        };
        let cab = box_shell([-0.95, -0.75, -2.25], [0.95, 0.75, 2.25]);
        let mut soup = Soup::default();
        for (pos, rot) in traffic.poses() {
            soup.add(&cab, |p| add(rotate(rot, p), pos));
        }
        if soup.is_empty() {
            return Ok(());
        }
        for (i, mesh) in soup.meshes()?.iter().enumerate() {
            presenter
                .upload_mesh(key(TRAFFIC_KEY, i), mesh)
                .map_err(|e| format!("traffic: {e}"))?;
            list.items.push(item(key(TRAFFIC_KEY, i), CAB));
        }
        Ok(())
    }

    fn push_crowd(
        &self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        eye: [f32; 3],
    ) -> Result<(), String> {
        let Some(crowd) = self.crowd.as_ref() else {
            return Ok(());
        };
        for (i, mesh) in crowd.meshes(eye, 1.0).iter().enumerate() {
            if mesh.indices.is_empty() {
                continue;
            }
            let data = engine_render::v1::upload_mesh_tiled(
                &mesh.positions,
                &mesh.normals,
                &mesh.uvs,
                &mesh.indices,
            )
            .map_err(|e| format!("crowd: {e:?}"))?;
            presenter
                .upload_mesh(key(CROWD_KEY, i), &data)
                .map_err(|e| format!("crowd: {e}"))?;
            list.items.push(item(
                key(CROWD_KEY, i),
                PbrMaterial {
                    base_color: SHIRT_COLOURS[i % SHIRTS],
                    metallic: 0.0,
                    roughness: 0.8,
                    exposure: 1.0,
                },
            ));
        }
        Ok(())
    }

    /// Aim the frame: a chase camera behind and above the body, the field of view and the
    /// shake from the speed feel.
    pub(crate) fn frame(&self, camera: &mut engine_render::v1::Camera) {
        let (pos, yaw, vel) = self.body();
        let speed = len(vel);
        let feel = speed_feel(speed);
        let ahead = [yaw.sin(), 0.0, -yaw.cos()];
        let dir = if speed > 1.0 {
            scale(vel, 1.0 / speed)
        } else {
            ahead
        };
        let flat = unit_or([dir[0], 0.0, dir[2]], ahead);
        // Shake: two incommensurate sines, deterministic from the demo's own clock.
        let s = feel.shake_m;
        let wobble = [
            s * (self.t_s * 37.0).sin(),
            s * (self.t_s * 53.0).sin(),
            0.0,
        ];
        let mut eye = add(add(pos, scale(flat, -CHASE_BACK_M)), [0.0, CHASE_UP_M, 0.0]);
        eye = add(eye, wobble);
        let floor = self.stage.height_at(eye[0], eye[2]) + 1.5;
        eye[1] = eye[1].max(floor);
        let target = add(pos, scale(dir, LOOK_AHEAD_M));
        camera.eye = eye.into();
        camera.target = target.into();
        camera.up = [0.0, 1.0, 0.0].into();
        camera.fov_y = feel.fov_deg.to_radians();
        camera.far = FAR_M;
    }

    /// The exit receipt line.
    pub(crate) fn line(&self) -> String {
        let mode = match self.mode {
            Mode::Fly => "fly",
            Mode::Swing => "swing",
            Mode::Drive => "car",
        };
        let world = if self.stage.is_nyc() { "nyc" } else { "dunes" };
        let (groups, buildings) = self.stage.counts();
        let (ragdolls, walking) = self
            .crowd
            .as_ref()
            .map_or((0, 0), |c| (c.woken, c.walking()));
        format!(
            "arena speed demo: world={world} top_mps={:.0} cuts={} swings={} pieces={} buildings={buildings} groups={groups} ragdolls={ragdolls} walking={walking} cabs_hit={} bumps={} mode={mode} autopilot={}",
            self.top_mps,
            self.cuts,
            self.sword.swings,
            self.pieces.len(),
            self.cabs_hit,
            self.bumps,
            !self.touched,
        )
    }
}

/// The walker loops with the blocks along `avenue` listed three times, so the sidewalks the
/// autopilot rides are the busy ones.
fn busy_loops(
    loops: Vec<(super::crowd::Loop, f32)>,
    avenue: Option<(f32, f32)>,
) -> Vec<(super::crowd::Loop, f32)> {
    let Some((west, east)) = avenue else {
        return loops;
    };
    let mid = (west + east) * 0.5;
    let busy: Vec<_> = loops
        .iter()
        .filter(|(lp, _)| ((lp.min[0] + lp.max[0]) * 0.5 - mid).abs() < BUSY_M)
        .copied()
        .collect();
    let mut out = loops;
    for _ in 0..2 {
        out.extend_from_slice(&busy);
    }
    out
}

/// The demo's yaw (0 looks down -z, + turns right) as the car's heading (0 faces +z).
fn car_heading(yaw: f32) -> f32 {
    std::f32::consts::PI - yaw
}

fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// A flyer aimed at a building ends outside it, slower than it came.
    #[test]
    fn a_flyer_does_not_pass_through_a_building() {
        let mut demo = SpeedDemo::new(true).expect("New York builds");
        demo.touched = true;
        let (min, max) = demo.stage.standing[0].bounds;
        let mid_z = (min[2] + max[2]) * 0.5;
        // Fly east at the building's middle height from 60 m west of its west face.
        let y = (min[1] + max[1]) * 0.5;
        demo.flight = Flight::new([min[0] - 60.0, y, mid_z]);
        demo.flight.yaw = std::f32::consts::FRAC_PI_2;
        demo.flight.speed = 80.0;
        for _ in 0..180 {
            demo.step(DT);
        }
        assert!(demo.bumps > 0, "the wall was hit: {}", demo.line());
        let p = demo.flight.pos;
        let inside = (0..3).all(|a| p[a] > min[a] && p[a] < max[a]);
        assert!(!inside, "the flyer is outside the building: {p:?}");
    }

    /// With no key pressed the autopilot finds a tower, swings through it, and the cut
    /// leaves a falling piece: what a hands-off capture shows.
    #[test]
    fn the_autopilot_cuts_a_tower_within_a_minute() {
        let mut demo = SpeedDemo::new(false).expect("the world builds");
        assert!(!demo.stage.standing.is_empty(), "the city stands");
        for _ in 0..3600 {
            demo.step(DT);
            if demo.cuts > 0 {
                break;
            }
        }
        assert!(demo.cuts > 0, "a tower is cut within 60 s: {}", demo.line());
        assert!(!demo.pieces.is_empty(), "the cut top is falling");
        assert!(demo.top_mps > 30.0, "always-faster is on: {}", demo.top_mps);
    }

    /// Always-faster: after 20 s of flight the speed has grown; with it off it holds.
    #[test]
    fn always_faster_makes_the_flight_faster_and_f_turns_it_off() {
        let mut on = SpeedDemo::new(false).expect("builds");
        let mut off = SpeedDemo::new(false).expect("builds");
        off.key(KeyCode::KeyF, true);
        off.key(KeyCode::KeyF, false);
        on.touched = true;
        for _ in 0..1200 {
            on.step(DT);
            off.step(DT);
        }
        assert!(
            on.speed() > off.speed() * 1.5,
            "on {} vs off {}",
            on.speed(),
            off.speed()
        );
        assert!(on.line().contains("mode=fly"));
    }

    /// New York hands-off: the fly leg cuts building bases and throws walkers; the swing leg
    /// hangs from a web; the drive leg drives.
    #[test]
    fn the_new_york_autopilot_cuts_throws_swings_and_drives() {
        let mut demo = SpeedDemo::new(true).expect("New York builds");
        let mut webbed = false;
        let mut drove = false;
        for _ in 0..(60 * 60) {
            demo.step(DT);
            webbed |= demo.mode == Mode::Swing && demo.swing.anchor.is_some();
            drove |= demo.mode == Mode::Drive && demo.car.speed_mps() > 5.0;
        }
        let line = demo.line();
        assert!(demo.cuts > 0, "buildings cut: {line}");
        let thrown = demo.crowd.as_ref().map_or(0, |c| c.woken);
        assert!(thrown > 0, "walkers thrown: {line}");
        assert!(webbed, "a web held: {line}");
        assert!(drove, "the car drove: {line}");
    }

    /// Tab cycles fly, swing, drive in New York and only fly and drive on the dunes.
    #[test]
    fn tab_cycles_the_movers() {
        let mut nyc = SpeedDemo::new(true).expect("builds");
        let mut seen = Vec::new();
        for _ in 0..3 {
            nyc.key(KeyCode::Tab, true);
            nyc.key(KeyCode::Tab, false);
            seen.push(nyc.mode);
        }
        assert_eq!(seen, vec![Mode::Swing, Mode::Drive, Mode::Fly]);
        let mut dunes = SpeedDemo::new(false).expect("builds");
        dunes.key(KeyCode::Tab, true);
        assert_eq!(dunes.mode, Mode::Drive);
    }
}
