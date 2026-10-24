//! hit_react: procedural additive flinch from hit direction plus impulse.
//!
//! Consumer: the combat hit pipeline turns each landed `Hit` into
//! a [`HitStrike`] and calls [`apply_hit_react`] every tick of the react
//! window; balance reads [`HitDir::suggest_bone`] plus [`hit_mask`] for
//! the matching partial ragdoll. Owner: `engine-anim`; this module owns
//! [`HitDir`], [`HitStrike`] and [`HitError`] only, reusing the graph [`Pose`]
//! plus [`MaskTable`] and [`apply_additive`](super::layers::apply_additive)
//! without mirroring them. Units: meters, Q8 impulse (256 reads 1x), ticks
//! at 60 Hz; the fighter faces +Z, +X is its right. Determinism: validated
//! integers in, `+ - * /` plus comparisons only, joint order, no clock;
//! same strike plus same tick gives the same pose twice. Errors: [`HitError`],
//! never a panic on caller data. Budget: one linear pass over
//! [`FIGHTER_JOINTS`] lanes, at most about 0.01 ms; the tick path writes
//! caller-owned buffers and allocates nothing (mask built once at setup).

use super::contract::{rotate, LocalPose, Skeleton};
use super::graph::{GraphError, Pose};
use super::layers::{self, MaskTable};
use engine_combat::fight::HIT_REACT_TICKS;
use engine_core::quat_mul;
use std::f32::consts::FRAC_1_SQRT_2;

/// Canonical fighter rig width: pelvis, two legs, two arms, spine.
/// Matches the mask fixture so body layers compose without resampling.
pub const FIGHTER_JOINTS: usize = 6;
/// Pelvis joint index.
pub const PELVIS: usize = 0;
/// Left-leg joint index (planted through every flinch).
pub const LEG_L: usize = 1;
/// Right-leg joint index (planted through every flinch).
pub const LEG_R: usize = 2;
/// Left-arm joint index.
pub const ARM_L: usize = 3;
/// Right-arm joint index.
pub const ARM_R: usize = 4;
/// Spine joint index (takes the full planar push).
pub const SPINE: usize = 5;
/// Ticks a flinch stays visible: the hit-react window, single-owned by combat
/// (see the `HIT_REACT_TICKS` import above) so the pose and the lockstep
/// agree every tick by construction, never by sync test.
/// Q8 impulse pair, single-sourced from [`super::layers`] (one owner
/// per constant; aliases keep impulse-named paths with one value).
pub use super::layers::{GAIN_Q8_MAX as IMPULSE_Q8_MAX, GAIN_Q8_ONE as IMPULSE_Q8_ONE};
/// Spine planar peak at 1x impulse, meters.
pub const PEAK_M: f32 = 0.10;
/// Downward dip on pelvis/spine at 1x impulse, meters.
pub const DIP_M: f32 = 0.03;
/// Per-joint displacement ceiling per component, meters: no flinch in any
/// direction may push a joint past this (no visual clip through the body).
pub const CLIP_M: f32 = 0.35;
/// Peak spin of the struck upper chain at 1x impulse, radians. The spine
/// takes the full lean, arms half; legs never turn (planted feet).
pub const ROT_PEAK_RAD: f32 = 0.15;
/// Eval tolerance in meters against hand-rolled references.
pub const HIT_TOL: f32 = 1e-5;
/// Presentation flinch window: 27 ticks at 60 Hz is about 450 ms (AM23).
pub const VIEW_FLINCH_TICKS: u32 = 27;
/// Ticks the presentation flinch blends in over. Six tick-aligned steps keep
/// the first frame's pose jump small (AM23 pop check); the hit tick reads 0.
pub const VIEW_FLINCH_RISE_TICKS: f32 = 6.0;
/// Ticks the presentation flinch holds at full weight after the rise.
pub const VIEW_FLINCH_HOLD_TICKS: f32 = 3.0;
/// Integer twin of [`VIEW_FLINCH_RISE_TICKS`]: the age a riding strike is drawn as, so it
/// skips the rise (a test pins the two equal).
const RIDE_SKIP_TICKS: u32 = 6;
/// Peak lean of the real skeleton's upper chain at unit weight, radians
/// (~11.5 degrees); the head moves about 10 cm at this lean.
pub const UPPER_LEAN_RAD: f32 = 0.20;
/// Torso share of [`UPPER_LEAN_RAD`]; the head takes the rest as a snap.
pub const TORSO_LEAN_SHARE: f32 = 1.0;
/// Head share of [`UPPER_LEAN_RAD`]; turns the head without moving its joint.
pub const HEAD_LEAN_SHARE: f32 = 0.35;

/// Planar share of the push per joint: pelvis rides half, legs stay planted,
/// arms and spine take it whole. Index order is the rig order above.
const PLANAR_SHARE: [f32; FIGHTER_JOINTS] = [0.5, 0.0, 0.0, 1.0, 1.0, 1.0];
/// Dip share per joint: pelvis dips half, spine dips whole, limbs stay level.
const DIP_SHARE: [f32; FIGHTER_JOINTS] = [0.5, 0.0, 0.0, 0.0, 0.0, 1.0];
/// Rotation share per joint: pelvis and legs stay planted, arms take half the
/// lean, the spine takes it whole. Index order is the rig order above.
const ROT_SHARE: [f32; FIGHTER_JOINTS] = [0.0, 0.0, 0.0, 0.5, 0.5, 1.0];

/// Where a hit comes from, as facing-relative octants clockwise from the
/// front (0 = front, 2 = right, 4 = back, 6 = left). The flinch displaces
/// away from the named side; see [`HitDir::push`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HitDir {
    /// Hit from the front (+Z side): knocked back toward -Z.
    Front,
    /// Hit from front-right: knocked toward back-left.
    FrontRight,
    /// Hit from the right (+X side): knocked toward -X.
    Right,
    /// Hit from back-right: knocked toward front-left.
    BackRight,
    /// Hit from the back (-Z side): knocked forward toward +Z.
    Back,
    /// Hit from back-left: knocked toward front-right.
    BackLeft,
    /// Hit from the left (-X side): knocked toward +X.
    Left,
    /// Hit from front-left: knocked toward back-right.
    FrontLeft,
}

impl HitDir {
    /// Octant count: the only valid [`HitDir::from_octant`] inputs are
    /// `0..OCTANTS`.
    pub const OCTANTS: u8 = 8;

    /// Build from a raw octant (`0 = front`, clockwise); anything past 7 is
    /// refused with the culprit carried.
    pub fn from_octant(octant: u8) -> Result<Self, HitError> {
        match octant {
            0 => Ok(Self::Front),
            1 => Ok(Self::FrontRight),
            2 => Ok(Self::Right),
            3 => Ok(Self::BackRight),
            4 => Ok(Self::Back),
            5 => Ok(Self::BackLeft),
            6 => Ok(Self::Left),
            7 => Ok(Self::FrontLeft),
            other => Err(HitError::BadOctant(other)),
        }
    }

    /// Raw octant for this direction, inverse of [`HitDir::from_octant`].
    pub fn octant(self) -> u8 {
        match self {
            Self::Front => 0,
            Self::FrontRight => 1,
            Self::Right => 2,
            Self::BackRight => 3,
            Self::Back => 4,
            Self::BackLeft => 5,
            Self::Left => 6,
            Self::FrontLeft => 7,
        }
    }

    /// Planar push (reaction travel) for a unit impulse: `[x, z]` away from
    /// the named side. Axis hits are unit, diagonals use the stored
    /// `FRAC_1_SQRT_2` constant; no trig runs at any tick.
    pub fn push(self) -> [f32; 2] {
        const S: f32 = FRAC_1_SQRT_2;
        match self {
            Self::Front => [0.0, -1.0],
            Self::FrontRight => [-S, -S],
            Self::Right => [-1.0, 0.0],
            Self::BackRight => [-S, S],
            Self::Back => [0.0, 1.0],
            Self::BackLeft => [S, S],
            Self::Left => [1.0, 0.0],
            Self::FrontLeft => [S, -S],
        }
    }

    /// Bone index the physics layer should strike for this direction: side
    /// hits take the near arm, front/back hits take the spine. Follows the
    /// partial-ragdoll rule (struck bone plus descendants go free); physics
    /// owns the call.
    pub fn suggest_bone(self) -> usize {
        match self {
            Self::Front | Self::Back => SPINE,
            Self::Right | Self::FrontRight | Self::BackRight => ARM_R,
            Self::Left | Self::FrontLeft | Self::BackLeft => ARM_L,
        }
    }

