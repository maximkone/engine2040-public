//! balance: center-of-mass balance core plus stagger, stumble and fall.
//!
//! Consumer: the arena hit pipeline observes bone centers plus masses every
//! tick and feeds each landed shove through [`BalanceState`]; on a staggered
//! verdict the rapier-gated [`BalanceState::strike`] frees the struck subtree
//! on the powered ragdoll servo, while heavier blows escalate through
//! [`BalanceState::strike_gait`] into a stumble gait, a full-body fall, an
//! arm-catch and the powered-ragdoll 60-tick get-up. Owner: this module owns
//! [`BalanceState`], [`BalanceParams`], [`BalanceVerdict`], [`GaitVerdict`],
//! [`GaitPhase`] and [`BalanceError`]. Units: meters,
//! kilograms, Q8 impulse units where 256 reads unity, ticks at 60 Hz.
//! Determinism: stable bone order with `+ - * /` and comparisons only, no
//! clock and no order-dependent maps; one event sequence hashes one way.
//! The gait phase plus its countdowns hash with the stagger core; servo pose
//! reads (`bone_poses`, `blend_weight`) never do. Errors: [`BalanceError`]
//! (plus [`ServoError`] with the servo feature),
//! never a panic on caller data. Budget: the tick path writes caller-owned
//! buffers plus the state struct only and allocates nothing; staggering and
//! heavier strikes take one mask buffer each at event rate, never per tick.
#![forbid(unsafe_code)]

use engine_core::Fnv64;
use std::fmt::{Display, Formatter};

/// Stagger recovery window, ticks at 60 Hz: a staggered fighter answers again
/// half a second after the shove unless a later extension holds the state.
pub const STAGGER_TICKS: u32 = 30;
/// Stumble gait window, ticks at 60 Hz: a stumbling fighter steps out the
/// blow for three quarters of a second, longer than a stagger, before the
/// stance settles. Presentation reads [`BalanceState::gait`] to step; the
/// window itself is hashed sim state.
pub const STUMBLE_TICKS: u32 = 45;
/// Prone window, ticks at 60 Hz: a fallen fighter lies fully ragdolled for
/// half a second before the arms drive into the catch.
pub const FALL_PRONE_TICKS: u32 = 30;
/// Catch window, ticks at 60 Hz: the arms track the animated reach-out while
/// the body stays down; on expiry the caller starts the powered-ragdoll get-up through
/// [`BalanceState::rise`], which blends back within 60 ticks.
pub const CATCH_TICKS: u32 = 15;
/// Stumble offset above the stagger threshold, Q8 impulse units: with the
/// default tuning the third of five evenly spaced shove strengths is the
/// first that stumbles.
pub const STUMBLE_ABOVE_STAGGER_Q8: u16 = 128;
/// Fall offset above the stagger threshold, Q8 impulse units: with the
/// default tuning the fifth of five evenly spaced shove strengths falls.
/// Tunings that push this past the impulse ceiling hold stagger-only, since
/// over-ceiling blows are refused rather than escalated.
pub const FALL_ABOVE_STAGGER_Q8: u16 = 384;
/// Recovery budget, ticks at 60 Hz: every shove response reaches a legal
/// stance again within this window. It mirrors the powered-ragdoll get-up
/// budget the servo proves under the rapier feature, so the sim side never
/// owes more ticks than the servo needs to stand back up.
pub const RECOVERY_BUDGET_TICKS: u32 = 60;
/// Five standard shove strengths, Q8 impulse units: with the default tuning
/// they read steady, stagger, stumble, stumble, fall in order. Harnesses and
/// the arena hit pipeline share this one table so every five-strength proof
/// pushes the same blows.
pub const RESPONSE_Q8: [u16; 5] = [128, 256, 384, 512, 640];
/// Default stagger threshold, Q8 impulse units: unity, so the second of five
/// evenly spaced shove strengths is the first that staggers.
pub const DEFAULT_STAGGER_Q8: u16 = 256;
/// Widest accepted shove impulse, Q8 units (sixteen times unity): past this
/// the caller mis-scaled, so the shove is refused and the state is unchanged.
pub const MAX_IMPULSE_Q8: u16 = 4096;
/// Position bound, meters: a bone center past this in any component is
/// refused, matching the adapter crates' thousand-meter fixturing ceiling.
pub const MAX_BONE_POS_M: f32 = 1000.0;
/// Mass bound, kilograms: each bone mass must sit in `(0, MAX_BONE_MASS_KG]`.
pub const MAX_BONE_MASS_KG: f32 = 1_000_000.0;
/// Millimetre hash range, meters scaled by 1000 as data: the sim position
/// bound (±1000 m) fits inside with room, and the named bounds keep the
/// [`com_mm`] clamp free of inline casts. Bit patterns give the exact
/// powers of two (±2^31) with no decimal rounding.
const MM_FLOOR_F32: f32 = f32::from_bits(0xCF00_0000);
/// Ceiling of the millimetre hash range; see [`MM_FLOOR_F32`].
const MM_CEIL_F32: f32 = f32::from_bits(0x4F00_0000);
/// Shove scale, newton-seconds per unity of Q8 impulse: one unity planar push
/// carries this into the struck bone, the magnitude the servo tracking tests
/// prove displaces a bone without tunneling. Response-matrix tuning later may
/// retune this one constant against the five-strength ordering proof.
#[cfg(feature = "rapier")]
const SHOVE_NEWTON_PER_UNITY: f32 = 0.05;

/// Shove verdict: steady means the stance holds, staggered means the fighter
/// answers late for [`STAGGER_TICKS`] ticks while the servo simulates freely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BalanceVerdict {
    /// Stance holds; the servo keeps tracking the animated pose.
    Steady,
    /// Stance broken; the fighter recovers after the stagger window.
    Staggered,
}

/// Impact verdict: the blow's own level, from a light touch to a fall. This
/// reports what the blow warrants, while [`GaitPhase`] tracks what the
/// fighter is doing: impacts never downgrade an in-flight fall, so the two
/// agree except while a heavier phase is still running its course.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaitVerdict {
    /// Under the stagger threshold; stance holds.
    Steady,
    /// At or past the stagger threshold; answers late for [`STAGGER_TICKS`].
    Staggered,
    /// At or past the stumble threshold; steps the blow out for
    /// [`STUMBLE_TICKS`] ticks.
    Stumbling,
    /// At or past the fall threshold; full ragdoll, then the arm-catch and
    /// the powered-ragdoll get-up.
    Fallen,
}

/// One row of the shove-response matrix: a standard strength plus what the
/// blow warrants and how many ticks the stance needs to go legal again.
/// Owner: this module builds rows in [`response_matrix`]; the arena hit
/// pipeline and the budget harnesses read them. Units: Q8 impulse units for
/// the strength, ticks at 60 Hz for the recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseRow {
    /// Shove strength, Q8 impulse units where 256 reads unity.
    pub strength_q8: u16,
    /// What the blow warrants at this strength under the row's tuning.
    pub verdict: GaitVerdict,
    /// Ticks from the shove until the stance is legal again: zero for a
    /// steady stance, the stagger window for a stagger, the stumble gait
    /// for a stumble, prone plus catch for a fall. Never past
    /// [`RECOVERY_BUDGET_TICKS`].
    pub recovery_ticks: u32,
}

/// Recovery ticks for one impact verdict: steady answers at once, stagger
/// clears with its window, stumble steps out inside its gait, and a fall
/// lies prone plus catches before the get-up flag rises — the caller then
/// blends back inside the servo get-up budget, so the whole fall still
/// closes within [`RECOVERY_BUDGET_TICKS`]. Pure, no allocation.
pub fn recovery_ticks(verdict: GaitVerdict) -> u32 {
    match verdict {
        GaitVerdict::Steady => 0,
        GaitVerdict::Staggered => STAGGER_TICKS,
        GaitVerdict::Stumbling => STUMBLE_TICKS,
        GaitVerdict::Fallen => FALL_PRONE_TICKS + CATCH_TICKS,
    }
}

/// Shove-response matrix: the five [`RESPONSE_Q8`] strengths mapped to their
/// verdicts plus recovery ticks under these params. Pure over the
/// thresholds — no state, no allocation — so tables, harnesses and the
/// budget proof all read the same rows the state machine in
/// [`BalanceState::apply_impact`] would deal.
pub fn response_matrix(params: BalanceParams) -> [ResponseRow; 5] {
    let stumble = params.stagger_q8.saturating_add(STUMBLE_ABOVE_STAGGER_Q8);
    let fall = params.stagger_q8.saturating_add(FALL_ABOVE_STAGGER_Q8);
    RESPONSE_Q8.map(|strength_q8| {
        let verdict = if strength_q8 >= fall {
            GaitVerdict::Fallen
        } else if strength_q8 >= stumble {
            GaitVerdict::Stumbling
        } else if strength_q8 >= params.stagger_q8 {
            GaitVerdict::Staggered
        } else {
            GaitVerdict::Steady
        };
        ResponseRow {
            strength_q8,
            verdict,
            recovery_ticks: recovery_ticks(verdict),
        }
    })
}

/// Support half-width, millimetres: the stance support polygon spans this
/// each way about the support center on both planar axes. Capture error
/// inside the box needs no hurry; past it the servo leans back toward
/// support and the recovery hurries.
pub const SUPPORT_HALF_MM: i32 = 150;
/// Capture error clamp, millimetres: past this the bias holds its ceiling
/// instead of growing, so a far-flung ragdoll cannot wind the servo up.
pub const CP_MAX_ERR_MM: i32 = 500;
/// Servo bias ceiling, Q8 units: half unity of target shift per axis.
pub const SERVO_BIAS_MAX_Q8: i16 = 128;
/// Recovery credit ceiling, ticks at 60 Hz: the bias hurries a recovery by
/// at most this many ticks, so every biased row still closes inside
/// [`RECOVERY_BUDGET_TICKS`].
pub const RECOVERY_CREDIT_MAX_TICKS: u32 = 5;
/// Millimetres of capture error past the support box that earn one tick of
/// recovery credit.
pub const MM_PER_CREDIT_TICK: u32 = 40;

/// Capture-point servo bias: integer planar error plus the three servo
/// target shifts the presentation servo adds. Owner: this module builds it
/// in [`servo_bias_targets`]; [`BalanceState::strike_gait_biased`] reads it
/// through [`BalanceState::capture_bias`] on every landed blow. Units:
/// millimetres for the error, Q8 units for the shifts. Determinism: integer
/// math only, no allocation, and the bias never enters [`BalanceState`]'s
/// hash, so the same shove sequence hashes one way with or without a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureBias {
    /// Capture error, millimetres: planar capture point minus the support
    /// center, clamped into `[-CP_MAX_ERR_MM, CP_MAX_ERR_MM]` per axis.
    pub cp_err_mm: [i32; 2],
    /// Ankle target shift, Q8: opposes the sagittal (x) error at full scale.
    pub ankle_q8: i16,
    /// Hip target shift, Q8: opposes the lateral (z) error at full scale.
    pub hip_q8: i16,
    /// Stance target shift, Q8: a quarter of the combined lean, widening the
    /// stance toward support.
    pub stance_q8: i16,
}

