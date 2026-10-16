//! warp_ground: GASP-class ground feel over the locomotion root.
//!
//! Owner: `engine-anim`; consumers: arena presentation post-processes each
//! [`LocoOutput`] through [`GroundWarp::warp_output`] before the viewer
//! draws the pose; foot lock meters the result. [`DistanceWarp`] plans and
//! meters straight starts/stops. Units: meters, seconds, radians, ticks at
//! 60 Hz; slide in integer millimeters. Determinism: `+ - * /` plus `sqrt`
//! only, no transcendentals, no wall clock, stable joint order; one input
//! script replays bit-identically. Errors are [`WarpError`], never panics,
//! and a rejected [`GroundWarp::warp_output`] mutates neither the output
//! nor the warp.
//!
//! Authority split: the root is authoritative and passes through
//! bit-identical; every warp rides the presentation side.
//!
//! * distance matching: the clip cursor advances at capsule speed, so the
//!   clip covers the capsule travel over starts, stops and pivots;
//! * orientation warping: the presentation facing turns toward the wish in
//!   small-angle steps, capped at 135 degrees total, so strafes read;
//! * stride warping: leg swing scales with the residual the cursor rate
//!   clamp cannot cover, so stance feet move at ground speed.
//!
//! Foot speed law: `clip * rate * stride == capsule` while both clamps
//! hold (rate 16x range, stride 4x residual); past that envelope the pair is
//! unmatched and the clamps hold the pose readable instead of teleporting.
//! Budget: O(1) per tick plus O(joints) pose writes; caller-owned buffers,
//! no heap per tick.

use super::clip::TICK_DT;
use super::contract::LocalPose;
use super::locomotion::{
    normalize2, LocoOutput, LocoState, MAX_WISH_SPEED, START_TICKS, STOP_TICKS, TURN_PIVOT,
    TURN_START,
};
use engine_core::{quat_mul, Fnv64};

/// Slowest capsule speed (m/s) the accept sweep covers.
pub const MIN_WARP_SPEED_M_S: f32 = 0.5;
/// Fastest capsule speed (m/s) the accept sweep covers.
pub const MAX_WARP_SPEED_M_S: f32 = 7.0;
/// Foot slide budget (mm) on straights: warped plants stay within 1 cm.
pub const SLIDE_BUDGET_MM: u32 = 10;
/// Cursor rate floor: the clip never runs slower than a quarter speed.
pub const RATE_MIN: f32 = 0.25;
/// Cursor rate ceiling: the clip never runs faster than quadruple speed.
pub const RATE_MAX: f32 = 4.0;
/// Pivot windows warp through [`GroundWarp`] with the fast pivot turn cap.
pub const PIVOT_SUPPORTED: bool = true;
/// Strafes warp through [`GroundWarp`] up to [`STRAFE_WARP_MAX_RAD`].
pub const STRAFE_SUPPORTED: bool = true;
/// Stride warps through [`GroundWarp`] between [`STRIDE_MIN`] and [`STRIDE_MAX`].
pub const STRIDE_SUPPORTED: bool = true;
/// Total orientation warp cap (rad): 135 degrees, the strafe readability limit.
pub const STRAFE_WARP_MAX_RAD: f32 = 2.356_194_5;
/// Per-tick orientation step (rad) inside pivot windows: the locomotion pivot turn.
pub const ORIENT_STEP_PIVOT_RAD: f32 = TURN_PIVOT;
/// Per-tick orientation step (rad) outside pivot windows.
pub const ORIENT_STEP_CRUISE_RAD: f32 = TURN_START;
/// Stride scale floor: the visual swing never halves below this.
pub const STRIDE_MIN: f32 = 0.5;
/// Stride scale ceiling: the visual swing never doubles past this.
pub const STRIDE_MAX: f32 = 2.0;
/// Exact-reversal tolerance on the facing/wish cross product.
const REVERSAL_EPS: f32 = 1e-6;

