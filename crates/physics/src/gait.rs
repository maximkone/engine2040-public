//! gait: a procedural walk cycle that drives the powered ragdoll's servo pose.
//!
//! Consumer: the arena locomotion loop advances `GaitCycle` from the wanted
//! speed and turn, reads `GaitCycle::pose`, feeds it to
//! `PoweredRagdoll::set_animated`
//! and lets `balance` bias it through
//! `GaitCycle::set_bias`. The legs carry the body: the root travel is derived
//! from the planted foot's own backward move, so a shove, a slope or a step
//! changes the contacts instead of swapping a clip. Owner: this module owns the
//! phase, the stand blend, the planted-foot travel, the recovery step and the
//! joint angles. Units: millimetres, metres, ticks at 60 Hz, Q10 radians for
//! turn, Q8 for the balance bias. Determinism: the phase maths is integer (a
//! const Q15 sine table, an integer index plus a linear interpolation); the
//! pose's float work is `+ - * /` and one `sqrt` per quaternion, so one input
//! sequence hashes one way. No libm and no allocation on the tick path
//! (`GaitCycle::pose_into` reuses caller buffers).
#![forbid(unsafe_code)]

use super::balance::CaptureBias;
use engine_anim::contract::{LocalPose, Skeleton};
use engine_core::checked::{f32_from_u64, saturating_f64_to_u64};
use engine_core::Fnv64;

/// Table steps in one cycle: the Q15 sine table resolution.
pub const PHASE_STEPS: u16 = 64;
/// One table step in Q8 units: the phase maths runs on integers, so the table
/// index and its fraction are exact and no float picks a table entry.
const STEP_Q8: u32 = 256;
/// One table step as a float, the interpolation's unit.
const STEP_F: f32 = 256.0;
/// One full table turn in Q8 units, [`PHASE_STEPS`] steps.
const TURN_Q8: u32 = 16_384;
/// Cycle length in ticks at [`REF_SPEED_MM_S`]: 20 ticks is a 1.0 m stride at
/// 3 m/s, so the cadence carries the body at the speed the caller asks for.
pub const CYCLE_TICKS_AT_3MPS: u16 = 20;
/// Reference walking speed the cycle length is given at, mm/s.
pub const REF_SPEED_MM_S: i32 = 3000;
/// Widest walking speed the cycle advances at, mm/s; faster callers clamp.
pub const MAX_SPEED_MM_S: i32 = 12_000;
/// Widest recovery step a shove may ask for, millimetres.
pub const STEP_OUT_MAX_MM: i32 = 220;
/// Ticks a recovery step spreads its extra travel over.
pub const STEP_OUT_TICKS: u16 = 10;
/// Height of the recovery step's hop, millimetres: enough to lift both foot
/// capsules clear of the floor, so the step is a step and not a glide.
pub const STEP_OUT_HOP_MM: i32 = 30;
/// Q15 one: the sine table's scale and the facing pair's scale.
const Q15_ONE: i32 = 32768;
/// Q10 one: one radian for the turn rate.
const Q10_ONE: i32 = 1024;
/// Degrees to radians, as data (no libm in the tick).
const DEG: f32 = core::f32::consts::PI / 180.0;
/// Hip lean the balance bias may add at full Q8 scale, radians.
const BIAS_HIP_RAD: f32 = 0.12;
/// Ankle lean the balance bias may add at full Q8 scale, radians.
const BIAS_ANKLE_RAD: f32 = 0.12;

/// One Q15 sine period: `sin(2*pi*i/64) * 32767`, rounded once, as data.
const SIN_Q15: [i16; 64] = [
    0, 3212, 6393, 9512, 12539, 15446, 18204, 20787, 23170, 25329, 27245, 28898, 30273, 31356,
    32137, 32609, 32767, 32609, 32137, 31356, 30273, 28898, 27245, 25329, 23170, 20787, 18204,
    15446, 12539, 9512, 6393, 3212, 0, -3212, -6393, -9512, -12539, -15446, -18204, -20787, -23170,
    -25329, -27245, -28898, -30273, -31356, -32137, -32609, -32767, -32609, -32137, -31356, -30273,
    -28898, -27245, -25329, -23170, -20787, -18204, -15446, -12539, -9512, -6393, -3212,
];

/// Walk tuning: the swing amplitudes plus the cycle, blend and turn rates.
/// Every field is clamped at use, so a tuned table can never put a non-finite
/// number into the tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GaitParams {
    /// Hip swing amplitude, degrees: the half stride angle. A stride per cycle
    /// is `4 * leg * sin(hip_swing)`; 19.5 deg on 0.9 m legs is 1.2 m.
    pub hip_swing_deg: f32,
    /// Knee swing amplitude, degrees: the knee flexes only in swing and is
    /// straight at both stance ends, so the stance foot stays planted.
    pub knee_swing_deg: f32,
    /// Toe lift in swing, degrees: the ankle levels the foot off the shin and
    /// lifts the toes clear of the floor.
    pub ankle_deg: f32,
    /// Arm counter-swing amplitude, degrees.
    pub arm_swing_deg: f32,
    /// Cycle length in ticks at [`REF_SPEED_MM_S`]: the cadence scales with
    /// the speed from here, so the stride and the speed agree.
    pub cycle_ticks_at_3mps: u16,
    /// Pelvis bob amplitude, millimetres, at twice the leg rate.
    pub pelvis_bob_mm: i32,
    /// Stand-to-walk blend, ticks: the pose eases in and out of the walk over
    /// this window, and reaching 0 is the stand.
    pub blend_ticks: u16,
    /// Widest turn, Q10 radians per tick (1024 = 1 rad per tick).
    pub max_turn_q10: i32,
}

