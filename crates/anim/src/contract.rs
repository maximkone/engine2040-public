//! contract: animation types that assets import into and render/arena
//! consume. Units: meters, seconds, radians; rotations are unit quaternions
//! `[x, y, z, w]`. Validation is typed; sampling helpers build on `clip.rs`.
//! This module never imports assets or arena.

use engine_core::{quat_mul, Fnv64};

/// Maximum joints per skeleton (skinning matrices stay bounded).
pub const MAX_JOINTS: usize = 256;
/// Name-key slots for the skeleton duplicate check, twice [`MAX_JOINTS`] so
/// the open-addressed probe always reaches an empty slot without growing.
const NAME_KEY_SLOTS: usize = 2 * MAX_JOINTS;
/// Empty marker for a name-key slot; a joint index stays under [`MAX_JOINTS`].
const NAME_KEY_EMPTY: u16 = u16::MAX;
/// Maximum joint-name length; charset is `[a-z0-9_]` with no leading digit.
pub const NAME_MAX_LEN: usize = 64;

/// One skeleton joint: a name plus its parent index (`None` = root).
#[derive(Clone, PartialEq, Debug)]
pub struct Joint {
    /// Joint name, `[a-z0-9_]`, no leading digit.
    pub name: String,
    /// Parent joint index; every non-root parent must be a smaller index.
    pub parent: Option<usize>,
}

/// Skeleton: joints in parent-first order (parents before children).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Skeleton {
    /// Joints, parents before children.
    pub joints: Vec<Joint>,
}

impl Skeleton {
    /// Joint count.
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// True when there are no joints.
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }
}

/// Local pose: one position plus one unit quaternion per joint.
#[derive(Clone, PartialEq, Debug)]
pub struct LocalPose {
    /// Positions in meters, one per joint.
    pub positions: Vec<[f32; 3]>,
    /// Unit quaternions `[x, y, z, w]`, one per joint.
    pub rotations: Vec<[f32; 4]>,
}

/// One tick of root motion: planar delta plus yaw delta.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RootDelta {
    /// Fixed tick this delta applies before.
    pub tick: u32,
    /// Planar delta `[dx, dz]` in meters.
    pub dp: [f32; 2],
    /// Yaw delta in radians.
    pub dyaw: f32,
}

/// Tick-root table: authoritative root intent per tick, sorted by tick.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RootTable {
    /// Deltas in tick order, no duplicate ticks.
    pub deltas: Vec<RootDelta>,
}

/// One foot-contact sample.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Contact {
    /// Fixed tick this sample belongs to.
    pub tick: u32,
    /// Foot index: `0` = left, `1` = right.
    pub foot: u8,
    /// True while the foot is planted.
    pub down: bool,
}

/// Contact table: foot plant/lift samples in tick order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContactTable {
    /// Samples in tick order.
    pub contacts: Vec<Contact>,
}

/// Animation contract failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Debug)]
pub enum AnimError {
    /// No joints at all.
    EmptySkeleton,
    /// Too many joints, count carried.
    TooManyJoints(usize),
    /// Joint name illegal (empty, too long, bad charset, leading digit or
    /// duplicate); joint index carried.
    BadName(usize),
    /// Parent index invalid (out of range, self-parent or not parent-first);
    /// joint index carried.
    BadParent(usize),
    /// Pose joint count differs from skeleton; expected + got carried.
    LengthMismatch {
        /// Expected joint count.
        expected: usize,
        /// Pose array length.
        got: usize,
    },
    /// Non-finite float in the named array.
    NonFinite(&'static str),
    /// Quaternion at the carried index is not unit length.
    UnnormalizedQuat(usize),
    /// Root table out of tick order or with a duplicate tick; index carried.
    BadRootOrder(usize),
    /// Contact foot is not `0` or `1`; sample index carried.
    BadFoot(usize),
    /// Contact samples out of tick order; sample index carried.
    BadContactOrder(usize),
    /// Source bone name maps to no known role; mapping index carried
    /// (`0` for a single-name lookup).
    UnknownRole(usize),
    /// Source bone name repeats an earlier mapping; duplicate index carried.
    DuplicateBone(usize),
    /// Required roles absent; missing roles carried in required order.
    MissingRoles(Vec<BoneRole>),
}

fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > NAME_MAX_LEN {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or('0');
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// FNV-1a64 of one joint name; `engine_core` owns the constants.
fn name_key(name: &str) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(name.as_bytes());
    hasher.finish()
}

/// Slot for a name key: low bits only, so the value always fits a `usize`.
fn key_slot(key: u64) -> usize {
    usize::try_from(key & u64::try_from(NAME_KEY_SLOTS - 1).unwrap_or(0)).unwrap_or(0)
}

/// Seen joint names as a stack-only open-addressed table: each slot holds a
/// name's key plus the joint index that owns it, and a key hit is confirmed
/// byte for byte, so the answer equals the earlier-name rescan it replaces
/// (same first offending index) at O(1) probes instead of O(n) string
/// compares. Never on the heap: the slot count is twice [`MAX_JOINTS`].
struct NameSet {
    keys: [u64; NAME_KEY_SLOTS],
    owner: [u16; NAME_KEY_SLOTS],
}

impl NameSet {
    fn new() -> Self {
        Self {
            keys: [0; NAME_KEY_SLOTS],
            owner: [NAME_KEY_EMPTY; NAME_KEY_SLOTS],
        }
    }