/// Capture error from the center of mass plus the support polygon: the
/// planar center (integer millimetres through [`com_mm`]) plus a
/// shove-velocity proxy of one millimetre of capture lead per Q8 impulse
/// unit along the blow (+x), clamped per axis.
/// Pure integer math, no allocation; the servo pose never enters the hash.
pub fn capture_error_mm(com_mm: [i32; 3], impulse_q8: u16) -> [i32; 2] {
    let lead = i32::from(impulse_q8);
    let cx = com_mm[0]
        .saturating_add(lead)
        .clamp(-CP_MAX_ERR_MM, CP_MAX_ERR_MM);
    let cz = com_mm[2].clamp(-CP_MAX_ERR_MM, CP_MAX_ERR_MM);
    [cx, cz]
}

/// Servo bias targets for one capture error: each axis opposes its error at
/// full Q8 scale against the clamp, so the servo aims back toward support;
/// the stance shares a quarter of the combined lean. Pure integer math, no
/// allocation.
pub fn servo_bias_targets(cp_err_mm: [i32; 2]) -> CaptureBias {
    let ankle_q8 = oppose_axis(cp_err_mm[0]);
    let hip_q8 = oppose_axis(cp_err_mm[1]);
    let stance_q8 = (i32::from(ankle_q8) + i32::from(hip_q8)) / 4;
    // Both shifts sit inside ±128, so their quarter sits inside ±64 and the
    // narrowing never fails; the fallback never runs.
    let stance_q8 = i16::try_from(stance_q8).unwrap_or(0);
    CaptureBias {
        cp_err_mm,
        ankle_q8,
        hip_q8,
        stance_q8,
    }
}

/// Oppose one error axis at full Q8 scale: the error fraction of the clamp
/// range times the bias ceiling, negated so the servo leans back toward
/// support. Integer math only; the clamp proves the narrowing conversion.
fn oppose_axis(err_mm: i32) -> i16 {
    let ceiling = i32::from(SERVO_BIAS_MAX_Q8);
    let scaled = err_mm.saturating_mul(ceiling) / CP_MAX_ERR_MM;
    let clamped = scaled.clamp(-ceiling, ceiling);
    let leaned = 0 - clamped;
    // Clamped to ±128 above, so the narrowing never fails; the fallback
    // never runs.
    i16::try_from(leaned).unwrap_or(0)
}

/// Recovery credit for one capture error, ticks at 60 Hz: the error excess
/// past the support box (Manhattan, per axis) earns one tick per
/// [`MM_PER_CREDIT_TICK`] millimetres up to [`RECOVERY_CREDIT_MAX_TICKS`].
/// Integer math only, no allocation.
fn bias_credit_ticks(cp_err_mm: [i32; 2]) -> u32 {
    // The support half-width is a positive const, so the widening never
    // fails; the fallback never runs.
    let half = u32::try_from(SUPPORT_HALF_MM).unwrap_or(0);
    let excess = cp_err_mm[0]
        .unsigned_abs()
        .saturating_sub(half)
        .saturating_add(cp_err_mm[1].unsigned_abs().saturating_sub(half));
    (excess / MM_PER_CREDIT_TICK).min(RECOVERY_CREDIT_MAX_TICKS)
}

/// Biased recovery ticks for one impact verdict under a capture bias: the
/// baseline [`recovery_ticks`] minus the bias credit, floored at zero for a
/// steady stance (no recovery to hurry) and inside
/// [`RECOVERY_BUDGET_TICKS`] everywhere else, since the credit never passes
/// its ceiling. Pure, no allocation; the sim step path is untouched, so the
/// bias hurries the servo budget without moving any hash.
pub fn biased_recovery_ticks(verdict: GaitVerdict, bias: &CaptureBias) -> u32 {
    let base = recovery_ticks(verdict);
    if base == 0 {
        return 0;
    }
    base.saturating_sub(bias_credit_ticks(bias.cp_err_mm))
}

/// Gait phase: what the fighter's body is doing. Stagger has no phase (the
/// staggered flag covers it); stumble, fall and catch each own a countdown
/// in [`BalanceState::step`]. Getting up belongs to the servo
/// ([`RagdollMode::GettingUp`](super::powered::RagdollMode) under the rapier
/// feature); the balance side only raises [`BalanceState::getup_due`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GaitPhase {
    /// Upright, driven or staggering; no gait countdown runs.
    Steady,
    /// Stepping the blow out; clears to steady after [`STUMBLE_TICKS`].
    Stumbling,
    /// Fully down; rolls into the catch after [`FALL_PRONE_TICKS`].
    Fallen,
    /// Arms driving, body down; raises the get-up flag after [`CATCH_TICKS`].
    Catching,
}

impl GaitPhase {
    /// Stable hash code: steady reads 0 through catching reads 3, so one
    /// phase hashes one way without a float-style canonicalization step.
    const fn code(self) -> u8 {
        match self {
            GaitPhase::Steady => 0,
            GaitPhase::Stumbling => 1,
            GaitPhase::Fallen => 2,
            GaitPhase::Catching => 3,
        }
    }
}

/// What refused a balance call. Refusals change nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum BalanceError {
    /// No bones were given; the center of mass of nothing is undefined.
    EmptyBones,
    /// Position and mass counts differ; expected + got carried.
    LengthMismatch {
        /// Position count.
        expected: usize,
        /// Mass count.
        got: usize,
    },
    /// A bone center is non-finite or past the position bound; the bone index
    /// is carried.
    BadPosition(usize),
    /// A bone mass is non-finite, non-positive, past the mass bound, or the
    /// mass total degenerated; the bone index is carried, or the bone count
    /// when the total rather than one bone is at fault.
    BadMass(usize),
    /// A shove impulse sits past the accepted ceiling; the culprit is carried.
    BadImpulse(u16),
    /// A stagger threshold is zero or past the accepted ceiling; the culprit
    /// is carried.
    BadThreshold(u16),
    /// A servo bone index sits past the ragdoll; bone plus bone count carried.
    BadBone {
        /// Requested bone.
        bone: usize,
        /// Bones on the ragdoll.
        bones: usize,
    },
    /// A strike mask width differs from the ragdoll; expected + got carried.
    BadMask {
        /// Ragdoll bone count.
        expected: usize,
        /// Mask width.
        got: usize,
    },
    /// A catch or rise ran in the wrong gait phase; want + got carried.
    /// Nothing mutates then, including the get-up flag.
    WrongPhase {
        /// Phase the call needed.
        want: GaitPhase,
        /// Phase the fighter is in.
        got: GaitPhase,
    },
    /// A rise ran before the catch window expired; the get-up flag is clear.
    /// Nothing mutates then.
    GetupNotDue,
}

impl Display for BalanceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BalanceError::EmptyBones => f.write_str("balance: need at least one bone"),
            BalanceError::LengthMismatch { expected, got } => {
                write!(f, "balance: need {expected} masses, got {got}")
            }
            BalanceError::BadPosition(b) => {
                write!(f, "balance: bone {b} center is non-finite or past 1000 m")
            }
            BalanceError::BadMass(b) => {
                write!(
                    f,
                    "balance: bone {b} mass is non-finite, non-positive or past bound"
                )
            }
            BalanceError::BadImpulse(i) => {
                write!(f, "balance: impulse {i} is past {MAX_IMPULSE_Q8}")
            }
            BalanceError::BadThreshold(t) => {
                write!(f, "balance: threshold {t} is zero or past {MAX_IMPULSE_Q8}")
            }
            BalanceError::BadBone { bone, bones } => {
                write!(f, "balance: bone {bone} is past {bones} ragdoll bones")
            }
            BalanceError::BadMask { expected, got } => {
                write!(f, "balance: need {expected} mask entries, got {got}")
            }
            BalanceError::WrongPhase { want, got } => {
                write!(f, "balance: gait phase {got:?} cannot run, need {want:?}")
            }
            BalanceError::GetupNotDue => {
                f.write_str("balance: catch window has not expired, get-up not due")
            }
        }
    }
}

impl std::error::Error for BalanceError {}

/// What refused a servo strike. Strike-shape refusals leave the balance state
/// and the ragdoll both unchanged; the servo maps its own errors through.
#[cfg(feature = "rapier")]
#[derive(Clone, Debug, PartialEq)]
pub enum ServoError {
    /// The shove or the strike shape was refused; nothing mutated.
    Balance(BalanceError),
    /// The servo refused the mask or the hit after validation; the inputs are
    /// checked before either system mutates, so this arm stays for future
    /// servo rules rather than reachable failures today.
    Ragdoll(super::powered::RagdollError),
}

#[cfg(feature = "rapier")]
impl Display for ServoError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ServoError::Balance(e) => write!(f, "balance servo: {e}"),
            ServoError::Ragdoll(e) => write!(f, "balance servo: {e}"),
        }
    }
}

#[cfg(feature = "rapier")]
impl std::error::Error for ServoError {}

/// Stagger tuning: the shove impulse, in Q8 units, at or past which a fighter
/// staggers. Lower values answer lighter touches; higher values hold the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BalanceParams {
    /// Stagger threshold, Q8 impulse units where 256 reads unity.
    pub stagger_q8: u16,
}

impl Default for BalanceParams {
    /// Default tuning: unity, so the second of five evenly spaced shove
    /// strengths is the first that staggers.
    fn default() -> Self {
        Self {
            stagger_q8: DEFAULT_STAGGER_Q8,
        }
    }
}

impl BalanceParams {
    /// Build with validation: zero would stagger on contact noise and values
    /// past the impulse ceiling mis-scale, so both are refused with the
    /// culprit carried.
    pub fn new(stagger_q8: u16) -> Result<Self, BalanceError> {
        if stagger_q8 == 0 || stagger_q8 > MAX_IMPULSE_Q8 {
            return Err(BalanceError::BadThreshold(stagger_q8));
        }
        Ok(Self { stagger_q8 })
    }
}

