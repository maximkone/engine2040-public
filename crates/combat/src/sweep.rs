//! sweep: continuous segment-vs-capsule weapon solve with shield cones.
//! Owner: engine-combat, consumer: arena match step (pending orchestrator mod).
//! Units: integer millimetres (1/1000 m) for lengths, ticks at 60 Hz, IDs are slots.
//! Determinism: integer math only (Q13 segment params, Q16 shield cosine,
//!   integer square root), fighter-ID order; same tables plus same poses
//!   give the same events.
//! Errors: SweepError for bad versions/rows/slots; solvers never panic on user data.
//! Budget: O(sweeps x fighters) over <=16 participants and <=8 sweeps, no heap per tick.
//! Shapes are sweep-local; fight.rs read for context only, nothing mirrored.
//! Cap: match slots come from `engine_input::MAX_PARTICIPANTS` (the one
//! canonical cap); this module declares no private fighter cap. Crate edge
//! `engine-combat -> engine-input` is in `tools/workspace-check.mjs`.

use crate::fight::SLOT_COUNT;
use engine_input::MAX_PARTICIPANTS;
/// Version tag of the shipped sweep rows.
pub const SWEEP_VERSION_V1: u32 = 1;
/// Max weapon sweeps resolved in one tick.
pub const MAX_SWEEPS_PER_TICK: usize = 8;
/// Caller buffer size for one tick's sweep events (8 sweeps x 3 victims).
pub const MAX_SWEEP_EVENTS_PER_TICK: usize = 24;
/// Fixed-point subdivisions for segment parameters (Q13).
pub const SWEEP_Q: i64 = 8192;
/// Strict bound for every position component, in mm (+/-200 m arena scale).
pub const POS_BOUND_MM: i32 = 200_000;
/// Strict bound for each facing-vector component (scale-free, nonzero required).
pub const FACING_BOUND: i32 = 1_000_000;
/// Max hurt-capsule half-height in V1 validation, in mm.
pub const HURT_HALF_MAX_MM: u32 = 2000;
/// Max hurt-capsule radius in V1 validation, in mm.
pub const HURT_RADIUS_MAX_MM: u32 = 2000;
/// Max shield half-angle in V1 validation, in degrees.
pub const SHIELD_HALF_MAX_DEG: u8 = 90;
/// Max blade half-thickness in V1 validation, in mm (thin rapier ~5).
pub const BLADE_HALF_MAX_MM: u32 = 500;
/// Conservative contact slack in V1, in mm (covers Q13 rounding with margin).
pub const CONTACT_SLACK_MM_V1: u32 = 4;
/// Longest single-tick blade travel accepted by V1, in mm.
pub const MAX_SWEEP_LEN_MM_V1: u32 = 6_000;
/// Wide-envelope position bound; beyond this pairs cannot touch (mm).
const ENVELOPE_POS_MM: i32 = 2_000_000;
/// Wide-envelope facing bound; beyond this inputs are not physical.
const ENVELOPE_FACING: i32 = 1_000_000;
/// Wide-envelope radius bound; beyond this inputs are not physical (mm).
const ENVELOPE_RADIUS_MM: u32 = 10_000;
/// Cosine scale for the shield cone check (Q16).
const COS_Q16: u64 = 65536;
/// Fixed-point pi in Q16 (3.14159265 * 65536, truncated).
const PI_Q16: i64 = 205887;

/// Tunable sweep rows: every number the solvers read, in one place.
///
/// Units: millimetres for lengths. `contact_slack_mm` conservatively expands
/// every contact test so Q13 rounding can never hide a true hit (no
/// tunneling); `max_sweep_len_mm` caps single-tick blade travel, and longer
/// sweeps short-circuit to a reach miss (strict callers get [`SweepError`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SweepTables {
    /// Conservative contact expansion in mm, `0..=16`.
    pub contact_slack_mm: u32,
    /// Longest accepted single-tick blade travel in mm, `100..=12000`.
    pub max_sweep_len_mm: u32,
}

impl SweepTables {
    /// The shipped sweep rows.
    pub const fn v1() -> Self {
        Self {
            contact_slack_mm: CONTACT_SLACK_MM_V1,
            max_sweep_len_mm: MAX_SWEEP_LEN_MM_V1,
        }
    }
}

/// The shipped sweep rows as one static row.
pub const SWEEP_V1: SweepTables = SweepTables::v1();

/// Versioned sweep rows: nonzero version plus validated rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SweepTableSet {
    /// Table version; nonzero, 1 for the shipped rows.
    pub version: u32,
    /// The tunable rows resolved under this version.
    pub tables: SweepTables,
}

impl SweepTableSet {
    /// The shipped rows as version 1.
    pub const fn v1() -> Self {
        Self {
            version: SWEEP_VERSION_V1,
            tables: SWEEP_V1,
        }
    }

    /// Explicit rows with a nonzero version; call [`validate_sweep_tables`] first.
    pub const fn new(version: u32, tables: SweepTables) -> Self {
        Self { version, tables }
    }

    /// Range-check this set; see [`validate_sweep_tables`].
    pub fn validate(&self) -> Result<(), SweepError> {
        validate_sweep_tables(self)
    }
}

/// Bounded sweep failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SweepError {
    /// Version 0 is reserved for legacy untagged replays, never valid anew.
    BadVersion(u32),
    /// One tunable row or pose field is outside its sane range; culprit
    /// carried. First failure wins, deterministically.
    BadValue {
        /// Table or pose field name, e.g. `"blade_half_mm"`.
        field: &'static str,
        /// Culprit value (magnitudes for signed fields).
        value: u32,
    },
    /// Replay or swap references a table version with no known rows.
    UnknownVersion(u32),
    /// Participant count outside `1..=MAX_PARTICIPANTS`.
    BadParticipantCount(u8),
    /// Participant slot outside the active `0..count` range.
    BadSlot(u8),
    /// More sweeps than [`MAX_SWEEPS_PER_TICK`] in one tick, length carried.
    TooManySweeps(usize),
    /// More fighters than `MAX_PARTICIPANTS` in one tick, length carried.
    TooManyFighters(usize),
    /// Same fighter ID twice in one tick, culprit carried.
    DuplicateFighter(u8),
}

/// Range-check a versioned sweep set. First failure wins, deterministically.
pub fn validate_sweep_tables(set: &SweepTableSet) -> Result<(), SweepError> {
    if set.version == 0 {
        return Err(SweepError::BadVersion(0));
    }
    range("contact_slack_mm", set.tables.contact_slack_mm, 0, 16)?;
    range("max_sweep_len_mm", set.tables.max_sweep_len_mm, 100, 12_000)?;
    Ok(())
}

fn range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), SweepError> {
    if value < min || value > max {
        return Err(SweepError::BadValue { field, value });
    }
    Ok(())
}

/// One fighter's sweep pose: position, facing, hurt capsule and shield cone.
///
/// The hurt capsule is the vertical spine through `pos_mm` from
/// `y - hurt_half_height_mm` to `y + hurt_half_height_mm` plus
/// `hurt_radius_mm`. Facing is an XZ vector `(facing_x, facing_z)` of any
/// nonzero scale; only its direction is read. The shield covers the cone
/// around facing with half-angle `shield_half_deg` while `shield_up`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SweepParticipant {
    /// Participant slot.
    pub id: u8,
    /// Position in mm; +Y up.
    pub pos_mm: [i32; 3],
    /// XZ facing vector, nonzero, scale-free.
    pub facing_x: i32,
    /// XZ facing vector, nonzero, scale-free.
    pub facing_z: i32,
    /// Hurt-capsule half-height in mm, `0..=2000`.
    pub hurt_half_height_mm: u32,
    /// Hurt-capsule radius in mm, `1..=2000`.
    pub hurt_radius_mm: u32,
    /// Shield cone half-angle in degrees, `0..=90`.
    pub shield_half_deg: u8,
    /// Whether the shield cone is raised.
    pub shield_up: bool,
}

/// Back-compat alias for the legacy name: identical layout, zero-cost.
/// External call-sites (e.g. `engine-arena`) migrate to [`SweepParticipant`];
/// kept so the rename never breaks a downstream crate mid-sprint.
pub type SweepFighter = SweepParticipant;

/// One weapon's continuous sweep for the tick: the blade tip path plus steel.
///
/// `p0_mm` is where the striking edge starts the tick, `p1_mm` where it ends
/// it; the segment between them is tested against every victim capsule, so a
/// thin fast blade cannot tunnel past a fighter between ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeaponSweep {
    /// Attacking slot; routing metadata (geometry comes from the poses).
    pub attacker: u8,
    /// Blade start of tick in mm.
    pub p0_mm: [i32; 3],
    /// Blade end of tick in mm.
    pub p1_mm: [i32; 3],
    /// Blade half-thickness in mm, `0..=500`.
    pub blade_half_mm: u32,
}

