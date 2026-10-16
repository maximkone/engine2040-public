//! grapple: powered-ragdoll velocity servo retargeted to opponent ragdoll state.
//!
//! Consumer: arena clinch play reads both fighters' ragdoll bones each tick
//! and applies the servo forces plus the breakaway flag. Owner: this module
//! owns the grip pair, tune, torque caps and breakaway counters. Units: mm,
//! ticks at the 60 Hz fixed step, forces in mN, torques in mN-mm. Determinism:
//! integer math only, bones in index order, fighters sorted by ID, no heap
//! use after attach. Errors are typed [`GrappleError`]; bad input never panics.
//!
//! Servo: per-bone powered-style velocity drive whose target is the opponent bone
//! position taken as plain [`BodySample`] data (powered types are never
//! imported or mirrored). Force is integer F=ma toward the capped drive
//! velocity; the grip moment is the L1 force times the L1 relative pull
//! across the grip anchor, downscaled past the torque cap; the grip breaks
//! after sustained over-torque or excess anchor separation, always within
//! [`BREAKAWAY_BUDGET_TICKS`] ticks. Budget: O(bones) integer ops per tick.
#![forbid(unsafe_code)]

use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};

/// Fixed tick rate the servo gains assume.
pub const TICK_HZ: u32 = 60;
/// Largest bone count per side; construction refuses more.
pub const MAX_BONES: usize = 32;
/// Q16 unit for integer gains and the torque-cap downscale.
pub const Q16_ONE: i32 = 65_536;
/// Hard ceiling on sustained over-torque before breakaway: attach params pick
/// `break_after_ticks` inside `1..=BREAKAWAY_BUDGET_TICKS`, so a lost grip
/// always breaks within this many ticks of continuous overload.
pub const BREAKAWAY_BUDGET_TICKS: u32 = 30;
/// Position clamp per axis, mm (+-1000 m).
pub const MAX_POS_MM: i32 = 1_000_000;
/// Velocity clamp per axis, mm per tick.
pub const MAX_VEL_MM_TICK: i32 = 100_000;
/// Force cap ceiling, mN (1000 N).
pub const MAX_FORCE_MN: i32 = 1_000_000;
/// Torque cap ceiling, mN-mm.
pub const MAX_TORQUE_MN_MM: i64 = 1_000_000_000_000;
/// Mass ceiling, g (1000 kg per bone is far past human scale).
pub const MAX_MASS_G: i32 = 1_000_000;
/// Gain ceiling, Q16 (16.0).
pub const MAX_GAIN_Q16: i32 = 16 * 65_536;
/// Break-distance ceiling, mm (2000 m; anything farther is no grapple).
pub const MAX_BREAK_MM: i32 = 2_000_000;
/// F=ma scale: force_mN = mass_g * accel_mm_tick2 * 36 / 10. One tick is
/// 1/60 s, so accel in m/s^2 is raw * 0.001 * 3600 = raw * 3.6, and newtons
/// are (mass_g / 1000) * accel_si; millinewtons fold to mass_g * raw * 36/10.
const FORCE_NUM: i64 = 36;
const FORCE_DEN: i64 = 10;
/// Sentinel for [`GrapplePair::break_tick`] and [`GrappleOutcome::break_tick`]
/// while the grip still holds.
pub const HELD_TICK: u64 = u64::MAX;

/// Typed errors for grapple setup and stepping. Validation runs before any
/// state changes, so a failed call leaves the pair untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrappleError {
    /// Both fighter IDs are equal; a grip needs two distinct fighters.
    DuplicateFighter,
    /// Step called with an ID this pair was not attached with.
    UnknownFighter(u32),
    /// A bone slice length differs from the attached count.
    BadBoneCount {
        /// Attached bone count.
        expected: usize,
        /// Slice length seen.
        got: usize,
    },
    /// A bone sample left its range; the static name says which field.
    BadSample(&'static str),
    /// A servo tune field left its range; the static name says which field.
    BadTune(&'static str),
    /// An attach param left its range; the static name says which field.
    BadParams(&'static str),
    /// Grip anchor index is out of range for the attached bones.
    BadBone(usize),
    /// Snapshot was taken from a different pair (IDs, counts, tune, params).
    SnapshotMismatch,
    /// The grip already broke; inspect the posted break tick, then release it.
    AlreadyBroken,
}

impl core::fmt::Display for GrappleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GrappleError::DuplicateFighter => {
                f.write_str("duplicate fighter: need two distinct IDs")
            }
            GrappleError::UnknownFighter(id) => {
                write!(f, "unknown fighter: {id} is not in this pair")
            }
            GrappleError::BadBoneCount { expected, got } => {
                write!(f, "bad bone count: need {expected} bones, got {got}")
            }
            GrappleError::BadSample(what) => write!(f, "bad bone sample: {what}"),
            GrappleError::BadTune(what) => write!(f, "bad servo tune: {what}"),
            GrappleError::BadParams(what) => write!(f, "bad grapple params: {what}"),
            GrappleError::BadBone(i) => write!(f, "bad anchor bone: index {i} out of range"),
            GrappleError::SnapshotMismatch => f.write_str("snapshot does not match this pair"),
            GrappleError::AlreadyBroken => f.write_str("already broken: grip released"),
        }
    }
}

impl std::error::Error for GrappleError {}

