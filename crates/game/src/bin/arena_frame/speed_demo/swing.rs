//! swing (HC5, docs/SPRINT-34-hero-city.md): the Spider-Man web swing.
//!
//! The hero fires a web at the top edge of a building ahead and above, then swings on a
//! rope. While attached the rope is a rigid rod: the swinger is held on the sphere of
//! radius `rope_m` around the anchor and loses the outward part of its velocity; while it is
//! shorter than the rope it is slack. Gravity and quadratic air drag act every step; steer
//! leans it sideways, perpendicular to the rope and the velocity; holding reel pulls the rope
//! in at 2 m/s down to 5 m. Release keeps the velocity, adds 15% along it and a small upward
//! kick. The swinger never sinks below the ground plus 1 m. Presentation only,
//! deterministic: no clock, no RNG. Units: meters, seconds.
//!
//! Frame: y is up, a steer of +1 leans right of the travel direction, and the look direction
//! follows the flight convention (yaw 0 looks down -z, + turns right).

use super::num::{add, cross, dot, f32_of, len, scale, sub, unit_or};

/// Gravity, m/s^2.
const GRAVITY_MPS2: f32 = 9.81;
/// Quadratic air drag: the drag acceleration is this times speed times velocity, 1/m.
const DRAG_PER_M: f32 = 0.002;
/// Sideways acceleration at full steer, m/s^2.
const STEER_MPS2: f32 = 6.0;
/// How fast a held reel shortens the rope, m/s.
const REEL_MPS: f32 = 2.0;
/// The shortest the rope reels down to, m.
const MIN_ROPE_M: f32 = 5.0;
/// The farthest an anchor can be, m (3-D distance from the swinger).
const MAX_RANGE_M: f32 = 120.0;
/// An anchor must sit at least this far above the swinger, m.
const MIN_ABOVE_M: f32 = 8.0;
/// An anchor must lie this far ahead: its direction dotted with the look direction exceeds
/// this (dimensionless, 0.3 is about 72 degrees off the look).
const AHEAD_DOT: f32 = 0.3;
/// The horizontal distance to the anchor the arc prefers, as a fraction of the height above
/// it (dimensionless: 0.6 gives a good, wide arc).
const ARC_RATIO: f32 = 0.6;
/// The release boost along the velocity (dimensionless: 15% faster).
const RELEASE_BOOST: f32 = 0.15;
/// The upward kick on release, m/s.
const RELEASE_UP_MPS: f32 = 3.0;
/// Height kept above the ground, m.
const GROUND_CLEARANCE_M: f32 = 1.0;
/// Integration substeps per call to `step` (a fixed 240 Hz at 60 fps frames).
const SUBSTEPS: usize = 4;
/// The look direction when there is no horizontal travel: forward at yaw 0, -z.
const FORWARD: [f32; 3] = [0.0, 0.0, -1.0];
/// Straight down, the rope direction when the swinger sits on its anchor.
const DOWN: [f32; 3] = [0.0, -1.0, 0.0];

/// One step's controls. `hold` is the web button (reel works only while it is held; the
/// caller calls `release` when it lets go), `reel` pulls the rope in, `steer` is -1..1
/// (+1 leans right of the travel direction; NaN reads as 0 and it is clamped).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct SwingInput {
    /// The web button is down.
    pub hold: bool,
    /// The reel pull is on; it shortens the rope while `hold` is also on.
    pub reel: bool,
    /// Lean: -1 left, +1 right, 0 none.
    pub steer: f32,
}

/// The swinging body. `anchor` is the web's far end while attached; `rope_m` is the rope
/// length while attached, meters. `drag_per_m` is the air drag (1/m); tests zero it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Swing {
    /// Position, meters.
    pub pos: [f32; 3],
    /// Velocity, m/s.
    pub vel: [f32; 3],
    /// The attached web's anchor, or None when free.
    pub anchor: Option<[f32; 3]>,
    /// The rope length while attached, meters.
    pub rope_m: f32,
    /// Air drag per meter (1/m).
    drag_per_m: f32,
}