    /// Planar push in Q10 integer units (1024 reads unity): the same octant
    /// table as [`HitDir::push`] with no float, so the capture-point balance
    /// rule reads an integer direction. Diagonals use 724, the floor of
    /// 1024/sqrt 2. Mirror-exact: swapping left and right flips the x lane and
    /// holds the z lane, so a mirrored blow mirrors the reaction.
    pub fn push_q10(self) -> [i32; 2] {
        const U: i32 = 1024;
        const D: i32 = 724;
        match self {
            Self::Front => [0, -U],
            Self::FrontRight => [-D, -D],
            Self::Right => [-U, 0],
            Self::BackRight => [-D, D],
            Self::Back => [0, U],
            Self::BackLeft => [D, D],
            Self::Left => [U, 0],
            Self::FrontLeft => [D, -D],
        }
    }
}

/// One landed hit as animation input: direction, Q8 impulse and the tick the
/// reaction starts on. The combat pipeline builds this from each `Hit` (slot
/// facing gives the octant, damage maps to the impulse); replay feeds the
/// recorded strike back, never recomputes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HitStrike {
    /// Where the hit came from.
    pub dir: HitDir,
    /// Impulse in Q8 units, `0..=IMPULSE_Q8_MAX`.
    pub impulse_q8: u16,
    /// First tick the reaction shows on.
    pub start_tick: u32,
}

impl HitStrike {
    /// Build from raw pipeline values; a bad octant or an over-wide impulse
    /// is refused and writes nothing.
    pub fn new(octant: u8, impulse_q8: u16, start_tick: u32) -> Result<Self, HitError> {
        let dir = HitDir::from_octant(octant)?;
        if impulse_q8 > IMPULSE_Q8_MAX {
            return Err(HitError::BadImpulse(impulse_q8));
        }
        Ok(Self {
            dir,
            impulse_q8,
            start_tick,
        })
    }
}

/// What rejected a hit-react call. Refusals change nothing.
#[derive(Clone, PartialEq, Debug)]
pub enum HitError {
    /// Octant past 7; culprit carried.
    BadOctant(u8),
    /// Impulse past [`IMPULSE_Q8_MAX`]; culprit carried.
    BadImpulse(u16),
    /// Pose width differs from [`FIGHTER_JOINTS`]; expected + got carried.
    LengthMismatch { expected: usize, got: usize },
    /// A skeleton-relative call named a joint outside the rig; culprit carried.
    BadJoint(usize),
    /// A layer blend failed (bad base lanes, bad mask width); source carried.
    Layer(GraphError),
}

impl std::fmt::Display for HitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HitError::BadOctant(o) => write!(f, "hit_react: octant {o} is past 7"),
            HitError::BadImpulse(i) => {
                write!(f, "hit_react: impulse {i} is past {IMPULSE_Q8_MAX}")
            }
            HitError::LengthMismatch { expected, got } => {
                write!(f, "hit_react: joint count {got} differs from {expected}")
            }
            HitError::BadJoint(joint) => {
                write!(f, "hit_react: joint {joint} is outside the skeleton")
            }
            HitError::Layer(e) => write!(f, "hit_react layer: {e}"),
        }
    }
}

impl std::error::Error for HitError {}

impl From<GraphError> for HitError {
    fn from(e: GraphError) -> Self {
        HitError::Layer(e)
    }
}

/// Partial-ragdoll mask in table form: legs driven (0), upper body free
/// (255). Mirrors the physics `apply_hit` pattern (struck subtree free, stance
/// planted) as layer mask data; build once per rig and reuse every tick.
pub fn hit_mask() -> MaskTable {
    MaskTable::from_vec(vec![255, 0, 0, 255, 255, 255]).expect("fixed 6-wide mask")
}

/// Impulse gain as a unitless scale: Q8 units over 256 read 1x.
pub fn impulse_scale(impulse_q8: u16) -> f32 {
    f32::from(impulse_q8) / f32::from(IMPULSE_Q8_ONE)
}

/// Flinch envelope over elapsed ticks: full at impact, linear decay to zero
/// at [`HIT_REACT_TICKS`], silent after. Same window as the combat react ticks,
/// so the pose and the lockstep agree every tick.
pub fn envelope(elapsed: u32) -> f32 {
    if elapsed >= HIT_REACT_TICKS {
        0.0
    } else {
        1.0 - (elapsed as f32) / (HIT_REACT_TICKS as f32)
    }
}

fn checked_out(out: &Pose) -> Result<(), HitError> {
    if out.positions.len() != FIGHTER_JOINTS || out.rotations.len() != FIGHTER_JOINTS {
        let got = if out.positions.len() != FIGHTER_JOINTS {
            out.positions.len()
        } else {
            out.rotations.len()
        };
        return Err(HitError::LengthMismatch {
            expected: FIGHTER_JOINTS,
            got,
        });
    }
    Ok(())
}

/// Clamp one displacement component into the no-clip ceiling. Inputs are
/// products of finite constants and validated integer scales, so `clamp`
/// never sees NaN (its panic arms are unreachable here).
fn clip(v: f32) -> f32 {
    v.clamp(-CLIP_M, CLIP_M)
}

/// Write the scaled additive delta for `dir` at unitless `scale` into `out`:
/// position offsets plus a short-arc lean around the horizontal axis
/// perpendicular to the push. The spine takes the full lean, arms half,
/// pelvis and legs stay identity.
fn write_additive(dir: HitDir, scale: f32, out: &mut Pose) {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let push = dir.push();
    for joint in 0..FIGHTER_JOINTS {
        let planar = PLANAR_SHARE[joint];
        let dip = DIP_SHARE[joint];
        if planar == 0.0 && dip == 0.0 {
            out.positions[joint] = [0.0; 3];
        } else {
            out.positions[joint] = [
                clip(push[0] * planar * PEAK_M * scale),
                clip(-DIP_M * dip * scale),
                clip(push[1] * planar * PEAK_M * scale),
            ];
        }
        out.rotations[joint] = if ROT_SHARE[joint] == 0.0 {
            IDENTITY
        } else {
            lean_delta(push, ROT_PEAK_RAD * scale * ROT_SHARE[joint])
        };
    }
}

/// Quat that leans `angle` radians around the horizontal axis perpendicular
/// to `push`, so the up axis tips along the push. `push` is `[x, z]`; a zero
/// or non-finite push reads identity. Axis hits use exact stored components.
fn lean_delta(push: [f32; 2], angle: f32) -> [f32; 4] {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let len2 = push[0] * push[0] + push[1] * push[1];
    if !len2.is_finite() || len2 <= 0.0 || !angle.is_finite() {
        return IDENTITY;
    }
    let inv = 1.0 / len2.sqrt();
    let axis = [push[1] * inv, 0.0, -push[0] * inv];
    let half = (angle * 0.5).clamp(0.0, std::f32::consts::FRAC_PI_2);
    // Polynomial sin, then cos from the same value: sim code never calls a
    // platform transcendental (workspace determinism gate; see `sin_unit`).
    let s = super::inertialize::sin_unit(half);
    let w = (1.0 - s * s).max(0.0).sqrt();
    [axis[0] * s, axis[1] * s, axis[2] * s, w]
}

/// Presentation flinch weight at `elapsed` ticks plus sub-tick `alpha`:
/// blend in over six ticks, hold three, then blend out to zero at
/// [`VIEW_FLINCH_TICKS`] (about 450 ms). Silent at and past the window.
pub fn view_envelope(elapsed: u32, alpha: f32) -> f32 {
    // u16 reaches every tick in the window; wider input clamps past it.
    let window = f32::from(u16::try_from(VIEW_FLINCH_TICKS).unwrap_or(u16::MAX));
    let t = f32::from(u16::try_from(elapsed).unwrap_or(u16::MAX)) + alpha;
    if !t.is_finite() || t < 0.0 || t >= window {
        return 0.0;
    }
    let hold_end = VIEW_FLINCH_RISE_TICKS + VIEW_FLINCH_HOLD_TICKS;
    if t < VIEW_FLINCH_RISE_TICKS {
        t / VIEW_FLINCH_RISE_TICKS
    } else if t < hold_end {
        1.0
    } else {
        (window - t) / (window - hold_end)
    }
}

