//! camera_rig: staged presentation rig over the frozen camera director.
//!
//! Purpose: named ordered follow -> lock -> shake stages beside the frozen
//! [`Director`](super::Director), plus the smoothed look intent
//! ([`LookIntent`]/[`DampedSync`]) and the trauma-curve shake helpers.
//! Split from the [`camera`](super) module; the names stay re-exported
//! there so existing `super::camera::{...}` uses keep compiling.
//! Tick math stays integer permille; all `f32` pose/shake/smoothing math is
//! presentation-only and never feeds simulation state. Owner: `engine-game`.
//! Units: meters; trauma in permille (`0..=1000`). Determinism: same inputs
//! replay byte-exact twice; shake never feeds simulation state. Errors:
//! [`CameraError`](super::CameraError) at every boundary; bad input fails
//! loudly, never wraps. Budget: zero heap bytes per call. Consumer:
//! `camera_feed` (`PresentationFeed`, `SmoothedFeed`) under the window loop.

use super::{
    fit_pair, frame_pair, select_lock_target, shake_offset, CameraError, CameraFrame, CameraPose,
    Director, DirectorParams, FighterView, Vec3, TRAUMA_SCALE,
};
use crate::camera_arm::{arm_length, Aabb, FLOOR_LEN};

// ---------------------------------------------------------------------------
// Staged rig, intent/smoothed damping and trauma-curve shake:
// named, ordered, bypassable presentation stages beside the frozen
// director. Tick math stays integer permille; all `f32` pose/shake/smoothing
// math is presentation-only and never feeds simulation state. Zero heap
// bytes per update; same inputs replay byte-exact twice. Consumer:
// `camera_feed` (`PresentationFeed`, `SmoothedFeed`) under the window loop.
// ---------------------------------------------------------------------------

/// GP19 rig stage: one named pure map in the follow -> lock -> shake chain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RigStage {
    /// Frame focus plus anchor (pose without lock choice or shake).
    Follow,
    /// Nearest-in-frustum lock selection from the unshaken pose.
    Lock,
    /// Trauma-squared shake offset on the presented eye only.
    Shake,
}

impl RigStage {
    /// Short stable name for receipts and reorder logs.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Lock => "lock",
            Self::Shake => "shake",
        }
    }
}

/// Ordered rig chain: follow feeds lock, lock feeds shake.
pub const RIG_STAGES: [RigStage; 3] = [RigStage::Follow, RigStage::Lock, RigStage::Shake];
/// Bypass bit that drops lock reporting (framing keeps the anchor).
pub const BYPASS_LOCK: u8 = 0x01;
/// Bypass bit that drops shake (the eye reports the unshaken pose).
pub const BYPASS_SHAKE: u8 = 0x02;

/// Follow stage as a pure map: frame `focus_m` plus `anchor_m`.
pub fn follow_stage(
    focus_m: Vec3,
    anchor_m: Option<Vec3>,
    params: &DirectorParams,
) -> Result<CameraPose, CameraError> {
    frame_pair(focus_m, anchor_m, params)
}

/// Lock stage as a pure map: nearest-in-frustum target from the pose.
pub fn lock_stage(
    eye_m: Vec3,
    forward: Vec3,
    params: &DirectorParams,
    focus_id: u8,
    fighters: &[FighterView],
) -> Result<Option<u8>, CameraError> {
    select_lock_target(
        eye_m,
        forward,
        params.fov_y_rad * 0.5,
        params.max_dist_m,
        focus_id,
        fighters,
    )
}

/// Shake stage as a pure map: trauma-squared offset for (`trauma`, `tick`).
#[must_use]
pub fn shake_stage(trauma_permille: u32, tick: u32, max_m: f32) -> Vec3 {
    shake_offset(trauma_permille, tick, max_m)
}

/// GP19 staged rig: the frozen [`Director`] driven as named ordered stages.
/// With no bypass the frames are byte-identical to the director; each bypass
/// bit drops one reporting stage while framing still holds, so a stage
/// reorder or removal needs only builder order, never new math.
#[derive(Clone, Debug)]
pub struct StagedRig {
    director: Director,
    bypass: u8,
}

impl StagedRig {
    /// Build a rig following `focus_id` under `params`, no stage bypassed.
    pub fn new(focus_id: u8, params: DirectorParams) -> Result<Self, CameraError> {
        Ok(Self {
            director: Director::new(focus_id, params)?,
            bypass: 0,
        })
    }

    /// Set the bypass mask (`BYPASS_LOCK | BYPASS_SHAKE`); unknown bits are
    /// kept but ignored, so newer stages never break older callers.
    pub fn set_bypass(&mut self, mask: u8) {
        self.bypass = mask;
    }

    /// Current bypass mask.
    #[must_use]
    pub fn bypass(&self) -> u8 {
        self.bypass
    }

    /// Ordered stage names served this update (follow, lock, shake).
    #[must_use]
    pub fn stage_names(&self) -> [&'static str; 3] {
        [
            RigStage::Follow.name(),
            RigStage::Lock.name(),
            RigStage::Shake.name(),
        ]
    }

    /// Followed fighter id.
    #[must_use]
    pub fn focus_id(&self) -> u8 {
        self.director.focus_id()
    }

    /// Active tuning table (copy).
    #[must_use]
    pub fn params(&self) -> DirectorParams {
        self.director.params()
    }

    /// Swap tuning at a tick boundary; validates first like the director.
    pub fn retune(&mut self, params: DirectorParams) -> Result<(), CameraError> {
        self.director.retune(params)
    }

