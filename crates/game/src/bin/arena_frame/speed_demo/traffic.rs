//! traffic: the cars on the avenues and streets (docs/SPRINT-34-hero-city.md, HC7).
//! Driving cars follow one lane each: they keep a centre-to-centre gap of 8 m to the car
//! ahead and stop before a crossing whose light is red or amber for their direction. A hit
//! over 6 m/s turns a car into a tumbling box that bounces on the road, slides, spins down
//! and comes to rest. Presentation only. Units: meters, seconds, radians per second. The
//! road surface is flat at `ROAD_Y` (0 m), so a driving car's box centre rides at 0.75 m.

use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{quat_mul, splitmix64};

use super::num::{
    add, axis_quat, cross, dot, f32_of, floor_idx, len, rotate, scale, sub, u32_of, unit_f32,
    unit_or,
};

/// Half extents of a car's box (m): 1.9 m wide, 1.5 m tall, 4.5 m long (local z is forward).
const HALF: [f32; 3] = [0.95, 0.75, 2.25];
/// Height of the road surface (m). Driving cars ride with their box centre at `HALF[1]` above it.
const ROAD_Y: f32 = 0.0;
/// Top speed of a driving car (m/s).
const CRUISE_MS: f32 = 12.0;
/// Speed a car starts with when the traffic is built (m/s).
const START_SPEED_MS: f32 = 6.0;
/// Pull-away acceleration (m/s^2).
const ACCEL: f32 = 2.5;
/// Hard braking (m/s^2). The follow limit is built on it, so a follower can always brake as hard as its leader.
const BRAKE: f32 = 6.0;
/// Gentler deceleration a car plans its stop with (m/s^2); below BRAKE, so the plan has slack.
const STOP_DECEL: f32 = 5.0;
/// Centre-to-centre gap kept behind the car ahead, and the spacing at which cars are spawned (m).
const GAP_M: f32 = 8.0;
/// Extra room kept on top of GAP_M in the follow limit (m), so discrete steps stay above GAP_M.
const GAP_MARGIN_M: f32 = 0.5;
/// Length of the light cycle (s): green, amber and red for each axis in turn.
const CYCLE_S: f32 = 30.0;
/// Green time of each axis (s).
const GREEN_S: f32 = 13.0;
/// Amber time of each axis (s).
const AMBER_S: f32 = 2.0;
/// Radius of a crossing's box (m): a car whose centre is inside it never stops for its light.
const CROSS_R_M: f32 = 12.0;
/// Distance before a crossing at which a car starts to obey its light (m).
const SEE_M: f32 = 30.0;
/// Centre distance from the crossing at which a stopped car waits (m): past the crossing's radius by the room a braking car overshoots; its front bumper is 2.25 m further out.
const STOP_M: f32 = 14.0;
/// A car this slow always stops for a red: it stops in under a metre, so it never counts as one that cannot (m/s).
const CREEP_MS: f32 = 3.0;
/// Hit impulses at or below this speed do nothing (m/s).
const HIT_SPEED_MS: f32 = 6.0;
/// Bounce of the vertical velocity on contact (fraction kept).
const RESTITUTION: f32 = 0.3;
/// Coulomb friction coefficient on contact.
const FRICTION: f32 = 0.6;
/// Gravity (m/s^2).
const GRAVITY: f32 = 9.81;
/// A corner this close to the road (m) counts as touching it.
const CONTACT_M: f32 = 0.02;
/// Spin decay on contact (per second).
const SPIN_DAMP_CONTACT: f32 = 2.0;
/// Spin decay in the air (per second).
const SPIN_DAMP_AIR: f32 = 0.2;
/// Speed below which a tumbling car may rest (m/s).
const REST_SPEED_MS: f32 = 0.3;
/// Spin below which a tumbling car may rest (rad/s).
const REST_SPIN_RAD_S: f32 = 0.3;
/// How long a tumbling car must stay still on the road before it rests (s).
const REST_HOLD_S: f32 = 0.5;
/// Spin cap of a tumbling car (rad/s).
const MAX_SPIN_RAD_S: f32 = 15.0;
/// Moment of inertia per unit mass of the car box (m^2): (w^2 + h^2 + l^2) / 12.
const INERTIA_PER_MASS: f32 = 2.18;
/// Centre-to-centre spacing below which two cars are never put in one lane (m).
const MIN_GAP_M: f32 = 8.0;