impl Default for GaitParams {
    /// A 1.0 m stride at 3 m/s on 0.9 m legs: 16.2 deg hips, 55 deg knees, a
    /// 3 mm bob, a 20-tick cycle and a 5-tick blend. The short blend keeps the
    /// start-up travel loss (about 100 mm at 3 m/s) inside the walk tolerance.
    fn default() -> Self {
        Self {
            hip_swing_deg: 16.2,
            knee_swing_deg: 55.0,
            ankle_deg: 10.0,
            arm_swing_deg: 18.0,
            cycle_ticks_at_3mps: CYCLE_TICKS_AT_3MPS,
            pelvis_bob_mm: 3,
            blend_ticks: 5,
            max_turn_q10: 24,
        }
    }
}

impl GaitParams {
    /// Hip amplitude, radians.
    fn hip_rad(&self) -> f32 {
        self.hip_swing_deg * DEG
    }
    /// Knee amplitude, radians.
    fn knee_rad(&self) -> f32 {
        self.knee_swing_deg * DEG
    }
    /// Toe lift, radians.
    fn ankle_rad(&self) -> f32 {
        self.ankle_deg * DEG
    }
    /// Arm amplitude, radians.
    fn arm_rad(&self) -> f32 {
        self.arm_swing_deg * DEG
    }
    /// Cycle length in ticks, at least one (zero would divide by it).
    fn cycle_ticks(&self) -> u16 {
        self.cycle_ticks_at_3mps.max(1)
    }
    /// Blend window in ticks, at least one.
    fn blend_window(&self) -> u16 {
        self.blend_ticks.max(1)
    }
}

/// Joint names the cycle poses. A rig without them keeps its rest pose there,
/// and the walk then travels nowhere rather than panic.
const PELVIS: &str = "pelvis";
const THIGH: [&str; 2] = ["thigh_l", "thigh_r"];
const SHIN: [&str; 2] = ["shin_l", "shin_r"];
const FOOT: [&str; 2] = ["foot_l", "foot_r"];
const ARM: [&str; 2] = ["arm_l", "arm_r"];

/// Bone slots the pose writes, resolved from joint names.
#[derive(Clone, Copy, Debug, Default)]
struct Slots {
    pelvis: Option<usize>,
    thigh: [Option<usize>; 2],
    shin: [Option<usize>; 2],
    foot: [Option<usize>; 2],
    arm: [Option<usize>; 2],
}

impl Slots {
    /// Resolve the biped slots by name; the pelvis falls back to joint 0, the
    /// root of a nameless rig.
    fn find(skeleton: &Skeleton) -> Self {
        let named = |name: &str| skeleton.joints.iter().position(|j| j.name == name);
        let root = if skeleton.is_empty() { None } else { Some(0) };
        let mut slots = Slots {
            pelvis: named(PELVIS).or(root),
            ..Slots::default()
        };
        for (side, arm) in ARM.iter().enumerate() {
            slots.thigh[side] = named(THIGH[side]);
            slots.shin[side] = named(SHIN[side]);
            slots.foot[side] = named(FOOT[side]);
            slots.arm[side] = named(arm);
        }
        slots
    }

    /// Leg segment lengths from the rest offsets, meters `[thigh, shin]`, read
    /// as the downward extent so a rig that names standard legs gets its own
    /// stride length. Missing legs read zero.
    fn leg_lengths(&self, rest: &LocalPose) -> [f32; 2] {
        let down = |slot: Option<usize>| -> f32 {
            slot.and_then(|i| rest.positions.get(i))
                .map(|p| -p[1])
                .filter(|d| d.is_finite() && *d > 0.0)
                .unwrap_or(0.0)
        };
        // Both legs share the rig's segment lengths; the left side is the owner.
        [down(self.shin[0]), down(self.foot[0])]
    }
}

/// One leg's joint pitch this tick, radians, positive swings the limb forward
/// toward +X: the hip, the knee (at or behind straight) and the ankle, which
/// levels the foot off the shin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct LegAngles {
    hip: f32,
    knee: f32,
    ankle: f32,
}

/// Procedural walk cycle: the phase, the stand blend, the planted-foot travel,
/// the facing and the joint-angle pose. One instance drives one body.
#[derive(Clone, Debug, PartialEq)]
pub struct GaitCycle {
    /// Walk tuning.
    params: GaitParams,
    /// Rest pose: unwalked joints keep these positions and rotations.
    rest: LocalPose,
    /// The rig's leg segment lengths, meters `[thigh, shin]`.
    lengths: [f32; 2],
    /// Cycle phase in Q8 phase units, `0..cycle_ticks * 256`.
    phase_q8: u32,
    /// Last commanded speed, mm/s.
    speed_mm_s: i32,
    /// Last commanded turn, Q10 radians per tick.
    turn_q10: i32,
    /// Stand blend in ticks, `0..=blend_ticks`; 0 is the stand.
    blend: u16,
    /// Root travel from the start, integer millimetres on the ground plane.
    travel_mm: [i64; 2],
    /// Facing heading as a Q15 `[cos, sin]` pair over `(x, z)`.
    face_q15: [i32; 2],
    /// Half heading as a Q15 `[cos, sin]` pair, for the pelvis quaternion.
    half_q15: [i32; 2],
    /// Recovery step still owed, millimetres.
    step_extra_mm: i32,
    /// Ticks left in the recovery step's spread.
    step_left: u16,
    /// Balance ankle bias, Q8.
    bias_ankle_q8: i16,
    /// Balance hip bias, Q8.
    bias_hip_q8: i16,
}