/// Ground-feel failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum WarpError {
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Target speed was negative; value carried.
    NegativeSpeed(f32),
    /// Target speed exceeded [`MAX_WISH_SPEED`]; value carried.
    SpeedTooHigh(f32),
    /// A per-tick step is out of bounds; the lane names the problem.
    BadStep(&'static str),
    /// Transient window was zero ticks; value carried.
    BadWindow(u32),
    /// Wish direction was zero-length (non-finite lanes report [`WarpError::NonFinite`]).
    BadWishDir,
    /// Pose holds no joints.
    EmptyPose,
    /// Pose position/rotation lanes disagree; expected plus got.
    LengthMismatch {
        /// Position lane length.
        expected: usize,
        /// Rotation lane length.
        got: usize,
    },
    /// Leg joint index out of range for the pose; value carried.
    BadJoint(usize),
    /// Rate was not positive finite; value carried.
    BadScale(f32),
}

impl core::fmt::Display for WarpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WarpError::NonFinite(lane) => write!(f, "warp: non-finite float in {lane}"),
            WarpError::NegativeSpeed(v) => write!(f, "warp: negative speed {v}"),
            WarpError::SpeedTooHigh(v) => write!(f, "warp: speed {v} past the wish limit"),
            WarpError::BadStep(lane) => write!(f, "warp: bad step ({lane})"),
            WarpError::BadWindow(dur) => write!(f, "warp: transient window is {dur} ticks"),
            WarpError::BadWishDir => write!(f, "warp: wish direction has zero length"),
            WarpError::EmptyPose => write!(f, "warp: pose holds no joints"),
            WarpError::LengthMismatch { expected, got } => {
                write!(
                    f,
                    "warp: pose lanes disagree ({got} rotations for {expected} positions)"
                )
            }
            WarpError::BadJoint(joint) => write!(f, "warp: leg joint {joint} out of range"),
            WarpError::BadScale(s) => write!(f, "warp: scale {s} is not positive finite"),
        }
    }
}

impl std::error::Error for WarpError {}

/// Straight transients the distance match covers this wave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WarpKind {
    /// Idle-to-move acceleration ramp over [`START_TICKS`] ticks.
    Start,
    /// Move-to-idle deceleration ramp over [`STOP_TICKS`] ticks.
    Stop,
}

/// Validated distance-match plan: transient kind, target speed in m/s,
/// tick window, and expected capsule travel in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WarpPlan {
    /// Transient kind (start or stop).
    pub kind: WarpKind,
    /// Target cruise speed in meters per second.
    pub target_speed_m_s: f32,
    /// Ticks in the ramp window.
    pub ticks: u32,
    /// Expected capsule travel in meters over the window.
    pub capsule_distance_m: f32,
}

/// Plan a straight start/stop for `target` m/s.
///
/// The capsule ramps linearly (start: `target*(t+1)/N`, stop:
/// `target*(N-t)/N`), so the window covers `target*dt*(N+1)/2` meters.
/// Rejects non-finite, negative and over-limit speeds loudly.
pub fn plan(kind: WarpKind, target: f32) -> Result<WarpPlan, WarpError> {
    if !target.is_finite() {
        return Err(WarpError::NonFinite("target"));
    }
    if target < 0.0 {
        return Err(WarpError::NegativeSpeed(target));
    }
    if target > MAX_WISH_SPEED {
        return Err(WarpError::SpeedTooHigh(target));
    }
    let ticks = match kind {
        WarpKind::Start => START_TICKS,
        WarpKind::Stop => STOP_TICKS,
    };
    if ticks == 0 {
        return Err(WarpError::BadWindow(ticks));
    }
    let n = f32::from(u16::try_from(ticks).unwrap_or(u16::MAX));
    let capsule_distance_m = target * TICK_DT * (n + 1.0) / 2.0;
    Ok(WarpPlan {
        kind,
        target_speed_m_s: target,
        ticks,
        capsule_distance_m,
    })
}

/// Distance-match rate for one tick: `capsule_step / anim_step`,
/// clamped to [`RATE_MIN`]..=[`RATE_MAX`] so the cursor slows or hurries
/// instead of teleporting. Both steps are meters along travel.
pub fn match_rate(capsule_step_m: f32, anim_step_m: f32) -> Result<f32, WarpError> {
    if !capsule_step_m.is_finite() {
        return Err(WarpError::NonFinite("capsule_step"));
    }
    if !anim_step_m.is_finite() {
        return Err(WarpError::NonFinite("anim_step"));
    }
    if capsule_step_m < 0.0 {
        return Err(WarpError::BadStep("capsule_step"));
    }
    if anim_step_m <= 0.0 {
        return Err(WarpError::BadStep("anim_step"));
    }
    let rate = capsule_step_m / anim_step_m;
    if !rate.is_finite() {
        return Err(WarpError::NonFinite("rate"));
    }
    // Both inputs are finite here and the bounds are finite consts, so
    // `clamp` cannot hit its NaN/panic edges; it only holds the pose
    // readable instead of teleporting past the envelope.
    Ok(rate.clamp(RATE_MIN, RATE_MAX))
}

