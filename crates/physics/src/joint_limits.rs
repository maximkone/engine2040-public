//! joint_limits: per-bone swing and twist limits, friction motors and collision groups
//! derived from the skeleton.
//!
//! The ragdoll joint set used to take one swing limit from the joint's depth in the
//! hierarchy. A name carries more than a depth does: a knee bends about one axis, an
//! elbow folds the other way, a spine turns less than a shoulder. `JointTable` holds
//! one `JointSpec` per bone, named from the skeleton (`knee`, `elbow`, `spine`,
//! `shoulder`, `hip` and their synonyms), with the depth limit as the fallback for a
//! bone the table does not know. A knee or an elbow gets `JointKind::Hinge`; every
//! other named bone gets `JointKind::Ball` with its own cone and twist.
//!
//! Two refusals keep a broken rig out of the solver: `JointSpec::ball` and
//! `JointSpec::hinge` refuse a limit in degrees outside `0..=180`, and
//! `JointTable::try_humanoid` refuses a skeleton whose parent links are not exactly
//! one tree (a bone without a joint flies apart under gravity). `bone_groups` builds
//! the collision groups that keep a bone from touching its parent or children while
//! the same bone of two ragdolls still collides.
//!
//! Values: radians for angles, newton-meters for torque, hertz for springs. The table
//! converts the degree rows once at spawn with a single constant multiply; nothing here
//! calls platform trig. The Rapier ragdoll is client-local and never hashed
//! (HASH-LAW "Ragdoll split"), so none of this reaches the golden.
#![forbid(unsafe_code)]

use super::ragdoll::{joint_depths, swing_limit};
use engine_anim::contract::Skeleton;
use rapier3d::geometry::{Group, InteractionGroups, InteractionTestMode};
use std::fmt::{Display, Formatter};

/// Degrees to radians. A constant ratio, not a platform function.
const DEG: f32 = 0.017_453_292;

/// Friction motor torque for hinge joints (knees, elbows), newton-meters.
pub const JOINT_FRICTION_HINGE_NM: f32 = 10.0;
/// Friction motor torque for ball joints, newton-meters.
pub const JOINT_FRICTION_BALL_NM: f32 = 5.0;
/// Spring rate for hinge joints, hertz.
pub const JOINT_SPRING_HINGE_HZ: f32 = 2.0;
/// Spring rate for ball joints, hertz.
pub const JOINT_SPRING_BALL_HZ: f32 = 0.5;
/// Symmetric twist half-range for ball joints, radians.
pub const JOINT_TWIST_BALL_RAD: f32 = 0.8;
/// Widest limit any joint may carry: a half turn, radians.
pub const JOINT_LIMIT_MAX_RAD: f32 = core::f32::consts::PI;
/// Widest limit any joint may carry, degrees.
pub const JOINT_LIMIT_MAX_DEG: f32 = 180.0;
/// Bones one collision-group bit can name.
pub const GROUP_BITS: usize = 32;

/// Per-bone joint kind: a hinge bends one way (knees, elbows), a ball cones plus twists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    /// Single-axis hinge: flexion in `[swing_min, swing_limit]` about the side axis,
    /// no twist.
    Hinge,
    /// Ball joint: swing cone plus symmetric twist.
    Ball,
}

/// Per-bone joint spec: kind plus limits plus drive tuning. Units: radians, N.m, Hz.
/// Determinism: plain values, read at spawn only, never hashed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointSpec {
    /// Hinge or ball.
    pub kind: JointKind,
    /// Cone half-angle about the joint's forward axis (ball), or the flexion high end
    /// (hinge), radians, in `[0, JOINT_LIMIT_MAX_RAD]`.
    pub swing_limit: f32,
    /// Cone half-angle about the joint's side axis (ball), radians; a hinge locks it
    /// to `0`.
    pub swing_lateral: f32,
    /// Flexion low end (hinge), radians; a ball keeps `0`.
    pub swing_min: f32,
    /// Twist range low, radians; hinge locks to `0`.
    pub twist_min: f32,
    /// Twist range high, radians; hinge locks to `0`.
    pub twist_max: f32,
    /// Friction motor torque, N.m, in `5..=10`.
    pub friction_torque: f32,
    /// Spring rate, Hz, in `0.5..=2`.
    pub spring_hz: f32,
}

