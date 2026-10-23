//! cut: deterministic blade-vs-limb sever resolver.
//!
//! Consumer: the break-tool verbs and the future limb body (`limb.rs`) read
//! [`Cut`] events; the arena match step feeds one [`BladeSwing`] plus the
//! victim's joint capsules per swing. Data owner: `engine-combat`. Units:
//! millimetres for geometry, Q16 fixed point for energy, sharpness, wear and
//! protection (`1.0 == 65536`), swing and joint ids as slots.
//! Determinism scope: [`cut_one`] and [`resolve_swing`] are pure over the
//! tables, the anatomy row, the worn set and the swing bytes in canonical
//! joint-id order; the same inputs give the same cuts on every PC, and
//! shuffled target order resolves identically. Errors: [`CutError`] for bad
//! versions, rows, slots and geometry; lenient solvers short-circuit
//! non-physical inputs to [`CutMiss`] and never panic on user data. Budget:
//! `O(targets)` integer ops per swing over at most [`MAX_TARGETS_PER_SWING`]
//! joints, caller buffers, no heap per tick; one sever decision costs at most
//! `0.15 ms` (see the budget test).
//! Presentation (stump caps, blood) never reads this module: the [`Cut`]
//! event carries the joint, so a slow GPU cannot change an outcome.

use crate::anatomy::{AnatomyTable, JointId, ARMOR_ALL, JOINT_TOTAL};
use crate::sweep::{dot3, sub3, validate_sweep, WeaponSweep, SWEEP_V1};
use engine_core::{splitmix64, Fnv64};

/// Version tag of the shipped cut rows.
pub const CUT_VERSION_V1: u32 = 1;
/// One [`Cut`] per joint per swing at most; the pelvis never severs.
pub const MAX_CUTS_PER_SWING: usize = 16;
/// Most joint capsules resolved for one swing (the full graph).
pub const MAX_TARGETS_PER_SWING: usize = 16;
/// Fixed-point scale for energy, sharpness, wear and protection (Q16).
pub const CUT_Q16: u64 = 65_536;
/// Sharpness `2.0` in Q16: a fresh gladius edge.
pub const MAX_SHARPNESS_Q16: u32 = 131_072;
/// Energy `32.0` in Q16: hardest tabulated swing.
pub const MAX_ENERGY_Q16: u32 = 2_097_152;
/// Wear `1.0` in Q16: a fully dulled edge multiplies energy to zero.
pub const MAX_WEAR_Q16: u32 = 65_536;
/// Sever ramp `4.0` in Q16: chance runs 0 at margin `-2.0`, full at `+2.0`.
pub const SEVER_SPAN_Q16_V1: u32 = 262_144;
/// Strict bound for every position component, in mm (+/-200 m arena scale).
pub const CUT_POS_BOUND_MM: i32 = 200_000;
/// Max blade half-thickness in Q16 validation, in mm.
pub const CUT_BLADE_HALF_MAX_MM: u32 = 500;
/// Max joint-capsule radius in validation, in mm.
pub const CUT_JOINT_RADIUS_MAX_MM: u32 = 500;
/// Conservative contact slack in V1, in mm.
pub const CUT_SLACK_MM_V1: u32 = 4;
/// Longest single-swing blade travel accepted by V1, in mm.
pub const CUT_SWING_LEN_MM_V1: u32 = 6_000;
/// Wide-envelope position bound; beyond this pairs cannot touch (mm).
const ENVELOPE_POS_MM: i32 = 2_000_000;
/// Wide-envelope radius bound; beyond this inputs are not physical (mm).
const ENVELOPE_RADIUS_MM: u32 = 10_000;
/// Quantization for the segment parameters in the contact solve.
const CUT_Q: i64 = 1024;

/// Tunable cut rows: every number the solver reads, in one place.
///
/// Units: millimetres for `contact_slack_mm` and `max_swing_len_mm`, Q16
/// for `sever_span_q16`. `contact_slack_mm` conservatively expands every
/// contact test so quantization can never hide a true graze;
/// `max_swing_len_mm` caps single-swing blade travel; `sever_span_q16` is
/// the margin ramp from first blood (`0`) to certain sever (`65535`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CutTables {
    /// Conservative contact expansion in mm, `0..=16`.
    pub contact_slack_mm: u32,
    /// Longest accepted single-swing blade travel in mm, `100..=12000`.
    pub max_swing_len_mm: u32,
    /// Margin ramp in Q16, `65536..=524288` (`1.0..=8.0`).
    pub sever_span_q16: u32,
}

impl CutTables {
    /// The shipped rows.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            contact_slack_mm: CUT_SLACK_MM_V1,
            max_swing_len_mm: CUT_SWING_LEN_MM_V1,
            sever_span_q16: SEVER_SPAN_Q16_V1,
        }
    }
}

/// The shipped rows as one static row.
pub const CUT_V1: CutTables = CutTables::v1();

/// Versioned cut rows: nonzero version plus validated rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CutTableSet {
    /// Table version; nonzero, 1 for the shipped rows.
    pub version: u32,
    /// The tunable rows resolved under this version.
    pub tables: CutTables,
}

impl CutTableSet {
    /// The shipped rows as version 1.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            version: CUT_VERSION_V1,
            tables: CUT_V1,
        }
    }

    /// Explicit rows with a nonzero version; call [`validate_cut_tables`] first.
    #[must_use]
    pub const fn new(version: u32, tables: CutTables) -> Self {
        Self { version, tables }
    }

    /// Range-check this set; see [`validate_cut_tables`].
    ///
    /// Errors: [`CutError`] naming the first out-of-range row.
    pub fn validate(&self) -> Result<(), CutError> {
        validate_cut_tables(self)
    }
}

/// Bounded cut failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CutError {
    /// Version 0 is reserved for legacy untagged replays, never valid anew.
    BadVersion(u32),
    /// Replay references a table version with no known rows.
    UnknownVersion(u32),
    /// One tunable row or swing field is outside its sane range; culprit
    /// carried. First failure wins, deterministically.
    BadValue {
        /// Table or swing field name, e.g. `"energy_q16"`.
        field: &'static str,
        /// Culprit value (magnitudes for signed fields).
        value: u32,
    },
    /// Attacker and victim are the same slot; blades never cut their owner.
    SelfTarget(u8),
    /// Joint wire value names no joint; culprit carried.
    BadJoint(u8),
    /// The pelvis root is not severable; culprit joint carried.
    NotSeverable(u8),
    /// More targets than [`MAX_TARGETS_PER_SWING`] in one swing.
    TooManyTargets(usize),
    /// Same joint twice in one swing; culprit wire value carried.
    DuplicateJoint(u8),
}

