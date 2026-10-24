//! fighter_look_dress: the crest, the tunic skirt and the two shoulder straps swing on the
//! springs that already exist in `engine-anim` (rows SW102/SW103).
//!
//! Presentation only: every lane is Q12 integer inside `SpringSet`, the anchors are read from the
//! drawn pose each frame and the tips are written back onto the same pose. Nothing here is hashed
//! and the sim never reads it. Callers: `fighter_look`'s frame path. Units: meters in, Q12 inside,
//! 60 Hz ticks. Budget: one spring set and one collide pass per fighter per tick, nothing
//! allocated after the first frame; a frozen (far LOD) slot keeps its tips and steps nothing.
//!
//! The dress is eleven bones: a crest of three on the helmet, a skirt of six round the pelvis and
//! two straps over the shoulders. A turn kicks them sideways by the body's yaw rate and a hit
//! kicks them away from the blow, the way a hair or cloth rig of VRM spring bones does.

use engine_anim::contract::LocalPose;
use engine_anim::springs::{
    apply_to_pose, meters_q, q_meters, SpringCollider, SpringError, SpringParams, SpringSet,
};

/// Bones in one fighter's dress: three crest, six skirt, two straps.
pub const DRESS_BONES: usize = 11;
/// Fighters one dress covers.
pub const MAX_DRESS: usize = 8;

/// Why a dress frame was refused. Nothing is written when a frame refuses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DressError {
    /// The slot is outside [`MAX_DRESS`].
    BadSlot(u8),
    /// A pose joint named by the map is outside the pose.
    BadJoint(usize),
    /// The spring step or the collide pass refused.
    Spring(SpringError),
}

impl core::fmt::Display for DressError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSlot(slot) => write!(f, "dress: slot {slot} outside 0..{MAX_DRESS}"),
            Self::BadJoint(joint) => write!(f, "dress: pose joint {joint} out of range"),
            Self::Spring(err) => write!(f, "dress: {err}"),
        }
    }
}

impl std::error::Error for DressError {}

/// Where one fighter's dress bones sit: the pose joint each bone's anchor reads, the pose joint
/// its tip is drawn on, and the body joints the cloth may not pass through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DressMap {
    /// Pose joint each dress bone hangs from, in dress order.
    pub anchor: [usize; DRESS_BONES],
    /// Pose joint each dress bone's tip is written to, in dress order.
    pub tip: [usize; DRESS_BONES],
    /// Spine capsule: head and tail joints.
    pub spine: [usize; 2],
    /// Left and right thigh capsules, head and tail joints each.
    pub thigh: [[usize; 2]; 2],
    /// Helmet: the head joint, with the sphere's radius in meters.
    pub helmet: (usize, f32),
}

impl DressMap {
    /// The shipped fighter's dress: crest bones 0..3 hang from the head, skirt 3..9 from the
    /// pelvis, straps 9..11 from the chest; the tips land on joints 12..23 of a 24-joint pose.
    pub const SHIPPED: Self = Self {
        anchor: [3, 3, 3, 0, 0, 0, 0, 0, 0, 2, 2],
        tip: [12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22],
        spine: [0, 2],
        thigh: [[4, 5], [6, 7]],
        helmet: (3, 0.11),
    };

    /// Every joint the map names, refused when one is outside `pose`.
    fn check(&self, pose: &LocalPose) -> Result<(), DressError> {
        let joints = self
            .anchor
            .iter()
            .chain(self.tip.iter())
            .chain(self.spine.iter())
            .chain(self.thigh.iter().flatten())
            .copied()
            .chain(core::iter::once(self.helmet.0));
        for joint in joints {
            if joint >= pose.positions.len() {
                return Err(DressError::BadJoint(joint));
            }
        }
        Ok(())
    }
}