impl Swing {
    /// A free body at `pos` moving at `vel`, with the default air drag.
    pub(super) fn new(pos: [f32; 3], vel: [f32; 3]) -> Self {
        Swing {
            pos,
            vel,
            anchor: None,
            rope_m: 0.0,
            drag_per_m: DRAG_PER_M,
        }
    }

    /// Fire a web at the best anchor in `boxes` (each `(min, max)` corner pair, meters). The
    /// look is the horizontal velocity, or forward (-z) when it has none. Nothing changes when
    /// no anchor is in range, ahead and above.
    pub(super) fn attach(&mut self, boxes: &[([f32; 3], [f32; 3])]) {
        let look = horizontal_or(self.vel, FORWARD);
        if let Some(anchor) = pick_anchor(self.pos, self.vel, look, boxes) {
            self.anchor = Some(anchor);
            self.rope_m = len(sub(anchor, self.pos));
        }
    }

    /// Let go of the web: the velocity is kept, 15% more along it, and 3 m/s up. Does nothing
    /// when free.
    pub(super) fn release(&mut self) {
        if self.anchor.is_none() {
            return;
        }
        self.anchor = None;
        self.rope_m = 0.0;
        let boosted = scale(self.vel, 1.0 + RELEASE_BOOST);
        self.vel = add(boosted, [0.0, RELEASE_UP_MPS, 0.0]);
    }

    /// Advance `dt` seconds in `SUBSTEPS` equal substeps. `ground(x, z)` is the terrain
    /// height; the body stays 1 m above it. A non-finite or non-positive `dt` does nothing.
    pub(super) fn step(&mut self, input: &SwingInput, dt: f32, ground: &dyn Fn(f32, f32) -> f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let sub_dt = dt / f32_of(SUBSTEPS);
        for _ in 0..SUBSTEPS {
            self.substep(input, sub_dt, ground);
        }
    }

    /// The speed, m/s.
    pub(super) fn speed(&self) -> f32 {
        len(self.vel)
    }

    /// One fixed substep: forces, then the rope, then the ground.
    fn substep(&mut self, input: &SwingInput, dt: f32, ground: &dyn Fn(f32, f32) -> f32) {
        let drag = scale(self.vel, -self.drag_per_m * len(self.vel));
        let mut acc = add([0.0, -GRAVITY_MPS2, 0.0], drag);
        if let Some(anchor) = self.anchor {
            let rope_dir = unit_or(sub(self.pos, anchor), DOWN);
            let side = unit_or(cross(rope_dir, self.vel), [0.0; 3]);
            let lean = steer_axis(input.steer);
            acc = add(acc, scale(side, STEER_MPS2 * lean));
            if input.hold && input.reel {
                self.rope_m = (self.rope_m - REEL_MPS * dt).max(MIN_ROPE_M);
            }
        }
        self.vel = add(self.vel, scale(acc, dt));
        self.pos = add(self.pos, scale(self.vel, dt));
        if let Some(anchor) = self.anchor {
            self.hold_rope(anchor);
        }
        let floor = ground(self.pos[0], self.pos[2]) + GROUND_CLEARANCE_M;
        if self.pos[1] < floor {
            self.pos[1] = floor;
            self.vel[1] = self.vel[1].max(0.0);
        }
    }

    /// The rope: when the swinger is at or beyond the rope length, put it back on the sphere
    /// and drop the outward part of its velocity. Shorter than the rope, nothing happens.
    fn hold_rope(&mut self, anchor: [f32; 3]) {
        let offset = sub(self.pos, anchor);
        let dist = len(offset);
        if dist < self.rope_m || dist <= 1e-9 {
            return;
        }
        let dir = scale(offset, 1.0 / dist);
        self.pos = add(anchor, scale(dir, self.rope_m));
        let outward = dot(self.vel, dir);
        if outward > 0.0 {
            self.vel = sub(self.vel, scale(dir, outward));
        }
    }
}