/// Why a sweep missed; every miss names its gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SweepMiss {
    /// Attacker and victim are the same slot; weapons never hit their owner.
    SelfTarget,
    /// Victim is behind (or exactly abeam of) the attacker's facing, or the
    /// attacker has no facing to confirm in-front with.
    BehindAttacker,
    /// Blade path stays outside blade plus capsule plus slack, the pair is
    /// beyond the physical envelope, or the travel is non-physical.
    OutOfReach,
}

/// Pure outcome of one weapon against one victim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SweepOutcome {
    /// Blade path touches the capsule and the shield cone does not cover it.
    Hit {
        /// Attacking slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
    },
    /// Blade path touches the capsule but the raised shield cone covers it.
    Blocked {
        /// Attacking slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
    },
    /// No contact; the gate that refused is named.
    Miss(SweepMiss),
}

/// One tick's sweep contacts, in (attacker, victim) ID order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SweepEvent {
    /// Unblocked blade contact.
    Hit {
        /// Attacking slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
    },
    /// Blade contact covered by the victim's shield cone.
    Blocked {
        /// Attacking slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
    },
}

/// Range-check one pose against the active fighter count.
pub fn validate_fighter(f: &SweepParticipant, count: u8) -> Result<(), SweepError> {
    if f.id >= count {
        return Err(SweepError::BadSlot(f.id));
    }
    let mut i = 0;
    while i < 3 {
        let c = f.pos_mm[i];
        if !(-POS_BOUND_MM..=POS_BOUND_MM).contains(&c) {
            return Err(SweepError::BadValue {
                field: "pos_mm",
                value: c.unsigned_abs(),
            });
        }
        i += 1;
    }
    if f.facing_x < -FACING_BOUND || f.facing_x > FACING_BOUND {
        return Err(SweepError::BadValue {
            field: "facing_x",
            value: f.facing_x.unsigned_abs(),
        });
    }
    if f.facing_z < -FACING_BOUND || f.facing_z > FACING_BOUND {
        return Err(SweepError::BadValue {
            field: "facing_z",
            value: f.facing_z.unsigned_abs(),
        });
    }
    if f.facing_x == 0 && f.facing_z == 0 {
        return Err(SweepError::BadValue {
            field: "facing",
            value: 0,
        });
    }
    if f.hurt_half_height_mm > HURT_HALF_MAX_MM {
        return Err(SweepError::BadValue {
            field: "hurt_half_height_mm",
            value: f.hurt_half_height_mm,
        });
    }
    if f.hurt_radius_mm == 0 || f.hurt_radius_mm > HURT_RADIUS_MAX_MM {
        return Err(SweepError::BadValue {
            field: "hurt_radius_mm",
            value: f.hurt_radius_mm,
        });
    }
    if f.shield_half_deg > SHIELD_HALF_MAX_DEG {
        return Err(SweepError::BadValue {
            field: "shield_half_deg",
            value: u32::from(f.shield_half_deg),
        });
    }
    Ok(())
}

/// Range-check one sweep against the active count and the table travel bound.
pub fn validate_sweep(s: &WeaponSweep, count: u8, tables: &SweepTables) -> Result<(), SweepError> {
    if s.attacker >= count {
        return Err(SweepError::BadSlot(s.attacker));
    }
    let mut i = 0;
    while i < 3 {
        let a = s.p0_mm[i];
        if !(-POS_BOUND_MM..=POS_BOUND_MM).contains(&a) {
            return Err(SweepError::BadValue {
                field: "sweep_p0_mm",
                value: a.unsigned_abs(),
            });
        }
        let b = s.p1_mm[i];
        if !(-POS_BOUND_MM..=POS_BOUND_MM).contains(&b) {
            return Err(SweepError::BadValue {
                field: "sweep_p1_mm",
                value: b.unsigned_abs(),
            });
        }
        i += 1;
    }
    if s.blade_half_mm > BLADE_HALF_MAX_MM {
        return Err(SweepError::BadValue {
            field: "blade_half_mm",
            value: s.blade_half_mm,
        });
    }
    let dx = i64::from(s.p1_mm[0]) - i64::from(s.p0_mm[0]);
    let dy = i64::from(s.p1_mm[1]) - i64::from(s.p0_mm[1]);
    let dz = i64::from(s.p1_mm[2]) - i64::from(s.p0_mm[2]);
    let max = i64::from(tables.max_sweep_len_mm);
    if dx * dx + dy * dy + dz * dz > max * max {
        let len_sq = u64::try_from(dx * dx + dy * dy + dz * dz).unwrap_or(u64::MAX);
        return Err(SweepError::BadValue {
            field: "sweep_len_mm",
            value: u32::try_from(len_sq.isqrt()).unwrap_or(u32::MAX),
        });
    }
    Ok(())
}

/// Continuous outcome of one weapon against one victim.
///
/// Total: never panics. Gates run in fixed order (self, envelope, attacker
/// facing, sweep travel, contact, shield), so every miss names exactly one
/// gate. `sweep.attacker` is routing metadata for [`resolve_tick`] and is
/// not re-checked here; [`try_sweep_one`] enforces it. Inputs beyond the
/// wide physical envelope short-circuit to [`SweepMiss::OutOfReach`]; strict
/// callers use the validators for typed errors instead.
pub fn sweep_one(
    attacker: &SweepParticipant,
    defender: &SweepParticipant,
    sweep: &WeaponSweep,
    tables: &SweepTables,
) -> SweepOutcome {
    let a = attacker.id;
    let v = defender.id;
    if a == v {
        return SweepOutcome::Miss(SweepMiss::SelfTarget);
    }
    if !in_envelope(attacker, defender, sweep) {
        return SweepOutcome::Miss(SweepMiss::OutOfReach);
    }
    if attacker.facing_x == 0 && attacker.facing_z == 0 {
        return SweepOutcome::Miss(SweepMiss::BehindAttacker);
    }
    // Behind-attacker gate on the XZ plane; shared XZ means point-blank.
    let dx = i64::from(defender.pos_mm[0]) - i64::from(attacker.pos_mm[0]);
    let dz = i64::from(defender.pos_mm[2]) - i64::from(attacker.pos_mm[2]);
    if (dx != 0 || dz != 0)
        && i64::from(attacker.facing_x) * dx + i64::from(attacker.facing_z) * dz <= 0
    {
        return SweepOutcome::Miss(SweepMiss::BehindAttacker);
    }
    // Non-physical single-tick travel short-circuits to a reach miss.
    let ux = i64::from(sweep.p1_mm[0]) - i64::from(sweep.p0_mm[0]);
    let uy = i64::from(sweep.p1_mm[1]) - i64::from(sweep.p0_mm[1]);
    let uz = i64::from(sweep.p1_mm[2]) - i64::from(sweep.p0_mm[2]);
    let max = i64::from(tables.max_sweep_len_mm);
    if ux * ux + uy * uy + uz * uz > max * max {
        return SweepOutcome::Miss(SweepMiss::OutOfReach);
    }
    // Hurt spine: vertical capsule through the defender position.
    let half = i32::try_from(defender.hurt_half_height_mm).unwrap_or(i32::MAX);
    let q0 = [
        defender.pos_mm[0],
        defender.pos_mm[1] - half,
        defender.pos_mm[2],
    ];
    let q1 = [
        defender.pos_mm[0],
        defender.pos_mm[1] + half,
        defender.pos_mm[2],
    ];
    let radius = u64::from(sweep.blade_half_mm)
        + u64::from(defender.hurt_radius_mm)
        + u64::from(tables.contact_slack_mm);
    let radius_mm = if radius > u64::from(u32::MAX) {
        u32::MAX
    } else {
        u32::try_from(radius).unwrap_or(u32::MAX)
    };
    if !seg_seg_touches(sweep.p0_mm, sweep.p1_mm, q0, q1, radius_mm) {
        return SweepOutcome::Miss(SweepMiss::OutOfReach);
    }
    if defender.shield_up && shield_blocks(defender, attacker.pos_mm) {
        return SweepOutcome::Blocked {
            attacker: a,
            victim: v,
        };
    }
    SweepOutcome::Hit {
        attacker: a,
        victim: v,
    }
}