    /// Current lock-on target (`None` while the lock stage is bypassed).
    #[must_use]
    pub fn lock(&self) -> Option<u8> {
        if self.bypass & BYPASS_LOCK != 0 {
            None
        } else {
            self.director.lock()
        }
    }

    /// Current trauma permille after decay.
    #[must_use]
    pub fn trauma_permille(&self) -> u32 {
        self.director.trauma_permille()
    }

    /// Record a hit: saturating add, clamped to `0..=1000`.
    pub fn add_trauma(&mut self, amount_permille: u32) {
        self.director.add_trauma(amount_permille);
    }

    /// Present one tick through the ordered stages: the director serves the
    /// follow-plus-lock core (frozen API), then the named shake stage
    /// re-derives the offset for audit (bit-exact: same pure inputs).
    /// Bypassed stages report neutral values while the unshaken pose still
    /// frames, so dropping lock or shake loses no framed tick.
    pub fn update(
        &mut self,
        fighters: &[FighterView],
        tick: u32,
    ) -> Result<CameraFrame, CameraError> {
        let mut frame = self.director.update(fighters, tick)?;
        let audited = shake_stage(
            frame.trauma_permille,
            tick,
            self.director.params().max_shake_m,
        );
        debug_assert_eq!(audited, frame.shake_m, "shake stage matches director");
        if self.bypass & BYPASS_SHAKE != 0 {
            frame.eye_m = frame.eye_m - frame.shake_m;
            frame.shake_m = Vec3::ZERO;
        }
        if self.bypass & BYPASS_LOCK != 0 {
            frame.lock = None;
        }
        Ok(frame)
    }
}

/// GP20 look intent: where the camera wants to be this tick (raw rig output,
/// unsmoothed). Presentation data only, never sim input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookIntent {
    /// Wanted eye in meters (shaken, as presented).
    pub eye_m: Vec3,
    /// Wanted look point in meters.
    pub look_m: Vec3,
}

impl LookIntent {
    /// Capture the raw intent from a presented frame (no smoothing).
    #[must_use]
    pub fn from_frame(frame: &CameraFrame) -> Self {
        Self {
            eye_m: frame.eye_m,
            look_m: frame.look_m,
        }
    }
}

/// GP20 damped sync: the smoothed scene transform kept apart from the raw
/// intent. The frame rate only decides how many `sync` calls run between
/// ticks, never the sim bytes. Gain is integer permille per sync call; the
/// `f32` approach math is presentation-only. Zero heap bytes per call.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DampedSync {
    eye_m: Vec3,
    look_m: Vec3,
    gain_permille: u32,
    ready: bool,
}

impl DampedSync {
    /// Build a sync with per-call gain `1..=1000` (1000 snaps every call).
    pub fn new(gain_permille: u32) -> Result<Self, CameraError> {
        if !(1..=1000).contains(&gain_permille) {
            return Err(CameraError::BadParam("damping_gain"));
        }
        Ok(Self {
            eye_m: Vec3::ZERO,
            look_m: Vec3::ZERO,
            gain_permille,
            ready: false,
        })
    }

    /// Per-call gain permille.
    #[must_use]
    pub fn gain_permille(&self) -> u32 {
        self.gain_permille
    }

    /// Smoothed pose so far, if any intent synced yet.
    #[must_use]
    pub fn smoothed(&self) -> Option<LookIntent> {
        if self.ready {
            Some(LookIntent {
                eye_m: self.eye_m,
                look_m: self.look_m,
            })
        } else {
            None
        }
    }

    /// Intent bypass: the raw intent passes through untouched (identical).
    #[must_use]
    pub fn bypass(intent: &LookIntent) -> LookIntent {
        *intent
    }

    /// Fold one intent into the smoothed transform: the first call snaps,
    /// later calls approach by `gain_permille / 1000` per call (full gain
    /// snaps, bit-exact). The same intent sequence gives the same smoothed
    /// bytes twice. No heap.
    pub fn sync(&mut self, intent: &LookIntent) -> LookIntent {
        if !self.ready || self.gain_permille == 1000 {
            self.eye_m = intent.eye_m;
            self.look_m = intent.look_m;
            self.ready = true;
            return *intent;
        }
        let t = f32::from(u16::try_from(self.gain_permille).unwrap_or(1000)) / 1000.0;
        self.eye_m = self.eye_m + (intent.eye_m - self.eye_m) * t;
        self.look_m = self.look_m + (intent.look_m - self.look_m) * t;
        LookIntent {
            eye_m: self.eye_m,
            look_m: self.look_m,
        }
    }
}

/// Jab trauma: one landed light hit (matches the feed `TRAUMA_HIT`).
pub const TRAUMA_JAB_PERMILLE: u32 = 180;
/// Super trauma: a full-meter super (saturated trauma).
pub const TRAUMA_SUPER_PERMILLE: u32 = 1000;

/// Trauma response curve: squared trauma gain (`t^2`), so a jab moves
/// far less than a super. Pure presentation math, never sim state.
#[must_use]
pub fn trauma_gain(trauma_permille: u32) -> f32 {
    let clamped = trauma_permille.min(TRAUMA_SCALE);
    let t = f32::from(u16::try_from(clamped).unwrap_or(1000)) / 1000.0;
    t * t
}

/// Restore the unshaken pose eye from a presented frame: the shake offset is
/// presentation-only, so removing it returns the pose eye the next tick
/// selects from (within one float rounding step).
#[must_use]
pub fn unshaken_eye(frame: &CameraFrame) -> Vec3 {
    frame.eye_m - frame.shake_m
}

/// Presented shake displacement in meters for one frame.
#[must_use]
pub fn shake_displacement_m(frame: &CameraFrame) -> f32 {
    frame.shake_m.len()
}