/// One warped tick: the applied rate plus the warped anim advance in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TickOut {
    /// Clamped cursor rate applied this tick.
    pub rate: f32,
    /// Warped anim advance in meters (`anim_step * rate`).
    pub warped_step_m: f32,
}

/// Per-transient distance warp: accumulates capsule vs warped travel so
/// the residual (hence foot slide) stays visible. Caller-owned, no heap.
#[derive(Clone, PartialEq, Debug)]
pub struct DistanceWarp {
    plan: WarpPlan,
    tick_idx: u32,
    capsule_covered_m: f32,
    warped_covered_m: f32,
}

impl DistanceWarp {
    /// New warp over a validated `plan`; counters start at zero.
    pub fn new(plan: WarpPlan) -> Self {
        Self {
            plan,
            tick_idx: 0,
            capsule_covered_m: 0.0,
            warped_covered_m: 0.0,
        }
    }

    /// The plan this warp runs.
    pub fn plan(&self) -> WarpPlan {
        self.plan
    }

    /// Ticks consumed so far.
    pub fn tick_idx(&self) -> u32 {
        self.tick_idx
    }

    /// Capsule travel covered so far in meters.
    pub fn capsule_covered_m(&self) -> f32 {
        self.capsule_covered_m
    }

    /// Warped anim travel covered so far in meters.
    pub fn warped_covered_m(&self) -> f32 {
        self.warped_covered_m
    }

    /// One tick: warp `anim_step_m` onto `capsule_step_m` (both meters).
    /// Rejected input leaves the warp untouched.
    pub fn tick(&mut self, capsule_step_m: f32, anim_step_m: f32) -> Result<TickOut, WarpError> {
        let rate = match_rate(capsule_step_m, anim_step_m)?;
        let warped_step_m = anim_step_m * rate;
        if !warped_step_m.is_finite() {
            return Err(WarpError::NonFinite("warped_step"));
        }
        self.capsule_covered_m += capsule_step_m;
        self.warped_covered_m += warped_step_m;
        self.tick_idx += 1;
        Ok(TickOut {
            rate,
            warped_step_m,
        })
    }

    /// Residual in integer millimeters: `round(|capsule - warped| * 1000)`.
    /// Residual in integer millimeters over the AM6 slide meter: the lock
    /// distance between capsule and warped travel, so warped plants read
    /// through the same meter the locker enforces.
    /// Non-finite cover saturates to `u32::MAX`, never a planted read.
    pub fn residual_mm(&self) -> u32 {
        super::feet::slide_mm(
            [self.capsule_covered_m, 0.0, 0.0],
            [self.warped_covered_m, 0.0, 0.0],
        )
    }
}

/// FNV-1a64 over warped steps (count plus `f32` bits); equal scripts hash
/// equally, the replay receipt for warped sequences.
pub fn warp_hash(steps: &[f32]) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(u64::try_from(steps.len()).unwrap_or(u64::MAX));
    for s in steps.iter() {
        h.write_f32(*s);
    }
    h.finish()
}

/// Cursor rate from per-second speeds: [`match_rate`] over capsule and clip
/// speed, holding exactly `1.0` while either side is stationary (a stationary
/// capsule pairs a stop clip through a fresh [`GroundWarp::begin_transient`],
/// never a rate). Rejects negative, non-finite and over-limit speeds loudly.
pub(crate) fn speed_rate(clip_speed: f32, capsule_speed: f32) -> Result<f32, WarpError> {
    check_speed(clip_speed, "clip")?;
    check_speed(capsule_speed, "capsule")?;
    if clip_speed <= 0.0 || capsule_speed <= 0.0 {
        return Ok(1.0);
    }
    match_rate(capsule_speed, clip_speed)
}

