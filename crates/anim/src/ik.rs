//! ik: two-bone analytic with a pole vector, the same solve over a local
//! pose, FABRIK fixed-iter and the foot-lock blend.
//!
//! Owner: `engine-anim`. Consumers: [`crate::feet`] and [`crate::fbik`] solve
//! graph poses through [`two_bone`]; the fighter look plants a clip pose's feet
//! through [`solve_limb`]. Units: meters, radians, unit quaternions
//! `[x, y, z, w]`. Determinism: `+ - * /` and `sqrt` only, no clock, so equal
//! inputs give equal bits. Errors: [`IkError`]; a refused call writes nothing.
//! Heap: none per call, every lane lives on the stack.

use super::contract::{rotate, LocalPose, Skeleton};
use engine_core::quat_mul;
use engine_core::vec3::cross3;

/// Shortest bone the solvers accept, in meters; a shorter one has no length
/// to reach with and is refused.
pub const MIN_BONE_M: f32 = 1.0e-6;
/// Shortest root-to-end reach the solver aims for, in meters: a folded chain
/// of two equal bones still has a defined bend.
const MIN_REACH_M: f32 = 1.0e-6;
/// Share of a vector's length its perpendicular part must keep to name a
/// bend side (a thousandth, about 0.06 degrees off the axis).
const SIDE_MIN_SHARE: f32 = 1.0e-3;

/// Why a two-bone or limb solve refused. A refused call changes nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum IkError {
    /// A bone is shorter than [`MIN_BONE_M`]: `0` is root-to-mid, `1` is
    /// mid-to-end.
    DegenerateBone(u8),
    /// Non-finite float in the named input.
    NonFinite(&'static str),
    /// The pole is the zero vector, so no side is named for the bend.
    ZeroPole,
    /// Joint index past the skeleton; value carried.
    BadJoint(usize),
    /// The three joints are not a parent-child chain (mid under root, end
    /// under mid).
    NotAChain,
    /// The pose does not have one lane per skeleton joint.
    PoseLength {
        /// Joints in the skeleton.
        skeleton: usize,
        /// Shorter of the pose's two lanes.
        pose: usize,
    },
}

impl core::fmt::Display for IkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IkError::DegenerateBone(bone) => {
                write!(f, "ik: bone {bone} is shorter than {MIN_BONE_M} m")
            }
            IkError::NonFinite(lane) => write!(f, "ik: non-finite float in {lane}"),
            IkError::ZeroPole => write!(f, "ik: pole vector has no length"),
            IkError::BadJoint(joint) => write!(f, "ik: joint {joint} past the skeleton"),
            IkError::NotAChain => write!(f, "ik: joints are not a parent-child chain"),
            IkError::PoseLength { skeleton, pose } => {
                write!(f, "ik: pose lane has {pose} joints, skeleton {skeleton}")
            }
        }
    }
}

impl std::error::Error for IkError {}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = len(v);
    if l <= 1e-9 {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// `base + dir * k`.
fn madd(base: [f32; 3], dir: [f32; 3], k: f32) -> [f32; 3] {
    [
        base[0] + dir[0] * k,
        base[1] + dir[1] * k,
        base[2] + dir[2] * k,
    ]
}
fn finite3(v: [f32; 3]) -> bool {
    v[0].is_finite() && v[1].is_finite() && v[2].is_finite()
}
fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}
/// `q` scaled to unit length.
fn unit(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n.is_finite() && n > 0.0 {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    } else {
        [q[0], q[1], q[2], q[3]]
    }
}

/// Unit part of `v` perpendicular to the axis `dir`, or `None` when `v` has
/// less than [`SIDE_MIN_SHARE`] of its own length off the axis.
fn side_of(v: [f32; 3], dir: [f32; 3]) -> Option<[f32; 3]> {
    let side = madd(v, dir, -dot(v, dir));
    let (off, whole) = (len(side), len(v));
    (off > SIDE_MIN_SHARE * whole && off > 1e-9).then(|| norm(side))
}

