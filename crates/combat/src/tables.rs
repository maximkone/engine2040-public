//! Versioned replacement move tables with validation and an immutable swap input.
//!
//! Consumer: the arena (live tuning) and the tuning panel. The arena
//! consumes a [`SwapInput`] at match build and applies a swap only at
//! recorded authority tick commands; this module only defines and validates
//! the input, it never touches a running match. Data owner: `engine-combat`.
//! Units match [`Tables`]: damage in permille, costs in stamina points, all
//! timing in fixed ticks (60 Hz). Determinism scope: [`validate_tables`]
//! and version resolution are pure; [`step_with_tables`] under the resolved
//! rows uses integer math and fighter-ID order only. Errors: [`CombatError`]
//! for bad versions, out-of-range rows and unknown replay versions.
//!
//! Version tags: [`TABLES_VERSION_V1`] (1) is the shipped base row kept in
//! [`TABLES_V1`]. Tag 0 means a legacy untagged tape and also resolves to
//! V1, so old tapes replay through the old rules. Replacement sets travel
//! as [`SwapInput`], which only constructs when [`validate_tables`] passes.
//!
//! Grandfather note: V1 ships `parry_window_ticks == attack_startup_ticks
//! == 4`, which the replacement rule `parry_window < startup` would reject.
//! V1 predates the rule and stays valid for replays; every *new* set must
//! satisfy the strict rule so a held block always ages out of the parry
//! window before the next impact.
//!
//! [`step_with_tables`]: crate::fight::step_with_tables
//! [`TABLES_V1`]: crate::fight::TABLES_V1
//! [`Tables`]: crate::fight::Tables

use crate::fight::{
    step_with_tables, try_step_with_tables, CombatError, FightEvent, FightState, Tables,
    MAX_HP_PERMILLE, MAX_STAMINA, TABLES_V1, TABLES_V2,
};
use engine_input::FighterInput;

/// Table version of the shipped base row kept in [`TABLES_V1`].
///
/// [`TABLES_V1`]: crate::fight::TABLES_V1
pub const TABLES_VERSION_V1: u32 = 1;

/// Table version of the readable-timing row kept in [`TABLES_V2`].
///
/// [`TABLES_V2`]: crate::fight::TABLES_V2
pub const TABLES_VERSION_V2: u32 = 2;

/// Ticks a press survives in the input buffer under V1 rows: none, so V1
/// bouts replay byte-identical. Units: ticks at 60 Hz.
pub const INPUT_BUFFER_TICKS_V1: u32 = 0;

/// Ticks a press survives in the input buffer under V2 rows: 8 ticks
/// (133 ms at 60 Hz), so a tap in recovery is kept for the first ready tick.
/// Single owner of the 8-tick value; the fight intake reads it through
/// [`buffer_window`](crate::feel::buffer_window), and
/// [`StreamliningParams::input_buffer_ticks`](crate::feel::StreamliningParams::input_buffer_ticks)
/// mirrors it rather than holding a second window. Units: ticks at 60 Hz.
/// This row is a forgiveness budget, never a latency: the PQ1 pin reads the
/// ticks the tick adds to a fresh press
/// ([`FRESH_PRESS_ADDED_TICKS`](crate::feel::FRESH_PRESS_ADDED_TICKS)) and
/// never this value.
pub const INPUT_BUFFER_TICKS_V2: u32 = 8;

/// V2 poise damage of one light strike, in points.
///
/// Consumer: [`poise_damage_for_kind`](crate::fight::poise_damage_for_kind)
/// maps it into [`PoiseMeter`](crate::depth::PoiseMeter); the depth set owns
/// the meter. Units: poise points against a 100 cap with 1 per quiet tick.
/// Determinism: pure const; same kind plus same tables give same damage.
/// A single light (60) leaves 40 on a full meter, so one never staggers
/// while two inside the regen delay do.
pub const LIGHT_POISE_DAMAGE_V2: u16 = 60;

/// V2 poise damage of one heavy strike, in points.
///
/// Consumer and units match [`LIGHT_POISE_DAMAGE_V2`]. A heavy (100) empties
/// a full 100 meter, so one heavy always staggers. V1 rows carry no poise
/// damage (the fight helper returns 0), so old tapes never trip this path.
pub const HEAVY_POISE_DAMAGE_V2: u16 = 100;