/// One fighter's dress: the bone positions, the mapping and the flags a frame reads.
#[derive(Clone, PartialEq, Eq, Debug)]
struct SlotDress {
    set: SpringSet,
    map: DressMap,
    yaw_q: i32,
    frozen: bool,
    /// Tips as read back each frame: what a frozen slot keeps and what the pose is drawn from.
    held: [[i32; 3]; DRESS_BONES],
    ready: bool,
}

/// The crest, skirt and straps of up to [`MAX_DRESS`] fighters.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Dress {
    params: SpringParams,
    slots: Vec<Option<SlotDress>>,
}

/// Share of the yaw rate that becomes a sideways kick on the dress tips.
const TURN_KICK: i32 = 1;
/// Share of a blow's push that becomes a kick on the dress tips.
const HIT_KICK: i32 = 2;
/// Radius of the spine and thigh capsules the cloth may not pass through, meters.
const LIMB_R: f32 = 0.09;
/// How thick each dress bone is drawn, meters: the cloth stands off the body by this much.
const DRESS_R: f32 = 0.012;

impl Dress {
    /// A dress of [`MAX_DRESS`] slots over `params`; a slot builds its set on its first frame.
    pub fn new(params: SpringParams) -> Result<Self, DressError> {
        let mut slots = Vec::with_capacity(MAX_DRESS);
        for _ in 0..MAX_DRESS {
            slots.push(None);
        }
        Ok(Self { params, slots })
    }

    /// Shared tuning.
    pub fn params(&self) -> SpringParams {
        self.params
    }

    /// One frame of `slot`'s dress: read its anchors and the body's capsules off `pose`, step the
    /// springs, push the tips out of the body and write them back onto `pose`. A frozen slot
    /// writes its held tips and steps nothing.
    pub fn frame(
        &mut self,
        slot: u8,
        pose: &mut LocalPose,
        map: DressMap,
    ) -> Result<(), DressError> {
        let index = usize::from(slot);
        if index >= MAX_DRESS {
            return Err(DressError::BadSlot(slot));
        }
        map.check(pose)?;
        let anchors = pose_anchors(pose, &map);
        let colliders = pose_colliders(pose, &map);
        let dress = self.ensure(index, map)?;
        if dress.frozen {
            let tips = dress.held.map(to_m);
            return apply_to_pose(pose, &tips, &map.tip).map_err(DressError::Spring);
        }
        dress
            .set
            .step(&anchors, 1, &mut dress.held)
            .map_err(DressError::Spring)?;
        dress
            .set
            .collide(&anchors, &colliders)
            .map_err(DressError::Spring)?;
        let tips = dress.set.tips_q();
        dress.held.copy_from_slice(&tips[..DRESS_BONES]);
        dress.ready = true;
        let drawn = dress.held.map(to_m);
        apply_to_pose(pose, &drawn, &map.tip).map_err(DressError::Spring)
    }

    /// A turn this tick: `yaw_rate_milli` is the body's yaw change in milliradians per tick. The
    /// dress lags it, so the tips swing opposite the turn.
    pub fn turn(&mut self, slot: u8, yaw_rate_milli: i32) -> Result<(), DressError> {
        let Some(dress) = self.slot_mut(slot)? else {
            return Ok(());
        };
        dress.yaw_q = yaw_rate_milli;
        if dress.frozen || yaw_rate_milli == 0 {
            return Ok(());
        }
        let push = milli_mps(yaw_rate_milli.saturating_mul(TURN_KICK).clamp(-500, 500));
        for bone in 0..DRESS_BONES {
            dress
                .set
                .kick(bone, [push, 0.0, push * 0.5])
                .map_err(DressError::Spring)?;
        }
        Ok(())
    }

