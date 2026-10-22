//! car (SD6, docs/SPRINT-33-speed-demo.md): drive on the generated terrain, ever faster.
//!
//! A rigid body with four raycast wheels. Each wheel drops straight down from its mount to
//! `ground(x, z)`; a spring and a damper turn the compression into the load (a spring never
//! pulls, and a wheel that hangs past its rest length is in the air). The tyre forces come
//! from the slip at the contact: longitudinal from throttle, brake and rolling, lateral from
//! the slip angle, both capped by the load times the friction, with the grip falling past
//! the peak slip. Always faster raises the engine's top speed over time, the same rule as
//! the flight row. Presentation only, deterministic: fixed substeps, no clock, no RNG.
//! Units: meters, seconds, kilograms, newtons, radians. Car frame: +x right, +y up, +z
//! forward; a heading of 0 faces +z.

use super::num::{add, cross, dot, len, scale, sub, unit};
use std::f32::consts::PI;

/// Car mass, kg.
const MASS_KG: f32 = 1400.0;
/// Gravity, m/s^2.
const GRAVITY_M_S2: f32 = 9.81;
/// Front axle to rear axle, m.
const WHEELBASE_M: f32 = 2.6;
/// Left wheel to right wheel, m.
const TRACK_M: f32 = 1.6;
/// Body box for the moments of inertia: length, width, height, m.
const BODY_LENGTH_M: f32 = 4.2;
const BODY_WIDTH_M: f32 = 1.8;
const BODY_HEIGHT_M: f32 = 1.4;
/// A wheel mount sits this far below the body's centre, m.
const MOUNT_BELOW_M: f32 = 0.2;
/// Wheel radius, m.
const WHEEL_RADIUS_M: f32 = 0.35;
/// Spring length with no load (the suspension's rest length), m.
const REST_LENGTH_M: f32 = 0.45;
/// Spring stiffness per wheel, N/m (about 2 Hz with a quarter of the car on it).
const SPRING_N_PER_M: f32 = 60_000.0;
/// Damper per wheel, N per m/s of compression (about half of critical).
const DAMPER_N_S_PER_M: f32 = 5_000.0;
/// Peak friction coefficient of the tyres.
const FRICTION_MU: f32 = 1.0;
/// Slip angle at the peak side grip, rad.
const PEAK_SLIP_RAD: f32 = 0.15;
/// Share of the peak side grip left once the slip is twice the peak.
const SLIDE_GRIP: f32 = 0.8;
/// Share of the side grip on a locked rear axle (handbrake).
const HANDBRAKE_GRIP: f32 = 0.5;
/// Front wheel steer angle at full lock, rad (35 degrees).
const MAX_STEER_RAD: f32 = 35.0 * PI / 180.0;
/// Engine drive over both rear wheels at full throttle, N.
const DRIVE_N: f32 = 8_400.0;
/// Service brake per wheel at full pedal, N.
const BRAKE_N: f32 = 6_000.0;
/// Extra brake on each rear wheel with the handbrake on, N.
const HANDBRAKE_N: f32 = 4_000.0;
/// Rolling resistance, share of the load.
const ROLLING_COEF: f32 = 0.015;
/// Air density, kg/m^3, and drag area (drag coefficient times frontal area), m^2.
const AIR_KG_M3: f32 = 1.2;
const DRAG_AREA_M2: f32 = 1.54;
/// Below this speed the brake and rolling forces ramp in, so a stop does not chatter, m/s.
const RAMP_SPEED_MPS: f32 = 0.2;
/// Floor on the speed that turns the side force into a slip angle, m/s.
const SLIP_SPEED_FLOOR_MPS: f32 = 1.0;
/// Spacing of the finite differences that give the ground normal, m.
const NORMAL_EPS_M: f32 = 0.1;
/// Physics substeps per `step` call.
const SUBSTEPS: u16 = 4;
/// Top speed of the engine at the start, m/s.
const TOP_BASE_MPS: f32 = 40.0;
/// Always faster: the top speed grows this share per second.
const TOP_GROWTH_PER_S: f32 = 0.04;
/// Always faster stops growing the top speed here, m/s.
const TOP_CAP_MPS: f32 = 300.0;
/// The engine's pull fades out over this many m/s below the top speed, m/s.
const TOP_TAPER_MPS: f32 = 2.0;

