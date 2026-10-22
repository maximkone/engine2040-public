//! sword (SD5, docs/SPRINT-33-speed-demo.md): the giant blade at the body's right side.
//!
//! A 12 m blade (the gladius's proportions, scaled) rides the right side of whatever moves
//! (the flyer or the car). At rest it sticks straight out to the right, so passing a tower
//! cuts it along the direction of travel. A swing sweeps it from behind to in front over
//! [`SWING_S`], tilted [`TILT_RAD`] down so the cut is slanted and the top half slides off.
//! The cut plane is the plane the blade sweeps (`engine_destruction::csg::blade_plane`).
//! Presentation only, deterministic. Units: meters, seconds, radians.

use super::num::{add, len, scale, sub};
use engine_destruction::csg::{blade_plane, CutPlane};

/// Blade length from hilt to tip, meters.
pub(super) const BLADE_M: f32 = 12.0;
/// Hilt offset from the body: to the right, up, forward; meters.
pub(super) const HILT_RIGHT_M: f32 = 1.2;
pub(super) const HILT_UP_M: f32 = 0.2;
pub(super) const HILT_FORWARD_M: f32 = 0.3;
/// One swing, seconds.
pub(super) const SWING_S: f32 = 0.35;
/// The swing's arc: from pointing straight back to straight ahead, radians about up.
pub(super) const SWING_FROM_RAD: f32 = -std::f32::consts::FRAC_PI_2;
pub(super) const SWING_TO_RAD: f32 = std::f32::consts::FRAC_PI_2;
/// The blade dips this far below level, radians (20 deg): a slanted cut slides.
pub(super) const TILT_RAD: f32 = 0.35;
/// Slower than this the blade is not moving through anything, m/s.
const CUT_MIN_SPEED_M_S: f32 = 0.5;

/// Where the blade is this frame and the plane it cuts along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Blade {
    pub hilt: [f32; 3],
    pub tip: [f32; 3],
    /// The plane the blade sweeps; `None` when it is not moving (nothing to cut with).
    pub plane: Option<CutPlane>,
}

/// The sword's state: idle, or this far into a swing.
#[derive(Clone, Debug, Default)]
pub(super) struct Sword {
    swing_s: Option<f32>,
    /// Swings started (the exit line counts them).
    pub swings: u32,
}

impl Sword {
    /// Start a swing unless one is running.
    pub(super) fn swing(&mut self) {
        if self.swing_s.is_none() {
            self.swing_s = Some(0.0);
            self.swings += 1;
        }
    }

    /// True while a swing runs.
    pub(super) fn swinging(&self) -> bool {
        self.swing_s.is_some()
    }

    /// Advance `dt` seconds; a finished swing returns to rest.
    pub(super) fn step(&mut self, dt: f32) {
        if let Some(t) = self.swing_s.as_mut() {
            *t += dt.max(0.0);
            if *t >= SWING_S {
                self.swing_s = None;
            }
        }
    }

    /// The blade's angle about up, from the body's right: 0 at rest, the arc in a swing.
    fn angle(&self) -> f32 {
        self.swing_s.map_or(0.0, |t| {
            let k = (t / SWING_S).clamp(0.0, 1.0);
            // Ease in and out: fast through the middle, where it hits.
            let e = k * k * (3.0 - 2.0 * k);
            SWING_FROM_RAD + (SWING_TO_RAD - SWING_FROM_RAD) * e
        })
    }

    /// The blade for a body at `pos` facing `yaw` (radians, 0 looks down -z, + turns right, as the flyer) moving at
    /// `vel` (m/s). The cut plane is spanned by the blade and its motion: the swing's own
    /// sweep plus the body's travel.
    pub(super) fn blade(&self, pos: [f32; 3], yaw: f32, vel: [f32; 3]) -> Blade {
        let forward = [yaw.sin(), 0.0, -yaw.cos()];
        let right = [yaw.cos(), 0.0, yaw.sin()];
        let up = [0.0, 1.0, 0.0];
        let hilt = add(
            pos,
            add(
                scale(right, HILT_RIGHT_M),
                add(scale(up, HILT_UP_M), scale(forward, HILT_FORWARD_M)),
            ),
        );
        let a = self.angle();
        let flat = add(scale(right, a.cos()), scale(forward, a.sin()));
        let dir = add(scale(flat, TILT_RAD.cos()), scale(up, -TILT_RAD.sin()));
        let tip = add(hilt, scale(dir, BLADE_M));
        // The sweep's own velocity at the blade's middle, plus the body's travel.
        let sweep = if self.swinging() {
            let rate = (SWING_TO_RAD - SWING_FROM_RAD) / SWING_S;
            let tangent = add(scale(right, -a.sin()), scale(forward, a.cos()));
            scale(tangent, rate * BLADE_M * 0.5)
        } else {
            [0.0; 3]
        };
        let motion = add(sweep, vel);
        let plane = if len(motion) < CUT_MIN_SPEED_M_S {
            None
        } else {
            blade_plane(&hilt, &dir, &motion).ok()
        };
        Blade { hilt, tip, plane }
    }
}