/// Plain-data read of one opponent ragdoll bone for this tick: world position
/// in mm plus linear velocity in mm per tick. This is the only coupling to
/// the ragdoll world on purpose: the caller copies numbers out of its bodies
/// (powered ragdoll state or anything else) and this module never imports,
/// mirrors or stores a foreign body type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodySample {
    /// World position per axis, mm, each within `+-MAX_POS_MM`.
    pub pos_mm: [i32; 3],
    /// Linear velocity per axis, mm per tick, each within `+-MAX_VEL_MM_TICK`.
    pub vel_mm_tick: [i32; 3],
}

/// Per-bone velocity-servo tune, powered-drive shape with integer units: drive velocity
/// is `(error_mm * gain_q16) >> 16` clamped per axis to `max_speed_mm_tick`,
/// force is integer F=ma toward that velocity clamped to `force_cap_mn`, and
/// the exerted grip moment (L1 force times L1 relative pull across the grip
/// anchor, an integer upper bound on the true vector moment) is downscaled
/// past `torque_cap_mn_mm`. Force saturation is normal servo life; only
/// torque-cap binding counts toward breakaway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServoTune {
    /// Position-to-velocity gain, Q16, `0..=MAX_GAIN_Q16`. Zero is a limp
    /// grip that still counts contact for masking purposes.
    pub gain_q16: i32,
    /// Drive speed cap per axis, mm per tick, `1..=MAX_VEL_MM_TICK`.
    pub max_speed_mm_tick: i32,
    /// Force cap per axis, mN, `1..=MAX_FORCE_MN`.
    pub force_cap_mn: i32,
    /// Torque cap per bone, mN-mm, `1..=MAX_TORQUE_MN_MM`.
    pub torque_cap_mn_mm: i64,
    /// Driven bone mass, g, `1..=MAX_MASS_G`.
    pub mass_g: i32,
}

impl ServoTune {
    /// Clinch tune: 12.0 gain, ~9.6 m/s speed cap, 500 N force cap, a torque
    /// cap that never binds while both bodies translate together (uniform
    /// error twists nothing) while binding on a differential wrench of a few
    /// meters, 5 kg per bone.
    pub fn default_grip() -> Self {
        Self {
            gain_q16: 12 * 65_536,
            max_speed_mm_tick: 160,
            force_cap_mn: 500_000,
            torque_cap_mn_mm: 2_000_000_000,
            mass_g: 5_000,
        }
    }

    /// Range-check every field; the pair keeps its old tune on failure.
    pub fn validate(&self) -> Result<(), GrappleError> {
        if self.gain_q16 < 0 || self.gain_q16 > MAX_GAIN_Q16 {
            return Err(GrappleError::BadTune("gain_q16"));
        }
        if self.max_speed_mm_tick < 1 || self.max_speed_mm_tick > MAX_VEL_MM_TICK {
            return Err(GrappleError::BadTune("max_speed_mm_tick"));
        }
        if self.force_cap_mn < 1 || self.force_cap_mn > MAX_FORCE_MN {
            return Err(GrappleError::BadTune("force_cap_mn"));
        }
        if self.torque_cap_mn_mm < 1 || self.torque_cap_mn_mm > MAX_TORQUE_MN_MM {
            return Err(GrappleError::BadTune("torque_cap_mn_mm"));
        }
        if self.mass_g < 1 || self.mass_g > MAX_MASS_G {
            return Err(GrappleError::BadTune("mass_g"));
        }
        Ok(())
    }
}

/// Grip-level params: how long the torque cap may bind before the hold fails,
/// and how far the two grip anchors may separate before it snaps at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrappleParams {
    /// Consecutive over-torque ticks that break the grip,
    /// `1..=BREAKAWAY_BUDGET_TICKS`.
    pub break_after_ticks: u32,
    /// Anchor separation that breaks the grip immediately, mm,
    /// `1..=MAX_BREAK_MM`.
    pub break_distance_mm: i32,
}

impl GrappleParams {
    /// Hold that survives 10 overloaded ticks and snaps past 1.5 m of grip
    /// separation.
    pub fn default_hold() -> Self {
        Self {
            break_after_ticks: 10,
            break_distance_mm: 1500,
        }
    }

    /// Range-check every field; the pair keeps its old params on failure.
    pub fn validate(&self) -> Result<(), GrappleError> {
        if self.break_after_ticks < 1 || self.break_after_ticks > BREAKAWAY_BUDGET_TICKS {
            return Err(GrappleError::BadParams("break_after_ticks"));
        }
        if self.break_distance_mm < 1 || self.break_distance_mm > MAX_BREAK_MM {
            return Err(GrappleError::BadParams("break_distance_mm"));
        }
        Ok(())
    }
}

/// Per-tick outcome. Forces themselves are read with [`GrapplePair::forces_for`]
/// keyed by fighter ID, so a mirrored tape (IDs swapped) reads mirrored data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrappleOutcome {
    /// True when any bone's torque cap bound this tick (counter advanced).
    pub over_torque: bool,
    /// True once the grip has broken (this tick or earlier).
    pub broken: bool,
    /// Tick the grip broke at, or [`HELD_TICK`] while it holds.
    pub break_tick: u64,
}

/// Owned snapshot: every field that affects future output. Restore refuses a
/// snapshot from a different pair instead of guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrappleSnapshot {
    lo_id: u32,
    hi_id: u32,
    tune: ServoTune,
    params: GrappleParams,
    lo_anchor: usize,
    hi_anchor: usize,
    bone_count: usize,
    tick: u64,
    over_ticks: u32,
    broken: bool,
    break_tick: u64,
    lo_force: [[i32; 3]; MAX_BONES],
    hi_force: [[i32; 3]; MAX_BONES],
}