    /// Record `name` for joint `index` of `all`; `true` when an earlier joint
    /// already owns that exact name.
    fn note(&mut self, name: &str, index: usize, all: &[Joint]) -> bool {
        let key = name_key(name);
        let mask = NAME_KEY_SLOTS - 1;
        let mut slot = key_slot(key);
        loop {
            let owner = self.owner[slot];
            if owner == NAME_KEY_EMPTY {
                self.keys[slot] = key;
                self.owner[slot] = u16::try_from(index).unwrap_or(NAME_KEY_EMPTY);
                return false;
            }
            if self.keys[slot] == key && all[usize::from(owner)].name == name {
                return true;
            }
            slot = (slot + 1) & mask;
        }
    }
}

/// Validate a skeleton: bounds, names, parent-first acyclic links.
pub fn validate_skeleton(skeleton: &Skeleton) -> Result<(), AnimError> {
    if skeleton.is_empty() {
        return Err(AnimError::EmptySkeleton);
    }
    if skeleton.len() > MAX_JOINTS {
        return Err(AnimError::TooManyJoints(skeleton.len()));
    }
    let mut seen = NameSet::new();
    for (index, joint) in skeleton.joints.iter().enumerate() {
        if !valid_name(&joint.name) || seen.note(&joint.name, index, &skeleton.joints) {
            return Err(AnimError::BadName(index));
        }
        match joint.parent {
            None => {}
            Some(parent) if parent < index => {}
            _ => return Err(AnimError::BadParent(index)),
        }
    }
    Ok(())
}

fn finite3(values: &[[f32; 3]], field: &'static str) -> Result<(), AnimError> {
    if values.iter().flatten().any(|v| !v.is_finite()) {
        return Err(AnimError::NonFinite(field));
    }
    Ok(())
}

/// Validate a pose against its skeleton: lengths, finite data, unit quats.
pub fn validate_pose(skeleton: &Skeleton, pose: &LocalPose) -> Result<(), AnimError> {
    validate_skeleton(skeleton)?;
    if pose.positions.len() != skeleton.len() {
        return Err(AnimError::LengthMismatch {
            expected: skeleton.len(),
            got: pose.positions.len(),
        });
    }
    if pose.rotations.len() != skeleton.len() {
        return Err(AnimError::LengthMismatch {
            expected: skeleton.len(),
            got: pose.rotations.len(),
        });
    }
    finite3(&pose.positions, "positions")?;
    for (index, quat) in pose.rotations.iter().enumerate() {
        if quat.iter().any(|v| !v.is_finite()) {
            return Err(AnimError::NonFinite("rotations"));
        }
        let norm = quat.iter().map(|v| v * v).sum::<f32>().sqrt();
        if (norm - 1.0).abs() > 1e-3 {
            return Err(AnimError::UnnormalizedQuat(index));
        }
    }
    Ok(())
}

/// Validate a root table: finite deltas in tick order, no duplicate ticks.
pub fn validate_root(table: &RootTable) -> Result<(), AnimError> {
    let mut prev: Option<u32> = None;
    for (index, delta) in table.deltas.iter().enumerate() {
        if delta.dp.iter().any(|v| !v.is_finite()) || !delta.dyaw.is_finite() {
            return Err(AnimError::NonFinite("root"));
        }
        if let Some(p) = prev {
            if delta.tick <= p {
                return Err(AnimError::BadRootOrder(index));
            }
        }
        prev = Some(delta.tick);
    }
    Ok(())
}

/// Validate a contact table: feet are `0`/`1`, samples in tick order.
pub fn validate_contacts(table: &ContactTable) -> Result<(), AnimError> {
    let mut prev: u32 = 0;
    for (index, contact) in table.contacts.iter().enumerate() {
        if contact.foot > 1 {
            return Err(AnimError::BadFoot(index));
        }
        if index > 0 && contact.tick < prev {
            return Err(AnimError::BadContactOrder(index));
        }
        prev = contact.tick;
    }
    Ok(())
}

/// Rotate `point` by the unit quaternion `quat` (`q * p * conj(q)`).
pub(crate) fn rotate(point: &[f32; 3], quat: &[f32; 4]) -> [f32; 3] {
    let q = quat_mul(*quat, [point[0], point[1], point[2], 0.0]);
    let conj = [-quat[0], -quat[1], -quat[2], quat[3]];
    let r = quat_mul(q, conj);
    [r[0], r[1], r[2]]
}

/// Compose a local pose into world-space skinning matrices (row-major
/// `[[f32; 4]; 4]` with translation in the last row); parents apply first,
/// so the output has one matrix per joint in skeleton order.
pub fn skinning_matrices(
    skeleton: &Skeleton,
    pose: &LocalPose,
) -> Result<Vec<[[f32; 4]; 4]>, AnimError> {
    validate_pose(skeleton, pose)?;
    let mut world_pos: Vec<[f32; 3]> = vec![[0.0; 3]; skeleton.len()];
    let mut world_rot: Vec<[f32; 4]> = vec![[0.0, 0.0, 0.0, 1.0]; skeleton.len()];
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let (parent_pos, parent_rot) = match joint.parent {
            Some(parent) => (world_pos[parent], world_rot[parent]),
            None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
        };
        let rotated = rotate(&pose.positions[index], &parent_rot);
        world_pos[index] = [
            parent_pos[0] + rotated[0],
            parent_pos[1] + rotated[1],
            parent_pos[2] + rotated[2],
        ];
        world_rot[index] = quat_mul(parent_rot, pose.rotations[index]);
    }
    Ok(world_pos
        .iter()
        .zip(world_rot.iter())
        .map(|(pos, rot)| {
            let x = rotate(&[1.0, 0.0, 0.0], rot);
            let y = rotate(&[0.0, 1.0, 0.0], rot);
            let z = rotate(&[0.0, 0.0, 1.0], rot);
            [
                [x[0], x[1], x[2], 0.0],
                [y[0], y[1], y[2], 0.0],
                [z[0], z[1], z[2], 0.0],
                [pos[0], pos[1], pos[2], 1.0],
            ]
        })
        .collect())
}