impl GaitCycle {
    /// Fresh cycle bound to the rig it walks: the rest pose plus the leg
    /// lengths the stride reads. Standing, phase zero, facing the rest pose's
    /// +X.
    pub fn new(params: GaitParams, skeleton: &Skeleton, rest: LocalPose) -> Self {
        let lengths = Slots::find(skeleton).leg_lengths(&rest);
        Self {
            params,
            rest,
            lengths,
            phase_q8: 0,
            speed_mm_s: 0,
            turn_q10: 0,
            blend: 0,
            travel_mm: [0, 0],
            face_q15: [Q15_ONE, 0],
            half_q15: [Q15_ONE, 0],
            step_extra_mm: 0,
            step_left: 0,
            bias_ankle_q8: 0,
            bias_hip_q8: 0,
        }
    }

    /// Current tuning.
    pub fn params(&self) -> GaitParams {
        self.params
    }

    /// The rest pose the cycle walks from.
    pub fn rest(&self) -> &LocalPose {
        &self.rest
    }

    /// Whole cycle phase, `0..=cycle_ticks`; the pose's phase argument.
    pub fn phase(&self) -> u16 {
        // The shift stays inside the cycle bound, so the narrowing is exact.
        u16::try_from(self.phase_q8 >> 8).unwrap_or(0)
    }

    /// Cycle phase in Q8 units: the tick path's finer phase.
    pub fn phase_q8(&self) -> u32 {
        self.phase_q8
    }

    /// Last commanded speed, mm/s.
    pub fn speed_mm_s(&self) -> i32 {
        self.speed_mm_s
    }

    /// Stand blend in ticks.
    pub fn blend(&self) -> u16 {
        self.blend
    }

    /// True when the body stands: no speed commanded and the blend reached 0,
    /// so the pose is the rest pose and the root no longer travels.
    pub fn standing(&self) -> bool {
        self.speed_mm_s == 0 && self.blend == 0
    }

    /// Root travel from the start, integer millimetres `[x, z]`.
    pub fn travel_mm(&self) -> [i64; 2] {
        self.travel_mm
    }

    /// Facing heading as a Q15 `[cos, sin]` pair over `(x, z)`.
    pub fn facing_q15(&self) -> [i32; 2] {
        self.face_q15
    }

    /// Recovery step still owed, millimetres.
    pub fn step_extra_mm(&self) -> i32 {
        self.step_extra_mm
    }

    /// Feet the pose plants this tick, `[left, right]`: the stance leg's foot
    /// holds its world place, so a presentation can time a step or mute a
    /// sliding foot on it.
    pub fn foot_stance(&self) -> [bool; 2] {
        let left = stance_left(self.idx_q8(self.phase_q8));
        [left, !left]
    }

    /// Install the balance capture bias: the ankle shift opposes the sagittal
    /// lean, the hip shift the lateral one (see `balance::servo_bias_targets`).
    /// Q8 scale, so the pose leans back toward support after a shove.
    pub fn set_bias(&mut self, bias: &CaptureBias) {
        self.bias_ankle_q8 = bias.ankle_q8;
        self.bias_hip_q8 = bias.hip_q8;
    }

    /// Ask for a recovery step: `mm` of extra forward travel, spread over
    /// [`STEP_OUT_TICKS`] ticks with a small hop, so the body steps out of a
    /// shove instead of gliding with both feet planted. Clamped to
    /// [`STEP_OUT_MAX_MM`] and additive with a step still owed.
    pub fn step_out(&mut self, mm: i32) {
        let wanted = self.step_extra_mm.saturating_add(mm);
        self.step_extra_mm = wanted.clamp(-STEP_OUT_MAX_MM, STEP_OUT_MAX_MM);
        self.step_left = STEP_OUT_TICKS;
    }

    /// Advance one fixed tick at the commanded speed and turn.
    ///
    /// With speed the blend eases toward walking, the phase advances by
    /// `speed / REF_SPEED_MM_S` whole phase units, the facing turns by the
    /// clamped turn, and the root travels by exactly the planted foot's own
    /// backward travel — that is what keeps a stance foot still. With no speed
    /// the blend eases back to the stand, the phase freezes and the root stops.
    pub fn advance(&mut self, speed: i32, turn_q10: i32) {
        let speed = speed.clamp(0, MAX_SPEED_MM_S);
        self.speed_mm_s = speed;
        self.turn_q10 = turn_q10.clamp(-self.params.max_turn_q10, self.params.max_turn_q10);
        // The pose the caller last saw used this walk fraction, so the travel
        // this tick owes the planted foot starts there.
        let walk_before = self.walk_frac();
        if speed == 0 {
            // Standing still: the pose eases back to rest, nothing travels.
            self.blend = self.blend.saturating_sub(1);
            self.step_extra_mm = 0;
            self.step_left = 0;
            return;
        }
        self.blend = self.blend.saturating_add(1).min(self.params.blend_window());
        let walk = self.walk_frac();
        let cycle_q8 = u32::from(self.params.cycle_ticks()) * 256;
        let old = self.phase_q8;
        self.phase_q8 = (old + phase_rate_q8(speed)) % cycle_q8;
        let planted = self.plant_advance_mm(old, self.phase_q8, walk_before, walk);
        let advance = planted + self.step_share_mm();
        let face = self.face_q15;
        let step = i64::from(advance);
        // Travel along the facing: a Q15 dot keeps this integer.
        self.travel_mm[0] += (step * i64::from(face[0])) >> 15;
        self.travel_mm[1] += (step * i64::from(face[1])) >> 15;
        if self.turn_q10 != 0 {
            self.turn_heading(self.turn_q10);
        }
    }