impl JointSpec {
    /// Ball joint from degrees: `swing_deg` about the forward axis, `lateral_deg` about
    /// the side axis and a symmetric `twist_deg`. Refuses a non-finite number or one
    /// outside `0..=180`.
    pub fn ball(swing_deg: f32, lateral_deg: f32, twist_deg: f32) -> Result<Self, JointError> {
        check_limit(swing_deg)?;
        check_limit(lateral_deg)?;
        check_limit(twist_deg)?;
        Ok(Self {
            kind: JointKind::Ball,
            swing_limit: swing_deg * DEG,
            swing_lateral: lateral_deg * DEG,
            swing_min: 0.0,
            twist_min: -twist_deg * DEG,
            twist_max: twist_deg * DEG,
            friction_torque: JOINT_FRICTION_BALL_NM,
            spring_hz: JOINT_SPRING_BALL_HZ,
        })
    }

    /// Hinge joint from degrees: flexion runs `min_deg..=max_deg` about one axis.
    /// Refuses a number outside `0..=180` and a range whose low end sits above its
    /// high end.
    pub fn hinge(min_deg: f32, max_deg: f32) -> Result<Self, JointError> {
        check_limit(min_deg)?;
        check_limit(max_deg)?;
        if min_deg > max_deg {
            return Err(JointError::ReversedHinge {
                min: min_deg,
                max: max_deg,
            });
        }
        Ok(Self {
            kind: JointKind::Hinge,
            swing_limit: max_deg * DEG,
            swing_lateral: 0.0,
            swing_min: min_deg * DEG,
            twist_min: 0.0,
            twist_max: 0.0,
            friction_torque: JOINT_FRICTION_HINGE_NM,
            spring_hz: JOINT_SPRING_HINGE_HZ,
        })
    }

    /// Depth fallback for a bone the table does not know: a ball with the same swing on
    /// both cone axes and the constant ball twist.
    pub(crate) fn fallback(swing: f32) -> Self {
        Self {
            kind: JointKind::Ball,
            swing_limit: swing,
            swing_lateral: swing,
            swing_min: 0.0,
            twist_min: -JOINT_TWIST_BALL_RAD,
            twist_max: JOINT_TWIST_BALL_RAD,
            friction_torque: JOINT_FRICTION_BALL_NM,
            spring_hz: JOINT_SPRING_BALL_HZ,
        }
    }

    /// Clamp a hinge flexion angle into `[swing_min, swing_limit]`. Non-finite input
    /// maps to `swing_min`. Math is comparisons only, deterministic on every platform.
    pub fn clamp_hinge(&self, angle: f32) -> f32 {
        if !angle.is_finite() || !self.swing_limit.is_finite() {
            return self.swing_min;
        }
        if angle < self.swing_min {
            self.swing_min
        } else if angle > self.swing_limit {
            self.swing_limit
        } else {
            angle
        }
    }

    /// Clamp a twist angle into `[twist_min, twist_max]`. Non-finite input maps
    /// to `twist_min`. Comparisons only, deterministic on every platform.
    pub fn clamp_twist(&self, angle: f32) -> f32 {
        if !angle.is_finite() {
            return self.twist_min;
        }
        if angle < self.twist_min {
            self.twist_min
        } else if angle > self.twist_max {
            self.twist_max
        } else {
            angle
        }
    }

    /// True when every limit is finite and inside `0..=JOINT_LIMIT_MAX_RAD`, the range
    /// matches the kind (a ball keeps `swing_min == 0`, a hinge keeps a lateral `0`),
    /// the twist range is not reversed and the motor numbers are positive.
    pub fn is_valid(&self) -> bool {
        let in_range =
            |value: f32| value.is_finite() && (0.0..=JOINT_LIMIT_MAX_RAD).contains(&value);
        let kind_ok = match self.kind {
            JointKind::Hinge => self.swing_min <= self.swing_limit && self.swing_lateral == 0.0,
            JointKind::Ball => self.swing_min == 0.0,
        };
        kind_ok
            && in_range(self.swing_limit)
            && in_range(self.swing_lateral)
            && in_range(self.swing_min)
            && self.twist_min.is_finite()
            && self.twist_max.is_finite()
            && self.twist_min <= self.twist_max
            && self.twist_min >= -JOINT_LIMIT_MAX_RAD
            && self.twist_max <= JOINT_LIMIT_MAX_RAD
            && self.friction_torque.is_finite()
            && self.friction_torque > 0.0
            && self.spring_hz.is_finite()
            && self.spring_hz > 0.0
    }
}