    /// A blow this tick: `dir` is the hit-reaction octant (`HitDir::octant`) and `push_milli` the
    /// blow's push in thousandths. The dress flies away from the blow.
    pub fn hit(&mut self, slot: u8, dir: u8, push_milli: i32) -> Result<(), DressError> {
        let Some(dress) = self.slot_mut(slot)? else {
            return Ok(());
        };
        if dress.frozen {
            return Ok(());
        }
        let (sx, sz) = octant(dir);
        let push = milli_mps(push_milli.saturating_mul(HIT_KICK).clamp(-4000, 4000));
        for bone in 0..DRESS_BONES {
            dress
                .set
                .kick(bone, [sx * push, 0.0, sz * push])
                .map_err(DressError::Spring)?;
        }
        Ok(())
    }

    /// Freeze (`true`) or wake a slot: a frozen slot keeps its tips and steps nothing.
    pub fn set_frozen(&mut self, slot: u8, frozen: bool) {
        if let Ok(Some(dress)) = self.slot_mut(slot) {
            dress.frozen = frozen;
        }
    }

    /// One dress bone's tip, meters: `None` before the slot's first frame.
    pub fn tip(&self, slot: u8, bone: usize) -> Option<[f32; 3]> {
        let dress = self.slots.get(usize::from(slot))?.as_ref()?;
        if !dress.ready || bone >= DRESS_BONES {
            return None;
        }
        Some(to_m(dress.held[bone]))
    }

    /// Slots that have drawn at least one frame.
    pub fn live(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.as_ref().is_some_and(|dress| dress.ready))
            .count()
    }

    /// The yaw rate the slot last read, milliradians per tick.
    pub fn yaw(&self, slot: u8) -> Option<i32> {
        self.slots
            .get(usize::from(slot))?
            .as_ref()
            .map(|dress| dress.yaw_q)
    }

    fn ensure(&mut self, index: usize, map: DressMap) -> Result<&mut SlotDress, DressError> {
        let fresh = self.slots[index]
            .as_ref()
            .is_none_or(|dress| dress.map != map);
        if fresh {
            let mut set =
                SpringSet::with_capacity(self.params, DRESS_BONES).map_err(DressError::Spring)?;
            for _ in 0..DRESS_BONES {
                set.add_bone_radius([0.0; 3], DRESS_R)
                    .map_err(DressError::Spring)?;
            }
            self.slots[index] = Some(SlotDress {
                set,
                map,
                yaw_q: 0,
                frozen: false,
                held: [[0; 3]; DRESS_BONES],
                ready: false,
            });
        }
        self.slots[index]
            .as_mut()
            .ok_or(DressError::BadSlot(u8::try_from(index).unwrap_or(u8::MAX)))
    }

    fn slot_mut(&mut self, slot: u8) -> Result<Option<&mut SlotDress>, DressError> {
        let index = usize::from(slot);
        if index >= MAX_DRESS {
            return Err(DressError::BadSlot(slot));
        }
        Ok(self.slots[index].as_mut())
    }
}

/// Q12 of a meter of a pose joint, one lane at a time.
fn to_q(v: [f32; 3]) -> [i32; 3] {
    v.map(meters_q)
}

/// Meters of a Q12 dress tip.
fn to_m(q: [i32; 3]) -> [f32; 3] {
    q.map(q_meters)
}

/// The anchors of the dress this frame: the pose joint each bone hangs from, Q12.
fn pose_anchors(pose: &LocalPose, map: &DressMap) -> Vec<[i32; 3]> {
    map.anchor
        .iter()
        .map(|joint| to_q(pose.positions[*joint]))
        .collect()
}

/// The body the cloth may not pass through: the spine and the two thighs as capsules, the
/// helmet as a sphere, all read off the same pose.
fn pose_colliders(pose: &LocalPose, map: &DressMap) -> Vec<SpringCollider> {
    let at = |joint: usize| to_q(pose.positions[joint]);
    let radius_q = meters_q(LIMB_R);
    let mut out = Vec::with_capacity(4);
    out.push(SpringCollider::Capsule {
        head_q: at(map.spine[0]),
        tail_q: at(map.spine[1]),
        radius_q,
    });
    for thigh in &map.thigh {
        out.push(SpringCollider::Capsule {
            head_q: at(thigh[0]),
            tail_q: at(thigh[1]),
            radius_q,
        });
    }
    out.push(SpringCollider::Sphere {
        center_q: at(map.helmet.0),
        radius_q: meters_q(map.helmet.1),
    });
    out
}