    /// Walking pose at the cycle's own phase (the tick path: keeps the sub-unit
    /// phase, so a slow walk moves smoothly between whole phase units).
    pub fn pose_now(&self, skeleton: &Skeleton) -> LocalPose {
        let mut out = self.rest.clone();
        self.write_pose(skeleton, self.phase_q8, self.speed_mm_s, &mut out);
        out
    }

    /// Walking pose at an explicit whole phase: the pure surface the arena and
    /// the accept's fixture read. The root still reads the cycle's travel (a
    /// plant is history, not a phase), so the same phase twice on the same
    /// cycle gives the same bytes. `speed` only scales the swing that trails
    /// the body (arms, toes), so a slow walk still plants the same feet.
    pub fn pose(&self, skeleton: &Skeleton, phase: u16, speed: i32) -> LocalPose {
        let mut out = self.rest.clone();
        let cycle_q8 = u32::from(self.params.cycle_ticks()) * 256;
        let want = u32::from(phase) * 256;
        let q8 = if want >= cycle_q8 { 0 } else { want };
        self.write_pose(skeleton, q8, speed, &mut out);
        out
    }

    /// Walking pose into caller buffers, no allocation once the buffers hold
    /// the rest pose at skeleton length.
    pub fn pose_into(&self, skeleton: &Skeleton, out: &mut LocalPose) {
        if out.positions.len() != self.rest.positions.len()
            || out.rotations.len() != self.rest.rotations.len()
        {
            out.positions.clear();
            out.positions.extend_from_slice(&self.rest.positions);
            out.rotations.clear();
            out.rotations.extend_from_slice(&self.rest.rotations);
        }
        self.write_pose(skeleton, self.phase_q8, self.speed_mm_s, out);
    }

