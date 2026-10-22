//! flight (SD2, docs/SPRINT-33-speed-demo.md): the flying body and "always faster".
//!
//! The body flies along its look direction: yaw turns it about the up axis, pitch tilts it
//! (clamped to +-85 deg). Its speed relaxes toward a target. The target is the cruise speed,
//! which starts at 30 m/s and, with always-faster on, grows 4% per second up to 900 m/s; with
//! it off the cruise holds where it is. Forward input moves the target by half the cruise,
//! either way; boost doubles it while held. Strafe and climb are side thrusts. Height never
//! drops below the ground plus 2 m, so a dive skims the terrain. The speed feel turns the
//! speed into field of view, camera shake and a streak rate. Presentation only,
//! deterministic: the caller passes each fixed step in, no clock, no RNG. Units: meters,
//! seconds, radians.

use super::num::{add, scale};
use std::f32::consts::{PI, TAU};

/// Cruise speed at the start and with always-faster off, m/s.
const BASE_SPEED_MPS: f32 = 30.0;
/// Top speed: the cruise and the body are capped here, m/s.
const MAX_SPEED_MPS: f32 = 900.0;
/// Cruise growth with always-faster on, per second (exponential: x1.04 each second).
const CRUISE_GROWTH_PER_S: f32 = 0.04;
/// The speed relaxes toward its target at this rate, 1/s (about half a second to settle).
const SPEED_RELAX_PER_S: f32 = 2.0;
/// Forward input moves the target speed by this fraction of the cruise, each way.
const THROTTLE_RANGE: f32 = 0.5;
/// Boost multiplies the target speed while held (dimensionless).
const BOOST_MULT: f32 = 2.0;
/// Strafe thrust at full input, m/s.
const STRAFE_MPS: f32 = 12.0;
/// Climb thrust at full input, m/s.
const CLIMB_MPS: f32 = 12.0;
/// Pitch limit: 85 degrees, radians.
const MAX_PITCH_RAD: f32 = 85.0 * PI / 180.0;
/// Height kept above the ground, m.
const GROUND_CLEARANCE_M: f32 = 2.0;

/// Field of view at rest, degrees.
const FOV_REST_DEG: f32 = 75.0;
/// Field of view at the top speed, degrees.
const FOV_TOP_DEG: f32 = 110.0;
/// How strongly the field of view bends toward its top with speed (dimensionless; near 0
/// the curve is a straight line).
const FOV_BEND: f32 = 4.0;
/// Camera shake at the top speed, m (a few decimetres); linear in speed, so 1 cm at 30 m/s.
const SHAKE_TOP_M: f32 = 0.3;
/// Speed streaks drawn per second at the top speed, 1/s; linear in speed.
const STREAKS_TOP_PER_S: f32 = 300.0;

/// One step's controls. Axes are -1..1 (clamped), turns are radians applied this step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct FlightInput {
    /// +1 pushes the target speed up (+50%), -1 brakes it (-50%).
    pub forward: f32,
    /// +1 strafes right.
    pub strafe: f32,
    /// +1 climbs, -1 descends.
    pub up: f32,
    /// Heading change this step, radians (+ turns right).
    pub yaw_delta: f32,
    /// Pitch change this step, radians (+ looks up).
    pub pitch_delta: f32,
    /// Doubles the target speed while held.
    pub boost: bool,
}

/// The flying body. `always_faster` is the F key; `speed` is along the look direction, m/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Flight {
    pub pos: [f32; 3],
    /// Heading about the up axis, radians (0 looks down -z).
    pub yaw: f32,
    /// Look angle above the horizon, radians, within +-85 deg.
    pub pitch: f32,
    pub speed: f32,
    pub always_faster: bool,
    /// The cruise speed the body relaxes toward, m/s.
    cruise_mps: f32,
}

impl Flight {
    /// A body at `pos`, looking down -z, at the base speed with always-faster on.
    pub(super) fn new(pos: [f32; 3]) -> Self {
        Flight {
            pos,
            yaw: 0.0,
            pitch: 0.0,
            speed: BASE_SPEED_MPS,
            always_faster: true,
            cruise_mps: BASE_SPEED_MPS,
        }
    }

    /// The unit look direction from yaw and pitch.
    pub(super) fn look(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, sp, -cy * cp]
    }

    /// Advance `dt` seconds. `ground(x, z)` is the terrain height; the body stays 2 m above it.
    pub(super) fn step(&mut self, input: &FlightInput, dt: f32, ground: &dyn Fn(f32, f32) -> f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let forward = axis(input.forward);
        let strafe = axis(input.strafe);
        let up = axis(input.up);
        self.yaw = wrap_angle(self.yaw + finite(input.yaw_delta));
        self.pitch = (self.pitch + finite(input.pitch_delta)).clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);
        if self.always_faster {
            let grown = self.cruise_mps * (1.0 + CRUISE_GROWTH_PER_S).powf(dt);
            self.cruise_mps = grown.min(MAX_SPEED_MPS);
        }
        let boost = if input.boost { BOOST_MULT } else { 1.0 };
        let target =
            (self.cruise_mps * boost * (1.0 + THROTTLE_RANGE * forward)).clamp(0.0, MAX_SPEED_MPS);
        let settle = 1.0 - (-SPEED_RELAX_PER_S * dt).exp();
        self.speed = (self.speed + (target - self.speed) * settle).clamp(0.0, MAX_SPEED_MPS);

        let along = scale(self.look(), self.speed);
        let side = scale(right(self.yaw), strafe * STRAFE_MPS);
        let lift = [0.0, up * CLIMB_MPS, 0.0];
        let vel = add(along, add(side, lift));
        self.pos = add(self.pos, scale(vel, dt));
        self.pos[1] = self.pos[1].max(ground(self.pos[0], self.pos[2]) + GROUND_CLEARANCE_M);
    }
}