impl core::fmt::Display for CutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(version) => {
                write!(f, "cut table version 0 is reserved ({version})")
            }
            Self::UnknownVersion(version) => {
                write!(f, "unknown cut table version ({version})")
            }
            Self::BadValue { field, value } => {
                write!(f, "cut field {field} out of range ({value})")
            }
            Self::SelfTarget(slot) => {
                write!(f, "cut swing targets its owner ({slot})")
            }
            Self::BadJoint(joint) => {
                write!(f, "cut joint {joint} names no joint")
            }
            Self::NotSeverable(joint) => {
                write!(f, "cut joint {joint} is not severable")
            }
            Self::TooManyTargets(count) => {
                write!(f, "cut swing carries too many targets ({count})")
            }
            Self::DuplicateJoint(joint) => {
                write!(f, "cut swing repeats joint ({joint})")
            }
        }
    }
}

impl std::error::Error for CutError {}

/// Range-check a versioned cut set. First failure wins, deterministically.
///
/// Errors: [`CutError::BadVersion`] for version 0,
/// [`CutError::UnknownVersion`] past V1, [`CutError::BadValue`] for the
/// first out-of-range row.
pub fn validate_cut_tables(set: &CutTableSet) -> Result<(), CutError> {
    if set.version == 0 {
        return Err(CutError::BadVersion(0));
    }
    if set.version != CUT_VERSION_V1 {
        return Err(CutError::UnknownVersion(set.version));
    }
    check_range("contact_slack_mm", set.tables.contact_slack_mm, 0, 16)?;
    check_range("max_swing_len_mm", set.tables.max_swing_len_mm, 100, 12_000)?;
    check_range("sever_span_q16", set.tables.sever_span_q16, 65_536, 524_288)?;
    Ok(())
}

fn check_range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), CutError> {
    if value < min || value > max {
        return Err(CutError::BadValue { field, value });
    }
    Ok(())
}

/// Resolve a replay version tag to its static rows: 1 gives V1, else unknown.
///
/// Determinism scope: pure lookup; the same tag gives the same rows.
#[must_use]
pub fn cut_for_version(version: u32) -> Option<&'static CutTables> {
    match version {
        CUT_VERSION_V1 => Some(&CUT_V1),
        _ => None,
    }
}

/// One blade swing for the tick: the striking edge path plus steel.
///
/// `p0_mm` is where the edge starts the tick, `p1_mm` where it ends it;
/// the segment between them is tested against every joint capsule, so a
/// thin fast edge cannot tunnel past a wrist between ticks. `energy_q16`
/// is swing energy in Q16, `sharpness_q16` the edge (`0..=2.0` in Q16),
/// `wear_q16` the dulled fraction (`0..=1.0` in Q16, `0` is fresh).
/// `swing` is the caller sequence number and seeds the deterministic roll.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BladeSwing {
    /// Attacking slot.
    pub attacker: u8,
    /// Victim slot; must differ from `attacker`.
    pub victim: u8,
    /// Caller sequence number; seeds the deterministic sever roll.
    pub swing: u32,
    /// Edge start of tick in mm.
    pub p0_mm: [i32; 3],
    /// Edge end of tick in mm.
    pub p1_mm: [i32; 3],
    /// Edge half-thickness in mm, `0..=500`.
    pub blade_half_mm: u32,
    /// Swing energy in Q16, `0..=2097152` (`0..=32.0`).
    pub energy_q16: u32,
    /// Edge sharpness in Q16, `0..=131072` (`0..=2.0`).
    pub sharpness_q16: u32,
    /// Dulled fraction in Q16, `0..=65536` (`0..=1.0`).
    pub wear_q16: u32,
}

/// One victim joint as a segment capsule in mm.
///
/// The spine runs `q0_mm -> q1_mm` plus `radius_mm`; the blade segment is
/// tested against it with the table slack added to the radii.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct JointTarget {
    /// Which joint this capsule guards.
    pub joint: JointId,
    /// Capsule spine start in mm.
    pub q0_mm: [i32; 3],
    /// Capsule spine end in mm.
    pub q1_mm: [i32; 3],
    /// Capsule radius in mm, `1..=500`.
    pub radius_mm: u32,
}

/// One severed joint: exactly 16 bytes on the wire.
///
/// Layout is `swing` (4) plus attacker/victim/joint/flags (4) plus energy
/// (4) plus protection (4). `energy_q16` is the effective edge energy that
/// won, `protection_q16` the joint resistance it beat (both Q16, saturated
/// to `u32`), so tapes stay byte-comparable across PCs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cut {
    /// Caller sequence number, copied from the swing.
    pub swing: u32,
    /// Attacking slot.
    pub attacker: u8,
    /// Victim slot.
    pub victim: u8,
    /// Severed joint wire value (`JointId::as_u8`).
    pub joint: u8,
    /// Reserved flags, always zero in V1.
    pub flags: u8,
    /// Winning effective energy in Q16, saturated.
    pub energy_q16: u32,
    /// Joint resistance in Q16, saturated.
    pub protection_q16: u32,
}

/// Why a joint did not sever; every miss names its gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CutMiss {
    /// Attacker and victim are the same slot.
    SelfTarget,
    /// The pelvis root can never detach.
    NotSeverable,
    /// Edge path stays outside edge plus capsule plus slack, the pair is
    /// beyond the physical envelope, or the travel is non-physical.
    OutOfReach,
    /// Edge touched but the deterministic roll held under the margin odds.
    Held,
}

/// Pure outcome of one edge against one joint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CutOutcome {
    /// Edge touched and the roll beat the margin odds.
    Sever(Cut),
    /// No sever; the gate that refused is named.
    Miss(CutMiss),
}

/// Effective edge energy in Q16: `energy * sharpness * (1 - wear)`.
///
/// All inputs are Q16; the product runs in `u64` so the V1 maxima
/// (`32.0 * 2.0 * 1.0`) never wrap. Determinism scope: pure integer
/// math, identical on every PC.
#[must_use]
pub fn effective_q16(swing: &BladeSwing) -> u64 {
    let sharp = u64::from(swing.sharpness_q16);
    let energy = u64::from(swing.energy_q16);
    let fresh = CUT_Q16.saturating_sub(u64::from(swing.wear_q16));
    energy
        .saturating_mul(sharp)
        .saturating_div(CUT_Q16)
        .saturating_mul(fresh)
        .saturating_div(CUT_Q16)
}

/// Sever odds in `0..=65536` from effective energy and joint resistance.
///
/// `margin = effective - protection` in Q16; odds ramp linearly from `0`
/// at `margin <= -span/2` to `65536` at `margin >= +span/2`, so raising
/// `energy * sharpness` never lowers the odds. Determinism scope: pure
/// `i128` math, no floats.
#[must_use]
pub fn chance_q16(effective: u64, protection: u64, span_q16: u32) -> u32 {
    let span = i128::from(span_q16);
    if span <= 0 {
        return 0;
    }
    let margin = i128::from(effective) - i128::from(protection);
    let half = span / 2;
    if margin <= -half {
        return 0;
    }
    if margin >= half {
        return 65_536;
    }
    let shifted = margin + half;
    let odds = shifted.saturating_mul(65_536).saturating_div(span);
    u32::try_from(odds).unwrap_or(u32::MAX).min(65_536)
}