    /// Canonical hash over the phase, blend, travel, facing and recovery step.
    /// Every field is integer, so twin runs hash equal without a float
    /// canonicalization step.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(self.phase_q8);
        h.write_i32(self.speed_mm_s);
        h.write_i32(self.turn_q10);
        h.write_u16(self.blend);
        h.write_i64(self.travel_mm[0]);
        h.write_i64(self.travel_mm[1]);
        h.write_i32(self.face_q15[0]);
        h.write_i32(self.face_q15[1]);
        h.write_i32(self.step_extra_mm);
        h.write_u16(self.step_left);
        h.write_i32(i32::from(self.bias_ankle_q8));
        h.write_i32(i32::from(self.bias_hip_q8));
        h.finish()
    }

    /// Walk fraction, `0` standing through `1` walking.
    fn walk_frac(&self) -> f32 {
        f32::from(self.blend) / f32::from(self.params.blend_window())
    }

    /// Table step for a Q8 phase, in Q8 step units `0..TURN_Q8`: integer, so
    /// the table index and its fraction stay exact and no float picks an entry.
    fn idx_q8(&self, phase_q8: u32) -> u32 {
        let cycle_q8 = u32::from(self.params.cycle_ticks()) * 256;
        let turned = phase_q8 % cycle_q8;
        let scaled = u64::from(turned) * u64::from(TURN_Q8) / u64::from(cycle_q8);
        u32::try_from(scaled).unwrap_or(0)
    }

    /// Extra travel this tick for a recovery step still owed, millimetres.
    fn step_share_mm(&mut self) -> i32 {
        if self.step_left == 0 || self.step_extra_mm == 0 {
            self.step_extra_mm = 0;
            self.step_left = 0;
            return 0;
        }
        let left = i32::from(self.step_left.max(1));
        let share = self.step_extra_mm / left;
        self.step_extra_mm -= share;
        self.step_left -= 1;
        if self.step_left == 0 {
            // The last tick carries the truncation remainder, so no step is
            // silently dropped.
            let rest = self.step_extra_mm;
            self.step_extra_mm = 0;
            return share + rest;
        }
        share
    }

    /// Hop the recovery step adds to the root height this tick, millimetres:
    /// a triangle over the spread, peaking in the middle.
    fn step_hop_mm(&self) -> f32 {
        if self.step_left == 0 {
            return 0.0;
        }
        let span = i32::from(STEP_OUT_TICKS);
        let left = i32::from(self.step_left);
        let shape = 2 * (span - left).min(left);
        // Integer millimetres: the pose only needs the unit change to metres.
        int_f32(STEP_OUT_HOP_MM * shape / span)
    }

    /// Rotate the heading by one tick's turn (Q10 radians), integer Q15.
    fn turn_heading(&mut self, turn_q10: i32) {
        let (sin_d, cos_d) = small_q15(turn_q10);
        let (sin_h, cos_h) = small_q15(turn_q10 / 2);
        self.face_q15 = rotate_q15(self.face_q15, sin_d, cos_d);
        self.half_q15 = rotate_q15(self.half_q15, sin_h, cos_h);
    }

    /// Root travel one tick gives: the planted leg's own backward foot move
    /// between two phases, as positive forward millimetres.
    ///
    /// Both ends carry their own walk fraction, so the blend that opens the
    /// stride ramps the travel with it: the planted foot holds its world place
    /// from the first tick, instead of skating the amplitude's growth.
    /// The tick splits at every whole table step, so a stance switch inside one
    /// tick lets each foot carry the body only while that foot is planted.
    /// Reading one foot across the switch counted the other foot's swing as
    /// travel, and that is what made a stance foot slide at speed.
    fn plant_advance_mm(&self, from_q8: u32, to_q8: u32, walk_from: f32, walk_to: f32) -> i32 {
        if (self.lengths[0] <= 0.0 && self.lengths[1] <= 0.0) || from_q8 == to_q8 {
            return 0;
        }
        let start = self.idx_q8(from_q8);
        let mut a = start;
        let mut b = self.idx_q8(to_q8);
        if b <= a {
            b += TURN_Q8;
        }
        let span = b - start;
        let mut total = 0.0f32;
        while a < b {
            let next = ((a / STEP_Q8) + 1) * STEP_Q8;
            let next = next.min(b);
            let left = stance_left(((a + next) / 2) % TURN_Q8);
            let from_x = foot_x_mm(
                &self.params,
                self.lengths,
                a,
                left,
                blend_walk(walk_from, walk_to, a - start, span),
            );
            let to_x = foot_x_mm(
                &self.params,
                self.lengths,
                next,
                left,
                blend_walk(walk_from, walk_to, next - start, span),
            );
            total += from_x - to_x;
            a = next;
        }
        // One rounding per tick, into the integer millimetres the root keeps.
        // The core's conversion carries the magnitude exactly, no cast needed.
        let rounded = total.round();
        let magnitude = saturating_f64_to_u64(f64::from(rounded.abs()));
        let millimetres = i32::try_from(magnitude).unwrap_or(i32::MAX);
        if rounded < 0.0 {
            -millimetres
        } else {
            millimetres
        }
    }

    /// Balance bias as a `(hip, ankle)` radian pair.
    fn bias(&self) -> [f32; 2] {
        let scale = |q8: i16| f32::from(q8) / 128.0;
        [
            scale(self.bias_hip_q8) * BIAS_HIP_RAD,
            scale(self.bias_ankle_q8) * BIAS_ANKLE_RAD,
        ]
    }

    /// Write the whole walk pose for one Q8 phase into `out`.
    fn write_pose(&self, skeleton: &Skeleton, phase_q8: u32, speed_mm_s: i32, out: &mut LocalPose) {
        let slots = Slots::find(skeleton);
        let walk = self.walk_frac();
        let speed_frac = int_f32(speed_mm_s.clamp(0, MAX_SPEED_MM_S)) / int_f32(REF_SPEED_MM_S);
        let bias = self.bias();
        let idx = self.idx_q8(phase_q8);
        let left = leg_angles(&self.params, idx, true, walk, speed_frac, bias);
        let right = leg_angles(&self.params, idx, false, walk, speed_frac, bias);
        let plant = if stance_left(idx) { left } else { right };
        let reach = leg_reach(self.lengths, plant);
        // The stance foot keeps its world height: the pelvis drops by exactly
        // what the spread legs shortened, so the contact does not hover.
        let drop = self.lengths[0] + self.lengths[1] - reach;
        let bob = sin_step_q8(idx + 32 * STEP_Q8);
        let bob_m = int_f32(self.params.pelvis_bob_mm) * bob * walk / 1000.0;
        let hop_m = self.step_hop_mm() / 1000.0;
        if let Some(p) = slots.pelvis {
            let base = self.rest.positions.get(p).copied().unwrap_or([0.0; 3]);
            out.positions[p] = [
                base[0] + mm_to_m(self.travel_mm[0]),
                base[1] - drop + bob_m + hop_m,
                base[2] + mm_to_m(self.travel_mm[1]),
            ];
            out.rotations[p] = yaw_quat(self.half_q15);
        }
        for side in 0..2 {
            let angles = if side == 0 { left } else { right };
            if let Some(b) = slots.thigh[side] {
                out.rotations[b] = pitch_quat(angles.hip);
            }
            if let Some(b) = slots.shin[side] {
                out.rotations[b] = pitch_quat(angles.knee);
            }
            if let Some(b) = slots.foot[side] {
                out.rotations[b] = pitch_quat(angles.ankle);
            }
            if let Some(b) = slots.arm[side] {
                // The arms swing against the same-side leg, and only as fast
                // as the body walks.
                let lag = if side == 0 { 0 } else { 32 * STEP_Q8 };
                let arm = -self.params.arm_rad() * sin_step_q8(idx + lag) * walk * speed_frac;
                out.rotations[b] = pitch_quat(arm);
            }
        }
    }
}

/// One leg's joint angles at a table step: hip from the sine table, knee flex
/// only in swing (zero at both stance ends), ankle levelling the foot.
fn leg_angles(
    params: &GaitParams,
    idx_q8: u32,
    left: bool,
    walk: f32,
    speed_frac: f32,
    bias: [f32; 2],
) -> LegAngles {
    let lag = if left { 0 } else { 32 * STEP_Q8 };
    let sin = sin_step_q8(idx_q8 + lag);
    // The knee flex is a sharp hump: a plain cosine still bends the knee 17 deg
    // one step before the plant, so straightening it snapped the foot 144 mm
    // forward in one tick at the plant. Cubed, the knee is near straight over
    // the last steps of swing and that snap is 36 mm: the foot arrives under
    // its own speed instead of a jump.
    let bend = cos_step_q8(idx_q8 + lag).max(0.0);
    let flex = bend * bend * bend;
    let hip = params.hip_rad() * sin * walk + bias[0];
    let knee = -params.knee_rad() * flex * walk;
    let ankle = -(hip + knee) + params.ankle_rad() * flex * walk * speed_frac + bias[1];
    LegAngles { hip, knee, ankle }
}

/// True while the left leg is in stance: the half cycle where its hip sweeps
/// from front to back. The right leg is in stance the other half.
fn stance_left(idx_q8: u32) -> bool {
    (16 * STEP_Q8..48 * STEP_Q8).contains(&idx_q8)
}

/// Vertical reach of a leg, meters: hip-to-foot height drop at these angles.
fn leg_reach(lengths: [f32; 2], a: LegAngles) -> f32 {
    let shin = a.hip + a.knee;
    lengths[0] * cos_t(a.hip) + lengths[1] * cos_t(shin)
}