/// Stride scale for the residual the rate clamp cannot cover:
/// `capsule / (clip * rate)` clamped to [`STRIDE_MIN`]..=[`STRIDE_MAX`]. With
/// an unclamped rate this is exactly `1.0`; it only bends while the rate
/// saturates. Rejects bad speeds and non-positive rates loudly.
pub(crate) fn stride_for(clip_speed: f32, capsule_speed: f32, rate: f32) -> Result<f32, WarpError> {
    check_speed(clip_speed, "clip")?;
    check_speed(capsule_speed, "capsule")?;
    if !rate.is_finite() || rate <= 0.0 {
        return Err(WarpError::BadScale(rate));
    }
    if clip_speed <= 0.0 || capsule_speed <= 0.0 {
        return Ok(1.0);
    }
    Ok((capsule_speed / (clip_speed * rate)).clamp(STRIDE_MIN, STRIDE_MAX))
}

/// One warped tick: the rate/stride/yaw the consumer applied plus the clip
/// advance and the tick stamp.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WarpReceipt {
    /// Fixed tick this receipt covers.
    pub tick: u32,
    /// Clip cursor rate applied this tick.
    pub rate: f32,
    /// Stride scale applied to the leg swing this tick.
    pub stride: f32,
    /// Yaw (rad) the presentation facing warped this tick.
    pub dyaw: f32,
    /// Clip advance (m) this tick: `rate * target * TICK_DT`.
    pub clip_step: f32,
}

/// GASP ground-feel warp over one fighter. Construct with [`GroundWarp::new`],
/// open each transient with [`GroundWarp::begin_transient`], then
/// post-process every locomotion output with [`GroundWarp::warp_output`]. The
/// root passes through bit-identical; the warp advances the clip cursor at
/// capsule speed and shapes the pose. Without an open clip target the rate and
/// stride hold `1.0` and only the orientation warp acts. `Clone` snapshots the
/// full warp state for rollback or repeat checks.
#[derive(Clone, PartialEq, Debug)]
pub struct GroundWarp {
    target_speed: f32,
    rate: f32,
    stride: f32,
    facing: [f32; 2],
    warp_yaw: f32,
    orient_cap: f32,
    trans_left: u32,
    trans_dur: u32,
    capsule_m: f32,
    clip_m: f32,
    clip_step_m: f32,
}

impl GroundWarp {
    /// New warp: no clip target (rate and stride `1.0`), facing `+Z`, cruise
    /// turn cap, no open transient.
    pub fn new() -> Self {
        Self {
            target_speed: 0.0,
            rate: 1.0,
            stride: 1.0,
            facing: [0.0, 1.0],
            warp_yaw: 0.0,
            orient_cap: ORIENT_STEP_CRUISE_RAD,
            trans_left: 0,
            trans_dur: 0,
            capsule_m: 0.0,
            clip_m: 0.0,
            clip_step_m: 0.0,
        }
    }

    /// Current cursor rate.
    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Current stride scale.
    pub fn stride(&self) -> f32 {
        self.stride
    }

    /// Presentation facing as a unit `[x, z]` vector.
    pub fn facing(&self) -> [f32; 2] {
        self.facing
    }

    /// Total yaw (rad) warped away from the synced facing, within
    /// `+-`[`STRAFE_WARP_MAX_RAD`].
    pub fn warp_yaw(&self) -> f32 {
        self.warp_yaw
    }

    /// Matched clip speed (m/s) of the open window.
    pub fn target_speed(&self) -> f32 {
        self.target_speed
    }

    /// True once the open transient window has fully ticked down.
    pub fn settled(&self) -> bool {
        self.trans_left == 0
    }

    /// Ticks left in the open transient window.
    pub fn remaining_ticks(&self) -> u32 {
        self.trans_left
    }

    /// Total ticks of the open transient window.
    pub fn window_ticks(&self) -> u32 {
        self.trans_dur
    }

    /// Capsule travel (m) accumulated while the window was open.
    pub fn capsule_travel_m(&self) -> f32 {
        self.capsule_m
    }

    /// Clip cursor travel (m) accumulated while the window was open: equals
    /// the capsule travel while the rate holds, which is the distance match.
    pub fn clip_travel_m(&self) -> f32 {
        self.clip_m
    }

    /// Clip advance (m) applied on the latest tick.
    pub fn clip_step_m(&self) -> f32 {
        self.clip_step_m
    }