/// V2 stamina cost of one light attack, in points.
///
/// Consumer: [`stamina_cost_for_kind`](crate::fight::stamina_cost_for_kind)
/// into `DepthSet::spend_stamina_kind` (test builds only).
/// Units: stamina points against a 100 cap. Determinism: pure const.
pub const LIGHT_STAMINA_COST_V2: u16 = 15;

/// V2 stamina cost of one heavy attack, in points.
///
/// Consumer and units match [`LIGHT_STAMINA_COST_V2`]. Heavy (30) costs
/// twice the light (15), so spam tires the leader first. V1 keeps its single
/// [`ATTACK_STAMINA_COST`](crate::fight::ATTACK_STAMINA_COST) for both kinds.
pub const HEAVY_STAMINA_COST_V2: u16 = 30;

/// V2 quiet ticks before poise regens, at 60 Hz.
///
/// Consumer: [`poise_regen_delay_for_tables`](crate::fight::poise_regen_delay_for_tables)
/// into [`DepthSet::new`](crate::depth::DepthSet::new). Units: ticks.
/// The 90-tick [`DEFAULT_POISE_DELAY_TICKS`](crate::depth::DEFAULT_POISE_DELAY_TICKS)
/// row stays survey-only; live V2 bouts start at 30.
pub const POISE_REGEN_DELAY_TICKS_V2: u32 = 30;

/// V2 freeze ticks for one light hit, at 60 Hz.
///
/// Consumer: [`for_kind`](crate::hitstop_kind::for_kind) via
/// [`for_impact`](crate::hitstop_kind::for_impact) into
/// [`hitstop_for_impact`](crate::fight::hitstop_for_impact) and the hit
/// branch of the live step. Units: ticks. A light taps (3), so weight reads
/// in the freeze. V1 keeps its single
/// [`HITSTOP_TICKS`](crate::fight::HITSTOP_TICKS).
pub const LIGHT_HITSTOP_TICKS_V2: u32 = 3;

/// V2 freeze ticks for one heavy hit, at 60 Hz.
///
/// Consumer: [`for_kind`](crate::hitstop_kind::for_kind) via
/// [`for_impact`](crate::hitstop_kind::for_impact) into
/// [`hitstop_for_impact`](crate::fight::hitstop_for_impact); the live step
/// reads the strike kind from `FighterState::attack_kind`. Units: ticks.
/// Heavy (6) freezes twice the light, so weight reads.
pub const HEAVY_HITSTOP_TICKS_V2: u32 = 6;

/// V2 freeze ticks for one parry, at 60 Hz.
///
/// Consumer: [`for_parry`](crate::hitstop_kind::for_parry) via
/// [`for_impact`](crate::hitstop_kind::for_impact) into
/// [`hitstop_for_impact`](crate::fight::hitstop_for_impact) and the parry
/// branch of the live step. Units: ticks. Parry (8) freezes longest, so the
/// answer reads. V1 keeps its single row.
pub const PARRY_HITSTOP_TICKS_V2: u32 = 8;

/// Readable-timing row: [`TABLES_V1`] with a slower light-attack startup
/// (15 ticks, inside 12-18) and a wider parry window (7 ticks, inside 6-8).
/// Every other row matches V1, and 7 < 15 satisfies the replacement rule,
/// so [`validate_tables`] accepts version 2. Single owner of the 15/7
/// literals; [`TABLES_V2`](crate::fight::TABLES_V2) reuses this row.
///
/// [`TABLES_V1`]: crate::fight::TABLES_V1
pub const fn v2_rows() -> Tables {
    Tables {
        attack_startup_ticks: 15,
        parry_window_ticks: 7,
        ..TABLES_V1
    }
}

/// Sane upper bounds for replacement rows, in the units of [`Tables`].
/// Damage never exceeds full health, startup fits in a second, recoveries
/// and staggers fit in two seconds, hit-stop fits in a second, knockdown in
/// ten seconds, costs never exceed full stamina, and chip never exceeds
/// full damage (`num <= den`, `den <= 16`).
const MAX_STARTUP_TICKS: u32 = 60;
const MAX_RECOVERY_TICKS: u32 = 120;
const MAX_DODGE_TICKS: u32 = 120;
const MAX_PARRY_WINDOW_TICKS: u32 = 60;
const MAX_STAGGER_TICKS: u32 = 120;
const MAX_HITSTOP_TICKS: u32 = 60;
const MAX_KNOCKDOWN_TICKS: u32 = 600;
const MAX_CHIP_DEN: u32 = 16;