/// Walk fraction at one point of a tick's phase span, linear between the two
/// ends: a split boundary reads the amplitude it is at, so the segments still
/// telescope into one exact plant.
fn blend_walk(walk_from: f32, walk_to: f32, done: u32, span: u32) -> f32 {
    if span == 0 {
        return walk_to;
    }
    let q = f32_from_u64(u64::from(done)) / f32_from_u64(u64::from(span));
    walk_from + (walk_to - walk_from) * q
}

/// Foot centre x offset from the hip joint, millimetres, at a table step.
fn foot_x_mm(params: &GaitParams, lengths: [f32; 2], idx_q8: u32, left: bool, walk: f32) -> f32 {
    let a = leg_angles(params, idx_q8, left, walk, 1.0, [0.0, 0.0]);
    let shin = a.hip + a.knee;
    (lengths[0] * sin_t(a.hip) + lengths[1] * sin_t(shin)) * 1000.0
}

/// Q8 phase units one tick advances at this speed: the cadence scales linearly
/// from the reference, `speed / REF_SPEED_MM_S` whole phase units.
fn phase_rate_q8(speed: i32) -> u32 {
    let speed = i64::from(speed.clamp(0, MAX_SPEED_MM_S));
    let rate = speed * 256 / i64::from(REF_SPEED_MM_S);
    u32::try_from(rate).unwrap_or(0)
}

/// Table sine at a Q8 step, linear between neighbouring entries.
fn sin_step_q8(idx_q8: u32) -> f32 {
    let turned = idx_q8 % TURN_Q8;
    let i = u16::try_from(turned / STEP_Q8).unwrap_or(0);
    let frac = f32::from(u8::try_from(turned % STEP_Q8).unwrap_or(0)) / STEP_F;
    let a = sin_step(i);
    let b = sin_step(i.wrapping_add(1));
    a + (b - a) * frac
}

/// Table cosine at a Q8 step: the sine a quarter turn on.
fn cos_step_q8(idx_q8: u32) -> f32 {
    sin_step_q8(idx_q8 + TURN_Q8 / 4)
}

/// One table entry as a float in `[-1, 1]`.
fn sin_step(i: u16) -> f32 {
    f32::from(SIN_Q15[usize::from(i & 63)]) / int_f32(Q15_ONE)
}

/// Q15 sine and cosine of a small angle in Q10 radians, integer: two
/// polynomial terms, exact to well under one Q15 step at a tick's turn.
fn small_q15(angle_q10: i32) -> (i32, i32) {
    let one = i64::from(Q15_ONE);
    let d = i64::from(Q10_ONE);
    let a = i64::from(angle_q10.clamp(-Q10_ONE, Q10_ONE));
    let sin = a * one / d - a * a * a * one / (6 * d * d * d);
    let cos = one - a * a * one / (2 * d * d);
    (
        i32::try_from(sin).unwrap_or(0),
        i32::try_from(cos).unwrap_or(Q15_ONE),
    )
}

/// Rotate a Q15 `[cos, sin]` pair by a Q15 sine/cosine step, integer.
fn rotate_q15(v: [i32; 2], sin_d: i32, cos_d: i32) -> [i32; 2] {
    let c = i64::from(v[0]);
    let s = i64::from(v[1]);
    let sd = i64::from(sin_d);
    let cd = i64::from(cos_d);
    let one = i64::from(Q15_ONE);
    [
        i32::try_from((c * cd - s * sd) / one).unwrap_or(v[0]),
        i32::try_from((s * cd + c * sd) / one).unwrap_or(v[1]),
    ]
}

/// Unit quaternion for a pitch about +Z (a limb swinging forward): the
/// half-angle sine and cosine come from the same short polynomials, so no libm
/// enters the tick. The one `sqrt` normalizes the pair.
fn pitch_quat(angle: f32) -> [f32; 4] {
    let h = 0.5 * angle;
    let s = sin_t(h);
    let c = cos_t(h);
    let n = (s * s + c * c).sqrt();
    [0.0, 0.0, s / n, c / n]
}

/// Unit quaternion for the heading: the half heading's Q15 pair is the
/// quaternion's `y`/`w`, negated so +X maps onto the heading in `(x, z)`.
fn yaw_quat(half_q15: [i32; 2]) -> [f32; 4] {
    let c = int_f32(half_q15[0]) / int_f32(Q15_ONE);
    let s = int_f32(half_q15[1]) / int_f32(Q15_ONE);
    let n = (s * s + c * c).sqrt();
    [0.0, -s / n, 0.0, c / n]
}

/// A non-negative integer as a float, through the core's u64 conversion: the
/// pose's tuning values, table scale and travel are small, so it is exact.
fn int_f32(v: i32) -> f32 {
    f32_from_u64(u64::from(u32::try_from(v.max(0)).unwrap_or(0)))
}

/// Integer millimetres to metres, keeping the sign: the magnitude takes the
/// exact u64 conversion, so the unit change is the only float step.
fn mm_to_m(mm: i64) -> f32 {
    let metres = f32_from_u64(mm.unsigned_abs()) / 1000.0;
    if mm < 0 {
        -metres
    } else {
        metres
    }
}

/// Sine of `x` radians (|x| up to about 1.5) from a short polynomial: no libm.
fn sin_t(x: f32) -> f32 {
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0)))
}

