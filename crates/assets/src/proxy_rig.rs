//! proxy_rig: the potato tier's cheap rig. A shipped fighter draws its body
//! with 43 joints and up to four influences a vertex; the low tier cannot pay
//! that, so this module folds the authored rig into at most
//! [`MAX_PROXY_BONES`](crate::proxy_rig::MAX_PROXY_BONES) bones and keeps the silhouette.
//!
//! Consumer: the low-tier body cook, before the render skin is built; the tick
//! never reads this module. Data owner: `engine-assets`; the rig and pose types
//! belong to `engine-anim`.
//!
//! Fold rule, one pass over the full rig in parent-first order: a joint named
//! in the keep list is a proxy bone and keeps its bind transform exactly;
//! every other joint merges into its nearest kept ancestor (thumbs into the
//! palm, toes and heels into the foot, the shoulder into the torso). A kept
//! bone's proxy bind position and rotation equal its full-rig world bind, so
//! the rest pose draws the same body at the same size.
//!
//! Weights: a vertex's four full-rig influences take the same fold; rows that
//! land on one proxy bone add up, the four largest remain and the row is
//! normalized, so the deformation follows the dominant authored bones. The
//! output is ordered vectors only; no hash map is built or returned.
//!
//! Pose: [`fold_pose`](crate::proxy_rig::fold_pose) keeps each kept bone's world transform, so a clip baked
//! on the full rig plays on the proxy rig with no retarget. Only the motion of
//! the merged bones is dropped, and that is the tier's cost.
//!
//! Determinism: fixed tables, ordered vectors, integer bounds checks and f32
//! `+ - * /` plus `sqrt`; the same rig bakes the same bytes twice.

use engine_anim::contract::{validate_skeleton, Joint, LocalPose, Skeleton};
use engine_core::{quat_conj, quat_mul, rotate_vec, Fnv64};
use std::fmt;

/// Most bones a proxy rig carries: the low tier's skinning budget.
pub const MAX_PROXY_BONES: usize = 16;
/// Far-tier frame height the silhouette budget is measured on: the page's
/// 1080p view.
pub const FAR_VIEW_H: u32 = 1080;
/// Far-tier vertical field of view in degrees: the presenter camera at 60 m.
pub const FAR_FOV_Y_DEG: f32 = 50.0;
/// Influences per vertex row; mirrors the A3 row the render skin uploads.
pub const INFLUENCES: usize = 4;
/// Proxy byte-image magic (`PXR1` in little-endian order).
pub const PROXY_MAGIC: u32 = 0x3152_5850;
/// Proxy byte-image version.
pub const PROXY_VERSION: u16 = 1;
/// A weight row must sum to one within this tolerance; mirrors A3.
const WEIGHT_SUM_EPS: f32 = 1e-3;
/// The rotation of a joint with no authored rotation.
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// No parent, in the byte image; a joint index always stays below this.
const NO_PARENT: u16 = u16::MAX;

/// The bones the shipped CT7 fighter keeps on the low tier, in the rig's
/// parent-first order: the trunk, the head, and each limb at upper, lower and
/// end. The shoulders fold into the torso, the fingers into the palm, and the
/// toes and heels into the foot: 15 bones, one under the cap.
pub static PROXY_KEEP: [&str; 15] = [
    "pelvis",
    "torso",
    "head",
    "arm_upper_l",
    "arm_lower_l",
    "palm_l",
    "arm_upper_r",
    "arm_lower_r",
    "palm_r",
    "leg_upper_l",
    "leg_lower_l",
    "foot_l",
    "leg_upper_r",
    "leg_lower_r",
    "foot_r",
];

/// Why a proxy fold refused its input; every refusal names what it saw.
#[derive(Clone, PartialEq, Debug)]
pub enum ProxyRigError {
    /// The full rig failed anim-contract validation.
    Rig(engine_anim::contract::AnimError),
    /// The bind or pose row count differs from the joint count.
    BindMismatch {
        /// Full-rig joint count.
        expected: usize,
        /// Rows found.
        got: usize,
    },
    /// The keep list names more bones than [`MAX_PROXY_BONES`].
    TooManyBones {
        /// Offending keep count.
        count: usize,
    },
    /// A keep name is not a joint of the rig; index in the keep list carried.
    UnknownJoint {
        /// Position in the keep list.
        index: usize,
    },
    /// The keep list leaves the rig's root out, so no bone can stay in place.
    RootNotKept,
    /// Joint and weight row counts differ.
    LengthMismatch {
        /// Joint row count.
        expected: usize,
        /// Weight row count.
        got: usize,
    },
    /// A weight row names a joint past the rig; vertex carried.
    JointOutOfRange {
        /// Vertex row index.
        vertex: usize,
        /// Offending joint index.
        joint: usize,
    },
    /// A weight row does not sum to one within `WEIGHT_SUM_EPS`.
    BadWeightSum {
        /// Vertex row index.
        vertex: usize,
    },
    /// A non-finite float in the named lane.
    NonFinite(&'static str),
}

impl fmt::Display for ProxyRigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rig(error) => write!(f, "proxy rig: full rig rejected: {error:?}"),
            Self::BindMismatch { expected, got } => {
                write!(f, "proxy rig wants {expected} bind rows, found {got}")
            }
            Self::TooManyBones { count } => {
                write!(
                    f,
                    "proxy rig keep list has {count} bones, cap {MAX_PROXY_BONES}"
                )
            }
            Self::UnknownJoint { index } => {
                write!(f, "proxy rig keep name {index} is not a joint")
            }
            Self::RootNotKept => write!(f, "proxy rig keep list drops the root"),
            Self::LengthMismatch { expected, got } => {
                write!(f, "proxy rig wants {expected} joint rows, found {got}")
            }
            Self::JointOutOfRange { vertex, joint } => {
                write!(f, "proxy rig vertex {vertex} addresses joint {joint}")
            }
            Self::BadWeightSum { vertex } => {
                write!(f, "proxy rig vertex {vertex} weights do not sum to one")
            }
            Self::NonFinite(lane) => write!(f, "proxy rig {lane} is not finite"),
        }
    }
}

impl std::error::Error for ProxyRigError {}

impl From<engine_anim::contract::AnimError> for ProxyRigError {
    fn from(error: engine_anim::contract::AnimError) -> Self {
        Self::Rig(error)
    }
}

/// The fold as data: which full-rig bone each proxy bone is, and which proxy
/// bone every full-rig joint reports to. Ordered vectors in rig order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProxyPlan {
    /// Full-rig joint index per proxy bone, in proxy order (parent first).
    pub kept: Vec<usize>,
    /// Proxy bone driven by each full-rig joint, full-rig order.
    pub proxy_of: Vec<u16>,
    /// Parent proxy bone per proxy bone, `None` for the root.
    pub parents: Vec<Option<u16>>,
}

impl ProxyPlan {
    /// Proxy bone count.
    pub fn len(&self) -> usize {
        self.kept.len()
    }

    /// True when no bone was kept; never for a planned rig, which keeps a root.
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }
}