/// Mutual grapple servo between two fighters: each side's bones track the
/// other side's bone positions with the powered drive law, both directions read
/// the same tick state, torque caps bind per axis, and the grip breaks after
/// sustained over-torque or excess anchor separation. Fighter order is sorted
/// by ID at attach, so argument order at step time never changes the result.
pub struct GrapplePair {
    lo_id: u32,
    hi_id: u32,
    tune: ServoTune,
    params: GrappleParams,
    lo_anchor: usize,
    hi_anchor: usize,
    bone_count: usize,
    tick: u64,
    over_ticks: u32,
    broken: bool,
    break_tick: u64,
    lo_force: [[i32; 3]; MAX_BONES],
    hi_force: [[i32; 3]; MAX_BONES],
}

impl GrapplePair {
    /// Attach a grip between two distinct fighters over `bone_count` bones per
    /// side. Anchors name the gripped bone per side (used for the separation
    /// break); every other bone tracks its same-index opponent bone.
    pub fn attach(
        a_id: u32,
        a_anchor: usize,
        b_id: u32,
        b_anchor: usize,
        bone_count: usize,
        tune: ServoTune,
        params: GrappleParams,
    ) -> Result<Self, GrappleError> {
        if a_id == b_id {
            return Err(GrappleError::DuplicateFighter);
        }
        if !(1..=MAX_BONES).contains(&bone_count) {
            return Err(GrappleError::BadParams("bone_count"));
        }
        tune.validate()?;
        params.validate()?;
        if a_anchor >= bone_count {
            return Err(GrappleError::BadBone(a_anchor));
        }
        if b_anchor >= bone_count {
            return Err(GrappleError::BadBone(b_anchor));
        }
        // Sorted fighter order fixes the solve order: every later read and
        // the hash walk lo before hi no matter how the caller passes sides.
        let (lo_id, lo_anchor, hi_id, hi_anchor) = if a_id < b_id {
            (a_id, a_anchor, b_id, b_anchor)
        } else {
            (b_id, b_anchor, a_id, a_anchor)
        };
        Ok(Self {
            lo_id,
            hi_id,
            tune,
            params,
            lo_anchor,
            hi_anchor,
            bone_count,
            tick: 0,
            over_ticks: 0,
            broken: false,
            break_tick: HELD_TICK,
            lo_force: [[0; 3]; MAX_BONES],
            hi_force: [[0; 3]; MAX_BONES],
        })
    }

    /// Fighter IDs in solve order (lo, hi).
    pub fn fighter_ids(&self) -> (u32, u32) {
        (self.lo_id, self.hi_id)
    }

    /// Attached bone count per side.
    pub fn bone_count(&self) -> usize {
        self.bone_count
    }

    /// Fixed steps taken since attach.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Consecutive over-torque ticks so far (zero after a clean tick).
    pub fn over_ticks(&self) -> u32 {
        self.over_ticks
    }

    /// Whether the grip has broken.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Tick the grip broke at, or [`HELD_TICK`] while it holds.
    pub fn break_tick(&self) -> u64 {
        self.break_tick
    }

    /// Last applied servo forces for one fighter by ID, bone order, mN per
    /// axis. `None` for an ID outside the pair.
    pub fn forces_for(&self, id: u32) -> Option<&[[i32; 3]]> {
        if id == self.lo_id {
            Some(&self.lo_force[..self.bone_count])
        } else if id == self.hi_id {
            Some(&self.hi_force[..self.bone_count])
        } else {
            None
        }
    }

    /// Advance one fixed tick: drive both sides toward the opponent bones read
    /// from the same tick state, bind torque caps, then count over-torque or
    /// break on separation. A broken pair reports [`AlreadyBroken`](
    /// GrappleError::AlreadyBroken) and changes nothing.
    pub fn step(
        &mut self,
        a_id: u32,
        a_bones: &[BodySample],
        b_id: u32,
        b_bones: &[BodySample],
    ) -> Result<GrappleOutcome, GrappleError> {
        if self.broken {
            return Err(GrappleError::AlreadyBroken);
        }
        // Map caller sides onto solve order first: (a, b) and (b, a) must
        // compute bit-identical results.
        let (lo_bones, hi_bones) = if a_id == self.lo_id && b_id == self.hi_id {
            (a_bones, b_bones)
        } else if a_id == self.hi_id && b_id == self.lo_id {
            (b_bones, a_bones)
        } else if a_id == self.lo_id || a_id == self.hi_id {
            return Err(GrappleError::UnknownFighter(b_id));
        } else {
            return Err(GrappleError::UnknownFighter(a_id));
        };
        if lo_bones.len() != self.bone_count {
            return Err(GrappleError::BadBoneCount {
                expected: self.bone_count,
                got: lo_bones.len(),
            });
        }
        if hi_bones.len() != self.bone_count {
            return Err(GrappleError::BadBoneCount {
                expected: self.bone_count,
                got: hi_bones.len(),
            });
        }
        for sample in lo_bones.iter().chain(hi_bones.iter()) {
            check_sample(sample)?;
        }
        // Both directions servo off the same validated tick state.
        let mut lo_out = [[0i32; 3]; MAX_BONES];
        let mut hi_out = [[0i32; 3]; MAX_BONES];
        let lo_over = drive_side(lo_bones, hi_bones, self.lo_anchor, &self.tune, &mut lo_out);
        let hi_over = drive_side(hi_bones, lo_bones, self.hi_anchor, &self.tune, &mut hi_out);
        let mut over = lo_over || hi_over;
        // Anchor separation snaps the grip at once, no counting.
        if anchor_separated(
            &lo_bones[self.lo_anchor],
            &hi_bones[self.hi_anchor],
            self.params.break_distance_mm,
        ) {
            over = true;
            self.over_ticks = self.params.break_after_ticks;
        } else if over {
            self.over_ticks = self.over_ticks.saturating_add(1);
        } else {
            self.over_ticks = 0;
        }
        self.lo_force = lo_out;
        self.hi_force = hi_out;
        if self.over_ticks >= self.params.break_after_ticks {
            self.broken = true;
            self.break_tick = self.tick;
        }
        let outcome = GrappleOutcome {
            over_torque: over,
            broken: self.broken,
            break_tick: self.break_tick,
        };
        self.tick = self.tick.wrapping_add(1);
        Ok(outcome)
    }