/// The best anchor for a swing from `pos` moving along `look_dir`, or None. Candidates are
/// the four top corners and the midpoints of the four top edges of each box (`(min, max)`
/// corner pairs, meters). A candidate counts when it is within 120 m, at least 8 m above
/// the swinger, and ahead of it (its direction dotted with the look exceeds 0.3). Among
/// those the best arc wins: the horizontal distance nearest 0.6 of the height above, and
/// the most aligned with the travel. Ties keep the first candidate.
pub(super) fn pick_anchor(
    pos: [f32; 3],
    vel: [f32; 3],
    look_dir: [f32; 3],
    boxes: &[([f32; 3], [f32; 3])],
) -> Option<[f32; 3]> {
    let look = unit_or(look_dir, horizontal_or(vel, FORWARD));
    let mut best: Option<(f32, [f32; 3])> = None;
    for &(min, max) in boxes {
        for candidate in top_candidates(min, max) {
            let Some(score) = arc_score(pos, look, candidate) else {
                continue;
            };
            match best {
                Some((known, _)) if known <= score => {}
                _ => best = Some((score, candidate)),
            }
        }
    }
    best.map(|(_, anchor)| anchor)
}

/// The score of one anchor for the arc, lower is better; None when it is out of range, too
/// low, or not ahead.
fn arc_score(pos: [f32; 3], look: [f32; 3], candidate: [f32; 3]) -> Option<f32> {
    let to = sub(candidate, pos);
    let dist = len(to);
    let height = to[1];
    if dist > MAX_RANGE_M || height < MIN_ABOVE_M {
        return None;
    }
    let ahead = dot(scale(to, 1.0 / dist), look);
    if ahead <= AHEAD_DOT || ahead.is_nan() {
        return None;
    }
    let horizontal = len([to[0], 0.0, to[2]]);
    let ratio = horizontal / height;
    Some((ratio - ARC_RATIO).abs() + 0.5 * (1.0 - ahead))
}

/// The eight anchor candidates of a box: its four top corners, then the midpoints of its
/// four top edges. The top is the `max` y.
fn top_candidates(min: [f32; 3], max: [f32; 3]) -> [[f32; 3]; 8] {
    let a = [min[0], max[1], min[2]];
    let b = [max[0], max[1], min[2]];
    let c = [max[0], max[1], max[2]];
    let d = [min[0], max[1], max[2]];
    [
        a,
        b,
        c,
        d,
        scale(add(a, b), 0.5),
        scale(add(b, c), 0.5),
        scale(add(c, d), 0.5),
        scale(add(d, a), 0.5),
    ]
}

/// `v` as a horizontal unit direction, or `fallback` when it has no horizontal part.
fn horizontal_or(v: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    unit_or([v[0], 0.0, v[2]], fallback)
}