/// Stable semantic bone role: one closed set shared by Mixamo-style
/// source names and engine bone names. Import-time only, never in the tick.
/// Consumer: a future assets importer maps source skeletons into these
/// roles; render and arena read the mapped roles. Determinism: parsing and
/// coverage use stable order plus integer math only.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum BoneRole {
    /// Scene root.
    Root,
    /// Pelvis and hips.
    Pelvis,
    /// Lower spine.
    SpineLower,
    /// Upper spine.
    SpineUpper,
    /// Head (neck folds into head).
    Head,
    /// Left upper arm.
    UpperArmLeft,
    /// Right upper arm.
    UpperArmRight,
    /// Left lower arm.
    LowerArmLeft,
    /// Right lower arm.
    LowerArmRight,
    /// Left hand.
    HandLeft,
    /// Right hand.
    HandRight,
    /// Left upper leg.
    UpperLegLeft,
    /// Right upper leg.
    UpperLegRight,
    /// Left lower leg.
    LowerLegLeft,
    /// Right lower leg.
    LowerLegRight,
    /// Left foot.
    FootLeft,
    /// Right foot.
    FootRight,
}

impl BoneRole {
    /// Stable snake-case name for diagnostics and receipts.
    pub fn name(self) -> &'static str {
        match self {
            BoneRole::Root => "root",
            BoneRole::Pelvis => "pelvis",
            BoneRole::SpineLower => "spine_lower",
            BoneRole::SpineUpper => "spine_upper",
            BoneRole::Head => "head",
            BoneRole::UpperArmLeft => "upper_arm_l",
            BoneRole::UpperArmRight => "upper_arm_r",
            BoneRole::LowerArmLeft => "lower_arm_l",
            BoneRole::LowerArmRight => "lower_arm_r",
            BoneRole::HandLeft => "hand_l",
            BoneRole::HandRight => "hand_r",
            BoneRole::UpperLegLeft => "upper_leg_l",
            BoneRole::UpperLegRight => "upper_leg_r",
            BoneRole::LowerLegLeft => "lower_leg_l",
            BoneRole::LowerLegRight => "lower_leg_r",
            BoneRole::FootLeft => "foot_l",
            BoneRole::FootRight => "foot_r",
        }
    }

    /// Stable index in [`REQUIRED_BONE_ROLES`] order.
    pub fn index(self) -> usize {
        match self {
            BoneRole::Root => 0,
            BoneRole::Pelvis => 1,
            BoneRole::SpineLower => 2,
            BoneRole::SpineUpper => 3,
            BoneRole::Head => 4,
            BoneRole::UpperArmLeft => 5,
            BoneRole::UpperArmRight => 6,
            BoneRole::LowerArmLeft => 7,
            BoneRole::LowerArmRight => 8,
            BoneRole::HandLeft => 9,
            BoneRole::HandRight => 10,
            BoneRole::UpperLegLeft => 11,
            BoneRole::UpperLegRight => 12,
            BoneRole::LowerLegLeft => 13,
            BoneRole::LowerLegRight => 14,
            BoneRole::FootLeft => 15,
            BoneRole::FootRight => 16,
        }
    }
}