/// Center-of-mass balance core: the last observed center of mass plus the
/// stagger countdown. Mass-weighted, order-stable, allocation-free on the
/// tick path; the stumble and fall gait rides beside the stagger core with
/// its own phase plus countdowns, and the powered-ragdoll get-up closes the fall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BalanceState {
    /// Stagger tuning.
    params: BalanceParams,
    /// Last observed center of mass, meters; origin until the first observe.
    com: [f32; 3],
    /// Whether the fighter is inside the stagger window.
    staggered: bool,
    /// Ticks left in the stagger window; zero outside it.
    stagger_left: u32,
    /// Ticks taken since spawn; folded into the hash.
    tick: u64,
    /// Applied-shove counter; folded into the hash so event order shows.
    shoves: u64,
    /// Gait phase; folded into the hash.
    gait: GaitPhase,
    /// Ticks left in the gait phase window; zero while steady.
    gait_left: u32,
    /// Fall-escalation counter; folded into the hash so falls show past the
    /// shove count.
    falls: u64,
    /// Set when the catch window expires; the caller then starts the powered-ragdoll
    /// get-up through [`BalanceState::rise`], which consumes it.
    getup_due: bool,
}

impl BalanceState {
    /// Fresh state: stance steady, center of mass at the origin until the
    /// first [`BalanceState::observe`] call.
    pub fn new(params: BalanceParams) -> Self {
        Self {
            params,
            com: [0.0; 3],
            staggered: false,
            stagger_left: 0,
            tick: 0,
            shoves: 0,
            gait: GaitPhase::Steady,
            gait_left: 0,
            falls: 0,
            getup_due: false,
        }
    }

    /// Current tuning.
    pub fn params(&self) -> BalanceParams {
        self.params
    }

    /// Last observed center of mass, meters; origin before the first observe.
    pub fn com(&self) -> [f32; 3] {
        self.com
    }

    /// Whether the fighter is inside the stagger window.
    pub fn staggered(&self) -> bool {
        self.staggered
    }

    /// Ticks left in the stagger window; zero outside it.
    pub fn stagger_remaining(&self) -> u32 {
        self.stagger_left
    }

    /// Ticks taken since spawn.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Applied-shove counter.
    pub fn shove_count(&self) -> u64 {
        self.shoves
    }

    /// Stumble threshold, Q8 impulse units: the stagger threshold plus the
    /// stumble offset, saturating. Blows at or past this stumble.
    pub fn stumble_q8(&self) -> u16 {
        self.params
            .stagger_q8
            .saturating_add(STUMBLE_ABOVE_STAGGER_Q8)
    }

    /// Fall threshold, Q8 impulse units: the stagger threshold plus the fall
    /// offset, saturating. Blows at or past this fall; a tuning that lands
    /// this past the impulse ceiling never falls.
    pub fn fall_q8(&self) -> u16 {
        self.params.stagger_q8.saturating_add(FALL_ABOVE_STAGGER_Q8)
    }

    /// Gait phase: what the fighter's body is doing. Presentation polls this
    /// after every [`BalanceState::step`] to catch once on `Catching`.
    pub fn gait(&self) -> GaitPhase {
        self.gait
    }

    /// Ticks left in the gait phase window; zero while steady.
    pub fn gait_remaining(&self) -> u32 {
        self.gait_left
    }

    /// Fall-escalation counter.
    pub fn fall_count(&self) -> u64 {
        self.falls
    }

    /// Whether the catch window expired and the powered-ragdoll get-up is due. Consumed
    /// by [`BalanceState::rise`], cleared by any fresh stumble or fall.
    pub fn getup_due(&self) -> bool {
        self.getup_due
    }

    /// Capture-point servo bias for the presentation servo: the planar
    /// center (integer millimetres) plus the shove lead, mapped to
    /// ankle/hip/stance target shifts through [`servo_bias_targets`].
    /// Read-only: the bias never enters
    /// the hash, so reads move nothing. Over-ceiling impulses are refused.
    /// Integer math only, no allocation.
    pub fn capture_bias(&self, impulse_q8: u16) -> Result<CaptureBias, BalanceError> {
        if impulse_q8 > MAX_IMPULSE_Q8 {
            return Err(BalanceError::BadImpulse(impulse_q8));
        }
        Ok(servo_bias_targets(capture_error_mm(
            com_mm(self.com),
            impulse_q8,
        )))
    }

    /// Observe one tick: validate bone centers plus masses, store their
    /// center of mass and hand it back. Counts must agree and every value
    /// must be finite inside its bound; slices in, array out, no allocation.
    pub fn observe(
        &mut self,
        positions: &[[f32; 3]],
        masses: &[f32],
    ) -> Result<[f32; 3], BalanceError> {
        let com = center_of_mass(positions, masses)?;
        self.com = com;
        Ok(com)
    }

    /// Feed one landed shove: impulses at or past the threshold stagger and
    /// open the recovery window, lighter ones hold steady. Impulses past the
    /// ceiling are refused; the counter advances on every accepted shove so
    /// the hash sees the event order.
    pub fn apply_shove(&mut self, impulse_q8: u16) -> Result<BalanceVerdict, BalanceError> {
        if impulse_q8 > MAX_IMPULSE_Q8 {
            return Err(BalanceError::BadImpulse(impulse_q8));
        }
        self.shoves = self.shoves.wrapping_add(1);
        if impulse_q8 >= self.params.stagger_q8 {
            self.staggered = true;
            self.stagger_left = STAGGER_TICKS;
            Ok(BalanceVerdict::Staggered)
        } else {
            Ok(BalanceVerdict::Steady)
        }
    }

    /// Feed one landed blow through the stagger core plus the gait: the
    /// verdict reports the blow's own level while the phase adopts it unless
    /// a heavier phase is still running, so impacts never downgrade an
    /// in-flight fall. A fresh stumble or fall clears a pending get-up flag;
    /// every fall escalation advances the fall counter. Impulses past the
    /// ceiling are refused with nothing changed. Scalar fields only, no
    /// allocation.
    pub fn apply_impact(&mut self, impulse_q8: u16) -> Result<GaitVerdict, BalanceError> {
        self.apply_shove(impulse_q8)?;
        if impulse_q8 >= self.fall_q8() {
            self.gait = GaitPhase::Fallen;
            self.gait_left = FALL_PRONE_TICKS;
            self.falls = self.falls.wrapping_add(1);
            self.getup_due = false;
            Ok(GaitVerdict::Fallen)
        } else if impulse_q8 >= self.stumble_q8() {
            if self.gait != GaitPhase::Fallen {
                self.gait = GaitPhase::Stumbling;
                self.gait_left = STUMBLE_TICKS;
                self.getup_due = false;
            }
            Ok(GaitVerdict::Stumbling)
        } else if impulse_q8 >= self.params.stagger_q8 {
            Ok(GaitVerdict::Staggered)
        } else {
            Ok(GaitVerdict::Steady)
        }
    }

    /// Feed one landed blow to the stagger core plus stagger memory: the
    /// blow's own level comes back beside what the beating warrants, so the
    /// arena hit pipeline drives one call for both. The impulse ceiling and
    /// the bone slot are checked before either system mutates, so a refusal
    /// leaves the stance and the memory both unchanged. Scalar fields only,
    /// no allocation.
    pub fn apply_memory_hit(
        &mut self,
        memory: &mut super::stagger_memory::StaggerMemory,
        impulse_q8: u16,
        bone: usize,
    ) -> Result<(GaitVerdict, super::stagger_memory::StaggerLevel), BalanceError> {
        if impulse_q8 > MAX_IMPULSE_Q8 {
            return Err(BalanceError::BadImpulse(impulse_q8));
        }
        memory.record_hit(impulse_q8, bone)?;
        let verdict = self.apply_impact(impulse_q8)?;
        Ok((verdict, memory.level()))
    }