/// Deterministic sever roll in `0..=65535` for one swing and joint.
///
/// The seed folds swing id, slots and joint through [`Fnv64`] then
/// [`splitmix64`]; the upper 16 bits are the roll, so every PC draws the
/// same number and tapes replay byte-equal.
#[must_use]
pub fn roll_q16(swing: &BladeSwing, joint: JointId) -> u32 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(swing.swing);
    hasher.write_u8(swing.attacker);
    hasher.write_u8(swing.victim);
    hasher.write_u8(joint.as_u8());
    hasher.write_u32(swing.energy_q16);
    hasher.write_u32(swing.sharpness_q16);
    hasher.write_u32(swing.wear_q16);
    let mixed = splitmix64(hasher.finish());
    // Masked to 16 bits so the conversion always fits; saturate rather than
    // panic so a future mask edit cannot introduce a solver panic.
    u32::try_from((mixed >> 48) & 0xFFFF).unwrap_or(u32::MAX)
}

/// Saturate a `u64` Q16 value into `u32` for the wire event.
#[must_use]
pub fn saturate_q16(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Range-check one swing against the table travel bound.
///
/// Errors: [`CutError::SelfTarget`] for owner swings,
/// [`CutError::BadValue`] for the first out-of-range field.
pub fn validate_swing(swing: &BladeSwing, tables: &CutTables) -> Result<(), CutError> {
    if swing.attacker == swing.victim {
        return Err(CutError::SelfTarget(swing.attacker));
    }
    let mut axis: usize = 0;
    while axis < 3 {
        let low = swing.p0_mm[axis];
        if !(-CUT_POS_BOUND_MM..=CUT_POS_BOUND_MM).contains(&low) {
            return Err(CutError::BadValue {
                field: "swing_p0_mm",
                value: low.unsigned_abs(),
            });
        }
        let high = swing.p1_mm[axis];
        if !(-CUT_POS_BOUND_MM..=CUT_POS_BOUND_MM).contains(&high) {
            return Err(CutError::BadValue {
                field: "swing_p1_mm",
                value: high.unsigned_abs(),
            });
        }
        axis += 1;
    }
    check_range(
        "blade_half_mm",
        swing.blade_half_mm,
        0,
        CUT_BLADE_HALF_MAX_MM,
    )?;
    check_range("energy_q16", swing.energy_q16, 0, MAX_ENERGY_Q16)?;
    check_range("sharpness_q16", swing.sharpness_q16, 0, MAX_SHARPNESS_Q16)?;
    check_range("wear_q16", swing.wear_q16, 0, MAX_WEAR_Q16)?;
    let dx = i64::from(swing.p1_mm[0]) - i64::from(swing.p0_mm[0]);
    let dy = i64::from(swing.p1_mm[1]) - i64::from(swing.p0_mm[1]);
    let dz = i64::from(swing.p1_mm[2]) - i64::from(swing.p0_mm[2]);
    let max = i64::from(tables.max_swing_len_mm);
    if dx * dx + dy * dy + dz * dz > max * max {
        let squared = dx * dx + dy * dy + dz * dz;
        // Sum of squares is non-negative so the conversion fits; saturate
        // rather than panic to keep validation total.
        let dist = u64::try_from(squared).unwrap_or(u64::MAX).isqrt();
        return Err(CutError::BadValue {
            field: "swing_len_mm",
            value: saturate_q16(dist),
        });
    }
    Ok(())
}

/// Range-check one joint capsule.
///
/// Errors: [`CutError::NotSeverable`] for the pelvis root,
/// [`CutError::BadValue`] for the first out-of-range field.
pub fn validate_target(target: &JointTarget) -> Result<(), CutError> {
    if !target.joint.is_severable() {
        return Err(CutError::NotSeverable(target.joint.as_u8()));
    }
    let mut axis: usize = 0;
    while axis < 3 {
        let low = target.q0_mm[axis];
        if !(-CUT_POS_BOUND_MM..=CUT_POS_BOUND_MM).contains(&low) {
            return Err(CutError::BadValue {
                field: "joint_q0_mm",
                value: low.unsigned_abs(),
            });
        }
        let high = target.q1_mm[axis];
        if !(-CUT_POS_BOUND_MM..=CUT_POS_BOUND_MM).contains(&high) {
            return Err(CutError::BadValue {
                field: "joint_q1_mm",
                value: high.unsigned_abs(),
            });
        }
        axis += 1;
    }
    check_range(
        "joint_radius_mm",
        target.radius_mm,
        1,
        CUT_JOINT_RADIUS_MAX_MM,
    )?;
    Ok(())
}

/// Continuous outcome of one edge against one joint.
///
/// Total: never panics. Gates run in fixed order (self, severable,
/// envelope, travel, contact, odds), so every miss names exactly one gate.
/// Inputs beyond the wide physical envelope short-circuit to
/// [`CutMiss::OutOfReach`]; strict callers use the validators for typed
/// errors instead. Joint order never matters here; [`resolve_swing`]
/// canonicalizes multi-joint output.
pub fn cut_one(
    swing: &BladeSwing,
    target: &JointTarget,
    anatomy: &AnatomyTable,
    worn: u8,
    tables: &CutTables,
) -> CutOutcome {
    if swing.attacker == swing.victim {
        return CutOutcome::Miss(CutMiss::SelfTarget);
    }
    if !target.joint.is_severable() {
        return CutOutcome::Miss(CutMiss::NotSeverable);
    }
    if !edge_reaches(swing, target, tables) {
        return CutOutcome::Miss(CutMiss::OutOfReach);
    }
    let masked_worn = worn & ARMOR_ALL;
    let protection = anatomy.protection_q16(target.joint, masked_worn);
    let effective = effective_q16(swing);
    let odds = chance_q16(effective, protection, tables.sever_span_q16);
    let roll = roll_q16(swing, target.joint);
    if roll < odds {
        CutOutcome::Sever(Cut {
            swing: swing.swing,
            attacker: swing.attacker,
            victim: swing.victim,
            joint: target.joint.as_u8(),
            flags: 0,
            energy_q16: saturate_q16(effective),
            protection_q16: saturate_q16(protection),
        })
    } else {
        CutOutcome::Miss(CutMiss::Held)
    }
}

/// Whether the edge reaches one joint capsule: the envelope, the travel bound
/// and the contact gate (blade half-thickness plus capsule radius plus the
/// table slack). Pure integer, shared by [`cut_one`] and [`plate_struck`].
#[must_use]
pub fn edge_reaches(swing: &BladeSwing, target: &JointTarget, tables: &CutTables) -> bool {
    if !in_envelope(swing, target) {
        return false;
    }
    let dx = i64::from(swing.p1_mm[0]) - i64::from(swing.p0_mm[0]);
    let dy = i64::from(swing.p1_mm[1]) - i64::from(swing.p0_mm[1]);
    let dz = i64::from(swing.p1_mm[2]) - i64::from(swing.p0_mm[2]);
    let max = i64::from(tables.max_swing_len_mm);
    if dx * dx + dy * dy + dz * dz > max * max {
        return false;
    }
    let radius_sum = u64::from(swing.blade_half_mm)
        .saturating_add(u64::from(target.radius_mm))
        .saturating_add(u64::from(tables.contact_slack_mm));
    let radius_mm = u32::try_from(radius_sum).unwrap_or(u32::MAX);
    edge_touches_capsule(
        swing.p0_mm,
        swing.p1_mm,
        target.q0_mm,
        target.q1_mm,
        radius_mm,
    )
}

/// Flat block of a worn plate in Q16 (FEEL4): `64.0`, the strongest edge the
/// table can land (`MAX_ENERGY_Q16 * MAX_SHARPNESS_Q16 / CUT_Q16`, pinned by
/// a test). An edge at or under the block is stopped by plate; past it the
/// blow would cut through, and the penetration reduction waits for weapon
/// energy (FEEL1 mass). The arena lands full-force blades, so today every
/// blow on plate is stopped.
pub const PLATE_BLOCK_Q16: u64 = 4_194_304;

/// Plate check (FEEL4): the first plated joint, in wire order, that the
/// edge reaches, when the edge is under the plate's flat block. `None` when
/// the blow carries past the block or no plated joint is reached, so the
/// caller resolves the blow with [`resolve_swing`]. Pure, no heap.
#[must_use]
pub fn plate_struck(
    swing: &BladeSwing,
    targets: &[JointTarget],
    anatomy: &AnatomyTable,
    worn: u8,
    tables: &CutTables,
) -> Option<JointId> {
    if effective_q16(swing) > PLATE_BLOCK_Q16 {
        return None;
    }
    let mut struck: Option<JointId> = None;
    for target in targets {
        if !anatomy.plated(target.joint, worn) || !edge_reaches(swing, target, tables) {
            continue;
        }
        let earlier = struck.is_some_and(|hit| hit.as_u8() <= target.joint.as_u8());
        if !earlier {
            struck = Some(target.joint);
        }
    }
    struck
}

/// Strict one-pair solve: validates the swing and the capsule, then solves.
///
/// Errors: [`CutError`] for bad rows, self-targets, unseverable joints and
/// out-of-range geometry; a valid touch with losing odds is `Ok(Miss(Held))`.
pub fn try_cut_one(
    swing: &BladeSwing,
    target: &JointTarget,
    anatomy: &AnatomyTable,
    worn: u8,
    tables: &CutTables,
) -> Result<CutOutcome, CutError> {
    validate_cut_tables(&CutTableSet::new(CUT_VERSION_V1, *tables))?;
    anatomy.validate().map_err(|_| CutError::BadValue {
        field: "anatomy",
        value: 0,
    })?;
    validate_swing(swing, tables)?;
    validate_target(target)?;
    Ok(cut_one(swing, target, anatomy, worn, tables))
}

/// Resolve one swing against every joint capsule into the caller buffer.
///
/// Deterministic order: targets sort by joint wire value and dedupe by
/// joint (first-in-slice wins), so shuffled inputs give identical cuts.
/// Emits at most one [`Cut`] per joint in ascending joint order, writes at
/// most `out.len()` cuts and returns the count. No heap, no floats.
pub fn resolve_swing(
    swing: &BladeSwing,
    targets: &[JointTarget],
    anatomy: &AnatomyTable,
    worn: u8,
    tables: &CutTables,
    out: &mut [Cut; MAX_CUTS_PER_SWING],
) -> usize {
    let mut order: [usize; MAX_TARGETS_PER_SWING] = [0; MAX_TARGETS_PER_SWING];
    let mut kept: usize = 0;
    let mut scan: usize = 0;
    while scan < targets.len() && kept < order.len() {
        let wire = targets[scan].joint.as_u8();
        let mut seen = false;
        let mut probe: usize = 0;
        while probe < kept {
            if targets[order[probe]].joint.as_u8() == wire {
                seen = true;
                break;
            }
            probe += 1;
        }
        if !seen {
            let mut slot = kept;
            while slot > 0 && targets[order[slot - 1]].joint.as_u8() > wire {
                order[slot] = order[slot - 1];
                slot -= 1;
            }
            order[slot] = scan;
            kept += 1;
        }
        scan += 1;
    }
    let mut emitted: usize = 0;
    let mut cursor: usize = 0;
    while cursor < kept {
        let target = &targets[order[cursor]];
        if let CutOutcome::Sever(hit) = cut_one(swing, target, anatomy, worn, tables) {
            if emitted < out.len() {
                out[emitted] = hit;
                emitted += 1;
            }
        }
        cursor += 1;
    }
    emitted
}

/// Strict swing resolve: validates counts, duplicates and geometry first.
///
/// Errors: [`CutError`] for bad tables, self-targets, too many or repeated
/// joints and out-of-range fields; a valid swing that the odds refuse is
/// `Ok(0)`.
pub fn try_resolve_swing(
    swing: &BladeSwing,
    targets: &[JointTarget],
    anatomy: &AnatomyTable,
    worn: u8,
    tables: &CutTables,
    out: &mut [Cut; MAX_CUTS_PER_SWING],
) -> Result<usize, CutError> {
    validate_cut_tables(&CutTableSet::new(CUT_VERSION_V1, *tables))?;
    anatomy.validate().map_err(|_| CutError::BadValue {
        field: "anatomy",
        value: 0,
    })?;
    validate_swing(swing, tables)?;
    if targets.len() > MAX_TARGETS_PER_SWING {
        return Err(CutError::TooManyTargets(targets.len()));
    }
    let mut cursor: usize = 0;
    while cursor < targets.len() {
        let mut probe: usize = cursor + 1;
        while probe < targets.len() {
            if targets[cursor].joint == targets[probe].joint {
                return Err(CutError::DuplicateJoint(targets[cursor].joint.as_u8()));
            }
            probe += 1;
        }
        validate_target(&targets[cursor])?;
        cursor += 1;
    }
    Ok(resolve_swing(swing, targets, anatomy, worn, tables, out))
}

/// Content digest over cuts in slice order.
///
/// The break replay corpus pins this: the same cuts give the same digest
/// on every PC, and any cut change flips it.
#[must_use]
pub fn digest_cuts(cuts: &[Cut]) -> u64 {
    let mut hasher = Fnv64::new();
    // Corpus length always fits u32 in practice; saturate to keep the digest total.
    hasher.write_u32(u32::try_from(cuts.len()).unwrap_or(u32::MAX));
    for hit in cuts {
        hasher.write_u32(hit.swing);
        hasher.write_u8(hit.attacker);
        hasher.write_u8(hit.victim);
        hasher.write_u8(hit.joint);
        hasher.write_u8(hit.flags);
        hasher.write_u32(hit.energy_q16);
        hasher.write_u32(hit.protection_q16);
    }
    hasher.finish()
}

/// Wide-envelope guard: all solver arithmetic provably fits past this gate.
fn in_envelope(swing: &BladeSwing, target: &JointTarget) -> bool {
    let mut axis: usize = 0;
    while axis < 3 {
        if swing.p0_mm[axis] < -ENVELOPE_POS_MM || swing.p0_mm[axis] > ENVELOPE_POS_MM {
            return false;
        }
        if swing.p1_mm[axis] < -ENVELOPE_POS_MM || swing.p1_mm[axis] > ENVELOPE_POS_MM {
            return false;
        }
        if target.q0_mm[axis] < -ENVELOPE_POS_MM || target.q0_mm[axis] > ENVELOPE_POS_MM {
            return false;
        }
        if target.q1_mm[axis] < -ENVELOPE_POS_MM || target.q1_mm[axis] > ENVELOPE_POS_MM {
            return false;
        }
        axis += 1;
    }
    if swing.blade_half_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    if target.radius_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    true
}

/// Clamp `num / den` to `[0, CUT_Q]` with round-to-nearest; `den <= 0`
/// yields 0 so degenerate edges never divide by zero.
fn clamp_edge_q(num: i128, den: i128) -> i64 {
    if den <= 0 {
        return 0;
    }
    if num <= 0 {
        return 0;
    }
    if num >= den {
        return CUT_Q;
    }
    let scaled = (num * i128::from(CUT_Q) + den / 2) / den;
    if scaled > i128::from(CUT_Q) {
        CUT_Q
    } else {
        // Guarded above to `0..=CUT_Q`, so the conversion fits; saturate to
        // keep the edge projection total.
        i64::try_from(scaled).unwrap_or(CUT_Q)
    }
}

/// True when the quantized distance at `(s_q, t_q)` fits inside `radius_q`.
fn edge_dist_le(
    p0: [i32; 3],
    edge: [i64; 3],
    s_q: i64,
    q0: [i32; 3],
    spine: [i64; 3],
    t_q: i64,
    radius_q: i128,
) -> bool {
    let steps = i128::from(CUT_Q);
    let dx = (i128::from(p0[0]) - i128::from(q0[0])) * steps
        + i128::from(s_q) * i128::from(edge[0])
        - i128::from(t_q) * i128::from(spine[0]);
    let dy = (i128::from(p0[1]) - i128::from(q0[1])) * steps
        + i128::from(s_q) * i128::from(edge[1])
        - i128::from(t_q) * i128::from(spine[1]);
    let dz = (i128::from(p0[2]) - i128::from(q0[2])) * steps
        + i128::from(s_q) * i128::from(edge[2])
        - i128::from(t_q) * i128::from(spine[2]);
    dx * dx + dy * dy + dz * dz <= radius_q * radius_q
}

/// True when the edge `p0 -> p1` touches the capsule spine `q0 -> q1`
/// plus `radius_mm`. Exact region solve with quantized candidates: the
/// true minimum lives in one of the nine regions, so the rounded best is
/// within two millimetres of it and the table slack covers the gap.
fn edge_touches_capsule(
    p0: [i32; 3],
    p1: [i32; 3],
    q0: [i32; 3],
    q1: [i32; 3],
    radius_mm: u32,
) -> bool {
    let edge = sub3(p1, p0);
    let spine = sub3(q1, q0);
    let offset = sub3(p0, q0);
    let along_edge = dot3(edge, edge);
    let across = dot3(edge, spine);
    let along_spine = dot3(spine, spine);
    let edge_offset = dot3(edge, offset);
    let spine_offset = dot3(spine, offset);
    let det = along_edge * along_spine - across * across;
    let (best_s, best_t) = if det > 0 {
        (
            clamp_edge_q(across * spine_offset - along_spine * edge_offset, det),
            clamp_edge_q(along_edge * spine_offset - across * edge_offset, det),
        )
    } else {
        (0, clamp_edge_q(spine_offset, along_spine))
    };
    let probes: [[i64; 2]; 9] = [
        [best_s, best_t],
        [0, clamp_edge_q(spine_offset, along_spine)],
        [CUT_Q, clamp_edge_q(spine_offset + across, along_spine)],
        [clamp_edge_q(-edge_offset, along_edge), 0],
        [clamp_edge_q(across - edge_offset, along_edge), CUT_Q],
        [0, 0],
        [0, CUT_Q],
        [CUT_Q, 0],
        [CUT_Q, CUT_Q],
    ];
    let radius_q = i128::from(radius_mm) * i128::from(CUT_Q);
    let mut probe: usize = 0;
    while probe < probes.len() {
        if edge_dist_le(
            p0,
            edge,
            probes[probe][0],
            q0,
            spine,
            probes[probe][1],
            radius_q,
        ) {
            return true;
        }
        probe += 1;
    }
    false
}

// ---- baked per-move hit tracks (EG42) --------------------------------------

/// Wire version of the baked hit track: version 1 is the first three-move
/// set. A reader refuses any other tag.
pub const HIT_TRACK_VERSION_V1: u32 = 1;
/// Most moves one baked track may carry.
pub const MAX_HIT_TRACK_MOVES: usize = 8;
/// Longest move baked, in 60 Hz ticks (60 s).
pub const MAX_HIT_TRACK_TICKS: usize = 60;

/// One baked tick of one attack move: the blade edge segment plus every bone
/// capsule, in integer millimetres.
///
/// `bones` is in [`JointId`] wire order, one capsule per joint
/// ([`JOINT_TOTAL`]). Rows are in the attacker's local frame at the clip
/// root; the caller adds the root pose. The bake lives in
/// `engine_anim::hit_track`; this crate reads `(move, tick)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HitTrackRow {
    /// Tick inside the move, `0` at the press.
    pub tick: u16,
    /// Blade edge start of this tick, mm.
    pub p0_mm: [i32; 3],
    /// Blade edge end of this tick, mm.
    pub p1_mm: [i32; 3],
    /// Blade half-thickness, mm.
    pub blade_half_mm: u32,
    /// Bone capsules in [`JointId`] wire order.
    pub bones: [JointTarget; JOINT_TOTAL],
}

