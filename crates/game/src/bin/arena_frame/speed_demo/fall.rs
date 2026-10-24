//! fall (SD9): a cut tower top is a physfx stone body, not a kinematic slide.
//!
//! `cut.rs` still cuts the tower and `Slide` still carries the top while its centre is
//! over the cut face; the moment it leaves the face this module takes it: one stone box
//! per half in a shared float world with the sand floor and every standing tower as a
//! static box, so the top falls under gravity, lands on the sand and is stopped by the
//! towers around it instead of sliding through them. Presentation only (engine-physfx is
//! never hashed), fixed 1/60 steps accumulated from the frame `dt`, no clock, no RNG: the
//! same spawn twice gives the same bytes.
//!
//! The draw path wants a rigid pose the way `flat_mesh` takes it (`offset`, `axis`,
//! `angle` about `pivot`), so [`Falls::pose`] hands back the body's translation delta and
//! the axis-angle of its rotation delta from the spawn.

use engine_physfx::{Batch, Key, Material, PhysFx, Shard, Tier, Transform};

/// Fixed step of the float world, seconds: `PhysFx::step` is one 60 Hz tick.
const FX_DT: f32 = 1.0 / 60.0;
/// Longest frame the accumulator takes in one call, seconds: a hitch does not fast-forward
/// a falling half through the sand.
const MAX_FRAME_S: f32 = 0.1;
/// Most 60 Hz steps one call may run.
const MAX_STEPS_PER_CALL: u32 = 6;
/// Sand floor half-extents, meters: the demo terrain is kilometres wide but a half never
/// travels far in [`crate::speed_demo::scene`]'s piece life.
const FLOOR_HALF: [f32; 3] = [400.0, 0.5, 400.0];
/// Sand top, meters (the demo's flat ground at the spawn point sits at y = 0).
const FLOOR_TOP_Y: f32 = 0.0;

/// One falling half, as the shared world knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Half {
    /// The debris batch the shard went into.
    batch: u32,
    /// Its index in that batch (one shard per spawn, so always zero).
    index: u32,
    /// Where it started: the pose the draw path still calls `pivot`.
    spawn: Transform,
}

/// A rigid pose for the piece mesh, in the shape `flat_mesh` takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pose {
    /// Translation from the spawn, meters.
    pub offset: [f32; 3],
    /// Rotation axis, unit.
    pub axis: [f32; 3],
    /// Rotation about `axis`, radians.
    pub angle: f32,
}

/// The shared float world for every falling half of one demo.
pub(super) struct Falls {
    fx: PhysFx,
    /// Unspent frame time, seconds.
    carry: f32,
    /// Halves spawned so far, for the receipt line.
    spawned: u32,
}

impl Falls {
    /// A world with the sand floor at [`FLOOR_TOP_Y`].
    pub(super) fn new(tier: Tier) -> Self {
        let mut fx = PhysFx::new(tier);
        let _ = fx.add_static_box(FLOOR_HALF, [0.0, FLOOR_TOP_Y - FLOOR_HALF[1], 0.0]);
        Self {
            fx,
            carry: 0.0,
            spawned: 0,
        }
    }

    /// Adds one standing tower as an immovable box so a falling half is stopped by it.
    pub(super) fn add_tower(&mut self, half: [f32; 3], centre: [f32; 3]) {
        if half.iter().all(|h| h.is_finite() && *h > 0.0) && centre.iter().all(|c| c.is_finite()) {
            let _ = self.fx.add_static_box(half, centre);
        }
    }

    /// Hands one cut top to the world: a stone box at `at` (the piece's pivot), pushed by
    /// the slide's velocity. `None` when the world refuses the spawn (a bad extent, past
    /// the body cap), which leaves the caller on its kinematic slide.
    pub(super) fn spawn(
        &mut self,
        at: Transform,
        half: [f32; 3],
        velocity: [f32; 3],
    ) -> Option<Half> {
        let shard = Shard { at, half, velocity };
        let Batch { id, admitted } = self
            .fx
            .spawn_debris_of(Material::Stone, &[shard], [0.0; 3])
            .ok()?;
        if admitted == 0 {
            return None;
        }
        self.spawned = self.spawned.saturating_add(1);
        Some(Half {
            batch: id,
            index: 0,
            spawn: at,
        })
    }

    /// Advances the float world by the frame time, in fixed 60 Hz steps.
    pub(super) fn step(&mut self, dt: f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        self.carry = (self.carry + dt.min(MAX_FRAME_S)).min(MAX_FRAME_S);
        let mut steps = 0;
        while self.carry >= FX_DT && steps < MAX_STEPS_PER_CALL {
            self.fx.step();
            self.carry -= FX_DT;
            steps += 1;
        }
    }

    /// The half's pose for the draw path, blended between the last two ticks like every
    /// other physfx read; `None` once the world dropped it (the cap recycled it).
    pub(super) fn pose(&self, half: &Half, alpha: f32) -> Option<Pose> {
        let placed = self.fx.transforms(alpha).ok()?;
        let at = placed
            .into_iter()
            .find(|one| {
                one.key
                    == Key::Shard {
                        batch: half.batch,
                        index: half.index,
                    }
            })?
            .at;
        Some(Pose {
            offset: [
                at.pos[0] - half.spawn.pos[0],
                at.pos[1] - half.spawn.pos[1],
                at.pos[2] - half.spawn.pos[2],
            ],
            ..axis_angle(at.rot)
        })
    }