/// What the camera and the streaks do at one speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SpeedFeel {
    /// Horizontal field of view, degrees (75 at rest, 110 at the top speed).
    pub fov_deg: f32,
    /// Camera shake amplitude, m.
    pub shake_m: f32,
    /// Speed streaks spawned per second.
    pub streaks_per_s: f32,
}

/// The speed feel at `speed_mps`; speed is clamped to 0..=900 m/s first.
pub(super) fn speed_feel(speed_mps: f32) -> SpeedFeel {
    let t = if speed_mps.is_nan() {
        0.0
    } else {
        (speed_mps / MAX_SPEED_MPS).clamp(0.0, 1.0)
    };
    SpeedFeel {
        fov_deg: FOV_REST_DEG + (FOV_TOP_DEG - FOV_REST_DEG) * fov_fraction(t),
        shake_m: SHAKE_TOP_M * t,
        streaks_per_s: STREAKS_TOP_PER_S * t,
    }
}

/// Normalised exponential ease: 0 at rest, 1 at the top, rising faster early on.
fn fov_fraction(t: f32) -> f32 {
    let norm = 1.0 - (-FOV_BEND).exp();
    ((1.0 - (-FOV_BEND * t).exp()) / norm).clamp(0.0, 1.0)
}

/// Right-hand unit vector for a heading (forward x up at yaw 0 is +x).
fn right(yaw: f32) -> [f32; 3] {
    let (s, c) = yaw.sin_cos();
    [c, 0.0, s]
}

/// Keep an angle in [-pi, pi).
fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

/// A control axis: NaN reads as 0, anything else is clamped to -1..1.
fn axis(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-1.0, 1.0)
    }
}