impl HitTrackRow {
    /// The row's blade edge as a rules-v2 sweep for `attacker`.
    #[must_use]
    pub fn blade(&self, attacker: u8) -> WeaponSweep {
        WeaponSweep {
            attacker,
            p0_mm: self.p0_mm,
            p1_mm: self.p1_mm,
            blade_half_mm: self.blade_half_mm,
        }
    }
}

/// One baked attack move: its stable id, clip short name and tick rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HitMove {
    /// Stable move id used by the `(move, tick)` lookup.
    pub id: u16,
    /// Clip short name for receipts (`attack_1h_1`).
    pub name: &'static str,
    /// One row per tick, `rows[i].tick == i`.
    pub rows: Vec<HitTrackRow>,
}

/// Baked per-move hit tracks: the authoritative integer-mm blade path and
/// bone capsules the fight step and cut resolver read by `(move, tick)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HitTrack {
    /// Wire version; [`HIT_TRACK_VERSION_V1`] is the shipped set.
    pub version: u32,
    /// Moves in id order; each owns its tick rows.
    pub moves: Vec<HitMove>,
}

impl HitTrack {
    /// One move's row at `tick`, or `None` when either is off the track.
    #[must_use]
    pub fn row(&self, move_id: u16, tick: u16) -> Option<&HitTrackRow> {
        self.moves
            .iter()
            .find(|mv| mv.id == move_id)?
            .rows
            .get(usize::from(tick))
    }