// ---------------------------------------------------------------------------
// Fight-camera spring plus smooth trauma: dt-exact halflife smoothing for
// yaw, distance and look, with value-noise shake beside the frozen stages.
// All presentation-only, never sim input and never hashed. Zero heap bytes
// per call; same inputs replay byte-exact twice. Consumer is the future
// fight-camera caller; this slice lands the helper plus its proof.
// ---------------------------------------------------------------------------

/// Damping coefficient from a halflife in seconds: `ln(2) / halflife`.
/// Presentation smoothing rate, never sim state.
/// Errors: [`CameraError::BadParam`] naming `halflife_s` for non-finite or
/// non-positive input.
pub fn halflife_to_damping(halflife_s: f32) -> Result<f32, CameraError> {
    if !halflife_s.is_finite() || halflife_s <= 0.0 {
        return Err(CameraError::BadParam("halflife_s"));
    }
    Ok(core::f32::consts::LN_2 / halflife_s)
}

/// Exact exponential blend factor for `damping` over `dt_s` seconds:
/// `1 - exp(-damping * dt)`, in `0..=1`.
/// Presentation math, never sim state.
/// Errors: [`CameraError::BadParam`] naming `damping` or `dt_s` for
/// non-finite, negative damping or non-positive dt.
pub fn damper_blend(damping: f32, dt_s: f32) -> Result<f32, CameraError> {
    if !damping.is_finite() || damping < 0.0 {
        return Err(CameraError::BadParam("damping"));
    }
    if !dt_s.is_finite() || dt_s <= 0.0 {
        return Err(CameraError::BadParam("dt_s"));
    }
    let blend = 1.0 - (-damping * dt_s).exp();
    if !blend.is_finite() {
        return Err(CameraError::BadParam("dt_s"));
    }
    Ok(blend.clamp(0.0, 1.0))
}

/// Exact damper step of `current` toward `target` over `dt_s` seconds.
/// Presentation math, never sim state.
/// Errors: [`CameraError::BadParam`] naming the culprit row for non-finite
/// values or a bad step.
pub fn damper_step(current: f32, target: f32, damping: f32, dt_s: f32) -> Result<f32, CameraError> {
    if !current.is_finite() || !target.is_finite() {
        return Err(CameraError::BadParam("damper_value"));
    }
    let blend = damper_blend(damping, dt_s)?;
    Ok(current + (target - current) * blend)
}

/// Exact damper step for a look point over `dt_s` seconds.
/// One owner: each lane runs the scalar [`damper_step`], so the blend and
/// the error names stay shared. Multi-fault inputs report in lane order
/// (lane `x` first), exactly like three scalar calls; the sole caller
/// (`FightCamSpring::step`) pre-validates, so live ticks never see it.
/// Presentation math, never sim state.
/// Errors: [`CameraError::BadParam`] naming the culprit row.
pub fn damper_step_vec3(
    current: Vec3,
    target: Vec3,
    damping: f32,
    dt_s: f32,
) -> Result<Vec3, CameraError> {
    Ok(Vec3::new(
        damper_step(current.x, target.x, damping, dt_s)?,
        damper_step(current.y, target.y, damping, dt_s)?,
        damper_step(current.z, target.z, damping, dt_s)?,
    ))
}

/// Shortest signed yaw delta from `from_rad` to `to_rad`, in `[-PI, PI]`.
/// Wraps the long way around, so a 350 deg to 10 deg step goes +20 deg.
/// Presentation math, never sim state.
/// Errors: [`CameraError::BadParam`] naming `yaw_rad` for non-finite input.
pub fn shortest_arc_yaw(from_rad: f32, to_rad: f32) -> Result<f32, CameraError> {
    if !from_rad.is_finite() || !to_rad.is_finite() {
        return Err(CameraError::BadParam("yaw_rad"));
    }
    let delta = to_rad - from_rad;
    Ok((delta + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU) - core::f32::consts::PI)
}

/// Exact damper step for yaw over `dt_s` seconds along the shortest arc.
/// Presentation math, never sim state.
/// Errors: [`CameraError::BadParam`] naming the culprit row.
pub fn damper_step_yaw(
    current_rad: f32,
    target_rad: f32,
    damping: f32,
    dt_s: f32,
) -> Result<f32, CameraError> {
    let delta = shortest_arc_yaw(current_rad, target_rad)?;
    let blend = damper_blend(damping, dt_s)?;
    let stepped = current_rad + delta * blend;
    if !stepped.is_finite() {
        return Err(CameraError::BadParam("yaw_rad"));
    }
    Ok(stepped)
}

// Value-noise lattice seeds, one per shake channel. Small integers keep the
// hash domain distinct; the core owner stays the splitmix routine.
const TRAUMA_SEED_X: u64 = 11;
const TRAUMA_SEED_Y: u64 = 101;
const TRAUMA_SEED_Z: u64 = 1001;
const TRAUMA_SEED_ROLL: u64 = 7007;
/// Per-channel share of the shake cap: three `[-1, 1]` channels sum to at most
/// `sqrt(3)` in length, so `1 / sqrt(3)` each keeps the vector within `mag`.
const OFFSET_NORM: f32 = 0.5773503;

/// Map a hash to `[-1.0, 1.0]` from its low 16 bits, exactly representable.
fn unit_from_hash_low16(hash: u64) -> f32 {
    let lo = u16::try_from(hash & 65535).unwrap_or(0);
    f32::from(lo) * (2.0 / 65535.0) - 1.0
}