/// A versioned set of move-table rows: the current tables as version 1, or
/// a replacement built from explicit rows.
///
/// `version` 0 is reserved for legacy untagged replays and rejected here;
/// use [`MoveTableSet::v1`] for the shipped rules or [`MoveTableSet::new`]
/// with an explicit nonzero version, then [`validate_tables`] (or
/// [`MoveTableSet::validate`]) before handing rows to the arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveTableSet {
    /// Table version; nonzero, 1 for the shipped rules.
    pub version: u32,
    /// The tunable rows stepped under this version.
    pub tables: Tables,
}

impl MoveTableSet {
    /// The shipped base rules as version 1.
    pub const fn v1() -> Self {
        Self {
            version: TABLES_VERSION_V1,
            tables: TABLES_V1,
        }
    }

    /// The readable-timing rules as version 2: [`v2_rows`] under
    /// [`TABLES_VERSION_V2`]. Validates under [`validate_tables`]; the arena
    /// steps this row through [`crate::fight::try_step_v2_into`] and
    /// [`crate::fight::try_step_v2_with_cues_into`].
    pub const fn v2() -> Self {
        Self {
            version: TABLES_VERSION_V2,
            tables: v2_rows(),
        }
    }

    /// Constructor from explicit rows and a nonzero version. Validation is
    /// separate: call [`validate_tables`] (or [`MoveTableSet::validate`]);
    /// only validated sets become a [`SwapInput`].
    pub const fn new(version: u32, tables: Tables) -> Self {
        Self { version, tables }
    }

    /// Range-check this set; see [`validate_tables`] for the rules.
    pub fn validate(&self) -> Result<(), CombatError> {
        validate_tables(self)
    }
}

/// Range-check a versioned table set. First failure wins, deterministically.
///
/// Rules: version is nonzero; damage is `1..=MAX_HP_PERMILLE` (no
/// zero-damage attacks, never more than full health); startup is
/// `1..=60`; recovery `0..=120`; stamina costs and regen `0..=MAX_STAMINA`;
/// dodge duration `1..=120` with i-frames inside it; chip denominator
/// `1..=16` with numerator inside it; parry window `0..=60` and strictly
/// below startup; stagger and hit-react `0..=120`; hit-stop `0..=60`;
/// knockdown `0..=600`.
pub fn validate_tables(set: &MoveTableSet) -> Result<(), CombatError> {
    if set.version == 0 {
        return Err(CombatError::BadTableVersion(0));
    }
    let t = &set.tables;
    range(
        "attack_damage",
        u32::from(t.attack_damage),
        1,
        u32::from(MAX_HP_PERMILLE),
    )?;
    range(
        "attack_startup_ticks",
        t.attack_startup_ticks,
        1,
        MAX_STARTUP_TICKS,
    )?;
    range(
        "attack_recovery_ticks",
        t.attack_recovery_ticks,
        0,
        MAX_RECOVERY_TICKS,
    )?;
    range(
        "attack_stamina_cost",
        u32::from(t.attack_stamina_cost),
        0,
        u32::from(MAX_STAMINA),
    )?;
    range(
        "dodge_stamina_cost",
        u32::from(t.dodge_stamina_cost),
        0,
        u32::from(MAX_STAMINA),
    )?;
    range(
        "block_hold_cost_per_tick",
        u32::from(t.block_hold_cost_per_tick),
        0,
        u32::from(MAX_STAMINA),
    )?;
    range(
        "stamina_regen_per_tick",
        u32::from(t.stamina_regen_per_tick),
        0,
        u32::from(MAX_STAMINA),
    )?;
    range(
        "dodge_duration_ticks",
        t.dodge_duration_ticks,
        1,
        MAX_DODGE_TICKS,
    )?;
    range(
        "dodge_iframes_ticks",
        t.dodge_iframes_ticks,
        0,
        t.dodge_duration_ticks,
    )?;
    range("block_chip_den", t.block_chip_den, 1, MAX_CHIP_DEN)?;
    range("block_chip_num", t.block_chip_num, 0, t.block_chip_den)?;
    range(
        "parry_window_ticks",
        t.parry_window_ticks,
        0,
        MAX_PARRY_WINDOW_TICKS,
    )?;
    // A held block must age out of the parry window before the next impact,
    // so a passive holder chips instead of parrying forever.
    if t.parry_window_ticks >= t.attack_startup_ticks {
        return Err(CombatError::BadTableValue {
            field: "parry_window_ticks",
            value: t.parry_window_ticks,
        });
    }
    range(
        "parry_stagger_ticks",
        t.parry_stagger_ticks,
        0,
        MAX_STAGGER_TICKS,
    )?;
    range("hit_react_ticks", t.hit_react_ticks, 0, MAX_STAGGER_TICKS)?;
    range("hitstop_ticks", t.hitstop_ticks, 0, MAX_HITSTOP_TICKS)?;
    range("knockdown_ticks", t.knockdown_ticks, 0, MAX_KNOCKDOWN_TICKS)?;
    Ok(())
}

