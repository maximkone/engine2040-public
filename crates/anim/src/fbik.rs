//! fbik: full-body IK for the combat reach/aim pipeline.
//!
//! Owner: `engine-anim`; consumers: combat reach/aim per tick via
//! [`combat_reach_tick`], hit-react (IK runs first, additive react
//! layers after), balance (reads wrist/spine outputs). Units: meters in,
//! integer millimeters on every error meter. Determinism: `+ - * /` plus
//! `sqrt` only, fixed joint order, fixed [`FABRIK_ITERS`] cap, no clock; one
//! target script replays byte-identically. Errors: [`FbikError`]. Budget:
//! worst case [`FULL_BODY_OPS_CAP`] analytic-plus-FABRIK ops per character
//! (legs excluded, owned by the foot locker); tick path writes caller-owned
//! poses only, steady-state allocates nothing.

use super::contract::MAX_JOINTS;
use super::feet::{
    dist_m, lock_graph_pose, solve_two_bone, FootError, FootLocker, LegMap, LegSolution,
};
use super::graph::Pose;
use super::ik::fabrik;
use engine_core::vec3::cross3;
use engine_core::{quat_mul, Fnv64};

/// FABRIK fallback iteration cap: every fallback runs at most this many
/// passes, so per-character cost is data-independent in the worst case.
pub const FABRIK_ITERS: u32 = 8;
/// FABRIK early-out tolerance in meters: 1 mm, the hand-error bar.
pub const FABRIK_TOL_M: f32 = 0.001;
/// Hand acceptance bar in millimeters: reach fixtures land inside 1 cm.
pub const HAND_TOL_MM: u32 = 10;
/// Posted cost of one analytic chain solve (closed-form pass + meters).
pub const OPS_ANALYTIC_CHAIN: u32 = 64;
/// Posted cost per joint per FABRIK iteration (forward + backward passes).
pub const OPS_FABRIK_JOINT_ITER: u32 = 16;
/// Posted cost of one look-at solve (normalize + quat build + apply).
pub const OPS_LOOK: u32 = 32;
/// Posted cost of one aim solve (same shape as look-at).
pub const OPS_AIM: u32 = 32;
/// Worst-case ops per character per tick: two arm chains, each analytic plus
/// a full FABRIK fallback, plus look plus aim. Legs are excluded: the foot
/// locker owns their budget. `2 * (64 + 8*3*16) + 32 + 32 = 960`.
pub const FULL_BODY_OPS_CAP: u32 = 960;
/// Scale receipt width: the squad-cost test runs this many fighters.
pub const SCALE_FIGHTERS: usize = 64;

/// Worst-case ops per character per tick; equals [`FULL_BODY_OPS_CAP`].
/// Combat and balance budget `fighters * full_body_cost_cap()` with no
/// data-dependent term: FABRIK never exceeds [`FABRIK_ITERS`].
pub fn full_body_cost_cap() -> u32 {
    FULL_BODY_OPS_CAP
}

/// Full-body IK failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Debug)]
pub enum FbikError {
    /// Chain or look/aim joint index out of range; value carried.
    BadJoint(usize),
    /// Pose lanes disagree or break the joint ceiling; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Zero-joint pose.
    EmptyPose,
    /// Primary target missing for a two-hand grip; lane named.
    BadTarget(&'static str),
    /// Off-hand grip missed past [`HAND_TOL_MM`], or an explicit left target
    /// was given under a two-hand grip (carries 0: the hands disagree by
    /// construction). Writes nothing.
    GripConflict { error_mm: u32 },
    /// Shield anchor plus two-hand grip requested together; the left hand
    /// cannot hold a shield and a weapon grip on one tick. Writes nothing.
    ShieldConflict,
    /// A leg-lock step refused; the foot locker's cause is carried through.
    Leg(FootError),
}

impl core::fmt::Display for FbikError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FbikError::BadJoint(joint) => write!(f, "fbik: joint {joint} out of range"),
            FbikError::LengthMismatch { expected, got } => {
                write!(f, "fbik: joint count {got} differs from {expected}")
            }
            FbikError::NonFinite(lane) => write!(f, "fbik: non-finite float in {lane}"),
            FbikError::EmptyPose => write!(f, "fbik: zero-joint pose is invalid"),
            FbikError::BadTarget(lane) => write!(f, "fbik: bad grip target ({lane})"),
            FbikError::GripConflict { error_mm } => {
                write!(f, "fbik: off-hand grip missed by {error_mm} mm")
            }
            FbikError::ShieldConflict => {
                write!(f, "fbik: shield anchor conflicts with two-hand grip")
            }
            FbikError::Leg(e) => write!(f, "fbik leg: {e}"),
        }
    }
}

impl std::error::Error for FbikError {}

impl From<FootError> for FbikError {
    fn from(e: FootError) -> Self {
        match e {
            FootError::BadJoint(joint) => FbikError::BadJoint(joint),
            FootError::LengthMismatch { expected, got } => {
                FbikError::LengthMismatch { expected, got }
            }
            FootError::NonFinite(lane) => FbikError::NonFinite(lane),
            FootError::BadFoot(foot) => FbikError::BadJoint(foot),
            FootError::BadGround => FbikError::NonFinite("ground"),
        }
    }
}

/// Which solver wrote the wrist: analytic wins ties, FABRIK polishes the rest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IkMethod {
    /// Closed-form two-bone solve (the limb best path: one pass, exact).
    Analytic,
    /// Fixed-cap FABRIK fallback over the same chain.
    Fabrik,
}

/// One three-joint arm chain: shoulder stays, elbow and wrist solve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArmChain {
    /// Shoulder joint index (chain root, pinned).
    pub shoulder: usize,
    /// Elbow joint index.
    pub elbow: usize,
    /// Wrist joint index (reaches the target).
    pub wrist: usize,
}

impl ArmChain {
    /// Name three lanes shoulder, elbow, wrist; range-checked at solve time.
    pub fn new(shoulder: usize, elbow: usize, wrist: usize) -> Self {
        Self {
            shoulder,
            elbow,
            wrist,
        }
    }
}