/// A straight lane centre line on the ground, in x/z meters. Driving cars go from `start` to `end` and wrap to `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Lane {
    /// The lane's first point (x, z) in meters.
    pub start: [f32; 2],
    /// The lane's last point (x, z) in meters.
    pub end: [f32; 2],
}

/// A traffic-light crossing at a point (x, z) in meters, shared by the lanes that pass within `CROSS_R_M` of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Crossing {
    /// The crossing's centre (x, z) in meters.
    pub at: [f32; 2],
    /// Seconds added to the shared clock before the light's phase is read.
    pub phase_offset_s: f32,
}

/// One car: driving along a lane, tumbling after a hit, or resting on the road.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum CarState {
    /// Driving: the lane index, the distance along it from its start (m) and the speed (m/s).
    Driving {
        /// Index into the traffic's lanes.
        lane: usize,
        /// Distance along the lane from its start (m), in `0..length`.
        s_m: f32,
        /// Forward speed along the lane (m/s).
        speed: f32,
    },
    /// Tumbling: box centre (m), velocity (m/s), orientation [x, y, z, w], spin (rad/s) and seconds still so far.
    Tumbling {
        /// Box centre (m).
        pos: [f32; 3],
        /// Velocity of the centre (m/s).
        vel: [f32; 3],
        /// Orientation as a unit quaternion [x, y, z, w].
        quat: [f32; 4],
        /// Angular velocity in the world (rad/s).
        spin: [f32; 3],
        /// Seconds the car has been still on the road.
        rest_s: f32,
    },
    /// Resting: the box centre (m) and orientation, frozen until the next hit.
    Resting {
        /// Box centre (m).
        pos: [f32; 3],
        /// Orientation as a unit quaternion [x, y, z, w].
        quat: [f32; 4],
    },
}

/// All the cars of one city, the lanes they drive, the crossings they obey and the clock (s).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Traffic {
    /// Every car, in lane order.
    pub cars: Vec<CarState>,
    /// The lanes the driving cars follow.
    lanes: Vec<Lane>,
    /// The crossings with traffic lights.
    crossings: Vec<Crossing>,
    /// The traffic clock (s), which drives the lights.
    time_s: f32,
}

/// A seeded word stream: splitmix64 over a golden-gamma counter.
struct Seeded {
    /// The counter, advanced by the golden gamma before each draw.
    state: u64,
}

impl Seeded {
    /// The next draw in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        unit_f32(splitmix64(self.state))
    }
}

/// A driving car's state as the snapshot the followers read in one step.
#[derive(Clone, Copy)]
struct Driver {
    /// Index of the car in the traffic.
    car: usize,
    /// Index of its lane.
    lane: usize,
    /// Distance along the lane (m).
    s_m: f32,
    /// Speed (m/s).
    speed: f32,
}

/// Unit direction of a lane on the ground (y = 0); zero for a lane of no length.
fn lane_dir(lane: &Lane) -> [f32; 3] {
    unit_or(
        [
            lane.end[0] - lane.start[0],
            0.0,
            lane.end[1] - lane.start[1],
        ],
        [0.0; 3],
    )
}

/// Length of a lane's centre line (m).
fn lane_len(lane: &Lane) -> f32 {
    len([
        lane.end[0] - lane.start[0],
        0.0,
        lane.end[1] - lane.start[1],
    ])
}

/// The box centre and the lane direction at distance `s` along lane `lane`, or none for a lane that is not there.
fn frame(lanes: &[Lane], lane: usize, s: f32) -> Option<([f32; 3], [f32; 3])> {
    let l = lanes.get(lane)?;
    let start = [l.start[0], ROAD_Y + HALF[1], l.start[1]];
    let dir = lane_dir(l);
    Some((add(start, scale(dir, s)), dir))
}