/// One-dimensional value noise in `[-1.0, 1.0]`: lattice values from the
/// core splitmix routine, smoothstep interpolation between neighbors.
/// Same seed plus same time gives the same value twice. Presentation only.
fn noise1(seed: u64, t: f32) -> f32 {
    if !t.is_finite() {
        return 0.0;
    }
    let base = t.floor();
    let frac = t - base;
    let h0 = engine_core::splitmix64(seed.wrapping_add(u64::from(base.to_bits())));
    let h1 = engine_core::splitmix64(seed.wrapping_add(u64::from((base + 1.0).to_bits())));
    let v0 = unit_from_hash_low16(h0);
    let v1 = unit_from_hash_low16(h1);
    let u = frac * frac * (3.0 - 2.0 * frac);
    v0 + (v1 - v0) * u
}

/// Smooth trauma shake: offset in meters plus roll in radians.
/// Presentation-only, never sim state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TraumaShake {
    /// Offset in meters, bounded by the squared trauma gain times `max_m`.
    pub offset_m: Vec3,
    /// Roll in radians, bounded by a tenth of the gain.
    pub roll_rad: f32,
}

/// Smooth trauma sample from value noise at `(tick + alpha) * 20 / 60`.
/// Three offset channels plus roll share the squared trauma gain, so a jab
/// moves far less than a super and the motion stays smooth across ticks.
/// Same inputs give the same shake twice. Presentation only, never sim state.
/// Errors: [`CameraError::BadParam`] naming `alpha` or `max_shake_m` for
/// out-of-range input; zero trauma or a non-positive cap reports zero shake.
pub fn smooth_trauma_offset(
    trauma_permille: u32,
    tick: u32,
    alpha: f32,
    max_m: f32,
) -> Result<TraumaShake, CameraError> {
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(CameraError::BadParam("alpha"));
    }
    if !max_m.is_finite() {
        return Err(CameraError::BadParam("max_shake_m"));
    }
    let clamped = trauma_permille.min(TRAUMA_SCALE);
    if clamped == 0 || max_m <= 0.0 {
        return Ok(TraumaShake {
            offset_m: Vec3::ZERO,
            roll_rad: 0.0,
        });
    }
    let gain = trauma_gain(clamped);
    let wrapped = tick % 4096;
    let tick_f = f32::from(u16::try_from(wrapped).unwrap_or(0));
    let t = (tick_f + alpha) * (20.0 / 60.0);
    let x = noise1(TRAUMA_SEED_X, t);
    let y = noise1(TRAUMA_SEED_Y, t);
    let z = noise1(TRAUMA_SEED_Z, t);
    let roll_noise = noise1(TRAUMA_SEED_ROLL, t);
    let mag = gain * max_m;
    // Three channels in `[-1, 1]` sum to at most `sqrt(3)` in length, so each
    // channel carries `mag / sqrt(3)` and the documented `mag` cap holds.
    let per_channel_m = mag * OFFSET_NORM;
    Ok(TraumaShake {
        offset_m: Vec3::new(x * per_channel_m, y * per_channel_m, z * per_channel_m),
        roll_rad: roll_noise * gain * 0.10,
    })
}

/// Whip cap: fastest the smoothed yaw ever pans, in radians per second.
/// The raw duel whips near 190 deg/s where the pair crosses, so an uncapped
/// tracker replays the whip instead of damping it. 35 deg/s keeps the
/// 30 fps per-frame eye travel inside the framing budget; distance and look
/// still track exactly, so framing never pays for the calm pan.
const MAX_PAN_RAD_S: f32 = 0.6108652; // 35 deg/s

/// Exact step of a velocity-compensated tracker: the closed-form update for
/// a target ramping with steady velocity over one step under `damping`.
/// `vel_tau` carries `v / damping` (velocity times `tau = 1 / damping`) and
/// `decay` carries `exp(-damping * dt)` as `1 - blend`. A still target
/// reduces to the exact exponential blend; a steadily moving target holds
/// the same `v * tau` lag at every frame rate, so 30 fps and 144 fps walk
/// the same continuous path instead of stranding a discretization gap.
/// One owner for the distance and look lanes; yaw adapts it along the
/// shortest arc in [`FightCamSpring::step`].
fn exact_step(old: f32, prev_target: f32, target: f32, vel_tau: f32, decay: f32) -> f32 {
    target - vel_tau + (old - prev_target + vel_tau) * decay
}

/// Fight-camera spring: velocity-compensated halflife tracking for yaw,
/// distance and look, with the yaw pan rate-limited. Distance and look hold
/// the exact exponential lag at every frame rate, so the rate only decides
/// the `dt_s` per `step` call; yaw adds the [`MAX_PAN_RAD_S`] whip cap on
/// top. Zero heap bytes per call; same target track gives the same smoothed
/// bytes twice. Presentation only, never sim input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FightCamSpring {
    yaw_rad: f32,
    dist_m: f32,
    look_m: Vec3,
    prev_yaw_rad: f32,
    prev_dist_m: f32,
    prev_look_m: Vec3,
    damping: f32,
    ready: bool,
}

impl FightCamSpring {
    /// Build a spring with halflife `halflife_s` seconds.
    /// Errors: [`CameraError::BadParam`] naming `halflife_s` for non-finite
    /// or non-positive input.
    pub fn new(halflife_s: f32) -> Result<Self, CameraError> {
        Ok(Self {
            yaw_rad: 0.0,
            dist_m: 0.0,
            look_m: Vec3::ZERO,
            prev_yaw_rad: 0.0,
            prev_dist_m: 0.0,
            prev_look_m: Vec3::ZERO,
            damping: halflife_to_damping(halflife_s)?,
            ready: false,
        })
    }