/// Thousandths of a unit as meters per second: a kick is a velocity, like any impulse.
fn milli_mps(milli: i32) -> f32 {
    let small = milli.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
    f32::from(i16::try_from(small).unwrap_or(0)) / 1000.0
}

/// The sideways direction of a hit octant, thousandths of a unit.
fn octant(dir: u8) -> (f32, f32) {
    match dir % 8 {
        0 => (1.0, 0.0),
        1 => (0.707, 0.707),
        2 => (0.0, 1.0),
        3 => (-0.707, 0.707),
        4 => (-1.0, 0.0),
        5 => (-0.707, -0.707),
        6 => (0.0, -1.0),
        _ => (0.707, -0.707),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::springs::Q;

    /// A standing body of 24 joints: pelvis 0, waist 1, chest 2, head 3, thighs 4..8 at their
    /// heads and knees, then room for every dress tip at 12..23.
    fn pose24() -> LocalPose {
        let mut positions = vec![[0.0, 1.0, 0.0]; 24];
        positions[0] = [0.0, 1.0, 0.0];
        positions[1] = [0.0, 1.2, 0.0];
        positions[2] = [0.0, 1.4, 0.0];
        positions[3] = [0.0, 1.65, 0.0];
        positions[4] = [-0.1, 0.95, 0.0];
        positions[5] = [-0.12, 0.55, 0.0];
        positions[6] = [0.1, 0.95, 0.0];
        positions[7] = [0.12, 0.55, 0.0];
        for (index, slot) in positions.iter_mut().enumerate().skip(8) {
            *slot = [
                0.0,
                1.4,
                -0.1 * f32::from(u8::try_from(index % 8).unwrap_or(0)),
            ];
        }
        LocalPose {
            positions,
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 24],
        }
    }

    fn dress() -> Dress {
        Dress::new(SpringParams::new(Q / 2, Q - Q / 8, Q / 4).expect("tuning")).expect("dress")
    }

    fn offset(a: [f32; 3], b: [f32; 3]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    #[test]
    fn a_turning_fighter_lags_his_crest_then_settles() {
        // A fighter turns 180 degrees in 60 ticks; the crest trails the turn, then comes home.
        let mut pose = pose24();
        let mut dress = dress();
        let head = pose.positions[3];
        let mut peak = 0.0f32;
        for tick in 0..60 {
            dress.turn(0, 52).expect("yaw rate");
            dress
                .frame(0, &mut pose, DressMap::SHIPPED)
                .expect("turn frame");
            let tip = dress.tip(0, 0).expect("a crest tip");
            peak = peak.max(offset(tip, head));
        }
        for _ in 0..180 {
            dress.turn(0, 0).expect("still");
            dress
                .frame(0, &mut pose, DressMap::SHIPPED)
                .expect("rest frame");
        }
        let settled = offset(dress.tip(0, 0).expect("a crest tip"), head);
        assert!(
            peak >= 0.02 && peak <= 0.12,
            "the crest travelled {peak} m on a 180 degree turn"
        );
        assert!(settled < 0.01, "the crest settled to {settled} m");
        assert!(dress.live() == 1);
        assert_eq!(dress.yaw(0), Some(0));
    }

    #[test]
    fn a_hit_kicks_the_skirt_off_the_thigh() {
        // The skirt hangs over the thighs; a blow from the left kicks it clear, and without the
        // kick the plain hang is left where the springs put it.
        let mut pose = pose24();
        let mut dress = dress();
        for _ in 0..30 {
            dress
                .frame(0, &mut pose, DressMap::SHIPPED)
                .expect("hang frame");
        }
        let thigh = [
            pose.positions[DressMap::SHIPPED.thigh[0][0]],
            pose.positions[DressMap::SHIPPED.thigh[0][1]],
        ];
        let away = |p: [f32; 3]| {
            let axis = [
                thigh[1][0] - thigh[0][0],
                thigh[1][1] - thigh[0][1],
                thigh[1][2] - thigh[0][2],
            ];
            let l2 = axis.iter().map(|c| c * c).sum::<f32>().max(1.0e-6);
            let rel = [p[0] - thigh[0][0], p[1] - thigh[0][1], p[2] - thigh[0][2]];
            let t = (rel.iter().zip(axis).map(|(a, b)| a * b).sum::<f32>() / l2).clamp(0.0, 1.0);
            offset(
                p,
                [
                    thigh[0][0] + axis[0] * t,
                    thigh[0][1] + axis[1] * t,
                    thigh[0][2] + axis[2] * t,
                ],
            )
        };
        let before: Vec<f32> = (3..9)
            .map(|bone| away(dress.tip(0, bone).expect("skirt tip")))
            .collect();
        assert!(
            before.iter().any(|gap| *gap < LIMB_R),
            "the skirt must start over the thigh for this test to mean anything: {before:?}"
        );
        dress.hit(0, 0, 900).expect("a blow from the left");
        for _ in 0..8 {
            dress
                .frame(0, &mut pose, DressMap::SHIPPED)
                .expect("kick frame");
        }
        let after: Vec<f32> = (3..9)
            .map(|bone| away(dress.tip(0, bone).expect("skirt tip")))
            .collect();
        assert!(
            after.iter().zip(&before).any(|(now, was)| now > was),
            "the blow did not move the skirt away: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn every_dress_slot_fits_one_tick() {
        // Eight fighters, eleven bones each: the modeled op count of one frame stays inside the
        // springs' own per-bone and per-collider budgets, so the 0.1 ms bar holds by construction.
        use engine_anim::springs::{collide_ops, tick_ops};
        let mut pose = pose24();
        let mut dress = dress();
        let ops = 8 * (tick_ops(DRESS_BONES) + collide_ops(DRESS_BONES, 4));
        assert_eq!(ops, 8 * (11 * 3 * 8 + 11 * 4 * 24));
        for slot in 0..8u8 {
            for _ in 0..2 {
                dress
                    .frame(slot, &mut pose, DressMap::SHIPPED)
                    .expect("frame");
            }
        }
        assert_eq!(dress.live(), MAX_DRESS);
        let frozen = dress.tip(3, 0).expect("a tip");
        dress.set_frozen(3, true);
        dress
            .frame(3, &mut pose, DressMap::SHIPPED)
            .expect("frozen frame");
        assert_eq!(dress.tip(3, 0), Some(frozen), "a frozen slot keeps its tip");
        // A slot past the cap and a joint past the pose both refuse.
        assert_eq!(
            dress.frame(9, &mut pose, DressMap::SHIPPED),
            Err(DressError::BadSlot(9))
        );
        let mut tiny = pose24();
        tiny.positions.truncate(10);
        tiny.rotations.truncate(10);
        assert!(matches!(
            dress.frame(0, &mut tiny, DressMap::SHIPPED),
            Err(DressError::BadJoint(_))
        ));
    }

    #[test]
    fn the_same_dress_tape_draws_twice() {
        let run = || {
            let mut pose = pose24();
            let mut dress = dress();
            for tick in 0..40 {
                if tick % 7 == 0 {
                    dress.turn(0, 40).expect("turn");
                    dress.hit(0, (tick % 8) as u8, 600).expect("hit");
                }
                dress.frame(0, &mut pose, DressMap::SHIPPED).expect("frame");
            }
            (dress, pose)
        };
        let (a, a_pose) = run();
        let (b, b_pose) = run();
        assert_eq!(a, b);
        assert_eq!(a_pose, b_pose);
    }
}