    /// Range-check the wire: version, move ids, tick order, blade geometry
    /// and every capsule. Reads only; the same bytes give the same result.
    ///
    /// # Errors
    /// [`CutError::BadVersion`] for another wire tag; [`CutError::BadValue`]
    /// for the first shape or geometry failure.
    pub fn validate(&self) -> Result<(), CutError> {
        if self.version != HIT_TRACK_VERSION_V1 {
            return Err(CutError::BadVersion(self.version));
        }
        let count = self.moves.len();
        if count == 0 || count > MAX_HIT_TRACK_MOVES {
            return Err(CutError::BadValue {
                field: "hit_track_moves",
                value: u32::try_from(count).unwrap_or(u32::MAX),
            });
        }
        let mut previous: Option<u16> = None;
        for mv in &self.moves {
            if let Some(last) = previous {
                if mv.id <= last {
                    return Err(CutError::BadValue {
                        field: "hit_track_move",
                        value: u32::from(mv.id),
                    });
                }
            }
            previous = Some(mv.id);
            if mv.name.is_empty() {
                return Err(CutError::BadValue {
                    field: "hit_track_name",
                    value: 0,
                });
            }
            let rows = mv.rows.len();
            if rows == 0 || rows > MAX_HIT_TRACK_TICKS {
                return Err(CutError::BadValue {
                    field: "hit_track_ticks",
                    value: u32::try_from(rows).unwrap_or(u32::MAX),
                });
            }
            for (tick, row) in mv.rows.iter().enumerate() {
                if usize::from(row.tick) != tick {
                    return Err(CutError::BadValue {
                        field: "hit_track_tick",
                        value: u32::from(row.tick),
                    });
                }
                if validate_sweep(&row.blade(0), 1, &SWEEP_V1).is_err() {
                    return Err(CutError::BadValue {
                        field: "hit_track_blade",
                        value: 0,
                    });
                }
                for (index, bone) in row.bones.iter().enumerate() {
                    if usize::from(bone.joint.as_u8()) != index {
                        return Err(CutError::BadValue {
                            field: "hit_track_bones",
                            value: u32::try_from(index).unwrap_or(u32::MAX),
                        });
                    }
                    validate_capsule(bone)?;
                }
            }
        }
        Ok(())
    }