    /// Damping coefficient derived from the halflife.
    #[must_use]
    pub fn damping(&self) -> f32 {
        self.damping
    }

    /// Smoothed yaw, distance and look so far, if any target stepped yet.
    #[must_use]
    pub fn smoothed(&self) -> Option<(f32, f32, Vec3)> {
        if self.ready {
            Some((self.yaw_rad, self.dist_m, self.look_m))
        } else {
            None
        }
    }

    /// Snap to a target without smoothing (tick boundaries, teleports).
    /// Errors: [`CameraError::BadParam`] naming the culprit row.
    pub fn reset(&mut self, yaw_rad: f32, dist_m: f32, look_m: Vec3) -> Result<(), CameraError> {
        if !yaw_rad.is_finite() {
            return Err(CameraError::BadParam("yaw_rad"));
        }
        if !dist_m.is_finite() || dist_m <= 0.0 {
            return Err(CameraError::BadParam("dist_m"));
        }
        if !look_m.is_finite() {
            return Err(CameraError::BadParam("look_m"));
        }
        self.yaw_rad = yaw_rad;
        self.dist_m = dist_m;
        self.look_m = look_m;
        self.prev_yaw_rad = yaw_rad;
        self.prev_dist_m = dist_m;
        self.prev_look_m = look_m;
        self.ready = true;
        Ok(())
    }

    /// Fold one target into the smoothed state with a velocity-compensated
    /// `dt_s` step: yaw tracks along the shortest arc at most
    /// [`MAX_PAN_RAD_S`] per second, distance and look hold the exact
    /// exponential lag, so a steadily moving target leaves no rate gap.
    /// The first call snaps (state and velocity reference both). Same target
    /// track gives the same bytes twice.
    /// Errors: [`CameraError::BadParam`] naming the culprit row
    /// (`target_*` for bad input, `yaw_rad`/`damper_value` for a
    /// non-finite update).
    pub fn step(
        &mut self,
        target_yaw_rad: f32,
        target_dist_m: f32,
        target_look_m: Vec3,
        dt_s: f32,
    ) -> Result<(f32, f32, Vec3), CameraError> {
        if !target_yaw_rad.is_finite() {
            return Err(CameraError::BadParam("target_yaw_rad"));
        }
        if !target_dist_m.is_finite() || target_dist_m <= 0.0 {
            return Err(CameraError::BadParam("target_dist_m"));
        }
        if !target_look_m.is_finite() {
            return Err(CameraError::BadParam("target_look_m"));
        }
        if !dt_s.is_finite() || dt_s <= 0.0 {
            return Err(CameraError::BadParam("dt_s"));
        }
        if !self.ready {
            self.yaw_rad = target_yaw_rad;
            self.dist_m = target_dist_m;
            self.look_m = target_look_m;
            self.prev_yaw_rad = target_yaw_rad;
            self.prev_dist_m = target_dist_m;
            self.prev_look_m = target_look_m;
            self.ready = true;
            return Ok((self.yaw_rad, self.dist_m, self.look_m));
        }
        // Shared decay `exp(-damping * dt)` through the one blend owner.
        let decay = 1.0 - damper_blend(self.damping, dt_s)?;
        // Yaw along the shortest arc: raw-target velocity (arc over dt)
        // feeds the exact ramp update, then the whip cap clamps the move.
        let yaw_vel = shortest_arc_yaw(self.prev_yaw_rad, target_yaw_rad)? / dt_s;
        let yaw_vt = yaw_vel / self.damping;
        let yaw_old_arc = shortest_arc_yaw(self.prev_yaw_rad, self.yaw_rad)?;
        let mut yaw = target_yaw_rad - yaw_vt + (yaw_old_arc + yaw_vt) * decay;
        if !yaw.is_finite() {
            return Err(CameraError::BadParam("yaw_rad"));
        }
        let yaw_move = shortest_arc_yaw(self.yaw_rad, yaw)?;
        let yaw_lim = MAX_PAN_RAD_S * dt_s;
        if yaw_move.abs() > yaw_lim {
            yaw = self.yaw_rad + yaw_move.signum() * yaw_lim;
        }
        // Distance and look: exact ramp update per lane, same lag at any rate.
        let dist_vel = (target_dist_m - self.prev_dist_m) / dt_s;
        let dist = exact_step(
            self.dist_m,
            self.prev_dist_m,
            target_dist_m,
            dist_vel / self.damping,
            decay,
        );
        if !dist.is_finite() {
            return Err(CameraError::BadParam("damper_value"));
        }
        let look_vel = (target_look_m - self.prev_look_m) * (1.0 / dt_s);
        let look_vt = look_vel * (1.0 / self.damping);
        let look = Vec3::new(
            exact_step(
                self.look_m.x,
                self.prev_look_m.x,
                target_look_m.x,
                look_vt.x,
                decay,
            ),
            exact_step(
                self.look_m.y,
                self.prev_look_m.y,
                target_look_m.y,
                look_vt.y,
                decay,
            ),
            exact_step(
                self.look_m.z,
                self.prev_look_m.z,
                target_look_m.z,
                look_vt.z,
                decay,
            ),
        );
        if !look.is_finite() {
            return Err(CameraError::BadParam("damper_value"));
        }
        self.yaw_rad = yaw;
        self.dist_m = dist;
        self.look_m = look;
        self.prev_yaw_rad = target_yaw_rad;
        self.prev_dist_m = target_dist_m;
        self.prev_look_m = target_look_m;
        Ok((self.yaw_rad, self.dist_m, self.look_m))
    }