/// Wheel mounts in the car frame, (x, z) in m: front left, front right, rear left, rear
/// right. The front pair steers, the rear pair drives.
const CORNERS: [(f32, f32); 4] = [
    (-TRACK_M / 2.0, WHEELBASE_M / 2.0),
    (TRACK_M / 2.0, WHEELBASE_M / 2.0),
    (-TRACK_M / 2.0, -WHEELBASE_M / 2.0),
    (TRACK_M / 2.0, -WHEELBASE_M / 2.0),
];

/// The car's axes in the world: right, up, forward.
type Axes = ([f32; 3], [f32; 3], [f32; 3]);

/// The driver's controls for one step. Each is clamped into its range on entry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct CarInput {
    /// Throttle, 0 to 1.
    pub throttle: f32,
    /// Brake, 0 to 1.
    pub brake: f32,
    /// Steer, -1 (full left) to 1 (full right).
    pub steer: f32,
    /// Handbrake: locks the rear wheels and cuts their side grip.
    pub handbrake: bool,
}

/// The car: a rigid body with four raycast wheels on the ground.
#[derive(Clone, Debug)]
pub(super) struct Car {
    /// Body centre, m.
    pos: [f32; 3],
    /// Velocity, m/s.
    vel: [f32; 3],
    /// Orientation as a unit quaternion [x, y, z, w].
    quat: [f32; 4],
    /// Angular velocity in the world frame, rad/s.
    ang_vel: [f32; 3],
    /// Moments of inertia about the right, up and forward axes, kg m^2.
    inertia: [f32; 3],
    /// Whether the engine's top speed grows over time.
    pub(super) always_faster: bool,
    /// Seconds of always-faster growth so far (0 while it is off).
    faster_s: f32,
    /// Load on each wheel at the last substep, N (0 while it is in the air).
    wheel_loads: [f32; 4],
}

impl Car {
    /// A car at rest at (`x`, `z`) facing `heading_rad` (0 faces +z), its springs at the
    /// static load of the ground under its centre.
    pub(super) fn new(
        x: f32,
        z: f32,
        heading_rad: f32,
        always_faster: bool,
        ground: &dyn Fn(f32, f32) -> f32,
    ) -> Self {
        let half = 0.5 * heading_rad;
        // Static compression of each spring with a quarter of the car on it.
        let static_compression = MASS_KG * GRAVITY_M_S2 / (4.0 * SPRING_N_PER_M);
        let height =
            ground(x, z) + MOUNT_BELOW_M + WHEEL_RADIUS_M + REST_LENGTH_M - static_compression;
        let (l, w, h) = (BODY_LENGTH_M, BODY_WIDTH_M, BODY_HEIGHT_M);
        Car {
            pos: [x, height, z],
            vel: [0.0; 3],
            quat: [0.0, half.sin(), 0.0, half.cos()],
            ang_vel: [0.0; 3],
            inertia: [
                MASS_KG * (h * h + l * l) / 12.0,
                MASS_KG * (w * w + l * l) / 12.0,
                MASS_KG * (w * w + h * h) / 12.0,
            ],
            always_faster,
            faster_s: 0.0,
            wheel_loads: [0.0; 4],
        }
    }