/// Proxy bind: one local offset and one local rotation per proxy bone, in
/// proxy order, folded so each kept bone's world bind stays where it was.
#[derive(Clone, PartialEq, Debug)]
pub struct ProxyBind {
    /// Local bind offsets in meters.
    pub positions: Vec<[f32; 3]>,
    /// Local bind rotations, unit quaternions `[x, y, z, w]`.
    pub rotations: Vec<[f32; 4]>,
}

/// Proxy skin: fixed four-influence rows over the proxy bones, each row
/// non-negative and normalized.
#[derive(Clone, PartialEq, Debug)]
pub struct ProxySkin {
    /// Proxy joint index per vertex.
    pub joints: Vec<[u16; INFLUENCES]>,
    /// Weight per vertex; each row sums to one.
    pub weights: Vec<[f32; INFLUENCES]>,
}

impl ProxySkin {
    /// Vertex count.
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }
}

/// A baked proxy rig: the cheap skeleton, its bind and the folded skin.
#[derive(Clone, PartialEq, Debug)]
pub struct ProxyRig {
    /// Proxy skeleton, parent first, names carried from the full rig.
    pub skeleton: Skeleton,
    /// Proxy bind, exact on every kept bone.
    pub bind: ProxyBind,
    /// Folded vertex rows.
    pub skin: ProxySkin,
    /// The fold this rig came from.
    pub plan: ProxyPlan,
    /// Full-rig joint count, carried for the receipt.
    pub source_joints: usize,
}

impl ProxyRig {
    /// Deterministic byte image: magic (`u32`), version (`u16`), proxy bone
    /// count (`u16`), full-rig joint count (`u32`), vertex count (`u32`), then
    /// every bone (name, parent, bind) and every vertex row.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PROXY_MAGIC.to_le_bytes());
        out.extend_from_slice(&PROXY_VERSION.to_le_bytes());
        out.extend_from_slice(&pack_u16(self.skeleton.len()));
        out.extend_from_slice(&pack_u32(self.source_joints));
        out.extend_from_slice(&pack_u32(self.skin.len()));
        for (index, joint) in self.skeleton.joints.iter().enumerate() {
            let name = joint.name.as_bytes();
            out.extend_from_slice(&pack_u32(name.len()));
            out.extend_from_slice(name);
            out.extend_from_slice(&pack_parent(joint.parent));
            for value in self.bind.positions[index] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            for value in self.bind.rotations[index] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for (joints, weights) in self.skin.joints.iter().zip(self.skin.weights.iter()) {
            for joint in joints {
                out.extend_from_slice(&joint.to_le_bytes());
            }
            for weight in weights {
                out.extend_from_slice(&weight.to_le_bytes());
            }
        }
        out
    }

    /// FNV-1a64 over the byte image: the number a cook-twice proof compares.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write(&self.to_bytes());
        hash.finish()
    }

    /// The fold's palette as `(source joints, proxy bones)`: the far tier's
    /// receipt number (43 -> 15 for the shipped CT7 hero).
    pub fn palette(&self) -> (usize, usize) {
        (self.source_joints, self.skeleton.len())
    }
}

/// One length as a `u32` byte field; a rig never reaches the impossible tail.
fn pack_u32(value: usize) -> [u8; 4] {
    u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes()
}

/// One count that fits the low tier as a `u16` byte field.
fn pack_u16(value: usize) -> [u8; 2] {
    u16::try_from(value).unwrap_or(u16::MAX).to_le_bytes()
}

/// A parent index as a byte field, [`NO_PARENT`] for the root.
fn pack_parent(parent: Option<usize>) -> [u8; 2] {
    match parent {
        Some(parent) => u16::try_from(parent).unwrap_or(NO_PARENT).to_le_bytes(),
        None => NO_PARENT.to_le_bytes(),
    }
}

/// Screen error in pixels of a world-space `error_m` seen `distance_m` away
/// through a `fov_y_deg` lens on a `viewport_h` px tall frame. One metre spans
/// `(viewport_h / 2) / (tan(fov_y / 2) * distance_m)` pixels, so one pixel is
/// about 5.2 cm at 60 m on the far tier's 1080p frame. `distance_m` must be
/// positive; a height past `u16::MAX` counts as `u16::MAX`.
pub fn silhouette_px(error_m: f32, distance_m: f32, viewport_h: u32, fov_y_deg: f32) -> f32 {
    let half_h = f32::from(u16::try_from(viewport_h).unwrap_or(u16::MAX)) * 0.5;
    let px_per_m = half_h / ((fov_y_deg.to_radians() * 0.5).tan() * distance_m);
    error_m * px_per_m
}

/// World transform per joint: position plus rotation, parents first.
fn world_transforms(
    skeleton: &Skeleton,
    positions: &[[f32; 3]],
    rotations: &[[f32; 4]],
) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let mut world_pos = vec![[0.0f32; 3]; skeleton.len()];
    let mut world_rot = vec![IDENTITY; skeleton.len()];
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let (parent_pos, parent_rot) = match joint.parent {
            Some(parent) => (world_pos[parent], world_rot[parent]),
            None => ([0.0f32; 3], IDENTITY),
        };
        let local = rotate_vec(&positions[index], &parent_rot);
        world_pos[index] = [
            parent_pos[0] + local[0],
            parent_pos[1] + local[1],
            parent_pos[2] + local[2],
        ];
        world_rot[index] = quat_mul(parent_rot, rotations[index]);
    }
    (world_pos, world_rot)
}

/// Plan the fold: resolve the keep names, then walk the rig parent first and
/// give every joint its nearest kept ancestor.
pub fn plan_proxy(skeleton: &Skeleton, keep: &[&str]) -> Result<ProxyPlan, ProxyRigError> {
    validate_skeleton(skeleton)?;
    if keep.len() > MAX_PROXY_BONES {
        return Err(ProxyRigError::TooManyBones { count: keep.len() });
    }
    let mut keeps = vec![false; skeleton.len()];
    for (index, name) in keep.iter().enumerate() {
        let found = skeleton
            .joints
            .iter()
            .position(|joint| joint.name == *name)
            .ok_or(ProxyRigError::UnknownJoint { index })?;
        keeps[found] = true;
    }
    if !keeps[0] {
        return Err(ProxyRigError::RootNotKept);
    }
    let mut slots = vec![NO_PARENT; skeleton.len()];
    let mut kept = Vec::with_capacity(keep.len());
    for index in 0..skeleton.len() {
        if keeps[index] {
            let slot = u16::try_from(kept.len()).unwrap_or(NO_PARENT);
            slots[index] = slot;
            kept.push(index);
        }
    }
    let mut proxy_of = vec![0u16; skeleton.len()];
    let mut parents = Vec::with_capacity(kept.len());
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let target = match joint.parent {
            None => slots[index],
            Some(_) if keeps[index] => slots[index],
            Some(parent) => proxy_of[parent],
        };
        proxy_of[index] = target;
        if keeps[index] {
            parents.push(joint.parent.map(|parent| proxy_of[parent]));
        }
    }
    Ok(ProxyPlan {
        kept,
        proxy_of,
        parents,
    })
}