    /// Stable digest over the wire: same rows give the same digest on every
    /// PC. Nothing hashes it in the golden today; see `docs/HASH-LAW.md`.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_u32(self.version);
        hash.write_len(self.moves.len());
        for mv in &self.moves {
            hash.write_u16(mv.id);
            hash.write_str(mv.name);
            hash.write_len(mv.rows.len());
            for row in &mv.rows {
                hash.write_u16(row.tick);
                for value in row.p0_mm.iter().chain(row.p1_mm.iter()) {
                    hash.write_i32(*value);
                }
                hash.write_u32(row.blade_half_mm);
                for bone in &row.bones {
                    hash.write_u8(bone.joint.as_u8());
                    for value in bone.q0_mm.iter().chain(bone.q1_mm.iter()) {
                        hash.write_i32(*value);
                    }
                    hash.write_u32(bone.radius_mm);
                }
            }
        }
        hash.finish()
    }
}

/// Range-check one baked capsule, including the non-severable pelvis root.
fn validate_capsule(capsule: &JointTarget) -> Result<(), CutError> {
    for value in capsule.q0_mm.iter().chain(capsule.q1_mm.iter()) {
        if !(-CUT_POS_BOUND_MM..=CUT_POS_BOUND_MM).contains(value) {
            return Err(CutError::BadValue {
                field: "hit_track_q_mm",
                value: value.unsigned_abs(),
            });
        }
    }
    check_range(
        "hit_track_radius_mm",
        capsule.radius_mm,
        1,
        CUT_JOINT_RADIUS_MAX_MM,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anatomy::{ARMOR_GALEA, ARMOR_LORICA};
    use engine_core::Rng;

    const ENERGY_HARD_Q16: u32 = 589_824;
    const SHARP_FRESH_Q16: u32 = 65_536;

    fn tables() -> CutTables {
        CUT_V1
    }

    fn rome() -> AnatomyTable {
        AnatomyTable::rome_kit()
    }

    fn swing(energy: u32, sharp: u32, wear: u32, id: u32) -> BladeSwing {
        BladeSwing {
            attacker: 0,
            victim: 1,
            swing: id,
            p0_mm: [-100, 0, 0],
            p1_mm: [100, 0, 0],
            blade_half_mm: 10,
            energy_q16: energy,
            sharpness_q16: sharp,
            wear_q16: wear,
        }
    }

    fn limb(joint: JointId) -> JointTarget {
        JointTarget {
            joint,
            q0_mm: [-20, -80, 0],
            q1_mm: [20, 80, 0],
            radius_mm: 60,
        }
    }

    fn resolve_all(swing: &BladeSwing, targets: &[JointTarget], worn: u8) -> (Vec<Cut>, usize) {
        let anatomy = rome();
        let table = tables();
        let mut out = [Cut {
            swing: 0,
            attacker: 0,
            victim: 0,
            joint: 0,
            flags: 0,
            energy_q16: 0,
            protection_q16: 0,
        }; MAX_CUTS_PER_SWING];
        let count = resolve_swing(swing, targets, &anatomy, worn, &table, &mut out);
        (out[..count].to_vec(), count)
    }

    #[test]
    fn v1_rows_match_consts_and_resolve_by_version() {
        assert_eq!(CUT_V1.contact_slack_mm, CUT_SLACK_MM_V1);
        assert_eq!(CUT_V1.max_swing_len_mm, CUT_SWING_LEN_MM_V1);
        assert_eq!(CUT_V1.sever_span_q16, SEVER_SPAN_Q16_V1);
        assert_eq!(CutTableSet::v1().version, CUT_VERSION_V1);
        assert_eq!(CutTableSet::v1().validate(), Ok(()));
        assert_eq!(cut_for_version(1), Some(&CUT_V1));
        assert_eq!(cut_for_version(9), None);
        assert_eq!(core::mem::size_of::<Cut>(), 16);
    }

    #[test]
    fn manica_stops_a_swing_a_bare_forearm_loses() {
        let table = tables();
        let anatomy = rome();
        // Bare elbow severs under a hard fresh swing for at least one seed.
        let bare_hit = (0..64u32).any(|seed| {
            let hit = swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, seed);
            matches!(
                cut_one(&hit, &limb(JointId::ElbowR), &anatomy, 0, &table),
                CutOutcome::Sever(_)
            )
        });
        assert!(bare_hit, "hard fresh swing must sever a bare elbow");
        // The same swing under a worn manica never severs: the guard adds
        // 5.0 Q16 over tissue, past the 4.0 ramp from this energy.
        let mut guarded_wins = 0;
        for seed in 0..64u32 {
            let hit = swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, seed);
            if let CutOutcome::Sever(_) = cut_one(
                &hit,
                &limb(JointId::ElbowR),
                &anatomy,
                crate::anatomy::ARMOR_MANICA,
                &table,
            ) {
                guarded_wins += 1;
            }
        }
        assert_eq!(guarded_wins, 0, "manica must stop the tabulated swing");
        // The wrong piece on the wrong joint guards nothing.
        let bare = anatomy.protection_q16(JointId::ElbowR, 0);
        assert_eq!(
            anatomy.protection_q16(JointId::ElbowR, crate::anatomy::ARMOR_GREAVE),
            bare
        );
    }

    #[test]
    fn odds_rise_monotonically_with_energy_times_sharpness() {
        let table = tables();
        let anatomy = rome();
        let joint = JointId::KneeR;
        let protection = anatomy.protection_q16(joint, 0);
        // Closed-form odds never fall as the product grows.
        let mut last = 0u32;
        let mut energy = 0u32;
        while energy <= MAX_ENERGY_Q16 {
            let probe = swing(energy, SHARP_FRESH_Q16, 0, 11);
            let effective = effective_q16(&probe);
            let odds = chance_q16(effective, protection, table.sever_span_q16);
            assert!(odds >= last, "odds fell at energy {energy}");
            last = odds;
            energy += 65_536;
        }
        // Endpoints: spent edge holds, hard edge presses past half odds.
        assert_eq!(chance_q16(0, protection, table.sever_span_q16), 0);
        let hard = effective_q16(&swing(MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, 0, 11));
        assert!(chance_q16(hard, protection, table.sever_span_q16) > 32_768);
        // Wear only dulls: a fully worn edge carries no energy.
        assert_eq!(
            effective_q16(&swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, MAX_WEAR_Q16, 3)),
            0
        );
        // Counted severs over a seed batch never fall with energy either.
        let batch = |energy: u32| {
            let mut wins = 0;
            for seed in 0..128u32 {
                let probe = swing(energy, SHARP_FRESH_Q16, 0, seed);
                if matches!(
                    cut_one(&probe, &limb(joint), &anatomy, 0, &table),
                    CutOutcome::Sever(_)
                ) {
                    wins += 1;
                }
            }
            wins
        };
        assert!(batch(ENERGY_HARD_Q16) >= batch(ENERGY_HARD_Q16 / 2));
        assert!(batch(ENERGY_HARD_Q16 / 2) >= batch(0));
    }

    #[test]
    fn ten_thousand_seeded_swings_replay_byte_equal() {
        let anatomy = rome();
        let table = tables();
        let joints = [
            limb(JointId::ElbowR),
            limb(JointId::KneeR),
            limb(JointId::WristL),
        ];
        let run = || {
            let mut rng = Rng::new(0xC07);
            let mut cuts: Vec<Cut> = Vec::new();
            let mut swing_id: u32 = 0;
            while swing_id < 10_000 {
                let energy = u32::try_from(rng.next_u64() % u64::from(MAX_ENERGY_Q16 + 1))
                    .expect("energy fits u32");
                let sharp = u32::try_from(rng.next_u64() % u64::from(MAX_SHARPNESS_Q16 + 1))
                    .expect("sharpness fits u32");
                let probe = BladeSwing {
                    energy_q16: energy,
                    sharpness_q16: sharp,
                    wear_q16: 0,
                    swing: swing_id,
                    ..swing(0, 0, 0, 0)
                };
                let mut out = [Cut {
                    swing: 0,
                    attacker: 0,
                    victim: 0,
                    joint: 0,
                    flags: 0,
                    energy_q16: 0,
                    protection_q16: 0,
                }; MAX_CUTS_PER_SWING];
                let count = resolve_swing(&probe, &joints, &anatomy, 0, &table, &mut out);
                cuts.extend_from_slice(&out[..count]);
                swing_id += 1;
            }
            digest_cuts(&cuts)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);
        assert_ne!(first, 0);
    }

    #[test]
    fn shuffled_targets_resolve_identically_in_joint_order() {
        let probe = swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, 77);
        let base = [
            limb(JointId::ElbowR),
            limb(JointId::KneeR),
            limb(JointId::WristL),
        ];
        let (expected, _) = resolve_all(&probe, &base, 0);
        for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0], [2, 1, 0]] {
            let shuffled = [base[order[0]], base[order[1]], base[order[2]]];
            let (actual, _) = resolve_all(&probe, &shuffled, 0);
            assert_eq!(actual, expected, "target order {order:?} diverged");
        }
        // Joint order is canonical: emitted wire values ascend.
        let mut last = 0u8;
        for hit in &expected {
            assert!(hit.joint >= last, "cuts out of joint order");
            last = hit.joint;
        }
    }

    #[test]
    fn self_pelvis_reach_and_travel_gates_name_themselves() {
        let table = tables();
        let anatomy = rome();
        let owner = BladeSwing {
            victim: 0,
            ..swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, 1)
        };
        assert_eq!(
            cut_one(&owner, &limb(JointId::ElbowR), &anatomy, 0, &table),
            CutOutcome::Miss(CutMiss::SelfTarget)
        );
        let pelvis = JointTarget {
            joint: JointId::Pelvis,
            ..limb(JointId::ElbowR)
        };
        assert_eq!(
            cut_one(
                &swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, 1),
                &pelvis,
                &anatomy,
                0,
                &table
            ),
            CutOutcome::Miss(CutMiss::NotSeverable)
        );
        // A metre to the side of the capsule misses despite hard steel.
        let wide = BladeSwing {
            p0_mm: [-500, 0, 1000],
            p1_mm: [500, 0, 1000],
            ..swing(MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, 0, 2)
        };
        assert_eq!(
            cut_one(&wide, &limb(JointId::ElbowR), &anatomy, 0, &table),
            CutOutcome::Miss(CutMiss::OutOfReach)
        );
        // Non-physical single-swing travel short-circuits to a reach miss.
        let far = BladeSwing {
            p0_mm: [-3500, 0, 0],
            p1_mm: [3500, 0, 0],
            ..swing(MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, 0, 3)
        };
        assert_eq!(
            cut_one(&far, &limb(JointId::ElbowR), &anatomy, 0, &table),
            CutOutcome::Miss(CutMiss::OutOfReach)
        );
        assert!(validate_swing(&far, &table).is_err());
    }

    #[test]
    fn validation_rejects_bad_versions_rows_and_slots() {
        let table = tables();
        assert_eq!(
            CutTableSet::new(0, CUT_V1).validate(),
            Err(CutError::BadVersion(0))
        );
        assert_eq!(
            CutTableSet::new(9, CUT_V1).validate(),
            Err(CutError::UnknownVersion(9))
        );
        let mut slack = CUT_V1;
        slack.contact_slack_mm = 17;
        assert!(CutTableSet::new(1, slack).validate().is_err());
        let mut span = CUT_V1;
        span.sever_span_q16 = 65_535;
        assert!(CutTableSet::new(1, span).validate().is_err());
        let owner = BladeSwing {
            victim: 0,
            ..swing(1, 1, 0, 0)
        };
        assert_eq!(validate_swing(&owner, &table), Err(CutError::SelfTarget(0)));
        let mut blunt = swing(1, 1, 0, 0);
        blunt.blade_half_mm = CUT_BLADE_HALF_MAX_MM + 1;
        assert!(validate_swing(&blunt, &table).is_err());
        let mut hot = swing(1, 1, 0, 0);
        hot.energy_q16 = MAX_ENERGY_Q16 + 1;
        assert!(validate_swing(&hot, &table).is_err());
        let pelvis = JointTarget {
            joint: JointId::Pelvis,
            ..limb(JointId::ElbowR)
        };
        assert_eq!(validate_target(&pelvis), Err(CutError::NotSeverable(0)));
        let mut fat = limb(JointId::ElbowR);
        fat.radius_mm = 0;
        assert!(validate_target(&fat).is_err());
        // Strict tick rejects repeats, overflow and owner swings.
        let mut out = [Cut {
            swing: 0,
            attacker: 0,
            victim: 0,
            joint: 0,
            flags: 0,
            energy_q16: 0,
            protection_q16: 0,
        }; MAX_CUTS_PER_SWING];
        let anatomy = rome();
        let dup = [limb(JointId::ElbowR), limb(JointId::ElbowR)];
        assert_eq!(
            try_resolve_swing(&swing(1, 1, 0, 0), &dup, &anatomy, 0, &table, &mut out),
            Err(CutError::DuplicateJoint(JointId::ElbowR.as_u8()))
        );
        let many = [limb(JointId::ElbowR); MAX_TARGETS_PER_SWING + 1];
        assert!(
            try_resolve_swing(&swing(1, 1, 0, 0), &many, &anatomy, 0, &table, &mut out).is_err()
        );
        assert!(try_cut_one(&owner, &limb(JointId::ElbowR), &anatomy, 0, &table).is_err());
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn sever_costs_under_point_one_five_ms() {
        let table = tables();
        let anatomy = rome();
        let probe = swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, 9);
        let target = limb(JointId::ElbowR);
        let start = std::time::Instant::now();
        let rounds = 1_000u32;
        let mut wins = 0u32;
        for _ in 0..rounds {
            if matches!(
                cut_one(&probe, &target, &anatomy, 0, &table),
                CutOutcome::Sever(_)
            ) {
                wins += 1;
            }
        }
        let elapsed = start.elapsed();
        let per_cut_nanos = elapsed.as_nanos() / u128::from(rounds);
        assert!(
            per_cut_nanos < 150_000,
            "sever costs {per_cut_nanos} ns, past the 0.15 ms bar"
        );
        let _ = wins;
    }

    #[test]
    fn resolve_allocates_nothing() {
        let table = tables();
        let anatomy = rome();
        let probe = swing(ENERGY_HARD_Q16, SHARP_FRESH_Q16, 0, 5);
        let targets = [limb(JointId::ElbowR), limb(JointId::KneeR)];
        let (_, heap) = engine_core::alloc_probe::measure(|| {
            let mut out = [Cut {
                swing: 0,
                attacker: 0,
                victim: 0,
                joint: 0,
                flags: 0,
                energy_q16: 0,
                protection_q16: 0,
            }; MAX_CUTS_PER_SWING];
            resolve_swing(&probe, &targets, &anatomy, 0, &table, &mut out)
        });
        assert_eq!(heap.calls, 0, "resolve allocated {} times", heap.calls);
    }

    #[test]
    fn plate_blocks_the_full_force_edge_and_names_its_waist() {
        // The flat block is the strongest edge the table lands (64.0 Q16 product).
        assert_eq!(
            PLATE_BLOCK_Q16,
            u64::from(MAX_ENERGY_Q16) * u64::from(MAX_SHARPNESS_Q16) / CUT_Q16
        );
        let anatomy = rome();
        let full = swing(MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, 0, 3);
        let targets = [limb(JointId::Waist), limb(JointId::HipL)];
        // Bare, nothing is struck by plate; plated, the waist it reaches is named.
        assert_eq!(plate_struck(&full, &targets, &anatomy, 0, &tables()), None);
        assert_eq!(
            plate_struck(&full, &targets, &anatomy, ARMOR_LORICA, &tables()),
            Some(JointId::Waist)
        );
        // A galea guards the neck, not plate: no deflect.
        assert_eq!(
            plate_struck(&full, &targets, &anatomy, ARMOR_GALEA, &tables()),
            None
        );
        // An edge past the block cuts through plate: no deflect.
        let past = swing(MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, 0, 4);
        let past = BladeSwing {
            sharpness_q16: MAX_SHARPNESS_Q16 + 1,
            ..past
        };
        assert_eq!(
            plate_struck(&past, &targets, &anatomy, ARMOR_LORICA, &tables()),
            None
        );
    }
}