/// Strict one-pair solve: validates routing, poses and sweep, then solves.
pub fn try_sweep_one(
    attacker: &SweepParticipant,
    defender: &SweepParticipant,
    sweep: &WeaponSweep,
    tables: &SweepTables,
    count: u8,
) -> Result<SweepOutcome, SweepError> {
    if count == 0 || count > MAX_PARTICIPANTS {
        return Err(SweepError::BadParticipantCount(count));
    }
    if attacker.id >= count {
        return Err(SweepError::BadSlot(attacker.id));
    }
    if defender.id >= count {
        return Err(SweepError::BadSlot(defender.id));
    }
    if sweep.attacker != attacker.id {
        return Err(SweepError::BadSlot(sweep.attacker));
    }
    validate_fighter(attacker, count)?;
    validate_fighter(defender, count)?;
    validate_sweep(sweep, count, tables)?;
    Ok(sweep_one(attacker, defender, sweep, tables))
}

/// Resolve one tick of sweeps into the caller buffer, in ID order.
///
/// Slots are generic: the fighter slice length is the slot count
/// (`1..=MAX_PARTICIPANTS`, the canonical cap) and any validated [`SweepTables`]
/// set resolves, so fixture rules need no engine edits.
/// Deterministic order: sweeps sort by (attacker, blade path, thickness) and
/// victims run in ascending fighter ID, so shuffled inputs give identical
/// events. Self pairs emit nothing. Oversized or duplicated inputs resolve
/// against the smallest IDs with first-in-slice winning ties; writes at most
/// `out.len()` events and returns the count. No heap, no floats.
pub fn resolve_tick(
    fighters: &[SweepParticipant],
    sweeps: &[WeaponSweep],
    tables: &SweepTables,
    out: &mut [SweepEvent; MAX_SWEEP_EVENTS_PER_TICK],
) -> usize {
    let (fkeep, fcount) = keep_fighters(fighters);
    let (skeep, scount) = keep_sweeps(sweeps);
    let mut n: usize = 0;
    let mut k = 0;
    while k < scount {
        let sweep = &sweeps[skeep[k]];
        let mut ai: usize = usize::MAX;
        let mut f = 0;
        while f < fcount {
            if fighters[fkeep[f].1].id == sweep.attacker {
                ai = fkeep[f].1;
                break;
            }
            f += 1;
        }
        if ai != usize::MAX {
            let mut w = 0;
            while w < fcount {
                let vi = fkeep[w].1;
                if fighters[vi].id != sweep.attacker {
                    match sweep_one(&fighters[ai], &fighters[vi], sweep, tables) {
                        SweepOutcome::Hit { attacker, victim } => {
                            if n < out.len() {
                                out[n] = SweepEvent::Hit { attacker, victim };
                                n += 1;
                            }
                        }
                        SweepOutcome::Blocked { attacker, victim } => {
                            if n < out.len() {
                                out[n] = SweepEvent::Blocked { attacker, victim };
                                n += 1;
                            }
                        }
                        SweepOutcome::Miss(_) => {}
                    }
                }
                w += 1;
            }
        }
        k += 1;
    }
    n
}

// ---- LF11 per-swing once-per-target gate ------------------------------------
// Ember swept-hit rule, integer port: the blade still sweeps every tick (no
// tunneling), but one swing contacts each victim once. Caller-owned beside
// `resolve_tick`, which stays frozen so the golden hash never moves.

/// Caller-owned per-swing once-per-target gate: one swing of one attacker
/// plus its contacted victims. Slots are fighter IDs; the table lives on
/// the caller stack, zero heap bytes per update.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SwingGate {
    /// Caller swing sequence number; a new id re-arms the hit-set.
    pub swing: u32,
    /// Attacking slot this gate tracks; foreign attackers pass through.
    pub attacker: u8,
    /// Contacted victim slots in first-contact order.
    seen: [u8; SLOT_COUNT],
    /// Live entries in `seen`, always `<= SLOT_COUNT`.
    seen_n: u8,
}

impl SwingGate {
    /// Fresh gate: nothing contacted yet under (`swing`, `attacker`).
    #[must_use]
    pub const fn fresh(swing: u32, attacker: u8) -> Self {
        Self {
            swing,
            attacker,
            seen: [0; SLOT_COUNT],
            seen_n: 0,
        }
    }

    /// Claim `victim` for this swing: true on first contact (now marked),
    /// false when already contacted or past the table. Never traps.
    pub fn claim(&mut self, victim: u8) -> bool {
        let mut i = 0;
        while i < self.seen_n {
            if self.seen[usize::from(i)] == victim {
                return false;
            }
            i += 1;
        }
        let n = usize::from(self.seen_n);
        if n < self.seen.len() {
            self.seen[n] = victim;
            self.seen_n += 1;
            return true;
        }
        false
    }

    /// Re-arm for a new swing: clears every contact.
    pub fn reset(&mut self, swing: u32, attacker: u8) {
        self.swing = swing;
        self.attacker = attacker;
        self.seen = [0; SLOT_COUNT];
        self.seen_n = 0;
    }
}

/// Resolve one tick through a [`SwingGate`]: each victim of (`swing`,
/// `attacker`) emits once, later ticks skip; a new id re-arms, foreign
/// attackers pass through. Order and replay match [`resolve_tick`].
pub fn resolve_swing_tick(
    fighters: &[SweepParticipant],
    sweeps: &[WeaponSweep],
    tables: &SweepTables,
    swing: u32,
    attacker: u8,
    gate: &mut SwingGate,
    out: &mut [SweepEvent; MAX_SWEEP_EVENTS_PER_TICK],
) -> usize {
    if gate.swing != swing || gate.attacker != attacker {
        gate.reset(swing, attacker);
    }
    let mut tmp = [SweepEvent::Hit {
        attacker: 0,
        victim: 0,
    }; MAX_SWEEP_EVENTS_PER_TICK];
    let n = resolve_tick(fighters, sweeps, tables, &mut tmp);
    let mut kept: usize = 0;
    let mut i = 0;
    while i < n {
        let (hit_attacker, victim) = match tmp[i] {
            SweepEvent::Hit { attacker, victim } => (attacker, victim),
            SweepEvent::Blocked { attacker, victim } => (attacker, victim),
        };
        if hit_attacker == gate.attacker {
            if gate.claim(victim) && kept < out.len() {
                out[kept] = tmp[i];
                kept += 1;
            }
        } else if kept < out.len() {
            out[kept] = tmp[i];
            kept += 1;
        }
        i += 1;
    }
    kept
}

/// Fighter reservoir: the (id, slice index) pairs of up to [`SLOT_COUNT`]
/// fighters, sorted by id. Smallest ids win and the first-in-slice wins
/// duplicates; returns the table and how many entries are live.
fn keep_fighters(fighters: &[SweepParticipant]) -> ([(u8, usize); SLOT_COUNT], usize) {
    let mut fkeep: [(u8, usize); SLOT_COUNT] = [(0, 0); SLOT_COUNT];
    let mut fcount: usize = 0;
    let mut fi = 0;
    while fi < fighters.len() {
        let id = fighters[fi].id;
        let mut dup = false;
        let mut k = 0;
        while k < fcount {
            if fkeep[k].0 == id {
                dup = true;
                break;
            }
            k += 1;
        }
        if !dup {
            if fcount < fkeep.len() {
                let mut pos = fcount;
                while pos > 0 && fkeep[pos - 1].0 > id {
                    fkeep[pos] = fkeep[pos - 1];
                    pos -= 1;
                }
                fkeep[pos] = (id, fi);
                fcount += 1;
            } else if id < fkeep[fkeep.len() - 1].0 {
                let mut pos = fkeep.len() - 1;
                while pos > 0 && fkeep[pos - 1].0 > id {
                    fkeep[pos] = fkeep[pos - 1];
                    pos -= 1;
                }
                fkeep[pos] = (id, fi);
            }
        }
        fi += 1;
    }
    (fkeep, fcount)
}

/// Sweep reservoir: slice indices of up to [`MAX_SWEEPS_PER_TICK`] sweeps in
/// content order; the first-in-slice wins full ties. Returns the table and
/// how many entries are live.
fn keep_sweeps(sweeps: &[WeaponSweep]) -> ([usize; MAX_SWEEPS_PER_TICK], usize) {
    let mut skeep: [usize; MAX_SWEEPS_PER_TICK] = [0; MAX_SWEEPS_PER_TICK];
    let mut scount: usize = 0;
    let mut si = 0;
    while si < sweeps.len() {
        if scount < skeep.len() {
            let mut pos = scount;
            while pos > 0 && sweep_less(&sweeps[si], &sweeps[skeep[pos - 1]]) {
                skeep[pos] = skeep[pos - 1];
                pos -= 1;
            }
            skeep[pos] = si;
            scount += 1;
        } else if sweep_less(&sweeps[si], &sweeps[skeep[skeep.len() - 1]]) {
            let mut pos = skeep.len() - 1;
            while pos > 0 && sweep_less(&sweeps[si], &sweeps[skeep[pos - 1]]) {
                skeep[pos] = skeep[pos - 1];
                pos -= 1;
            }
            skeep[pos] = si;
        }
        si += 1;
    }
    (skeep, scount)
}

