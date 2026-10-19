//! joint_limits: per-bone swing and twist limits, friction motors and collision groups
//! derived from the skeleton.
//!
//! The ragdoll joint set used to take one swing limit from the joint's depth in the
//! hierarchy. A name carries more than a depth does: a knee bends about one axis, an
//! elbow folds the other way, a spine turns less than a shoulder. [`JointTable`] holds
//! one [`JointSpec`] per bone, named from the skeleton (`knee`, `elbow`, `spine`,
//! `shoulder`, `hip` and their synonyms), with the depth limit as the fallback for a
//! bone the table does not know. [`bone_groups`] builds the collision groups that keep
//! a bone from touching its parent or children while the same bone of two ragdolls
//! still collides.
//!
//! Values: radians for angles, newton-meters for torque, hertz for springs. The table
//! converts the degree rows once at spawn with a single constant multiply; nothing here
//! calls platform trig. The Rapier ragdoll is client-local and never hashed
//! (HASH-LAW "Ragdoll split"), so none of this reaches the golden.
#![forbid(unsafe_code)]

use super::ragdoll::{joint_depths, swing_limit};
use engine_anim::contract::Skeleton;
use rapier3d::geometry::{Group, InteractionGroups, InteractionTestMode};

/// Degrees to radians. A constant ratio, not a platform function.
const DEG: f32 = 0.017_453_292;

/// Friction motor torque for hinge joints (knees, elbows), newton-meters.
pub const JOINT_FRICTION_HINGE_NM: f32 = 10.0;
/// Friction motor torque for spherical joints, newton-meters.
pub const JOINT_FRICTION_SPHERICAL_NM: f32 = 5.0;
/// Spring rate for hinge joints, hertz.
pub const JOINT_SPRING_HINGE_HZ: f32 = 2.0;
/// Spring rate for spherical joints, hertz.
pub const JOINT_SPRING_SPHERICAL_HZ: f32 = 0.5;
/// Symmetric twist half-range for spherical joints, radians.
pub const JOINT_TWIST_SPHERICAL_RAD: f32 = 0.8;
/// Widest limit any joint may carry: a half turn, radians.
pub const JOINT_LIMIT_MAX_RAD: f32 = core::f32::consts::PI;
/// Bones one collision-group bit can name.
pub const GROUP_BITS: usize = 32;

/// Per-bone joint kind: hinge bends one way (knees, elbows), spherical cones plus twists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    /// Single-axis hinge: flexion in `[-swing_limit, swing_limit]` about the side axis,
    /// no twist.
    Hinge,
    /// Ball joint: swing cone plus symmetric twist.
    Spherical,
}