    /// Eye from the smoothed yaw, distance and look under `params` boom,
    /// shortened at pillars and walls by the `camera_arm` sphere sweep.
    /// Yaw zero sits behind on `+z` like the pair framing; other yaws orbit
    /// the boom around the look point. `obstacles` holds the arena pillar
    /// and wall boxes; the fighter focus body is excluded by the caller
    /// (Godot `excluded_objects`) so it never blocks. Presentation only.
    /// PORT(godotengine/godot@4d10419cacd6d3009976e8f9fd3abd970fd58018:scene/3d/physics/spring_arm_3d.cpp):
    /// `process_spring` shape cast gives `cast_motion`, margin off the hit.
    /// PORT(godotengine/godot docs 2026-10-15:https://docs.godotengine.org/en/stable/classes/class_springarm3d.html):
    /// `spring_length` max extent, `margin` 0.01, `get_hit_length` reads length.
    #[must_use]
    pub fn eye_m(&self, params: &DirectorParams, obstacles: &[Aabb]) -> Vec3 {
        let sin_yaw = self.yaw_rad.sin();
        let cos_yaw = self.yaw_rad.cos();
        let smooth_eye = Vec3::new(
            self.look_m.x + sin_yaw * self.dist_m * params.boom_back,
            self.look_m.y + params.boom_up * self.dist_m,
            self.look_m.z + cos_yaw * self.dist_m * params.boom_back,
        );
        let dir = smooth_eye - self.look_m;
        let dist = dir.len();
        if !dist.is_finite() || dist <= 0.0 {
            return smooth_eye;
        }
        let pivot = [self.look_m.x, self.look_m.y, self.look_m.z];
        let raw = [dir.x, dir.y, dir.z];
        let radius = if params.boom_radius_m.is_finite() && params.boom_radius_m >= 0.0 {
            params.boom_radius_m
        } else {
            0.0
        };
        let margin = if params.boom_margin_m.is_finite() && params.boom_margin_m >= 0.0 {
            params.boom_margin_m
        } else {
            0.0
        };
        let hit = arm_length(pivot, raw, dist, radius, obstacles);
        if hit >= dist {
            return smooth_eye;
        }
        let clamped = (hit - margin).max(FLOOR_LEN).min(dist);
        self.look_m + dir * (clamped / dist)
    }
}