    /// Advance `dt` seconds under `input` over `ground`. A `dt` that is not a positive
    /// finite number does nothing.
    pub(super) fn step(&mut self, input: &CarInput, dt: f32, ground: &dyn Fn(f32, f32) -> f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let input = CarInput {
            throttle: input.throttle.clamp(0.0, 1.0),
            brake: input.brake.clamp(0.0, 1.0),
            steer: input.steer.clamp(-1.0, 1.0),
            handbrake: input.handbrake,
        };
        let h = dt / f32::from(SUBSTEPS);
        for _ in 0..SUBSTEPS {
            self.substep(&input, h, ground);
        }
        self.faster_s = if self.always_faster {
            self.faster_s + dt
        } else {
            0.0
        };
    }

    /// The body position (m) and its orientation quaternion [x, y, z, w].
    pub(super) fn pose(&self) -> ([f32; 3], [f32; 4]) {
        (self.pos, self.quat)
    }

    /// Put the body back at `pos` after it met a wall whose outward unit normal is `normal`:
    /// the velocity loses the part pointing into the wall and a fifth of the rest (the scrape).
    pub(super) fn bump(&mut self, pos: [f32; 3], normal: [f32; 3]) {
        self.pos = pos;
        let into = dot(self.vel, normal).min(0.0);
        self.vel = scale(sub(self.vel, scale(normal, into)), 0.8);
    }

    /// Speed over the ground, m/s.
    pub(super) fn speed_mps(&self) -> f32 {
        len(self.vel)
    }

    /// The engine's top speed now, m/s: 40 with always faster off; with it on, 4% more per
    /// second of growth, capped at 300.
    pub(super) fn top_speed_mps(&self) -> f32 {
        if self.always_faster {
            (TOP_BASE_MPS * (1.0 + TOP_GROWTH_PER_S).powf(self.faster_s)).min(TOP_CAP_MPS)
        } else {
            TOP_BASE_MPS
        }
    }

    /// One substep of `h` seconds: the wheel forces at each contact, then the body moves.
    fn substep(&mut self, input: &CarInput, h: f32, ground: &dyn Fn(f32, f32) -> f32) {
        let axes = basis(self.quat);
        let (right, up, fwd) = axes;
        let top = self.top_speed_mps();
        let engine =
            input.throttle * DRIVE_N * ((top - dot(self.vel, fwd)) / TOP_TAPER_MPS).clamp(0.0, 1.0);
        // Gravity, then air drag against the velocity.
        let mut force = [0.0, -MASS_KG * GRAVITY_M_S2, 0.0];
        let speed = len(self.vel);
        force = sub(
            force,
            scale(self.vel, 0.5 * AIR_KG_M3 * DRAG_AREA_M2 * speed),
        );
        let mut torque = [0.0; 3];

        for (i, &(lx, lz)) in CORNERS.iter().enumerate() {
            let front = i < 2;
            let offset = add(
                add(scale(right, lx), scale(up, -MOUNT_BELOW_M)),
                scale(fwd, lz),
            );
            let mount = add(self.pos, offset);
            let (x, z) = (mount[0], mount[2]);
            let ground_y = ground(x, z);
            // The ray drops straight down from the mount: the spring is shorter than its rest
            // length by `compression` (a wheel hanging past its rest length is in the air).
            let spring_len = mount[1] - ground_y - WHEEL_RADIUS_M;
            let compression = REST_LENGTH_M - spring_len;
            if compression <= 0.0 {
                self.wheel_loads[i] = 0.0;
                continue;
            }
            let normal = ground_normal(ground, x, z);
            let contact = [x, ground_y, z];
            let arm = sub(contact, self.pos);
            // The damper resists the rate at which the mount drops onto the ground.
            let mount_vel = add(self.vel, cross(self.ang_vel, offset));
            let compression_rate = -dot(mount_vel, normal);
            let load =
                (SPRING_N_PER_M * compression + DAMPER_N_S_PER_M * compression_rate).max(0.0);
            self.wheel_loads[i] = load;
            if load <= 0.0 {
                continue;
            }

            // The wheel's frame on the ground: the car's heading, turned by the steer on the
            // front pair, and its side.
            let fwd_ground = unit(sub(fwd, scale(normal, dot(fwd, normal))));
            let steer = if front {
                input.steer * MAX_STEER_RAD
            } else {
                0.0
            };
            let wheel_fwd = add(
                scale(fwd_ground, steer.cos()),
                scale(cross(normal, fwd_ground), steer.sin()),
            );
            let wheel_side = cross(normal, wheel_fwd);
            let v_contact = add(self.vel, cross(self.ang_vel, arm));
            let v_long = dot(v_contact, wheel_fwd);
            let v_lat = dot(v_contact, wheel_side);

            // Side force: the slip angle sets the grip, which opposes the slide.
            let rear_brake = input.handbrake && !front;
            let side_grip = if rear_brake { HANDBRAKE_GRIP } else { 1.0 };
            let friction = FRICTION_MU * load;
            let slip = v_lat.atan2(v_long.abs().max(SLIP_SPEED_FLOOR_MPS));
            let side = -friction * side_grip * grip_fraction(slip);

            // Along the wheel: drive, brake and rolling resistance, inside the friction that
            // the side force leaves over.
            let ramp = v_long / (v_long.abs() + RAMP_SPEED_MPS);
            let drive = if front { 0.0 } else { 0.5 * engine };
            let brake = input.brake * BRAKE_N + if rear_brake { HANDBRAKE_N } else { 0.0 };
            let along_req = drive - (brake + ROLLING_COEF * load) * ramp;
            let along_cap = (friction * friction - side * side).max(0.0).sqrt();
            let along = along_req.clamp(-along_cap, along_cap);

            let tyre = add(scale(wheel_fwd, along), scale(wheel_side, side));
            let f = add(scale(normal, load), tyre);
            force = add(force, f);
            torque = add(torque, cross(arm, f));
        }
        self.integrate(axes, force, torque, h);
    }