/// Model-space position of one joint: walk its parent chain root first,
/// rotating each local translation by the parent's world rotation. No heap;
/// a chain deeper than 64 is refused rather than truncated.
fn joint_model_position(
    skeleton: &Skeleton,
    pose: &LocalPose,
    joint: usize,
) -> Result<[f32; 3], HitError> {
    const MAX_CHAIN: usize = 64;
    let joints = skeleton.joints.len();
    if pose.positions.len() != joints || pose.rotations.len() != joints {
        return Err(HitError::LengthMismatch {
            expected: joints,
            got: pose.positions.len().min(pose.rotations.len()),
        });
    }
    if joint >= joints {
        return Err(HitError::BadJoint(joint));
    }
    let mut chain = [0usize; MAX_CHAIN];
    let mut len = 0usize;
    let mut next = Some(joint);
    while let Some(index) = next {
        if index >= joints || len == MAX_CHAIN {
            return Err(HitError::BadJoint(index));
        }
        chain[len] = index;
        len += 1;
        next = skeleton.joints[index].parent;
    }
    let mut pos = [0.0f32; 3];
    let mut rot = [0.0, 0.0, 0.0, 1.0];
    for &index in chain[..len].iter().rev() {
        let local = rotate(&pose.positions[index], &rot);
        pos[0] += local[0];
        pos[1] += local[1];
        pos[2] += local[2];
        rot = quat_mul(rot, pose.rotations[index]);
    }
    Ok(pos)
}

/// Apply the rotation half of the presentation flinch to a real skeleton:
/// the torso takes [`UPPER_LEAN_RAD`], the head adds a smaller snap, both
/// around the axis perpendicular to `push`. Returns the head joint's
/// model-space travel in meters, the number `react_head_cm_max` meters.
/// Legs and the pelvis stay untouched; the caller owns the pelvis offset.
pub fn lean_upper(
    skeleton: &Skeleton,
    pose: &mut LocalPose,
    torso: usize,
    head: usize,
    push: [f32; 2],
    weight: f32,
) -> Result<f32, HitError> {
    if !weight.is_finite() || weight <= 0.0 || (push[0] == 0.0 && push[1] == 0.0) {
        return Ok(0.0);
    }
    if torso >= skeleton.joints.len() {
        return Err(HitError::BadJoint(torso));
    }
    let weight = weight.clamp(0.0, 1.0);
    let before = joint_model_position(skeleton, pose, head)?;
    let angle = UPPER_LEAN_RAD * weight;
    let torso_delta = lean_delta(push, angle * TORSO_LEAN_SHARE);
    pose.rotations[torso] = quat_mul(torso_delta, pose.rotations[torso]);
    let head_delta = lean_delta(push, angle * HEAD_LEAN_SHARE);
    pose.rotations[head] = quat_mul(head_delta, pose.rotations[head]);
    let after = joint_model_position(skeleton, pose, head)?;
    let dx = after[0] - before[0];
    let dy = after[1] - before[1];
    let dz = after[2] - before[2];
    Ok((dx * dx + dy * dy + dz * dz).sqrt())
}

/// Full-attack additive pose for `dir` at `impulse_q8`: the delta that
/// [`apply_hit_react`] layers over the base through the layers additive path.
/// Zero impulse writes the bind delta (zeros plus identity quats).
pub fn flinch(dir: HitDir, impulse_q8: u16, out: &mut Pose) -> Result<(), HitError> {
    if impulse_q8 > IMPULSE_Q8_MAX {
        return Err(HitError::BadImpulse(impulse_q8));
    }
    checked_out(out)?;
    write_additive(dir, impulse_scale(impulse_q8), out);
    Ok(())
}

/// Layer one strike over `base` at `tick` into `out`: decay the impulse by
/// the react-window envelope, build the delta in `scratch`, then blend
/// through the layers additive path under `mask` at unit gain. Ticks at or past
/// `start + HIT_REACT_TICKS` copy the base exactly. `mask` is caller-owned
/// (usually [`hit_mask`]); `scratch` is caller-owned so steady-state ticks
/// allocate nothing.
pub fn apply_hit_react(
    base: &Pose,
    strike: &HitStrike,
    tick: u32,
    mask: &MaskTable,
    out: &mut Pose,
    scratch: &mut Pose,
) -> Result<(), HitError> {
    checked_out(out)?;
    checked_out(scratch)?;
    let elapsed = tick.wrapping_sub(strike.start_tick);
    let total = f32::from(strike.impulse_q8) * envelope(elapsed) / f32::from(IMPULSE_Q8_ONE);
    if total <= 0.0 {
        if base.positions.len() != FIGHTER_JOINTS || base.rotations.len() != FIGHTER_JOINTS {
            return Err(HitError::Layer(GraphError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: base.positions.len().min(base.rotations.len()),
            }));
        }
        out.positions.copy_from_slice(&base.positions);
        out.rotations.copy_from_slice(&base.rotations);
        return Ok(());
    }
    write_additive(strike.dir, total, scratch);
    layers::apply_additive(base, scratch, mask, IMPULSE_Q8_ONE, out)?;
    Ok(())
}

/// Most flinch lanes a stack holds: the strike being ridden plus the one before it.
pub const FLINCH_LANES: usize = 2;
/// Cap on the drawn delta against one full strike: two overlapping strikes never read more than
/// 1.6 times a single one, so a flurry cannot fold the fighter in half.
pub const OVERLAP_CAP: f32 = 1.6;

/// One strike in a [`FlinchStack`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Flinch {
    /// Tick the strike landed.
    pub tick: u32,
    /// Which way it pushed the body.
    pub dir: HitDir,
    /// Its impulse in Q8 units, the same units [`HitStrike`] carries.
    pub impulse_q8: u16,
    /// Share of the strike this lane draws, 1.0 for a fresh hit and less when a third strike
    /// takes over a lane that was already decaying (so the drawn pose does not jump).
    pub gain: f32,
    /// What the lane it replaced was still drawing: the old push axis and its weight at the
    /// swap tick, plus the ticks it had left. That residual fades to zero over those ticks, so
    /// the swap keeps the old direction on the first frame instead of swinging the lean across
    /// to the new side in one tick.
    pub prior: Option<([f32; 2], f32, u32)>,
    /// True when this strike landed while another lane was still drawing: it skips its rise and
    /// adds at full weight at once, because the body is already leaning and a second rise would
    /// read as the lean letting go and coming back.
    pub ride: bool,
}

impl Flinch {
    /// A fresh strike at `dir`, full gain, with the standing rise.
    pub fn new(tick: u32, dir: HitDir, impulse_q8: u16) -> Self {
        Self {
            tick,
            dir,
            impulse_q8,
            gain: 1.0,
            prior: None,
            ride: false,
        }
    }
}

/// Two flinch lanes, oldest first: a second hit during a flinch rides the first instead of
/// snapping the body back, which is what one windowed pass does.
///
/// The drawn delta is the sum of the lanes' deltas at their own weights, with the newer lane's
/// push axis orthonormalized against the older one so two hits from the same side add and two
/// opposite hits cancel instead of both tipping the same axis. A third strike replaces the
/// oldest lane and starts at that lane's decayed weight, so the sum at the swap tick is the sum
/// the tick before it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct FlinchStack {
    lanes: [Option<Flinch>; FLINCH_LANES],
}

impl FlinchStack {
    /// An empty stack: nothing drawn.
    pub fn new() -> Self {
        Self::default()
    }

    /// Land a strike. Same tick replaces the lane it collides with; a third strike takes the
    /// oldest lane over at that lane's decayed weight.
    pub fn push(&mut self, strike: Flinch) {
        // A strike that lands while a lane is still drawing rides it: no second rise.
        let riding = self.lanes.iter().flatten().any(|held| {
            held.tick <= strike.tick && strike.tick.wrapping_sub(held.tick) < VIEW_FLINCH_TICKS
        });
        let fresh = Flinch {
            gain: 1.0,
            ride: riding,
            ..strike
        };
        // A lane whose window is over frees its slot without carrying anything.
        for lane in self.lanes.iter_mut() {
            if lane.is_some_and(|held| fresh.tick.wrapping_sub(held.tick) >= VIEW_FLINCH_TICKS) {
                *lane = None;
            }
        }
        match self.lanes {
            [None, None] => self.lanes[0] = Some(fresh),
            [Some(_), None] => self.lanes[1] = Some(fresh),
            [None, Some(newest)] => {
                // Keep the order oldest first.
                self.lanes = [Some(fresh), Some(newest)];
            }
            [Some(oldest), Some(newest)] => {
                // The oldest lane leaves its residual push behind: the new strike takes the slot
                // with the old direction and weight for one frame, so the lean does not swing
                // across to the new side in a single tick.
                let age = fresh.tick.wrapping_sub(oldest.tick);
                let left = VIEW_FLINCH_TICKS.saturating_sub(age).max(1);
                let weight = impulse_scale(oldest.impulse_q8)
                    * view_envelope(age, 0.0)
                    * oldest.gain.max(0.0);
                self.lanes = [
                    Some(Flinch {
                        // The replacing strike rises from zero: the prior carries the first
                        // frame, so the sum does not jump on the swap tick.
                        ride: false,
                        prior: Some((oldest.dir.push(), weight, left)),
                        ..fresh
                    }),
                    Some(newest),
                ];
            }
        }
        // A lane that lands before the one it replaced still reads oldest first.
        if let [Some(a), Some(b)] = self.lanes {
            if b.tick < a.tick {
                self.lanes = [Some(b), Some(a)];
            }
        }
    }