/// The steer input: NaN reads as 0, anything else is clamped to -1..1.
fn steer_axis(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::num::floor_idx;
    use super::*;

    /// The 60 Hz frame step the tests drive the swing with, seconds.
    const DT: f32 = 1.0 / 60.0;

    /// Flat ground at height 0.
    fn flat(_x: f32, _z: f32) -> f32 {
        0.0
    }

    /// The number of fixed frame steps in `secs` seconds.
    fn steps(secs: f32) -> usize {
        floor_idx((secs / DT).round())
    }

    /// A 25 m tall bump of ground about 10 m across at the origin.
    fn bump(x: f32, z: f32) -> f32 {
        25.0 * (-(x * x + z * z) / 50.0).exp()
    }

    /// A swing hung from `anchor` on a rope of `rope` meters, released from rest `deg`
    /// degrees from the vertical on the +x side, with no air drag.
    fn pendulum(anchor: [f32; 3], rope: f32, deg: f32) -> Swing {
        let rad = deg.to_radians();
        let pos = add(anchor, [rope * rad.sin(), -rope * rad.cos(), 0.0]);
        let mut s = Swing::new(pos, [0.0; 3]);
        s.anchor = Some(anchor);
        s.rope_m = rope;
        s.drag_per_m = 0.0;
        s
    }

    /// A street canyon along z: two rows of 20 m boxes, 40 m tall, 10 m wide, each side
    /// of a 40 m wide corridor, running from z = -200 to +200.
    fn canyon() -> Vec<([f32; 3], [f32; 3])> {
        let mut boxes = Vec::new();
        for i in 0..20 {
            let z0 = -200.0 + 20.0 * f32_of(i);
            boxes.push(([-30.0, 0.0, z0], [-20.0, 40.0, z0 + 20.0]));
            boxes.push(([20.0, 0.0, z0], [30.0, 40.0, z0 + 20.0]));
        }
        boxes
    }

    /// From rest at 60 degrees on a 40 m rope, the swing reaches the far side at about the
    /// height it started at (within 5%): energy is kept with no drag.
    #[test]
    fn a_swing_from_rest_at_60_deg_reaches_the_far_side_near_its_start_height() {
        let anchor = [0.0, 60.0, 0.0];
        let mut s = pendulum(anchor, 40.0, 60.0);
        let start_h = s.pos[1];
        let mut crossed = false;
        let mut turn: Option<(f32, f32)> = None;
        for _ in 0..steps(9.0) {
            s.step(&SwingInput::default(), DT, &flat);
            if s.pos[0] < anchor[0] {
                crossed = true;
            }
            if crossed {
                let speed = s.speed();
                let better = match turn {
                    Some((best, _)) => speed < best,
                    None => true,
                };
                if better {
                    turn = Some((speed, s.pos[1]));
                }
            }
        }
        assert!(crossed, "the swing crosses to the far side");
        let (_, far_h) = turn.unwrap_or((0.0, 0.0));
        assert!(
            (far_h - start_h).abs() <= 0.05 * start_h,
            "far side {far_h} m vs start {start_h} m"
        );
    }

    /// A taut rope never stretches past 1% over 20 s of swinging, even with steer on.
    #[test]
    fn a_taut_rope_never_stretches_past_one_percent() {
        let anchor = [0.0, 60.0, 0.0];
        let mut s = pendulum(anchor, 40.0, 60.0);
        let input = SwingInput {
            hold: true,
            reel: false,
            steer: 0.5,
        };
        for _ in 0..steps(20.0) {
            s.step(&input, DT, &flat);
            let stretch = len(sub(s.pos, anchor)) - 40.0;
            assert!(stretch <= 0.01 * 40.0, "rope stretched {stretch} m");
        }
    }

    /// Release keeps the speed (never less) and adds the 15% boost and the 3 m/s kick.
    #[test]
    fn release_keeps_the_speed_and_adds_the_boost() {
        let mut s = pendulum([0.0, 60.0, 0.0], 40.0, 60.0);
        s.vel = [6.0, 2.0, -3.0];
        let before = s.speed();
        s.release();
        assert!(s.anchor.is_none(), "released");
        assert!(s.speed() >= before, "speed {} vs {}", s.speed(), before);
        let want = add(scale([6.0, 2.0, -3.0], 1.15), [0.0, 3.0, 0.0]);
        assert!(len(sub(s.vel, want)) < 1e-5, "velocity {:?}", s.vel);
    }

    /// Release on a free body does nothing.
    #[test]
    fn release_on_a_free_body_changes_nothing() {
        let mut s = Swing::new([0.0, 50.0, 0.0], [1.0, 0.0, 0.0]);
        let before = s;
        s.release();
        assert_eq!(s, before, "a free body is untouched");
    }

    /// No box within 120 m, or none ahead and above, gives no anchor.
    #[test]
    fn no_anchor_when_none_is_in_range_or_all_are_behind() {
        let pos = [0.0, 20.0, 0.0];
        let look = [0.0, 0.0, -1.0];
        let far = [([-5.0, 60.0, -200.0], [5.0, 70.0, -190.0])];
        assert_eq!(pick_anchor(pos, [0.0; 3], look, &far), None, "out of range");
        let behind = [([-5.0, 40.0, 10.0], [5.0, 50.0, 20.0])];
        assert_eq!(pick_anchor(pos, [0.0; 3], look, &behind), None, "behind");
        let low = [([-5.0, 22.0, -30.0], [5.0, 26.0, -20.0])];
        assert_eq!(pick_anchor(pos, [0.0; 3], look, &low), None, "under 8 m up");
        assert_eq!(pick_anchor(pos, [0.0; 3], look, &[]), None, "no boxes");
    }

    /// In a street canyon the anchor is ahead of the swinger and above it.
    #[test]
    fn the_anchor_in_a_canyon_is_ahead_and_above() {
        let pos = [0.0, 20.0, 0.0];
        let vel = [0.0, 0.0, 15.0];
        let look = [0.0, 0.0, 1.0];
        let boxes = canyon();
        let Some(a) = pick_anchor(pos, vel, look, &boxes) else {
            panic!("the canyon has an anchor ahead");
        };
        let to = sub(a, pos);
        assert!(a[2] > 0.0, "ahead: {a:?}");
        assert!(a[1] >= pos[1] + MIN_ABOVE_M, "above: {a:?}");
        assert!(dot(scale(to, 1.0 / len(to)), look) > AHEAD_DOT);
    }

    /// `attach` takes the canyon's anchor and sets the rope to the distance to it.
    #[test]
    fn attach_takes_the_anchor_and_sets_the_rope() {
        let mut s = Swing::new([0.0, 20.0, 0.0], [0.0, 0.0, 15.0]);
        s.attach(&canyon());
        let a = s.anchor.unwrap_or([f32::NAN; 3]);
        assert!(a[2] > 0.0, "attached ahead: {a:?}");
        assert!(
            (s.rope_m - len(sub(a, s.pos))).abs() < 1e-4,
            "rope is the distance"
        );
        let mut none = Swing::new([0.0, 20.0, 0.0], [0.0, 0.0, 15.0]);
        none.attach(&[]);
        assert!(none.anchor.is_none(), "no box, no attach");
    }

    /// The rope reels in at 2 m/s while held (down to 5 m), and not without the reel.
    #[test]
    fn reel_shortens_the_rope_only_while_held() {
        let mut s = pendulum([0.0, 60.0, 0.0], 40.0, 10.0);
        let idle = SwingInput {
            hold: true,
            ..SwingInput::default()
        };
        s.step(&idle, 1.0, &flat);
        assert!((s.rope_m - 40.0).abs() < 1e-5, "no reel, rope {}", s.rope_m);
        let reel = SwingInput {
            hold: true,
            reel: true,
            steer: 0.0,
        };
        s.step(&reel, 1.0, &flat);
        assert!((s.rope_m - 38.0).abs() < 1e-3, "2 m/s, rope {}", s.rope_m);
        for _ in 0..60 {
            s.step(&reel, 1.0, &flat);
        }
        assert!(
            (s.rope_m - MIN_ROPE_M).abs() < 1e-5,
            "floor 5 m, rope {}",
            s.rope_m
        );
    }

    /// The swinger never goes below the ground plus 1 m, even swinging past a bump.
    #[test]
    fn the_swinger_never_goes_below_ground_plus_one() {
        let anchor = [0.0, 40.0, 0.0];
        let mut s = pendulum(anchor, 40.0, 80.0);
        let input = SwingInput {
            hold: true,
            reel: false,
            steer: 1.0,
        };
        for _ in 0..steps(12.0) {
            s.step(&input, DT, &bump);
            let floor = bump(s.pos[0], s.pos[2]) + GROUND_CLEARANCE_M;
            assert!(s.pos[1] >= floor - 1e-4, "under the ground at {:?}", s.pos);
        }
    }

    /// The same inputs give the same state, bit for bit.
    #[test]
    fn the_same_inputs_give_the_same_state() {
        let run = || {
            let mut s = pendulum([0.0, 60.0, 0.0], 40.0, 45.0);
            for i in 0..600 {
                let input = SwingInput {
                    hold: true,
                    reel: i % 200 < 50,
                    steer: (f32_of(i) * 0.05).sin(),
                };
                s.step(&input, DT, &bump);
            }
            s
        };
        assert_eq!(run(), run());
    }
}