    /// Semi-implicit Euler: velocity, then spin, then position and attitude from the new
    /// rates.
    fn integrate(&mut self, axes: Axes, force: [f32; 3], torque: [f32; 3], h: f32) {
        self.vel = add(self.vel, scale(force, h / MASS_KG));
        let w_body = spin_step(
            to_body(axes, self.ang_vel),
            to_body(axes, torque),
            self.inertia,
            h,
        );
        self.ang_vel = from_body(axes, w_body);
        self.pos = add(self.pos, scale(self.vel, h));
        let spin = quat_mul(
            [self.ang_vel[0], self.ang_vel[1], self.ang_vel[2], 0.0],
            self.quat,
        );
        let q: [f32; 4] = std::array::from_fn(|k| self.quat[k] + 0.5 * h * spin[k]);
        self.quat = normalize4(q);
    }
}

/// Side grip as a share of the peak: linear up to the peak slip, then falling to
/// `SLIDE_GRIP` by twice the peak slip. Signed like the slip.
fn grip_fraction(slip_rad: f32) -> f32 {
    let a = slip_rad.abs();
    let share = if a <= PEAK_SLIP_RAD {
        a / PEAK_SLIP_RAD
    } else {
        let past = ((a - PEAK_SLIP_RAD) / PEAK_SLIP_RAD).min(1.0);
        1.0 - (1.0 - SLIDE_GRIP) * past
    };
    share.copysign(slip_rad)
}

/// The ground's unit normal at (x, z), from finite differences of `ground`.
fn ground_normal(ground: &dyn Fn(f32, f32) -> f32, x: f32, z: f32) -> [f32; 3] {
    let e = NORMAL_EPS_M;
    let dx = (ground(x + e, z) - ground(x - e, z)) / (2.0 * e);
    let dz = (ground(x, z + e) - ground(x, z - e)) / (2.0 * e);
    unit([-dx, 1.0, -dz])
}

/// The body's axes in the world from its quaternion [x, y, z, w].
fn basis(q: [f32; 4]) -> Axes {
    let [x, y, z, w] = q;
    let right = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
    ];
    let up = [
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
    ];
    let fwd = [
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    (right, up, fwd)
}