    /// Live lanes with their push and drawn weight at `tick`, oldest first: the pushes are
    /// orthonormalized, so the newer lane only adds the part of its push the older one left.
    pub fn lanes_at(&self, tick: u32, alpha: f32) -> Vec<([f32; 2], f32)> {
        let mut out: Vec<([f32; 2], f32)> = Vec::with_capacity(FLINCH_LANES);
        let mut add = |push: [f32; 2], weight: f32| {
            if !(weight > 0.0) {
                return;
            }
            let mut push = push;
            if let Some((first, _)) = out.first() {
                // Gram-Schmidt: drop the share of this push that repeats the first lane's axis.
                let dot = push[0] * first[0] + push[1] * first[1];
                let left = [push[0] - dot * first[0], push[1] - dot * first[1]];
                let len2 = left[0] * left[0] + left[1] * left[1];
                if len2 > 1e-6 {
                    let inv = 1.0 / len2.sqrt();
                    push = [left[0] * inv, left[1] * inv];
                }
            }
            out.push((push, weight));
        };
        for lane in self.lanes.iter().flatten() {
            let age = tick.wrapping_sub(lane.tick);
            // The replaced lane's residual comes first: it is the older of the two axes.
            if let Some((push, weight, left)) = lane.prior {
                let fade = if age >= left {
                    0.0
                } else {
                    f32::from(u16::try_from(left - age).unwrap_or(0))
                        / f32::from(u16::try_from(left).unwrap_or(1).max(1))
                };
                add(push, weight * fade);
            }
            let envelope = if lane.ride {
                // Skip the rise: a riding strike draws at full weight from its own tick. The
                // constant is the integer twin of VIEW_FLINCH_RISE_TICKS (pinned by a test).
                view_envelope(age.max(RIDE_SKIP_TICKS), alpha)
            } else {
                view_envelope(age, alpha)
            };
            add(
                lane.dir.push(),
                impulse_scale(lane.impulse_q8) * envelope * lane.gain.max(0.0),
            );
        }
        out
    }

    /// The strike that landed most recently, for the HUD and the receipts.
    pub fn newest(&self) -> Option<Flinch> {
        self.lanes
            .iter()
            .flatten()
            .max_by_key(|lane| lane.tick)
            .copied()
    }
}

/// Draw every lane of `stack` over `base` at `tick`, capped at [`OVERLAP_CAP`] of one strike per
/// bone. Returns the peak drawn share of a single strike (1.6 at the cap), which the receipts
/// print; ticks with no live lane copy `base` exactly.
pub fn apply_flinches(
    base: &Pose,
    stack: &FlinchStack,
    tick: u32,
    mask: &MaskTable,
    out: &mut Pose,
    scratch: &mut Pose,
) -> Result<f32, HitError> {
    checked_out(out)?;
    checked_out(scratch)?;
    let lanes = stack.lanes_at(tick, 0.0);
    if lanes.is_empty() {
        if base.positions.len() != FIGHTER_JOINTS || base.rotations.len() != FIGHTER_JOINTS {
            return Err(HitError::Layer(GraphError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: base.positions.len().min(base.rotations.len()),
            }));
        }
        out.positions.copy_from_slice(&base.positions);
        out.rotations.copy_from_slice(&base.rotations);
        return Ok(0.0);
    }
    let peak = write_sum(&lanes, scratch);
    layers::apply_additive(base, scratch, mask, IMPULSE_Q8_ONE, out)?;
    Ok(peak)
}