    /// Halves this world has taken.
    pub(super) fn spawned(&self) -> u32 {
        self.spawned
    }
}

/// Axis-angle of a unit quaternion `[x, y, z, w]`; a still or degenerate one reads zero.
fn axis_angle(q: [f32; 4]) -> Pose {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let len = (x * x + y * y + z * z + w * w).sqrt();
    if !len.is_finite() || len <= 1.0e-9 {
        return Pose {
            offset: [0.0; 3],
            axis: [0.0, 1.0, 0.0],
            angle: 0.0,
        };
    }
    let w = (w / len).clamp(-1.0, 1.0);
    let angle = 2.0 * w.acos();
    let s = (x * x + y * y + z * z).sqrt() / len;
    let axis = if s <= 1.0e-6 || !s.is_finite() {
        [0.0, 1.0, 0.0]
    } else {
        [x / len / s, y / len / s, z / len / s]
    };
    Pose {
        offset: [0.0; 3],
        axis,
        angle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facing_up() -> Transform {
        Transform {
            pos: [0.0, 4.0, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn half() -> [f32; 3] {
        [0.5, 0.5, 0.5]
    }

    /// The cut top falls under gravity instead of hanging at the cut.
    #[test]
    fn a_cut_top_falls_from_the_cut() {
        let mut falls = Falls::new(Tier::Comfort);
        let body = falls.spawn(facing_up(), half(), [0.0; 3]).expect("spawn");
        for _ in 0..30 {
            falls.step(FX_DT);
        }
        let pose = falls.pose(&body, 1.0).expect("pose");
        println!("fall: 30 ticks drop {:.2} m", -pose.offset[1]);
        assert!(
            pose.offset[1] < -1.0,
            "the half falls, not hangs: {}",
            pose.offset[1]
        );
    }

    /// It lands on the sand and stops there: the ground is a real contact.
    #[test]
    fn a_cut_top_lands_on_the_sand_and_stays_above_it() {
        let mut falls = Falls::new(Tier::Comfort);
        let at = facing_up();
        let body = falls.spawn(at, half(), [0.0; 3]).expect("spawn");
        for _ in 0..240 {
            falls.step(FX_DT);
        }
        let pose = falls.pose(&body, 1.0).expect("pose");
        let centre_y = at.pos[1] + pose.offset[1];
        println!("fall: landed centre y {centre_y:.3} m");
        assert!(
            centre_y > FLOOR_TOP_Y,
            "the sand holds it above the floor: {centre_y}"
        );
        assert!(
            centre_y < FLOOR_TOP_Y + 1.5,
            "it comes to rest near the sand, not on top of a stack: {centre_y}"
        );
    }

    /// A standing tower beside the drop stops the half: it never reaches the sand there.
    #[test]
    fn a_standing_tower_stops_the_falling_half() {
        // A tower whose top is 2 m up, right under the drop.
        let mut blocked = Falls::new(Tier::Comfort);
        blocked.add_tower([0.6, 2.0, 0.6], [0.0, 2.0, 0.0]);
        let at = facing_up();
        let on_tower = blocked.spawn(at, half(), [0.0; 3]).expect("spawn");
        for _ in 0..240 {
            blocked.step(FX_DT);
        }
        let pose = blocked.pose(&on_tower, 1.0).expect("pose");
        let centre_y = at.pos[1] + pose.offset[1];
        println!("fall: over a tower the half rests at y {centre_y:.3} m");
        assert!(
            centre_y > FLOOR_TOP_Y + 2.0,
            "the tower holds it at its top ({centre_y}), not the sand"
        );

        // The same drop with no tower reaches the sand.
        let mut open = Falls::new(Tier::Comfort);
        let clear = open.spawn(at, half(), [0.0; 3]).expect("spawn");
        for _ in 0..240 {
            open.step(FX_DT);
        }
        let low = at.pos[1] + open.pose(&clear, 1.0).expect("pose").offset[1];
        assert!(
            low < centre_y - 1.0,
            "with nothing in the way it lands lower ({low} vs {centre_y})"
        );
    }

    /// The same spawn twice gives the same poses byte for byte.
    #[test]
    fn the_same_fall_twice_is_byte_equal() {
        let run = || {
            let mut falls = Falls::new(Tier::Comfort);
            let body = falls
                .spawn(facing_up(), half(), [1.5, 0.0, 0.0])
                .expect("spawn");
            let mut bytes = Vec::new();
            for _ in 0..90 {
                falls.step(FX_DT);
                let pose = falls.pose(&body, 1.0).expect("pose");
                for v in pose
                    .offset
                    .iter()
                    .chain(pose.axis.iter())
                    .chain([&pose.angle])
                {
                    bytes.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            bytes
        };
        assert_eq!(run(), run());
    }
}