/// A refused joint limit or joint graph. User input never panics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointError {
    /// A limit in degrees was non-finite or outside `0..=180`.
    LimitOutOfRange {
        /// The refused number, degrees.
        degrees: f32,
    },
    /// A hinge whose low end sits above its high end, degrees.
    ReversedHinge {
        /// Flexion low end, degrees.
        min: f32,
        /// Flexion high end, degrees.
        max: f32,
    },
    /// The parent links leave a rig bone without a joint: `bodies` bones carry
    /// `links` links, so a bone the table names stands on its own and flies apart
    /// under gravity.
    JointCount {
        /// Bones in the skeleton.
        bodies: usize,
        /// Bones that carry a parent link.
        links: usize,
    },
}

impl Display for JointError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            JointError::LimitOutOfRange { degrees } => {
                write!(f, "joint limit {degrees} out of 0..=180 degrees")
            }
            JointError::ReversedHinge { min, max } => {
                write!(f, "reversed hinge: {min} > {max} degrees")
            }
            JointError::JointCount { bodies, links } => write!(
                f,
                "{bodies} bones need {} joints, found {links}",
                bodies.saturating_sub(1)
            ),
        }
    }
}

impl std::error::Error for JointError {}

/// Refuse a number of degrees outside `0..=180`.
fn check_limit(degrees: f32) -> Result<(), JointError> {
    if !degrees.is_finite() || !(0.0..=JOINT_LIMIT_MAX_DEG).contains(&degrees) {
        return Err(JointError::LimitOutOfRange { degrees });
    }
    Ok(())
}

/// Refuse a skeleton whose parent links leave a named rig bone unjoined. A full
/// skeleton has one link per bone but the root, so `bodies - 1` links is one tree
/// and passes at once. Other roots are free bodies and a scene may spawn them (a
/// blade and a limb that never touch). A root the table names is a bone torn off
/// the humanoid rig, and nothing joins it again.
pub fn check_joint_count(skeleton: &Skeleton) -> Result<(), JointError> {
    let bodies = skeleton.joints.len();
    let links = skeleton
        .joints
        .iter()
        .filter(|joint| joint.parent.is_some())
        .count();
    if links == bodies.saturating_sub(1) {
        return Ok(());
    }
    let mut roots = skeleton
        .joints
        .iter()
        .filter(|joint| joint.parent.is_none());
    // The first root owns the body; a later root the table names is a detached bone.
    let _ = roots.next();
    if roots.any(|joint| named_limits(&joint.name).is_some()) {
        return Err(JointError::JointCount { bodies, links });
    }
    Ok(())
}

/// Per-bone joint table: one `JointSpec` per bone in skeleton order (the root row
/// carries the defaults and is unused, its parent is `None`).
#[derive(Clone, Debug, PartialEq)]
pub struct JointTable {
    /// One entry per bone, skeleton order.
    pub rows: Vec<JointSpec>,
}

/// A named row of the table, in degrees: kind plus three numbers. A ball carries
/// `[swing, lateral, twist]`; a hinge carries `[min, max, 0]`.
type NamedRow = (JointKind, [f32; 3]);

impl JointTable {
    /// Table from the skeleton's joint names. A hinge name (a knee, an elbow) gets
    /// `JointKind::Hinge` with a 1..145 degree flexion range; a spine, chest, neck,
    /// head, shoulder, upper arm, hip or thigh gets its named cone and twist; every
    /// other bone falls back to the depth limit it had before. Allocates once for the
    /// table (never per tick); math is comparisons plus one constant multiply. A row
    /// the published numbers cannot express keeps the depth fallback;
    /// `JointTable::try_humanoid` reports that instead of hiding it.
    pub fn humanoid(skeleton: &Skeleton) -> Self {
        let depths = joint_depths(skeleton);
        let mut rows = Vec::with_capacity(skeleton.len());
        for (i, joint) in skeleton.joints.iter().enumerate() {
            let depth = depths.get(i).copied().unwrap_or(0);
            let fallback = swing_limit(depth);
            rows.push(
                named_row(&joint.name, fallback).unwrap_or_else(|_| JointSpec::fallback(fallback)),
            );
        }
        Self { rows }
    }