/// Express one kept bone's world transform against its nearest kept ancestor:
/// the local row the proxy bone carries for the world transforms given.
fn local_against(
    world_pos: &[[f32; 3]],
    world_rot: &[[f32; 4]],
    plan: &ProxyPlan,
    slot: usize,
) -> ([f32; 3], [f32; 4]) {
    let full = plan.kept[slot];
    match plan.parents[slot] {
        None => (world_pos[full], world_rot[full]),
        Some(parent) => {
            let base = plan.kept[usize::from(parent)];
            let offset = [
                world_pos[full][0] - world_pos[base][0],
                world_pos[full][1] - world_pos[base][1],
                world_pos[full][2] - world_pos[base][2],
            ];
            (
                rotate_vec(&offset, &quat_conj(&world_rot[base])),
                quat_mul(quat_conj(&world_rot[base]), world_rot[full]),
            )
        }
    }
}

/// Fold the bind: every kept bone's world bind is preserved, so its proxy
/// local offset and rotation are relative to the nearest kept ancestor.
pub fn proxy_bind(
    skeleton: &Skeleton,
    positions: &[[f32; 3]],
    rotations: &[[f32; 4]],
    plan: &ProxyPlan,
) -> Result<ProxyBind, ProxyRigError> {
    if positions.len() != skeleton.len() || rotations.len() != skeleton.len() {
        return Err(ProxyRigError::BindMismatch {
            expected: skeleton.len(),
            got: positions.len().min(rotations.len()),
        });
    }
    if positions.iter().flatten().any(|value| !value.is_finite()) {
        return Err(ProxyRigError::NonFinite("bind positions"));
    }
    if rotations.iter().flatten().any(|value| !value.is_finite()) {
        return Err(ProxyRigError::NonFinite("bind rotations"));
    }
    let (world_pos, world_rot) = world_transforms(skeleton, positions, rotations);
    let mut out_pos = Vec::with_capacity(plan.len());
    let mut out_rot = Vec::with_capacity(plan.len());
    for slot in 0..plan.len() {
        let (position, rotation) = local_against(&world_pos, &world_rot, plan, slot);
        out_pos.push(position);
        out_rot.push(rotation);
    }
    Ok(ProxyBind {
        positions: out_pos,
        rotations: out_rot,
    })
}

/// Fold a pose: each kept bone holds the world position and rotation the full
/// rig gives it, expressed against its nearest kept ancestor.
pub fn fold_pose(
    skeleton: &Skeleton,
    plan: &ProxyPlan,
    pose: &LocalPose,
) -> Result<LocalPose, ProxyRigError> {
    if pose.positions.len() != skeleton.len() || pose.rotations.len() != skeleton.len() {
        return Err(ProxyRigError::BindMismatch {
            expected: skeleton.len(),
            got: pose.positions.len().min(pose.rotations.len()),
        });
    }
    let (world_pos, world_rot) = world_transforms(skeleton, &pose.positions, &pose.rotations);
    let mut positions = Vec::with_capacity(plan.len());
    let mut rotations = Vec::with_capacity(plan.len());
    for slot in 0..plan.len() {
        let (position, rotation) = local_against(&world_pos, &world_rot, plan, slot);
        positions.push(position);
        rotations.push(rotation);
    }
    Ok(LocalPose {
        positions,
        rotations,
    })
}

/// Fold one weight row: influences that land on one proxy bone add up, the
/// four largest stay (ties broken by the lower proxy index) and the row is
/// normalized. Unused slots pad with joint 0 and weight 0.
fn fold_row(
    plan: &ProxyPlan,
    joints: &[u16; INFLUENCES],
    weights: &[f32; INFLUENCES],
) -> ([u16; INFLUENCES], [f32; INFLUENCES]) {
    let mut merged_j = [0u16; INFLUENCES];
    let mut merged_w = [0.0f32; INFLUENCES];
    let mut count = 0usize;
    for (joint, weight) in joints.iter().zip(weights.iter()) {
        if *weight <= 0.0 {
            continue;
        }
        let proxy = plan.proxy_of[usize::from(*joint)];
        let mut slot = 0usize;
        while slot < count && merged_j[slot] != proxy {
            slot += 1;
        }
        if slot == count {
            // Four influences make at most four proxies, so a fifth never fits.
            if count == INFLUENCES {
                continue;
            }
            merged_j[count] = proxy;
            count += 1;
        }
        merged_w[slot] += *weight;
    }
    // Selection sort by weight, largest first; a tie keeps the lower proxy.
    for head in 0..count {
        let mut best = head;
        for other in (head + 1)..count {
            let larger = merged_w[other] > merged_w[best]
                || (merged_w[other] == merged_w[best] && merged_j[other] < merged_j[best]);
            if larger {
                best = other;
            }
        }
        merged_j.swap(head, best);
        merged_w.swap(head, best);
    }
    let total: f32 = merged_w[..count].iter().sum();
    let mut out_j = [0u16; INFLUENCES];
    let mut out_w = [0.0f32; INFLUENCES];
    if total > 0.0 {
        for slot in 0..count {
            out_j[slot] = merged_j[slot];
            out_w[slot] = merged_w[slot] / total;
        }
    }
    (out_j, out_w)
}

/// Fold the skin: one proxy row per vertex, normalized over the proxy bones.
pub fn fold_weights(
    plan: &ProxyPlan,
    joints: &[[u16; INFLUENCES]],
    weights: &[[f32; INFLUENCES]],
) -> Result<ProxySkin, ProxyRigError> {
    if joints.len() != weights.len() {
        return Err(ProxyRigError::LengthMismatch {
            expected: joints.len(),
            got: weights.len(),
        });
    }
    let mut out_joints = Vec::with_capacity(joints.len());
    let mut out_weights = Vec::with_capacity(joints.len());
    for (vertex, (row_j, row_w)) in joints.iter().zip(weights.iter()).enumerate() {
        let mut sum = 0.0f32;
        for (joint, weight) in row_j.iter().zip(row_w.iter()) {
            if usize::from(*joint) >= plan.proxy_of.len() {
                return Err(ProxyRigError::JointOutOfRange {
                    vertex,
                    joint: usize::from(*joint),
                });
            }
            if !weight.is_finite() {
                return Err(ProxyRigError::NonFinite("weights"));
            }
            sum += *weight;
        }
        if (sum - 1.0).abs() > WEIGHT_SUM_EPS {
            return Err(ProxyRigError::BadWeightSum { vertex });
        }
        let (row_j, row_w) = fold_row(plan, row_j, row_w);
        out_joints.push(row_j);
        out_weights.push(row_w);
    }
    Ok(ProxySkin {
        joints: out_joints,
        weights: out_weights,
    })
}