/// A finite value, or 0 for NaN and infinities.
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

    const DT: f32 = 1.0 / 60.0;

    fn flat(_x: f32, _z: f32) -> f32 {
        0.0
    }

    /// A gaussian hill: 60 m high at the origin, about 150 m across.
    fn hill(x: f32, z: f32) -> f32 {
        60.0 * (-(x * x + z * z) / (2.0 * 150.0 * 150.0)).exp()
    }

    fn idle() -> FlightInput {
        FlightInput::default()
    }

    fn fly(f: &mut Flight, input: FlightInput, secs: f32, ground: &dyn Fn(f32, f32) -> f32) {
        for _ in 0..(secs / DT).round() as usize {
            f.step(&input, DT, ground);
        }
    }

    /// Always-faster on grows the speed past the 30 m/s base; off holds exactly 30.
    #[test]
    fn always_faster_outruns_a_held_cruise_after_10_s() {
        let mut on = Flight::new([0.0, 100.0, 0.0]);
        let mut off = Flight::new([0.0, 100.0, 0.0]);
        off.always_faster = false;
        fly(&mut on, idle(), 10.0, &flat);
        fly(&mut off, idle(), 10.0, &flat);
        assert!(
            on.speed > 40.0,
            "1.04^10 of 30 is about 44, got {}",
            on.speed
        );
        assert!(
            (off.speed - BASE_SPEED_MPS).abs() < 1e-3,
            "off holds 30, got {}",
            off.speed
        );
        assert!(on.speed > off.speed);
    }

    /// The speed never passes 900 m/s, and after 120 s it sits on the cap.
    #[test]
    fn the_900_m_s_cap_holds_after_120_s() {
        let mut f = Flight::new([0.0, 100.0, 0.0]);
        for _ in 0..(120.0 / DT).round() as usize {
            f.step(&idle(), DT, &flat);
            assert!(f.speed <= MAX_SPEED_MPS, "over the cap: {}", f.speed);
        }
        assert!(
            f.speed > MAX_SPEED_MPS - 1.0,
            "after 120 s it is at the cap, got {}",
            f.speed
        );
    }

    /// The field of view rises with speed, never leaves 75..=110, and hits both ends.
    #[test]
    fn fov_rises_with_speed_and_stays_in_range() {
        let mut last = f32::MIN;
        for i in 0..=200 {
            let v = i as f32 * 5.0;
            let fov = speed_feel(v).fov_deg;
            assert!(
                (FOV_REST_DEG..=FOV_TOP_DEG).contains(&fov),
                "fov {fov} at {v} m/s"
            );
            assert!(fov >= last, "fov falls at {v} m/s");
            last = fov;
        }
        assert!((speed_feel(0.0).fov_deg - FOV_REST_DEG).abs() < 1e-4);
        assert!((speed_feel(MAX_SPEED_MPS).fov_deg - FOV_TOP_DEG).abs() < 1e-4);
    }

    /// Shake is about a centimetre at base speed and a few decimetres at the top; shake and
    /// streaks both grow with speed.
    #[test]
    fn shake_and_streaks_grow_with_speed() {
        let base = speed_feel(BASE_SPEED_MPS);
        let top = speed_feel(MAX_SPEED_MPS);
        assert!(
            (0.005..0.02).contains(&base.shake_m),
            "about a centimetre at base speed, got {}",
            base.shake_m
        );
        assert!(
            (0.2..0.5).contains(&top.shake_m),
            "a few decimetres at the top, got {}",
            top.shake_m
        );
        let mut last = speed_feel(0.0);
        for i in 1..=180 {
            let s = speed_feel(i as f32 * 5.0);
            assert!(s.shake_m >= last.shake_m, "shake falls at step {i}");
            assert!(
                s.streaks_per_s >= last.streaks_per_s,
                "streaks fall at step {i}"
            );
            last = s;
        }
    }

    /// A 45 degree dive toward a hill skims the ground: the body never goes under
    /// ground + 2 m, and it does reach that skim height.
    #[test]
    fn a_45_degree_dive_over_a_hill_never_goes_under_ground_plus_2() {
        let mut f = Flight::new([0.0, 150.0, 600.0]);
        f.step(
            &FlightInput {
                pitch_delta: -FRAC_PI_4,
                ..FlightInput::default()
            },
            DT,
            &hill,
        );
        assert!((f.pitch + FRAC_PI_4).abs() < 1e-6, "pitched 45 deg down");
        let cruise = FlightInput {
            forward: 1.0,
            ..FlightInput::default()
        };
        let mut skimmed = false;
        for _ in 0..(40.0 / DT).round() as usize {
            f.step(&cruise, DT, &hill);
            let floor = hill(f.pos[0], f.pos[2]) + GROUND_CLEARANCE_M;
            assert!(f.pos[1] >= floor - 1e-4, "under the ground at {:?}", f.pos);
            skimmed |= f.pos[1] <= floor + 1e-4;
        }
        assert!(skimmed, "a 45 degree dive reaches the skim height");
    }

    /// Yaw input turns the heading, and the body moves along the new heading.
    #[test]
    fn yaw_input_turns_the_heading() {
        let mut f = Flight::new([0.0, 100.0, 0.0]);
        let turn = FlightInput {
            forward: 1.0,
            yaw_delta: 0.5,
            ..FlightInput::default()
        };
        f.step(&turn, DT, &flat);
        assert!((f.yaw - 0.5).abs() < 1e-6, "yaw is 0.5, got {}", f.yaw);
        assert!(f.look()[0] > 0.4, "0.5 rad turns the look toward +x");
        assert!(f.pos[0] > 0.0, "and the body moves that way");
    }

    /// With no strafe or climb, the body moves exactly along the look direction.
    #[test]
    fn motion_follows_the_look_direction() {
        let mut f = Flight::new([0.0, 100.0, 0.0]);
        f.yaw = 1.0;
        f.pitch = 0.3;
        let before = f.pos;
        let forward = FlightInput {
            forward: 1.0,
            ..FlightInput::default()
        };
        f.step(&forward, DT, &flat);
        let d = [
            f.pos[0] - before[0],
            f.pos[1] - before[1],
            f.pos[2] - before[2],
        ];
        let l = f.look();
        let norm = d.iter().map(|c| c * c).sum::<f32>().sqrt();
        let along = (d[0] * l[0] + d[1] * l[1] + d[2] * l[2]) / norm;
        assert!(along > 0.9999, "motion is off the look direction: {along}");
    }

    /// Pitch input stops at 85 degrees.
    #[test]
    fn pitch_stops_at_85_degrees() {
        let mut f = Flight::new([0.0, 100.0, 0.0]);
        let up = FlightInput {
            pitch_delta: 10.0,
            ..FlightInput::default()
        };
        f.step(&up, DT, &flat);
        assert!((f.pitch - MAX_PITCH_RAD).abs() < 1e-6, "pitch {}", f.pitch);
    }

    fn scripted() -> Flight {
        let mut f = Flight::new([10.0, 120.0, -20.0]);
        for i in 0..900 {
            let t = i as f32 * DT;
            let input = FlightInput {
                forward: t.sin(),
                strafe: (0.7 * t).cos(),
                up: 0.5 * (0.3 * t).sin(),
                yaw_delta: 0.01 * (2.0 * t).sin(),
                pitch_delta: 0.005 * t.cos(),
                boost: i % 120 < 40,
            };
            f.step(&input, DT, &hill);
        }
        f
    }

    /// The same inputs give the same state, bit for bit.
    #[test]
    fn the_same_inputs_give_the_same_state() {
        assert_eq!(scripted(), scripted());
    }
}