/// Required roles in stable coverage order.
pub const REQUIRED_BONE_ROLES: [BoneRole; 17] = [
    BoneRole::Root,
    BoneRole::Pelvis,
    BoneRole::SpineLower,
    BoneRole::SpineUpper,
    BoneRole::Head,
    BoneRole::UpperArmLeft,
    BoneRole::UpperArmRight,
    BoneRole::LowerArmLeft,
    BoneRole::LowerArmRight,
    BoneRole::HandLeft,
    BoneRole::HandRight,
    BoneRole::UpperLegLeft,
    BoneRole::UpperLegRight,
    BoneRole::LowerLegLeft,
    BoneRole::LowerLegRight,
    BoneRole::FootLeft,
    BoneRole::FootRight,
];

/// Count of required roles.
pub const REQUIRED_BONE_COUNT: usize = 17;

/// Full coverage score in per-mille (`1000` equals `1.0`).
pub const FULL_COVERAGE_PERMILLE: u16 = 1000;

/// One source bone mapped to a stable role. Import-time only, never in the
/// tick. `confidence_permille` is `0..=1000`; `method` is a static tag
/// (`"exact"`, `"alias"` or `"manual"`). Determinism: stable order only.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BoneMapping {
    /// Source bone name as authored (engine or Mixamo-style).
    pub source: String,
    /// Stable role for this source bone.
    pub role: BoneRole,
    /// Match confidence in per-mille, integer only.
    pub confidence_permille: u16,
    /// How the map was built; static tag only.
    pub method: &'static str,
}