    /// Advance one fixed tick: the stagger countdown falls toward zero and
    /// clears the staggered flag on arrival, and the gait phase runs its own
    /// window: stumble clears to steady, fall rolls into the catch, catch
    /// raises the get-up flag. Scalar fields only, no allocation.
    pub fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.staggered {
            self.stagger_left = self.stagger_left.saturating_sub(1);
            if self.stagger_left == 0 {
                self.staggered = false;
            }
        }
        if self.gait == GaitPhase::Steady {
            return;
        }
        self.gait_left = self.gait_left.saturating_sub(1);
        if self.gait_left > 0 {
            return;
        }
        match self.gait {
            GaitPhase::Steady => {}
            GaitPhase::Stumbling => self.gait = GaitPhase::Steady,
            GaitPhase::Fallen => {
                self.gait = GaitPhase::Catching;
                self.gait_left = CATCH_TICKS;
            }
            GaitPhase::Catching => {
                self.gait = GaitPhase::Steady;
                self.getup_due = true;
            }
        }
    }

    /// Whether the stance is legal: no open stagger window and no running
    /// gait phase. A raised get-up flag still counts as legal — the fall
    /// already closed and only the caller's rise through
    /// [`BalanceState::rise`] remains.
    pub fn stance_legal(&self) -> bool {
        !self.staggered && self.gait == GaitPhase::Steady
    }

    /// Drive one shove to a legal stance: applies the blow through
    /// [`BalanceState::apply_impact`], then steps until
    /// [`BalanceState::stance_legal`] holds. Returns the ticks stepped, which
    /// matches [`recovery_ticks`] for the dealt verdict on a fresh state and
    /// never passes [`RECOVERY_BUDGET_TICKS`]. An over-ceiling impulse is
    /// refused with nothing changed. Scalar fields only, no allocation; the
    /// loop always terminates because the stagger window clears within
    /// [`STAGGER_TICKS`] steps and any gait phase within [`STUMBLE_TICKS`].
    pub fn respond(&mut self, impulse_q8: u16) -> Result<u32, BalanceError> {
        self.apply_impact(impulse_q8)?;
        let mut stepped: u32 = 0;
        while !self.stance_legal() {
            self.step();
            stepped = stepped.saturating_add(1);
        }
        Ok(stepped)
    }

    /// Canonical hash over the tuning, the center of mass, the stagger flag
    /// plus countdown, the gait phase plus its countdown, the get-up flag and
    /// all three counters. The center hashes as integer millimetres through
    /// [`com_mm`], so sub-millimetre FMA or summation-order drift on another
    /// CPU cannot split the hash. Servo pose reads never enter this hash.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u16(self.params.stagger_q8);
        for mm in com_mm(self.com) {
            h.write_i32(mm);
        }
        h.write_bool(self.staggered);
        h.write_u32(self.stagger_left);
        h.write_u64(self.tick);
        h.write_u64(self.shoves);
        h.write_u8(self.gait.code());
        h.write_u32(self.gait_left);
        h.write_u64(self.falls);
        h.write_bool(self.getup_due);
        h.finish()
    }

    /// Wire one landed hit-react strike into the powered ragdoll servo: the
    /// strike impulse feeds [`BalanceState::apply_shove`], and only a stagger
    /// touches the ragdoll. A stagger unions the caller mask over the live
    /// servo mask (upper body free, stance planted, by the hit-react mask
    /// convention) and frees the struck subtree through the servo, so the hit
    /// both poses and simulates. The strike bone plus the mask width are
    /// validated before either system mutates, so a refusal leaves the state
    /// and the ragdoll both unchanged; a staggering strike interrupts an
    /// in-flight get-up, matching the servo hit rule.
    #[cfg(feature = "rapier")]
    pub fn strike(
        &mut self,
        ragdoll: &mut super::powered::PoweredRagdoll,
        strike: &engine_anim::hit_react::HitStrike,
        mask: &engine_anim::layers::MaskTable,
        tick: u64,
    ) -> Result<BalanceVerdict, ServoError> {
        let bone = strike_shape(ragdoll, strike, mask)?;
        let verdict = self
            .apply_shove(strike.impulse_q8)
            .map_err(ServoError::Balance)?;
        if verdict == BalanceVerdict::Steady {
            return Ok(verdict);
        }
        drive_stagger(ragdoll, bone, strike, mask, tick)?;
        Ok(verdict)
    }

    /// Wire one landed blow through the stagger core plus the gait: the
    /// verdict reports the blow's own level while the phase adopts it per
    /// [`BalanceState::apply_impact`]. Stagger and stumble share the servo
    /// treatment (unioned mask plus the freed struck subtree, the longer
    /// stumble window living in sim state for the stepping presentation); a
    /// fall goes fully ragdolled so the prone body can catch and rise. Shape
    /// refusals leave both systems unchanged; servo strikes always land their
    /// impulse even when the phase refuses to downgrade, and any heavier
    /// strike interrupts an in-flight get-up, matching the servo hit rule.
    #[cfg(feature = "rapier")]
    pub fn strike_gait(
        &mut self,
        ragdoll: &mut super::powered::PoweredRagdoll,
        strike: &engine_anim::hit_react::HitStrike,
        mask: &engine_anim::layers::MaskTable,
        tick: u64,
    ) -> Result<GaitVerdict, ServoError> {
        let bone = strike_shape(ragdoll, strike, mask)?;
        let verdict = self
            .apply_impact(strike.impulse_q8)
            .map_err(ServoError::Balance)?;
        match verdict {
            GaitVerdict::Steady => Ok(verdict),
            GaitVerdict::Staggered | GaitVerdict::Stumbling => {
                drive_stagger(ragdoll, bone, strike, mask, tick)?;
                Ok(verdict)
            }
            GaitVerdict::Fallen => {
                // Impulse into the struck bone first, then the full mask: the
                // prone body simulates whole instead of one subtree.
                let push = strike.dir.push();
                let scale = impact_scale(strike.impulse_q8);
                let impulse = [push[0] * scale, 0.0, push[1] * scale];
                ragdoll
                    .apply_hit(bone, impulse, tick)
                    .map_err(ServoError::Ragdoll)?;
                let downed = vec![true; ragdoll.bone_count()];
                ragdoll.set_mask(&downed).map_err(ServoError::Ragdoll)?;
                Ok(verdict)
            }
        }
    }

    /// Landed-blow entry with capture bias: runs [`BalanceState::strike_gait`]
    /// for the servo treatment, then reads [`BalanceState::capture_bias`] for
    /// the same blow so every landed hit returns its ankle/hip/stance target
    /// shifts beside the verdict. Read-only after the strike: the bias never
    /// enters the hash, so a biased strike hashes exactly like the plain one.
    /// Scalar fields only, no allocation; event rate only, never the tick path.
    #[cfg(feature = "rapier")]
    pub fn strike_gait_biased(
        &mut self,
        ragdoll: &mut super::powered::PoweredRagdoll,
        strike: &engine_anim::hit_react::HitStrike,
        mask: &engine_anim::layers::MaskTable,
        tick: u64,
    ) -> Result<(GaitVerdict, CaptureBias), ServoError> {
        let verdict = self.strike_gait(ragdoll, strike, mask, tick)?;
        let bias = self
            .capture_bias(strike.impulse_q8)
            .map_err(ServoError::Balance)?;
        Ok((verdict, bias))
    }

    /// Drive the fall-catch: every bone stays free except the fighter-rig arm
    /// slots, which track the animated reach-out while the body stays down.
    /// Runs once when [`BalanceState::gait`] reports catching; shorter rigs
    /// without arm slots keep the full mask and still count the catch ticks.
    /// A wrong phase refuses with both systems unchanged. Event rate only.
    #[cfg(feature = "rapier")]
    pub fn catch(
        &mut self,
        ragdoll: &mut super::powered::PoweredRagdoll,
    ) -> Result<(), ServoError> {
        if self.gait != GaitPhase::Catching {
            return Err(ServoError::Balance(BalanceError::WrongPhase {
                want: GaitPhase::Catching,
                got: self.gait,
            }));
        }
        use engine_anim::hit_react::{ARM_L, ARM_R};
        let bones = ragdoll.bone_count();
        let mut catching = vec![true; bones];
        for arm in [ARM_L, ARM_R] {
            if arm < bones {
                catching[arm] = false;
            }
        }
        ragdoll.set_mask(&catching).map_err(ServoError::Ragdoll)?;
        Ok(())
    }

    /// Start the powered-ragdoll get-up once the catch window expires: every drive
    /// re-engages and the output blends back within 60 ticks. Consumes the
    /// get-up flag; a rise with the flag clear, or a downed-body refusal from
    /// the servo, changes nothing.
    #[cfg(feature = "rapier")]
    pub fn rise(&mut self, ragdoll: &mut super::powered::PoweredRagdoll) -> Result<(), ServoError> {
        if !self.getup_due {
            return Err(ServoError::Balance(BalanceError::GetupNotDue));
        }
        ragdoll.start_getup().map_err(ServoError::Ragdoll)?;
        self.getup_due = false;
        Ok(())
    }
}

/// Validate one strike shape against the ragdoll: the suggested bone must sit
/// on the ragdoll and the mask width must agree. Returns the strike bone for
/// the servo treatment; refuses before either system mutates.
#[cfg(feature = "rapier")]
fn strike_shape(
    ragdoll: &super::powered::PoweredRagdoll,
    strike: &engine_anim::hit_react::HitStrike,
    mask: &engine_anim::layers::MaskTable,
) -> Result<usize, ServoError> {
    let bones = ragdoll.bone_count();
    let bone = strike.dir.suggest_bone();
    if bone >= bones {
        return Err(ServoError::Balance(BalanceError::BadBone { bone, bones }));
    }
    if mask.len() != bones {
        return Err(ServoError::Balance(BalanceError::BadMask {
            expected: bones,
            got: mask.len(),
        }));
    }
    Ok(bone)
}

/// Shove impulse scale, newtons per Q8 unit of strike impulse: unity carries
/// [`SHOVE_NEWTON_PER_UNITY`] into the struck bone under the rapier feature.
#[cfg(feature = "rapier")]
fn impact_scale(impulse_q8: u16) -> f32 {
    use engine_anim::hit_react::IMPULSE_Q8_ONE;
    f32::from(impulse_q8) / f32::from(IMPULSE_Q8_ONE) * SHOVE_NEWTON_PER_UNITY
}

/// Stagger servo treatment: union the strike mask over the live mask at event
/// rate (one buffer per hit, never on the tick path), then free the struck
/// subtree with the scaled planar push. The mask sets the stance, the hit
/// sets the subtree free.
#[cfg(feature = "rapier")]
fn drive_stagger(
    ragdoll: &mut super::powered::PoweredRagdoll,
    bone: usize,
    strike: &engine_anim::hit_react::HitStrike,
    mask: &engine_anim::layers::MaskTable,
    tick: u64,
) -> Result<(), ServoError> {
    let mut union = Vec::with_capacity(ragdoll.bone_count());
    for (live, wanted) in ragdoll.mask().iter().zip(mask.weights().iter()) {
        union.push(*live || *wanted != 0);
    }
    ragdoll.set_mask(&union).map_err(ServoError::Ragdoll)?;
    let push = strike.dir.push();
    let scale = impact_scale(strike.impulse_q8);
    let impulse = [push[0] * scale, 0.0, push[1] * scale];
    ragdoll
        .apply_hit(bone, impulse, tick)
        .map_err(ServoError::Ragdoll)?;
    Ok(())
}

/// Mass-weighted center of mass over caller slices: lengths must agree, every
/// center must be finite inside the position bound and every mass finite
/// inside `(0, bound]`. Only `+ - * /` plus comparisons in bone order, so the
/// same bones give the same center twice; slices in, array out, no allocation.
pub fn center_of_mass(positions: &[[f32; 3]], masses: &[f32]) -> Result<[f32; 3], BalanceError> {
    if positions.len() != masses.len() {
        return Err(BalanceError::LengthMismatch {
            expected: positions.len(),
            got: masses.len(),
        });
    }
    if positions.is_empty() {
        return Err(BalanceError::EmptyBones);
    }
    let mut total = 0.0f32;
    let mut acc = [0.0f32; 3];
    for (i, (center, mass)) in positions.iter().zip(masses.iter()).enumerate() {
        for c in center.iter() {
            if !c.is_finite() || c.abs() > MAX_BONE_POS_M {
                return Err(BalanceError::BadPosition(i));
            }
        }
        if !mass.is_finite() || *mass <= 0.0 || *mass > MAX_BONE_MASS_KG {
            return Err(BalanceError::BadMass(i));
        }
        total += *mass;
        acc[0] += center[0] * *mass;
        acc[1] += center[1] * *mass;
        acc[2] += center[2] * *mass;
    }
    if !total.is_finite() || total <= 0.0 {
        return Err(BalanceError::BadMass(positions.len()));
    }
    Ok([acc[0] / total, acc[1] / total, acc[2] / total])
}

/// Center of mass in integer millimetres: each meter component scales by
/// 1000 and rounds half away from zero, so platform float drift under half
/// a millimetre hashes equal while a real 1 mm move hashes different.
/// Quantization runs on integer bit math through [`meters_to_mm`], so no
/// float-to-int cast lowering can split the hash between CPUs; non-finite
/// inputs (unreachable after validation) map to one sentinel so the hash
/// stays total. Pure, no allocation; servo math untouched.
/// Golden: the legacy fixture (seed 2040, 600 ticks) still hashes the
/// stored value twice with this quantization; the balance center never
/// reaches that fixture, so no bump is needed.
pub fn com_mm(com: [f32; 3]) -> [i32; 3] {
    com.map(meters_to_mm)
}

/// Meters to integer millimetres with the hash clamp: scales by 1000,
/// rounds half away from zero and clamps into the millimetre range, then
/// quantizes with [`quantize_mm`]. Both zero signs read 0. Pure, no
/// allocation.
fn meters_to_mm(v: f32) -> i32 {
    if !v.is_finite() {
        return i32::MAX;
    }
    // The value is pre-rounded and clamped to the millimetre range, so the
    // quantizer below stays total. The explicit clamp states the range
    // (±1000 m fits) as data.
    quantize_mm((v * 1000.0).round().clamp(MM_FLOOR_F32, MM_CEIL_F32))
}