    /// Canonical hash over IDs in solve order plus tune, params, counters and
    /// both force tables. Two runs over the same scripted tape hash equal;
    /// mirrored tapes (IDs swapped, inputs swapped) hash equal too.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = FNV64_OFFSET;
        fold_u32(&mut h, self.lo_id);
        fold_u32(&mut h, self.hi_id);
        fold_i32(&mut h, self.tune.gain_q16);
        fold_i32(&mut h, self.tune.max_speed_mm_tick);
        fold_i32(&mut h, self.tune.force_cap_mn);
        fold_i64(&mut h, self.tune.torque_cap_mn_mm);
        fold_i32(&mut h, self.tune.mass_g);
        fold_u32(&mut h, self.params.break_after_ticks);
        fold_i32(&mut h, self.params.break_distance_mm);
        fold_u64(&mut h, count_as_u64(self.lo_anchor));
        fold_u64(&mut h, count_as_u64(self.hi_anchor));
        fold_u64(&mut h, count_as_u64(self.bone_count));
        fold_u64(&mut h, self.tick);
        fold_u32(&mut h, self.over_ticks);
        fold_u64(&mut h, u64::from(self.broken));
        fold_u64(&mut h, self.break_tick);
        for side in [&self.lo_force, &self.hi_force] {
            for bone in &side[..self.bone_count] {
                for axis in bone {
                    fold_i32(&mut h, *axis);
                }
            }
        }
        h
    }

    /// Copy out the full future-affecting state.
    pub fn snapshot(&self) -> GrappleSnapshot {
        GrappleSnapshot {
            lo_id: self.lo_id,
            hi_id: self.hi_id,
            tune: self.tune,
            params: self.params,
            lo_anchor: self.lo_anchor,
            hi_anchor: self.hi_anchor,
            bone_count: self.bone_count,
            tick: self.tick,
            over_ticks: self.over_ticks,
            broken: self.broken,
            break_tick: self.break_tick,
            lo_force: self.lo_force,
            hi_force: self.hi_force,
        }
    }

    /// Restore a snapshot from the same pair. IDs, counts, tune, params and
    /// anchors must match; anything else reports [`SnapshotMismatch`](
    /// GrappleError::SnapshotMismatch) and changes nothing.
    pub fn restore(&mut self, snapshot: GrappleSnapshot) -> Result<(), GrappleError> {
        if snapshot.lo_id != self.lo_id
            || snapshot.hi_id != self.hi_id
            || snapshot.tune != self.tune
            || snapshot.params != self.params
            || snapshot.lo_anchor != self.lo_anchor
            || snapshot.hi_anchor != self.hi_anchor
            || snapshot.bone_count != self.bone_count
        {
            return Err(GrappleError::SnapshotMismatch);
        }
        if snapshot.broken && snapshot.break_tick == HELD_TICK {
            return Err(GrappleError::SnapshotMismatch);
        }
        if !snapshot.broken && snapshot.break_tick != HELD_TICK {
            return Err(GrappleError::SnapshotMismatch);
        }
        if snapshot.over_ticks > BREAKAWAY_BUDGET_TICKS {
            return Err(GrappleError::SnapshotMismatch);
        }
        self.tick = snapshot.tick;
        self.over_ticks = snapshot.over_ticks;
        self.broken = snapshot.broken;
        self.break_tick = snapshot.break_tick;
        self.lo_force = snapshot.lo_force;
        self.hi_force = snapshot.hi_force;
        Ok(())
    }
}

/// Integrate one bone under an applied servo force for a tick: velocity gains
/// the inverse of the servo force law, then position gains the velocity, both
/// saturated at the module clamps. Pure integer math, so scripted grapple
/// tapes replay exactly; the arena may use its own integrator instead, in
/// which case only the force tables (not these positions) are authoritative.
pub fn integrate_bone(sample: &BodySample, force_mn: [i32; 3], mass_g: i32) -> BodySample {
    if !(1..=MAX_MASS_G).contains(&mass_g) {
        return *sample;
    }
    let denom = i64::from(mass_g) * FORCE_NUM;
    let mut vel = sample.vel_mm_tick;
    let mut pos = sample.pos_mm;
    for a in 0..3 {
        let accel = i64::from(force_mn[a]) * FORCE_DEN / denom;
        let v = clamp(
            i64::from(vel[a]) + accel,
            -i64::from(MAX_VEL_MM_TICK),
            i64::from(MAX_VEL_MM_TICK),
        );
        vel[a] = v as i32;
        let p = clamp(
            i64::from(pos[a]) + v,
            -i64::from(MAX_POS_MM),
            i64::from(MAX_POS_MM),
        );
        pos[a] = p as i32;
    }
    BodySample {
        pos_mm: pos,
        vel_mm_tick: vel,
    }
}