/// Bake one proxy rig: plan, fold the bind, fold the skin, keep the receipt.
pub fn bake_proxy_rig(
    skeleton: &Skeleton,
    positions: &[[f32; 3]],
    rotations: &[[f32; 4]],
    joints: &[[u16; INFLUENCES]],
    weights: &[[f32; INFLUENCES]],
    keep: &[&str],
) -> Result<ProxyRig, ProxyRigError> {
    let plan = plan_proxy(skeleton, keep)?;
    let bind = proxy_bind(skeleton, positions, rotations, &plan)?;
    let skin = fold_weights(&plan, joints, weights)?;
    let proxy_joints = plan
        .kept
        .iter()
        .enumerate()
        .map(|(slot, &full)| Joint {
            name: skeleton.joints[full].name.clone(),
            parent: plan.parents[slot].map(usize::from),
        })
        .collect();
    Ok(ProxyRig {
        skeleton: Skeleton {
            joints: proxy_joints,
        },
        bind,
        skin,
        plan,
        source_joints: skeleton.len(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::{validate_pose, validate_skeleton};

    /// The shipped CT7 hero body's 43 skin joints, read off
    /// `content/fighters/glb/hero_body.glb` in the file's parent-first order:
    /// `(joint, parent, bind offset, bind rotation)`, meters and `[x, y, z, w]`.
    /// Names are the game's sanitized form (`palm_L` becomes `palm_l`). The
    /// first test checks this count against the shipped file.
    static HERO_RIG: [RigRow; 43] = [
        (
            "pelvis",
            None,
            [0.0, 0.8923538, -3.373265e-8],
            [0.07943931, 1.188326e-7, 9.469904e-9, 0.9968397],
        ),
        (
            "leg_upper_l",
            Some("pelvis"),
            [0.09085749, 0.04492927, -0.007206644],
            [0.6960161, -0.008417353, -0.7082512, 0.1177751],
        ),
        (
            "leg_lower_l",
            Some("leg_upper_l"),
            [-3.49246e-10, 0.4358965, 3.85262e-9],
            [0.05934752, 0.7079085, -0.03808044, 0.7027755],
        ),
        (
            "foot_l",
            Some("leg_lower_l"),
            [2.382876e-9, 0.4380578, -9.313226e-10],
            [-0.6113753, -0.01970251, 0.04244615, 0.7899559],
        ),
        (
            "heel_l",
            Some("foot_l"),
            [-8.149073e-9, 7.043127e-9, 4.190952e-9],
            [0.7003633, -0.01076194, 0.01277969, 0.713591],
        ),
        (
            "toes_l",
            Some("foot_l"),
            [-1.103035e-8, 0.1833664, 2.793968e-9],
            [0.001521932, 0.9826005, -0.1849789, 0.01663871],
        ),
        (
            "leg_upper_r",
            Some("pelvis"),
            [-0.09085749, 0.04492927, -0.007206689],
            [0.696016, 0.008417315, 0.7082513, 0.1177751],
        ),
        (
            "leg_lower_r",
            Some("leg_upper_r"),
            [-4.656613e-10, 0.4358964, 9.807991e-9],
            [0.05934751, -0.7079084, 0.03808044, 0.7027756],
        ),
        (
            "foot_r",
            Some("leg_lower_r"),
            [-1.207809e-9, 0.4380578, 1.862645e-9],
            [-0.6113755, 0.01970197, -0.0424457, 0.7899559],
        ),
        (
            "heel_r",
            Some("foot_r"),
            [-1.012813e-8, 2.444722e-9, -3.259629e-9],
            [0.7003635, 0.01076245, -0.01278019, 0.7135909],
        ),
        (
            "toes_r",
            Some("foot_r"),
            [-7.305061e-9, 0.1833664, 3.72529e-9],
            [0.001522217, -0.9826005, 0.1849788, 0.0166386],
        ),
        (
            "torso",
            Some("pelvis"),
            [6.217249e-15, 0.1695047, -5.587935e-9],
            [-0.1379575, -1.120941e-14, -3.289163e-8, 0.9904382],
        ),
        (
            "head",
            Some("torso"),
            [2.664535e-15, 0.5035205, -1.862645e-8],
            [0.1253834, 1.835249e-14, 2.989373e-8, 0.9921084],
        ),
        (
            "shoulder_l",
            Some("torso"),
            [0.01025882, 0.3161732, 0.08017681],
            [-0.4623945, -0.3524101, -0.3097669, 0.7523582],
        ),
        (
            "arm_upper_l",
            Some("shoulder_l"),
            [-1.490116e-8, 0.2091267, -9.406358e-8],
            [-0.480507, 0.07562256, -0.2800363, 0.8276316],
        ),
        (
            "arm_lower_l",
            Some("arm_upper_l"),
            [2.561137e-9, 0.2465016, -1.14087e-7],
            [0.08354509, -0.04766805, -0.1032522, 0.9899934],
        ),
        (
            "palm_l",
            Some("arm_lower_l"),
            [7.450581e-9, 0.2002213, -6.286427e-8],
            [-0.06067554, 0.7130308, 0.03328469, 0.6977089],
        ),
        (
            "index_upper_l",
            Some("palm_l"),
            [-0.0008182985, 0.1005081, 0.02715274],
            [0.09294704, 0.05117077, 0.0009619204, 0.9943548],
        ),
        (
            "index_lower_l",
            Some("index_upper_l"),
            [1.129229e-8, 0.04113122, 1.862645e-9],
            [0.001259164, -0.02036117, -0.04058463, 0.9989679],
        ),
        (
            "middle_upper_l",
            Some("palm_l"),
            [-0.002912349, 0.1017738, 0.01045302],
            [0.03062599, 0.03408706, -0.01588595, 0.9988232],
        ),
        (
            "middle_lower_l",
            Some("middle_upper_l"),
            [-1.895824e-7, 0.03964588, -3.259629e-9],
            [0.003874874, -0.008316917, -0.03015893, 0.999503],
        ),
        (
            "pinky_upper_l",
            Some("palm_l"),
            [-0.003301572, 0.1018655, -0.02737971],
            [-0.08202803, -0.04654581, -0.02342373, 0.9952669],
        ),
        (
            "pinky_lower_l",
            Some("pinky_upper_l"),
            [1.522712e-7, 0.04137624, -1.117587e-8],
            [0.009484438, -0.02029662, -0.04156827, 0.9988845],
        ),
        (
            "ring_upper_l",
            Some("palm_l"),
            [-0.003323065, 0.1026532, -0.01015223],
            [-0.005253655, -0.01441146, -0.01711478, 0.9997358],
        ),
        (
            "ring_lower_l",
            Some("ring_upper_l"),
            [-7.287599e-8, 0.03975936, 0.0],
            [-0.01118031, -0.0133684, -0.0405641, 0.9990249],
        ),
        (
            "thumb_upper_l",
            Some("palm_l"),
            [0.02432891, 0.02908562, 0.02551491],
            [0.1394165, -0.08989055, -0.4448151, 0.8801263],
        ),
        (
            "thumb_lower_l",
            Some("thumb_upper_l"),
            [7.334165e-9, 0.03369768, -5.029142e-8],
            [-0.06995098, -0.6840794, 0.04850004, 0.724424],
        ),
        (
            "wep_pos_l",
            Some("palm_l"),
            [0.01494575, 0.08633251, -0.0002042032],
            [0.6192575, -0.3128123, -0.3250257, 0.6426716],
        ),
        (
            "shoulder_r",
            Some("torso"),
            [-0.01025886, 0.3161732, 0.08017682],
            [-0.4623946, 0.3524099, 0.3097668, 0.7523583],
        ),
        (
            "arm_upper_r",
            Some("shoulder_r"),
            [2.607703e-8, 0.2091267, 4.097819e-8],
            [-0.4805067, -0.07562255, 0.2800363, 0.8276317],
        ),
        (
            "arm_lower_r",
            Some("arm_upper_r"),
            [6.519258e-9, 0.2465017, -1.243316e-7],
            [0.08354503, 0.04766806, 0.1032522, 0.9899934],
        ),
        (
            "palm_r",
            Some("arm_lower_r"),
            [-9.313226e-9, 0.2002214, -9.173527e-8],
            [-0.06067554, -0.7130308, -0.03328465, 0.6977089],
        ),
        (
            "index_upper_r",
            Some("palm_r"),
            [0.0008180295, 0.1005081, 0.02715273],
            [0.09294701, -0.05117076, -0.0009619861, 0.9943548],
        ),
        (
            "index_lower_r",
            Some("index_upper_r"),
            [-1.280569e-8, 0.04113127, 9.313226e-10],
            [0.001259181, 0.02036116, 0.04058467, 0.9989679],
        ),
        (
            "middle_upper_r",
            Some("palm_r"),
            [0.0029122, 0.1017737, 0.01045302],
            [0.030626, -0.03408705, 0.01588584, 0.9988232],
        ),
        (
            "middle_lower_r",
            Some("middle_upper_r"),
            [1.172884e-8, 0.03964591, 4.190952e-9],
            [0.003874871, 0.008316914, 0.03015896, 0.999503],
        ),
        (
            "pinky_upper_r",
            Some("palm_r"),
            [0.003301455, 0.1018653, -0.02737972],
            [-0.08202796, 0.04654581, 0.02342361, 0.995267],
        ),
        (
            "pinky_lower_r",
            Some("pinky_upper_r"),
            [-1.396984e-9, 0.04137616, 4.656613e-9],
            [0.009484399, 0.0202966, 0.04156831, 0.9988845],
        ),
        (
            "ring_upper_r",
            Some("palm_r"),
            [0.003322977, 0.1026531, -0.01015224],
            [-0.005253638, 0.01441147, 0.01711468, 0.9997359],
        ),
        (
            "ring_lower_r",
            Some("ring_upper_r"),
            [-1.478475e-8, 0.03975944, -3.72529e-9],
            [-0.01118032, 0.01336839, 0.04056408, 0.999025],
        ),
        (
            "thumb_upper_r",
            Some("palm_r"),
            [-0.02432918, 0.02908554, 0.0255149],
            [0.1394166, 0.08989056, 0.444815, 0.8801263],
        ),
        (
            "thumb_lower_r",
            Some("thumb_upper_r"),
            [-3.032619e-8, 0.03369765, 5.587935e-9],
            [-0.069951, 0.6840793, -0.04850003, 0.724424],
        ),
        (
            "wep_pos_r",
            Some("palm_r"),
            [-0.01494596, 0.0863324, -0.0002042058],
            [0.6192576, 0.3128123, 0.3250258, 0.6426716],
        ),
    ];

    /// One shipped-rig row: joint, parent, bind offset, bind rotation.
    type RigRow = (&'static str, Option<&'static str>, [f32; 3], [f32; 4]);

    /// A potato body: bind-space points plus their four-influence rows.
    type Body = (
        Vec<[f32; 3]>,
        Vec<[u16; INFLUENCES]>,
        Vec<[f32; INFLUENCES]>,
    );

    /// The shipped body as the anim contract's skeleton plus its bind.
    fn hero_rig() -> (Skeleton, Vec<[f32; 3]>, Vec<[f32; 4]>) {
        let mut joints: Vec<Joint> = Vec::with_capacity(HERO_RIG.len());
        let mut positions = Vec::with_capacity(HERO_RIG.len());
        let mut rotations = Vec::with_capacity(HERO_RIG.len());
        for (name, parent, offset, rotation) in HERO_RIG {
            let parent = parent.map(|parent| {
                joints
                    .iter()
                    .position(|joint: &Joint| joint.name == parent)
                    .expect("the fixture is parent first")
            });
            joints.push(Joint {
                name: name.to_string(),
                parent,
            });
            positions.push(offset);
            rotations.push(rotation);
        }
        (Skeleton { joints }, positions, rotations)
    }

    /// Index of one joint of a rig this test built.
    fn joint_index(skeleton: &Skeleton, name: &str) -> usize {
        skeleton
            .joints
            .iter()
            .position(|joint| joint.name == name)
            .expect("the rig carries the joint")
    }

    /// First offset of `needle` inside `hay`, byte-wise.
    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || needle.len() > hay.len() {
            return None;
        }
        (0..=hay.len() - needle.len()).find(|start| &hay[*start..*start + needle.len()] == needle)
    }

    /// How many numbers a JSON array body holds, so a count needs no parser.
    fn count_json_numbers(list: &[u8]) -> usize {
        let mut count = 0usize;
        let mut in_number = false;
        for byte in list {
            if byte.is_ascii_digit() {
                if !in_number {
                    count += 1;
                    in_number = true;
                }
            } else {
                in_number = false;
            }
        }
        count
    }

    /// The shipped GLB's skin joint count, read from its JSON chunk. The glTF
    /// loader rides the `import` feature, so the default build reads the count
    /// only; the fixture above carries the rest.
    fn shipped_skin_joint_count() -> usize {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/fighters/glb/hero_body.glb");
        let bytes = std::fs::read(&path).expect("the shipped hero body is on disk");
        assert_eq!(bytes.get(..4), Some(&b"glTF"[..]), "the body is a GLB");
        let length = bytes.get(12..16).expect("a chunk header");
        let json_len = u32::from_le_bytes(length.try_into().expect("four bytes"));
        let json_len = usize::try_from(json_len).expect("a chunk length fits a usize");
        let json = bytes.get(20..20 + json_len).expect("the JSON chunk");
        let skin = find(json, b"\"skins\"").expect("the body has a skin");
        let head = skin + find(&json[skin..], b"\"joints\":[").expect("the skin lists joints") + 11;
        let tail = head
            + json[head..]
                .iter()
                .position(|byte| *byte == b']')
                .expect("the list closes");
        count_json_numbers(&json[head..tail])
    }

    /// Unit vector; a zero-length axis falls back to `+Y`.
    fn unit(vector: [f32; 3]) -> [f32; 3] {
        let squared = vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2];
        let length = squared.sqrt();
        if length < 1e-9 {
            [0.0, 1.0, 0.0]
        } else {
            [vector[0] / length, vector[1] / length, vector[2] / length]
        }
    }

    /// Cross product of two 3-vectors.
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// Difference of two 3-vectors.
    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    /// Distance between two 3-vectors, meters.
    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        let d = sub(a, b);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    /// Absolute dot product of two unit quaternions; `1.0` is the same turn.
    fn turn_dot(a: [f32; 4], b: [f32; 4]) -> f32 {
        (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs()
    }

    /// `count` points on a circle of `radius` around `center`, perpendicular to
    /// `axis`: the ring a limb's skin wraps a bone with.
    fn ring(center: [f32; 3], axis: [f32; 3], radius: f32, count: usize) -> Vec<[f32; 3]> {
        let reference = if axis[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u = unit(cross(axis, reference));
        let v = unit(cross(axis, u));
        let steps = f32::from(u8::try_from(count).expect("a ring has at most 255 points"));
        let mut out = Vec::with_capacity(count);
        for step in 0..count {
            let turn = std::f32::consts::TAU * f32::from(u8::try_from(step).expect("a step fits"));
            let angle = turn / steps;
            let (sin, cos) = (angle.sin(), angle.cos());
            out.push([
                center[0] + radius * (cos * u[0] + sin * v[0]),
                center[1] + radius * (cos * u[1] + sin * v[1]),
                center[2] + radius * (cos * u[2] + sin * v[2]),
            ]);
        }
        out
    }

    /// A potato body: a ring of vertices on every bone, weighted mostly to the
    /// bone with a share on its child, plus a ring at the joint itself split
    /// with the parent. That is the blend a shipped limb skin carries, so the
    /// fold is measured where influences cross a merged bone.
    fn potato_body(skeleton: &Skeleton, positions: &[[f32; 3]], rotations: &[[f32; 4]]) -> Body {
        let (world_pos, _) = world_transforms(skeleton, positions, rotations);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); skeleton.len()];
        for (index, joint) in skeleton.joints.iter().enumerate() {
            if let Some(parent) = joint.parent {
                children[parent].push(index);
            }
        }
        let mut points = Vec::new();
        let mut joints = Vec::new();
        let mut weights = Vec::new();
        for index in 0..skeleton.len() {
            let slot = u16::try_from(index).expect("a hero joint fits a u16");
            let axis = match skeleton.joints[index].parent {
                Some(parent) => unit(sub(world_pos[index], world_pos[parent])),
                None => [0.0, 1.0, 0.0],
            };
            let shared = match skeleton.joints[index].parent {
                Some(parent) => (slot, u16::try_from(parent).expect("a parent fits"), 0.5),
                None => (slot, slot, 0.0),
            };
            for point in ring(world_pos[index], axis, 0.03, 4) {
                points.push(point);
                joints.push([shared.0, shared.1, 0, 0]);
                weights.push([1.0 - shared.2, shared.2, 0.0, 0.0]);
            }
            for &child in &children[index] {
                let bone = sub(world_pos[child], world_pos[index]);
                let mid = [
                    world_pos[index][0] + 0.5 * bone[0],
                    world_pos[index][1] + 0.5 * bone[1],
                    world_pos[index][2] + 0.5 * bone[2],
                ];
                let child_slot = u16::try_from(child).expect("a child fits a u16");
                for point in ring(mid, unit(bone), 0.045, 6) {
                    points.push(point);
                    joints.push([slot, child_slot, 0, 0]);
                    weights.push([0.8, 0.2, 0.0, 0.0]);
                }
            }
        }
        (points, joints, weights)
    }

    /// A world transform set: position plus rotation per joint.
    struct Posed {
        positions: Vec<[f32; 3]>,
        rotations: Vec<[f32; 4]>,
    }

    /// Build one [`Posed`] from a rig and its rows.
    fn posed(skeleton: &Skeleton, positions: &[[f32; 3]], rotations: &[[f32; 4]]) -> Posed {
        let (positions, rotations) = world_transforms(skeleton, positions, rotations);
        Posed {
            positions,
            rotations,
        }
    }

    /// One vertex under one pose: the linear blend of its influences, each
    /// mapped through its joint's bind-relative skinning transform.
    fn skinned(
        point: [f32; 3],
        joints: &[u16; INFLUENCES],
        weights: &[f32; INFLUENCES],
        bind: &Posed,
        pose: &Posed,
    ) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for (joint, weight) in joints.iter().zip(weights.iter()) {
            if *weight <= 0.0 {
                continue;
            }
            let index = usize::from(*joint);
            let local = rotate_vec(
                &sub(point, bind.positions[index]),
                &quat_conj(&bind.rotations[index]),
            );
            let turned = rotate_vec(&local, &pose.rotations[index]);
            let base = pose.positions[index];
            out[0] += *weight * (base[0] + turned[0]);
            out[1] += *weight * (base[1] + turned[1]);
            out[2] += *weight * (base[2] + turned[2]);
        }
        out
    }

    /// Unit quaternion for one Euler turn about one axis, degrees in.
    fn axis_quat(axis: usize, degrees: f32) -> [f32; 4] {
        let half = 0.5 * degrees.to_radians();
        let (sin, cos) = (half.sin(), half.cos());
        match axis {
            0 => [sin, 0.0, 0.0, cos],
            1 => [0.0, sin, 0.0, cos],
            _ => [0.0, 0.0, sin, cos],
        }
    }

    /// The mid-swing pose, degrees per joint in the joint's own frame, applied
    /// Z * X * Y: a sword arm swinging through, the trunk twisted into it, the
    /// head watching, the legs braced and the grip closed. The grip is small
    /// because the shipped hand mesh is modelled around a hilt, so folding the
    /// fingers into the palm costs the tier almost nothing.
    fn mid_swing() -> [(&'static str, [f32; 3]); 38] {
        [
            ("torso", [0.0, 24.0, 0.0]),
            ("head", [6.0, -14.0, 0.0]),
            ("shoulder_l", [0.0, 0.0, 10.0]),
            ("shoulder_r", [0.0, 0.0, -10.0]),
            ("arm_upper_l", [74.0, 0.0, 0.0]),
            ("arm_lower_l", [56.0, 0.0, 0.0]),
            ("palm_l", [0.0, 18.0, 0.0]),
            ("arm_upper_r", [26.0, 0.0, 0.0]),
            ("arm_lower_r", [40.0, 0.0, 0.0]),
            ("palm_r", [0.0, -12.0, 0.0]),
            ("leg_upper_l", [20.0, 0.0, 0.0]),
            ("leg_lower_l", [-30.0, 0.0, 0.0]),
            ("foot_l", [11.0, 0.0, 0.0]),
            ("leg_upper_r", [-10.0, 0.0, 0.0]),
            ("leg_lower_r", [18.0, 0.0, 0.0]),
            ("foot_r", [-7.0, 0.0, 0.0]),
            ("toes_l", [10.0, 0.0, 0.0]),
            ("toes_r", [7.0, 0.0, 0.0]),
            ("index_upper_l", [12.0, 0.0, 0.0]),
            ("index_lower_l", [10.0, 0.0, 0.0]),
            ("middle_upper_l", [12.0, 0.0, 0.0]),
            ("middle_lower_l", [10.0, 0.0, 0.0]),
            ("ring_upper_l", [12.0, 0.0, 0.0]),
            ("ring_lower_l", [10.0, 0.0, 0.0]),
            ("pinky_upper_l", [12.0, 0.0, 0.0]),
            ("pinky_lower_l", [10.0, 0.0, 0.0]),
            ("thumb_upper_l", [10.0, 0.0, 0.0]),
            ("thumb_lower_l", [8.0, 0.0, 0.0]),
            ("index_upper_r", [12.0, 0.0, 0.0]),
            ("index_lower_r", [10.0, 0.0, 0.0]),
            ("middle_upper_r", [12.0, 0.0, 0.0]),
            ("middle_lower_r", [10.0, 0.0, 0.0]),
            ("ring_upper_r", [12.0, 0.0, 0.0]),
            ("ring_lower_r", [10.0, 0.0, 0.0]),
            ("pinky_upper_r", [12.0, 0.0, 0.0]),
            ("pinky_lower_r", [10.0, 0.0, 0.0]),
            ("thumb_upper_r", [10.0, 0.0, 0.0]),
            ("thumb_lower_r", [8.0, 0.0, 0.0]),
        ]
    }

    /// The shipped bind with the mid-swing turns applied in each joint's frame.
    fn hero_swing(
        skeleton: &Skeleton,
        positions: &[[f32; 3]],
        rotations: &[[f32; 4]],
    ) -> LocalPose {
        let mut pose = LocalPose {
            positions: positions.to_vec(),
            rotations: rotations.to_vec(),
        };
        for (name, degrees) in mid_swing() {
            let index = joint_index(skeleton, name);
            let turn = quat_mul(
                quat_mul(axis_quat(2, degrees[2]), axis_quat(0, degrees[0])),
                axis_quat(1, degrees[1]),
            );
            pose.rotations[index] = quat_mul(rotations[index], turn);
        }
        pose
    }

    #[test]
    fn folds_the_fighter_rig_and_holds_the_silhouette() {
        let (skeleton, positions, rotations) = hero_rig();
        validate_skeleton(&skeleton).expect("the shipped rig validates");
        assert_eq!(
            skeleton.len(),
            43,
            "the shipped hero body carries 43 skin joints"
        );
        assert_eq!(
            shipped_skin_joint_count(),
            skeleton.len(),
            "the fixture is the shipped rig"
        );

        let plan = plan_proxy(&skeleton, &PROXY_KEEP).expect("the fighter folds");
        assert_eq!(plan.len(), 15, "15 proxy bones");
        assert!(
            plan.len() <= MAX_PROXY_BONES,
            "the fold must fit the low tier"
        );
        // Leaf bones merge into their nearest kept ancestor.
        for (leaf, keeper) in [
            ("thumb_lower_l", "palm_l"),
            ("index_lower_r", "palm_r"),
            ("wep_pos_r", "palm_r"),
            ("toes_l", "foot_l"),
            ("heel_r", "foot_r"),
            ("shoulder_l", "torso"),
            ("head", "head"),
        ] {
            let slot = usize::from(plan.proxy_of[joint_index(&skeleton, leaf)]);
            assert_eq!(
                plan.kept[slot],
                joint_index(&skeleton, keeper),
                "{leaf} folds into {keeper}"
            );
        }

        let (points, joints, weights) = potato_body(&skeleton, &positions, &rotations);
        let rig = bake_proxy_rig(
            &skeleton,
            &positions,
            &rotations,
            &joints,
            &weights,
            &PROXY_KEEP,
        )
        .expect("the fighter bakes");
        validate_skeleton(&rig.skeleton).expect("the proxy rig validates");
        assert_eq!(rig.skin.len(), points.len());

        // The bind holds: every kept bone sits where the full rig put it.
        let bind = posed(&skeleton, &positions, &rotations);
        let proxy_bind = posed(&rig.skeleton, &rig.bind.positions, &rig.bind.rotations);
        for (slot, &full) in plan.kept.iter().enumerate() {
            let drift = distance(proxy_bind.positions[slot], bind.positions[full]);
            assert!(
                drift < 1e-4,
                "{} bind moved {drift} m",
                rig.skeleton.joints[slot].name
            );
            let turn = 1.0 - turn_dot(proxy_bind.rotations[slot], bind.rotations[full]);
            assert!(
                turn < 1e-5,
                "{} bind rotation turned {turn}",
                rig.skeleton.joints[slot].name
            );
        }

        // The pose holds too: a kept bone keeps its world transform.
        let pose = hero_swing(&skeleton, &positions, &rotations);
        validate_pose(&skeleton, &pose).expect("the swing pose is playable");
        let proxy_pose = fold_pose(&skeleton, &plan, &pose).expect("the pose folds");
        validate_pose(&rig.skeleton, &proxy_pose).expect("the proxy pose is playable");
        let full_posed = posed(&skeleton, &pose.positions, &pose.rotations);
        let proxy_posed = posed(&rig.skeleton, &proxy_pose.positions, &proxy_pose.rotations);
        for (slot, &full) in plan.kept.iter().enumerate() {
            let drift = distance(proxy_posed.positions[slot], full_posed.positions[full]);
            assert!(
                drift < 1e-4,
                "{} posed {drift} m off",
                rig.skeleton.joints[slot].name
            );
        }

        // The silhouette: the same vertices, skinned both ways under the swing.
        let count = f32::from(u16::try_from(points.len()).expect("the body fits a u16"));
        let mut total = 0.0f32;
        let mut worst = 0.0f32;
        let mut worst_bone = 0usize;
        for (vertex, point) in points.iter().enumerate() {
            let full_point = skinned(
                *point,
                &joints[vertex],
                &weights[vertex],
                &bind,
                &full_posed,
            );
            let proxy_point = skinned(
                *point,
                &rig.skin.joints[vertex],
                &rig.skin.weights[vertex],
                &proxy_bind,
                &proxy_posed,
            );
            let error = distance(full_point, proxy_point);
            total += error;
            if error > worst {
                worst = error;
                worst_bone = usize::from(rig.skin.joints[vertex][0]);
            }
        }
        let mean = total / count;
        eprintln!(
            "proxy_rig: source_joints={} proxy_bones={} vertices={} mean_error_cm={:.3} worst_error_cm={:.3} worst_bone={}",
            skeleton.len(),
            plan.len(),
            points.len(),
            mean * 100.0,
            worst * 100.0,
            rig.skeleton.joints[worst_bone].name
        );
        assert!(mean <= 0.02, "mean error {mean} m is over 2 cm");
        assert!(worst <= 0.05, "worst error {worst} m is over 5 cm");
    }

    #[test]
    fn the_far_proxy_palette_is_under_seventeen_and_a_pixel_at_sixty_metres() {
        let (skeleton, positions, rotations) = hero_rig();
        let (points, joints, weights) = potato_body(&skeleton, &positions, &rotations);
        let rig = bake_proxy_rig(
            &skeleton,
            &positions,
            &rotations,
            &joints,
            &weights,
            &PROXY_KEEP,
        )
        .expect("the fighter bakes");
        let (source, palette) = rig.palette();
        assert_eq!(source, 43, "the shipped hero body skins 43 joints");
        // The row asks for 17 roles; the shipped keep list carries 15 and the cap is 16. This
        // proves the palette the fold really produces, not a count it never reached.
        assert_eq!(palette, PROXY_KEEP.len(), "one proxy bone per kept name");
        assert!(
            palette <= MAX_PROXY_BONES,
            "a palette of {palette} bones over the low tier's {MAX_PROXY_BONES}"
        );

        let bind = posed(&skeleton, &positions, &rotations);
        let proxy_bind = posed(&rig.skeleton, &rig.bind.positions, &rig.bind.rotations);
        let pose = hero_swing(&skeleton, &positions, &rotations);
        let proxy_pose = fold_pose(&skeleton, &rig.plan, &pose).expect("the pose folds");
        let full_posed = posed(&skeleton, &pose.positions, &pose.rotations);
        let proxy_posed = posed(&rig.skeleton, &proxy_pose.positions, &proxy_pose.rotations);

        let count = f32::from(u16::try_from(points.len()).expect("the body fits a u16"));
        let mut total = 0.0f32;
        let mut worst = 0.0f32;
        for (vertex, point) in points.iter().enumerate() {
            let full_point = skinned(
                *point,
                &joints[vertex],
                &weights[vertex],
                &bind,
                &full_posed,
            );
            let proxy_point = skinned(
                *point,
                &rig.skin.joints[vertex],
                &rig.skin.weights[vertex],
                &proxy_bind,
                &proxy_posed,
            );
            let error = distance(full_point, proxy_point);
            total += error;
            if error > worst {
                worst = error;
            }
        }
        let mean = total / count;
        let mean_px = silhouette_px(mean, 60.0, FAR_VIEW_H, FAR_FOV_Y_DEG);
        let worst_px = silhouette_px(worst, 60.0, FAR_VIEW_H, FAR_FOV_Y_DEG);
        eprintln!(
            "proxy_rig far tier: source={source} palette={palette} mean_px={mean_px:.3} worst_px={worst_px:.3} mean_error_cm={:.3} worst_error_cm={:.3}",
            mean * 100.0,
            worst * 100.0
        );
        assert!(
            worst_px < 1.0,
            "silhouette error {worst} m is {worst_px} px at 60 m"
        );
    }

    #[test]
    fn the_silhouette_pixel_law_matches_the_camera_table() {
        let px_per_m = silhouette_px(1.0, 60.0, FAR_VIEW_H, FAR_FOV_Y_DEG);
        // 540 px of half-frame over tan(25 deg) * 60 m: 19.3 px per metre at 60 m.
        assert!(
            (px_per_m - 19.3).abs() < 0.05,
            "{px_per_m} px per metre at 60 m"
        );
        assert_eq!(
            silhouette_px(0.0, 60.0, FAR_VIEW_H, FAR_FOV_Y_DEG),
            0.0,
            "no world error is no pixels"
        );
        let one_px = silhouette_px(0.052, 60.0, FAR_VIEW_H, FAR_FOV_Y_DEG);
        assert!((one_px - 1.0).abs() < 0.05, "5.2 cm at 60 m is {one_px} px");
        // Linear in the world error, inverse in the distance: the two ways the law can drift.
        let doubled = silhouette_px(0.104, 60.0, FAR_VIEW_H, FAR_FOV_Y_DEG);
        assert!(
            (doubled - 2.0 * one_px).abs() < 1e-4,
            "twice the error is twice the pixels: {doubled}"
        );
        let nearer = silhouette_px(0.052, 30.0, FAR_VIEW_H, FAR_FOV_Y_DEG);
        assert!(
            (nearer - 2.0 * one_px).abs() < 1e-4,
            "half the distance doubles the pixels: {nearer}"
        );
    }

    #[test]
    fn bakes_the_proxy_rig_twice_to_equal_bytes() {
        let (skeleton, positions, rotations) = hero_rig();
        let (_, joints, weights) = potato_body(&skeleton, &positions, &rotations);
        let baked = || {
            bake_proxy_rig(
                &skeleton,
                &positions,
                &rotations,
                &joints,
                &weights,
                &PROXY_KEEP,
            )
            .expect("the fighter bakes")
        };
        let first = baked();
        let second = baked();
        assert_eq!(
            first.to_bytes(),
            second.to_bytes(),
            "the same rig bakes the same bytes"
        );
        assert_eq!(first.digest(), second.digest());
        // The image carries both counts, so a receipt reads them without the rig.
        let bytes = first.to_bytes();
        let proxy_count = u16::from_le_bytes(bytes[6..8].try_into().expect("two bytes"));
        let source_count = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes"));
        let vertex_count = u32::from_le_bytes(bytes[12..16].try_into().expect("four bytes"));
        assert_eq!(
            source_count, 43,
            "the bake records the full-rig joint count"
        );
        assert_eq!(proxy_count, 15, "the bake records the proxy bone count");
        assert_eq!(
            usize::try_from(vertex_count).expect("a vertex count fits"),
            first.skin.len()
        );
        assert!(
            bytes.len() > 15 * 8,
            "the image carries the bones and the rows"
        );
        // A different keep list is a different bake, never the same bytes.
        let other = bake_proxy_rig(
            &skeleton,
            &positions,
            &rotations,
            &joints,
            &weights,
            &["pelvis", "torso", "head"],
        )
        .expect("a smaller keep list still bakes");
        assert_ne!(other.to_bytes(), bytes);
    }

    #[test]
    fn refusals_name_the_problem() {
        let (skeleton, positions, rotations) = hero_rig();
        assert_eq!(
            plan_proxy(&skeleton, &["pelvis", "no_such_bone"]),
            Err(ProxyRigError::UnknownJoint { index: 1 })
        );
        assert_eq!(
            plan_proxy(&skeleton, &["torso", "head"]),
            Err(ProxyRigError::RootNotKept)
        );
        let mut crowded: Vec<&str> = PROXY_KEEP.to_vec();
        crowded.push("shoulder_l");
        crowded.push("shoulder_r");
        assert_eq!(
            plan_proxy(&skeleton, &crowded),
            Err(ProxyRigError::TooManyBones { count: 17 })
        );
        let plan = plan_proxy(&skeleton, &PROXY_KEEP).expect("the fighter folds");
        assert_eq!(
            proxy_bind(&skeleton, &positions[..42], &rotations, &plan),
            Err(ProxyRigError::BindMismatch {
                expected: 43,
                got: 42
            })
        );
        // A row that does not sum to one is refused by vertex.
        let (_, joints, weights) = potato_body(&skeleton, &positions, &rotations);
        let mut short = weights.clone();
        short[3] = [0.4, 0.4, 0.0, 0.0];
        assert_eq!(
            fold_weights(&plan, &joints, &short),
            Err(ProxyRigError::BadWeightSum { vertex: 3 })
        );
        // A row past the rig is refused by joint.
        let mut far = joints.clone();
        far[5] = [99, 0, 0, 0];
        assert_eq!(
            fold_weights(&plan, &far, &weights),
            Err(ProxyRigError::JointOutOfRange {
                vertex: 5,
                joint: 99
            })
        );
        // Four influences that all land on one bone fold to one weight of 1.
        let palm = u16::try_from(joint_index(&skeleton, "palm_l")).expect("a joint fits");
        let thumb = u16::try_from(joint_index(&skeleton, "thumb_lower_l")).expect("a joint fits");
        let palm_slot = plan.proxy_of[usize::from(palm)];
        let folded = fold_weights(&plan, &[[palm, thumb, palm, thumb]], &[[0.25; INFLUENCES]])
            .expect("the rows fold");
        assert_eq!(folded.joints, vec![[palm_slot, 0, 0, 0]]);
        assert!(
            (folded.weights[0][0] - 1.0).abs() < 1e-6,
            "{:?}",
            folded.weights
        );
    }
}