/// Whole-millimetre float to `i32` on integer bit math: decodes the
/// IEEE-754 bits directly (sign, exponent, significand), so the result is
/// exact for finite magnitudes under 2^31 and saturates past them, bit for
/// bit like the old saturating cast. Pure, no allocation.
fn quantize_mm(v: f32) -> i32 {
    let bits = v.to_bits();
    let neg = bits & 0x8000_0000 != 0;
    let exp = (bits >> 23) & 0xFF;
    let frac = bits & 0x00FF_FFFF;
    if exp == 0 {
        // Zero or subnormal: under one millimetre, and only whole
        // millimetres arrive here, so this reads 0 either signed way.
        return 0;
    }
    if exp > 158 {
        // Past 2^31 either way: hold the nearer bound.
        return if neg { i32::MIN } else { i32::MAX };
    }
    let significand = u64::from(frac | 0x0080_0000);
    let magnitude = if exp >= 150 {
        significand << (exp - 150)
    } else if exp <= 86 {
        // Under 2^-17 even with the leading one: truncates to 0, and the
        // shift itself would overshoot the 64-bit lane.
        0
    } else {
        significand >> (150 - exp)
    };
    if magnitude >= 0x8000_0000 {
        // Exactly -2^31 reads MIN; +2^31 and past hold the nearer bound.
        return if neg { i32::MIN } else { i32::MAX };
    }
    // The range check above proves this conversion; the fallback never runs.
    let positive = i32::try_from(magnitude).unwrap_or(i32::MAX);
    if neg {
        0i32.wrapping_sub(positive)
    } else {
        positive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    /// Three-bone asymmetric rig with a hand-computed center: total mass 6,
    /// x = 12/6, y = 14/6, z = 8/6.
    fn rig_positions() -> Vec<[f32; 3]> {
        vec![[0.0, 1.0, 0.0], [0.0, 2.0, 4.0], [4.0, 3.0, 0.0]]
    }

    fn rig_masses() -> Vec<f32> {
        vec![1.0, 2.0, 3.0]
    }

    /// Five evenly spaced shove strengths in Q8 units: the first sits under
    /// the default threshold and the rest sit at or past it.
    const SHOVE_Q8: [u16; 5] = [128, 256, 384, 512, 640];

    fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    #[test]
    fn balance_stagger_com_matches_hand_computed_within_cm() {
        let com = center_of_mass(&rig_positions(), &rig_masses()).expect("valid rig");
        let expected = [12.0f32 / 6.0, 14.0f32 / 6.0, 8.0f32 / 6.0];
        assert!(
            dist3(com, expected) < 0.01,
            "center must land within 1 cm: {com:?} vs {expected:?}"
        );
        let mut state = BalanceState::new(BalanceParams::default());
        assert_eq!(state.com(), [0.0; 3]);
        let stored = state
            .observe(&rig_positions(), &rig_masses())
            .expect("valid observe");
        assert_eq!(stored, com);
        assert_eq!(state.com(), com);
        // One bone carries its own center exactly.
        let solo = center_of_mass(&[[1.0, 2.0, 3.0]], &[5.0]).expect("solo bone");
        assert_eq!(solo, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn balance_stagger_threshold_orders_five_strengths() {
        let mut state = BalanceState::new(BalanceParams::default());
        state
            .observe(&rig_positions(), &rig_masses())
            .expect("valid observe");
        // A zero shove holds steady and still counts as an event.
        assert_eq!(
            state.apply_shove(0).expect("zero shove"),
            BalanceVerdict::Steady
        );
        assert!(!state.staggered());
        let mut verdicts = Vec::with_capacity(SHOVE_Q8.len());
        for q8 in SHOVE_Q8 {
            verdicts.push(state.apply_shove(q8).expect("bounded shove"));
        }
        assert_eq!(
            verdicts,
            [
                BalanceVerdict::Steady,
                BalanceVerdict::Staggered,
                BalanceVerdict::Staggered,
                BalanceVerdict::Staggered,
                BalanceVerdict::Staggered,
            ],
            "only the first of five strengths holds steady"
        );
        assert!(state.staggered());
        assert_eq!(state.stagger_remaining(), STAGGER_TICKS);
        assert_eq!(
            state.shove_count(),
            1 + u64::try_from(SHOVE_Q8.len()).unwrap_or(u64::MAX)
        );
        // A lighter follow-up shove holds its own verdict but must not clear
        // an open stagger window.
        assert_eq!(
            state.apply_shove(1).expect("light shove"),
            BalanceVerdict::Steady
        );
        assert!(state.staggered());
    }

    #[test]
    fn balance_com_hash_quantizes_to_mm() {
        let mut base = BalanceState::new(BalanceParams::default());
        base.observe(&rig_positions(), &rig_masses())
            .expect("valid observe");
        let anchor = base.com();
        // Sub-millimetre jitter (FMA or summation-order drift) hashes equal.
        let mut jittered = base;
        jittered.com = [anchor[0] + 0.0001, anchor[1] - 0.0001, anchor[2] + 0.0001];
        assert_eq!(
            base.state_hash(),
            jittered.state_hash(),
            "a 1e-4 m COM jitter must hash equal"
        );
        // A real 1 mm move hashes different.
        let mut moved = base;
        moved.com = [anchor[0] + 0.001, anchor[1], anchor[2]];
        assert_ne!(
            base.state_hash(),
            moved.state_hash(),
            "a 1 mm COM move must hash different"
        );
        // Helper units: metres to integer millimetres, zero signs merged.
        assert_eq!(com_mm([1.0, -0.0, 2.0004]), [1000, 0, 2000]);
        assert_eq!(
            com_mm([f32::NAN, f32::INFINITY, f32::NEG_INFINITY]),
            [i32::MAX; 3]
        );
    }

    #[test]
    fn balance_stagger_recovers_within_window() {
        let mut state = BalanceState::new(BalanceParams::default());
        state.apply_shove(512).expect("staggering shove");
        for _ in 0..STAGGER_TICKS - 1 {
            state.step();
            assert!(state.staggered(), "window must hold until the last tick");
        }
        state.step();
        assert!(
            !state.staggered(),
            "window must clear on its thirtieth tick"
        );
        assert_eq!(state.stagger_remaining(), 0);
        assert_eq!(state.tick(), u64::from(STAGGER_TICKS));
        // A fresh shove reopens the full window instead of stacking.
        state.apply_shove(640).expect("second shove");
        assert_eq!(state.stagger_remaining(), STAGGER_TICKS);
    }

    #[test]
    fn balance_stagger_hash_stable_x2() {
        let seq = |state: &mut BalanceState| {
            state
                .observe(&rig_positions(), &rig_masses())
                .expect("valid observe");
            state.apply_shove(128).expect("light shove");
            state.apply_shove(512).expect("staggering shove");
            for _ in 0..10 {
                state.step();
            }
        };
        let mut a = BalanceState::new(BalanceParams::default());
        let mut b = BalanceState::new(BalanceParams::default());
        seq(&mut a);
        seq(&mut b);
        assert_eq!(a.state_hash(), b.state_hash(), "same sequence hashes twice");
        assert_eq!(a, b);
        // The event order reaches the hash: an extra shove must move it.
        b.apply_shove(128).expect("extra shove");
        assert_ne!(
            a.state_hash(),
            b.state_hash(),
            "event order must reach the hash"
        );
    }

    #[test]
    fn balance_stagger_rejects_bad_input() {
        // Params refuse a hair trigger and a mis-scaled ceiling.
        assert_eq!(
            BalanceParams::new(0).unwrap_err(),
            BalanceError::BadThreshold(0)
        );
        assert_eq!(
            BalanceParams::new(MAX_IMPULSE_Q8 + 1).unwrap_err(),
            BalanceError::BadThreshold(MAX_IMPULSE_Q8 + 1)
        );
        assert_eq!(
            BalanceParams::new(256).expect("valid threshold"),
            BalanceParams::default()
        );
        // Empty and ragged rigs are refused.
        let empty: Vec<[f32; 3]> = vec![];
        assert_eq!(
            center_of_mass(&empty, &[]).unwrap_err(),
            BalanceError::EmptyBones
        );
        assert_eq!(
            center_of_mass(&rig_positions(), &[1.0, 2.0]).unwrap_err(),
            BalanceError::LengthMismatch {
                expected: 3,
                got: 2
            }
        );
        // Each bone index carries its own failure.
        let mut bad_spot = rig_positions();
        bad_spot[1] = [0.0, f32::NAN, 0.0];
        assert_eq!(
            center_of_mass(&bad_spot, &rig_masses()).unwrap_err(),
            BalanceError::BadPosition(1)
        );
        bad_spot[1] = [0.0, MAX_BONE_POS_M + 1.0, 0.0];
        assert_eq!(
            center_of_mass(&bad_spot, &rig_masses()).unwrap_err(),
            BalanceError::BadPosition(1)
        );
        let mut bad_mass = rig_masses();
        bad_mass[2] = 0.0;
        assert_eq!(
            center_of_mass(&rig_positions(), &bad_mass).unwrap_err(),
            BalanceError::BadMass(2)
        );
        bad_mass[2] = -3.0;
        assert_eq!(
            center_of_mass(&rig_positions(), &bad_mass).unwrap_err(),
            BalanceError::BadMass(2)
        );
        bad_mass[2] = f32::INFINITY;
        assert_eq!(
            center_of_mass(&rig_positions(), &bad_mass).unwrap_err(),
            BalanceError::BadMass(2)
        );
        // An over-ceiling shove is refused and records nothing.
        let mut state = BalanceState::new(BalanceParams::default());
        let before = state.state_hash();
        assert_eq!(
            state.apply_shove(MAX_IMPULSE_Q8 + 1).unwrap_err(),
            BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
        );
        assert_eq!(state.shove_count(), 0);
        assert_eq!(state.state_hash(), before);
    }

    #[test]
    fn balance_stumble_tick_path_alloc_free() {
        let positions = rig_positions();
        let masses = rig_masses();
        let mut state = BalanceState::new(BalanceParams::default());
        let ((), allocs) = alloc_probe::measure(|| {
            state.observe(&positions, &masses).expect("valid observe");
            state.apply_shove(512).expect("staggering shove");
            for _ in 0..STAGGER_TICKS {
                state.step();
            }
            let _ = state.state_hash();
        });
        assert_eq!(
            allocs.calls, 0,
            "tick path must not allocate, got {allocs:?}"
        );
        assert!(!state.staggered());
    }

    /// Acceptance: five evenly spaced strengths order steady, stagger,
    /// stumble, stumble, fall; the fall runs prone plus catch inside 60
    /// ticks and raises the get-up flag; twins hash twice.
    #[test]
    fn balance_stumble() {
        use GaitVerdict::{Fallen, Staggered, Steady, Stumbling};

        let mut state = BalanceState::new(BalanceParams::default());
        assert_eq!(state.stumble_q8(), 384);
        assert_eq!(state.fall_q8(), 640);
        let mut verdicts = Vec::with_capacity(SHOVE_Q8.len());
        for q8 in SHOVE_Q8 {
            verdicts.push(state.apply_impact(q8).expect("bounded impact"));
        }
        assert_eq!(
            verdicts,
            [Steady, Staggered, Stumbling, Stumbling, Fallen],
            "stumble at 3-4/5, fall at 5/5"
        );
        assert_eq!(state.gait(), GaitPhase::Fallen);
        assert_eq!(state.fall_count(), 1);
        assert_eq!(state.gait_remaining(), FALL_PRONE_TICKS);
        // An identical twin hashes the same way twice.
        let mut twin = BalanceState::new(BalanceParams::default());
        for q8 in SHOVE_Q8 {
            twin.apply_impact(q8).expect("bounded impact");
        }
        assert_eq!(state, twin);
        assert_eq!(
            state.state_hash(),
            twin.state_hash(),
            "same impact sequence hashes twice"
        );
        // Prone 30 ticks roll into the catch, catch 15 raise the get-up
        // flag: 45 ticks from the fall, inside the 60-tick budget.
        for _ in 0..FALL_PRONE_TICKS {
            state.step();
            twin.step();
        }
        assert_eq!(state.gait(), GaitPhase::Catching);
        assert_eq!(state.gait_remaining(), CATCH_TICKS);
        assert!(!state.getup_due());
        for _ in 0..CATCH_TICKS {
            state.step();
            twin.step();
        }
        assert_eq!(state.gait(), GaitPhase::Steady);
        assert!(state.getup_due());
        assert_eq!(state.tick(), u64::from(FALL_PRONE_TICKS + CATCH_TICKS));
        assert_eq!(
            state.state_hash(),
            twin.state_hash(),
            "twins stay equal through the windows"
        );
        // The fall count reaches the hash: one more fall moves it.
        twin.apply_impact(640).expect("second fall");
        assert_eq!(twin.fall_count(), 2);
        assert_ne!(
            state.state_hash(),
            twin.state_hash(),
            "fall order must reach the hash"
        );
    }

    #[test]
    fn balance_stumble_gait_recovers_without_downgrading_fall() {
        use GaitVerdict::{Fallen, Staggered, Steady, Stumbling};

        let mut state = BalanceState::new(BalanceParams::default());
        assert_eq!(state.apply_impact(384).expect("stumble"), Stumbling);
        assert_eq!(state.gait(), GaitPhase::Stumbling);
        assert_eq!(state.gait_remaining(), STUMBLE_TICKS);
        // The shorter stagger window clears first while the gait holds.
        for _ in 0..STAGGER_TICKS {
            state.step();
        }
        assert!(!state.staggered());
        assert_eq!(state.gait(), GaitPhase::Stumbling);
        for _ in 0..(STUMBLE_TICKS - STAGGER_TICKS - 1) {
            state.step();
            assert_eq!(state.gait(), GaitPhase::Stumbling);
        }
        state.step();
        assert_eq!(state.gait(), GaitPhase::Steady);
        assert_eq!(state.gait_remaining(), 0);
        assert!(!state.getup_due(), "a stumble never owes a get-up");
        // A fall refuses downgrade: lighter blows report their own level
        // while the phase runs its course, and a fresh stumble clears a
        // pending get-up flag by superseding it.
        assert_eq!(state.apply_impact(640).expect("fall"), Fallen);
        assert_eq!(state.apply_impact(128).expect("light"), Steady);
        assert_eq!(state.apply_impact(256).expect("stagger"), Staggered);
        assert_eq!(state.apply_impact(384).expect("stumble"), Stumbling);
        assert_eq!(
            state.gait(),
            GaitPhase::Fallen,
            "impacts never downgrade an in-flight fall"
        );
        assert_eq!(state.fall_count(), 1);
    }

    #[test]
    fn balance_stumble_rejects_bad_input() {
        // An over-ceiling blow is refused and records nothing.
        let mut state = BalanceState::new(BalanceParams::default());
        let before = state.state_hash();
        assert_eq!(
            state.apply_impact(MAX_IMPULSE_Q8 + 1).unwrap_err(),
            BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
        );
        assert_eq!(state.shove_count(), 0);
        assert_eq!(state.fall_count(), 0);
        assert_eq!(state.gait(), GaitPhase::Steady);
        assert_eq!(state.state_hash(), before);
        // Thresholds derive from the stagger tuning, saturating.
        let soft = BalanceParams::new(128).expect("soft tuning");
        let soft_state = BalanceState::new(soft);
        assert_eq!(soft_state.stumble_q8(), 256);
        assert_eq!(soft_state.fall_q8(), 512);
        let maxed = BalanceState::new(BalanceParams::new(MAX_IMPULSE_Q8).expect("max tuning"));
        assert_eq!(maxed.stumble_q8(), MAX_IMPULSE_Q8 + 128);
        assert_eq!(maxed.fall_q8(), MAX_IMPULSE_Q8 + 384);
    }

    #[test]
    fn balance_stumble_tick_path_stays_alloc_free() {
        let positions = rig_positions();
        let masses = rig_masses();
        let mut state = BalanceState::new(BalanceParams::default());
        let ((), allocs) = alloc_probe::measure(|| {
            state.observe(&positions, &masses).expect("valid observe");
            state.apply_impact(512).expect("stumbling impact");
            for _ in 0..STUMBLE_TICKS {
                state.step();
            }
            let _ = state.state_hash();
        });
        assert_eq!(
            allocs.calls, 0,
            "gait tick path must not allocate, got {allocs:?}"
        );
        assert_eq!(state.gait(), GaitPhase::Steady);
    }

    /// Wired memory leg: a light blow holds the stance steady yet still
    /// leaves pain behind, and refusals move neither system.
    #[test]
    fn balance_stagger_memory_wires_hit_or_neither_moves() {
        use super::super::stagger_memory::{StaggerMemory, MAX_MEMORY_BONES};
        let mut state = BalanceState::new(BalanceParams::default());
        let mut memory = StaggerMemory::new();
        let (verdict, level) = state
            .apply_memory_hit(&mut memory, 192, 0)
            .expect("light wired hit");
        assert_eq!(verdict, GaitVerdict::Steady);
        assert_eq!(state.shove_count(), 1);
        assert_eq!(memory.hit_count(), 1);
        let _ = level;
        let state_before = state.state_hash();
        let memory_before = memory.state_hash();
        assert_eq!(
            state
                .apply_memory_hit(&mut memory, MAX_IMPULSE_Q8 + 1, 0)
                .unwrap_err(),
            BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
        );
        assert_eq!(
            state
                .apply_memory_hit(&mut memory, 192, MAX_MEMORY_BONES)
                .unwrap_err(),
            BalanceError::BadBone {
                bone: MAX_MEMORY_BONES,
                bones: MAX_MEMORY_BONES,
            }
        );
        assert_eq!(state.shove_count(), 1);
        assert_eq!(memory.hit_count(), 1);
        assert_eq!(state.state_hash(), state_before);
        assert_eq!(memory.state_hash(), memory_before);
    }

    /// Impact severity rank for the ordering proof: steady reads 0 through
    /// fallen reads 3, so a sorted rank row means an ordered matrix.
    fn severity(verdict: GaitVerdict) -> u8 {
        match verdict {
            GaitVerdict::Steady => 0,
            GaitVerdict::Staggered => 1,
            GaitVerdict::Stumbling => 2,
            GaitVerdict::Fallen => 3,
        }
    }

    /// Response matrix acceptance: the five standard strengths order steady,
    /// stagger, stumble, stumble, fall; the pure matrix agrees with the state
    /// machine; every response closes within the recovery budget; twins hash
    /// twice and stay equal through the windows.
    #[test]
    fn balance_matrix_orders_five_strengths_within_budget() {
        use GaitVerdict::{Fallen, Staggered, Steady, Stumbling};

        let params = BalanceParams::default();
        let matrix = response_matrix(params);
        assert_eq!(
            matrix.map(|row| row.strength_q8),
            RESPONSE_Q8,
            "the matrix answers the shared five-strength table"
        );
        assert_eq!(
            matrix.map(|row| row.verdict),
            [Steady, Staggered, Stumbling, Stumbling, Fallen],
            "default tuning orders 1/5 steady, 2/5 stagger, 3-4/5 stumble, 5/5 fall"
        );
        for row in matrix {
            assert_eq!(row.recovery_ticks, recovery_ticks(row.verdict));
            assert!(
                row.recovery_ticks <= RECOVERY_BUDGET_TICKS,
                "strength {} recovers in {} ticks, past budget {RECOVERY_BUDGET_TICKS}",
                row.strength_q8,
                row.recovery_ticks
            );
            // The state machine deals what the matrix promises and lands
            // legal exactly on the row's recovery tick.
            let mut state = BalanceState::new(params);
            let dealt = state
                .apply_impact(row.strength_q8)
                .expect("matrix strengths sit inside the ceiling");
            assert_eq!(dealt, row.verdict);
            let mut stepped: u32 = 0;
            while !state.stance_legal() {
                state.step();
                stepped = stepped.saturating_add(1);
            }
            assert_eq!(
                stepped, row.recovery_ticks,
                "strength {} must close on its row tick",
                row.strength_q8
            );
            assert!(state.stance_legal());
            // The driver takes the same path: one apply plus the same steps.
            let mut driven = BalanceState::new(params);
            let took = driven
                .respond(row.strength_q8)
                .expect("matrix strengths sit inside the ceiling");
            assert_eq!(took, row.recovery_ticks);
            assert!(driven.stance_legal());
            assert_eq!(driven, state);
            assert_eq!(driven.state_hash(), state.state_hash());
        }
        // Retunes keep the order: verdict severity never falls as strength
        // rises, from a hair trigger to a held line.
        for stagger in [128u16, 256, 512, 1024] {
            let tuned = BalanceParams::new(stagger).expect("tuning inside the ceiling");
            let verdicts = response_matrix(tuned).map(|row| severity(row.verdict));
            let mut ordered = verdicts;
            ordered.sort_unstable();
            assert_eq!(
                verdicts, ordered,
                "stagger {stagger} must keep the matrix ordered"
            );
        }
        // Twins driven through the whole matrix hash twice; one more fall
        // moves the hash.
        let mut a = BalanceState::new(params);
        let mut b = BalanceState::new(params);
        for strength_q8 in RESPONSE_Q8 {
            a.respond(strength_q8)
                .expect("matrix strengths sit inside the ceiling");
            b.respond(strength_q8)
                .expect("matrix strengths sit inside the ceiling");
        }
        assert_eq!(a, b);
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "same matrix run hashes twice"
        );
        b.respond(640).expect("second fall");
        assert_ne!(
            a.state_hash(),
            b.state_hash(),
            "event order must reach the hash"
        );
    }

    /// Response matrix failure leg: an over-ceiling shove is refused with
    /// nothing recorded, and a spent state still reports legal at the budget.
    #[test]
    fn balance_matrix_rejects_over_ceiling_unchanged() {
        let params = BalanceParams::default();
        let mut state = BalanceState::new(params);
        let before = state.state_hash();
        assert_eq!(
            state.respond(MAX_IMPULSE_Q8 + 1).unwrap_err(),
            BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
        );
        assert_eq!(state.shove_count(), 0);
        assert_eq!(state.fall_count(), 0);
        assert!(state.stance_legal());
        assert_eq!(state.state_hash(), before);
        // A fall closes at prone plus catch and holds legal through the
        // budget: illegal at the budget would fail this leg.
        state.respond(640).expect("falling shove");
        assert!(state.stance_legal());
        for _ in 0..(RECOVERY_BUDGET_TICKS - (FALL_PRONE_TICKS + CATCH_TICKS)) {
            state.step();
        }
        assert!(
            state.stance_legal(),
            "a closed fall must hold legal through the budget"
        );
    }

    /// Response matrix budget leg: the pure matrix plus the drive-to-legal
    /// path plus the hash allocates nothing.
    #[test]
    fn balance_matrix_tick_path_stays_alloc_free() {
        let params = BalanceParams::default();
        let ((), allocs) = alloc_probe::measure(|| {
            let matrix = response_matrix(params);
            let mut state = BalanceState::new(params);
            for row in matrix {
                state
                    .respond(row.strength_q8)
                    .expect("matrix strengths sit inside the ceiling");
            }
            let _ = state.state_hash();
        });
        assert_eq!(
            allocs.calls, 0,
            "matrix path must not allocate, got {allocs:?}"
        );
    }

    /// Stance rig for the capture-bias proof: feet split +/-150 mm about the
    /// origin under a centered pelvis, so the planar center sits at the
    /// support middle and the bias answers the shove lead alone.
    fn stance_positions() -> Vec<[f32; 3]> {
        vec![[0.0, 1.0, 0.0], [-0.15, 0.05, 0.0], [0.15, 0.05, 0.0]]
    }

    fn stance_masses() -> Vec<f32> {
        vec![10.0, 5.0, 5.0]
    }

    /// Capture-bias acceptance: the five standard strengths read through the
    /// capture bias beat the baseline recovery total by a posted
    /// number, every biased row closes inside the budget, bias reads move no
    /// hash, reversed bone insertion digests identical, over-ceiling blows
    /// are refused and the whole path allocates nothing.
    #[test]
    fn balance_matrix_capture_bias_beats_baseline_within_budget() {
        use GaitVerdict::{Fallen, Staggered, Steady, Stumbling};

        let params = BalanceParams::default();
        let matrix = response_matrix(params);
        assert_eq!(
            matrix.map(|row| row.verdict),
            [Steady, Staggered, Stumbling, Stumbling, Fallen],
            "default tuning orders 1/5 steady, 2/5 stagger, 3-4/5 stumble, 5/5 fall"
        );
        let baseline_total: u32 = matrix.iter().map(|row| row.recovery_ticks).sum();
        assert_eq!(
            baseline_total, 165,
            "the baseline totals 0 + 30 + 45 + 45 + 45"
        );
        // Pure helpers answer the shove lead before any state exists.
        assert_eq!(capture_error_mm([0, 525, 0], 512), [500, 0]);
        assert_eq!(capture_error_mm([450, 0, 0], 640), [500, 0]);
        assert_eq!(
            servo_bias_targets([0, 0]),
            CaptureBias {
                cp_err_mm: [0, 0],
                ankle_q8: 0,
                hip_q8: 0,
                stance_q8: 0,
            }
        );
        assert_eq!(servo_bias_targets([0, 500]).hip_q8, -128);

        let mut state = BalanceState::new(params);
        state
            .observe(&stance_positions(), &stance_masses())
            .expect("stance observe");
        assert_eq!(com_mm(state.com()), [0, 525, 0]);
        let hash_before = state.state_hash();
        let mut biased_total: u32 = 0;
        let mut ankles = [0i16; 5];
        let mut stances = [0i16; 5];
        for (i, row) in matrix.iter().enumerate() {
            let bias = state
                .capture_bias(row.strength_q8)
                .expect("matrix strengths sit inside the ceiling");
            ankles[i] = bias.ankle_q8;
            stances[i] = bias.stance_q8;
            assert_eq!(bias.hip_q8, 0, "no lateral error, no hip shift");
            let biased = biased_recovery_ticks(row.verdict, &bias);
            assert!(
                biased <= row.recovery_ticks,
                "strength {} biased {biased} must not pass baseline {}",
                row.strength_q8,
                row.recovery_ticks
            );
            assert!(
                biased <= RECOVERY_BUDGET_TICKS,
                "strength {} biased {biased} must close inside the budget",
                row.strength_q8
            );
            biased_total = biased_total.saturating_add(biased);
        }
        assert_eq!(
            ankles,
            [-32, -65, -98, -128, -128],
            "the ankle leans back at the error and holds its ceiling"
        );
        assert_eq!(
            stances,
            [-8, -16, -24, -32, -32],
            "the stance shares a quarter of the lean"
        );
        assert_eq!(state.state_hash(), hash_before, "bias reads move no hash");
        let improvement = baseline_total.saturating_sub(biased_total);
        assert_eq!(biased_total, 148, "biased rows total 0 + 28 + 40 + 40 + 40");
        assert!(
            improvement >= 10,
            "biased {biased_total} beats baseline {baseline_total} by {improvement}, want at least 10"
        );
        // Shuffled insertion digests identical: reversed bone order observes
        // the same millimetres, the same bias and the same hash. The skew
        // values (tenths times halves) sum inexactly in f32, so a naive
        // float hash would split here while the millimetre hash holds.
        let skew_positions = vec![[0.1, 1.2, 0.3], [0.4, 0.9, 0.6], [0.7, 1.1, 0.9]];
        let skew_masses = vec![0.5, 1.5, 2.5];
        let mut fwd = BalanceState::new(params);
        let mut rev = BalanceState::new(params);
        fwd.observe(&skew_positions, &skew_masses)
            .expect("forward observe");
        let mut turned_positions = skew_positions.clone();
        turned_positions.reverse();
        let mut turned_masses = skew_masses.clone();
        turned_masses.reverse();
        rev.observe(&turned_positions, &turned_masses)
            .expect("reversed observe");
        assert_eq!(com_mm(fwd.com()), com_mm(rev.com()));
        assert_eq!(
            fwd.capture_bias(512).expect("forward bias"),
            rev.capture_bias(512).expect("reversed bias")
        );
        assert_eq!(fwd.state_hash(), rev.state_hash());
        // An over-ceiling blow is refused with nothing to move.
        assert_eq!(
            state.capture_bias(MAX_IMPULSE_Q8 + 1).unwrap_err(),
            BalanceError::BadImpulse(MAX_IMPULSE_Q8 + 1)
        );
        // The observe plus impact plus bias plus hash path allocates nothing.
        let positions = stance_positions();
        let masses = stance_masses();
        let ((), allocs) = alloc_probe::measure(|| {
            let mut probe = BalanceState::new(params);
            probe.observe(&positions, &masses).expect("valid observe");
            for row in response_matrix(params) {
                let bias = probe
                    .capture_bias(row.strength_q8)
                    .expect("bounded bias read");
                let _ = biased_recovery_ticks(row.verdict, &bias);
            }
            let _ = probe.state_hash();
        });
        assert_eq!(
            allocs.calls, 0,
            "bias path must not allocate, got {allocs:?}"
        );
    }
}

#[cfg(all(test, feature = "rapier"))]
mod servo_tests {
    use super::super::powered::{DriveParams, PoweredRagdoll};
    use super::super::ragdoll::BoneShape;
    use super::*;
    use engine_anim::contract::{Joint, LocalPose, Skeleton};
    use engine_anim::hit_react::{hit_mask, HitStrike};

    /// Six-bone chain in fighter order: pelvis, legs, arms, spine.
    fn fighter_skeleton() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "pelvis".into(),
                    parent: None,
                },
                Joint {
                    name: "leg_l".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "leg_r".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "arm_l".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "arm_r".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "spine".into(),
                    parent: Some(0),
                },
            ],
        }
    }

    fn fighter_pose() -> LocalPose {
        LocalPose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, -0.9, 0.0],
                [0.2, -0.9, 0.0],
                [-0.5, 0.4, 0.0],
                [0.5, 0.4, 0.0],
                [0.0, 0.5, 0.0],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 6],
        }
    }

    fn fighter_ragdoll() -> PoweredRagdoll {
        let shapes = vec![
            BoneShape {
                radius: 0.15,
                half_height: 0.2,
            };
            6
        ];
        let drives = vec![DriveParams::default_drive(); 6];
        PoweredRagdoll::spawn(&fighter_skeleton(), &fighter_pose(), &shapes, &drives)
            .expect("fighter spawn")
    }

    #[test]
    fn balance_stagger_servo_frees_struck_subtree_on_stagger() {
        use super::super::powered::RagdollMode;
        let mut state = BalanceState::new(BalanceParams::default());
        let mut ragdoll = fighter_ragdoll();
        // A light strike holds steady and never touches the servo.
        let calm = HitStrike::new(6, 128, 0).expect("light strike");
        assert_eq!(
            state
                .strike(&mut ragdoll, &calm, &hit_mask(), 0)
                .expect("steady strike"),
            BalanceVerdict::Steady
        );
        assert_eq!(ragdoll.mask(), &[false; 6]);
        assert_eq!(ragdoll.mode(), RagdollMode::Animated);
        // A front strike at unity staggers: the spine subtree goes free under
        // the union of the live mask and the hit-react mask.
        let front = HitStrike::new(0, 256, 1).expect("front strike");
        assert_eq!(
            state
                .strike(&mut ragdoll, &front, &hit_mask(), 1)
                .expect("staggering strike"),
            BalanceVerdict::Staggered
        );
        assert_eq!(ragdoll.mask(), &[true, false, false, true, true, true]);
        assert_eq!(ragdoll.mode(), RagdollMode::Partial);
    }

    #[test]
    fn balance_stagger_servo_refuses_bad_shape_untouched() {
        let mut state = BalanceState::new(BalanceParams::default());
        let mut ragdoll = fighter_ragdoll();
        let front = HitStrike::new(0, 256, 0).expect("front strike");
        let narrow = engine_anim::layers::MaskTable::splat(4, 255).expect("narrow mask");
        let before = ragdoll.state_hash();
        assert_eq!(
            state.strike(&mut ragdoll, &front, &narrow, 0).unwrap_err(),
            ServoError::Balance(BalanceError::BadMask {
                expected: 6,
                got: 4
            })
        );
        assert_eq!(ragdoll.mask(), &[false; 6]);
        assert_eq!(ragdoll.state_hash(), before);
        assert_eq!(state.shove_count(), 0);
    }

    #[test]
    fn balance_biased_strike_returns_live_bias_without_moving_hash() {
        use super::GaitVerdict::Stumbling;

        // The landed-blow entry returns the live bias beside the verdict: a
        // 384-impulse shove on a centered mass leads 384 mm to +x, so the
        // ankle opposes at -98 with a quarter-stance share of -24.
        let mut plain = BalanceState::new(BalanceParams::default());
        let mut biased_state = BalanceState::new(BalanceParams::default());
        let mut plain_ragdoll = fighter_ragdoll();
        let mut biased_ragdoll = fighter_ragdoll();
        let trip = HitStrike::new(0, 384, 1).expect("stumble strike");
        let verdict = plain
            .strike_gait(&mut plain_ragdoll, &trip, &hit_mask(), 1)
            .expect("stumble");
        assert_eq!(verdict, Stumbling);
        let (biased_verdict, bias) = biased_state
            .strike_gait_biased(&mut biased_ragdoll, &trip, &hit_mask(), 1)
            .expect("biased stumble");
        assert_eq!(biased_verdict, verdict);
        assert_eq!(
            bias,
            biased_state.capture_bias(384).expect("direct bias read")
        );
        assert_eq!(bias.cp_err_mm, [384, 0]);
        assert_eq!(bias.ankle_q8, -98);
        assert_eq!(bias.hip_q8, 0);
        assert_eq!(bias.stance_q8, -24);
        assert_eq!(biased_state.state_hash(), plain.state_hash());
        assert_eq!(biased_ragdoll.mask(), plain_ragdoll.mask());
        let biased_ticks = biased_recovery_ticks(biased_verdict, &bias);
        assert!(biased_ticks <= recovery_ticks(biased_verdict));
        // A refused shape moves neither system and yields no bias.
        let narrow = engine_anim::layers::MaskTable::splat(4, 255).expect("narrow mask");
        let hash_before = biased_state.state_hash();
        assert_eq!(
            biased_state
                .strike_gait_biased(&mut biased_ragdoll, &trip, &narrow, 2)
                .unwrap_err(),
            ServoError::Balance(BalanceError::BadMask {
                expected: 6,
                got: 4
            })
        );
        assert_eq!(biased_state.state_hash(), hash_before);
    }

    #[test]
    fn balance_stumble_servo_fall_catch_rise_within_budget() {
        use super::super::powered::{RagdollMode, GETUP_BUDGET_TICKS};
        use super::super::rapier::FIXED_DT;
        use super::GaitVerdict::{Fallen, Stumbling};

        let mut state = BalanceState::new(BalanceParams::default());
        let mut ragdoll = fighter_ragdoll();
        // Stumble at 3/5: the struck subtree goes free under the unioned
        // mask and the gait holds 45 ticks.
        let trip = HitStrike::new(0, 384, 1).expect("stumble strike");
        assert_eq!(
            state
                .strike_gait(&mut ragdoll, &trip, &hit_mask(), 1)
                .expect("stumble"),
            Stumbling
        );
        assert_eq!(ragdoll.mask(), &[true, false, false, true, true, true]);
        assert_eq!(ragdoll.mode(), RagdollMode::Partial);
        assert_eq!(state.gait_remaining(), STUMBLE_TICKS);
        for _ in 0..STUMBLE_TICKS {
            state.step();
            ragdoll.step(FIXED_DT).expect("fixed step");
        }
        assert_eq!(state.gait(), GaitPhase::Steady);
        // Fall at 5/5: the whole body goes down.
        let slam = HitStrike::new(0, 640, 2).expect("fall strike");
        assert_eq!(
            state
                .strike_gait(&mut ragdoll, &slam, &hit_mask(), 2)
                .expect("fall"),
            Fallen
        );
        assert_eq!(ragdoll.mask(), &[true; 6]);
        assert_eq!(ragdoll.mode(), RagdollMode::Full);
        // Prone 30 ticks roll into the catch: the arms drive while the body
        // stays down.
        for _ in 0..FALL_PRONE_TICKS {
            state.step();
            ragdoll.step(FIXED_DT).expect("fixed step");
        }
        assert_eq!(state.gait(), GaitPhase::Catching);
        state.catch(&mut ragdoll).expect("catch");
        assert_eq!(ragdoll.mask(), &[true, true, true, false, false, true]);
        assert_eq!(ragdoll.mode(), RagdollMode::Partial);
        // Catch 15 ticks raise the flag; the rise blends back within the
        // 60-tick get-up budget.
        for _ in 0..CATCH_TICKS {
            state.step();
            ragdoll.step(FIXED_DT).expect("fixed step");
        }
        assert!(state.getup_due());
        state.rise(&mut ragdoll).expect("rise");
        assert_eq!(ragdoll.mode(), RagdollMode::GettingUp);
        assert_eq!(ragdoll.getup_remaining(), GETUP_BUDGET_TICKS);
        for _ in 0..GETUP_BUDGET_TICKS {
            state.step();
            ragdoll.step(FIXED_DT).expect("fixed step");
        }
        assert_eq!(ragdoll.mode(), RagdollMode::Animated);
        assert!(!state.getup_due());
        // A spent flag refuses a second rise.
        assert_eq!(
            state.rise(&mut ragdoll).unwrap_err(),
            ServoError::Balance(BalanceError::GetupNotDue)
        );
    }

    #[test]
    fn balance_stumble_servo_refuses_bad_phase_untouched() {
        use super::super::powered::RagdollMode;

        let mut state = BalanceState::new(BalanceParams::default());
        let mut ragdoll = fighter_ragdoll();
        // Catch and rise refuse outside their phases; both systems unchanged.
        let before = ragdoll.state_hash();
        assert_eq!(
            state.catch(&mut ragdoll).unwrap_err(),
            ServoError::Balance(BalanceError::WrongPhase {
                want: GaitPhase::Catching,
                got: GaitPhase::Steady,
            })
        );
        assert_eq!(
            state.rise(&mut ragdoll).unwrap_err(),
            ServoError::Balance(BalanceError::GetupNotDue)
        );
        assert_eq!(ragdoll.mask(), &[false; 6]);
        assert_eq!(ragdoll.mode(), RagdollMode::Animated);
        assert_eq!(ragdoll.state_hash(), before);
        assert_eq!(state.shove_count(), 0);
        // A bad strike shape refuses before either system mutates.
        let slam = HitStrike::new(0, 640, 0).expect("fall strike");
        let narrow = engine_anim::layers::MaskTable::splat(4, 255).expect("narrow mask");
        let hash_before = state.state_hash();
        assert_eq!(
            state
                .strike_gait(&mut ragdoll, &slam, &narrow, 0)
                .unwrap_err(),
            ServoError::Balance(BalanceError::BadMask {
                expected: 6,
                got: 4
            })
        );
        assert_eq!(state.shove_count(), 0);
        assert_eq!(state.state_hash(), hash_before);
        assert_eq!(ragdoll.state_hash(), before);
        // Presentation reads never enter the balance hash.
        state.apply_impact(384).expect("stumble");
        let during = state.state_hash();
        let _ = ragdoll.bone_poses();
        let _ = ragdoll.blend_weight();
        assert_eq!(state.state_hash(), during);
    }

    #[test]
    fn balance_matrix_budget_pins_servo_getup() {
        use super::super::powered::GETUP_BUDGET_TICKS;

        assert_eq!(
            u64::from(RECOVERY_BUDGET_TICKS),
            GETUP_BUDGET_TICKS,
            "the sim recovery budget mirrors the servo get-up budget"
        );
    }
}