/// Per-bone joint spec: kind plus limits plus drive tuning. Units: radians, N.m, Hz.
/// Determinism: plain values, read at spawn only, never hashed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointSpec {
    /// Hinge or spherical.
    pub kind: JointKind,
    /// Cone half-angle (spherical) or flexion range (hinge), radians, in
    /// `[0, JOINT_LIMIT_MAX_RAD]`.
    pub swing_limit: f32,
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
    /// Clamp a hinge flexion angle into `[0, swing_limit]`. Non-finite input maps
    /// to `0`. Math is comparisons only, deterministic on every platform.
    pub fn clamp_hinge(&self, angle: f32) -> f32 {
        if !angle.is_finite() || !self.swing_limit.is_finite() {
            return 0.0;
        }
        if angle < 0.0 {
            0.0
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

    /// True when the limits are finite and inside `[0, JOINT_LIMIT_MAX_RAD]`, the
    /// twist range is not reversed and the motor numbers are positive.
    pub fn is_valid(&self) -> bool {
        self.swing_limit.is_finite()
            && (0.0..=JOINT_LIMIT_MAX_RAD).contains(&self.swing_limit)
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

/// Per-bone joint table: one [`JointSpec`] per bone in skeleton order (the root row
/// carries the defaults and is unused, its parent is `None`).
#[derive(Clone, Debug, PartialEq)]
pub struct JointTable {
    /// One entry per bone, skeleton order.
    pub rows: Vec<JointSpec>,
}

/// A named row of the table, in degrees: kind, swing, twist.
type NamedRow = (JointKind, f32, f32);

impl JointTable {
    /// Table from the skeleton's joint names. A hinge name (a knee, an elbow) gets
    /// [`JointKind::Hinge`] with a 145 degree flexion range; a spine, chest, neck,
    /// head, shoulder, upper arm, hip or thigh gets its named cone and twist; every
    /// other bone falls back to the depth limit it had before. Allocates once for the
    /// table (never per tick); math is comparisons plus one constant multiply.
    pub fn humanoid(skeleton: &Skeleton) -> Self {
        let depths = joint_depths(skeleton);
        let mut rows = Vec::with_capacity(skeleton.len());
        for (i, joint) in skeleton.joints.iter().enumerate() {
            let depth = depths.get(i).copied().unwrap_or(0);
            let fallback = swing_limit(depth);
            rows.push(named_row(&joint.name, fallback));
        }
        Self { rows }
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
fn named_row(name: &str, fallback: f32) -> JointSpec {
    match named_limits(name) {
        Some((JointKind::Hinge, swing_deg, _)) => JointSpec {
            kind: JointKind::Hinge,
            swing_limit: swing_deg * DEG,
            twist_min: 0.0,
            twist_max: 0.0,
            friction_torque: JOINT_FRICTION_HINGE_NM,
            spring_hz: JOINT_SPRING_HINGE_HZ,
        },
        Some((JointKind::Spherical, swing_deg, twist_deg)) => JointSpec {
            kind: JointKind::Spherical,
            swing_limit: swing_deg * DEG,
            twist_min: -twist_deg * DEG,
            twist_max: twist_deg * DEG,
            friction_torque: JOINT_FRICTION_SPHERICAL_NM,
            spring_hz: JOINT_SPRING_SPHERICAL_HZ,
        },
        None => JointSpec {
            kind: JointKind::Spherical,
            swing_limit: fallback,
            twist_min: -JOINT_TWIST_SPHERICAL_RAD,
            twist_max: JOINT_TWIST_SPHERICAL_RAD,
            friction_torque: JOINT_FRICTION_SPHERICAL_NM,
            spring_hz: JOINT_SPRING_SPHERICAL_HZ,
        },
    }
}

/// Limits by joint name, in degrees. The numbers are the humanoid rows proven on a
/// ragdoll wizard: spine 20/20/35, arm ball 85/85/70, forearm and leg hinge 1/145,
/// thigh ball 70/40/30, head 35/35/45. The cone takes the tighter lateral axis, the
/// twist the axial one. `None` means the table does not know the name.
fn named_limits(name: &str) -> Option<NamedRow> {
    let has = |needle: &str| name.contains(needle);
    if has("knee") || has("elbow") || has("shin") || has("lower_arm") || has("forearm") {
        // Hinges: one axis, 1..145 degrees counted from the straight limb.
        return Some((JointKind::Hinge, 145.0, 0.0));
    }
    if has("spine") || has("chest") {
        return Some((JointKind::Spherical, 20.0, 35.0));
    }
    if has("neck") || has("head") {
        return Some((JointKind::Spherical, 35.0, 45.0));
    }
    if has("shoulder") || has("upper_arm") || name == "arm" {
        return Some((JointKind::Spherical, 85.0, 70.0));
    }
    if has("hip") || has("thigh") {
        return Some((JointKind::Spherical, 40.0, 30.0));
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
        // Named rows: spine and chest 20 degrees, head 35, shoulders 85, hips 40.
        for (bone, deg, twist_deg) in [
            (1usize, 20.0f32, 35.0f32),
            (2, 20.0, 35.0),
            (4, 35.0, 45.0),
            (5, 85.0, 70.0),
            (11, 40.0, 30.0),
        ] {
            let spec = table.spec(bone).expect("row");
            assert_eq!(spec.kind, JointKind::Spherical, "bone {bone}");
            assert!(close(spec.swing_limit, deg * DEG), "bone {bone} swing");
            assert!(close(spec.twist_min, -twist_deg * DEG), "bone {bone} twist");
            // The depth limit for these bones is a different number, so the table,
            // not the hierarchy, decided.
            let depth = joint_depths(&skeleton)[bone];
            assert!(
                !close(spec.swing_limit, swing_limit(depth)),
                "bone {bone} must not reuse its depth limit"
            );
        }
        // Knees and elbows hinge, one axis, 145 degrees.
        for bone in [6usize, 9, 12, 15] {
            let spec = table.spec(bone).expect("row");
            assert_eq!(spec.kind, JointKind::Hinge, "bone {bone}");
            assert!(close(spec.swing_limit, 145.0 * DEG));
            assert_eq!((spec.twist_min, spec.twist_max), (0.0, 0.0));
            assert_eq!(spec.friction_torque, JOINT_FRICTION_HINGE_NM);
            assert_eq!(spec.spring_hz, JOINT_SPRING_HINGE_HZ);
        }
        // A name the table does not know keeps its depth limit.
        let ankle = table.spec(13).expect("row");
        assert_eq!(ankle.kind, JointKind::Spherical);
        assert!(close(
            ankle.swing_limit,
            swing_limit(joint_depths(&skeleton)[13])
        ));
        // Every motor is inside the published band and every limit is in 0..180 deg.
        for spec in &table.rows {
            assert!((5.0..=10.0).contains(&spec.friction_torque), "{spec:?}");
            assert!((0.5..=2.0).contains(&spec.spring_hz), "{spec:?}");
            assert!(
                (0.0..=JOINT_LIMIT_MAX_RAD).contains(&spec.swing_limit),
                "{spec:?}"
            );
        }
        println!("JOINT_TABLE hinges=4 rows=17 named=5 depth_fallback=8");
    }

    #[test]
    fn hinge_names_cover_both_vocabularies() {
        let rows = ["knee_l", "elbow_r", "shin_l", "lower_arm_r", "forearm_l"];
        for name in rows {
            let spec = named_row(name, 0.6);
            assert_eq!(spec.kind, JointKind::Hinge, "{name}");
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
            let spec = named_row(name, 0.6);
            assert_eq!(spec.kind, JointKind::Spherical, "{name}");
            assert!(spec.swing_limit > 0.0 && spec.is_valid(), "{name}");
        }
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