/// The direction the mid joint leaves the root-to-end axis in: the pole's
/// perpendicular part, else the current bend's, else any perpendicular.
fn bend_axis(dir: [f32; 3], pole: [f32; 3], current: [f32; 3]) -> [f32; 3] {
    side_of(pole, dir)
        .or_else(|| side_of(current, dir))
        .unwrap_or_else(|| {
            let spare = if dir[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            side_of(spare, dir).unwrap_or([0.0, 0.0, 1.0])
        })
}

/// Two-bone analytic with a pole vector: bones `root -> mid -> end` keep their
/// current lengths (law of cosines), `end` lands on `target`, and `mid` bends
/// toward `pole` (the part of `pole` perpendicular to the root-to-target axis;
/// a pole along the axis falls back to the chain's current bend, then to any
/// perpendicular). A target past full extension stretches the chain straight
/// at `target`'s direction; one inside the folded chain stops at the fold.
/// Returns the new `(mid, end)`.
///
/// Units: meters; `pole` is a direction in the same space, any length above
/// zero. Errors: a bone shorter than [`MIN_BONE_M`], a zero pole or a
/// non-finite input refuse loudly. No allocation.
pub fn two_bone_pole(
    root: [f32; 3],
    mid: [f32; 3],
    end: [f32; 3],
    target: [f32; 3],
    pole: [f32; 3],
) -> Result<([f32; 3], [f32; 3]), IkError> {
    for (name, lane) in [
        ("root", root),
        ("mid", mid),
        ("end", end),
        ("target", target),
        ("pole", pole),
    ] {
        if !finite3(lane) {
            return Err(IkError::NonFinite(name));
        }
    }
    let (l1, l2) = (len(sub(mid, root)), len(sub(end, mid)));
    if l1 < MIN_BONE_M {
        return Err(IkError::DegenerateBone(0));
    }
    if l2 < MIN_BONE_M {
        return Err(IkError::DegenerateBone(1));
    }
    if len(pole) <= 1e-9 {
        return Err(IkError::ZeroPole);
    }
    let to_target = sub(target, root);
    let dist = len(to_target);
    // A target on the root has no direction: keep the chain's own axis.
    let dir = if dist > MIN_REACH_M {
        norm(to_target)
    } else {
        norm(sub(end, root))
    };
    let d = dist.clamp((l1 - l2).abs().max(MIN_REACH_M), l1 + l2);
    // Law of cosines: the mid joint sits `a` along the axis and `h` off it.
    let a = ((l1 * l1 - l2 * l2 + d * d) / (2.0 * d)).clamp(-l1, l1);
    let h = (l1 * l1 - a * a).max(0.0).sqrt();
    let bend = bend_axis(dir, pole, sub(mid, root));
    let mid_p = madd(madd(root, dir, a), bend, h);
    Ok((mid_p, madd(root, dir, d)))
}

/// Two-bone analytic that keeps the chain's current bend side: the pole is the
/// mid joint's own offset from the root. Clamps unreachable targets to max
/// reach; keeps the current mid and end when the chain is degenerate or any
/// input is non-finite. Returns `(mid, end)`.
pub fn two_bone(
    root: [f32; 3],
    mid: [f32; 3],
    end: [f32; 3],
    target: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    two_bone_pole(root, mid, end, target, sub(mid, root)).unwrap_or((mid, end))
}

/// Shortest-arc unit quaternion that turns direction `from` onto `to`
/// (any lengths). A zero-length input gives identity; opposite directions
/// turn half a circle about an axis perpendicular to both.
#[must_use]
pub fn turn_between(from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    let (a, b) = (norm(from), norm(to));
    if len(a) == 0.0 || len(b) == 0.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let d = dot(a, b);
    if d < -1.0 + 1.0e-6 {
        let axis = bend_axis(a, [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        return [axis[0], axis[1], axis[2], 0.0];
    }
    let c = cross3(a, b);
    unit([c[0], c[1], c[2], 1.0 + d])
}

/// Three joints of one limb inside a skeleton: the root joint (hip,
/// shoulder), the bending joint (knee, elbow) and the end joint (ankle,
/// wrist), each the parent of the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limb {
    /// Hip or shoulder joint index.
    pub root: usize,
    /// Knee or elbow joint index; a child of `root`.
    pub mid: usize,
    /// Ankle or wrist joint index; a child of `mid`.
    pub end: usize,
}

impl Limb {
    /// The limb over `skeleton`, or the reason the joints do not form one.
    /// Cold path: call once at load.
    pub fn new(skeleton: &Skeleton, root: usize, mid: usize, end: usize) -> Result<Self, IkError> {
        let limb = Self { root, mid, end };
        limb.check(skeleton)?;
        Ok(limb)
    }

    fn check(&self, skeleton: &Skeleton) -> Result<(), IkError> {
        for joint in [self.root, self.mid, self.end] {
            if joint >= skeleton.len() {
                return Err(IkError::BadJoint(joint));
            }
        }
        let chain = skeleton.joints[self.mid].parent == Some(self.root)
            && skeleton.joints[self.end].parent == Some(self.mid);
        if chain {
            Ok(())
        } else {
            Err(IkError::NotAChain)
        }
    }
}

/// A limb's model-space frames: the world position and rotation of each joint.
struct LimbFrame {
    root_p: [f32; 3],
    root_q: [f32; 4],
    mid_p: [f32; 3],
    mid_q: [f32; 4],
    end_p: [f32; 3],
    end_q: [f32; 4],
}

/// World position and rotation of `joint`: its local transform composed up
/// through every ancestor, parents applying first (the order
/// [`super::contract::skinning_matrices`] uses).
fn world_of(
    skeleton: &Skeleton,
    pose: &LocalPose,
    joint: usize,
) -> Result<([f32; 3], [f32; 4]), IkError> {
    let (mut p, mut q) = (pose.positions[joint], pose.rotations[joint]);
    let mut at = joint;
    for _ in 0..skeleton.len() {
        let Some(parent) = skeleton.joints[at].parent else {
            return Ok((p, q));
        };
        let turned = rotate(&p, &pose.rotations[parent]);
        let origin = pose.positions[parent];
        p = [
            origin[0] + turned[0],
            origin[1] + turned[1],
            origin[2] + turned[2],
        ];
        q = quat_mul(pose.rotations[parent], q);
        at = parent;
    }
    // More parents than joints: the parent links loop.
    Err(IkError::NotAChain)
}

fn limb_frame(skeleton: &Skeleton, pose: &LocalPose, limb: Limb) -> Result<LimbFrame, IkError> {
    let joints = skeleton.len();
    let lanes = pose.positions.len().min(pose.rotations.len());
    if pose.positions.len() != joints || pose.rotations.len() != joints {
        return Err(IkError::PoseLength {
            skeleton: joints,
            pose: lanes,
        });
    }
    limb.check(skeleton)?;
    let (root_p, root_q) = world_of(skeleton, pose, limb.root)?;
    let step = |p: [f32; 3], q: [f32; 4], joint: usize| {
        let turned = rotate(&pose.positions[joint], &q);
        (
            [p[0] + turned[0], p[1] + turned[1], p[2] + turned[2]],
            quat_mul(q, pose.rotations[joint]),
        )
    };
    let (mid_p, mid_q) = step(root_p, root_q, limb.mid);
    let (end_p, end_q) = step(mid_p, mid_q, limb.end);
    Ok(LimbFrame {
        root_p,
        root_q,
        mid_p,
        mid_q,
        end_p,
        end_q,
    })
}

/// The limb's joint positions in the pose's model space: root, mid, end.
/// Errors: indices off the skeleton, a broken chain or a pose with the wrong
/// lane count. No allocation.
pub fn limb_positions(
    skeleton: &Skeleton,
    pose: &LocalPose,
    limb: Limb,
) -> Result<[[f32; 3]; 3], IkError> {
    let frame = limb_frame(skeleton, pose, limb)?;
    Ok([frame.root_p, frame.mid_p, frame.end_p])
}

/// Solve one limb of a local pose in place: the end joint reaches `target`
/// (model space) through [`two_bone_pole`], the root and mid joints turn by
/// the shortest arcs that put the bones there, and the end joint keeps its
/// world orientation (a foot stays flat). Bone offsets never change, so the
/// bone lengths stay exact. Other joints keep their local transforms; the
/// limb's children follow it.
///
/// Errors: everything [`two_bone_pole`] refuses, plus [`IkError::BadJoint`],
/// [`IkError::NotAChain`] and [`IkError::PoseLength`]; on any error the pose
/// is untouched. No allocation.
pub fn solve_limb(
    skeleton: &Skeleton,
    pose: &mut LocalPose,
    limb: Limb,
    target: [f32; 3],
    pole: [f32; 3],
) -> Result<(), IkError> {
    let f = limb_frame(skeleton, pose, limb)?;
    let (mid, end) = two_bone_pole(f.root_p, f.mid_p, f.end_p, target, pole)?;
    // The root joint swings the old mid onto the new one; the mid joint then
    // swings what the root carried along onto the new end.
    let swing_root = turn_between(sub(f.mid_p, f.root_p), sub(mid, f.root_p));
    let carried = rotate(&sub(f.end_p, f.mid_p), &swing_root);
    let swing_mid = turn_between(carried, sub(end, mid));
    let root_q = quat_mul(swing_root, f.root_q);
    let mid_q = quat_mul(swing_mid, quat_mul(swing_root, f.mid_q));
    let parent_q = quat_mul(f.root_q, conj(pose.rotations[limb.root]));
    let locals = [
        unit(quat_mul(conj(parent_q), root_q)),
        unit(quat_mul(conj(root_q), mid_q)),
        unit(quat_mul(conj(mid_q), f.end_q)),
    ];
    if locals.iter().flatten().any(|lane| !lane.is_finite()) {
        return Err(IkError::NonFinite("rotation"));
    }
    pose.rotations[limb.root] = locals[0];
    pose.rotations[limb.mid] = locals[1];
    pose.rotations[limb.end] = locals[2];
    Ok(())
}

/// FABRIK fixed-iter on a joint chain. joints mutated toward target.
/// Returns iterations used (<= max_iter). Deterministic, no alloc.
pub fn fabrik(joints: &mut [[f32; 3]], target: [f32; 3], max_iter: u32, tol: f32) -> u32 {
    if joints.len() < 2 {
        return 0;
    }
    let n = joints.len();
    let root = joints[0];
    let mut lens = [0.0f32; 16];
    let m = (n - 1).min(16);
    let mut total = 0.0;
    for i in 0..m {
        lens[i] = len(sub(joints[i + 1], joints[i]));
        total += lens[i];
    }
    if total <= 1e-9 {
        return 0;
    }
    let mut used = 0;
    for it in 0..max_iter.min(64) {
        used = it + 1;
        // forward: end -> target
        joints[n - 1] = target;
        for i in (0..n - 1).rev() {
            let l = if i < 16 {
                lens[i]
            } else {
                len(sub(joints[i + 1], joints[i]))
            };
            let d = sub(joints[i + 1], joints[i]);
            let dl = len(d);
            if dl > 1e-9 {
                let f = l / dl;
                joints[i] = [
                    joints[i + 1][0] - d[0] * f,
                    joints[i + 1][1] - d[1] * f,
                    joints[i + 1][2] - d[2] * f,
                ];
            }
        }
        // backward: root pinned
        joints[0] = root;
        for i in 0..n - 1 {
            let l = if i < 16 {
                lens[i]
            } else {
                len(sub(joints[i + 1], joints[i]))
            };
            let d = sub(joints[i + 1], joints[i]);
            let dl = len(d);
            if dl > 1e-9 {
                let f = l / dl;
                joints[i + 1] = [
                    joints[i][0] + d[0] * f,
                    joints[i][1] + d[1] * f,
                    joints[i][2] + d[2] * f,
                ];
            }
        }
        if len(sub(joints[n - 1], target)) <= tol {
            break;
        }
    }
    used
}

/// Foot lock: blend foot toward locked pos when within lock_radius, full lock inside.
/// Returns locked foot pos. Deterministic.
pub fn foot_lock(current: [f32; 3], locked: [f32; 3], lock_radius: f32) -> [f32; 3] {
    let d = len(sub(current, locked));
    if d <= 1e-9 || d >= lock_radius {
        return current;
    }
    let f = 1.0 - d / lock_radius; // 1 at lock point
    [
        current[0] + (locked[0] - current[0]) * f,
        current[1] + (locked[1] - current[1]) * f,
        current[2] + (locked[2] - current[2]) * f,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Joint;
    use engine_core::alloc_probe::measure;

    #[test]
    fn two_bone_reaches() {
        let (mid, end) = two_bone(
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -2.0, 0.0],
            [1.0, -1.0, 0.0],
        );
        let d = len(sub(end, [1.0, -1.0, 0.0]));
        assert!(d < 0.05, "end {end:?} d={d}");
        assert!((len(sub(mid, [0.0, 0.0, 0.0])) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn two_bone_clamps_unreachable() {
        let (_, end) = two_bone(
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -2.0, 0.0],
            [0.0, -99.0, 0.0],
        );
        assert!((len(sub(end, [0.0, 0.0, 0.0])) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn fabrik_converges_capped() {
        let mut j = [
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -2.0, 0.0],
            [0.0, -3.0, 0.0],
        ];
        let used = fabrik(&mut j, [1.0, -2.0, 0.0], 16, 1e-4);
        assert!(used <= 16);
        assert!(len(sub(j[3], [1.0, -2.0, 0.0])) < 0.05, "{j:?}");
        assert_eq!(j[0], [0.0, 0.0, 0.0]); // root pinned
    }

    #[test]
    fn foot_lock_holds() {
        let l = [1.0, 0.0, 0.0];
        assert_eq!(foot_lock([5.0, 0.0, 0.0], l, 1.0), [5.0, 0.0, 0.0]); // outside
        let near = foot_lock([1.3, 0.0, 0.0], l, 1.0);
        assert!(len(sub(near, l)) < 0.3, "{near:?}");
    }

    /// A straight leg hanging from the origin: hip, knee 0.5 m down, ankle
    /// 0.4 m below that. Bones of unequal length, no bend yet.
    const HIP: [f32; 3] = [0.0, 0.0, 0.0];
    const KNEE: [f32; 3] = [0.0, -0.5, 0.0];
    const ANKLE: [f32; 3] = [0.0, -0.9, 0.0];
    /// Knee forward (+z), as a leg bends.
    const FORWARD: [f32; 3] = [0.0, 0.0, 1.0];
    /// Chain length of the fixture leg.
    const REACH: f32 = 0.9;

    #[test]
    fn two_bone_pole_reaches_nine_tenths_of_the_chain_and_keeps_the_pole_side() {
        for dir in [
            [0.0, -1.0, 0.0],
            [0.5, -1.0, 0.2],
            [-0.3, -0.6, 0.8],
            [1.0, -0.2, 0.0],
            [0.2, 1.0, -0.4],
        ] {
            let target = madd(HIP, norm(dir), 0.9 * REACH);
            for pole in [FORWARD, [0.0, 0.0, -1.0]] {
                let (mid, end) = two_bone_pole(HIP, KNEE, ANKLE, target, pole)
                    .unwrap_or_else(|err| panic!("dir {dir:?}: {err}"));
                let miss = len(sub(end, target));
                assert!(miss < 1.0e-3, "dir {dir:?}: end misses by {miss} m");
                assert!((len(sub(mid, HIP)) - 0.5).abs() < 1.0e-4, "thigh length");
                assert!((len(sub(end, mid)) - 0.4).abs() < 1.0e-4, "shin length");
                // The mid joint leaves the axis on the pole's side.
                let axis = norm(sub(end, HIP));
                let off = sub(mid, madd(HIP, axis, dot(sub(mid, HIP), axis)));
                let want = sub(pole, madd([0.0; 3], axis, dot(pole, axis)));
                assert!(
                    dot(off, want) > 0.0,
                    "dir {dir:?} pole {pole:?}: mid {mid:?} is on the wrong side"
                );
            }
        }
        // The two poles bend opposite ways: the check above can fail.
        let target = [0.0, -0.8, 0.0];
        let front = two_bone_pole(HIP, KNEE, ANKLE, target, FORWARD).expect("front");
        let back = two_bone_pole(HIP, KNEE, ANKLE, target, [0.0, 0.0, -1.0]).expect("back");
        assert!(front.0[2] > 0.1 && back.0[2] < -0.1, "{front:?} {back:?}");
    }

    #[test]
    fn two_bone_pole_stretches_to_full_extension_without_nan() {
        for far in [1.0f32, 2.0, 50.0, 1.0e6] {
            let target = [0.4 * far, -0.8 * far, 0.2 * far];
            let (mid, end) =
                two_bone_pole(HIP, KNEE, ANKLE, target, FORWARD).expect("far target solves");
            assert!(finite3(mid) && finite3(end), "far {far}: {mid:?} {end:?}");
            let reach = len(sub(end, HIP));
            assert!(
                (reach - REACH).abs() < 1.0e-4,
                "far {far}: reach {reach}, want {REACH}"
            );
            // Straight at the target's direction.
            let aim = dot(norm(sub(end, HIP)), norm(target));
            assert!(aim > 0.999_99, "far {far}: aim {aim}");
        }
        // A target on top of the root folds the chain, still finite.
        let (mid, end) = two_bone_pole(HIP, KNEE, ANKLE, HIP, FORWARD).expect("on the root");
        assert!(finite3(mid) && finite3(end), "{mid:?} {end:?}");
        assert!(
            (len(sub(end, HIP)) - 0.1).abs() < 1.0e-4,
            "folded to |l1-l2|"
        );
    }

    #[test]
    fn two_bone_pole_names_the_side_of_a_straight_chain() {
        // The chain is straight, so only the pole can say which way it bends;
        // a pole along the axis falls back to the chain's own bend or any
        // perpendicular, never NaN.
        let target = [0.0, -0.6, 0.0];
        let (mid, _) = two_bone_pole(HIP, KNEE, ANKLE, target, [0.3, -5.0, 1.0]).expect("tilted");
        assert!(mid[2] > 0.1, "{mid:?}");
        let (mid, end) =
            two_bone_pole(HIP, KNEE, ANKLE, target, [0.0, -1.0, 0.0]).expect("pole on the axis");
        assert!(finite3(mid) && len(sub(end, target)) < 1.0e-4, "{mid:?}");
        let bent = [0.0, -0.45, 0.2];
        let (mid, _) = two_bone_pole(HIP, bent, [0.0, -0.8, 0.0], target, [0.0, -1.0, 0.0])
            .expect("pole on the axis, chain bent");
        assert!(mid[2] > 0.1, "a bent chain keeps its own side: {mid:?}");
    }

    #[test]
    fn two_bone_pole_refuses_degenerate_chains_and_bad_numbers() {
        let t = [0.1, -0.5, 0.0];
        assert_eq!(
            two_bone_pole(HIP, HIP, ANKLE, t, FORWARD),
            Err(IkError::DegenerateBone(0))
        );
        assert_eq!(
            two_bone_pole(HIP, KNEE, KNEE, t, FORWARD),
            Err(IkError::DegenerateBone(1))
        );
        assert_eq!(
            two_bone_pole(HIP, KNEE, ANKLE, t, [0.0; 3]),
            Err(IkError::ZeroPole)
        );
        assert_eq!(
            two_bone_pole(HIP, KNEE, ANKLE, [f32::NAN, 0.0, 0.0], FORWARD),
            Err(IkError::NonFinite("target"))
        );
        assert_eq!(
            two_bone_pole(HIP, [0.0, f32::INFINITY, 0.0], ANKLE, t, FORWARD),
            Err(IkError::NonFinite("mid"))
        );
        assert_eq!(
            two_bone_pole(HIP, KNEE, ANKLE, t, [0.0, f32::NAN, 1.0]),
            Err(IkError::NonFinite("pole"))
        );
        // The pole-less entry keeps the chain it was given instead.
        assert_eq!(two_bone(HIP, HIP, ANKLE, t), (HIP, ANKLE));
        assert_eq!(
            two_bone(HIP, KNEE, ANKLE, [f32::NAN, 0.0, 0.0]),
            (KNEE, ANKLE)
        );
    }

    #[test]
    fn turn_between_carries_one_direction_onto_the_other() {
        for (from, to) in [
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ([0.3, -1.0, 0.2], [-0.5, -0.2, 0.9]),
            ([0.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [0.0, -1.0, 0.0]),
            ([1.0, 0.0, 0.0], [-1.0, 1.0e-4, 0.0]),
        ] {
            let q = turn_between(from, to);
            let norm_q = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!((norm_q - 1.0).abs() < 1.0e-5, "{from:?}: |q| {norm_q}");
            let got = rotate(&norm(from), &q);
            let miss = len(sub(got, norm(to)));
            assert!(miss < 1.0e-3, "{from:?} -> {to:?}: lands {got:?}");
        }
        assert_eq!(
            turn_between([0.0; 3], [1.0, 0.0, 0.0]),
            [0.0, 0.0, 0.0, 1.0]
        );
    }

    /// A pelvis, a hip, a knee, an ankle and a toe, with tilts so no joint
    /// frame is the identity. The knee leads forward (+z) of the hip-ankle line.
    fn leg() -> (Skeleton, LocalPose, Limb) {
        let names = ["pelvis", "hip", "knee", "ankle", "toe"];
        let parents = [None, Some(0), Some(1), Some(2), Some(3)];
        let skeleton = Skeleton {
            joints: names
                .iter()
                .zip(parents)
                .map(|(name, parent)| Joint {
                    name: (*name).to_string(),
                    parent,
                })
                .collect(),
        };
        let about = |axis: [f32; 3], angle: f32| {
            let (s, c) = (angle * 0.5).sin_cos();
            let n = norm(axis);
            [n[0] * s, n[1] * s, n[2] * s, c]
        };
        let pose = LocalPose {
            positions: vec![
                [0.1, 0.95, -0.05],
                [0.1, -0.05, 0.0],
                [0.0, -0.38, 0.2],
                [0.0, -0.38, -0.2],
                [0.0, -0.05, 0.12],
            ],
            rotations: vec![
                about([1.0, 0.0, 0.0], 0.25),
                about([0.0, 1.0, 0.0], 0.3),
                about([1.0, 0.0, 0.0], -0.2),
                about([0.2, 0.0, 1.0], 0.15),
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let limb = Limb::new(&skeleton, 1, 2, 3).expect("the fixture is a chain");
        (skeleton, pose, limb)
    }

    #[test]
    fn solve_limb_puts_the_end_on_target_and_keeps_bones_side_and_orientation() {
        let (skeleton, pose, limb) = leg();
        let before = limb_positions(&skeleton, &pose, limb).expect("fk");
        let (hip, knee, ankle) = (before[0], before[1], before[2]);
        let (l1, l2) = (len(sub(knee, hip)), len(sub(ankle, knee)));
        let (_, end_q_before) = world_of(&skeleton, &pose, limb.end).expect("fk");
        let toe = world_of(&skeleton, &pose, 4).expect("fk").0;
        for (shift, label) in [
            ([0.12, 0.0, 0.0], "sideways"),
            ([0.0, 0.1, 0.0], "up"),
            ([-0.05, 0.02, 0.15], "forward"),
        ] {
            let target = [
                ankle[0] + shift[0],
                ankle[1] + shift[1],
                ankle[2] + shift[2],
            ];
            assert!(
                len(sub(target, hip)) < 0.95 * (l1 + l2),
                "{label}: the fixture target must be reachable"
            );
            let mut solved = pose.clone();
            solve_limb(&skeleton, &mut solved, limb, target, FORWARD).expect(label);
            let after = limb_positions(&skeleton, &solved, limb).expect("fk");
            assert!(
                len(sub(after[2], target)) < 1.0e-3,
                "{label}: ankle {:?} misses {target:?}",
                after[2]
            );
            assert!(
                (len(sub(after[1], after[0])) - l1).abs() < 1.0e-4,
                "{label}"
            );
            assert!(
                (len(sub(after[2], after[1])) - l2).abs() < 1.0e-4,
                "{label}"
            );
            assert_eq!(after[0], hip, "{label}: the hip does not move");
            // The knee stays in front of the hip-ankle line.
            let axis = norm(sub(after[2], after[0]));
            let off = sub(
                after[1],
                madd(after[0], axis, dot(sub(after[1], after[0]), axis)),
            );
            assert!(off[2] > 0.0, "{label}: knee {:?} swung behind", after[1]);
            // The ankle keeps its world orientation; the toe follows it.
            let (_, end_q) = world_of(&skeleton, &solved, limb.end).expect("fk");
            let same: f32 = end_q.iter().zip(end_q_before).map(|(a, b)| a * b).sum();
            assert!(same.abs() > 0.999_99, "{label}: ankle turned, dot {same}");
            let toe_after = world_of(&skeleton, &solved, 4).expect("fk").0;
            let toe_gap = sub(toe_after, after[2]);
            let toe_was = sub(toe, ankle);
            assert!(len(sub(toe_gap, toe_was)) < 1.0e-4, "{label}: toe offset");
            for q in &solved.rotations {
                let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
                assert!((n - 1.0).abs() < 1.0e-5, "{label}: rotation norm {n}");
            }
            assert_eq!(solved.positions, pose.positions, "{label}: offsets stay");
        }
    }

    #[test]
    fn solve_limb_refuses_loudly_and_leaves_the_pose_alone() {
        let (skeleton, pose, limb) = leg();
        let ankle = limb_positions(&skeleton, &pose, limb).expect("fk")[2];
        let mut probe = pose.clone();
        assert_eq!(
            solve_limb(&skeleton, &mut probe, limb, [f32::NAN, 0.0, 0.0], FORWARD),
            Err(IkError::NonFinite("target"))
        );
        assert_eq!(
            solve_limb(&skeleton, &mut probe, limb, ankle, [0.0; 3]),
            Err(IkError::ZeroPole)
        );
        let off = Limb {
            root: 1,
            mid: 2,
            end: 9,
        };
        assert_eq!(
            solve_limb(&skeleton, &mut probe, off, ankle, FORWARD),
            Err(IkError::BadJoint(9))
        );
        let skewed = Limb {
            root: 1,
            mid: 3,
            end: 4,
        };
        assert_eq!(
            solve_limb(&skeleton, &mut probe, skewed, ankle, FORWARD),
            Err(IkError::NotAChain)
        );
        assert_eq!(Limb::new(&skeleton, 2, 3, 1), Err(IkError::NotAChain));
        let mut short = LocalPose {
            positions: pose.positions[..3].to_vec(),
            rotations: pose.rotations[..3].to_vec(),
        };
        assert_eq!(
            solve_limb(&skeleton, &mut short, limb, ankle, FORWARD),
            Err(IkError::PoseLength {
                skeleton: 5,
                pose: 3
            })
        );
        // A zero-length bone is a loud refusal too.
        let mut folded = pose.clone();
        folded.positions[3] = [0.0; 3];
        assert_eq!(
            solve_limb(&skeleton, &mut folded, limb, ankle, FORWARD),
            Err(IkError::DegenerateBone(1))
        );
        assert_eq!(probe, pose, "refusals write nothing");
        assert!(IkError::ZeroPole.to_string().contains("pole"));
    }

    #[test]
    fn solving_a_limb_allocates_nothing_and_repeats_bit_for_bit() {
        let (_, seen) = measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls > 0, "the probe must see a real allocation");
        let (skeleton, pose, limb) = leg();
        let mut a = pose.clone();
        let mut b = pose.clone();
        let (res, heap) = measure(|| {
            let mut out = Ok(());
            for k in 0..500u16 {
                let t = f32::from(k) * 0.001;
                out = solve_limb(&skeleton, &mut a, limb, [0.1 + t, 0.4, 0.05], FORWARD);
                std::hint::black_box(&a);
            }
            out
        });
        assert_eq!(res, Ok(()));
        assert_eq!(heap.calls, 0, "500 limb solves touch no heap");
        for k in 0..500u16 {
            let t = f32::from(k) * 0.001;
            solve_limb(&skeleton, &mut b, limb, [0.1 + t, 0.4, 0.05], FORWARD).expect("solve");
        }
        assert_eq!(a, b, "the same inputs give the same bits");
    }
}