/// True when the blade segment passes through the box `min..max` (slab test).
pub(super) fn blade_hits_box(blade: &Blade, min: [f32; 3], max: [f32; 3]) -> bool {
    let d = sub(blade.tip, blade.hilt);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        if d[axis].abs() < 1e-9 {
            if blade.hilt[axis] < min[axis] || blade.hilt[axis] > max[axis] {
                return false;
            }
            continue;
        }
        let a = (min[axis] - blade.hilt[axis]) / d[axis];
        let b = (max[axis] - blade.hilt[axis]) / d[axis];
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
        if t0 > t1 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::num::dot;
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// At rest the blade sticks out to the right; a swing carries the tip from behind the
    /// body, past its right side, to in front of it, at full length the whole way.
    #[test]
    fn the_tip_traces_the_arc_from_behind_to_ahead() {
        let mut sword = Sword::default();
        let rest = sword.blade([0.0; 3], 0.0, [0.0; 3]);
        assert!(
            rest.tip[0] > BLADE_M * 0.9,
            "at rest the blade points right (+x at yaw 0)"
        );
        assert!(rest.plane.is_none(), "a still blade cuts nothing");
        sword.swing();
        let start = sword.blade([0.0; 3], 0.0, [0.0; 3]);
        assert!(
            start.tip[2] > BLADE_M * 0.8,
            "the swing starts behind (+z at yaw 0)"
        );
        let mut tips = Vec::new();
        while sword.swinging() {
            let b = sword.blade([0.0; 3], 0.0, [0.0; 3]);
            assert!(
                (len(sub(b.tip, b.hilt)) - BLADE_M).abs() < 1e-3,
                "full length"
            );
            assert!(b.tip[1] < b.hilt[1], "tilted down");
            tips.push(b.tip);
            sword.step(DT);
        }
        let end = tips.last().copied().expect("frames");
        assert!(end[2] < -BLADE_M * 0.8, "it ends ahead (-z at yaw 0)");
        assert!(
            tips.iter().any(|t| t[0] > BLADE_M * 0.85),
            "it passes the right side"
        );
        assert_eq!(sword.swings, 1);
        assert!(!sword.swinging(), "back at rest after the swing");
    }

    /// Mid-swing the cut plane holds the blade and its sweep: its normal is square to both.
    #[test]
    fn the_cut_plane_is_the_plane_the_blade_sweeps() {
        let mut sword = Sword::default();
        sword.swing();
        sword.step(SWING_S * 0.5);
        let b = sword.blade([3.0, 50.0, -2.0], 0.4, [0.0; 3]);
        let plane = b.plane.expect("a moving blade cuts");
        let dir = sub(b.tip, b.hilt);
        assert!(
            dot(plane.normal, dir).abs() / len(dir) < 1e-4,
            "the blade lies in the plane"
        );
        let on = dot(plane.normal, b.hilt) - plane.dist;
        assert!(on.abs() < 1e-3, "the hilt lies in the plane");
        assert!(
            plane.normal[1].abs() > 0.9,
            "a mostly level sweep cuts mostly level"
        );
    }

    /// A tower within reach at the right side is hit during one swing, one far away never;
    /// at rest the blade passing a tower by flight hits it too.
    #[test]
    fn a_tower_in_reach_is_hit_and_a_far_one_never() {
        let near = ([6.0, -20.0, -2.0], [10.0, 40.0, 2.0]);
        let far = ([40.0, -20.0, -2.0], [44.0, 40.0, 2.0]);
        let mut sword = Sword::default();
        sword.swing();
        let (mut near_hits, mut far_hits) = (0, 0);
        while sword.swinging() {
            let b = sword.blade([0.0; 3], 0.0, [0.0; 3]);
            near_hits += usize::from(blade_hits_box(&b, near.0, near.1));
            far_hits += usize::from(blade_hits_box(&b, far.0, far.1));
            sword.step(DT);
        }
        assert!(near_hits > 0, "the swing goes through the near tower");
        assert_eq!(far_hits, 0, "the far tower is out of reach");
        let passing = Sword::default().blade([0.0, 0.0, 0.0], 0.0, [0.0, 0.0, -80.0]);
        assert!(
            blade_hits_box(&passing, near.0, near.1),
            "flying past, the resting blade hits"
        );
        assert!(
            passing.plane.is_some(),
            "and travel alone gives it a cut plane"
        );
    }
}