/// Fight targets from a fighter pair under `params`: yaw perpendicular to
/// the pair axis (side view), distance fitting the separation like the pair
/// framing, look at the midpoint. Reuses the frozen pair fit, so the raw
/// targets frame exactly and the spring only smooths time.
/// Errors: [`CameraError::BadParam`] naming the culprit row.
pub fn fight_targets(
    focus_m: Vec3,
    other_m: Vec3,
    params: &DirectorParams,
) -> Result<(f32, f32, Vec3), CameraError> {
    if !focus_m.is_finite() || !other_m.is_finite() {
        return Err(CameraError::BadParam("focus_m"));
    }
    let pose = fit_pair(focus_m, Some(other_m), params)?;
    let look_m = pose.look_m;
    let boom_len_sq = params.boom_up * params.boom_up + params.boom_back * params.boom_back;
    let boom_len = boom_len_sq.sqrt();
    if !boom_len.is_finite() || boom_len <= 0.0 {
        return Err(CameraError::BadParam("boom"));
    }
    let reach = (pose.eye_m - pose.look_m).len();
    let dist_m = reach / boom_len;
    if !dist_m.is_finite() || dist_m <= 0.0 {
        return Err(CameraError::BadParam("dist_m"));
    }
    let dx = other_m.x - focus_m.x;
    let dz = other_m.z - focus_m.z;
    let yaw_rad = (-dz).atan2(dx);
    if !yaw_rad.is_finite() {
        return Err(CameraError::BadParam("yaw_rad"));
    }
    Ok((yaw_rad, dist_m, look_m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{count_unframed, CameraFrame, DirectorParams, FighterView};
    use core::f32::consts::PI;

    fn deg_of(rad: f32) -> f32 {
        rad * 180.0 / PI
    }

    fn percentile99(mut values: Vec<f32>) -> f32 {
        values.sort_by(|a, b| a.total_cmp(b));
        let at = ((values.len() as f32 * 0.99) as usize).min(values.len().saturating_sub(1));
        values[at]
    }

    fn pair_at(t_s: f32) -> (Vec3, Vec3) {
        let a = Vec3::new(0.0, 1.0, 0.0);
        let bx = -8.0 + 1.7 * t_s;
        let b = Vec3::new(bx, 1.0, 0.5);
        (a, b)
    }

    struct DuelRun {
        yaws: Vec<f32>,
        looks: Vec<Vec3>,
        eyes: Vec<Vec3>,
        pairs: Vec<(Vec3, Vec3)>,
        raw_yaws: Vec<f32>,
        raw_eyes: Vec<Vec3>,
    }

    fn run_duel(dt_s: f32, steps: usize, halflife_s: f32, params: &DirectorParams) -> DuelRun {
        let mut spring = FightCamSpring::new(halflife_s).expect("spring builds");
        let mut yaws = Vec::with_capacity(steps);
        let mut looks = Vec::with_capacity(steps);
        let mut eyes = Vec::with_capacity(steps);
        let mut pairs = Vec::with_capacity(steps);
        let mut raw_yaws = Vec::with_capacity(steps);
        let mut raw_eyes = Vec::with_capacity(steps);
        for n in 0..steps {
            let t_s = n as f32 * dt_s;
            let (a, b) = pair_at(t_s);
            let (yaw, dist, look) = fight_targets(a, b, params).expect("targets");
            raw_yaws.push(yaw);
            let sin_y = yaw.sin();
            let cos_y = yaw.cos();
            raw_eyes.push(Vec3::new(
                look.x + sin_y * dist * params.boom_back,
                look.y + params.boom_up * dist,
                look.z + cos_y * dist * params.boom_back,
            ));
            let (sy, _sd, sl) = spring.step(yaw, dist, look, dt_s).expect("step");
            yaws.push(sy);
            looks.push(sl);
            eyes.push(spring.eye_m(params, &[]));
            pairs.push((a, b));
        }
        DuelRun {
            yaws,
            looks,
            eyes,
            pairs,
            raw_yaws,
            raw_eyes,
        }
    }

    fn yaw_rates_deg(yaws: &[f32], dt_s: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(yaws.len().saturating_sub(1));
        for pair in yaws.windows(2) {
            let delta = shortest_arc_yaw(pair[0], pair[1]).expect("arc");
            out.push(deg_of(delta.abs() / dt_s));
        }
        out
    }

    fn eye_steps_cm(eyes: &[Vec3]) -> Vec<f32> {
        let mut out = Vec::with_capacity(eyes.len().saturating_sub(1));
        for pair in eyes.windows(2) {
            out.push((pair[1] - pair[0]).len() * 100.0);
        }
        out
    }

    #[test]
    fn fight_cam_spring_smooths_duel_at_both_rates() {
        let params = DirectorParams::default();
        let halflife_s = 0.25;
        let dt30 = 1.0 / 30.0;
        let dt144 = 1.0 / 144.0;
        let run30 = run_duel(dt30, 301, halflife_s, &params);
        let run144 = run_duel(dt144, 1441, halflife_s, &params);
        assert_eq!(run30.yaws.len(), 301);
        assert_eq!(run144.yaws.len(), 1441);

        let raw_rates = yaw_rates_deg(&run30.raw_yaws, dt30);
        let raw_p99 = percentile99(raw_rates);
        assert!(
            raw_p99 > 60.0,
            "fixture must whip fast unsmoothed, raw p99 {raw_p99}"
        );
        let raw_steps = eye_steps_cm(&run30.raw_eyes);
        let raw_peak = raw_steps.iter().fold(0.0f32, |a, b| a.max(*b));
        assert!(
            raw_peak > 25.0,
            "unsmoothed eye must jump, raw peak {raw_peak} cm"
        );

        for (name, run, dt) in [("30fps", &run30, dt30), ("144fps", &run144, dt144)] {
            let rates = yaw_rates_deg(&run.yaws, dt);
            let p99 = percentile99(rates);
            assert!(p99 <= 60.0, "{name} yaw p99 {p99} deg/s over 60");
            let steps = eye_steps_cm(&run.eyes);
            let peak = steps.iter().fold(0.0f32, |a, b| a.max(*b));
            assert!(peak <= 25.0, "{name} eye peak {peak} cm over 25");
            for (index, ((a, b), (eye, look))) in run
                .pairs
                .iter()
                .zip(run.eyes.iter().zip(run.looks.iter()))
                .enumerate()
            {
                let frame = CameraFrame {
                    tick: index as u32,
                    eye_m: *eye,
                    look_m: *look,
                    shake_m: Vec3::ZERO,
                    lock: None,
                    trauma_permille: 0,
                };
                let views = [
                    FighterView {
                        id: 0,
                        pos_m: *a,
                        alive: true,
                    },
                    FighterView {
                        id: 1,
                        pos_m: *b,
                        alive: true,
                    },
                ];
                assert_eq!(
                    count_unframed(&frame, &views, &params).expect("oracle"),
                    0,
                    "{name} lost framing at frame {index}"
                );
            }
        }

        let eye_gap_cm =
            (*run30.eyes.last().expect("last") - *run144.eyes.last().expect("last")).len() * 100.0;
        let look_gap_cm =
            (*run30.looks.last().expect("last") - *run144.looks.last().expect("last")).len()
                * 100.0;
        assert!(eye_gap_cm <= 1.0, "rate gap {eye_gap_cm} cm over 1 cm");
        assert!(look_gap_cm <= 1.0, "look gap {look_gap_cm} cm over 1 cm");

        let again30 = run_duel(dt30, 301, halflife_s, &params);
        assert_eq!(run30.yaws, again30.yaws, "same track replays exactly");
        assert_eq!(run30.eyes, again30.eyes, "same eyes replay exactly");
    }

    #[test]
    fn fight_cam_spring_rejects_bad_inputs() {
        let params = DirectorParams::default();
        assert_eq!(
            FightCamSpring::new(0.0),
            Err(CameraError::BadParam("halflife_s"))
        );
        assert_eq!(
            FightCamSpring::new(f32::NAN),
            Err(CameraError::BadParam("halflife_s"))
        );
        assert_eq!(
            halflife_to_damping(-1.0),
            Err(CameraError::BadParam("halflife_s"))
        );
        assert_eq!(
            damper_blend(f32::NAN, 0.01),
            Err(CameraError::BadParam("damping"))
        );
        assert_eq!(damper_blend(2.0, 0.0), Err(CameraError::BadParam("dt_s")));
        assert_eq!(
            damper_step(0.0, f32::INFINITY, 2.0, 0.01),
            Err(CameraError::BadParam("damper_value"))
        );
        assert_eq!(
            shortest_arc_yaw(0.0, f32::NAN),
            Err(CameraError::BadParam("yaw_rad"))
        );
        let from = 350.0 * PI / 180.0;
        let to = 10.0 * PI / 180.0;
        let delta = shortest_arc_yaw(from, to).expect("arc");
        assert!(
            (deg_of(delta) - 20.0).abs() < 0.5,
            "wrap must take the short way, got {} deg",
            deg_of(delta)
        );
        let mut spring = FightCamSpring::new(0.2).expect("spring");
        assert_eq!(spring.smoothed(), None);
        assert_eq!(
            spring.step(0.0, 4.0, Vec3::ZERO, 0.0),
            Err(CameraError::BadParam("dt_s"))
        );
        assert_eq!(
            fight_targets(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::ZERO, &params),
            Err(CameraError::BadParam("focus_m"))
        );
        spring
            .reset(0.1, 4.0, Vec3::new(0.0, 1.0, 0.0))
            .expect("reset");
        assert!(spring.smoothed().is_some());
        assert_eq!(
            spring.reset(f32::NAN, 4.0, Vec3::ZERO),
            Err(CameraError::BadParam("yaw_rad"))
        );
    }

    #[test]
    fn fight_cam_spring_trauma_is_smooth_and_bounded() {
        let params = DirectorParams::default();
        let calm = smooth_trauma_offset(0, 9, 0.5, params.max_shake_m).expect("calm");
        assert_eq!(calm.offset_m, Vec3::ZERO);
        assert_eq!(calm.roll_rad, 0.0);
        let full_a = smooth_trauma_offset(1000, 40, 0.0, params.max_shake_m).expect("full");
        let full_b = smooth_trauma_offset(1000, 40, 0.0, params.max_shake_m).expect("full");
        assert_eq!(full_a, full_b, "same inputs give the same shake");
        let mag = params.max_shake_m;
        for tick in [0u32, 1, 40, 400] {
            let shake = smooth_trauma_offset(1000, tick, 0.25, mag).expect("shake");
            assert!(
                shake.offset_m.len() <= mag + 1.0e-5,
                "shake over cap at tick {tick}"
            );
            assert!(
                shake.roll_rad.abs() <= 0.10 + 1.0e-5,
                "roll over cap at tick {tick}"
            );
        }
        let jab = smooth_trauma_offset(180, 40, 0.25, mag).expect("jab");
        let sup = smooth_trauma_offset(1000, 40, 0.25, mag).expect("super");
        assert!(
            jab.offset_m.len() <= 0.25 * sup.offset_m.len() + 1.0e-5 || sup.offset_m.len() < 1.0e-6,
            "jab must stay far under super"
        );
        let s0 = smooth_trauma_offset(1000, 40, 0.0, mag).expect("s0");
        let s1 = smooth_trauma_offset(1000, 40, 0.5, mag).expect("s1");
        let sub_step = (s1.offset_m - s0.offset_m).len();
        assert!(
            sub_step <= 0.30,
            "sub-tick step {sub_step} m over smooth bound"
        );
        let t0 = smooth_trauma_offset(1000, 40, 0.5, mag).expect("t0");
        let t1 = smooth_trauma_offset(1000, 41, 0.5, mag).expect("t1");
        let tick_step = (t1.offset_m - t0.offset_m).len();
        assert!(
            tick_step <= 0.30,
            "tick step {tick_step} m over smooth bound"
        );
        assert_eq!(
            smooth_trauma_offset(1000, 40, 2.0, mag),
            Err(CameraError::BadParam("alpha"))
        );
        assert_eq!(
            smooth_trauma_offset(1000, 40, 0.5, f32::NAN),
            Err(CameraError::BadParam("max_shake_m"))
        );
    }

    #[test]
    fn damper_step_vec3_matches_scalar_lanes() {
        // Value parity: one shared blend per lane, bit-exact on valid inputs.
        let cases = [
            (
                Vec3::new(0.0, 1.0, -2.0),
                Vec3::new(4.0, -1.0, 3.0),
                2.0,
                0.016,
            ),
            (Vec3::new(5.0, 5.0, 5.0), Vec3::new(5.0, 5.0, 5.0), 0.0, 0.1),
            (
                Vec3::new(-8.0, 0.5, 2.0),
                Vec3::new(1.0, 4.0, 6.0),
                10.0,
                1.0 / 144.0,
            ),
        ];
        for (current, target, damping, dt) in cases {
            let vec = damper_step_vec3(current, target, damping, dt).expect("vec step");
            let lanes = Vec3::new(
                damper_step(current.x, target.x, damping, dt).expect("lane x"),
                damper_step(current.y, target.y, damping, dt).expect("lane y"),
                damper_step(current.z, target.z, damping, dt).expect("lane z"),
            );
            assert_eq!(vec, lanes, "vec3 must equal per-lane scalar steps");
        }
        // Error parity, single fault: the same name either way.
        assert_eq!(
            damper_step_vec3(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::ZERO, 2.0, 0.01),
            Err(CameraError::BadParam("damper_value"))
        );
        assert_eq!(
            damper_step_vec3(Vec3::ZERO, Vec3::ZERO, f32::NAN, 0.01),
            Err(CameraError::BadParam("damping"))
        );
        assert_eq!(
            damper_step_vec3(Vec3::ZERO, Vec3::ZERO, 2.0, 0.0),
            Err(CameraError::BadParam("dt_s"))
        );
        // Multi-fault precedence follows lane order: lane `x` reports first,
        // so a bad blend wins over a later bad lane, exactly like three
        // scalar calls.
        assert_eq!(
            damper_step_vec3(
                Vec3::new(1.0, f32::NAN, 3.0),
                Vec3::new(4.0, 5.0, 6.0),
                f32::NAN,
                0.01
            ),
            Err(CameraError::BadParam("damping"))
        );
    }
}