    /// The same table, refusing a skeleton whose parent links leave a named rig bone
    /// unjoined and a row that breaks `JointSpec::ball` or `JointSpec::hinge`.
    /// `spawn` reads this one, so a detached bone stops before the solver sees it.
    pub fn try_humanoid(skeleton: &Skeleton) -> Result<Self, JointError> {
        check_joint_count(skeleton)?;
        let depths = joint_depths(skeleton);
        let mut rows = Vec::with_capacity(skeleton.len());
        for (i, joint) in skeleton.joints.iter().enumerate() {
            let depth = depths.get(i).copied().unwrap_or(0);
            rows.push(named_row(&joint.name, swing_limit(depth))?);
        }
        Ok(Self { rows })
    }

    /// Row count (one per bone).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Spec for one bone, `None` past the end.
    pub fn spec(&self, bone: usize) -> Option<JointSpec> {
        self.rows.get(bone).copied()
    }

    /// True when every row passes [`JointSpec::is_valid`].
    pub fn is_valid(&self) -> bool {
        self.rows.iter().all(JointSpec::is_valid)
    }
}

/// The named row for one joint, or the depth fallback.
fn named_row(name: &str, fallback: f32) -> Result<JointSpec, JointError> {
    match named_limits(name) {
        Some((JointKind::Hinge, [min, max, _])) => JointSpec::hinge(min, max),
        Some((JointKind::Ball, [swing, lateral, twist])) => JointSpec::ball(swing, lateral, twist),
        None => Ok(JointSpec::fallback(fallback)),
    }
}

/// Limits by joint name, in degrees, with the measurement that set each one. These
/// are the humanoid rows proven on a ragdoll wizard: spine 20/20/35, arm ball
/// 85/85/70, forearm and leg hinge 1/145, thigh ball 70/40/30, head 35/35/45. A ball
/// row reads `[swing, lateral, twist]`, a hinge row `[min, max, 0]`. `None` means the
/// table does not know the name.
fn named_limits(name: &str) -> Option<NamedRow> {
    let has = |needle: &str| name.contains(needle);
    if has("knee") || has("elbow") || has("shin") || has("lower_arm") || has("forearm") {
        // One axis, 1..145 degrees from the straight limb. 140 was tried and missed
        // the last 5 degrees of a deep squat; 0 let a knee hyperextend.
        return Some((JointKind::Hinge, [1.0, 145.0, 0.0]));
    }
    if has("spine") || has("chest") {
        // 25 locked the torso: a fighter could not turn into a punch.
        return Some((JointKind::Ball, [20.0, 20.0, 35.0]));
    }
    if has("neck") || has("head") {
        return Some((JointKind::Ball, [35.0, 35.0, 45.0]));
    }
    if has("shoulder") || has("upper_arm") || name == "arm" {
        return Some((JointKind::Ball, [85.0, 85.0, 70.0]));
    }
    if has("hip") || has("thigh") {
        // 45 detonated the hips at the first shove; the side cone stays at 40.
        return Some((JointKind::Ball, [70.0, 40.0, 30.0]));
    }
    None
}