    /// Sync the presentation facing to the capsule facing (unit `[x, z]`);
    /// resets the accumulated warp yaw. Rejects non-finite or zero vectors
    /// without touching state.
    pub fn sync_facing(&mut self, facing: [f32; 2]) -> Result<(), WarpError> {
        self.facing = unit_wish(facing)?;
        self.warp_yaw = 0.0;
        Ok(())
    }

    /// Open a transient window of `dur_ticks` ticks covering a clip that
    /// travels at `target_speed` m/s while the capsule moves at
    /// `capsule_speed` m/s. [`LocoState::Pivot`] windows turn at the pivot cap,
    /// every other state at the cruise cap. Zero windows and bad speeds refuse
    /// without touching state.
    pub fn begin_transient(
        &mut self,
        state: LocoState,
        dur_ticks: u32,
        target_speed: f32,
        capsule_speed: f32,
    ) -> Result<(), WarpError> {
        if dur_ticks == 0 {
            return Err(WarpError::BadWindow(0));
        }
        let rate = speed_rate(target_speed, capsule_speed)?;
        let stride = stride_for(target_speed, capsule_speed, rate)?;
        self.target_speed = target_speed;
        self.rate = rate;
        self.stride = stride;
        self.trans_left = dur_ticks;
        self.trans_dur = dur_ticks;
        self.capsule_m = 0.0;
        self.clip_m = 0.0;
        self.clip_step_m = 0.0;
        self.orient_cap = if state == LocoState::Pivot {
            ORIENT_STEP_PIVOT_RAD
        } else {
            ORIENT_STEP_CRUISE_RAD
        };
        Ok(())
    }

    /// Recompute the live rate and stride from the current capsule speed
    /// (m/s) against the window target. Steady ticks call this every frame so
    /// the match tracks acceleration; a stationary capsule holds `1.0`.
    pub fn retarget(&mut self, capsule_speed: f32) -> Result<(), WarpError> {
        let rate = speed_rate(self.target_speed, capsule_speed)?;
        self.stride = stride_for(self.target_speed, capsule_speed, rate)?;
        self.rate = rate;
        Ok(())
    }

    /// One warp tick over a capsule planar delta (m): counts the transient
    /// window down, folds the capsule and clip travel into the window
    /// accounts while it is open, and returns the clip advance (m) the
    /// consumer feeds its clip cursor. The root delta itself is authoritative
    /// and passes through untouched. No allocation.
    pub fn tick(&mut self, dp: [f32; 2]) -> Result<f32, WarpError> {
        check_finite2(dp, "root delta")?;
        let clip_step_m = self.rate * self.target_speed * TICK_DT;
        if self.trans_left > 0 {
            self.capsule_m += (dp[0] * dp[0] + dp[1] * dp[1]).sqrt();
            self.clip_m += clip_step_m;
            self.trans_left -= 1;
        }
        self.clip_step_m = clip_step_m;
        Ok(clip_step_m)
    }

    /// Turn the presentation facing toward the wish direction by one capped
    /// step; returns the yaw applied. An exact reversal breaks toward `+yaw`
    /// (the locomotion pivot convention); an aligned wish takes no step. The
    /// accumulated warp never exceeds [`STRAFE_WARP_MAX_RAD`] either way.
    /// No allocation.
    pub fn orient_toward(&mut self, wish_dir: [f32; 2]) -> Result<f32, WarpError> {
        let wish = unit_wish(wish_dir)?;
        let f = self.facing;
        let cross = f[0] * wish[1] - f[1] * wish[0];
        let dot = f[0] * wish[0] + f[1] * wish[1];
        let mut step = if cross.abs() < REVERSAL_EPS {
            if dot < 0.0 {
                self.orient_cap
            } else {
                0.0
            }
        } else {
            (-cross).clamp(-self.orient_cap, self.orient_cap)
        };
        step = step.clamp(
            -STRAFE_WARP_MAX_RAD - self.warp_yaw,
            STRAFE_WARP_MAX_RAD - self.warp_yaw,
        );
        let cos = 1.0 - step * step * 0.5;
        self.facing = normalize2([f[0] * cos + f[1] * step, -f[0] * step + f[1] * cos]);
        self.warp_yaw += step;
        Ok(step)
    }