/// One inclusive-range check with the field name carried on failure.
fn range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), CombatError> {
    if value < min || value > max {
        return Err(CombatError::BadTableValue { field, value });
    }
    Ok(())
}

/// Immutable validated swap input for the arena at match build.
///
/// Private fields plus a validating constructor make a recorded swap a
/// snapshot: the input owns its rows, so later tuning edits never mutate a
/// swap already handed to the arena. Cloning is explicit (`Clone`, never an
/// implicit copy), so clone-and-modify leaves the original intact. The
/// arena applies a swap only at recorded authority tick commands;
/// this type never steps a match by itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SwapInput {
    version: u32,
    tables: Tables,
}

impl SwapInput {
    /// Validated constructor: rejects version 0 and out-of-range rows with
    /// [`CombatError`], mirroring [`validate_tables`].
    pub fn new(version: u32, tables: Tables) -> Result<Self, CombatError> {
        validate_tables(&MoveTableSet { version, tables })?;
        Ok(Self { version, tables })
    }

    /// Validated table version carried by this input.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Validated rows, returned by value: a snapshot, never an alias.
    pub const fn tables(&self) -> Tables {
        self.tables
    }

    /// The full versioned set for stepping or recording.
    pub const fn table_set(&self) -> MoveTableSet {
        MoveTableSet {
            version: self.version,
            tables: self.tables,
        }
    }
}

/// Resolve a replay version tag to its static rows: 1 and legacy 0 give
/// the shipped V1 row; 2 gives the readable-timing V2 row; anything else
/// is unknown. Replacement sets are not
/// statics: they travel as [`SwapInput`] and step via
/// [`step_with_tables`](crate::fight::step_with_tables).
pub fn tables_for_version(version: u32) -> Option<&'static Tables> {
    match version {
        0 | TABLES_VERSION_V1 => Some(&TABLES_V1),
        TABLES_VERSION_V2 => Some(&TABLES_V2),
        _ => None,
    }
}

/// Hits to kill from one damage number: ceil(hp / damage) in permille.
///
/// Consumer: the GP33 time-to-kill table (playtest reads this per tier and
/// move kind into `target/reports/rome-ttk.csv`). Data owner:
/// `engine-combat` (the table rows own the damage numbers). Units: permille
/// in, hit count out. Determinism: pure integer math, same inputs give the
/// same count on every PC. Edge cases: zero damage returns `u32::MAX`
/// (never kills), zero hp returns 0 (already down).
pub fn hits_to_kill(attack_damage_permille: u16, hp_permille: u16) -> u32 {
    if hp_permille == 0 {
        return 0;
    }
    if attack_damage_permille == 0 {
        return u32::MAX;
    }
    // Total widening: `From` keeps every value exact. Ceiling division
    // counts the remainder hit; both inputs are nonzero here.
    let hp = u32::from(hp_permille);
    let dmg = u32::from(attack_damage_permille);
    hp.div_ceil(dmg)
}

/// Upper-bound ticks to land a kill: one full cycle per landed hit.
///
/// Consumer, owner, units and determinism match [`hits_to_kill`]. One cycle
/// is startup plus recovery plus hit-stop freeze, so `hits * cycle` bounds
/// the fastest possible kill when every swing lands. Saturates instead of
/// wrapping, so hostile rows stay finite and replay-identical in debug and
/// release.
pub fn ttk_ticks(hits: u32, startup_ticks: u32, recovery_ticks: u32, hitstop_ticks: u32) -> u64 {
    // Total widening: `From` keeps every value exact.
    let cycle = (u64::from(startup_ticks))
        .saturating_add(u64::from(recovery_ticks))
        .saturating_add(u64::from(hitstop_ticks));
    (u64::from(hits)).saturating_mul(cycle)
}

