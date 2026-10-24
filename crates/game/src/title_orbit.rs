//! title_orbit: the title screen circles the idle pair, and the first press
//! starts the bout (row SW175).
//!
//! On the title the camera walks a slow circle around the two idle fighters;
//! each tick advances the orbit one degree, so a full turn takes six seconds.
//! The eye is placed on that circle through `camera_arm::arm_length`, the same
//! boom the fight camera uses, so a pillar or a wall shortens the orbit instead
//! of letting the lens pierce it. The first key or pad press enters `Match` on
//! the tick it arrives, so the screen never waits a frame on the press.
//!
//! Presentation only: the orbit angle and the eye are view state, never tick
//! state, so the sim and the golden `d57b5c8ae3096614` never read them. Pure
//! integers for the angle, one float at the eye. Determinism: the same tick
//! count gives the same angle and eye; no clock, no heap. Errors: none; every
//! function is total.

use crate::camera_arm::{arm_length, Aabb, FLOOR_LEN};
use crate::flow::{Flow, FlowEvent};

/// Degrees the orbit advances each tick: a full turn in 360 ticks, six
/// seconds at 60 Hz.
pub const ORBIT_DEG_PER_TICK: u16 = 1;
/// Whole turn in degrees: the orbit wraps here.
pub const ORBIT_TURN_DEG: u16 = 360;
/// Circle radius around the idle pair before the boom shortens it, metres.
pub const ORBIT_RADIUS_M: f32 = 4.5;
/// Eye height above the pair's feet, metres: a standing shot over the sand.
pub const ORBIT_HEIGHT_M: f32 = 1.7;
/// Radius of the lens so the boom clears a pillar by a camera's body, metres.
pub const ORBIT_LENS_M: f32 = 0.2;

/// The title camera's slow circle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleOrbit {
    angle_deg: u16,
}

impl TitleOrbit {
    /// The orbit at yaw zero: looking at the pair from +z.
    #[must_use]
    pub const fn new() -> Self {
        Self { angle_deg: 0 }
    }

    /// One tick of the circle: [`ORBIT_DEG_PER_TICK`] further round, wrapping
    /// at a whole turn.
    pub fn step(&mut self) {
        self.angle_deg = (self.angle_deg + ORBIT_DEG_PER_TICK) % ORBIT_TURN_DEG;
    }

    /// Where the circle has turned to, degrees `0..360`, zero on +z.
    #[must_use]
    pub const fn angle_deg(&self) -> u16 {
        self.angle_deg
    }

    /// The unit ground direction from the pair to the eye at this angle:
    /// `[sin, 0, cos]` in quarter-turn steps, so the modulo does no float work.
    #[must_use]
    pub fn dir(&self) -> [f32; 3] {
        // Quarter turns are exact; the four arcs use the small-angle mirror so
        // the table stays 91 entries and the eye repeats tick for tick.
        quarter_dir(self.angle_deg)
    }

    /// The title eye at this angle: [`ORBIT_RADIUS_M`] out from `pair_m`,
    /// [`ORBIT_HEIGHT_M`] up, shortened by the camera boom so a box between the
    /// pair and the lens pulls the eye in instead of being pierced.
    #[must_use]
    pub fn eye_m(&self, pair_m: [f32; 3], boxes: &[Aabb]) -> [f32; 3] {
        let dir = self.dir();
        let pivot = [pair_m[0], pair_m[1] + ORBIT_HEIGHT_M, pair_m[2]];
        let arm = arm_length(pivot, dir, ORBIT_RADIUS_M, ORBIT_LENS_M, boxes);
        let len = if arm.is_finite() && arm >= FLOOR_LEN {
            arm
        } else {
            FLOOR_LEN
        };
        [pair_m[0] + dir[0] * len, pivot[1], pair_m[2] + dir[2] * len]
    }
}