/// Map one source bone name to its stable role. Both engine names
/// (`"pelvis"`) and Mixamo-style names (`"mixamorig_Hips"`) map to the same
/// closed role. Unknown names are refused. Determinism: string match only.
pub fn bone_role_for_name(name: &str) -> Result<BoneRole, AnimError> {
    match name {
        "root" | "Root" | "mixamorig_Root" => Ok(BoneRole::Root),
        "pelvis" | "Hips" | "hips" | "mixamorig_Hips" => Ok(BoneRole::Pelvis),
        "spine_lower" | "Spine" | "spine" | "mixamorig_Spine" => Ok(BoneRole::SpineLower),
        "spine_upper" | "Spine1" | "spine1" | "mixamorig_Spine1" | "Spine2"
        | "mixamorig_Spine2" => Ok(BoneRole::SpineUpper),
        "head" | "Head" | "Neck" | "neck" | "mixamorig_Head" | "mixamorig_Neck" => {
            Ok(BoneRole::Head)
        }
        "upper_arm_l" | "LeftArm" | "mixamorig_LeftArm" => Ok(BoneRole::UpperArmLeft),
        "upper_arm_r" | "RightArm" | "mixamorig_RightArm" => Ok(BoneRole::UpperArmRight),
        "lower_arm_l" | "LeftForeArm" | "mixamorig_LeftForeArm" => Ok(BoneRole::LowerArmLeft),
        "lower_arm_r" | "RightForeArm" | "mixamorig_RightForeArm" => Ok(BoneRole::LowerArmRight),
        "hand_l" | "LeftHand" | "mixamorig_LeftHand" => Ok(BoneRole::HandLeft),
        "hand_r" | "RightHand" | "mixamorig_RightHand" => Ok(BoneRole::HandRight),
        "upper_leg_l" | "LeftUpLeg" | "mixamorig_LeftUpLeg" => Ok(BoneRole::UpperLegLeft),
        "upper_leg_r" | "RightUpLeg" | "mixamorig_RightUpLeg" => Ok(BoneRole::UpperLegRight),
        "lower_leg_l" | "LeftLeg" | "mixamorig_LeftLeg" => Ok(BoneRole::LowerLegLeft),
        "lower_leg_r" | "RightLeg" | "mixamorig_RightLeg" => Ok(BoneRole::LowerLegRight),
        "foot_l" | "LeftFoot" | "mixamorig_LeftFoot" => Ok(BoneRole::FootLeft),
        "foot_r" | "RightFoot" | "mixamorig_RightFoot" => Ok(BoneRole::FootRight),
        _ => Err(AnimError::UnknownRole(0)),
    }
}