/// Checked participant count for the strict tick: the `u8` slot range is
/// narrower than `usize`, so an over-long slice refuses as
/// [`SweepError::TooManyFighters`] instead of wrapping the low bits with
/// `as`.
fn fighter_count_u8(len: usize) -> Result<u8, SweepError> {
    u8::try_from(len).map_err(|_| SweepError::TooManyFighters(len))
}

/// Strict tick resolve: validates counts, duplicates, poses and sweeps first.
pub fn try_resolve_tick(
    fighters: &[SweepParticipant],
    sweeps: &[WeaponSweep],
    tables: &SweepTables,
    out: &mut [SweepEvent; MAX_SWEEP_EVENTS_PER_TICK],
) -> Result<usize, SweepError> {
    if fighters.is_empty() {
        return Err(SweepError::BadParticipantCount(0));
    }
    if fighters.len() > SLOT_COUNT {
        return Err(SweepError::TooManyFighters(fighters.len()));
    }
    if sweeps.len() > MAX_SWEEPS_PER_TICK {
        return Err(SweepError::TooManySweeps(sweeps.len()));
    }
    let count = fighter_count_u8(fighters.len())?;
    let mut i = 0;
    while i < fighters.len() {
        let mut j = i + 1;
        while j < fighters.len() {
            if fighters[i].id == fighters[j].id {
                return Err(SweepError::DuplicateFighter(fighters[i].id));
            }
            j += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while i < fighters.len() {
        validate_fighter(&fighters[i], count)?;
        i += 1;
    }
    let mut s = 0;
    while s < sweeps.len() {
        validate_sweep(&sweeps[s], count, tables)?;
        s += 1;
    }
    Ok(resolve_tick(fighters, sweeps, tables, out))
}

/// Content order for sweeps: attacker, then blade path, then thickness.
fn sweep_less(a: &WeaponSweep, b: &WeaponSweep) -> bool {
    if a.attacker != b.attacker {
        return a.attacker < b.attacker;
    }
    let mut i = 0;
    while i < 3 {
        if a.p0_mm[i] != b.p0_mm[i] {
            return a.p0_mm[i] < b.p0_mm[i];
        }
        i += 1;
    }
    let mut j = 0;
    while j < 3 {
        if a.p1_mm[j] != b.p1_mm[j] {
            return a.p1_mm[j] < b.p1_mm[j];
        }
        j += 1;
    }
    a.blade_half_mm < b.blade_half_mm
}

/// Wide-envelope guard: all solver arithmetic provably fits past this gate.
fn in_envelope(a: &SweepParticipant, d: &SweepParticipant, s: &WeaponSweep) -> bool {
    let mut i = 0;
    while i < 3 {
        if a.pos_mm[i] < -ENVELOPE_POS_MM || a.pos_mm[i] > ENVELOPE_POS_MM {
            return false;
        }
        if d.pos_mm[i] < -ENVELOPE_POS_MM || d.pos_mm[i] > ENVELOPE_POS_MM {
            return false;
        }
        if s.p0_mm[i] < -ENVELOPE_POS_MM || s.p0_mm[i] > ENVELOPE_POS_MM {
            return false;
        }
        if s.p1_mm[i] < -ENVELOPE_POS_MM || s.p1_mm[i] > ENVELOPE_POS_MM {
            return false;
        }
        i += 1;
    }
    if a.facing_x < -ENVELOPE_FACING || a.facing_x > ENVELOPE_FACING {
        return false;
    }
    if a.facing_z < -ENVELOPE_FACING || a.facing_z > ENVELOPE_FACING {
        return false;
    }
    if d.facing_x < -ENVELOPE_FACING || d.facing_x > ENVELOPE_FACING {
        return false;
    }
    if d.facing_z < -ENVELOPE_FACING || d.facing_z > ENVELOPE_FACING {
        return false;
    }
    if a.hurt_half_height_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    if a.hurt_radius_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    if d.hurt_half_height_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    if d.hurt_radius_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    if s.blade_half_mm > ENVELOPE_RADIUS_MM {
        return false;
    }
    true
}

/// True when the raised shield cone covers the direction to the attacker.
///
/// Integer only: the facing/dot comparison is exact, the cosine comes from
/// a fixed-point polynomial (no trig calls), and lengths use `u64::isqrt`.
/// Division truncation shrinks the cone by under one cosine count, which
/// favors the attacker; fixtures keep a wide margin from the cone edge.
fn shield_blocks(def: &SweepParticipant, atk_pos: [i32; 3]) -> bool {
    let dx = i64::from(atk_pos[0]) - i64::from(def.pos_mm[0]);
    let dz = i64::from(atk_pos[2]) - i64::from(def.pos_mm[2]);
    if dx == 0 && dz == 0 {
        return false;
    }
    let dot = i64::from(def.facing_x) * dx + i64::from(def.facing_z) * dz;
    if dot <= 0 {
        return false;
    }
    let fx2 = i64::from(def.facing_x) * i64::from(def.facing_x);
    let fz2 = i64::from(def.facing_z) * i64::from(def.facing_z);
    let len_f = u64::try_from(fx2 + fz2).unwrap_or(0).isqrt();
    if len_f == 0 {
        return false;
    }
    let len_d = u64::try_from(dx * dx + dz * dz).unwrap_or(0).isqrt();
    let cos = u64::from(cos_half_q16(def.shield_half_deg));
    u64::try_from(dot).unwrap_or(0) * COS_Q16 >= cos * len_f * len_d
}

/// Cosine of a shield half-angle in Q16 via a fixed-point polynomial.
///
/// Five Taylor terms around zero; exact at 0 degrees and within a few counts
/// of the true cosine through 90 degrees. Inputs past 90 clamp to 90.
/// Uses only `+ - * /`, never a trig call.
fn cos_half_q16(half_deg: u8) -> u32 {
    let deg = i64::from(if half_deg > 90 { 90 } else { half_deg });
    let x = deg * PI_Q16 / 180;
    let x2 = (x * x) >> 16;
    let x4 = (x2 * x2) >> 16;
    let x6 = (x4 * x2) >> 16;
    let x8 = (x6 * x2) >> 16;
    let cos = 65536 - x2 / 2 + x4 / 24 - x6 / 720 + x8 / 40320;
    if cos < 0 {
        0
    } else {
        u32::try_from(cos).unwrap_or(0)
    }
}

/// Component-wise `a - b` widened to `i64` so millimetre deltas never wrap;
/// shared by the sweep and cut solvers.
pub(crate) fn sub3(a: [i32; 3], b: [i32; 3]) -> [i64; 3] {
    [
        i64::from(a[0]) - i64::from(b[0]),
        i64::from(a[1]) - i64::from(b[1]),
        i64::from(a[2]) - i64::from(b[2]),
    ]
}

/// Dot product widened to `i128`; bounded deltas keep it exact.
pub(crate) fn dot3(a: [i64; 3], b: [i64; 3]) -> i128 {
    i128::from(a[0]) * i128::from(b[0])
        + i128::from(a[1]) * i128::from(b[1])
        + i128::from(a[2]) * i128::from(b[2])
}

/// Clamp `num / den` to `[0, SWEEP_Q]` with round-to-nearest; `den <= 0`
/// yields 0 so degenerate segments never divide by zero.
fn clamp_param_q(num: i128, den: i128) -> i64 {
    if den <= 0 {
        return 0;
    }
    if num <= 0 {
        return 0;
    }
    if num >= den {
        return SWEEP_Q;
    }
    let q = (num * i128::from(SWEEP_Q) + den / 2) / den;
    if q > i128::from(SWEEP_Q) {
        SWEEP_Q
    } else {
        i64::try_from(q).unwrap_or(i64::MAX)
    }
}

/// True when the Q-scaled distance at `(s_q, t_q)` fits inside `r_q`.
fn dist_le_q(
    p0: [i32; 3],
    u: [i64; 3],
    s_q: i64,
    q0: [i32; 3],
    v: [i64; 3],
    t_q: i64,
    r_q: i128,
) -> bool {
    let q = i128::from(SWEEP_Q);
    let dx = (i128::from(p0[0]) - i128::from(q0[0])) * q + i128::from(s_q) * i128::from(u[0])
        - i128::from(t_q) * i128::from(v[0]);
    let dy = (i128::from(p0[1]) - i128::from(q0[1])) * q + i128::from(s_q) * i128::from(u[1])
        - i128::from(t_q) * i128::from(v[1]);
    let dz = (i128::from(p0[2]) - i128::from(q0[2])) * q + i128::from(s_q) * i128::from(u[2])
        - i128::from(t_q) * i128::from(v[2]);
    dx * dx + dy * dy + dz * dz <= r_q * r_q
}

/// True when blade segment `p0->p1` touches capsule spine `q0->q1` plus
/// `radius_mm`. Exact region solve with Q13-rounded candidates: the true
/// minimum lives in one of the nine regions, so the rounded best is within
/// two millimetres of it and the table slack always covers the gap.
fn seg_seg_touches(p0: [i32; 3], p1: [i32; 3], q0: [i32; 3], q1: [i32; 3], radius_mm: u32) -> bool {
    let u = sub3(p1, p0);
    let v = sub3(q1, q0);
    let w0 = sub3(p0, q0);
    let a = dot3(u, u);
    let b = dot3(u, v);
    let c = dot3(v, v);
    let d = dot3(u, w0);
    let e = dot3(v, w0);
    let dd = a * c - b * b;
    let (si, ti) = if dd > 0 {
        (
            clamp_param_q(b * e - c * d, dd),
            clamp_param_q(a * e - b * d, dd),
        )
    } else {
        (0, clamp_param_q(e, c))
    };
    let cands: [[i64; 2]; 9] = [
        [si, ti],
        [0, clamp_param_q(e, c)],
        [SWEEP_Q, clamp_param_q(e + b, c)],
        [clamp_param_q(-d, a), 0],
        [clamp_param_q(b - d, a), SWEEP_Q],
        [0, 0],
        [0, SWEEP_Q],
        [SWEEP_Q, 0],
        [SWEEP_Q, SWEEP_Q],
    ];
    let r_q = i128::from(radius_mm) * i128::from(SWEEP_Q);
    let mut i = 0;
    while i < cands.len() {
        if dist_le_q(p0, u, cands[i][0], q0, v, cands[i][1], r_q) {
            return true;
        }
        i += 1;
    }
    false
}

// ---- GP23 active-steel gate ------------------------------------------------
// Sweep half of the phased move schema: steel is live exactly while the
// fighter's phased age sits in its active window. The row type stays owned
// by `exchange.rs` (reused here, never mirrored); this section adds only the
// pure age-to-steel gate the tick already evaluates before emitting a
// sweep. Integer math only, no heap, never panics.

use crate::exchange::{phase_at, phased_row, MovePhase, PhasedMoveRow, PhasedTableSet};

/// True while a phased move age carries live steel: exactly its active
/// window. Startup, endlag and done never emit.
pub const fn sweep_live_at(age: u32, row: &PhasedMoveRow) -> bool {
    matches!(phase_at(age, row), MovePhase::Active)
}

/// True while the named move's age carries live steel under its set;
/// unknown moves read as dead steel, never a panic.
pub const fn sweep_live_for(set: &PhasedTableSet, move_id: u8, age: u32) -> bool {
    match phased_row(set, move_id) {
        Some(row) => sweep_live_at(age, &row),
        None => false,
    }
}

#[cfg(test)]
mod phased_steel_tests {
    use super::*;
    use crate::exchange::{PHASED_CROSS, PHASED_JAB, PHASED_V1};

    #[test]
    fn steel_live_only_during_active() {
        // Every age of both shipped moves: steel agrees with the phase.
        for row in [PHASED_JAB, PHASED_CROSS] {
            let mut age = 0u32;
            while age < row.total() + 2 {
                let live = sweep_live_at(age, &row);
                let active = phase_at(age, &row) == MovePhase::Active;
                assert_eq!(live, active, "move {} age {age}", row.move_id);
                assert_eq!(live, sweep_live_for(&PHASED_V1, row.move_id, age));
                age += 1;
            }
        }
        // Jab edges: whiff at 3, live at 4..=5, committed at 6.
        assert!(!sweep_live_at(3, &PHASED_JAB));
        assert!(sweep_live_at(4, &PHASED_JAB));
        assert!(sweep_live_at(5, &PHASED_JAB));
        assert!(!sweep_live_at(6, &PHASED_JAB));
        // Unknown moves never emit steel.
        assert!(!sweep_live_for(&PHASED_V1, 7, 4));
    }

    #[test]
    fn phased_steel_tape_replays_twice() {
        // Two fighters sweep only on live ticks over a scripted 24-tick
        // duel: fighter 0 loops jab, fighter 1 rides cross once. Each
        // fighter's live-tick tape replays identically twice.
        let run = || {
            let mut age0 = 0u32;
            let mut age1 = 0u32;
            let mut live0 = [false; 24];
            let mut live1 = [false; 24];
            let mut i = 0usize;
            while i < live0.len() {
                live0[i] = sweep_live_for(&PHASED_V1, 0, age0);
                live1[i] = sweep_live_for(&PHASED_V1, 1, age1);
                age0 = age0.saturating_add(1);
                age1 = age1.saturating_add(1);
                if age0 >= PHASED_JAB.total() {
                    age0 = 0;
                }
                i += 1;
            }
            (live0, live1)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);
        // Steel really gates: jab whiffs at 0..=3, fires at 4..=5, then
        // commits; the loop re-fires 14 ticks later.
        assert!(!first.0[0], "jab age 0 must not emit");
        assert!(!first.0[3], "jab age 3 must not emit");
        assert!(first.0[4], "jab age 4 must emit");
        assert!(first.0[5], "jab age 5 must emit");
        assert!(!first.0[6], "jab age 6 must not emit");
        assert!(first.0[18], "jab re-press must emit again");
        // Cross rides once: quiet at 0, live at 6..=8, spent after 21.
        assert!(!first.1[0], "cross age 0 must not emit");
        assert!(first.1[6], "cross age 6 must emit");
        assert!(first.1[8], "cross age 8 must emit");
        assert!(!first.1[9], "cross age 9 must not emit");
        assert!(!first.1[23], "cross past total must not emit");
    }

    #[test]
    fn phased_steel_allocates_nothing() {
        let (_, heap) = engine_core::alloc_probe::measure(|| {
            let mut live = 0u32;
            let mut age = 0u32;
            while age < 48 {
                if sweep_live_for(&PHASED_V1, 0, age) {
                    live += 1;
                }
                if sweep_live_for(&PHASED_V1, 1, age) {
                    live += 1;
                }
                age += 1;
            }
            live
        });
        assert_eq!(heap.calls, 0, "phased steel allocated {} times", heap.calls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> SweepTables {
        SWEEP_V1
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn pose(
        id: u8,
        x: i32,
        z: i32,
        fx: i32,
        fz: i32,
        half_height: u32,
        shield_deg: u8,
        shield_up: bool,
    ) -> SweepParticipant {
        SweepParticipant {
            id,
            pos_mm: [x, 0, z],
            facing_x: fx,
            facing_z: fz,
            hurt_half_height_mm: half_height,
            hurt_radius_mm: 150,
            shield_half_deg: shield_deg,
            shield_up,
        }
    }

    fn duel_attacker() -> SweepParticipant {
        pose(0, -1000, 0, 1, 0, 800, 60, false)
    }

    fn duel_defender() -> SweepParticipant {
        pose(1, 0, 0, -1, 0, 800, 60, false)
    }

    fn sweep(attacker: u8, p0: [i32; 3], p1: [i32; 3], blade: u32) -> WeaponSweep {
        WeaponSweep {
            attacker,
            p0_mm: p0,
            p1_mm: p1,
            blade_half_mm: blade,
        }
    }

    fn resolve(fighters: &[SweepParticipant], sweeps: &[WeaponSweep]) -> (Vec<SweepEvent>, usize) {
        let t = tables();
        let mut out = [SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        }; MAX_SWEEP_EVENTS_PER_TICK];
        let n = resolve_tick(fighters, sweeps, &t, &mut out);
        (out[..n].to_vec(), n)
    }

    fn pair(e: SweepEvent) -> (u8, u8, bool) {
        match e {
            SweepEvent::Hit { attacker, victim } => (attacker, victim, false),
            SweepEvent::Blocked { attacker, victim } => (attacker, victim, true),
        }
    }

    #[test]
    fn v1_rows_match_consts() {
        let t = SWEEP_V1;
        assert_eq!(t.contact_slack_mm, 4);
        assert_eq!(t.max_sweep_len_mm, 6000);
        assert_eq!(SweepTableSet::v1().version, 1);
        assert_eq!(SweepTableSet::v1().validate(), Ok(()));
        assert_eq!(MAX_PARTICIPANTS, 16);
        assert_eq!(MAX_SWEEPS_PER_TICK, 8);
    }

    #[test]
    fn direct_blade_through_capsule_hits() {
        let t = tables();
        let atk = duel_attacker();
        let def = duel_defender();
        let s = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &def, &s, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn thin_fast_rapier_never_tunnels() {
        // 3 m single-tick crossing, 5 mm blade, victim off the midpoint: every
        // discrete sample (both ends plus the middle) misses, the continuous
        // segment runs through the capsule spine and must hit.
        let t = tables();
        let atk = pose(0, 0, 0, 1, 0, 800, 60, false);
        let def = pose(1, 700, 100, -1, 0, 800, 60, false);
        let p0 = [-1500, 0, 100];
        let p1 = [1500, 0, 100];
        let mid = [0, 0, 100];
        let probe = |p| sweep(0, p, p, 5);
        assert_eq!(
            sweep_one(&atk, &def, &probe(p0), &t),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
        assert_eq!(
            sweep_one(&atk, &def, &probe(p1), &t),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
        assert_eq!(
            sweep_one(&atk, &def, &probe(mid), &t),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
        assert_eq!(
            sweep_one(&atk, &def, &sweep(0, p0, p1, 5), &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn out_of_reach_sweep_misses() {
        let t = tables();
        let atk = duel_attacker();
        let def = duel_defender();
        // Blade sweeps a metre to the side of the capsule.
        let s = sweep(0, [-100, 0, 1000], [100, 0, 1000], 10);
        assert_eq!(
            sweep_one(&atk, &def, &s, &t),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
    }

    #[test]
    fn behind_attacker_misses_despite_contact() {
        let t = tables();
        // Victim behind the attacker while the blade overlaps the capsule.
        let atk = pose(0, 0, 0, 1, 0, 800, 60, false);
        let behind = pose(1, -500, 0, -1, 0, 800, 60, false);
        let s = sweep(0, [-600, 0, 0], [-400, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &behind, &s, &t),
            SweepOutcome::Miss(SweepMiss::BehindAttacker)
        );
        // Same geometry mirrored in front hits.
        let front = pose(1, 500, 0, -1, 0, 800, 60, false);
        let s = sweep(0, [400, 0, 0], [600, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &front, &s, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn facing_edge_sideways_misses_point_blank_hits() {
        let t = tables();
        let atk = pose(0, 0, 0, 1, 0, 800, 60, false);
        // Exactly abeam: dot is zero, the gate refuses.
        let side = pose(1, 0, 1000, -1, 0, 800, 60, false);
        let s = sweep(0, [-100, 0, 1000], [100, 0, 1000], 10);
        assert_eq!(
            sweep_one(&atk, &side, &s, &t),
            SweepOutcome::Miss(SweepMiss::BehindAttacker)
        );
        // Shared XZ means point-blank: the gate is skipped, steel decides.
        let close = pose(1, 0, 0, -1, 0, 800, 60, false);
        let s = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &close, &s, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn shield_cone_blocks_front_but_not_flank_or_lowered() {
        let t = tables();
        let atk = pose(0, -1000, 0, 1, 0, 800, 60, false);
        let s = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        // Defender faces the attacker behind a raised shield: blocked.
        let guard = pose(1, 0, 0, -1, 0, 800, 60, true);
        assert_eq!(
            sweep_one(&atk, &guard, &s, &t),
            SweepOutcome::Blocked {
                attacker: 0,
                victim: 1
            }
        );
        // Same steel from the defender's blind side: hits.
        let flanked = pose(1, 0, 0, 1, 0, 800, 60, true);
        assert_eq!(
            sweep_one(&atk, &flanked, &s, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
        // Facing the attacker with the shield down: hits.
        let open = pose(1, 0, 0, -1, 0, 800, 60, false);
        assert_eq!(
            sweep_one(&atk, &open, &s, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn shield_half_angle_decides_oblique_attacks() {
        // Attacker 45 degrees off the defender's facing; only the angle row
        // changes between the two solves.
        let t = tables();
        let atk = pose(0, -1000, -1000, 1, 1, 800, 60, false);
        let s = sweep(0, [-100, 0, -100], [100, 0, 100], 10);
        let narrow = pose(1, 0, 0, -1, 0, 800, 15, true);
        assert_eq!(
            sweep_one(&atk, &narrow, &s, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
        let wide = pose(1, 0, 0, -1, 0, 800, 60, true);
        assert_eq!(
            sweep_one(&atk, &wide, &s, &t),
            SweepOutcome::Blocked {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn contact_slack_covers_grazes_but_not_clear_misses() {
        // Point capsule (zero height) radius 150 plus blade 10 is 160; slack
        // 4 blocks 160..=164. Axis-aligned, so the distances are exact.
        let t = tables();
        let atk = pose(0, -1000, 0, 1, 0, 800, 60, false);
        let def = pose(1, 0, 0, -1, 0, 0, 60, false);
        let graze = sweep(0, [-500, 0, 162], [500, 0, 162], 10);
        assert_eq!(
            sweep_one(&atk, &def, &graze, &t),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
        let clear = sweep(0, [-500, 0, 170], [500, 0, 170], 10);
        assert_eq!(
            sweep_one(&atk, &def, &clear, &t),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
    }

    #[test]
    fn self_target_never_hits() {
        let t = tables();
        let atk = duel_attacker();
        let s = sweep(0, [-1100, 0, 0], [-900, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &atk, &s, &t),
            SweepOutcome::Miss(SweepMiss::SelfTarget)
        );
        let fighters = [atk, duel_defender()];
        let (events, n) = resolve(&fighters, &[s]);
        assert_eq!(n, 0);
        assert!(events.is_empty());
    }

    #[test]
    fn nonphysical_travel_short_circuits_to_reach_miss() {
        let t = tables();
        let atk = duel_attacker();
        let def = duel_defender();
        // 7 m in one tick overlaps the capsule but is not a real swing.
        let s = sweep(0, [-3500, 0, 0], [3500, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &def, &s, &t),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
        assert_eq!(
            validate_sweep(&s, 2, &t),
            Err(SweepError::BadValue {
                field: "sweep_len_mm",
                value: 7000
            })
        );
    }

    #[test]
    fn shuffled_inputs_give_identical_sorted_events() {
        // Two weapons on one victim; every permutation of fighters and sweeps
        // must resolve to the same attacker-ordered events.
        let f0 = pose(0, -1000, 0, 1, 0, 800, 60, false);
        let f1 = pose(1, 0, 0, -1, 0, 800, 60, false);
        let f2 = pose(2, 1000, 0, -1, 0, 800, 60, false);
        let s0 = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        let s2 = sweep(2, [-100, 0, 0], [100, 0, 0], 10);
        let expected = vec![
            SweepEvent::Hit {
                attacker: 0,
                victim: 1,
            },
            SweepEvent::Hit {
                attacker: 2,
                victim: 1,
            },
        ];
        let orders: [[usize; 3]; 6] = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        let base = [f0, f1, f2];
        for o in orders {
            let fighters = [base[o[0]], base[o[1]], base[o[2]]];
            let (a, _) = resolve(&fighters, &[s0, s2]);
            let (b, _) = resolve(&fighters, &[s2, s0]);
            assert_eq!(a, expected, "fighter order {o:?} sweeps forward");
            assert_eq!(b, expected, "fighter order {o:?} sweeps reversed");
        }
    }

    #[test]
    fn replay_exact_twice_over_sweep_tape() {
        let t = tables();
        let fighters = [duel_attacker(), duel_defender()];
        let tape: Vec<Vec<WeaponSweep>> = vec![
            vec![sweep(0, [-100, 0, 0], [100, 0, 0], 10)],
            vec![sweep(0, [-100, 0, 1000], [100, 0, 1000], 10)],
            vec![
                sweep(0, [-100, 0, 0], [100, 0, 0], 10),
                sweep(1, [-1100, 0, 0], [-900, 0, 0], 10),
            ],
            vec![],
        ];
        let run = || {
            let mut all = Vec::new();
            for sweeps in &tape {
                let mut out = [SweepEvent::Hit {
                    attacker: 0,
                    victim: 0,
                }; MAX_SWEEP_EVENTS_PER_TICK];
                let n = resolve_tick(&fighters, sweeps, &t, &mut out);
                all.push(out[..n].to_vec());
            }
            all
        };
        let a = run();
        let b = run();
        assert_eq!(a, b);
        assert_eq!(
            a[0],
            vec![SweepEvent::Hit {
                attacker: 0,
                victim: 1
            }]
        );
        assert!(a[1].is_empty());
    }

    #[test]
    fn validation_rejects_bad_versions_rows_and_slots() {
        let t = tables();
        assert_eq!(
            SweepTableSet::new(0, SWEEP_V1).validate(),
            Err(SweepError::BadVersion(0))
        );
        let mut bad = SWEEP_V1;
        bad.contact_slack_mm = 17;
        assert_eq!(
            SweepTableSet::new(3, bad).validate(),
            Err(SweepError::BadValue {
                field: "contact_slack_mm",
                value: 17
            })
        );
        let mut bad = SWEEP_V1;
        bad.max_sweep_len_mm = 50;
        assert_eq!(
            SweepTableSet::new(3, bad).validate(),
            Err(SweepError::BadValue {
                field: "max_sweep_len_mm",
                value: 50
            })
        );
        assert_eq!(
            try_sweep_one(
                &duel_attacker(),
                &duel_defender(),
                &sweep(0, [0, 0, 0], [10, 0, 0], 10),
                &t,
                0
            ),
            Err(SweepError::BadParticipantCount(0))
        );
        assert_eq!(
            try_sweep_one(
                &duel_attacker(),
                &duel_defender(),
                &sweep(0, [0, 0, 0], [10, 0, 0], 10),
                &t,
                MAX_PARTICIPANTS + 1
            ),
            Err(SweepError::BadParticipantCount(MAX_PARTICIPANTS + 1))
        );
        // Five fighters are legal under the 16-slot cap: routing still
        // validates slots against the count.
        assert_eq!(
            try_sweep_one(
                &duel_attacker(),
                &duel_defender(),
                &sweep(0, [0, 0, 0], [10, 0, 0], 10),
                &t,
                5
            )
            .map(|_| ()),
            Ok(())
        );
        let ghost = pose(7, 0, 0, 1, 0, 800, 60, false);
        assert_eq!(
            try_sweep_one(
                &ghost,
                &duel_defender(),
                &sweep(7, [0, 0, 0], [10, 0, 0], 10),
                &t,
                2
            ),
            Err(SweepError::BadSlot(7))
        );
        let routed = sweep(1, [0, 0, 0], [10, 0, 0], 10);
        assert_eq!(
            try_sweep_one(&duel_attacker(), &duel_defender(), &routed, &t, 2),
            Err(SweepError::BadSlot(1))
        );
        let mut far = duel_defender();
        far.pos_mm = [POS_BOUND_MM + 1, 0, 0];
        assert_eq!(
            validate_fighter(&far, 2),
            Err(SweepError::BadValue {
                field: "pos_mm",
                value: (POS_BOUND_MM as u32) + 1
            })
        );
        let mut flat = duel_attacker();
        flat.facing_x = 0;
        flat.facing_z = 0;
        assert_eq!(
            validate_fighter(&flat, 2),
            Err(SweepError::BadValue {
                field: "facing",
                value: 0
            })
        );
        let mut wide = duel_defender();
        wide.shield_half_deg = 91;
        assert_eq!(
            validate_fighter(&wide, 2),
            Err(SweepError::BadValue {
                field: "shield_half_deg",
                value: 91
            })
        );
        let mut thin = duel_defender();
        thin.hurt_radius_mm = 0;
        assert_eq!(
            validate_fighter(&thin, 2),
            Err(SweepError::BadValue {
                field: "hurt_radius_mm",
                value: 0
            })
        );
        let fat = sweep(0, [0, 0, 0], [10, 0, 0], BLADE_HALF_MAX_MM + 1);
        assert_eq!(
            validate_sweep(&fat, 2, &t),
            Err(SweepError::BadValue {
                field: "blade_half_mm",
                value: BLADE_HALF_MAX_MM + 1
            })
        );
    }

    #[test]
    fn strict_tick_rejects_counts_duplicates_and_overflow() {
        let t = tables();
        let mut out = [SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        }; MAX_SWEEP_EVENTS_PER_TICK];
        assert_eq!(
            try_resolve_tick(&[], &[], &t, &mut out),
            Err(SweepError::BadParticipantCount(0))
        );
        // Five fighters are legal under the 16-slot cap; seventeen refuse.
        let five = [
            duel_attacker(),
            duel_defender(),
            pose(2, 0, 0, 1, 0, 800, 60, false),
            pose(3, 0, 0, 1, 0, 800, 60, false),
            pose(4, 0, 0, 1, 0, 800, 60, false),
        ];
        assert!(try_resolve_tick(&five, &[], &t, &mut out).is_ok());
        let seventeen: Vec<SweepParticipant> = (0..17u8)
            .map(|id| pose(id, 0, 0, 1, 0, 800, 60, false))
            .collect();
        assert_eq!(
            try_resolve_tick(&seventeen, &[], &t, &mut out),
            Err(SweepError::TooManyFighters(17))
        );
        let dup = [duel_attacker(), duel_attacker()];
        assert_eq!(
            try_resolve_tick(&dup, &[], &t, &mut out),
            Err(SweepError::DuplicateFighter(0))
        );
        let fighters = [duel_attacker(), duel_defender()];
        let many = [sweep(0, [0, 0, 0], [1, 0, 0], 1); MAX_SWEEPS_PER_TICK + 1];
        assert_eq!(
            try_resolve_tick(&fighters, &many, &t, &mut out),
            Err(SweepError::TooManySweeps(MAX_SWEEPS_PER_TICK + 1))
        );
        // The same valid tick passes strict and matches the lenient path.
        let one = [sweep(0, [-100, 0, 0], [100, 0, 0], 10)];
        let n = try_resolve_tick(&fighters, &one, &t, &mut out).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            out[0],
            SweepEvent::Hit {
                attacker: 0,
                victim: 1
            }
        );
    }

    #[test]
    fn fighter_count_u8_refuses_above_u8_max() {
        // Narrowing pin: the strict tick
        // funnels its participant count through one checked conversion, so a
        // length past `u8::MAX` refuses as `TooManyFighters` instead of
        // wrapping the low bits with `as`.
        assert_eq!(fighter_count_u8(0), Ok(0));
        assert_eq!(fighter_count_u8(16), Ok(16));
        assert_eq!(fighter_count_u8(255), Ok(255));
        assert_eq!(fighter_count_u8(256), Err(SweepError::TooManyFighters(256)));
        assert_eq!(
            fighter_count_u8(usize::MAX),
            Err(SweepError::TooManyFighters(usize::MAX))
        );
    }

    #[test]
    fn lenient_paths_never_panic_on_garbage() {
        let t = tables();
        let wild = SweepParticipant {
            id: 0,
            pos_mm: [i32::MAX, i32::MIN, 0],
            facing_x: i32::MIN,
            facing_z: i32::MAX,
            hurt_half_height_mm: u32::MAX,
            hurt_radius_mm: u32::MAX,
            shield_half_deg: u8::MAX,
            shield_up: true,
        };
        let wild_sweep = WeaponSweep {
            attacker: 0,
            p0_mm: [i32::MIN, i32::MAX, 0],
            p1_mm: [i32::MAX, i32::MIN, 0],
            blade_half_mm: u32::MAX,
        };
        let tame = duel_defender();
        // Every pairing of wild and tame inputs resolves to a miss, not a trap.
        assert!(matches!(
            sweep_one(&wild, &tame, &wild_sweep, &t),
            SweepOutcome::Miss(_)
        ));
        assert!(matches!(
            sweep_one(&tame, &wild, &wild_sweep, &t),
            SweepOutcome::Miss(_)
        ));
        let fighters = [wild, tame, duel_attacker()];
        let sweeps = [wild_sweep; MAX_SWEEPS_PER_TICK + 3];
        let mut out = [SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        }; MAX_SWEEP_EVENTS_PER_TICK];
        let n = resolve_tick(&fighters, &sweeps, &t, &mut out);
        assert!(n <= out.len());
        let empty: [SweepParticipant; 0] = [];
        assert_eq!(resolve_tick(&empty, &[], &t, &mut out), 0);
    }

    #[test]
    fn integer_sqrt_and_cosine_helpers_hold() {
        assert_eq!(0u64.isqrt(), 0);
        assert_eq!(1u64.isqrt(), 1);
        assert_eq!(2u64.isqrt(), 1);
        assert_eq!(15u64.isqrt(), 3);
        assert_eq!(16u64.isqrt(), 4);
        assert_eq!(3_200_000_000_000u64.isqrt(), 1_788_854);
        assert_eq!(u64::MAX.isqrt(), 4_294_967_295);
        // Cosine rows: exact at 0 degrees, near-exact at 60, near zero at
        // 90, monotone between (fixed-point rounding needs a few counts).
        assert_eq!(cos_half_q16(0), 65536);
        assert!((cos_half_q16(60) as i64 - 32768).abs() <= 8);
        assert!(cos_half_q16(90) <= 8);
        assert!(cos_half_q16(15) > cos_half_q16(30));
        assert!(cos_half_q16(30) > cos_half_q16(45));
        assert!(cos_half_q16(45) > cos_half_q16(60));
        assert!(cos_half_q16(60) > cos_half_q16(75));
    }

    fn sweep_hash(all: &[Vec<SweepEvent>]) -> u64 {
        const FNV_OFF: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let mut h = FNV_OFF;
        let mut mix = |w: u64| {
            h ^= w;
            h = h.wrapping_mul(FNV_PRIME);
        };
        for tick_events in all {
            for e in tick_events {
                match *e {
                    SweepEvent::Hit { attacker, victim } => {
                        mix(1);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                    }
                    SweepEvent::Blocked { attacker, victim } => {
                        mix(2);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                    }
                }
            }
            mix(0xFF);
        }
        h
    }

    #[test]
    fn slots_beyond_four_resolve_in_id_order() {
        // Generic slots: attackers past the old 4-cap resolve in
        // attacker-ID order, and the full 16-slot cap validates.
        let t = tables();
        let fighters = [
            pose(0, -1000, 0, 1, 0, 800, 60, false),
            pose(1, 0, 0, -1, 0, 800, 60, false),
            pose(2, 2000, 0, -1, 0, 800, 60, false),
            pose(3, 3000, 0, -1, 0, 800, 60, false),
            pose(4, -1000, 0, 1, 0, 800, 60, false),
            pose(5, 4000, 0, -1, 0, 800, 60, false),
        ];
        let s0 = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        let s4 = sweep(4, [-100, 0, 0], [100, 0, 0], 10);
        let (events, _) = resolve(&fighters, &[s0, s4]);
        assert_eq!(
            events,
            vec![
                SweepEvent::Hit {
                    attacker: 0,
                    victim: 1
                },
                SweepEvent::Hit {
                    attacker: 4,
                    victim: 1
                },
            ]
        );
        // Six fighters pass the strict path; sixteen validate, seventeen refuse.
        let mut out = [SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        }; MAX_SWEEP_EVENTS_PER_TICK];
        assert!(try_resolve_tick(&fighters, &[s0], &t, &mut out).is_ok());
        let sixteen: Vec<SweepParticipant> = (0..16u8)
            .map(|id| pose(id, 0, 0, 1, 0, 800, 60, false))
            .collect();
        assert!(try_resolve_tick(&sixteen, &[], &t, &mut out).is_ok());
        let seventeen: Vec<SweepParticipant> = (0..17u8)
            .map(|id| pose(id, 0, 0, 1, 0, 800, 60, false))
            .collect();
        assert_eq!(
            try_resolve_tick(&seventeen, &[], &t, &mut out),
            Err(SweepError::TooManyFighters(17))
        );
    }

    #[test]
    fn fixture_sweep_rows_resolve_without_engine_edits() {
        // Tables own tuning: a versioned fixture set with different numbers
        // resolves through the same solvers, no engine edits.
        let v2 = SweepTables {
            contact_slack_mm: 8,
            max_sweep_len_mm: 3000,
        };
        assert_eq!(SweepTableSet::new(2, v2).validate(), Ok(()));
        let atk = pose(0, -1000, 0, 1, 0, 800, 60, false);
        let pin = pose(1, 0, 0, -1, 0, 0, 60, false);
        // Graze at 166 mm: past V1 reach (150 + 10 + 4) but inside the
        // fixture slack (150 + 10 + 8).
        let graze = sweep(0, [-500, 0, 166], [500, 0, 166], 10);
        assert_eq!(
            sweep_one(&atk, &pin, &graze, &SWEEP_V1),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
        assert_eq!(
            sweep_one(&atk, &pin, &graze, &v2),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
        // 4 m single-tick travel: a real swing under V1, non-physical under
        // the tighter fixture bound.
        let far = sweep(0, [-2000, 0, 0], [2000, 0, 0], 10);
        assert_eq!(
            sweep_one(&atk, &pin, &far, &SWEEP_V1),
            SweepOutcome::Hit {
                attacker: 0,
                victim: 1
            }
        );
        assert_eq!(
            sweep_one(&atk, &pin, &far, &v2),
            SweepOutcome::Miss(SweepMiss::OutOfReach)
        );
        assert_eq!(
            validate_sweep(&far, 2, &v2),
            Err(SweepError::BadValue {
                field: "sweep_len_mm",
                value: 4000
            })
        );
        // Each row replays identically twice over a short tape.
        let tape: Vec<Vec<WeaponSweep>> = vec![vec![graze], vec![far], vec![]];
        let run = |rows: &SweepTables| {
            let fighters = [atk, pin];
            tape.iter()
                .map(|sweeps| {
                    let mut out = [SweepEvent::Hit {
                        attacker: 0,
                        victim: 0,
                    }; MAX_SWEEP_EVENTS_PER_TICK];
                    let n = resolve_tick(&fighters, sweeps, rows, &mut out);
                    out[..n].to_vec()
                })
                .collect::<Vec<_>>()
        };
        let a = run(&SWEEP_V1);
        let b = run(&SWEEP_V1);
        assert_eq!(sweep_hash(&a), sweep_hash(&b));
        assert!(a.iter().any(|tick| !tick.is_empty()));
        let c = run(&v2);
        let d = run(&v2);
        assert_eq!(sweep_hash(&c), sweep_hash(&d));
        assert!(c.iter().any(|tick| !tick.is_empty()));
        // The numbers decide: the two rows answer the same tape differently.
        assert_ne!(a, c);
    }

    #[test]
    fn swing_gate_hits_each_victim_once_across_six_ticks() {
        // LF11 spike (b): one swing arc over 6 ticks vs 3 victims; each
        // takes exactly one Hit in ID order, replayed byte-equal twice.
        let t = tables();
        let fighters = [
            pose(0, -1000, 0, 1, 0, 800, 60, false),
            pose(1, 0, 0, -1, 0, 800, 60, false),
            pose(2, 0, 0, -1, 0, 800, 60, false),
            pose(3, 0, 0, -1, 0, 800, 60, false),
        ];
        let arc = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        let run = || {
            let mut gate = SwingGate::fresh(7, 0);
            let mut all = Vec::new();
            let mut tick = 0;
            while tick < 6 {
                let mut out = [SweepEvent::Hit {
                    attacker: 0,
                    victim: 0,
                }; MAX_SWEEP_EVENTS_PER_TICK];
                let n = resolve_swing_tick(&fighters, &[arc], &t, 7, 0, &mut gate, &mut out);
                all.push(out[..n].to_vec());
                tick += 1;
            }
            all
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "same swing ticks replay byte-equal x2");
        let total: usize = a.iter().map(Vec::len).sum();
        assert_eq!(
            total, 3,
            "3 victims take exactly one Hit per swing, got {a:?}"
        );
        assert_eq!(a[0].len(), 3, "first tick hits all 3 victims: {a:?}");
        assert_eq!(pair(a[0][0]), (0, 1, false));
        assert_eq!(pair(a[0][1]), (0, 2, false));
        assert_eq!(pair(a[0][2]), (0, 3, false));
        assert!(
            a[1..].iter().all(Vec::is_empty),
            "later ticks of the same swing skip: {a:?}"
        );
    }

    #[test]
    fn swing_gate_blocked_marks_and_fresh_gate_reemits() {
        // First contact wins, shielded or not; a new id re-arms, and a
        // gate never suppresses another attacker.
        let t = tables();
        let fighters = [
            pose(0, -1000, 0, 1, 0, 800, 60, false),
            pose(1, 0, 0, -1, 0, 800, 60, true),
        ];
        let arc = sweep(0, [-100, 0, 0], [100, 0, 0], 10);
        let mut gate = SwingGate::fresh(7, 0);
        let mut out = [SweepEvent::Hit {
            attacker: 0,
            victim: 0,
        }; MAX_SWEEP_EVENTS_PER_TICK];
        let n0 = resolve_swing_tick(&fighters, &[arc], &t, 7, 0, &mut gate, &mut out);
        assert_eq!(n0, 1);
        assert_eq!(pair(out[0]), (0, 1, true));
        let n1 = resolve_swing_tick(&fighters, &[arc], &t, 7, 0, &mut gate, &mut out);
        assert_eq!(n1, 0, "blocked contact still marks the swing");
        let n2 = resolve_swing_tick(&fighters, &[arc], &t, 8, 0, &mut gate, &mut out);
        assert_eq!(n2, 1, "a new swing id re-arms every victim");
        let n3 = resolve_swing_tick(&fighters, &[arc], &t, 8, 1, &mut gate, &mut out);
        assert_eq!(n3, 1, "a gate tracks only its own attacker");
    }
}