/// Shared EG34 blow script: a fighter walking along x is observed every tick,
/// takes a stagger blow at tick 40, a stumble at tick 70 and a knockdown-level
/// blow at tick 100, and steps 200 ticks; `client` sees every verdict the way
/// a client-local ragdoll would (seeded from the event, never feeding back).
/// Returns the balance hash after every tick.
#[cfg(test)]
pub(crate) fn knockdown_script(client: &mut dyn FnMut(u64, GaitVerdict)) -> Vec<u64> {
    const BLOWS: [(u64, u16); 3] = [(40, 256), (70, 384), (100, 640)];
    let mut state = BalanceState::new(BalanceParams::default());
    let mut hashes = Vec::with_capacity(200);
    for tick in 0..200u64 {
        let x = f32::from(u16::try_from(tick).unwrap_or(0)) * 0.01;
        state
            .observe(&[[x, 0.9, 0.0], [x, 1.4, 0.0]], &[50.0, 30.0])
            .expect("finite bones");
        for (at, impulse) in BLOWS {
            if at == tick {
                let verdict = state.apply_impact(impulse).expect("impulse in range");
                client(tick, verdict);
            }
        }
        state.step();
        hashes.push(state.state_hash());
    }
    hashes
}

#[cfg(test)]
mod balance_hash_tests {
    use super::*;

    #[test]
    fn balance_hash_knockdown_at_tick_100_twice_equal() {
        let mut verdicts = Vec::new();
        let first = knockdown_script(&mut |tick, verdict| verdicts.push((tick, verdict)));
        let second = knockdown_script(&mut |_, _| {});
        assert_eq!(first, second, "balance hash x2");
        assert_eq!(
            verdicts,
            vec![
                (40, GaitVerdict::Staggered),
                (70, GaitVerdict::Stumbling),
                (100, GaitVerdict::Fallen),
            ]
        );
        // The knockdown is in the hash: the tick-100 row moves with it.
        assert_ne!(first[99], first[100]);
    }

    #[test]
    fn balance_hash_never_reads_the_client_side() {
        // The split (EG34): whatever a client does with the verdicts (a
        // Rapier ragdoll, a tint, nothing) the authoritative hash is equal.
        let quiet = knockdown_script(&mut |_, _| {});
        let mut busy = 0u64;
        let watched = knockdown_script(&mut |tick, _| busy = busy.wrapping_add(tick));
        assert_eq!(quiet, watched);
        assert_eq!(busy, 210);
    }
}