/// One solved arm: positions plus the meter readings for this tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ReachSolution {
    /// Solved elbow position in meters.
    pub elbow: [f32; 3],
    /// Solved wrist position in meters.
    pub wrist: [f32; 3],
    /// Residual in millimeters (`round(|wrist - target| * 1000)`).
    pub error_mm: u32,
    /// True when the target lay past max reach and clamped onto it.
    pub clamped: bool,
    /// Solver that wrote the wrist; analytic wins every tie.
    pub method: IkMethod,
    /// Passes used: 1 for analytic-only, else the FABRIK pass count.
    pub iters_used: u32,
    /// Posted ops spent on this chain.
    pub cost_ops: u32,
}

fn finite3(p: [f32; 3]) -> bool {
    p[0].is_finite() && p[1].is_finite() && p[2].is_finite()
}

/// Residual meter: integer millimeters, saturates on bad lanes (callers
/// validate finite first, so saturation only arms against logic slips).
fn error_mm(wrist: [f32; 3], target: [f32; 3]) -> u32 {
    super::feet::slide_mm(wrist, target)
}

fn checked_joints(pose: &Pose) -> Result<usize, FbikError> {
    if pose.positions.len() != pose.rotations.len() {
        return Err(FbikError::LengthMismatch {
            expected: pose.positions.len(),
            got: pose.rotations.len(),
        });
    }
    let joints = pose.positions.len();
    if joints == 0 {
        return Err(FbikError::EmptyPose);
    }
    if joints > MAX_JOINTS {
        return Err(FbikError::LengthMismatch {
            expected: MAX_JOINTS,
            got: joints,
        });
    }
    if pose.positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(FbikError::NonFinite("positions"));
    }
    if pose.rotations.iter().flatten().any(|v| !v.is_finite()) {
        return Err(FbikError::NonFinite("rotations"));
    }
    Ok(joints)
}

fn checked_chain(joints: usize, chain: &ArmChain) -> Result<(), FbikError> {
    for joint in [chain.shoulder, chain.elbow, chain.wrist] {
        if joint >= joints {
            return Err(FbikError::BadJoint(joint));
        }
    }
    Ok(())
}

/// Pure three-lane reach: analytic first, fixed-cap FABRIK fallback, best
/// wrist wins with analytic taking ties. Never touches a pose; the wrappers
/// below commit the result, so grip conflicts write nothing.
fn reach_on_lanes(
    shoulder: [f32; 3],
    elbow: [f32; 3],
    wrist: [f32; 3],
    target: [f32; 3],
) -> ([f32; 3], [f32; 3], IkMethod, u32, u32, bool) {
    let l1 = dist_m(elbow, shoulder);
    let l2 = dist_m(wrist, elbow);
    let clamped = dist_m(target, shoulder) > l1 + l2;
    let (a_elbow, a_wrist) = solve_two_bone(shoulder, elbow, wrist, target);
    let a_err = error_mm(a_wrist, target);
    if a_err <= HAND_TOL_MM {
        return (a_elbow, a_wrist, IkMethod::Analytic, 1, a_err, clamped);
    }
    let mut buf = [shoulder, elbow, wrist];
    let used = fabrik(&mut buf, target, FABRIK_ITERS, FABRIK_TOL_M);
    let f_err = error_mm(buf[2], target);
    let fabrik_cost = used * 3 * OPS_FABRIK_JOINT_ITER;
    if f_err < a_err {
        (buf[1], buf[2], IkMethod::Fabrik, used, f_err, clamped)
    } else {
        let _ = fabrik_cost;
        (a_elbow, a_wrist, IkMethod::Analytic, used, a_err, clamped)
    }
}

fn reach_cost(iters_used: u32, method: IkMethod) -> u32 {
    if method == IkMethod::Analytic && iters_used <= 1 {
        OPS_ANALYTIC_CHAIN
    } else {
        OPS_ANALYTIC_CHAIN + iters_used * 3 * OPS_FABRIK_JOINT_ITER
    }
}

/// One arm solved on stack copies of its lanes: the posted solution, nothing
/// written to `pose`. Callers range- and finite-check first.
fn reach_arm(pose: &Pose, chain: &ArmChain, target: [f32; 3]) -> ReachSolution {
    let (elbow, wrist, method, iters, err, clamped) = reach_on_lanes(
        pose.positions[chain.shoulder],
        pose.positions[chain.elbow],
        pose.positions[chain.wrist],
        target,
    );
    ReachSolution {
        elbow,
        wrist,
        error_mm: err,
        clamped,
        method,
        iters_used: iters,
        cost_ops: reach_cost(iters, method),
    }
}