/// Drive one side's bones toward the other side's same-index bones. Returns
/// true when any bone's torque cap bound. Only `+ - * /`, shifts and
/// comparisons; out-of-range input is rejected by the caller, never here.
fn drive_side(
    driven: &[BodySample],
    target: &[BodySample],
    anchor: usize,
    tune: &ServoTune,
    out: &mut [[i32; 3]; MAX_BONES],
) -> bool {
    // Anchor error first: the twist of every other bone is its error relative
    // to the gripped bone, so a rigid translation twists nothing by design.
    let anchor_err = error_between(&driven[anchor], &target[anchor]);
    let mut over = false;
    for (i, (mine, theirs)) in driven.iter().zip(target.iter()).enumerate() {
        let mut freq = [0i64; 3];
        let err = error_between(mine, theirs);
        for a in 0..3 {
            // Desired velocity toward the opponent bone, Q16 gain, per-axis
            // speed cap: the powered servo law in integer units.
            let want = (err[a] * i64::from(tune.gain_q16)) >> 16;
            let capped = clamp(
                want,
                -i64::from(tune.max_speed_mm_tick),
                i64::from(tune.max_speed_mm_tick),
            );
            let accel = capped - i64::from(mine.vel_mm_tick[a]);
            let force = accel * i64::from(tune.mass_g) * FORCE_NUM / FORCE_DEN;
            freq[a] = clamp(
                force,
                -i64::from(tune.force_cap_mn),
                i64::from(tune.force_cap_mn),
            );
        }
        // Grip moment for this bone: L1 exerted force times L1 relative pull
        // across the anchor (mN-mm). L1 norms bound the euclidean ones, so
        // this bounds the true vector moment about the grip; rigid tracking
        // errors cancel out of the twist instead of faking a load. Every
        // in-range input keeps the product in i64 with wide margin.
        let mut force_l1: i64 = 0;
        let mut twist_l1: i64 = 0;
        for a in 0..3 {
            force_l1 += if freq[a] < 0 { -freq[a] } else { freq[a] };
            let twist = err[a] - anchor_err[a];
            twist_l1 += if twist < 0 { -twist } else { twist };
        }
        let moment = force_l1 * twist_l1;
        let mut scale = i64::from(Q16_ONE);
        if moment > tune.torque_cap_mn_mm {
            over = true;
            // moment > cap >= 1, so the quotient is well-defined and < Q16.
            scale = tune.torque_cap_mn_mm * i64::from(Q16_ONE) / moment;
        }
        for a in 0..3 {
            out[i][a] = ((freq[a] * scale) >> 16) as i32;
        }
    }
    over
}

/// Per-axis position error between a driven bone and its opponent target, mm.
fn error_between(mine: &BodySample, theirs: &BodySample) -> [i64; 3] {
    [
        i64::from(theirs.pos_mm[0]) - i64::from(mine.pos_mm[0]),
        i64::from(theirs.pos_mm[1]) - i64::from(mine.pos_mm[1]),
        i64::from(theirs.pos_mm[2]) - i64::from(mine.pos_mm[2]),
    ]
}

/// True when the two grip anchors sit farther apart than the snap distance.
/// Squared distances stay in i64 for every in-range input, so no root is needed.
fn anchor_separated(a: &BodySample, b: &BodySample, break_mm: i32) -> bool {
    let mut sum: i64 = 0;
    for axis in 0..3 {
        let d = i64::from(a.pos_mm[axis]) - i64::from(b.pos_mm[axis]);
        sum += d * d;
    }
    sum > i64::from(break_mm) * i64::from(break_mm)
}

/// Range-check one bone sample.
fn check_sample(sample: &BodySample) -> Result<(), GrappleError> {
    for axis in sample.pos_mm {
        if !(-MAX_POS_MM..=MAX_POS_MM).contains(&axis) {
            return Err(GrappleError::BadSample("pos_mm"));
        }
    }
    for axis in sample.vel_mm_tick {
        if !(-MAX_VEL_MM_TICK..=MAX_VEL_MM_TICK).contains(&axis) {
            return Err(GrappleError::BadSample("vel_mm_tick"));
        }
    }
    Ok(())
}