    /// Post-process one locomotion output in place: retargets the rate and
    /// stride to `capsule_speed`, advances the window accounts, warps the
    /// facing toward `wish_dir`, scales the `legs` swing and yaws the pose by
    /// this tick's warp step. The root passes through bit-identical.
    /// Atomic: any rejection leaves both the output and the warp untouched.
    /// Writes in place only, never allocates.
    pub fn warp_output(
        &mut self,
        out: &mut LocoOutput,
        capsule_speed: f32,
        wish_dir: [f32; 2],
        legs: &[usize],
    ) -> Result<WarpReceipt, WarpError> {
        check_speed(capsule_speed, "capsule")?;
        unit_wish(wish_dir)?;
        check_finite2(out.root.dp, "root delta")?;
        let joints = check_pose(&out.pose)?;
        if let Some(bad) = legs.iter().find(|joint| **joint >= joints) {
            return Err(WarpError::BadJoint(*bad));
        }
        self.retarget(capsule_speed)?;
        let dyaw = self.orient_toward(wish_dir)?;
        let clip_step = self.tick(out.root.dp)?;
        for joint in legs.iter() {
            out.pose.positions[*joint][2] *= self.stride;
        }
        yaw_pose(&mut out.pose, dyaw);
        Ok(WarpReceipt {
            tick: out.root.tick,
            rate: self.rate,
            stride: self.stride,
            dyaw,
            clip_step,
        })
    }
}

impl Default for GroundWarp {
    fn default() -> Self {
        Self::new()
    }
}

/// Yaw the whole pose by a small step (rad): positions rotate in the ground
/// plane by the `(sin, cos) = (d, 1 - d*d/2)` pair, rotations premultiply the
/// matching half-angle quaternion. Same sign as the locomotion turn (`+Z`
/// toward `+X` is positive). Steps past ~0.2 rad belong in smaller pieces.
fn yaw_pose(pose: &mut LocalPose, dyaw: f32) {
    if dyaw.abs() < 1e-9 {
        return;
    }
    let (sin, cos) = (dyaw, 1.0 - dyaw * dyaw * 0.5);
    for p in pose.positions.iter_mut() {
        let (x, z) = (p[0], p[2]);
        p[0] = x * cos + z * sin;
        p[2] = -x * sin + z * cos;
    }
    let half = dyaw * 0.5;
    let inv = 1.0 / (half * half + 1.0).sqrt();
    let yaw_q = [0.0, half * inv, 0.0, inv];
    for q in pose.rotations.iter_mut() {
        *q = quat_mul(yaw_q, *q);
    }
}

fn check_speed(value: f32, lane: &'static str) -> Result<(), WarpError> {
    if !value.is_finite() {
        return Err(WarpError::NonFinite(lane));
    }
    if value < 0.0 {
        return Err(WarpError::NegativeSpeed(value));
    }
    if value > MAX_WISH_SPEED {
        return Err(WarpError::SpeedTooHigh(value));
    }
    Ok(())
}

fn check_finite2(v: [f32; 2], lane: &'static str) -> Result<(), WarpError> {
    if v[0].is_finite() && v[1].is_finite() {
        Ok(())
    } else {
        Err(WarpError::NonFinite(lane))
    }
}

/// Unit wish from a finite, non-zero direction.
fn unit_wish(dir: [f32; 2]) -> Result<[f32; 2], WarpError> {
    check_finite2(dir, "wish dir")?;
    let len2 = dir[0] * dir[0] + dir[1] * dir[1];
    if len2 <= 0.0 {
        return Err(WarpError::BadWishDir);
    }
    let inv = 1.0 / len2.sqrt();
    Ok([dir[0] * inv, dir[1] * inv])
}

/// Joint count of a pose whose lanes agree and hold only finite floats.
fn check_pose(pose: &LocalPose) -> Result<usize, WarpError> {
    let joints = pose.positions.len();
    if joints == 0 {
        return Err(WarpError::EmptyPose);
    }
    if pose.rotations.len() != joints {
        return Err(WarpError::LengthMismatch {
            expected: joints,
            got: pose.rotations.len(),
        });
    }
    if pose.positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(WarpError::NonFinite("pose positions"));
    }
    if pose.rotations.iter().flatten().any(|v| !v.is_finite()) {
        return Err(WarpError::NonFinite("pose rotations"));
    }
    Ok(joints)
}