/// Solve one arm toward `target`: analytic two-bone first, FABRIK at a fixed
/// cap as fallback. Unreachable targets clamp to max reach and report
/// `clamped` with the overshoot in `error_mm`; degenerate or non-finite
/// input is refused loudly and writes nothing.
pub fn solve_reach(
    pose: &mut Pose,
    chain: &ArmChain,
    target: [f32; 3],
) -> Result<ReachSolution, FbikError> {
    let joints = checked_joints(pose)?;
    checked_chain(joints, chain)?;
    if !finite3(target) {
        return Err(FbikError::NonFinite("target"));
    }
    let solution = reach_arm(pose, chain, target);
    pose.positions[chain.elbow] = solution.elbow;
    pose.positions[chain.wrist] = solution.wrist;
    Ok(solution)
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn norm3_or(v: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let l = len3(v);
    if !l.is_finite() || l <= 1e-9 {
        fallback
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

fn quat_norm(q: [f32; 4]) -> [f32; 4] {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !n2.is_finite() || n2 <= 0.0 {
        return IDENTITY;
    }
    let inv = 1.0 / n2.sqrt();
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// Shortest-arc quat rotating unit `from` onto unit `to`: `cross` plus
/// `1 + dot`, normalized; the antipodal case picks a stable orthogonal axis.
/// Uses `+ - * /` plus `sqrt` only, no trig.
fn quat_from_to(from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    let dot = dot3(from, to);
    if dot >= 1.0 - 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if dot <= -1.0 + 1e-6 {
        let axis = if from[0] * from[0] < 0.81 {
            norm3_or(cross3(from, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0])
        } else {
            norm3_or(cross3(from, [0.0, 1.0, 0.0]), [1.0, 0.0, 0.0])
        };
        return [axis[0], axis[1], axis[2], 0.0];
    }
    let c = cross3(from, to);
    quat_norm([c[0], c[1], c[2], 1.0 + dot])
}

fn quat_rot(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let t = [
        2.0 * (q[1] * v[2] - q[2] * v[1]),
        2.0 * (q[2] * v[0] - q[0] * v[2]),
        2.0 * (q[0] * v[1] - q[1] * v[0]),
    ];
    [
        v[0] + q[3] * t[0] + (q[1] * t[2] - q[2] * t[1]),
        v[1] + q[3] * t[1] + (q[2] * t[0] - q[0] * t[2]),
        v[2] + q[3] * t[2] + (q[0] * t[1] - q[1] * t[0]),
    ]
}

/// Rotation that turns `joint`'s local +Z (fighter forward) onto `target`,
/// or `None` when the target sits on the joint. Pure: writes nothing.
fn turned_rotation(pose: &Pose, joint: usize, target: [f32; 3]) -> Option<[f32; 4]> {
    let to = sub3(target, pose.positions[joint]);
    if !len3(to).is_finite() || len3(to) <= 1e-6 {
        return None;
    }
    let dir = norm3_or(to, [0.0, 0.0, 1.0]);
    let fwd = quat_rot(pose.rotations[joint], [0.0, 0.0, 1.0]);
    let fwd_n = norm3_or(fwd, [0.0, 0.0, 1.0]);
    let q = quat_from_to(fwd_n, dir);
    Some(quat_norm(quat_mul(q, pose.rotations[joint])))
}

/// Turn `joint` so its local +Z (fighter forward) points at `target`.
/// Returns `true` when the rotation was applied; a target sitting on the
/// joint keeps the current rotation (`false`, still `Ok`). Positions stay.
fn turn_to(pose: &mut Pose, joint: usize, target: [f32; 3]) -> Result<bool, FbikError> {
    let joints = checked_joints(pose)?;
    if joint >= joints {
        return Err(FbikError::BadJoint(joint));
    }
    if !finite3(target) {
        return Err(FbikError::NonFinite("target"));
    }
    match turned_rotation(pose, joint, target) {
        Some(rotation) => {
            pose.rotations[joint] = rotation;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Look-at: turn the head joint toward `target` (rotation only, so the feet
/// and hands stay where the reach solves put them). Returns whether the head
/// turned; see [`turn_to`] for the on-joint rule.
pub fn solve_look(pose: &mut Pose, head: usize, target: [f32; 3]) -> Result<bool, FbikError> {
    turn_to(pose, head, target)
}

/// Aim: turn the aim joint (spine or shoulder) so the weapon axis (+Z)
/// points at `target`. Combat aims the torso first, then reaches the hands;
/// balance reads the resulting spine lane. Same on-joint rule as
/// [`solve_look`].
pub fn solve_aim(pose: &mut Pose, joint: usize, target: [f32; 3]) -> Result<bool, FbikError> {
    turn_to(pose, joint, target)
}

/// Off-hand grip mode for a full-body solve.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GripMode {
    /// Each hand solves its own target independently.
    Single,
    /// Right hand is primary: the left hand grips the weapon at
    /// `primary_wrist + grip_offset_m`. An explicit left target or a shield
    /// anchor alongside refuses loudly.
    TwoHand {
        /// Weapon grip offset from the primary wrist, in meters.
        grip_offset_m: [f32; 3],
    },
}

/// Joint map for a full-body solve: two arm chains plus look/aim joints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodyChains {
    /// Left arm chain.
    pub arm_l: ArmChain,
    /// Right arm chain (two-hand primary).
    pub arm_r: ArmChain,
    /// Head joint for look-at.
    pub head: usize,
    /// Aim joint (spine or shoulder) for weapon aim.
    pub aim: usize,
}

/// Per-tick targets for a full-body solve; every `Some` lane must be finite.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodyTargets {
    /// Left-hand reach target (`None` keeps the bind wrist).
    pub hand_l: Option<[f32; 3]>,
    /// Right-hand reach target (`None` keeps the bind wrist).
    pub hand_r: Option<[f32; 3]>,
    /// Look-at target (`None` keeps the head rotation).
    pub look: Option<[f32; 3]>,
    /// Aim target (`None` keeps the aim rotation).
    pub aim: Option<[f32; 3]>,
    /// Off-hand grip mode.
    pub grip: GripMode,
    /// Left-hand shield anchor (`None` for no shield); conflicts with
    /// [`GripMode::TwoHand`].
    pub shield_l: Option<[f32; 3]>,
}

/// One solved character: per-chain meters plus the posted cost spent.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BodySolution {
    /// Left-arm solve (`None` when no left target, grip or shield ran).
    pub hand_l: Option<ReachSolution>,
    /// Right-arm solve (`None` when no right target ran).
    pub hand_r: Option<ReachSolution>,
    /// Shield-grip solve (`None` when no shield anchor ran).
    pub shield: Option<ReachSolution>,
    /// True when a look-at target turned the head.
    pub look_applied: bool,
    /// True when an aim target turned the aim joint.
    pub aim_applied: bool,
    /// Posted ops spent this tick, at most [`FULL_BODY_OPS_CAP`].
    pub cost_ops: u32,
}

/// Planned (uncommitted) full-body result: computed on stack copies so grip
/// conflicts return before any lane is written.
struct PlannedBody {
    elbow_l: Option<[f32; 3]>,
    wrist_l: Option<[f32; 3]>,
    elbow_r: Option<[f32; 3]>,
    wrist_r: Option<[f32; 3]>,
    head_rot: Option<[f32; 4]>,
    aim_rot: Option<[f32; 4]>,
    solution: BodySolution,
}

/// Range and finite checks shared by every full-body plan: pose lanes, both
/// chains, look/aim joints, every `Some` target and the grip conflicts.
fn check_body_inputs(
    pose: &Pose,
    chains: &BodyChains,
    targets: &BodyTargets,
) -> Result<(), FbikError> {
    let joints = checked_joints(pose)?;
    checked_chain(joints, &chains.arm_l)?;
    checked_chain(joints, &chains.arm_r)?;
    for joint in [chains.head, chains.aim] {
        if joint >= joints {
            return Err(FbikError::BadJoint(joint));
        }
    }
    for target in [
        targets.hand_l,
        targets.hand_r,
        targets.look,
        targets.aim,
        targets.shield_l,
    ]
    .into_iter()
    .flatten()
    {
        if !finite3(target) {
            return Err(FbikError::NonFinite("target"));
        }
    }
    if let GripMode::TwoHand { grip_offset_m } = targets.grip {
        if !finite3(grip_offset_m) {
            return Err(FbikError::NonFinite("grip"));
        }
        if targets.shield_l.is_some() {
            return Err(FbikError::ShieldConflict);
        }
        if targets.hand_l.is_some() {
            return Err(FbikError::GripConflict { error_mm: 0 });
        }
        if targets.hand_r.is_none() {
            return Err(FbikError::BadTarget("two-hand needs hand_r"));
        }
    }
    Ok(())
}

fn plan_full_body(
    pose: &Pose,
    chains: &BodyChains,
    targets: &BodyTargets,
) -> Result<PlannedBody, FbikError> {
    check_body_inputs(pose, chains, targets)?;
    // Fixed order: right (two-hand primary), left, shield, look, aim.
    let sol_r = targets
        .hand_r
        .map(|target| reach_arm(pose, &chains.arm_r, target));
    let sol_l = match targets.grip {
        GripMode::Single => targets
            .hand_l
            .map(|target| reach_arm(pose, &chains.arm_l, target)),
        GripMode::TwoHand { grip_offset_m } => {
            let primary = sol_r.expect("two-hand plans hand_r first").wrist;
            let grip = [
                primary[0] + grip_offset_m[0],
                primary[1] + grip_offset_m[1],
                primary[2] + grip_offset_m[2],
            ];
            if !finite3(grip) {
                return Err(FbikError::NonFinite("grip"));
            }
            let solution = reach_arm(pose, &chains.arm_l, grip);
            if solution.error_mm > HAND_TOL_MM {
                return Err(FbikError::GripConflict {
                    error_mm: solution.error_mm,
                });
            }
            Some(solution)
        }
    };
    let sol_shield = targets
        .shield_l
        .map(|anchor| reach_arm(pose, &chains.arm_l, anchor));
    // The shield solve overwrites the left-arm lanes when both ran.
    let left = sol_shield.or(sol_l);
    let head_rot = targets
        .look
        .and_then(|target| turned_rotation(pose, chains.head, target));
    let aim_rot = targets
        .aim
        .and_then(|target| turned_rotation(pose, chains.aim, target));
    let mut cost = 0u32;
    for solution in [sol_r, sol_l, sol_shield].into_iter().flatten() {
        cost += solution.cost_ops;
    }
    if head_rot.is_some() {
        cost += OPS_LOOK;
    }
    if aim_rot.is_some() {
        cost += OPS_AIM;
    }
    debug_assert!(cost <= FULL_BODY_OPS_CAP);
    Ok(PlannedBody {
        elbow_l: left.map(|solution| solution.elbow),
        wrist_l: left.map(|solution| solution.wrist),
        elbow_r: sol_r.map(|solution| solution.elbow),
        wrist_r: sol_r.map(|solution| solution.wrist),
        head_rot,
        aim_rot,
        solution: BodySolution {
            hand_l: sol_l,
            hand_r: sol_r,
            shield: sol_shield,
            look_applied: head_rot.is_some(),
            aim_applied: aim_rot.is_some(),
            cost_ops: cost,
        },
    })
}

fn commit_planned(pose: &mut Pose, chains: &BodyChains, planned: &PlannedBody) {
    if let (Some(elbow), Some(wrist)) = (planned.elbow_l, planned.wrist_l) {
        pose.positions[chains.arm_l.elbow] = elbow;
        pose.positions[chains.arm_l.wrist] = wrist;
    }
    if let (Some(elbow), Some(wrist)) = (planned.elbow_r, planned.wrist_r) {
        pose.positions[chains.arm_r.elbow] = elbow;
        pose.positions[chains.arm_r.wrist] = wrist;
    }
    if let Some(rot) = planned.head_rot {
        pose.rotations[chains.head] = rot;
    }
    if let Some(rot) = planned.aim_rot {
        pose.rotations[chains.aim] = rot;
    }
}

/// Solve one character for one tick: arms (plus grip), look, aim, in fixed
/// order. Atomic: grip conflicts validate before any lane is written, so a
/// refusal leaves the pose untouched. Steady-state ticks allocate nothing.
pub fn solve_full_body(
    pose: &mut Pose,
    chains: &BodyChains,
    targets: &BodyTargets,
) -> Result<BodySolution, FbikError> {
    let planned = plan_full_body(pose, chains, targets)?;
    commit_planned(pose, chains, &planned);
    Ok(planned.solution)
}

/// Combat reach/aim entry for one fighter tick: plant the legs through
/// [`lock_graph_pose`], then solve the upper body through [`solve_full_body`].
/// The body plan is computed before the leg lock commits, so a grip refusal
/// still plants the feet but never half-writes the arms; the additive
/// hit-react layers run after this returns, and balance reads the
/// wrist/spine lanes. Arm, head and aim joints must not alias the leg write
/// lanes.
pub fn combat_reach_tick(
    pose: &mut Pose,
    legs: &[LegMap; 2],
    planted: [bool; 2],
    locker: &mut FootLocker,
    chains: &BodyChains,
    targets: &BodyTargets,
) -> Result<CombatOutcome, FbikError> {
    let planned = plan_full_body(pose, chains, targets)?;
    for map in [chains.arm_l, chains.arm_r] {
        for joint in [map.shoulder, map.elbow, map.wrist] {
            for leg in legs.iter() {
                if joint == leg.knee || joint == leg.ankle {
                    return Err(FbikError::BadJoint(joint));
                }
            }
        }
    }
    for joint in [chains.head, chains.aim] {
        for leg in legs.iter() {
            if joint == leg.knee || joint == leg.ankle {
                return Err(FbikError::BadJoint(joint));
            }
        }
    }
    let leg_solutions = lock_graph_pose(pose, legs, planted, locker).map_err(FbikError::Leg)?;
    commit_planned(pose, chains, &planned);
    Ok(CombatOutcome {
        legs: leg_solutions,
        body: planned.solution,
    })
}

/// One combat tick outcome: planted-leg solves plus the upper-body solve.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CombatOutcome {
    /// Per-leg solves in foot order.
    pub legs: [LegSolution; 2],
    /// Upper-body reach/look/aim solve.
    pub body: BodySolution,
}

/// FNV-1a64 over the full pose (positions plus rotations, `f32` bits);
/// equal poses hash equally, which is the replay receipt for IK sequences.
pub fn fbik_hash(pose: &Pose) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(pose.positions.len() as u64);
    for p in pose.positions.iter() {
        for lane in p.iter() {
            h.write_f32(*lane);
        }
    }
    for q in pose.rotations.iter() {
        for lane in q.iter() {
            h.write_f32(*lane);
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::super::loco_bake::bake_hero_set;
    use super::*;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    /// Fifteen-joint reach rig (meters): pelvis, spine, head, two arm
    /// chains, two leg chains. Shoulders sit +-0.22 on X at 1.42; upper
    /// 0.30, forearm 0.43 long; hips +-0.11 at 0.90.
    fn rig() -> Pose {
        Pose {
            positions: vec![
                [0.0, 0.95, 0.0],     // 0 pelvis
                [0.0, 1.20, 0.0],     // 1 spine (aim)
                [0.0, 1.50, 0.0],     // 2 head (look)
                [0.22, 1.42, 0.0],    // 3 shoulder L
                [0.24, 1.12, 0.02],   // 4 elbow L
                [0.26, 0.70, 0.05],   // 5 wrist L
                [-0.22, 1.42, 0.0],   // 6 shoulder R
                [-0.24, 1.12, 0.02],  // 7 elbow R
                [-0.26, 0.70, 0.05],  // 8 wrist R
                [0.11, 0.90, 0.0],    // 9 hip L
                [0.11, 0.50, 0.02],   // 10 knee L
                [0.11, 0.05, 0.05],   // 11 ankle L
                [-0.11, 0.90, 0.0],   // 12 hip R
                [-0.11, 0.50, -0.02], // 13 knee R
                [-0.11, 0.12, -0.05], // 14 ankle R
            ],
            rotations: vec![IDENTITY; 15],
        }
    }

    fn chains() -> BodyChains {
        BodyChains {
            arm_l: ArmChain::new(3, 4, 5),
            arm_r: ArmChain::new(6, 7, 8),
            head: 2,
            aim: 1,
        }
    }

    fn legs() -> [LegMap; 2] {
        [
            LegMap::new(9, 10, 11, 0).expect("left map"),
            LegMap::new(12, 13, 14, 1).expect("right map"),
        ]
    }

    #[test]
    fn reach_lands_inside_1cm_analytic_first() {
        let chain = ArmChain::new(3, 4, 5);
        // Reachable targets around the left hand: front, high, wide.
        let targets = [
            [0.45, 1.20, 0.35],
            [0.30, 1.60, 0.10],
            [0.55, 0.90, -0.15],
            [0.22, 1.42, 0.50],
        ];
        for target in targets {
            let mut pose = rig();
            let sol = solve_reach(&mut pose, &chain, target).expect("reach");
            assert!(!sol.clamped, "fixture {target:?} must be reachable");
            assert_eq!(sol.method, IkMethod::Analytic);
            assert_eq!(sol.iters_used, 1);
            assert!(
                sol.error_mm <= HAND_TOL_MM,
                "hand misses by {} mm at {target:?}",
                sol.error_mm
            );
            // Bone lengths survive the solve (no stretch).
            let l1 = dist_m(pose.positions[3], pose.positions[4]);
            let l2 = dist_m(pose.positions[4], pose.positions[5]);
            assert!((l1 - 0.30).abs() < 0.012, "upper {l1}");
            assert!((l2 - 0.43).abs() < 0.012, "fore {l2}");
            println!(
                "AM9 reach: target={target:?} err={} mm method={:?}",
                sol.error_mm, sol.method
            );
        }
    }

    #[test]
    fn analytic_is_best_on_two_bone_fabrik_matches() {
        // Head-to-head on the same lanes: analytic must match or beat the
        // fixed-cap FABRIK fallback on every two-bone fixture (names the
        // limb best path); FABRIK stays as the multi-joint fallback.
        let pose = rig();
        let targets = [
            [0.45, 1.20, 0.35],
            [0.10, 1.00, 0.45],
            [0.60, 1.30, -0.20],
            [3.0, 1.0, 0.0],
        ];
        for target in targets {
            let (a_e, a_w) = solve_two_bone(
                pose.positions[3],
                pose.positions[4],
                pose.positions[5],
                target,
            );
            let a_err = error_mm(a_w, target);
            let mut buf = [pose.positions[3], pose.positions[4], pose.positions[5]];
            let used = fabrik(&mut buf, target, FABRIK_ITERS, FABRIK_TOL_M);
            assert!(used <= FABRIK_ITERS);
            let f_err = error_mm(buf[2], target);
            assert!(
                a_err <= f_err,
                "analytic {a_err} mm vs fabrik {f_err} mm at {target:?}"
            );
            let _ = a_e;
            println!(
                "AM9 analytic-vs-fabrik: target={target:?} analytic={a_err} mm fabrik={f_err} mm"
            );
        }
        // Multi-joint chain (spine through wrist): FABRIK converges inside
        // the cap with the root pinned, which is the fallback's own lane.
        let mut spine_chain = [
            pose.positions[1],
            pose.positions[3],
            pose.positions[4],
            pose.positions[5],
        ];
        let root = spine_chain[0];
        let used = fabrik(
            &mut spine_chain,
            [0.55, 1.10, 0.40],
            FABRIK_ITERS,
            FABRIK_TOL_M,
        );
        assert!(used <= FABRIK_ITERS);
        assert_eq!(spine_chain[0], root);
        assert!(
            error_mm(spine_chain[3], [0.55, 1.10, 0.40]) <= HAND_TOL_MM,
            "{spine_chain:?}"
        );
    }

    #[test]
    fn unreachable_clamps_and_reports() {
        let chain = ArmChain::new(6, 7, 8);
        let mut pose = rig();
        let sol = solve_reach(&mut pose, &chain, [3.0, 1.0, 0.0]).expect("clamps");
        assert!(sol.clamped);
        assert!(
            sol.error_mm > HAND_TOL_MM,
            "overshoot must read past the bar"
        );
        // Wrist sits at max reach along the target dir, shoulder pinned.
        let l1 = dist_m(rig().positions[7], rig().positions[6]);
        let l2 = dist_m(rig().positions[8], rig().positions[7]);
        let reach = dist_m(pose.positions[8], pose.positions[6]);
        assert!((reach - (l1 + l2)).abs() < 0.003, "reach {reach}");
        assert_eq!(pose.positions[6], rig().positions[6]);
        // Degenerate chain (zero upper length) keeps the pose, stays finite.
        let mut pose = rig();
        pose.positions[4] = pose.positions[3];
        let sol = solve_reach(&mut pose, &chain, [0.0, 1.0, 0.4]).expect("degen");
        assert!(sol.error_mm < u32::MAX, "degenerate still meters");
        for lane in pose
            .positions
            .iter()
            .chain(std::iter::once(&pose.positions[8]))
        {
            assert!(finite3(*lane));
        }
    }

    #[test]
    fn look_and_aim_face_the_target() {
        let mut pose = rig();
        assert!(solve_look(&mut pose, 2, [0.6, 1.45, 0.8]).expect("look"));
        let dir = norm3_or(sub3([0.6, 1.45, 0.8], pose.positions[2]), [0.0, 0.0, 1.0]);
        let fwd = norm3_or(
            quat_rot(pose.rotations[2], [0.0, 0.0, 1.0]),
            [0.0, 0.0, 1.0],
        );
        assert!(dot3(fwd, dir) > 0.999, "head {fwd:?} vs {dir:?}");
        assert!(solve_aim(&mut pose, 1, [-0.7, 1.10, 0.9]).expect("aim"));
        let dir = norm3_or(sub3([-0.7, 1.10, 0.9], pose.positions[1]), [0.0, 0.0, 1.0]);
        let fwd = norm3_or(
            quat_rot(pose.rotations[1], [0.0, 0.0, 1.0]),
            [0.0, 0.0, 1.0],
        );
        assert!(dot3(fwd, dir) > 0.999, "spine {fwd:?} vs {dir:?}");
        // Rotations stay unit; positions untouched by look/aim.
        for q in pose.rotations.iter() {
            let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
            assert!((n2 - 1.0).abs() < 1e-5, "{q:?}");
        }
        assert_eq!(pose.positions, rig().positions);
        // Antipodal target (straight behind) still faces within tolerance.
        let mut pose = rig();
        assert!(solve_look(&mut pose, 2, [0.0, 1.50, -2.0]).expect("behind"));
        let dir = norm3_or(sub3([0.0, 1.50, -2.0], pose.positions[2]), [0.0, 0.0, 1.0]);
        let fwd = norm3_or(
            quat_rot(pose.rotations[2], [0.0, 0.0, 1.0]),
            [0.0, 0.0, 1.0],
        );
        assert!(dot3(fwd, dir) > 0.999, "behind {fwd:?}");
        // On-joint target keeps the rotation and reports false.
        let mut pose = rig();
        let head_pos = pose.positions[2];
        assert!(!solve_look(&mut pose, 2, head_pos).expect("on-joint"));
        assert_eq!(pose.rotations[2], IDENTITY);
    }

    #[test]
    fn two_hand_grip_meets_at_the_weapon() {
        let chains = chains();
        let mut pose = rig();
        // Primary in front-right, grip 0.25 m toward the left hand lane.
        let sol = solve_full_body(
            &mut pose,
            &chains,
            &BodyTargets {
                hand_l: None,
                hand_r: Some([-0.02, 1.15, 0.42]),
                look: None,
                aim: None,
                grip: GripMode::TwoHand {
                    grip_offset_m: [-0.20, 0.0, 0.0],
                },
                shield_l: None,
            },
        )
        .expect("two-hand");
        let (l, r) = (sol.hand_l.expect("off"), sol.hand_r.expect("primary"));
        assert!(l.error_mm <= HAND_TOL_MM, "off misses by {}", l.error_mm);
        assert!(
            r.error_mm <= HAND_TOL_MM,
            "primary misses by {}",
            r.error_mm
        );
        // Hands sit one grip apart (2 mm tolerance past the 1 cm hand bar).
        let span = dist_m(pose.positions[5], pose.positions[8]);
        assert!((span - 0.20).abs() < 0.012, "span {span}");
        assert!(sol.cost_ops <= FULL_BODY_OPS_CAP);
    }

    #[test]
    fn shield_grip_holds_the_anchor() {
        let chains = chains();
        let mut pose = rig();
        let sol = solve_full_body(
            &mut pose,
            &chains,
            &BodyTargets {
                hand_l: None,
                hand_r: Some([-0.35, 1.15, 0.40]),
                look: Some([0.0, 1.45, 2.0]),
                aim: Some([0.0, 1.10, 2.0]),
                grip: GripMode::Single,
                shield_l: Some([0.45, 1.05, 0.30]),
            },
        )
        .expect("shield");
        let shield = sol.shield.expect("shield solve");
        assert!(shield.error_mm <= HAND_TOL_MM, "shield {}", shield.error_mm);
        assert!(sol.look_applied && sol.aim_applied);
        assert!(sol.cost_ops <= FULL_BODY_OPS_CAP);
        println!(
            "AM9 shield: err={} mm cost={} ops cap={}",
            shield.error_mm, sol.cost_ops, FULL_BODY_OPS_CAP
        );
    }

    #[test]
    fn grip_and_shield_conflicts_refuse_atomically() {
        let chains = chains();
        // Shield plus two-hand: ShieldConflict, pose untouched.
        let mut pose = rig();
        let before = pose.clone();
        assert_eq!(
            solve_full_body(
                &mut pose,
                &chains,
                &BodyTargets {
                    hand_l: None,
                    hand_r: Some([-0.05, 1.15, 0.45]),
                    look: None,
                    aim: None,
                    grip: GripMode::TwoHand {
                        grip_offset_m: [-0.25, 0.0, 0.0],
                    },
                    shield_l: Some([0.45, 1.05, 0.30]),
                },
            )
            .unwrap_err(),
            FbikError::ShieldConflict
        );
        assert_eq!(pose, before);
        // Explicit left target under two-hand: GripConflict, pose untouched.
        assert_eq!(
            solve_full_body(
                &mut pose,
                &chains,
                &BodyTargets {
                    hand_l: Some([0.45, 1.20, 0.35]),
                    hand_r: Some([-0.05, 1.15, 0.45]),
                    look: None,
                    aim: None,
                    grip: GripMode::TwoHand {
                        grip_offset_m: [-0.25, 0.0, 0.0],
                    },
                    shield_l: None,
                },
            )
            .unwrap_err(),
            FbikError::GripConflict { error_mm: 0 }
        );
        assert_eq!(pose, before);
        // Grip 2 m away: off hand cannot meet it, error names the miss.
        let err = solve_full_body(
            &mut pose,
            &chains,
            &BodyTargets {
                hand_l: None,
                hand_r: Some([-0.05, 1.15, 0.45]),
                look: None,
                aim: None,
                grip: GripMode::TwoHand {
                    grip_offset_m: [2.0, 0.0, 0.0],
                },
                shield_l: None,
            },
        )
        .unwrap_err();
        match err {
            FbikError::GripConflict { error_mm } => {
                assert!(error_mm > HAND_TOL_MM, "{error_mm}");
            }
            other => panic!("expected GripConflict, got {other:?}"),
        }
        assert_eq!(pose, before);
        // Two-hand without a primary target names the lane.
        assert_eq!(
            solve_full_body(
                &mut pose,
                &chains,
                &BodyTargets {
                    hand_l: None,
                    hand_r: None,
                    look: None,
                    aim: None,
                    grip: GripMode::TwoHand {
                        grip_offset_m: [-0.25, 0.0, 0.0],
                    },
                    shield_l: None,
                },
            )
            .unwrap_err(),
            FbikError::BadTarget("two-hand needs hand_r")
        );
        assert_eq!(pose, before);
    }

    #[test]
    fn combat_tick_plants_feet_then_reaches() {
        let chains = chains();
        let leg_maps = legs();
        // Ten-tick skirmish script: feet plant while hands track the weapon.
        let run = || {
            let mut locker = FootLocker::with_defaults();
            let mut pose = rig();
            let mut out = Vec::with_capacity(10);
            for tick in 0..10u32 {
                let phase = tick as f32 / 10.0;
                let sweep = 0.30 + phase * 0.10;
                let outcome = combat_reach_tick(
                    &mut pose,
                    &leg_maps,
                    [tick % 2 == 0, tick % 2 == 1],
                    &mut locker,
                    &chains,
                    &BodyTargets {
                        hand_l: None,
                        hand_r: Some([-0.10 - phase * 0.02, 1.15, sweep]),
                        look: Some([0.0, 1.45, 2.0]),
                        aim: Some([0.0, 1.10, 2.0]),
                        grip: GripMode::Single,
                        shield_l: None,
                    },
                )
                .expect("combat tick");
                assert!(outcome.body.cost_ops <= FULL_BODY_OPS_CAP);
                assert!(
                    outcome.body.hand_r.expect("reach").error_mm <= HAND_TOL_MM,
                    "tick {tick} hand"
                );
                out.push((pose.clone(), fbik_hash(&pose)));
            }
            out
        };
        let (first, second) = (run(), run());
        assert_eq!(first, second);
        println!(
            "AM9 combat: 10 ticks replay-equal hash=0x{:016x} cost_cap={} ops",
            first[9].1, FULL_BODY_OPS_CAP
        );
        // Arm joints aliasing leg write lanes refuse before writing.
        let mut pose = rig();
        let before = pose.clone();
        let mut locker = FootLocker::with_defaults();
        let bad_chains = BodyChains {
            arm_l: ArmChain::new(9, 10, 11),
            ..chains
        };
        assert_eq!(
            combat_reach_tick(
                &mut pose,
                &leg_maps,
                [true, false],
                &mut locker,
                &bad_chains,
                &BodyTargets {
                    hand_l: None,
                    hand_r: Some([-0.10, 1.15, 0.35]),
                    look: None,
                    aim: None,
                    grip: GripMode::Single,
                    shield_l: None,
                },
            )
            .unwrap_err(),
            FbikError::BadJoint(10)
        );
        assert_eq!(pose, before);
    }

    #[test]
    fn fixed_cost_and_64_fighter_scale() {
        assert_eq!(full_body_cost_cap(), FULL_BODY_OPS_CAP);
        assert_eq!(FULL_BODY_OPS_CAP, 960);
        let chains = chains();
        let mut total = 0u32;
        let mut worst = 0u32;
        for fighter in 0..SCALE_FIGHTERS {
            let lane = fighter as f32 * 0.001;
            let mut pose = rig();
            let sol = solve_full_body(
                &mut pose,
                &chains,
                &BodyTargets {
                    hand_l: Some([0.45 - lane, 1.20, 0.35]),
                    hand_r: Some([-0.35 + lane, 1.15, 0.40]),
                    look: Some([lane, 1.45, 2.0]),
                    aim: Some([-lane, 1.10, 2.0]),
                    grip: GripMode::Single,
                    shield_l: None,
                },
            )
            .expect("scale tick");
            assert!(sol.cost_ops <= FULL_BODY_OPS_CAP, "fighter {fighter}");
            assert!(sol.hand_l.expect("l").error_mm <= HAND_TOL_MM);
            assert!(sol.hand_r.expect("r").error_mm <= HAND_TOL_MM);
            total += sol.cost_ops;
            if sol.cost_ops > worst {
                worst = sol.cost_ops;
            }
        }
        assert!(total <= SCALE_FIGHTERS as u32 * FULL_BODY_OPS_CAP);
        println!(
            "AM9 scale: fighters={} total={} ops worst={} ops cap={} ops/char",
            SCALE_FIGHTERS, total, worst, FULL_BODY_OPS_CAP
        );
    }

    #[test]
    fn ct3_bake_replays_through_reach() {
        let first = bake_hero_set().expect("hero bakes");
        let second = bake_hero_set().expect("hero bakes again");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.db.entries, second.db.entries);
        // Digest-derived sweep: same bake gives the same reach script twice.
        let lane = (first.digest % 50) as f32 * 0.001;
        let run = || {
            let chains = chains();
            let mut pose = rig();
            let mut hashes = Vec::with_capacity(8);
            for frame in 0..8u32 {
                let sweep = 0.30 + frame as f32 * 0.02 + lane;
                let sol = solve_full_body(
                    &mut pose,
                    &chains,
                    &BodyTargets {
                        hand_l: Some([0.45, 1.20, sweep]),
                        hand_r: Some([-0.35, 1.15, sweep]),
                        look: Some([0.0, 1.45, 2.0]),
                        aim: Some([0.0, 1.10, 2.0]),
                        grip: GripMode::Single,
                        shield_l: None,
                    },
                )
                .expect("ct3 reach");
                assert!(sol.hand_l.expect("l").error_mm <= HAND_TOL_MM);
                assert!(sol.hand_r.expect("r").error_mm <= HAND_TOL_MM);
                hashes.push(fbik_hash(&pose));
            }
            hashes
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        println!(
            "AM9 CT3: digest=0x{:016x} frames={} reach_hash=0x{:016x}",
            first.digest, first.frames, a[7]
        );
    }

    #[test]
    fn failures_are_typed_never_panics() {
        let chains = chains();
        let chain = ArmChain::new(3, 4, 5);
        let mut pose = rig();
        assert_eq!(
            solve_reach(&mut pose, &ArmChain::new(3, 4, 99), [0.4, 1.2, 0.3]).unwrap_err(),
            FbikError::BadJoint(99)
        );
        assert_eq!(
            solve_reach(&mut pose, &chain, [f32::NAN, 0.0, 0.0]).unwrap_err(),
            FbikError::NonFinite("target")
        );
        assert_eq!(
            solve_look(&mut pose, 99, [0.0, 1.5, 1.0]).unwrap_err(),
            FbikError::BadJoint(99)
        );
        assert_eq!(
            solve_aim(&mut pose, 1, [f32::INFINITY, 0.0, 0.0]).unwrap_err(),
            FbikError::NonFinite("target")
        );
        let mut narrow = Pose {
            positions: vec![[0.0; 3]; 4],
            rotations: vec![IDENTITY; 4],
        };
        assert_eq!(
            solve_reach(&mut narrow, &chain, [0.4, 1.2, 0.3]).unwrap_err(),
            FbikError::BadJoint(4)
        );
        let mut empty = Pose::zero(0);
        assert_eq!(
            solve_reach(&mut empty, &chain, [0.4, 1.2, 0.3]).unwrap_err(),
            FbikError::EmptyPose
        );
        let mut wide = Pose::zero(MAX_JOINTS + 1);
        assert_eq!(
            solve_full_body(
                &mut wide,
                &chains,
                &BodyTargets {
                    hand_l: None,
                    hand_r: None,
                    look: None,
                    aim: None,
                    grip: GripMode::Single,
                    shield_l: None,
                },
            )
            .unwrap_err(),
            FbikError::LengthMismatch {
                expected: MAX_JOINTS,
                got: MAX_JOINTS + 1
            }
        );
        // Non-finite lanes refuse with the lane named.
        let mut poisoned = rig();
        poisoned.positions[3][0] = f32::NAN;
        assert_eq!(
            solve_reach(&mut poisoned, &chain, [0.4, 1.2, 0.3]).unwrap_err(),
            FbikError::NonFinite("positions")
        );
        // Leg refusal carries the foot locker's cause through.
        let mut pose = rig();
        let mut locker = FootLocker::with_defaults();
        let bad_legs = [
            LegMap::new(9, 10, 11, 0).expect("left"),
            LegMap {
                hip: 12,
                knee: 13,
                ankle: 99,
                foot: 1,
            },
        ];
        match combat_reach_tick(
            &mut pose,
            &bad_legs,
            [true, true],
            &mut locker,
            &chains,
            &BodyTargets {
                hand_l: None,
                hand_r: None,
                look: None,
                aim: None,
                grip: GripMode::Single,
                shield_l: None,
            },
        )
        .unwrap_err()
        {
            FbikError::Leg(FootError::BadJoint(99)) => {}
            other => panic!("expected leg BadJoint(99), got {other:?}"),
        }
        // Errors display loudly for the combat log.
        assert!(FbikError::BadJoint(7).to_string().contains('7'));
        assert!(FbikError::ShieldConflict.to_string().contains("shield"));
        assert!(FbikError::GripConflict { error_mm: 42 }
            .to_string()
            .contains("42"));
    }

    #[test]
    fn steady_state_reuses_caller_buffers() {
        let chains = chains();
        let leg_maps = legs();
        let mut pose = rig();
        let mut locker = FootLocker::with_defaults();
        let targets = BodyTargets {
            hand_l: Some([0.45, 1.20, 0.35]),
            hand_r: Some([-0.35, 1.15, 0.40]),
            look: Some([0.0, 1.45, 2.0]),
            aim: Some([0.0, 1.10, 2.0]),
            grip: GripMode::Single,
            shield_l: None,
        };
        solve_full_body(&mut pose, &chains, &targets).expect("warmup");
        let pos_ptr = pose.positions.as_ptr();
        let rot_ptr = pose.rotations.as_ptr();
        let (pos_cap, rot_cap) = (pose.positions.capacity(), pose.rotations.capacity());
        for tick in 0..60u32 {
            combat_reach_tick(
                &mut pose,
                &leg_maps,
                [tick % 2 == 0, tick % 2 == 1],
                &mut locker,
                &chains,
                &targets,
            )
            .expect("steady tick");
        }
        assert_eq!(pose.positions.as_ptr(), pos_ptr);
        assert_eq!(pose.rotations.as_ptr(), rot_ptr);
        assert_eq!(pose.positions.capacity(), pos_cap);
        assert_eq!(pose.rotations.capacity(), rot_cap);
        assert_eq!(pose.positions.len(), 15);
        assert_eq!(fbik_hash(&pose), fbik_hash(&pose));
    }
}