/// Sine and cosine of a compass angle with zero on +z, on a unit circle: the
/// angle is snapped to whole degrees and read from quarter-turn identities, so
/// the same tick gives the same eye on every machine.
fn quarter_dir(angle_deg: u16) -> [f32; 3] {
    let turn = angle_deg % ORBIT_TURN_DEG;
    let quarter = turn / 90;
    let rest = turn % 90;
    let rad = f32::from(rest) * core::f32::consts::PI / 180.0;
    let (s, c) = (rad.sin(), rad.cos());
    // Zero on +z, growing toward +x: (sin, cos) of the angle.
    match quarter {
        0 => [s, 0.0, c],
        1 => [c, 0.0, -s],
        2 => [-s, 0.0, -c],
        _ => [-c, 0.0, s],
    }
}

/// The flow after one frame of title input: the first key or pad press enters
/// `Match` on that same tick, and any other state is left alone.
#[must_use]
pub fn title_flow(flow: Flow, pressed: bool) -> Flow {
    if flow == Flow::Title && pressed {
        flow.on(FlowEvent::Start)
    } else {
        flow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pillar a few metres out on the +x side of the pair.
    fn pillar() -> Aabb {
        Aabb {
            min: [4.0, 0.0, -1.0],
            max: [5.0, 4.0, 1.0],
        }
    }

    #[test]
    fn the_title_orbit_turns_each_tick_and_the_first_press_starts_the_match() {
        // SW175, first half: one degree a tick, a full turn in 360, and the
        // eye walks the circle with it.
        let pair = [0.0, 0.0, 0.0];
        let mut orbit = TitleOrbit::new();
        assert_eq!(orbit.angle_deg(), 0);
        let at_zero = orbit.eye_m(pair, &[]);
        assert!((at_zero[2] - ORBIT_RADIUS_M).abs() < 1e-4, "starts on +z");
        assert!((at_zero[1] - ORBIT_HEIGHT_M).abs() < 1e-6, "at eye height");
        for tick in 1..=90_u16 {
            orbit.step();
            assert_eq!(orbit.angle_deg(), tick, "tick {tick} turned a degree");
        }
        let at_quarter = orbit.eye_m(pair, &[]);
        assert!((at_quarter[0] - ORBIT_RADIUS_M).abs() < 1e-4, "on +x at 90");
        assert!(at_quarter[2].abs() < 1e-4, "and level with the pair");
        for _ in 0..(ORBIT_TURN_DEG - 90) {
            orbit.step();
        }
        assert_eq!(orbit.angle_deg(), 0, "a full turn wraps to zero");
        assert_eq!(orbit.eye_m(pair, &[]), at_zero, "same angle, same eye");

        // The boom shortens the orbit instead of piercing a pillar at 90
        // degrees, and never comes in under the floor length.
        let clipped = orbit_at(90).eye_m(pair, &[pillar()]);
        let free = orbit_at(90).eye_m(pair, &[]);
        assert!(
            (clipped[0] - pair[0]).abs() < (free[0] - pair[0]).abs(),
            "the pillar pulled the eye in: {clipped:?} against {free:?}"
        );
        assert!(clipped[0] >= FLOOR_LEN, "never past the floor length");

        // SW175, second half: the first press on the title enters Match on
        // that tick; no press keeps the title.
        let mut flow = Flow::Title;
        assert_eq!(title_flow(flow, false), Flow::Title, "no press, no bout");
        flow = title_flow(flow, true);
        assert_eq!(flow, Flow::Match, "the press starts the bout this tick");
        assert!(flow.ticks_run(), "the match tick runs from that press on");
        assert_eq!(
            title_flow(flow, true),
            Flow::Match,
            "a press in a bout is not a restart"
        );
        // x2: the same tick count gives the same eye twice.
        assert_eq!(
            orbit_at(37).eye_m(pair, &[pillar()]),
            orbit_at(37).eye_m(pair, &[pillar()])
        );
    }

    /// An orbit turned to `deg`.
    fn orbit_at(deg: u16) -> TitleOrbit {
        let mut orbit = TitleOrbit::new();
        for _ in 0..deg {
            orbit.step();
        }
        orbit
    }
}