/// Score one role map over the required roles. Returns per-mille coverage
/// (`1000` equals `1.0`) when every required role is present. Refuses
/// unknown source names, duplicate source bones and missing roles with
/// typed errors. Determinism: stable nested loops plus integer math only.
pub fn bone_coverage(mappings: &[BoneMapping]) -> Result<u16, AnimError> {
    for (index, mapping) in mappings.iter().enumerate() {
        if bone_role_for_name(mapping.source.as_str()).is_err() {
            return Err(AnimError::UnknownRole(index));
        }
    }
    for (index, mapping) in mappings.iter().enumerate() {
        for earlier in mappings.iter().take(index) {
            if earlier.source == mapping.source {
                return Err(AnimError::DuplicateBone(index));
            }
        }
    }
    let mut missing: Vec<BoneRole> = Vec::new();
    for required in REQUIRED_BONE_ROLES.iter() {
        let mut found = false;
        for mapping in mappings.iter() {
            if mapping.role == *required {
                found = true;
                break;
            }
        }
        if !found {
            missing.push(*required);
        }
    }
    if !missing.is_empty() {
        return Err(AnimError::MissingRoles(missing));
    }
    Ok(FULL_COVERAGE_PERMILLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                },
                Joint {
                    name: "spine".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "head".into(),
                    parent: Some(1),
                },
            ],
        }
    }

    fn pose() -> LocalPose {
        LocalPose {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 3],
        }
    }

    #[test]
    fn valid_skeleton_and_pose_pass() {
        let (skeleton, pose) = (skeleton(), pose());
        validate_skeleton(&skeleton).unwrap();
        validate_pose(&skeleton, &pose).unwrap();
    }

    #[test]
    fn skeleton_rejections_name_the_joint() {
        assert_eq!(
            validate_skeleton(&Skeleton::default()),
            Err(AnimError::EmptySkeleton)
        );
        let mut bad = skeleton();
        bad.joints[2].parent = Some(2);
        assert_eq!(validate_skeleton(&bad), Err(AnimError::BadParent(2)));
        let mut bad = skeleton();
        bad.joints[2].parent = Some(9);
        assert_eq!(validate_skeleton(&bad), Err(AnimError::BadParent(2)));
        let mut bad = skeleton();
        bad.joints[1].name = "Bad Name!".into();
        assert_eq!(validate_skeleton(&bad), Err(AnimError::BadName(1)));
        let mut bad = skeleton();
        bad.joints[2].name = "spine".into();
        assert_eq!(validate_skeleton(&bad), Err(AnimError::BadName(2)));
    }

    #[test]
    fn pose_rejections_name_the_problem() {
        let (skeleton, mut short) = (skeleton(), pose());
        short.positions.pop();
        assert_eq!(
            validate_pose(&skeleton, &short),
            Err(AnimError::LengthMismatch {
                expected: 3,
                got: 2
            })
        );
        let mut flat = pose();
        flat.rotations[1] = [0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            validate_pose(&skeleton, &flat),
            Err(AnimError::UnnormalizedQuat(1))
        );
        let mut inf = pose();
        inf.positions[0][0] = f32::INFINITY;
        assert_eq!(
            validate_pose(&skeleton, &inf),
            Err(AnimError::NonFinite("positions"))
        );
    }

    #[test]
    fn root_and_contact_tables_validate() {
        let table = RootTable {
            deltas: vec![
                RootDelta {
                    tick: 0,
                    dp: [0.1, 0.0],
                    dyaw: 0.0,
                },
                RootDelta {
                    tick: 1,
                    dp: [0.1, 0.0],
                    dyaw: 0.1,
                },
            ],
        };
        validate_root(&table).unwrap();
        let bad = RootTable {
            deltas: vec![
                RootDelta {
                    tick: 1,
                    dp: [0.1, 0.0],
                    dyaw: 0.0,
                },
                RootDelta {
                    tick: 1,
                    dp: [0.1, 0.0],
                    dyaw: 0.0,
                },
            ],
        };
        assert_eq!(validate_root(&bad), Err(AnimError::BadRootOrder(1)));

        let contacts = ContactTable {
            contacts: vec![
                Contact {
                    tick: 0,
                    foot: 0,
                    down: true,
                },
                Contact {
                    tick: 5,
                    foot: 0,
                    down: false,
                },
            ],
        };
        validate_contacts(&contacts).unwrap();
        let bad = ContactTable {
            contacts: vec![Contact {
                tick: 0,
                foot: 2,
                down: true,
            }],
        };
        assert_eq!(validate_contacts(&bad), Err(AnimError::BadFoot(0)));
    }

    /// Chain of `joints` uniquely named joints, each parent the index below:
    /// the joint count an imported humanoid rig reaches (a Mixamo skeleton is
    /// 65 bones), which is where the duplicate scan bit.
    fn wide_skeleton(joints: usize) -> Skeleton {
        let mut out = Vec::with_capacity(joints);
        for index in 0..joints {
            out.push(Joint {
                name: format!("joint_{index}"),
                parent: if index == 0 { None } else { Some(index - 1) },
            });
        }
        Skeleton { joints: out }
    }

    /// The duplicate check as it stood before the keyed table: every joint
    /// rescans all earlier names. Kept here as the equality and timing oracle
    /// for [`validate_skeleton`].
    fn validate_skeleton_linear_scan(skeleton: &Skeleton) -> Result<(), AnimError> {
        if skeleton.is_empty() {
            return Err(AnimError::EmptySkeleton);
        }
        if skeleton.len() > MAX_JOINTS {
            return Err(AnimError::TooManyJoints(skeleton.len()));
        }
        for (index, joint) in skeleton.joints.iter().enumerate() {
            if !valid_name(&joint.name)
                || skeleton.joints[..index]
                    .iter()
                    .any(|other| other.name == joint.name)
            {
                return Err(AnimError::BadName(index));
            }
            match joint.parent {
                None => {}
                Some(parent) if parent < index => {}
                _ => return Err(AnimError::BadParent(index)),
            }
        }
        Ok(())
    }

    /// One skeleton per way a rig can be valid or broken, each with the answer
    /// the linear scan gives and the keyed table must reproduce.
    fn validation_cases() -> Vec<(Skeleton, Result<(), AnimError>)> {
        let mut cases = vec![
            (Skeleton::default(), Err(AnimError::EmptySkeleton)),
            (
                wide_skeleton(MAX_JOINTS + 1),
                Err(AnimError::TooManyJoints(MAX_JOINTS + 1)),
            ),
            (wide_skeleton(1), Ok(())),
            (wide_skeleton(2), Ok(())),
            (wide_skeleton(64), Ok(())),
            (wide_skeleton(MAX_JOINTS), Ok(())),
        ];
        let mut duplicate_tail = wide_skeleton(64);
        duplicate_tail.joints[63].name = "joint_0".into();
        cases.push((duplicate_tail, Err(AnimError::BadName(63))));
        let mut duplicate_mid = wide_skeleton(64);
        duplicate_mid.joints[30].name = "joint_7".into();
        cases.push((duplicate_mid, Err(AnimError::BadName(30))));
        let mut duplicate_twice = wide_skeleton(MAX_JOINTS);
        duplicate_twice.joints[10].name = "joint_0".into();
        duplicate_twice.joints[255].name = "joint_9".into();
        cases.push((duplicate_twice, Err(AnimError::BadName(10))));
        let mut near_miss = wide_skeleton(16);
        near_miss.joints[15].name = "joint_01".into();
        cases.push((near_miss, Ok(())));
        let mut underscore = wide_skeleton(8);
        underscore.joints[2].name = "_joint_2".into();
        cases.push((underscore, Ok(())));
        let mut uppercase = wide_skeleton(8);
        uppercase.joints[5].name = "Joint_5".into();
        cases.push((uppercase, Err(AnimError::BadName(5))));
        let mut long_name = wide_skeleton(8);
        long_name.joints[3].name = "j".repeat(NAME_MAX_LEN + 1);
        cases.push((long_name, Err(AnimError::BadName(3))));
        let mut leading_digit = wide_skeleton(8);
        leading_digit.joints[2].name = "9joint".into();
        cases.push((leading_digit, Err(AnimError::BadName(2))));
        let mut forward_parent = wide_skeleton(8);
        forward_parent.joints[1].parent = Some(4);
        cases.push((forward_parent, Err(AnimError::BadParent(1))));
        let mut self_parent = wide_skeleton(8);
        self_parent.joints[2].parent = Some(2);
        cases.push((self_parent, Err(AnimError::BadParent(2))));
        let mut parent_beats_duplicate = wide_skeleton(8);
        parent_beats_duplicate.joints[1].parent = Some(5);
        parent_beats_duplicate.joints[4].name = "joint_2".into();
        cases.push((parent_beats_duplicate, Err(AnimError::BadParent(1))));
        cases
    }

    /// The keyed name table answers exactly what the earlier-name rescan
    /// answered: the same `Ok`, or the same error variant with the same joint
    /// index, over valid rigs and every way a skeleton can break.
    #[test]
    fn keyed_name_table_matches_the_linear_scan_oracle() {
        let cases = validation_cases();
        assert!(cases.len() >= 15, "the case list shrank");
        for (case, want) in cases {
            assert_eq!(
                validate_skeleton(&case),
                want,
                "keyed table moved: {} joints",
                case.len()
            );
            assert_eq!(
                validate_skeleton_linear_scan(&case),
                want,
                "oracle moved: {} joints",
                case.len()
            );
            assert_eq!(
                validate_skeleton(&case),
                validate_skeleton_linear_scan(&case),
                "the two paths disagree: {} joints",
                case.len()
            );
        }
    }

    /// Timing receipt: p50 microseconds of one [`validate_skeleton`] pass over
    /// a 64-joint fixture, keyed name table against the earlier-name rescan it
    /// replaced, interleaved so the same load sits under both sides. The keyed
    /// side stays only while the pair holds and it wins.
    /// Run: cargo test -p engine-anim --release --lib skeleton_name_scan -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn skeleton_name_scan_paired_receipt() {
        const JOINTS: usize = 64;
        const ITERS: usize = 200;
        let skeleton = wide_skeleton(JOINTS);
        assert_eq!(
            validate_skeleton(&skeleton),
            validate_skeleton_linear_scan(&skeleton),
            "the two timed paths disagree"
        );
        let mut rescan = Vec::with_capacity(ITERS);
        let mut keyed = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let begin = std::time::Instant::now();
            let _ = std::hint::black_box(validate_skeleton_linear_scan(&skeleton));
            rescan.push(begin.elapsed().as_secs_f64() * 1.0e6);
            let begin = std::time::Instant::now();
            let _ = std::hint::black_box(validate_skeleton(&skeleton));
            keyed.push(begin.elapsed().as_secs_f64() * 1.0e6);
        }
        rescan.sort_by(|a, b| a.partial_cmp(b).unwrap());
        keyed.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let (rescan, keyed) = (rescan[ITERS / 2], keyed[ITERS / 2]);
        eprintln!(
            "contract_skeleton_scan rescan_p50_us={rescan:.2} keyed_p50_us={keyed:.2} ratio_x={:.1} joints={JOINTS} iters={ITERS}",
            rescan / keyed
        );
        assert!(
            keyed < rescan,
            "the keyed name table must not be the slower one: {keyed:.2} vs {rescan:.2}"
        );
    }

    #[test]
    fn skinning_composes_parent_first() {
        let matrices = skinning_matrices(&skeleton(), &pose()).unwrap();
        assert_eq!(matrices.len(), 3);
        // Root at origin, spine/head offset +1 m in Y each.
        assert_eq!(matrices[0][3], [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(matrices[1][3], [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(matrices[2][3], [0.0, 2.0, 0.0, 1.0]);
        // Identity rotations keep the basis axes.
        assert_eq!(matrices[2][0], [1.0, 0.0, 0.0, 0.0]);
        // Same inputs twice give the same matrices.
        assert_eq!(matrices, skinning_matrices(&skeleton(), &pose()).unwrap());
    }

    fn full_role_map() -> Vec<BoneMapping> {
        vec![
            BoneMapping {
                source: "root".into(),
                role: BoneRole::Root,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "pelvis".into(),
                role: BoneRole::Pelvis,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "spine_lower".into(),
                role: BoneRole::SpineLower,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "spine_upper".into(),
                role: BoneRole::SpineUpper,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "head".into(),
                role: BoneRole::Head,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "upper_arm_l".into(),
                role: BoneRole::UpperArmLeft,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "upper_arm_r".into(),
                role: BoneRole::UpperArmRight,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "lower_arm_l".into(),
                role: BoneRole::LowerArmLeft,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "lower_arm_r".into(),
                role: BoneRole::LowerArmRight,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "hand_l".into(),
                role: BoneRole::HandLeft,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "hand_r".into(),
                role: BoneRole::HandRight,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "upper_leg_l".into(),
                role: BoneRole::UpperLegLeft,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "upper_leg_r".into(),
                role: BoneRole::UpperLegRight,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "lower_leg_l".into(),
                role: BoneRole::LowerLegLeft,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "lower_leg_r".into(),
                role: BoneRole::LowerLegRight,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "foot_l".into(),
                role: BoneRole::FootLeft,
                confidence_permille: 1000,
                method: "exact",
            },
            BoneMapping {
                source: "foot_r".into(),
                role: BoneRole::FootRight,
                confidence_permille: 1000,
                method: "exact",
            },
        ]
    }

    #[test]
    fn full_humanoid_map_scores_full() {
        let map = full_role_map();
        assert_eq!(map.len(), REQUIRED_BONE_COUNT);
        // 1000 per-mille equals 1.0 coverage.
        assert_eq!(bone_coverage(&map), Ok(FULL_COVERAGE_PERMILLE));
        assert_eq!(bone_coverage(&map), Ok(1000));
    }

    #[test]
    fn two_names_map_to_same_role() {
        assert_eq!(bone_role_for_name("pelvis"), Ok(BoneRole::Pelvis));
        assert_eq!(bone_role_for_name("mixamorig_Hips"), Ok(BoneRole::Pelvis));
        // Two distinct sources share one role; the role counts once.
        let mut map = full_role_map();
        map.push(BoneMapping {
            source: "mixamorig_Hips".into(),
            role: BoneRole::Pelvis,
            confidence_permille: 900,
            method: "alias",
        });
        assert_eq!(bone_coverage(&map), Ok(1000));
    }

    #[test]
    fn missing_left_lower_leg_names_it() {
        let mut map = full_role_map();
        map.retain(|mapping| mapping.role != BoneRole::LowerLegLeft);
        assert_eq!(
            bone_coverage(&map),
            Err(AnimError::MissingRoles(vec![BoneRole::LowerLegLeft]))
        );
    }

    #[test]
    fn duplicate_sources_refuse() {
        let mut map = full_role_map();
        let first = map[0].clone();
        map.push(first);
        assert_eq!(bone_coverage(&map), Err(AnimError::DuplicateBone(17)));
    }

    #[test]
    fn unknown_roles_refuse_and_runs_agree() {
        assert_eq!(
            bone_role_for_name("no_such_bone"),
            Err(AnimError::UnknownRole(0))
        );
        let map = full_role_map();
        let first = bone_coverage(&map);
        let second = bone_coverage(&map);
        assert_eq!(first, second);
        assert_eq!(first, Ok(1000));
        let mut bad = full_role_map();
        bad[5].source = "no_such_bone".into();
        assert_eq!(bone_coverage(&bad), Err(AnimError::UnknownRole(5)));
    }
}