/// The orientation that turns the car's long axis (local z) onto `dir`, a turn about y.
fn yaw_quat(dir: [f32; 3]) -> [f32; 4] {
    axis_quat([0.0, 1.0, 0.0], dir[0].atan2(dir[2]))
}

/// `q` at unit length; the identity when `q` is degenerate.
fn quat_norm(q: [f32; 4]) -> [f32; 4] {
    let n = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if n > 1e-9 {
        q.map(|c| c / n)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// `q` turned by the world angular velocity `spin` (rad/s) for `dt` seconds, kept unit.
fn advance_quat(q: [f32; 4], spin: [f32; 3], dt: f32) -> [f32; 4] {
    let dq = quat_mul([spin[0], spin[1], spin[2], 0.0], q);
    quat_norm(std::array::from_fn(|k| q[k] + 0.5 * dt * dq[k]))
}

/// The local offset (m) of box corner `k` (bits pick the signs of x, y and z).
fn corner_offset(k: usize) -> [f32; 3] {
    let pick = |bit: usize, h: f32| if k & bit != 0 { h } else { -h };
    [pick(1, HALF[0]), pick(2, HALF[1]), pick(4, HALF[2])]
}

/// The clearance (m) of the box's lowest corner above the ground: the least of the eight corners' heights over the ground beneath each.
fn min_clearance(pos: [f32; 3], quat: [f32; 4], ground: &dyn Fn(f32, f32) -> f32) -> f32 {
    (0..8)
        .map(|k| {
            let c = add(pos, rotate(quat, corner_offset(k)));
            c[1] - ground(c[0], c[2])
        })
        .fold(f32::INFINITY, f32::min)
}

/// A tumbling car's velocity after a touch of the road: the vertical bounce, and friction that slows the sliding (at most FRICTION * g * dt per step).
fn grounded_velocity(v: [f32; 3], dt: f32) -> [f32; 3] {
    let vy = if v[1] < 0.0 {
        -RESTITUTION * v[1]
    } else {
        v[1]
    };
    let h = (v[0] * v[0] + v[2] * v[2]).sqrt();
    let drop = FRICTION * GRAVITY * dt;
    let k = if h > 1e-6 {
        (h - drop).max(0.0) / h
    } else {
        0.0
    };
    [v[0] * k, vy, v[2] * k]
}

/// One step of a tumbling car: gravity, spin, the lowest corner held on the ground, and rest once still on the road.
fn tumble(
    pos: [f32; 3],
    vel: [f32; 3],
    quat: [f32; 4],
    spin: [f32; 3],
    rest_s: f32,
    dt: f32,
    ground: &dyn Fn(f32, f32) -> f32,
) -> CarState {
    let vel = add(vel, [0.0, -GRAVITY * dt, 0.0]);
    let pos = add(pos, scale(vel, dt));
    let quat = advance_quat(quat, spin, dt);
    let clear = min_clearance(pos, quat, ground);
    let touching = clear < CONTACT_M;
    let pos = if clear < 0.0 {
        add(pos, [0.0, -clear, 0.0])
    } else {
        pos
    };
    let vel = if touching {
        grounded_velocity(vel, dt)
    } else {
        vel
    };
    let damp = if touching {
        SPIN_DAMP_CONTACT
    } else {
        SPIN_DAMP_AIR
    };
    let spin = scale(spin, (-damp * dt).exp());
    let still = touching && len(vel) < REST_SPEED_MS && len(spin) < REST_SPIN_RAD_S;
    let rest_s = if still { rest_s + dt } else { 0.0 };
    if rest_s >= REST_HOLD_S {
        CarState::Resting { pos, quat }
    } else {
        CarState::Tumbling {
            pos,
            vel,
            quat,
            spin,
            rest_s,
        }
    }
}

/// Whether a light shows green at `time_s` on the given axis: z lanes (north-south) run first, x lanes after the amber.
fn is_green(time_s: f32, z_axis: bool) -> bool {
    let t = time_s.rem_euclid(CYCLE_S);
    if z_axis {
        t < GREEN_S
    } else {
        (GREEN_S + AMBER_S..GREEN_S + AMBER_S + GREEN_S).contains(&t)
    }
}

/// The speed limit (m/s) from the nearest car ahead in the same lane: the follower can brake at BRAKE as hard as the leader and still keep the gap.
fn follow_limit(car: usize, lane: usize, s: f32, length: f32, board: &[Driver]) -> f32 {
    let mut gap = f32::INFINITY;
    let mut lead_speed = 0.0;
    for d in board.iter().filter(|d| d.lane == lane && d.car != car) {
        let ahead = (d.s_m - s).rem_euclid(length);
        if ahead > 1e-3 && ahead < gap {
            gap = ahead;
            lead_speed = d.speed;
        }
    }
    if gap.is_infinite() {
        return f32::INFINITY;
    }
    let room = (gap - GAP_M - GAP_MARGIN_M).max(0.0);
    (lead_speed * lead_speed + 2.0 * BRAKE * room).sqrt()
}

impl Traffic {
    /// Traffic on `lanes` with `cars_per_lane` cars each (fewer on a short lane), spaced evenly with seeded jitter and never closer than 8 m.
    pub(super) fn new(
        lanes: Vec<Lane>,
        crossings: Vec<Crossing>,
        cars_per_lane: usize,
        seed: u64,
    ) -> Self {
        let mut rng = Seeded { state: seed };
        let mut cars = Vec::new();
        for (lane, l) in lanes.iter().enumerate() {
            let length = lane_len(l);
            let n = cars_per_lane.min(floor_idx(length / MIN_GAP_M));
            if n == 0 {
                continue;
            }
            let spacing = length / f32_of(n);
            let slack = ((spacing - MIN_GAP_M) * 0.5).max(0.0);
            for j in 0..n {
                let jitter = (rng.unit() * 2.0 - 1.0) * slack;
                let s_m = (f32_of(j) * spacing + 0.5 * spacing + jitter).rem_euclid(length);
                cars.push(CarState::Driving {
                    lane,
                    s_m,
                    speed: START_SPEED_MS,
                });
            }
        }
        Self {
            cars,
            lanes,
            crossings,
            time_s: 0.0,
        }
    }

    /// Advances every car by `dt` seconds; `ground(x, z)` is the road height (m) under a point.
    pub(super) fn step(&mut self, dt: f32, ground: &dyn Fn(f32, f32) -> f32) {
        let board = self.board();
        let next: Vec<CarState> = self
            .cars
            .iter()
            .enumerate()
            .map(|(i, car)| self.advance(i, *car, dt, ground, &board))
            .collect();
        self.cars = next;
        self.time_s += dt;
    }

    /// Hits every driving car whose centre is within `radius_m` of `at` (m), if the impulse is over 6 m/s: the car tumbles with its drive velocity plus `impulse_vel`, and the impulse's lever arm sets its spin. Returns how many cars it hit.
    pub(super) fn hit(&mut self, at: [f32; 3], radius_m: f32, impulse_vel: [f32; 3]) -> u32 {
        if len(impulse_vel) <= HIT_SPEED_MS {
            return 0;
        }
        let lanes = &self.lanes;
        let mut count = 0usize;
        for car in self.cars.iter_mut() {
            let (lane, s_m, speed) = match *car {
                CarState::Driving { lane, s_m, speed } => (lane, s_m, speed),
                _ => continue,
            };
            let Some((centre, dir)) = frame(lanes, lane, s_m) else {
                continue;
            };
            if len(sub(centre, at)) > radius_m {
                continue;
            }
            let vel = add(scale(dir, speed), impulse_vel);
            let torque = cross(sub(at, centre), impulse_vel);
            let spin = clamp_len(scale(torque, 1.0 / INERTIA_PER_MASS), MAX_SPIN_RAD_S);
            *car = CarState::Tumbling {
                pos: centre,
                vel,
                quat: yaw_quat(dir),
                spin,
                rest_s: 0.0,
            };
            count += 1;
        }
        u32_of(count)
    }

    /// Each car's box centre (m) and orientation [x, y, z, w]; driving cars face along their lane.
    pub(super) fn poses(&self) -> Vec<([f32; 3], [f32; 4])> {
        self.cars
            .iter()
            .filter_map(|car| match *car {
                CarState::Driving { lane, s_m, .. } => {
                    frame(&self.lanes, lane, s_m).map(|(c, dir)| (c, yaw_quat(dir)))
                }
                CarState::Tumbling { pos, quat, .. } | CarState::Resting { pos, quat } => {
                    Some((pos, quat))
                }
            })
            .collect()
    }

    /// The driving cars as they are at the start of a step, for the followers to read.
    fn board(&self) -> Vec<Driver> {
        self.cars
            .iter()
            .enumerate()
            .filter_map(|(car, c)| match *c {
                CarState::Driving { lane, s_m, speed } => Some(Driver {
                    car,
                    lane,
                    s_m,
                    speed,
                }),
                _ => None,
            })
            .collect()
    }

    /// One step of car `car` (in state `state`).
    fn advance(
        &self,
        car: usize,
        state: CarState,
        dt: f32,
        ground: &dyn Fn(f32, f32) -> f32,
        board: &[Driver],
    ) -> CarState {
        match state {
            CarState::Driving { lane, s_m, speed } => self.drive(car, lane, s_m, speed, dt, board),
            CarState::Tumbling {
                pos,
                vel,
                quat,
                spin,
                rest_s,
            } => tumble(pos, vel, quat, spin, rest_s, dt, ground),
            resting @ CarState::Resting { .. } => resting,
        }
    }

    /// One step of a driving car: it heads for the lowest of its cruise, follow and stop limits, accelerating or braking toward it.
    fn drive(
        &self,
        car: usize,
        lane: usize,
        s: f32,
        v: f32,
        dt: f32,
        board: &[Driver],
    ) -> CarState {
        let Some(l) = self.lanes.get(lane) else {
            return CarState::Driving {
                lane,
                s_m: s,
                speed: v,
            };
        };
        let length = lane_len(l);
        if length <= 1e-6 {
            return CarState::Driving {
                lane,
                s_m: s,
                speed: v,
            };
        }
        let target = CRUISE_MS
            .min(follow_limit(car, lane, s, length, board))
            .min(self.stop_limit(l, s, v));
        let speed = if v < target {
            (v + ACCEL * dt).min(target)
        } else {
            (v - BRAKE * dt).max(target)
        };
        let s_m = (s + 0.5 * (v + speed) * dt).rem_euclid(length);
        CarState::Driving { lane, s_m, speed }
    }

    /// The speed limit (m/s) from the crossings ahead on lane `l`: a car that can still stop short of the stop line at the planned deceleration does; one that cannot goes on through.
    fn stop_limit(&self, l: &Lane, s: f32, v: f32) -> f32 {
        let dir = lane_dir(l);
        let z_axis = dir[2].abs() > dir[0].abs();
        let start = [l.start[0], 0.0, l.start[1]];
        let mut limit = f32::INFINITY;
        for c in &self.crossings {
            let rel = sub([c.at[0], 0.0, c.at[1]], start);
            let along = dot(rel, dir) - s;
            let lateral = cross(rel, dir)[1].abs();
            let approaching = along > 0.0 && along <= SEE_M && lateral <= CROSS_R_M;
            let inside = along.hypot(lateral) <= CROSS_R_M;
            if !approaching || inside || is_green(self.time_s + c.phase_offset_s, z_axis) {
                continue;
            }
            let room = along - STOP_M;
            if v <= CREEP_MS || v * v <= 2.0 * BRAKE * room + 1e-3 {
                limit = limit.min((2.0 * STOP_DECEL * room.max(0.0)).sqrt());
            }
        }
        limit
    }
}

/// `v` at most `max` long (direction kept).
fn clamp_len(v: [f32; 3], max: f32) -> [f32; 3] {
    let l = len(v);
    if l > max {
        scale(v, max / l)
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One 60 Hz frame in seconds.
    const DT: f32 = 1.0 / 60.0;

    /// Flat road at height 0.
    fn flat(_x: f32, _z: f32) -> f32 {
        0.0
    }

    /// A gentle slope rising toward +z.
    fn slope(_x: f32, z: f32) -> f32 {
        0.03 * z
    }

    /// Three lanes: a north-going avenue, a south-going one 7.5 m east, and an east-west street, with one crossing at their meeting point.
    fn city_lanes() -> (Vec<Lane>, Vec<Crossing>) {
        let lanes = vec![
            Lane {
                start: [0.0, 0.0],
                end: [0.0, 300.0],
            },
            Lane {
                start: [7.5, 300.0],
                end: [7.5, 0.0],
            },
            Lane {
                start: [-150.0, 150.0],
                end: [150.0, 150.0],
            },
        ];
        let crossings = vec![Crossing {
            at: [0.0, 150.0],
            phase_offset_s: 0.0,
        }];
        (lanes, crossings)
    }

    /// The smallest centre-to-centre distance along a lane between any two driving cars in it (m).
    fn worst_gap(t: &Traffic) -> f32 {
        let mut worst = f32::INFINITY;
        for (i, a) in t.cars.iter().enumerate() {
            for b in t.cars.iter().skip(i + 1) {
                if let (
                    CarState::Driving {
                        lane: la, s_m: sa, ..
                    },
                    CarState::Driving {
                        lane: lb, s_m: sb, ..
                    },
                ) = (*a, *b)
                {
                    if la != lb {
                        continue;
                    }
                    let length = lane_len(&t.lanes[la]);
                    let d = (sa - sb).rem_euclid(length);
                    worst = worst.min(d.min(length - d));
                }
            }
        }
        worst
    }

    /// The lowest box-corner clearance over every tumbling car in `t` (m); infinity when none tumble.
    fn worst_tumble_clearance(t: &Traffic, ground: &dyn Fn(f32, f32) -> f32) -> f32 {
        t.cars
            .iter()
            .filter_map(|c| match *c {
                CarState::Tumbling { pos, quat, .. } => Some(min_clearance(pos, quat, ground)),
                _ => None,
            })
            .fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn cars_in_one_lane_never_come_within_7_5_m() {
        let (lanes, crossings) = city_lanes();
        let mut t = Traffic::new(lanes, crossings, 10, 42);
        let mut worst = worst_gap(&t);
        for _ in 0..7200 {
            t.step(DT, &flat);
            worst = worst.min(worst_gap(&t));
        }
        assert!(worst >= 7.5, "closest gap in a lane was {worst} m");
    }

    #[test]
    fn a_car_stops_before_a_red_crossing_and_goes_on_green() {
        let lanes = vec![Lane {
            start: [0.0, 0.0],
            end: [0.0, 400.0],
        }];
        let crossings = vec![Crossing {
            at: [0.0, 200.0],
            phase_offset_s: 0.0,
        }];
        let mut t = Traffic {
            cars: vec![CarState::Driving {
                lane: 0,
                s_m: 0.0,
                speed: CRUISE_MS,
            }],
            lanes,
            crossings,
            time_s: 0.0,
        };
        let mut speed_at_25 = f32::NAN;
        let mut closest_on_red = f32::INFINITY;
        while t.time_s < 45.0 {
            t.step(DT, &flat);
            let CarState::Driving { s_m, speed, .. } = t.cars[0] else {
                panic!("the car left the lane");
            };
            if t.time_s >= 15.0 && t.time_s < 30.0 {
                closest_on_red = closest_on_red.min(200.0 - s_m);
            }
            if (t.time_s - 25.0).abs() < DT * 0.5 {
                speed_at_25 = speed;
            }
        }
        assert!(
            speed_at_25 < 0.05,
            "still moving at 25 s: {speed_at_25} m/s"
        );
        assert!(
            closest_on_red >= CROSS_R_M,
            "the car came {closest_on_red} m from the crossing on red"
        );
        let CarState::Driving { s_m, .. } = t.cars[0] else {
            panic!("the car left the lane");
        };
        assert!(
            s_m > 200.0 + CROSS_R_M,
            "the car did not go on at green: s = {s_m}"
        );
    }

    #[test]
    fn a_hit_car_comes_to_rest_on_the_road_within_8_s() {
        let (lanes, _) = city_lanes();
        let mut t = Traffic {
            cars: vec![CarState::Driving {
                lane: 0,
                s_m: 100.0,
                speed: CRUISE_MS,
            }],
            lanes,
            crossings: Vec::new(),
            time_s: 0.0,
        };
        let (centre, _) = t.poses()[0];
        let hit = t.hit(
            [centre[0] + 0.9, centre[1], centre[2]],
            3.0,
            [6.0, 3.0, 6.0],
        );
        assert_eq!(hit, 1);
        let mut rest_at = None;
        for _ in 0..480 {
            t.step(DT, &flat);
            assert!(
                worst_tumble_clearance(&t, &flat) >= -0.05,
                "a corner sank under the road"
            );
            if rest_at.is_none() && matches!(t.cars[0], CarState::Resting { .. }) {
                rest_at = Some(t.time_s);
            }
        }
        assert!(
            rest_at.is_some_and(|s| s <= 8.0),
            "the hit car rested at {rest_at:?}"
        );
    }

    #[test]
    fn a_tumbling_car_never_sinks_under_a_slope() {
        let lanes = vec![Lane {
            start: [0.0, -150.0],
            end: [0.0, 150.0],
        }];
        let mut t = Traffic {
            cars: vec![CarState::Driving {
                lane: 0,
                s_m: 150.0,
                speed: CRUISE_MS,
            }],
            lanes,
            crossings: Vec::new(),
            time_s: 0.0,
        };
        let (centre, _) = t.poses()[0];
        t.hit(
            [centre[0] + 0.9, centre[1], centre[2]],
            3.0,
            [6.0, 4.0, 8.0],
        );
        for _ in 0..600 {
            t.step(DT, &slope);
            let worst = worst_tumble_clearance(&t, &slope);
            assert!(worst >= -0.05, "a corner sank {worst} m under the slope");
        }
    }

    #[test]
    fn a_hit_below_6_ms_does_nothing() {
        let (lanes, crossings) = city_lanes();
        let mut t = Traffic::new(lanes, crossings, 4, 3);
        let before = t.cars.clone();
        assert_eq!(t.hit([0.0, 0.75, 150.0], 50.0, [3.0, 0.0, 4.0]), 0);
        assert_eq!(t.hit([0.0, 0.75, 150.0], 50.0, [6.0, 0.0, 0.0]), 0);
        assert_eq!(t.cars, before);
    }

    #[test]
    fn the_same_seed_gives_the_same_poses_bit_for_bit() {
        let run = |seed: u64| {
            let (lanes, crossings) = city_lanes();
            let mut t = Traffic::new(lanes, crossings, 8, seed);
            for k in 0..3600 {
                t.step(DT, &flat);
                if k == 1800 {
                    t.hit([0.0, 0.75, 150.0], 20.0, [8.0, 0.0, 0.0]);
                }
            }
            t.poses()
                .iter()
                .map(|(p, q)| (p.map(f32::to_bits), q.map(f32::to_bits)))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(9), run(9));
        assert_ne!(run(9), run(10));
    }

    #[test]
    fn spawned_cars_start_at_least_8_m_apart() {
        let (lanes, crossings) = city_lanes();
        let t = Traffic::new(lanes, crossings, 10, 5);
        assert!(worst_gap(&t) >= MIN_GAP_M);
    }
}