/// Clamp with comparisons only; every caller keeps inputs in i64 range.
fn clamp(v: i64, lo: i64, hi: i64) -> i64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Golden-pinned fold prime: the original literal relaxed one hex digit (an
/// extra zero next to the standard prime), so this digest is not standard
/// FNV-1a. The bytes rule the simulation golden, so the fold keeps this
/// prime until a golden-bump packet standardizes it; do not "fix" it here.
const GRAPPLE_PRIME: u64 = FNV64_PRIME.wrapping_add(16492674416640);

/// Bone index or count to hash word with saturating semantics: attached
/// counts and anchors are at most [`MAX_BONES`], so this matches the old
/// conversion exactly; an absurd value clamps instead of wrapping.
fn count_as_u64(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// Fold little-endian bytes with the golden-pinned prime (the width stays at
/// the call site, so `u32` and `u64` folds can never be confused).
fn fold_le_bytes(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(GRAPPLE_PRIME);
    }
}

/// Fold one u32 as little-endian bytes with the golden-pinned prime.
fn fold_u32(h: &mut u64, v: u32) {
    fold_le_bytes(h, &v.to_le_bytes());
}

/// Fold one i32 as little-endian bytes with the golden-pinned prime.
fn fold_i32(h: &mut u64, v: i32) {
    fold_le_bytes(h, &v.to_le_bytes());
}

/// Fold one u64 as little-endian bytes with the golden-pinned prime.
fn fold_u64(h: &mut u64, v: u64) {
    fold_le_bytes(h, &v.to_le_bytes());
}

/// Fold one i64 as little-endian bytes with the golden-pinned prime.
fn fold_i64(h: &mut u64, v: i64) {
    fold_le_bytes(h, &v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm_tape_bones(count: usize, x0_mm: i32) -> Vec<BodySample> {
        let mut bones = Vec::with_capacity(count);
        for i in 0..count {
            bones.push(BodySample {
                pos_mm: [x0_mm + i as i32 * 500, 3000, 0],
                vel_mm_tick: [0, 0, 0],
            });
        }
        bones
    }

    /// Wrench tape: like the calm tape but each bone starts an extra 40 mm out,
    /// so the grip twists across the anchor instead of translating rigidly.
    fn wrench_tape_bones(count: usize, x0_mm: i32) -> Vec<BodySample> {
        let mut bones = calm_tape_bones(count, x0_mm);
        for (i, bone) in bones.iter_mut().enumerate() {
            bone.pos_mm[0] += i as i32 * 40;
        }
        bones
    }

    fn attach_pair() -> GrapplePair {
        GrapplePair::attach(
            7,
            0,
            42,
            0,
            4,
            ServoTune::default_grip(),
            GrappleParams::default_hold(),
        )
        .expect("default attach")
    }

    /// Run a scripted tape: foe holds still, self integrates under the servo
    /// forces. Returns the final max position error plus the pair hash.
    fn run_calm_tape() -> (i64, u64, GrapplePair) {
        let mut pair = attach_pair();
        let foe = calm_tape_bones(4, 0);
        let mut mine = calm_tape_bones(4, 200);
        for _ in 0..60 {
            let outcome = pair.step(7, &mine, 42, &foe).expect("calm step");
            assert!(!outcome.broken, "calm tape must hold the grip");
            let forces = pair.forces_for(7).expect("side 7").to_vec();
            for (bone, force) in mine.iter_mut().zip(forces.iter()) {
                *bone = integrate_bone(bone, *force, ServoTune::default_grip().mass_g);
            }
        }
        let mut worst: i64 = 0;
        for (mine, theirs) in mine.iter().zip(foe.iter()) {
            for a in 0..3 {
                let d = (mine.pos_mm[a] - theirs.pos_mm[a]) as i64;
                let mag = if d < 0 { -d } else { d };
                if mag > worst {
                    worst = mag;
                }
            }
        }
        let hash = pair.state_hash();
        (worst, hash, pair)
    }

    #[test]
    fn servo_tracks_opponent_within_budget() {
        let (worst, _, pair) = run_calm_tape();
        assert!(
            worst < 50,
            "servo must close a 200 mm gap in 60 ticks, left {worst} mm"
        );
        assert_eq!(pair.tick(), 60);
        assert!(!pair.is_broken());
    }

    #[test]
    fn hash_twice_equal_on_grapple_tape() {
        let (_, hash_a, _) = run_calm_tape();
        let (_, hash_b, _) = run_calm_tape();
        assert_eq!(hash_a, hash_b, "same tape must hash equal twice");
    }

    #[test]
    fn hash_keeps_golden_pinned_prime() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn fold(h: &mut u64, prime: u64, bytes: &[u8]) {
            for b in bytes {
                *h ^= u64::from(*b);
                *h = h.wrapping_mul(prime);
            }
        }
        fn legacy(pair: &GrapplePair) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            fold(&mut h, super::GRAPPLE_PRIME, &pair.lo_id.to_le_bytes());
            fold(&mut h, super::GRAPPLE_PRIME, &pair.hi_id.to_le_bytes());
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.tune.gain_q16.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.tune.max_speed_mm_tick.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.tune.force_cap_mn.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.tune.torque_cap_mn_mm.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.tune.mass_g.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.params.break_after_ticks.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &pair.params.break_distance_mm.to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &super::count_as_u64(pair.lo_anchor).to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &super::count_as_u64(pair.hi_anchor).to_le_bytes(),
            );
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &super::count_as_u64(pair.bone_count).to_le_bytes(),
            );
            fold(&mut h, super::GRAPPLE_PRIME, &pair.tick.to_le_bytes());
            fold(&mut h, super::GRAPPLE_PRIME, &pair.over_ticks.to_le_bytes());
            fold(
                &mut h,
                super::GRAPPLE_PRIME,
                &u64::from(pair.broken).to_le_bytes(),
            );
            fold(&mut h, super::GRAPPLE_PRIME, &pair.break_tick.to_le_bytes());
            for side in [&pair.lo_force, &pair.hi_force] {
                for bone in &side[..pair.bone_count] {
                    for axis in bone {
                        fold(&mut h, super::GRAPPLE_PRIME, &axis.to_le_bytes());
                    }
                }
            }
            h
        }
        // The pinned prime is not the standard one: an owner swap here moves
        // the golden, so it rides a golden-bump packet, never a silent edit.
        assert_ne!(super::GRAPPLE_PRIME, FNV64_PRIME);
        // Fresh attach plus a stepped tape cover zero and nonzero force tables.
        let fresh = attach_pair();
        assert_eq!(fresh.state_hash(), legacy(&fresh));
        let (_, _, stepped) = run_calm_tape();
        assert_eq!(stepped.state_hash(), legacy(&stepped));
        // Byte order pins the id fold: swapped ids must hash differently.
        let mut swapped = attach_pair();
        core::mem::swap(&mut swapped.lo_id, &mut swapped.hi_id);
        assert_ne!(fresh.state_hash(), swapped.state_hash());
    }

    #[test]
    fn arg_order_never_changes_result() {
        let mut fwd = attach_pair();
        let mut rev = attach_pair();
        let a = calm_tape_bones(4, 200);
        let b = calm_tape_bones(4, 0);
        let out_fwd = fwd.step(7, &a, 42, &b).expect("forward step");
        let out_rev = rev.step(42, &b, 7, &a).expect("reversed step");
        assert_eq!(out_fwd, out_rev);
        assert_eq!(fwd.state_hash(), rev.state_hash());
        assert_eq!(fwd.forces_for(7), rev.forces_for(7));
        assert_eq!(fwd.forces_for(42), rev.forces_for(42));
    }

    #[test]
    fn mirrored_tape_mirrors_checkpoints() {
        // Same bodies, fighter IDs swapped: every per-ID checkpoint must follow
        // its fighter, and the pair hash must match exactly.
        let mut first = GrapplePair::attach(
            7,
            0,
            42,
            0,
            4,
            ServoTune::default_grip(),
            GrappleParams::default_hold(),
        )
        .expect("attach");
        let mut second = GrapplePair::attach(
            42,
            0,
            7,
            0,
            4,
            ServoTune::default_grip(),
            GrappleParams::default_hold(),
        )
        .expect("mirrored attach");
        let side_a = calm_tape_bones(4, 200);
        let side_b = calm_tape_bones(4, 0);
        for _ in 0..24 {
            first.step(7, &side_a, 42, &side_b).expect("first");
            second.step(7, &side_a, 42, &side_b).expect("second");
        }
        assert_eq!(first.state_hash(), second.state_hash());
        assert_eq!(first.forces_for(7), second.forces_for(7));
        assert_eq!(first.forces_for(42), second.forces_for(42));
        assert_eq!(first.fighter_ids(), (7, 42));
        assert_eq!(second.fighter_ids(), (7, 42));
    }

    #[test]
    fn torque_cap_binds_force_and_flags_overload() {
        let tune = ServoTune {
            torque_cap_mn_mm: 1,
            ..ServoTune::default_grip()
        };
        let mut pair = GrapplePair::attach(1, 0, 2, 0, 4, tune, GrappleParams::default_hold())
            .expect("attach");
        let mine = wrench_tape_bones(4, 200);
        let foe = calm_tape_bones(4, 0);
        let before = pair.state_hash();
        let outcome = pair.step(1, &mine, 2, &foe).expect("capped step");
        assert!(outcome.over_torque, "1 mN-mm cap must bind");
        assert_eq!(pair.over_ticks(), 1);
        // The anchor bone carries no twist, so it transmits full force; the
        // twisted bones bind at the 1 mN-mm cap instead of saturating.
        let anchor_force = pair.forces_for(1).expect("side")[0];
        assert_eq!(anchor_force[0], -tune.force_cap_mn);
        for side in [1, 2] {
            for bone in pair.forces_for(side).expect("side").iter().skip(1) {
                for axis in bone {
                    let mag = if *axis < 0 { -*axis } else { *axis };
                    assert!(mag < 1000, "torque cap must bind force, saw {axis}");
                    assert!(
                        mag <= tune.force_cap_mn,
                        "force cap still holds under torque bind"
                    );
                }
            }
        }
        assert_ne!(before, pair.state_hash(), "tick must advance the hash");
    }

    #[test]
    fn breakaway_lands_within_posted_budget() {
        let tune = ServoTune {
            torque_cap_mn_mm: 1,
            ..ServoTune::default_grip()
        };
        let params = GrappleParams {
            break_after_ticks: 6,
            break_distance_mm: MAX_BREAK_MM,
        };
        let mut pair = GrapplePair::attach(1, 0, 2, 0, 4, tune, params).expect("attach");
        let mine = wrench_tape_bones(4, 200);
        let foe = calm_tape_bones(4, 0);
        let mut broke_at: Option<u64> = None;
        for _ in 0..20 {
            let outcome = pair.step(1, &mine, 2, &foe).expect("overload step");
            if outcome.broken {
                broke_at = Some(pair.tick() - 1);
                break;
            }
        }
        let tick = broke_at.expect("sustained overload must break the grip");
        assert!(
            tick < 6,
            "break must land within 6 overload ticks, broke at {tick}"
        );
        assert_eq!(pair.break_tick(), tick);
        assert!(pair.is_broken());
        let frozen = pair.state_hash();
        assert_eq!(
            pair.step(1, &mine, 2, &foe),
            Err(GrappleError::AlreadyBroken)
        );
        assert_eq!(frozen, pair.state_hash(), "failed step must change nothing");
    }

    #[test]
    fn separation_snaps_at_once() {
        let mut pair = attach_pair();
        let mut mine = calm_tape_bones(4, 0);
        let foe = calm_tape_bones(4, 0);
        // Yank the grip anchor 10 m away: past the 1.5 m snap distance.
        mine[0].pos_mm[0] += 10_000;
        let outcome = pair.step(7, &mine, 42, &foe).expect("snap step");
        assert!(outcome.broken, "excess separation must snap at once");
        assert_eq!(pair.break_tick(), 0);
    }

    #[test]
    fn snapshot_restore_replays_identically() {
        let mut a = attach_pair();
        let mut b = attach_pair();
        let foe = calm_tape_bones(4, 0);
        let mut mine_a = calm_tape_bones(4, 200);
        for _ in 0..10 {
            a.step(7, &mine_a, 42, &foe).expect("tape a");
            let forces = a.forces_for(7).expect("side 7").to_vec();
            for (bone, force) in mine_a.iter_mut().zip(forces.iter()) {
                *bone = integrate_bone(bone, *force, ServoTune::default_grip().mass_g);
            }
        }
        let snap = a.snapshot();
        // Rollback discipline: the twin adopts the snapshot AND the same bone
        // inputs the original had when it was taken, then resimulates alone.
        b.restore(snap).expect("restore twin");
        let mut mine_b = mine_a.clone();
        assert_eq!(a.state_hash(), b.state_hash());
        for _ in 0..20 {
            a.step(7, &mine_a, 42, &foe).expect("cont a");
            let fa = a.forces_for(7).expect("fa").to_vec();
            for (bone, force) in mine_a.iter_mut().zip(fa.iter()) {
                *bone = integrate_bone(bone, *force, ServoTune::default_grip().mass_g);
            }
            b.step(7, &mine_b, 42, &foe).expect("cont b");
            let fb = b.forces_for(7).expect("fb").to_vec();
            for (bone, force) in mine_b.iter_mut().zip(fb.iter()) {
                *bone = integrate_bone(bone, *force, ServoTune::default_grip().mass_g);
            }
        }
        assert_eq!(mine_a, mine_b, "restored twin must resimulate identically");
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn bad_input_is_typed_and_changes_nothing() {
        assert_eq!(
            GrapplePair::attach(
                5,
                0,
                5,
                0,
                4,
                ServoTune::default_grip(),
                GrappleParams::default_hold()
            )
            .err(),
            Some(GrappleError::DuplicateFighter)
        );
        assert_eq!(
            GrapplePair::attach(
                1,
                0,
                2,
                0,
                0,
                ServoTune::default_grip(),
                GrappleParams::default_hold()
            )
            .err(),
            Some(GrappleError::BadParams("bone_count"))
        );
        assert_eq!(
            GrapplePair::attach(
                1,
                0,
                2,
                0,
                MAX_BONES + 1,
                ServoTune::default_grip(),
                GrappleParams::default_hold()
            )
            .err(),
            Some(GrappleError::BadParams("bone_count"))
        );
        assert_eq!(
            GrapplePair::attach(
                1,
                9,
                2,
                0,
                4,
                ServoTune::default_grip(),
                GrappleParams::default_hold()
            )
            .err(),
            Some(GrappleError::BadBone(9))
        );
        let bad_tune = ServoTune {
            force_cap_mn: 0,
            ..ServoTune::default_grip()
        };
        assert_eq!(
            GrapplePair::attach(1, 0, 2, 0, 4, bad_tune, GrappleParams::default_hold()).err(),
            Some(GrappleError::BadTune("force_cap_mn"))
        );
        let bad_params = GrappleParams {
            break_after_ticks: BREAKAWAY_BUDGET_TICKS + 1,
            ..GrappleParams::default_hold()
        };
        assert_eq!(
            GrapplePair::attach(1, 0, 2, 0, 4, ServoTune::default_grip(), bad_params).err(),
            Some(GrappleError::BadParams("break_after_ticks"))
        );

        let mut pair = attach_pair();
        let frozen = pair.state_hash();
        let good = calm_tape_bones(4, 0);
        let short = calm_tape_bones(3, 0);
        assert_eq!(
            pair.step(7, &short, 42, &good),
            Err(GrappleError::BadBoneCount {
                expected: 4,
                got: 3
            })
        );
        assert_eq!(
            pair.step(7, &good, 99, &good),
            Err(GrappleError::UnknownFighter(99))
        );
        assert_eq!(
            pair.step(99, &good, 42, &good),
            Err(GrappleError::UnknownFighter(99))
        );
        let mut wild = good.clone();
        wild[2].pos_mm[1] = MAX_POS_MM + 1;
        assert_eq!(
            pair.step(7, &wild, 42, &good),
            Err(GrappleError::BadSample("pos_mm"))
        );
        wild = good.clone();
        wild[1].vel_mm_tick[0] = -MAX_VEL_MM_TICK - 1;
        assert_eq!(
            pair.step(7, &good, 42, &wild),
            Err(GrappleError::BadSample("vel_mm_tick"))
        );
        assert_eq!(
            frozen,
            pair.state_hash(),
            "rejected steps must change nothing"
        );
        assert_eq!(pair.forces_for(1234), None);

        let mut other = GrapplePair::attach(
            7,
            0,
            43,
            0,
            4,
            ServoTune::default_grip(),
            GrappleParams::default_hold(),
        )
        .expect("sibling pair");
        assert_eq!(
            other.restore(pair.snapshot()),
            Err(GrappleError::SnapshotMismatch)
        );
        // A broken snapshot never carries the held sentinel and vice versa.
        let mut snap = pair.snapshot();
        snap.broken = true;
        assert_eq!(pair.restore(snap), Err(GrappleError::SnapshotMismatch));
        // integrate_bone refuses a bad mass by returning the input untouched.
        let sample = good[0];
        assert_eq!(integrate_bone(&sample, [1000, 0, 0], 0), sample);
    }
}