/// Cosine of `x` radians (|x| up to about 1.5) from a short polynomial.
fn cos_t(x: f32) -> f32 {
    let x2 = x * x;
    1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::{validate_pose, Joint};

    /// The one fixture rig: a biped the walk names every bone of.
    fn biped() -> Skeleton {
        let names: [(&str, Option<usize>); 11] = [
            ("pelvis", None),
            ("spine", Some(0)),
            ("head", Some(1)),
            ("thigh_l", Some(0)),
            ("shin_l", Some(3)),
            ("foot_l", Some(4)),
            ("thigh_r", Some(0)),
            ("shin_r", Some(6)),
            ("foot_r", Some(7)),
            ("arm_l", Some(1)),
            ("arm_r", Some(1)),
        ];
        Skeleton {
            joints: names
                .iter()
                .map(|(name, parent)| Joint {
                    name: (*name).into(),
                    parent: *parent,
                })
                .collect(),
        }
    }

    /// Rest pose: hips at 0.95 m, 0.45 m segments, feet 0.05 m off the floor.
    fn biped_rest() -> LocalPose {
        LocalPose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [0.0, 0.25, 0.0],
                [0.0, 0.25, 0.0],
                [0.0, -0.05, 0.09],
                [0.0, -0.45, 0.0],
                [0.0, -0.45, 0.0],
                [0.0, -0.05, -0.09],
                [0.0, -0.45, 0.0],
                [0.0, -0.45, 0.0],
                [0.0, 0.15, 0.16],
                [0.0, 0.15, -0.16],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 11],
        }
    }

    fn cycle() -> GaitCycle {
        GaitCycle::new(GaitParams::default(), &biped(), biped_rest())
    }

    /// Rotate a vector by a unit quaternion.
    fn rotate(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
        let u = [q[0], q[1], q[2]];
        let w = q[3];
        let uv = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let uuv = [
            u[1] * uv[2] - u[2] * uv[1],
            u[2] * uv[0] - u[0] * uv[2],
            u[0] * uv[1] - u[1] * uv[0],
        ];
        [
            v[0] + 2.0 * (w * uv[0] + uuv[0]),
            v[1] + 2.0 * (w * uv[1] + uuv[1]),
            v[2] + 2.0 * (w * uv[2] + uuv[2]),
        ]
    }

    /// The fixture's foot slots, `[left, right]`.
    fn foot_slots() -> [usize; 2] {
        [5, 8]
    }

    #[test]
    fn gait_table_is_one_sine_period() {
        // The table is the tick's only trig source: it must be a real sine
        // period at Q15, symmetric and starting at zero.
        assert_eq!(SIN_Q15[0], 0);
        assert_eq!(SIN_Q15[16], 32767);
        assert_eq!(SIN_Q15[32], 0);
        assert_eq!(SIN_Q15[48], -32767);
        for i in 1..64 {
            assert_eq!(SIN_Q15[i], -SIN_Q15[(i + 32) & 63], "half period at {i}");
        }
        // Linear interpolation never overshoots the entries it joins.
        for i in 0..64u16 {
            let a = u32::from(i) * STEP_Q8 + STEP_Q8 / 4;
            let got = sin_step_q8(a);
            let lo = sin_step(i).min(sin_step(i + 1));
            let hi = sin_step(i).max(sin_step(i + 1));
            assert!(
                got >= lo - 1e-6 && got <= hi + 1e-6,
                "step {i} stays in range"
            );
        }
        // The Q8 units are exactly one table step and one turn.
        assert_eq!(TURN_Q8, u32::from(PHASE_STEPS) * STEP_Q8);
        assert_eq!(STEP_F, f32::from(u16::try_from(STEP_Q8).unwrap_or(0)));
    }

    #[test]
    fn gait_stride_matches_the_reference_speed() {
        // Stride per cycle is 4 * leg * sin(hip): 1.0 m on 0.9 m legs at 16.2
        // deg, which over a 20-tick cycle is 3 m/s, the reference speed.
        let mut c = cycle();
        for _ in 0..c.params().blend_ticks {
            c.advance(REF_SPEED_MM_S, 0);
        }
        let start = c.travel_mm();
        for _ in 0..c.params().cycle_ticks_at_3mps {
            c.advance(REF_SPEED_MM_S, 0);
        }
        let travel = c.travel_mm()[0] - start[0];
        assert!(
            (travel - 1005).abs() <= 40,
            "one cycle must travel 1.0 m, got {travel} mm"
        );
        assert!(c.travel_mm()[1].abs() <= 1, "no side travel without turn");
    }

    #[test]
    fn gait_pose_holds_the_stance_foot_still() {
        // The row's kill criterion: a stance foot must not slide more than
        // 100 mm at 3 m/s while the body walks. The pose is what the servo
        // tracks, so measure the planted foot there: per tick, and over a
        // whole stance from its plant to its toe-off.
        let mut c = cycle();
        let skel = biped();
        // Both feet, from each stance's first plant tick (so the stand blend
        // is inside the measure) to its toe-off.
        let mut last: [Option<f32>; 2] = [None, None];
        let mut plant: [Option<f32>; 2] = [None, None];
        let mut worst_tick = 0.0f32;
        let mut worst_stance = 0.0f32;
        for _ in 0..96 {
            c.advance(REF_SPEED_MM_S, 0);
            let pose = c.pose(&skel, c.phase(), c.speed_mm_s());
            // The rig's own compose, the same one the ragdoll reports: a local
            // rotation multiplies its parent's.
            let (world, _) = crate::ragdoll::world_transforms(&skel, &pose);
            let stance = c.foot_stance();
            for (side, foot) in foot_slots().into_iter().enumerate() {
                if !stance[side] {
                    last[side] = None;
                    plant[side] = None;
                    continue;
                }
                let x = world[foot][0];
                if let Some(was) = last[side] {
                    worst_tick = worst_tick.max((x - was).abs());
                }
                let anchor = *plant[side].get_or_insert(x);
                worst_stance = worst_stance.max((x - anchor).abs());
                last[side] = Some(x);
            }
        }
        println!(
            "pose stance foot: worst tick {:.1} mm, worst stance {:.1} mm",
            worst_tick * 1000.0,
            worst_stance * 1000.0
        );
        assert!(
            worst_tick < 3.0,
            "stance foot slid {:.1} mm in one tick",
            worst_tick * 1000.0
        );
        assert!(
            worst_stance <= 0.100,
            "stance foot slid {:.1} mm across one stance at 3 m/s (kill bar 100)",
            worst_stance * 1000.0
        );
    }

    /// The rest pose shifted by the cycle's own travel: what standing looks
    /// like after a walk (the root stops where it walked to).
    fn stand_pose(c: &GaitCycle) -> LocalPose {
        let mut want = biped_rest();
        want.positions[0][0] += c.travel_mm()[0] as f32 / 1000.0;
        want.positions[0][2] += c.travel_mm()[1] as f32 / 1000.0;
        want
    }

    #[test]
    fn gait_walk_blends_in_and_back_to_stand() {
        let mut c = cycle();
        assert!(c.standing());
        assert_eq!(
            c.pose(&biped(), 0, 0),
            biped_rest(),
            "stand is the rest pose"
        );
        for _ in 0..4 {
            c.advance(REF_SPEED_MM_S, 0);
        }
        assert!(!c.standing());
        assert_eq!(c.blend(), 4);
        // A stop reaches the stand inside the accept's 30-tick budget.
        let mut ticks = 0;
        while !c.standing() {
            c.advance(0, 0);
            ticks += 1;
            assert!(ticks <= 30, "stand must arrive inside 30 ticks");
        }
        assert_eq!(c.blend(), 0);
        assert_eq!(
            c.pose(&biped(), 0, 0),
            stand_pose(&c),
            "standing pose is the rest pose again"
        );
    }

    #[test]
    fn gait_stop_freezes_the_root() {
        let mut c = cycle();
        for _ in 0..48 {
            c.advance(REF_SPEED_MM_S, 0);
        }
        let travelled = c.travel_mm()[0];
        assert!(travelled > 0, "the walk must move the root");
        while !c.standing() {
            c.advance(0, 0);
        }
        assert_eq!(
            c.travel_mm()[0],
            travelled,
            "a stopped cycle travels no further"
        );
        assert_eq!(c.pose(&biped(), c.phase(), 0), stand_pose(&c));
    }

    #[test]
    fn gait_pose_passes_the_animation_contract() {
        // The pose feeds the powered ragdoll, which validates unit quats.
        let mut c = cycle();
        let skel = biped();
        for _ in 0..12 {
            c.advance(REF_SPEED_MM_S, 3);
            validate_pose(&skel, &c.pose(&skel, c.phase(), c.speed_mm_s())).expect("unit pose");
        }
    }

    #[test]
    fn gait_turn_heads_where_the_quat_points() {
        // A turning walk must keep the pelvis quaternion pointed along the
        // heading the travel uses, or the body walks sideways.
        let mut c = cycle();
        for _ in 0..10 {
            c.advance(REF_SPEED_MM_S, 24);
        }
        let face = c.facing_q15();
        let q = yaw_quat(c.half_q15);
        let x = rotate([1.0, 0.0, 0.0], q);
        let want = [
            face[0] as f32 / Q15_ONE as f32,
            face[1] as f32 / Q15_ONE as f32,
        ];
        assert!(
            (x[0] - want[0]).abs() < 0.01 && (x[2] - want[1]).abs() < 0.01,
            "quat {x:?} must match heading {want:?}"
        );
        assert!(c.travel_mm()[1] > 0, "a left turn travels toward +z");
    }

    #[test]
    fn gait_step_out_adds_travel_and_lifts_both_feet() {
        // A recovery step must move the body further than the walk alone and
        // rise off the floor on the way, or the shove would glide.
        let mut c = cycle();
        let skel = biped();
        for _ in 0..12 {
            c.advance(REF_SPEED_MM_S, 0);
        }
        let before = c.travel_mm()[0];
        c.step_out(150);
        assert_eq!(c.step_extra_mm(), 150);
        let mut peak = 0.0f32;
        let rest_y = c.rest().positions[0][1];
        for _ in 0..STEP_OUT_TICKS {
            c.advance(REF_SPEED_MM_S, 0);
            let pose = c.pose(&skel, c.phase(), c.speed_mm_s());
            peak = peak.max(pose.positions[0][1] - rest_y);
        }
        assert_eq!(c.step_extra_mm(), 0);
        let walked = c.travel_mm()[0] - before;
        assert!(
            walked > 150,
            "the step must add its travel, got {walked} mm"
        );
        assert!(peak > 0.01, "the step must lift the root, peak {peak} m");
    }

    #[test]
    fn gait_replay_is_byte_identical() {
        let run = || {
            let mut c = cycle();
            let skel = biped();
            let mut h = Fnv64::new();
            for tick in 0..120 {
                c.advance(REF_SPEED_MM_S, if tick < 60 { 0 } else { -12 });
                let pose = c.pose(&skel, c.phase(), c.speed_mm_s());
                for p in pose.positions.iter().flatten() {
                    h.write_f32(*p);
                }
                for q in pose.rotations.iter().flatten() {
                    h.write_f32(*q);
                }
                h.write_u64(c.state_hash());
            }
            (h.finish(), c)
        };
        let (h1, c1) = run();
        let (h2, c2) = run();
        assert_eq!(h1, h2);
        assert_eq!(c1, c2);
    }
}