/// Upper-bound ticks to kill under one table row at full health.
///
/// Consumer, owner, units and determinism match [`hits_to_kill`]. Shorthand
/// over [`hits_to_kill`] plus [`ttk_ticks`] with this row's damage,
/// startup, recovery and hit-stop, so the future playtest TTK module calls
/// one function per (tier, move kind) pair.
pub fn ttk_ticks_for_tables(tables: Tables, hp_permille: u16) -> u64 {
    ttk_ticks(
        hits_to_kill(tables.attack_damage, hp_permille),
        tables.attack_startup_ticks,
        tables.attack_recovery_ticks,
        tables.hitstop_ticks,
    )
}

/// Lenient replay step under a version tag: resolves the rows, then steps
/// like [`step_with_tables`](crate::fight::step_with_tables). Unknown
/// versions fail with [`CombatError::UnknownTableVersion`] and step
/// nothing, so old tapes always replay through V1.
pub fn step_for_version(
    inputs: &[FighterInput],
    state: &mut FightState,
    version: u32,
) -> Result<Vec<FightEvent>, CombatError> {
    let tables = tables_for_version(version).ok_or(CombatError::UnknownTableVersion(version))?;
    Ok(step_with_tables(inputs, state, tables))
}

/// Strict replay step under a version tag: like [`step_for_version`] but
/// rejects bad inputs with [`CombatError`].
pub fn try_step_for_version(
    inputs: &[FighterInput],
    state: &mut FightState,
    version: u32,
) -> Result<Vec<FightEvent>, CombatError> {
    let tables = tables_for_version(version).ok_or(CombatError::UnknownTableVersion(version))?;
    try_step_with_tables(inputs, state, tables)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::{
        step, ATTACK_DAMAGE, ATTACK_RECOVERY_TICKS, ATTACK_STAMINA_COST, ATTACK_STARTUP_TICKS,
        BLOCK_CHIP_DEN, BLOCK_CHIP_NUM, BLOCK_HOLD_COST_PER_TICK, DODGE_DURATION_TICKS,
        DODGE_IFRAMES_TICKS, DODGE_STAMINA_COST, HITSTOP_TICKS, HIT_REACT_TICKS, KNOCKDOWN_TICKS,
        PARRY_STAGGER_TICKS, PARRY_WINDOW_TICKS, STAMINA_REGEN_PER_TICK,
    };
    use engine_input::fighter::{ATTACK, BLOCK};
    use engine_input::{Buttons, FighterId, FighterInput};

    fn input(fighter: u8, buttons: u8) -> FighterInput {
        FighterInput {
            tick: 0,
            fighter: FighterId::new(fighter).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(buttons),
        }
    }

    fn none(fighter: u8) -> FighterInput {
        input(fighter, 0)
    }

    /// Rows that satisfy the new strict rules: V1 with the parry window
    /// tightened below startup (V1 itself is grandfathered for replays).
    fn strict_rows() -> Tables {
        let mut rows = TABLES_V1;
        rows.parry_window_ticks = ATTACK_STARTUP_TICKS - 1;
        rows
    }

    fn bad(version: u32, f: fn(&mut Tables)) -> Result<(), CombatError> {
        let mut rows = strict_rows();
        f(&mut rows);
        validate_tables(&MoveTableSet::new(version, rows))
    }

    #[test]
    fn v1_constant_keeps_shipped_rows() {
        let t = TABLES_V1;
        assert_eq!(t.attack_damage, ATTACK_DAMAGE);
        assert_eq!(t.attack_startup_ticks, ATTACK_STARTUP_TICKS);
        assert_eq!(t.attack_recovery_ticks, ATTACK_RECOVERY_TICKS);
        assert_eq!(t.attack_stamina_cost, ATTACK_STAMINA_COST);
        assert_eq!(t.dodge_stamina_cost, DODGE_STAMINA_COST);
        assert_eq!(t.block_hold_cost_per_tick, BLOCK_HOLD_COST_PER_TICK);
        assert_eq!(t.stamina_regen_per_tick, STAMINA_REGEN_PER_TICK);
        assert_eq!(t.dodge_duration_ticks, DODGE_DURATION_TICKS);
        assert_eq!(t.dodge_iframes_ticks, DODGE_IFRAMES_TICKS);
        assert_eq!(t.block_chip_num, BLOCK_CHIP_NUM);
        assert_eq!(t.block_chip_den, BLOCK_CHIP_DEN);
        assert_eq!(t.parry_window_ticks, PARRY_WINDOW_TICKS);
        assert_eq!(t.parry_stagger_ticks, PARRY_STAGGER_TICKS);
        assert_eq!(t.hit_react_ticks, HIT_REACT_TICKS);
        assert_eq!(t.hitstop_ticks, HITSTOP_TICKS);
        assert_eq!(t.knockdown_ticks, KNOCKDOWN_TICKS);
        assert_eq!(TABLES_V1, Tables::v1());
        assert_eq!(
            MoveTableSet::v1(),
            MoveTableSet {
                version: TABLES_VERSION_V1,
                tables: TABLES_V1
            }
        );
        assert_eq!(TABLES_VERSION_V1, 1);
    }

    #[test]
    fn replacement_tables_validate_and_change_outcomes() {
        let mut rows = strict_rows();
        rows.attack_damage = 240;
        let set = MoveTableSet::new(2, rows);
        assert_eq!(validate_tables(&set), Ok(()));
        assert_eq!(set.validate(), Ok(()));

        // Scripted bout: fighter 0 attacks once, fighter 1 idles; the drain
        // covers impact plus hit-stop under either table.
        let bout = |tables: &Tables| {
            let mut state = FightState::new(2).unwrap();
            let mut all = Vec::new();
            all.push(step_with_tables(
                &[input(0, ATTACK), none(1)],
                &mut state,
                tables,
            ));
            for _ in 0..12 {
                all.push(step_with_tables(&[none(0), none(1)], &mut state, tables));
            }
            (all, state)
        };
        let hit_damage = |all: &[Vec<FightEvent>]| {
            all.iter().flatten().find_map(|e| match *e {
                FightEvent::Hit { damage, .. } => Some(damage),
                _ => None,
            })
        };
        let (v1_events, v1_state) = bout(&TABLES_V1);
        let (v2_events, v2_state) = bout(&rows);
        assert_eq!(
            v1_state.fighters[1].hp_permille,
            MAX_HP_PERMILLE - ATTACK_DAMAGE
        );
        assert_eq!(v2_state.fighters[1].hp_permille, MAX_HP_PERMILLE - 240);
        assert_ne!(v1_state, v2_state);
        assert_eq!(hit_damage(&v1_events), Some(ATTACK_DAMAGE));
        assert_eq!(hit_damage(&v2_events), Some(240));

        // The same rows through the swap input step identically.
        let swap = SwapInput::new(2, rows).unwrap();
        let (swap_events, swap_state) = bout(&swap.tables());
        assert_eq!((swap_events, swap_state), (v2_events, v2_state));
    }

    #[test]
    fn invalid_tables_rejected_per_rule() {
        assert_eq!(
            validate_tables(&MoveTableSet::new(2, strict_rows())),
            Ok(())
        );
        // Version 0 is reserved for legacy untagged replays.
        assert_eq!(bad(0, |_| {}), Err(CombatError::BadTableVersion(0)));
        // No zero-damage attacks; never more than full health.
        assert_eq!(
            bad(2, |t| t.attack_damage = 0),
            Err(CombatError::BadTableValue {
                field: "attack_damage",
                value: 0
            })
        );
        assert!(matches!(
            bad(2, |t| t.attack_damage = MAX_HP_PERMILLE + 1),
            Err(CombatError::BadTableValue {
                field: "attack_damage",
                ..
            })
        ));
        // Startup/recovery ranges.
        assert!(matches!(
            bad(2, |t| t.attack_startup_ticks = 0),
            Err(CombatError::BadTableValue {
                field: "attack_startup_ticks",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.attack_startup_ticks = MAX_STARTUP_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "attack_startup_ticks",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.attack_recovery_ticks = MAX_RECOVERY_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "attack_recovery_ticks",
                ..
            })
        ));
        // Cost ranges.
        assert!(matches!(
            bad(2, |t| t.attack_stamina_cost = MAX_STAMINA + 1),
            Err(CombatError::BadTableValue {
                field: "attack_stamina_cost",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.dodge_stamina_cost = MAX_STAMINA + 1),
            Err(CombatError::BadTableValue {
                field: "dodge_stamina_cost",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.block_hold_cost_per_tick = MAX_STAMINA + 1),
            Err(CombatError::BadTableValue {
                field: "block_hold_cost_per_tick",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.stamina_regen_per_tick = MAX_STAMINA + 1),
            Err(CombatError::BadTableValue {
                field: "stamina_regen_per_tick",
                ..
            })
        ));
        // Dodge duration and i-frames inside it.
        assert!(matches!(
            bad(2, |t| t.dodge_duration_ticks = 0),
            Err(CombatError::BadTableValue {
                field: "dodge_duration_ticks",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.dodge_iframes_ticks = t.dodge_duration_ticks + 1),
            Err(CombatError::BadTableValue {
                field: "dodge_iframes_ticks",
                ..
            })
        ));
        // Chip denominator never zero; chip never exceeds damage.
        assert!(matches!(
            bad(2, |t| t.block_chip_den = 0),
            Err(CombatError::BadTableValue {
                field: "block_chip_den",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.block_chip_num = t.block_chip_den + 1),
            Err(CombatError::BadTableValue {
                field: "block_chip_num",
                ..
            })
        ));
        // Parry window strictly below startup: equal and above both fail.
        assert_eq!(
            bad(2, |t| t.parry_window_ticks = t.attack_startup_ticks),
            Err(CombatError::BadTableValue {
                field: "parry_window_ticks",
                value: ATTACK_STARTUP_TICKS,
            })
        );
        assert!(matches!(
            bad(2, |t| t.parry_window_ticks = MAX_PARRY_WINDOW_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "parry_window_ticks",
                ..
            })
        ));
        // Stagger, react, hit-stop and knockdown ranges.
        assert!(matches!(
            bad(2, |t| t.parry_stagger_ticks = MAX_STAGGER_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "parry_stagger_ticks",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.hit_react_ticks = MAX_STAGGER_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "hit_react_ticks",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.hitstop_ticks = MAX_HITSTOP_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "hitstop_ticks",
                ..
            })
        ));
        assert!(matches!(
            bad(2, |t| t.knockdown_ticks = MAX_KNOCKDOWN_TICKS + 1),
            Err(CombatError::BadTableValue {
                field: "knockdown_ticks",
                ..
            })
        ));
    }

    #[test]
    fn old_version_replay_uses_v1() {
        // Scripted bout across attack, block, dodge and stamina paths.
        let script: Vec<[FighterInput; 2]> = vec![
            [input(0, ATTACK), none(1)],
            [none(0), input(1, BLOCK)],
            [input(0, ATTACK), input(1, BLOCK)],
            [none(0), input(1, BLOCK)],
            [input(0, ATTACK), input(1, ATTACK)],
            [none(0), none(1)],
        ];
        let run_plain = || {
            let mut state = FightState::new(2).unwrap();
            let mut all = Vec::new();
            for tick_inputs in &script {
                all.push(step(&tick_inputs[..], &mut state));
            }
            for _ in 0..HITSTOP_TICKS {
                all.push(step(&[none(0), none(1)], &mut state));
            }
            (all, state)
        };
        let run_tagged = |version: u32| {
            let mut state = FightState::new(2).unwrap();
            let mut all = Vec::new();
            for tick_inputs in &script {
                all.push(step_for_version(&tick_inputs[..], &mut state, version).unwrap());
            }
            for _ in 0..HITSTOP_TICKS {
                all.push(step_for_version(&[none(0), none(1)], &mut state, version).unwrap());
            }
            (all, state)
        };
        let plain = run_plain();
        // Version 1 and legacy untagged 0 replay through the same V1 rows.
        assert_eq!(run_tagged(1), plain);
        assert_eq!(run_tagged(0), plain);
        assert_eq!(tables_for_version(0), Some(&TABLES_V1));
        assert_eq!(tables_for_version(1), Some(&TABLES_V1));
        assert_eq!(tables_for_version(2), Some(&TABLES_V2));
        // The strict path agrees on the tagged version too.
        let mut a = FightState::new(2).unwrap();
        let mut b = FightState::new(2).unwrap();
        for tick_inputs in &script {
            let left = crate::fight::try_step(&tick_inputs[..], &mut a).unwrap();
            let right = try_step_for_version(&tick_inputs[..], &mut b, 1).unwrap();
            assert_eq!(left, right);
        }
        assert_eq!(a, b);
        // Unknown versions refuse on both paths without stepping.
        let mut state = FightState::new(2).unwrap();
        let before = state;
        assert_eq!(
            step_for_version(&[none(0), none(1)], &mut state, 7),
            Err(CombatError::UnknownTableVersion(7))
        );
        assert_eq!(
            try_step_for_version(&[none(0), none(1)], &mut state, 7),
            Err(CombatError::UnknownTableVersion(7))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn swap_input_is_immutable_snapshot() {
        let mut rows = strict_rows();
        rows.attack_damage = 240;
        let original = SwapInput::new(2, rows).unwrap();
        let cloned = original.clone();
        // A new swap built from the clone's rows leaves the original intact.
        let mut tweaked = cloned.tables();
        tweaked.attack_damage = 300;
        let modified = SwapInput::new(3, tweaked).unwrap();
        assert_eq!(original.version(), 2);
        assert_eq!(original.tables().attack_damage, 240);
        assert_eq!(original.tables(), rows);
        assert_eq!(modified.version(), 3);
        assert_eq!(modified.tables().attack_damage, 300);
        assert_eq!(original.table_set(), MoveTableSet::new(2, rows));
        assert_eq!(cloned, original);
        // Invalid rows never become a swap.
        let mut bad_rows = rows;
        bad_rows.attack_damage = 0;
        assert_eq!(
            SwapInput::new(2, bad_rows),
            Err(CombatError::BadTableValue {
                field: "attack_damage",
                value: 0
            })
        );
        assert_eq!(
            SwapInput::new(0, rows),
            Err(CombatError::BadTableVersion(0))
        );
    }

    #[test]
    fn ttk_hits_count_ceil_division() {
        assert_eq!(hits_to_kill(200, 1000), 5);
        // Ceiling, not floor: a remainder needs one more hit.
        assert_eq!(hits_to_kill(300, 1000), 4);
        assert_eq!(hits_to_kill(1000, 1000), 1);
        // Zero damage never kills; zero hp is already down.
        assert_eq!(hits_to_kill(0, 1000), u32::MAX);
        assert_eq!(hits_to_kill(200, 0), 0);
    }

    #[test]
    fn ttk_ticks_bounds_one_cycle_per_hit() {
        assert_eq!(ttk_ticks(5, 4, 10, 3), 85);
        assert_eq!(ttk_ticks(0, 4, 10, 3), 0);
        // Hostile rows saturate instead of wrapping.
        assert_eq!(ttk_ticks(u32::MAX, u32::MAX, u32::MAX, u32::MAX), u64::MAX);
    }

    #[test]
    fn ttk_helpers_use_total_conversions() {
        // Ratchet pin: the TTK helpers widen with total `From`
        // conversions, never a truncating-capable `as` cast, so a
        // future narrowing edit fails here instead of silently
        // shifting kill math. Precedent: the forward-shader content
        // asserts in `sun_shadow_slope_and_snap_are_live`.
        let src = include_str!("tables.rs");
        for marker in ["fn hits_to_kill", "fn ttk_ticks("] {
            let start = src
                .find(marker)
                .unwrap_or_else(|| panic!("TTK helper present: {marker}"));
            let rest = &src[start..];
            let end = rest.find("\n}\n").expect("helper body ends");
            let body = &rest[..end];
            for cast in [" as u8", " as u16", " as u32", " as u64", " as usize"] {
                assert!(
                    !body.contains(cast),
                    "TTK helper `{marker}` must use `From`, found `{cast}`"
                );
            }
        }
    }

    #[test]
    fn ttk_for_tables_uses_row() {
        let ticks = ttk_ticks_for_tables(TABLES_V1, 1000);
        let expect = ttk_ticks(
            hits_to_kill(TABLES_V1.attack_damage, 1000),
            TABLES_V1.attack_startup_ticks,
            TABLES_V1.attack_recovery_ticks,
            TABLES_V1.hitstop_ticks,
        );
        assert_eq!(ticks, expect);
        assert!(ticks > 0);
    }
}