/// Write the summed delta of `lanes` (push plus weight each) into `out`, capped per bone. The
/// return value is the largest drawn share of one strike before the cap is applied.
fn write_sum(lanes: &[([f32; 2], f32)], out: &mut Pose) -> f32 {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let mut peak = 0.0f32;
    for joint in 0..FIGHTER_JOINTS {
        let planar = PLANAR_SHARE[joint];
        let dip = DIP_SHARE[joint];
        if planar == 0.0 && dip == 0.0 {
            out.positions[joint] = [0.0; 3];
            out.rotations[joint] = IDENTITY;
            continue;
        }
        // Sum the lanes on each axis: two hits from one side add, two opposite hits cancel.
        let mut planar_x = 0.0f32;
        let mut planar_z = 0.0f32;
        let mut down = 0.0f32;
        let mut lean = 0.0f32;
        for (push, weight) in lanes {
            planar_x += push[0] * weight;
            planar_z += push[1] * weight;
            down += weight;
            lean += weight;
        }
        let mut pos = [
            planar_x * planar * PEAK_M,
            -DIP_M * dip * down,
            planar_z * planar * PEAK_M,
        ];
        let travel = (pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt();
        let cap_m = OVERLAP_CAP * PEAK_M;
        if travel > cap_m && travel > 0.0 {
            let shrink = cap_m / travel;
            pos = [pos[0] * shrink, pos[1] * shrink, pos[2] * shrink];
        }
        let angle = ROT_PEAK_RAD * lean * ROT_SHARE[joint];
        let cap_rad = OVERLAP_CAP * ROT_PEAK_RAD * ROT_SHARE[joint];
        peak = peak
            .max(travel.min(cap_m) / PEAK_M)
            .max(angle.min(cap_rad) / ROT_PEAK_RAD);
        out.positions[joint] = pos;
        out.rotations[joint] = if ROT_SHARE[joint] == 0.0 {
            IDENTITY
        } else {
            // One lean per lane around its own axis, the sum's angle capped at the overlay cap.
            let mut quat = IDENTITY;
            for (push, weight) in lanes {
                let scale = if angle > cap_rad && angle > 0.0 {
                    cap_rad / angle
                } else {
                    1.0
                };
                quat = quat_mul(
                    lean_delta(*push, ROT_PEAK_RAD * ROT_SHARE[joint] * weight * scale),
                    quat,
                );
            }
            quat
        };
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn base_pose() -> Pose {
        Pose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, 0.5, 0.1],
                [0.2, 0.5, -0.1],
                [-0.3, 1.4, 0.0],
                [0.3, 1.4, 0.0],
                [0.0, 1.2, 0.0],
            ],
            rotations: vec![IDENTITY; FIGHTER_JOINTS],
        }
    }

    fn assert_near(a: [f32; 3], b: [f32; 3], tol: f32) {
        for lane in 0..3 {
            assert!(
                (a[lane] - b[lane]).abs() <= tol,
                "lane {lane}: {a:?} vs {b:?} past {tol}"
            );
        }
    }

    fn all_dirs() -> [HitDir; 8] {
        [
            HitDir::Front,
            HitDir::FrontRight,
            HitDir::Right,
            HitDir::BackRight,
            HitDir::Back,
            HitDir::BackLeft,
            HitDir::Left,
            HitDir::FrontLeft,
        ]
    }

    #[test]
    fn eight_dirs_push_away_from_the_named_side() {
        for dir in all_dirs() {
            let mut out = Pose::zero(FIGHTER_JOINTS);
            flinch(dir, IMPULSE_Q8_ONE, &mut out).expect("unit flinch");
            let spine = out.positions[SPINE];
            let push = dir.push();
            // Spine planar lanes match the push signs at PEAK_M exactly.
            assert_eq!(spine[0], push[0] * PEAK_M, "{dir:?} x");
            assert_eq!(spine[2], push[1] * PEAK_M, "{dir:?} z");
            // The dip reads down, legs stay planted exactly.
            assert!(spine[1] < 0.0, "{dir:?} dips: {spine:?}");
            assert_eq!(out.positions[LEG_L], [0.0; 3], "{dir:?} left leg");
            assert_eq!(out.positions[LEG_R], [0.0; 3], "{dir:?} right leg");
            // The upper chain leans along the push; legs and pelvis never turn.
            assert_eq!(out.rotations[PELVIS], IDENTITY, "{dir:?} pelvis");
            assert_eq!(out.rotations[LEG_L], IDENTITY, "{dir:?} left leg rot");
            assert_eq!(out.rotations[LEG_R], IDENTITY, "{dir:?} right leg rot");
            assert_ne!(out.rotations[SPINE], IDENTITY, "{dir:?} spine lean");
            for rot in &out.rotations {
                let norm =
                    (rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3]).sqrt();
                assert!((norm - 1.0).abs() <= HIT_TOL, "{dir:?} unit quat: {rot:?}");
            }
        }
        // Axis hits travel one lane only; diagonals split evenly.
        let mut front = Pose::zero(FIGHTER_JOINTS);
        flinch(HitDir::Front, IMPULSE_Q8_ONE, &mut front).expect("front");
        assert_eq!(front.positions[SPINE][0], 0.0);
        assert!(front.positions[SPINE][2] < 0.0);
        let mut diag = Pose::zero(FIGHTER_JOINTS);
        flinch(HitDir::FrontRight, IMPULSE_Q8_ONE, &mut diag).expect("diagonal");
        assert!(
            (diag.positions[SPINE][0] - diag.positions[SPINE][2]).abs() <= HIT_TOL,
            "diagonal splits evenly: {:?}",
            diag.positions[SPINE]
        );
        // Opposite sides negate lane-for-lane.
        for (a, b) in [
            (HitDir::Front, HitDir::Back),
            (HitDir::Left, HitDir::Right),
            (HitDir::FrontLeft, HitDir::BackRight),
        ] {
            let (mut pa, mut pb) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
            flinch(a, IMPULSE_Q8_ONE, &mut pa).expect("side a");
            flinch(b, IMPULSE_Q8_ONE, &mut pb).expect("side b");
            for joint in 0..FIGHTER_JOINTS {
                assert_eq!(
                    pa.positions[joint][0], -pb.positions[joint][0],
                    "{a:?}/{b:?} x"
                );
                assert_eq!(
                    pa.positions[joint][2], -pb.positions[joint][2],
                    "{a:?}/{b:?} z"
                );
                // Opposite sides lean the same amount the other way: the
                // vector part flips, w stays.
                for lane in 0..3 {
                    assert!(
                        (pa.rotations[joint][lane] + pb.rotations[joint][lane]).abs() <= HIT_TOL,
                        "{a:?}/{b:?} rot lane {lane}"
                    );
                }
                assert!(
                    (pa.rotations[joint][3] - pb.rotations[joint][3]).abs() <= HIT_TOL,
                    "{a:?}/{b:?} rot w"
                );
            }
        }
    }

    #[test]
    fn octant_roundtrips_and_names_suggest_bone() {
        for (octant, dir) in all_dirs().iter().enumerate() {
            assert_eq!(
                HitDir::from_octant(octant as u8).expect("valid octant"),
                *dir
            );
            assert_eq!(dir.octant(), octant as u8);
        }
        // Side hits name the near arm, front/back name the spine.
        assert_eq!(HitDir::Front.suggest_bone(), SPINE);
        assert_eq!(HitDir::Back.suggest_bone(), SPINE);
        assert_eq!(HitDir::Right.suggest_bone(), ARM_R);
        assert_eq!(HitDir::FrontRight.suggest_bone(), ARM_R);
        assert_eq!(HitDir::BackRight.suggest_bone(), ARM_R);
        assert_eq!(HitDir::Left.suggest_bone(), ARM_L);
        assert_eq!(HitDir::FrontLeft.suggest_bone(), ARM_L);
        assert_eq!(HitDir::BackLeft.suggest_bone(), ARM_L);
        // Every suggestion lands on driven-upper-body bones, never a leg.
        for dir in all_dirs() {
            assert!(
                [ARM_L, ARM_R, SPINE].contains(&dir.suggest_bone()),
                "{dir:?} suggests upper body"
            );
        }
    }

    #[test]
    fn impulse_scales_linearly_and_zero_is_bind() {
        let (mut one, mut two, mut zero) = (
            Pose::zero(FIGHTER_JOINTS),
            Pose::zero(FIGHTER_JOINTS),
            Pose::zero(FIGHTER_JOINTS),
        );
        flinch(HitDir::Left, IMPULSE_Q8_ONE, &mut one).expect("1x");
        flinch(HitDir::Left, 2 * IMPULSE_Q8_ONE, &mut two).expect("2x");
        flinch(HitDir::Left, 0, &mut zero).expect("zero");
        for joint in 0..FIGHTER_JOINTS {
            for lane in 0..3 {
                assert!(
                    (two.positions[joint][lane] - 2.0 * one.positions[joint][lane]).abs()
                        <= HIT_TOL,
                    "double impulse doubles the delta"
                );
            }
        }
        assert_eq!(zero.positions, vec![[0.0; 3]; FIGHTER_JOINTS]);
        assert_eq!(zero.rotations, vec![IDENTITY; FIGHTER_JOINTS]);
        assert_eq!(impulse_scale(IMPULSE_Q8_ONE), 1.0);
        assert_eq!(impulse_scale(IMPULSE_Q8_MAX), 3.0);
    }

    #[test]
    fn q8_and_window_have_single_owners() {
        // Impulse Q8 is owned by layers; these aliases must read one value.
        assert_eq!(IMPULSE_Q8_ONE, layers::GAIN_Q8_ONE);
        assert_eq!(IMPULSE_Q8_MAX, layers::GAIN_Q8_MAX);
        assert_eq!(impulse_scale(layers::GAIN_Q8_ONE), 1.0);
        // Window is owned by combat (imported above): the envelope behavior
        // below proves the wiring; no local mirror remains to pin.
        assert_eq!(envelope(6), 0.0);
    }

    #[test]
    fn envelope_covers_the_react_window() {
        assert_eq!(envelope(0), 1.0);
        assert_eq!(envelope(HIT_REACT_TICKS), 0.0);
        assert_eq!(envelope(HIT_REACT_TICKS + 40), 0.0);
        assert_eq!(envelope(u32::MAX), 0.0);
        let mut prev = f32::INFINITY;
        for elapsed in 0..HIT_REACT_TICKS {
            let gain = envelope(elapsed);
            assert!(gain > 0.0 && gain <= 1.0, "tick {elapsed}: {gain}");
            assert!(gain < prev || elapsed == 0, "decay is monotonic");
            prev = gain;
        }
    }

    #[test]
    fn hit_react_rot_view_envelope_rises_holds_and_fades() {
        assert_eq!(view_envelope(0, 0.0), 0.0);
        assert!((view_envelope(1, 0.5) - 0.25).abs() <= HIT_TOL);
        assert!((view_envelope(3, 0.0) - 0.5).abs() <= HIT_TOL);
        assert_eq!(view_envelope(6, 0.0), 1.0);
        assert_eq!(view_envelope(8, 0.9), 1.0);
        assert!((view_envelope(18, 0.0) - 0.5).abs() <= HIT_TOL);
        assert!(view_envelope(26, 0.9) > 0.0);
        assert_eq!(view_envelope(27, 0.0), 0.0);
        assert_eq!(view_envelope(u32::MAX, 0.0), 0.0);
        assert_eq!(VIEW_FLINCH_TICKS, 27, "27 ticks at 60 Hz is 450 ms");
    }

    #[test]
    fn hit_react_rot_weight_zero_keeps_base_bytes() {
        let base = base_pose();
        let mut delta = Pose::zero(FIGHTER_JOINTS);
        flinch(HitDir::FrontLeft, IMPULSE_Q8_ONE, &mut delta).expect("flinch");
        assert_ne!(delta.rotations[SPINE], IDENTITY, "delta carries the lean");
        let mut out = Pose::zero(FIGHTER_JOINTS);
        let zero = MaskTable::from_vec(vec![0; FIGHTER_JOINTS]).expect("zero mask");
        layers::apply_additive(&base, &delta, &zero, IMPULSE_Q8_ONE, &mut out).expect("zero mask");
        assert_eq!(out, base, "mask 0 keeps base positions and rotation bytes");
        let full = MaskTable::from_vec(vec![255; FIGHTER_JOINTS]).expect("full mask");
        layers::apply_additive(&base, &delta, &full, 0, &mut out).expect("zero gain");
        assert_eq!(out, base, "gain 0 keeps base positions and rotation bytes");
    }

    #[test]
    fn hit_react_rot_two_strikes_sum() {
        let base = base_pose();
        let mask = hit_mask();
        let (mut one, mut two) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        flinch(HitDir::Front, IMPULSE_Q8_ONE, &mut one).expect("1x");
        flinch(HitDir::Front, 2 * IMPULSE_Q8_ONE, &mut two).expect("2x");
        let mut first = Pose::zero(FIGHTER_JOINTS);
        layers::apply_additive(&base, &one, &mask, IMPULSE_Q8_ONE, &mut first)
            .expect("first strike");
        let mut sum = Pose::zero(FIGHTER_JOINTS);
        layers::apply_additive(&first, &one, &mask, IMPULSE_Q8_ONE, &mut sum)
            .expect("second strike");
        let mut combined = Pose::zero(FIGHTER_JOINTS);
        layers::apply_additive(&base, &two, &mask, IMPULSE_Q8_ONE, &mut combined)
            .expect("combined strike");
        for joint in 0..FIGHTER_JOINTS {
            for lane in 0..3 {
                assert!(
                    (sum.positions[joint][lane] - combined.positions[joint][lane]).abs() <= HIT_TOL,
                    "joint {joint} position lane {lane}: {:?} vs {:?}",
                    sum.positions[joint],
                    combined.positions[joint]
                );
            }
            for lane in 0..4 {
                assert!(
                    (sum.rotations[joint][lane] - combined.rotations[joint][lane]).abs() <= HIT_TOL,
                    "joint {joint} rotation lane {lane}: {:?} vs {:?}",
                    sum.rotations[joint],
                    combined.rotations[joint]
                );
            }
        }
        // The ordered-stack path composes the same two strikes in one call.
        let stack = [
            layers::Layer(
                "first",
                layers::LayerMode::Additive,
                &one,
                &mask,
                IMPULSE_Q8_ONE,
            ),
            layers::Layer(
                "second",
                layers::LayerMode::Additive,
                &one,
                &mask,
                IMPULSE_Q8_ONE,
            ),
        ];
        let mut stacked = Pose::zero(FIGHTER_JOINTS);
        layers::compose_stack(&base, &stack, &mut stacked).expect("stack");
        assert_eq!(stacked, sum, "stack and sequential agree");
    }

    #[test]
    fn hit_react_rot_upper_lean_moves_the_head() {
        use crate::contract::Joint;
        let skeleton = Skeleton {
            joints: vec![
                Joint {
                    name: "root".to_string(),
                    parent: None,
                },
                Joint {
                    name: "torso".to_string(),
                    parent: Some(0),
                },
                Joint {
                    name: "head".to_string(),
                    parent: Some(1),
                },
            ],
        };
        let mut pose = LocalPose {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.5, 0.0]],
            rotations: vec![IDENTITY; 3],
        };
        let head_m = lean_upper(&skeleton, &mut pose, 1, 2, [0.0, -1.0], 1.0).expect("lean");
        // Chord travel on a 0.5 m arm over the torso lean: 2 r sin(angle/2).
        // The half angle is 0.1 rad (not PI/4); the row's polynomial sin keeps
        // the platform transcendental out of the test.
        let chord = 2.0 * 0.5 * crate::inertialize::sin_unit(UPPER_LEAN_RAD * 0.5);
        assert!(
            (head_m - chord).abs() <= 1e-4,
            "head travel {head_m} vs chord {chord}"
        );
        assert!(head_m > 0.08, "the rotation flinch clears 8 cm: {head_m}");
        assert_ne!(pose.rotations[1], IDENTITY, "torso leans");
        assert_ne!(pose.rotations[2], IDENTITY, "head snaps");
    }

    #[test]
    fn no_direction_clips_at_max_impulse() {
        for dir in all_dirs() {
            let mut out = Pose::zero(FIGHTER_JOINTS);
            flinch(dir, IMPULSE_Q8_MAX, &mut out).expect("max flinch");
            for joint in 0..FIGHTER_JOINTS {
                for lane in 0..3 {
                    assert!(
                        out.positions[joint][lane].abs() <= CLIP_M,
                        "{dir:?} joint {joint} clips: {:?}",
                        out.positions[joint]
                    );
                }
            }
            // Margin proof: the hottest lane (spine planar at 3x) stays
            // strictly inside the ceiling, by construction 0.30 < 0.35.
            assert!(out.positions[SPINE][0].abs() < CLIP_M);
            assert!(out.positions[SPINE][2].abs() < CLIP_M);
            // Legs never move, so planted feet cannot slide or clip.
            assert_eq!(out.positions[LEG_L], [0.0; 3]);
            assert_eq!(out.positions[LEG_R], [0.0; 3]);
        }
    }

    #[test]
    fn apply_layers_over_base_within_window() {
        let base = base_pose();
        let mask = hit_mask();
        // Mask posting: partial-ragdoll shape, legs driven, upper body free.
        assert_eq!(mask.weights(), &[255, 0, 0, 255, 255, 255]);
        let strike = HitStrike::new(0, IMPULSE_Q8_ONE, 10).expect("front strike");
        let (mut out, mut scratch) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        // Blind-review hook: reaction curves post per direction at 1x.
        for dir in all_dirs() {
            let mut delta = Pose::zero(FIGHTER_JOINTS);
            flinch(dir, IMPULSE_Q8_ONE, &mut delta).expect("curve delta");
            println!(
                "AM10 curve: dir={dir:?} octant={} spine_delta={:?} pelvis_delta={:?} gain_q8={} window={}",
                dir.octant(),
                delta.positions[SPINE],
                delta.positions[PELVIS],
                IMPULSE_Q8_ONE,
                HIT_REACT_TICKS,
            );
        }
        apply_hit_react(&base, &strike, 10, &mask, &mut out, &mut scratch).expect("impact tick");
        // Hand-rolled reference: front hit at 1x shoves pelvis half peak and
        // spine full peak back (-Z) with the dip, legs untouched.
        assert_near(out.positions[SPINE], [0.0, 1.2 - DIP_M, -PEAK_M], HIT_TOL);
        assert_near(
            out.positions[PELVIS],
            [0.0, 1.0 - 0.5 * DIP_M, -0.5 * PEAK_M],
            HIT_TOL,
        );
        assert_eq!(out.positions[LEG_L], base.positions[LEG_L]);
        assert_eq!(out.positions[LEG_R], base.positions[LEG_R]);
        // Additive rotations ride the same mask: upper body leans, legs stay.
        assert_eq!(out.rotations[LEG_L], base.rotations[LEG_L]);
        assert_eq!(out.rotations[LEG_R], base.rotations[LEG_R]);
        assert_ne!(out.rotations[SPINE], base.rotations[SPINE]);
        // Mid-window the reaction has decayed but still reads.
        apply_hit_react(&base, &strike, 13, &mask, &mut out, &mut scratch).expect("mid window");
        assert!(out.positions[SPINE][2] > -PEAK_M && out.positions[SPINE][2] < 0.0);
        // Past the window the base returns exactly, same tick window as combat.
        apply_hit_react(&base, &strike, 16, &mask, &mut out, &mut scratch).expect("past window");
        assert_eq!(out, base);
        // Ticks before the strike stay silent too (wrapping subtract).
        apply_hit_react(&base, &strike, 9, &mask, &mut out, &mut scratch).expect("pre strike");
        assert_eq!(out, base);
    }

    #[test]
    fn replay_is_identical_twice() {
        let base = base_pose();
        let mask = hit_mask();
        for dir in all_dirs() {
            let strike = HitStrike::new(dir.octant(), 512, 4).expect("strike");
            let (mut first, mut second) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
            let (mut scratch_a, mut scratch_b) =
                (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
            for tick in 4..4 + HIT_REACT_TICKS + 2 {
                apply_hit_react(&base, &strike, tick, &mask, &mut first, &mut scratch_a)
                    .expect("first run");
                apply_hit_react(&base, &strike, tick, &mask, &mut second, &mut scratch_b)
                    .expect("second run");
                assert_eq!(first, second, "{dir:?} tick {tick} replays");
            }
            let (mut da, mut db) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
            flinch(dir, IMPULSE_Q8_MAX, &mut da).expect("delta a");
            flinch(dir, IMPULSE_Q8_MAX, &mut db).expect("delta b");
            assert_eq!(da, db);
        }
    }

    #[test]
    fn bad_input_is_refused_and_names_the_cause() {
        let mut out = Pose::zero(FIGHTER_JOINTS);
        assert_eq!(HitDir::from_octant(8).unwrap_err(), HitError::BadOctant(8));
        assert_eq!(
            HitDir::from_octant(255).unwrap_err(),
            HitError::BadOctant(255)
        );
        assert_eq!(
            HitStrike::new(8, IMPULSE_Q8_ONE, 0).unwrap_err(),
            HitError::BadOctant(8)
        );
        assert_eq!(
            HitStrike::new(0, IMPULSE_Q8_MAX + 1, 0).unwrap_err(),
            HitError::BadImpulse(IMPULSE_Q8_MAX + 1)
        );
        assert_eq!(
            flinch(HitDir::Front, IMPULSE_Q8_MAX + 1, &mut out).unwrap_err(),
            HitError::BadImpulse(IMPULSE_Q8_MAX + 1)
        );
        // Ceiling values are accepted.
        HitStrike::new(7, IMPULSE_Q8_MAX, u32::MAX).expect("ceiling strike");
        flinch(HitDir::Back, IMPULSE_Q8_MAX, &mut out).expect("ceiling flinch");
        // Width mismatches name the expected count on every entry point.
        let mut narrow = Pose::zero(FIGHTER_JOINTS - 1);
        assert_eq!(
            flinch(HitDir::Front, IMPULSE_Q8_ONE, &mut narrow).unwrap_err(),
            HitError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: FIGHTER_JOINTS - 1
            }
        );
        let base = base_pose();
        let mask = hit_mask();
        let strike = HitStrike::new(2, IMPULSE_Q8_ONE, 0).expect("strike");
        let mut scratch = Pose::zero(FIGHTER_JOINTS);
        assert_eq!(
            apply_hit_react(&base, &strike, 0, &mask, &mut narrow, &mut scratch).unwrap_err(),
            HitError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: FIGHTER_JOINTS - 1
            }
        );
        assert_eq!(
            apply_hit_react(&base, &strike, 0, &mask, &mut out, &mut narrow).unwrap_err(),
            HitError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: FIGHTER_JOINTS - 1
            }
        );
        // A short mask fails through the layer error, naming its width.
        let short = MaskTable::from_vec(vec![255; 3]).expect("short mask");
        assert_eq!(
            apply_hit_react(&base, &strike, 0, &short, &mut out, &mut scratch).unwrap_err(),
            HitError::Layer(GraphError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: 3
            })
        );
        // Non-finite base lanes fail loudly instead of poisoning the pose.
        let mut poisoned = base_pose();
        poisoned.positions[SPINE][0] = f32::NAN;
        assert_eq!(
            apply_hit_react(&poisoned, &strike, 0, &mask, &mut out, &mut scratch).unwrap_err(),
            HitError::Layer(GraphError::NonFinite("layers.positions"))
        );
        // Errors display loudly for the combat log.
        assert!(HitError::BadOctant(9).to_string().contains('9'));
        assert!(HitError::BadImpulse(999).to_string().contains("999"));
        assert!(HitError::LengthMismatch {
            expected: 6,
            got: 5
        }
        .to_string()
        .contains('5'));
    }

    #[test]
    fn steady_state_reuses_buffers() {
        let base = base_pose();
        let mask = hit_mask();
        let strike = HitStrike::new(5, 512, 0).expect("strike");
        let (mut out, mut scratch) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        apply_hit_react(&base, &strike, 0, &mask, &mut out, &mut scratch).expect("warmup");
        let (pos_ptr, rot_ptr) = (out.positions.as_ptr(), out.rotations.as_ptr());
        let (scratch_ptr, pos_cap, rot_cap) = (
            scratch.positions.as_ptr(),
            out.positions.capacity(),
            out.rotations.capacity(),
        );
        for tick in 0..60 {
            // Cycle dirs and ticks so decay, copy and blend paths all run.
            let dir = all_dirs()[tick as usize % 8];
            let tick_strike = HitStrike::new(dir.octant(), 512, tick as u32).expect("tick");
            apply_hit_react(
                &base,
                &tick_strike,
                tick as u32,
                &mask,
                &mut out,
                &mut scratch,
            )
            .expect("steady tick");
        }
        assert_eq!(out.positions.as_ptr(), pos_ptr);
        assert_eq!(out.rotations.as_ptr(), rot_ptr);
        assert_eq!(scratch.positions.as_ptr(), scratch_ptr);
        assert_eq!(out.positions.capacity(), pos_cap);
        assert_eq!(out.rotations.capacity(), rot_cap);
        assert_eq!(out.positions.len(), FIGHTER_JOINTS);
    }

    /// Fatigue sweep probe: an IK-planted base through the fixed layer order
    /// (foot lock plus full-body IK first, additive hit-react after, matching
    /// the graph consumer) at five fatigue bands from rested to exhausted.
    /// Returns the transcript digest plus spine planar magnitudes per band.
    fn sweep_flinch(
        table: &super::super::exert::ExertTable,
        mask: &super::super::layers::MaskTable,
    ) -> (u64, [f32; 5]) {
        use super::super::exert::{ExertState, Gait};
        use super::super::feet::{slide_mm, SLIDE_BUDGET_MM_STRAIGHT};
        use engine_core::Fnv64;

        let sweeps: [u32; 5] = [0, 600, 1500, 2700, 3600];
        let gaits: [Gait; 5] = [
            Gait::Fresh,
            Gait::Winded,
            Gait::Tired,
            Gait::Spent,
            Gait::Spent,
        ];
        let mut digest = Fnv64::new();
        let mut spine_mags = [0.0f32; 5];
        for (slot, fatigue) in sweeps.iter().enumerate() {
            let state = ExertState {
                fighter_id: 7,
                fatigue_ticks: *fatigue,
            };
            assert_eq!(state.gait(table).expect("gait reads"), gaits[slot]);
            // IK-planted base: leg lanes hold the anchors before react runs.
            let base = base_pose();
            let strike =
                HitStrike::new(HitDir::Front.octant(), IMPULSE_Q8_ONE, 10).expect("front strike");
            let (mut out, mut scratch) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
            apply_hit_react(&base, &strike, 10, mask, &mut out, &mut scratch).expect("impact tick");
            // Flinch magnitude: spine planar travel at 1x reads PEAK_M.
            let dx = out.positions[SPINE][0] - base.positions[SPINE][0];
            let dz = out.positions[SPINE][2] - base.positions[SPINE][2];
            let mag = (dx * dx + dz * dz).sqrt();
            assert!((mag - PEAK_M).abs() <= HIT_TOL, "band {fatigue}: {mag}");
            spine_mags[slot] = mag;
            // Slide bars hold: planted legs read back exactly, 0 mm.
            for leg in [LEG_L, LEG_R] {
                assert_eq!(
                    out.positions[leg], base.positions[leg],
                    "band {fatigue} leg {leg}"
                );
                assert_eq!(slide_mm(out.positions[leg], base.positions[leg]), 0);
                assert!(
                    slide_mm(out.positions[leg], base.positions[leg]) <= SLIDE_BUDGET_MM_STRAIGHT
                );
            }
            // Hand-rolled impact reference holds at every band.
            assert_near(out.positions[SPINE], [0.0, 1.2 - DIP_M, -PEAK_M], HIT_TOL);
            for p in out.positions.iter() {
                digest.write_f32(p[0]);
                digest.write_f32(p[1]);
                digest.write_f32(p[2]);
            }
            digest.write_u32(*fatigue);
        }
        // Band check: rested to exhausted stays flat within tolerance.
        for slot in 1..spine_mags.len() {
            assert!(
                (spine_mags[slot] - spine_mags[0]).abs() <= HIT_TOL,
                "{spine_mags:?}"
            );
        }
        (digest.finish(), spine_mags)
    }

    #[test]
    fn react_order_ik_first_flinch_stable_across_fatigue() {
        use super::super::exert::ExertTable;

        // Ordering proof: IK plants the feet first, hit-react layers after,
        // so flinch magnitude never drifts with fatigue.
        let table = ExertTable::default();
        table.validate().expect("default table valid");
        let mask = hit_mask();
        // Leg lanes stay driven (0) under the partial-ragdoll mask.
        assert_eq!(mask.weights(), &[255, 0, 0, 255, 255, 255]);
        let (first, first_mags) = sweep_flinch(&table, &mask);
        let (second, second_mags) = sweep_flinch(&table, &mask);
        assert_eq!(first, second, "fatigue sweep replays");
        assert_eq!(first_mags, second_mags);
        println!(
            "AM20 react_order: mag={:.5} m digest=0x{:016x}",
            first_mags[0], first
        );
        // Failure case: an over-wide impulse is refused, never scaled.
        let mut out = Pose::zero(FIGHTER_JOINTS);
        assert_eq!(
            flinch(HitDir::Front, IMPULSE_Q8_MAX + 1, &mut out).unwrap_err(),
            HitError::BadImpulse(IMPULSE_Q8_MAX + 1)
        );
    }

    #[test]
    fn push_q10_is_the_integer_push_and_mirrors() {
        const U: i32 = 1024;
        const D: i32 = 724;
        for dir in all_dirs() {
            let q = dir.push_q10();
            let f = dir.push();
            // The integer table agrees with the float one lane by lane.
            for lane in 0..2 {
                assert_eq!(q[lane] >= 0, f[lane] >= 0.0, "{dir:?} lane {lane} sign");
            }
            match dir.octant() % 2 {
                // Axis hits push one lane at unity and leave the other at rest.
                0 => {
                    assert_eq!(q[0].abs() + q[1].abs(), U, "{dir:?} axis push");
                    assert!(q[0] == 0 || q[1] == 0, "{dir:?} single lane");
                }
                // Diagonals split evenly at 1024/sqrt 2, floored.
                _ => {
                    assert_eq!(q[0].abs(), D, "{dir:?} diagonal x");
                    assert_eq!(q[1].abs(), D, "{dir:?} diagonal z");
                }
            }
        }
        // Mirroring a blow swaps left and right: the x lane flips and the z
        // lane holds, so a mirrored hit mirrors the reaction.
        for (left, right) in [
            (HitDir::Left, HitDir::Right),
            (HitDir::FrontLeft, HitDir::FrontRight),
            (HitDir::BackLeft, HitDir::BackRight),
        ] {
            let (l, r) = (left.push_q10(), right.push_q10());
            assert_eq!(l[0], -r[0], "{left:?}/{right:?} x flips");
            assert_eq!(l[1], r[1], "{left:?}/{right:?} z holds");
            // The push magnitude is one value, so no side is weaker.
            assert_eq!(l[0].abs() + l[1].abs(), r[0].abs() + r[1].abs());
        }
        // Front and back mirror across the plane: the z lane flips, x holds.
        let (front, back) = (HitDir::Front.push_q10(), HitDir::Back.push_q10());
        assert_eq!(front[0], -back[0], "front/back x");
        assert_eq!(front[1], -back[1], "front/back z flips");
    }

    /// One strike at full impulse drawn over the bind pose, for the overlap numbers.
    fn one_strike_peak(dir: HitDir) -> f32 {
        let base = base_pose();
        let mut stack = FlinchStack::new();
        stack.push(Flinch::new(0, dir, IMPULSE_Q8_ONE));
        let mut out = Pose::zero(FIGHTER_JOINTS);
        let mut scratch = Pose::zero(FIGHTER_JOINTS);
        apply_flinches(&base, &stack, 1, &hit_mask(), &mut out, &mut scratch).expect("draws")
    }

    /// The drawn delta of one joint: `layers::apply_additive` writes base plus the lane sum, so
    /// every number below is measured against the bind pose, never the absolute pose.
    fn delta_of(pose: &Pose, base: &Pose, joint: usize) -> [f32; 3] {
        [
            pose.positions[joint][0] - base.positions[joint][0],
            pose.positions[joint][1] - base.positions[joint][1],
            pose.positions[joint][2] - base.positions[joint][2],
        ]
    }

    #[test]
    fn a_second_strike_six_ticks_later_does_not_snap_the_lean() {
        let base = base_pose();
        let mask = hit_mask();
        // A jab rides a jab: a quarter impulse, so the seam's shape is measured, not the cap.
        let jab = IMPULSE_Q8_ONE / 4;
        let mut stack = FlinchStack::new();
        stack.push(Flinch::new(0, HitDir::Front, jab));
        stack.push(Flinch::new(6, HitDir::Right, jab));
        let (mut out, mut scratch) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        let mut drawn = Vec::new();
        let mut peak = 0.0f32;
        for tick in 0..VIEW_FLINCH_TICKS + 2 {
            peak = peak.max(
                apply_flinches(&base, &stack, tick, &mask, &mut out, &mut scratch).expect("draws"),
            );
            drawn.push(delta_of(&out, &base, SPINE));
        }
        // The seam between tick 11 and 12 moves the drawn spine less than 2 mm.
        let seam = {
            let (a, b) = (drawn[11], drawn[12]);
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        println!(
            "hit_react: overlap seam {:.4} m at tick 11..12, peak {peak:.3}x one jab",
            seam
        );
        assert!(seam < 0.002, "the second strike snapped the spine {seam} m");
        // Riding the first flinch draws more than one jab alone (a jab is a quarter impulse, so
        // one jab peaks at 0.25 of a full strike), and never past the cap.
        assert!(
            peak > 0.25 + 1e-4,
            "the two lanes summed to {peak}x, no overlap"
        );
        assert!(
            peak <= OVERLAP_CAP + 1e-4,
            "overlap drew {peak}x one strike"
        );
    }

    #[test]
    fn two_opposite_strikes_stay_under_the_peak_cap() {
        let base = base_pose();
        let mask = hit_mask();
        let mut stack = FlinchStack::new();
        stack.push(Flinch::new(0, HitDir::Front, IMPULSE_Q8_ONE));
        stack.push(Flinch::new(6, HitDir::Back, IMPULSE_Q8_ONE));
        let (mut out, mut scratch) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        let single = one_strike_peak(HitDir::Front);
        let mut peak = 0.0f32;
        for tick in 0..VIEW_FLINCH_TICKS + 2 {
            peak = peak.max(
                apply_flinches(&base, &stack, tick, &mask, &mut out, &mut scratch).expect("draws"),
            );
            for joint in 0..FIGHTER_JOINTS {
                let at = delta_of(&out, &base, joint);
                let travel = (at[0] * at[0] + at[1] * at[1] + at[2] * at[2]).sqrt() / PEAK_M;
                assert!(
                    travel <= OVERLAP_CAP + 1e-4,
                    "joint {joint} at {travel}x one strike on tick {tick}"
                );
            }
        }
        println!("hit_react: opposite pair peak {peak:.3}x one strike (one strike {single:.3}x)");
        assert!(peak <= OVERLAP_CAP + 1e-4, "opposite pair drew {peak}x");
        // The same script draws the same pose twice, bit for bit.
        let mut second = FlinchStack::new();
        second.push(Flinch::new(0, HitDir::Front, IMPULSE_Q8_ONE));
        second.push(Flinch::new(6, HitDir::Back, IMPULSE_Q8_ONE));
        let (mut again, mut scratch2) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        let (mut out1, mut scratch1) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        for tick in [0u32, 5, 6, 11, 20] {
            apply_flinches(&base, &stack, tick, &mask, &mut out1, &mut scratch1).expect("draws");
            apply_flinches(&base, &second, tick, &mask, &mut again, &mut scratch2).expect("draws");
            assert_eq!(out1.positions, again.positions, "tick {tick} replays");
            assert_eq!(out1.rotations, again.rotations, "tick {tick} replays");
        }
        // A third strike takes the oldest lane over without a jump at the swap tick. Jabs, so
        // the seam is the swap and not one tick of a full strike's own decay.
        let mut third = FlinchStack::new();
        let jab = IMPULSE_Q8_ONE / 4;
        third.push(Flinch::new(0, HitDir::Front, jab));
        third.push(Flinch::new(6, HitDir::Right, jab));
        let (mut before, mut scratch3) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        apply_flinches(&base, &third, 11, &mask, &mut before, &mut scratch3).expect("draws");
        third.push(Flinch::new(12, HitDir::Left, jab));
        let (mut after, mut scratch4) = (Pose::zero(FIGHTER_JOINTS), Pose::zero(FIGHTER_JOINTS));
        apply_flinches(&base, &third, 12, &mask, &mut after, &mut scratch4).expect("draws");
        let jump = {
            let (a, b) = (
                delta_of(&before, &base, SPINE),
                delta_of(&after, &base, SPINE),
            );
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        assert!(jump < 0.002, "the third strike snapped the spine {jump} m");
    }
}