/// Collision groups for one bone, or `None` when the skeleton is wider than
/// [`GROUP_BITS`] (then the joint's own `contacts_enabled(false)` is the only filter).
///
/// Every bone is its own membership bit; the filter clears the parent and the children
/// so joined bones never collide, while the same bone of two ragdolls still does: a
/// bone's own bit stays set in its filter, so bone `i` of one ragdoll meets bone `i` of
/// another. Static geometry keeps Rapier's default groups and still collides.
pub fn bone_groups(skeleton: &Skeleton, bone: usize) -> Option<InteractionGroups> {
    let n = skeleton.len();
    if n == 0 || n > GROUP_BITS || bone >= n {
        return None;
    }
    let all: u32 = if n == 32 { u32::MAX } else { (1u32 << n) - 1 };
    let mut clear = 0u32;
    if let Some(parent) = skeleton.joints[bone].parent {
        clear |= 1u32 << parent;
    }
    for (child, joint) in skeleton.joints.iter().enumerate() {
        if joint.parent == Some(bone) {
            clear |= 1u32 << child;
        }
    }
    Some(InteractionGroups::new(
        Group::from_bits_retain(1u32 << bone),
        Group::from_bits_retain(all & !clear),
        InteractionTestMode::And,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::Joint;

    /// The 17-bone humanoid the ragdoll tests use, names only.
    fn humanoid() -> Skeleton {
        let rows: [(Option<usize>, &str); 17] = [
            (None, "pelvis"),
            (Some(0), "spine"),
            (Some(1), "chest"),
            (Some(2), "neck"),
            (Some(3), "head"),
            (Some(2), "shoulder_l"),
            (Some(5), "elbow_l"),
            (Some(6), "wrist_l"),
            (Some(2), "shoulder_r"),
            (Some(8), "elbow_r"),
            (Some(9), "wrist_r"),
            (Some(0), "hip_l"),
            (Some(11), "knee_l"),
            (Some(12), "ankle_l"),
            (Some(0), "hip_r"),
            (Some(14), "knee_r"),
            (Some(15), "ankle_r"),
        ];
        Skeleton {
            joints: rows
                .iter()
                .map(|(parent, name)| Joint {
                    name: (*name).into(),
                    parent: *parent,
                })
                .collect(),
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn named_joints_read_their_table_not_their_depth() {
        let skeleton = humanoid();
        let table = JointTable::humanoid(&skeleton);
        assert_eq!(table.len(), 17);
        assert!(table.is_valid());
        assert_eq!(table, JointTable::humanoid(&skeleton));
        // Named balls in degrees: spine and chest 20/20/35, head 35/35/45,
        // shoulders 85/85/70, hips 70/40/30.
        for (bone, swing, lateral, twist) in [
            (1usize, 20.0f32, 20.0f32, 35.0f32),
            (2, 20.0, 20.0, 35.0),
            (4, 35.0, 35.0, 45.0),
            (5, 85.0, 85.0, 70.0),
            (11, 70.0, 40.0, 30.0),
            (14, 70.0, 40.0, 30.0),
        ] {
            let spec = table.spec(bone).expect("row");
            assert_eq!(spec.kind, JointKind::Ball, "bone {bone}");
            assert!(close(spec.swing_limit, swing * DEG), "bone {bone} swing");
            assert!(
                close(spec.swing_lateral, lateral * DEG),
                "bone {bone} lateral"
            );
            assert!(close(spec.twist_min, -twist * DEG), "bone {bone} twist");
            // The depth limit for these bones is a different number, so the table,
            // not the hierarchy, decided.
            let depth = joint_depths(&skeleton)[bone];
            assert!(
                !close(spec.swing_limit, swing_limit(depth)),
                "bone {bone} must not reuse its depth limit"
            );
        }
        // Knees and elbows hinge on one axis, 1..145 degrees.
        for bone in [6usize, 9, 12, 15] {
            let spec = table.spec(bone).expect("row");
            assert_eq!(spec.kind, JointKind::Hinge, "bone {bone}");
            assert!(close(spec.swing_min, 1.0 * DEG));
            assert!(close(spec.swing_limit, 145.0 * DEG));
            assert_eq!(spec.swing_lateral, 0.0);
            assert_eq!((spec.twist_min, spec.twist_max), (0.0, 0.0));
            assert_eq!(spec.friction_torque, JOINT_FRICTION_HINGE_NM);
            assert_eq!(spec.spring_hz, JOINT_SPRING_HINGE_HZ);
        }
        // A name the table does not know keeps its depth limit on both cone axes.
        let ankle = table.spec(13).expect("row");
        assert_eq!(ankle.kind, JointKind::Ball);
        assert!(close(
            ankle.swing_limit,
            swing_limit(joint_depths(&skeleton)[13])
        ));
        assert_eq!(ankle.swing_lateral, ankle.swing_limit);
        assert_eq!(ankle.swing_min, 0.0);
        // Every motor is inside the published band and every limit is in 0..180 deg.
        for spec in &table.rows {
            assert!((5.0..=10.0).contains(&spec.friction_torque), "{spec:?}");
            assert!((0.5..=2.0).contains(&spec.spring_hz), "{spec:?}");
            assert!(
                (0.0..=JOINT_LIMIT_MAX_RAD).contains(&spec.swing_limit),
                "{spec:?}"
            );
            assert!(
                (0.0..=JOINT_LIMIT_MAX_RAD).contains(&spec.swing_lateral),
                "{spec:?}"
            );
            assert!(spec.swing_min <= spec.swing_limit, "{spec:?}");
        }
        println!("JOINT_TABLE hinges=4 rows=17 named=12 depth_fallback=5");
    }

    #[test]
    fn hinge_names_cover_both_vocabularies() {
        let rows = ["knee_l", "elbow_r", "shin_l", "lower_arm_r", "forearm_l"];
        for name in rows {
            let spec = named_row(name, 0.6).expect("hinge row");
            assert_eq!(spec.kind, JointKind::Hinge, "{name}");
            assert!(spec.is_valid(), "{name}");
        }
        let balls = [
            "spine",
            "chest",
            "neck",
            "head",
            "shoulder_l",
            "upper_arm_r",
            "hip_l",
            "thigh_r",
        ];
        for name in balls {
            let spec = named_row(name, 0.6).expect("ball row");
            assert_eq!(spec.kind, JointKind::Ball, "{name}");
            assert!(spec.swing_limit > 0.0 && spec.is_valid(), "{name}");
        }
        // An unknown name takes the depth fallback on both cone axes.
        let unknown = named_row("tail_1", 0.6).expect("fallback row");
        assert_eq!(unknown.kind, JointKind::Ball);
        assert_eq!((unknown.swing_limit, unknown.swing_lateral), (0.6, 0.6));
    }

    #[test]
    fn joint_table_refuses_a_limit_outside_180_degrees() {
        assert_eq!(
            JointSpec::ball(181.0, 20.0, 35.0),
            Err(JointError::LimitOutOfRange { degrees: 181.0 })
        );
        assert_eq!(
            JointSpec::ball(85.0, -1.0, 70.0),
            Err(JointError::LimitOutOfRange { degrees: -1.0 })
        );
        assert!(matches!(
            JointSpec::hinge(1.0, f32::NAN),
            Err(JointError::LimitOutOfRange { degrees }) if degrees.is_nan()
        ));
        assert_eq!(
            JointSpec::hinge(1.0, f32::INFINITY),
            Err(JointError::LimitOutOfRange {
                degrees: f32::INFINITY
            })
        );
        assert_eq!(
            JointSpec::hinge(145.0, 1.0),
            Err(JointError::ReversedHinge {
                min: 145.0,
                max: 1.0
            })
        );
        // The edge of the band is legal, and so is the knee row.
        let edge = JointSpec::ball(180.0, 0.0, 180.0).expect("180 is inside the band");
        assert!(edge.is_valid());
        let knee = JointSpec::hinge(1.0, 145.0).expect("the knee row");
        assert!(knee.is_valid());
        assert_eq!((knee.swing_min, knee.swing_lateral), (1.0 * DEG, 0.0));
        // A row that sneaks a bad number past the constructor fails the gate.
        let mut broken = JointSpec::ball(20.0, 20.0, 35.0).expect("spine row");
        broken.swing_lateral = f32::NAN;
        assert!(!broken.is_valid());
        println!("JOINT_REFUSAL limits=0..180 reversed_hinge=refused nonfinite=refused");
    }

    #[test]
    fn bone_groups_filter_parent_and_children_only() {
        let skeleton = humanoid();
        let bits = |bone: usize| bone_groups(&skeleton, bone).expect("groups");
        let accepts = |a: usize, b: usize| {
            let (ga, gb) = (bits(a), bits(b));
            (ga.memberships.bits() & gb.filter.bits()) != 0
                && (gb.memberships.bits() & ga.filter.bits()) != 0
        };
        // Joined pairs never touch: chest with its parent spine, its children
        // shoulders and neck.
        for (a, b) in [(2usize, 1usize), (2, 3), (2, 5), (2, 8), (5, 6)] {
            assert!(!accepts(a, b), "joined pair {a}/{b} must not collide");
        }
        // A bone and its grandchildren still touch (contacts stay on).
        assert!(accepts(0, 2), "pelvis and chest must collide");
        // The same bone of two ragdolls still touches: every bone keeps its own bit.
        for bone in 0..skeleton.len() {
            let g = bits(bone);
            assert!(
                (g.memberships.bits() & g.filter.bits()) != 0,
                "bone {bone} must meet its twin"
            );
        }
        // A skeleton wider than the bit budget falls back to joint contacts.
        let wide = Skeleton {
            joints: (0..GROUP_BITS + 1)
                .map(|i| Joint {
                    name: format!("b{i}"),
                    parent: i.checked_sub(1),
                })
                .collect(),
        };
        assert!(bone_groups(&wide, 0).is_none());
        assert!(bone_groups(&skeleton, skeleton.len()).is_none());
    }
}