/// A world vector in the body frame.
fn to_body(axes: Axes, v: [f32; 3]) -> [f32; 3] {
    let (right, up, fwd) = axes;
    [dot(v, right), dot(v, up), dot(v, fwd)]
}

/// A body-frame vector in the world.
fn from_body(axes: Axes, v: [f32; 3]) -> [f32; 3] {
    let (right, up, fwd) = axes;
    add(add(scale(right, v[0]), scale(up, v[1])), scale(fwd, v[2]))
}

/// Body-frame spin after `h` seconds under torque `tau`, with the gyroscopic term.
fn spin_step(w: [f32; 3], tau: [f32; 3], inertia: [f32; 3], h: f32) -> [f32; 3] {
    let iw = [inertia[0] * w[0], inertia[1] * w[1], inertia[2] * w[2]];
    let gyro = cross(w, iw);
    [0, 1, 2].map(|k| w[k] + h * (tau[k] - gyro[k]) / inertia[k])
}

/// Hamilton product of two quaternions [x, y, z, w].
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn normalize4(q: [f32; 4]) -> [f32; 4] {
    let n = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    q.map(|c| c / n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground at height zero.
    fn flat(_x: f32, _z: f32) -> f32 {
        0.0
    }

    /// Step the car `seconds` at 60 Hz under a fixed input.
    fn run(car: &mut Car, input: &CarInput, seconds: f32, ground: &dyn Fn(f32, f32) -> f32) {
        let steps = (seconds * 60.0).round() as usize;
        for _ in 0..steps {
            car.step(input, 1.0 / 60.0, ground);
        }
    }

    /// The heading in radians: 0 faces +z, positive turns toward +x.
    fn heading(car: &Car) -> f32 {
        let (_, _, fwd) = basis(car.quat);
        fwd[0].atan2(fwd[2])
    }

    /// An angle folded into (-PI, PI].
    fn wrap(a: f32) -> f32 {
        (a + PI).rem_euclid(2.0 * PI) - PI
    }

    /// On flat ground the car settles level within 3 s, and its four wheels carry the same
    /// load to within 10 percent.
    #[test]
    fn a_flat_ground_settles_the_car_level() {
        let mut car = Car::new(0.0, 0.0, 0.0, false, &flat);
        car.pos[1] += 0.3;
        car.ang_vel = [0.0, 0.0, 0.3];
        run(&mut car, &CarInput::default(), 3.0, &flat);
        let (_, _, fwd) = basis(car.quat);
        let (right, _, _) = basis(car.quat);
        assert!(fwd[1].abs() < 0.01, "pitch settles, got {}", fwd[1]);
        assert!(right[1].abs() < 0.01, "roll settles, got {}", right[1]);
        let loads = car.wheel_loads;
        let mean = loads.iter().sum::<f32>() / 4.0;
        for load in loads {
            assert!(
                (load - mean).abs() <= 0.1 * mean,
                "each wheel carries the mean load, got {loads:?}"
            );
        }
    }

    /// Full throttle climbs a 5 percent slope and the car stays on the ground.
    #[test]
    fn full_throttle_climbs_a_gentle_slope() {
        let slope = |_x: f32, z: f32| 0.05 * z;
        let mut car = Car::new(0.0, 0.0, 0.0, false, &slope);
        let full = CarInput {
            throttle: 1.0,
            ..Default::default()
        };
        run(&mut car, &full, 10.0, &slope);
        let (pos, _) = car.pose();
        assert!(pos[2] > 20.0, "it drove up the slope, got z={}", pos[2]);
        let above = pos[1] - slope(pos[0], pos[2]);
        assert!(
            above > 0.5 && above < 2.0,
            "it stays on the ground, got {above}"
        );
    }

    /// Facing uphill on a 60 percent slope with no throttle, the car slides back down.
    #[test]
    fn a_steep_slope_rolls_the_car_down_with_no_throttle() {
        let steep = |_x: f32, z: f32| 0.6 * z;
        let mut car = Car::new(0.0, 0.0, 0.0, false, &steep);
        run(&mut car, &CarInput::default(), 4.0, &steep);
        let (pos, _) = car.pose();
        assert!(pos[2] < -5.0, "it rolled back down, got z={}", pos[2]);
    }

    /// Full lock at 15 m/s turns the heading by more than 90 degrees within 6 s.
    #[test]
    fn full_lock_at_speed_turns_the_heading_past_ninety_degrees() {
        let mut car = Car::new(0.0, 0.0, 0.0, false, &flat);
        car.vel = [0.0, 0.0, 15.0];
        let lock = CarInput {
            steer: 1.0,
            ..Default::default()
        };
        let mut last = heading(&car);
        let mut turned = 0.0;
        for _ in 0..360 {
            car.step(&lock, 1.0 / 60.0, &flat);
            let now = heading(&car);
            turned += wrap(now - last);
            last = now;
        }
        assert!(
            turned.abs() > PI / 2.0,
            "the heading turned past 90 degrees, got {} deg",
            turned.to_degrees()
        );
    }

    /// The top speed grows 4 percent a second and caps at 300 m/s; off it holds at 40.
    #[test]
    fn the_top_speed_grows_four_percent_a_second_and_caps() {
        let mut car = Car::new(0.0, 0.0, 0.0, true, &flat);
        car.faster_s = 10.0;
        assert!((car.top_speed_mps() - 40.0 * 1.04_f32.powf(10.0)).abs() < 1e-3);
        car.faster_s = 100.0;
        assert_eq!(car.top_speed_mps(), TOP_CAP_MPS);
        car.always_faster = false;
        assert_eq!(car.top_speed_mps(), TOP_BASE_MPS);
    }

    /// With always faster on, the car is faster after 10 s of full throttle than with it off.
    #[test]
    fn always_faster_beats_the_fixed_top_speed_after_ten_seconds() {
        let full = CarInput {
            throttle: 1.0,
            ..Default::default()
        };
        let mut off = Car::new(0.0, 0.0, 0.0, false, &flat);
        let mut on = Car::new(0.0, 0.0, 0.0, true, &flat);
        run(&mut off, &full, 10.0, &flat);
        run(&mut on, &full, 10.0, &flat);
        assert!(
            off.speed_mps() < TOP_BASE_MPS + 1.0,
            "the fixed top holds, got {}",
            off.speed_mps()
        );
        assert!(
            on.speed_mps() > off.speed_mps() + 5.0,
            "always faster beats it, got {} vs {}",
            on.speed_mps(),
            off.speed_mps()
        );
    }

    /// The same inputs over the same ground give bit-identical state, run twice.
    #[test]
    fn the_same_inputs_give_bit_identical_state() {
        let hills = |x: f32, z: f32| 0.8 * (x * 0.07).sin() + 0.03 * z;
        let bits = |c: &Car| {
            (
                c.pos.map(f32::to_bits),
                c.quat.map(f32::to_bits),
                c.vel.map(f32::to_bits),
                c.ang_vel.map(f32::to_bits),
            )
        };
        let mut a = Car::new(0.0, 0.0, 0.3, true, &hills);
        let mut b = Car::new(0.0, 0.0, 0.3, true, &hills);
        for i in 0..900 {
            let input = CarInput {
                throttle: (i % 120) as f32 / 120.0,
                brake: if i % 300 > 250 { 0.8 } else { 0.0 },
                steer: (i as f32 * 0.01).sin(),
                handbrake: i % 500 > 480,
            };
            a.step(&input, 1.0 / 60.0, &hills);
            b.step(&input, 1.0 / 60.0, &hills);
        }
        assert_eq!(bits(&a), bits(&b));
        assert!(a.speed_mps().is_finite() && a.speed_mps() > 0.0);
    }
}