#[cfg(test)]
mod tester_atomic {
    //! Tester vectors: rejected warps leave state untouched.
    //!
    //! Caller contract in the header promises atomic rejects
    //! (`sync_facing`, `begin_transient` and `warp_output` touch nothing
    //! on error). The sibling sweeps pin distances and slides; these pin
    //! the untouched state a caller reads back through the getters.

    use super::{GroundWarp, WarpError, ORIENT_STEP_CRUISE_RAD};
    use crate::locomotion::LocoState;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-5
    }

    #[test]
    fn sync_rejection_leaves_state_untouched_and_reset_clears_yaw() {
        let mut warp = GroundWarp::new();
        warp.sync_facing([0.0, 1.0]).expect("sync holds");
        warp.orient_toward([1.0, 0.0])
            .expect("strafe step builds yaw");
        assert!(warp.warp_yaw() > 0.0, "yaw built: {}", warp.warp_yaw());
        let (facing, yaw, rate, stride) =
            (warp.facing(), warp.warp_yaw(), warp.rate(), warp.stride());
        assert_eq!(
            warp.sync_facing([0.0, 0.0]),
            Err(WarpError::BadWishDir),
            "zero wish refuses"
        );
        assert_eq!(
            warp.sync_facing([f32::NAN, 0.0]),
            Err(WarpError::NonFinite("wish dir")),
            "non-finite wish refuses"
        );
        assert_eq!(warp.facing(), facing, "failed sync keeps facing");
        assert_eq!(warp.warp_yaw(), yaw, "failed sync keeps yaw");
        assert_eq!(warp.rate(), rate, "failed sync keeps rate");
        assert_eq!(warp.stride(), stride, "failed sync keeps stride");
        warp.sync_facing([1.0, 0.0]).expect("good sync resets");
        assert_eq!(warp.warp_yaw(), 0.0, "good sync clears yaw");
        assert!(
            approx(warp.facing()[0], 1.0) && approx(warp.facing()[1], 0.0),
            "good sync sets facing: {:?}",
            warp.facing()
        );
    }

    #[test]
    fn begin_rejection_leaves_window_untouched() {
        let mut warp = GroundWarp::new();
        warp.sync_facing([0.0, 1.0]).expect("sync holds");
        warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
            .expect("window opens");
        let (left, dur, rate, stride, target) = (
            warp.remaining_ticks(),
            warp.window_ticks(),
            warp.rate(),
            warp.stride(),
            warp.target_speed(),
        );
        assert_eq!(
            warp.begin_transient(LocoState::Start, 0, 1.5, 1.5),
            Err(WarpError::BadWindow(0)),
            "zero window refuses"
        );
        assert_eq!(
            warp.begin_transient(LocoState::Start, 10, -1.0, 1.5),
            Err(WarpError::NegativeSpeed(-1.0)),
            "negative target refuses"
        );
        assert_eq!(
            warp.begin_transient(LocoState::Start, 10, f32::NAN, 1.5),
            Err(WarpError::NonFinite("clip")),
            "non-finite target refuses"
        );
        assert_eq!(
            warp.begin_transient(LocoState::Start, 10, 99.0, 1.5),
            Err(WarpError::SpeedTooHigh(99.0)),
            "over-limit target refuses"
        );
        assert_eq!(
            warp.remaining_ticks(),
            left,
            "failed begin keeps ticks left"
        );
        assert_eq!(warp.window_ticks(), dur, "failed begin keeps window");
        assert_eq!(warp.rate(), rate, "failed begin keeps rate");
        assert_eq!(warp.stride(), stride, "failed begin keeps stride");
        assert_eq!(warp.target_speed(), target, "failed begin keeps target");
    }

    #[test]
    fn exact_reversal_steps_toward_positive_yaw() {
        let mut warp = GroundWarp::new();
        warp.sync_facing([0.0, 1.0]).expect("sync holds");
        let step = warp.orient_toward([0.0, -1.0]).expect("reversal steps");
        assert!(
            approx(step, ORIENT_STEP_CRUISE_RAD),
            "reversal uses the cruise cap: {step}"
        );
        assert!(step > 0.0, "reversal breaks toward +yaw: {step}");
        assert_eq!(warp.warp_yaw(), step, "yaw follows the step");
    }
}
