//! fight: data-table fight rules as a pure deterministic step.
//!
//! Consumer: `arena` owns match state; this module owns combat state and
//! events. Units: hp in permille, stamina in points, timing in fixed ticks
//! (60 Hz). Determinism: integer math and fighter-ID order only. Errors:
//! [`CombatError`]; [`step`] skips invalid inputs, [`try_step`] rejects them.
//! Rules as data: [`Tables`] owns every tunable row and [`TABLES_V1`] is the
//! frozen shipped set that old tapes replay through.

use engine_core::checked::saturating_u32_to_u16;
use engine_input::fighter::{ATTACK, BLOCK, DODGE};
use engine_input::{Buttons, FighterError, FighterInput, MAX_PARTICIPANTS};

use crate::feel::{StreamBuffer, StreamliningParams};
use crate::sweep::{
    sweep_one, validate_fighter, validate_sweep, SweepOutcome, SweepParticipant, WeaponSweep,
    SWEEP_V1,
};

/// Length of every per-slot array: the canonical participant cap as `usize`.
pub(crate) const SLOT_COUNT: usize = MAX_PARTICIPANTS as usize;

// ---- data tables (consts) -------------------------------------------------
// The step core reads only `Tables`; these consts build TABLES_V1 so shipped
// tapes replay. New rule sets travel as versioned `Tables`, never as source
// edits. Structural bounds stay consts: they bound the state encoding.

/// Full health in permille.
pub const MAX_HP_PERMILLE: u16 = 1000;
/// Full stamina in points.
pub const MAX_STAMINA: u16 = 100;
/// Damage of one landed attack in permille.
pub const ATTACK_DAMAGE: u16 = 120;
/// Ticks from attack press to impact.
pub const ATTACK_STARTUP_TICKS: u32 = 4;
/// Ticks the attacker is locked after impact.
pub const ATTACK_RECOVERY_TICKS: u32 = 8;
/// Stamina cost to start an attack.
pub const ATTACK_STAMINA_COST: u16 = 25;
/// Stamina cost to start a dodge.
pub const DODGE_STAMINA_COST: u16 = 20;
/// Stamina drained per tick while block is held.
pub const BLOCK_HOLD_COST_PER_TICK: u16 = 1;
/// Stamina regained per tick by every fighter.
pub const STAMINA_REGEN_PER_TICK: u16 = 2;
/// Ticks a dodge lasts in total.
pub const DODGE_DURATION_TICKS: u32 = 10;
/// Leading ticks of a dodge that negate hits.
pub const DODGE_IFRAMES_TICKS: u32 = 6;
/// Block chip numerator: chip = damage * NUM / DEN.
pub const BLOCK_CHIP_NUM: u32 = 1;
/// Block chip denominator: chip = damage * NUM / DEN.
pub const BLOCK_CHIP_DEN: u32 = 5;
/// Ticks after a block press during which an incoming hit is parried.
pub const PARRY_WINDOW_TICKS: u32 = 4;
/// Ticks the attacker is staggered after being parried.
pub const PARRY_STAGGER_TICKS: u32 = 12;
/// Ticks the victim cannot act after a landed hit.
pub const HIT_REACT_TICKS: u32 = 6;
/// Ticks the whole fight freezes after a landed hit or parry.
pub const HITSTOP_TICKS: u32 = 4;
/// Ticks a knocked-down fighter stays down.
pub const KNOCKDOWN_TICKS: u32 = 30;
/// Consecutive clean hits a stunned victim can take before the next one
/// knocks it down (AIN6 juggle budget, Ikemen GO's 15). The first hit sets
/// the counter to one; hit 16 converts to a knockdown and resets it.
pub const JUGGLE_BUDGET: u8 = 15;
/// Hard bound on inputs accepted for one tick.
pub const MAX_INPUTS_PER_TICK: usize = 32;
/// Hard bound on events emitted for one tick: one `StaminaOut` per fighter
/// plus, per attacker whose startup completes, one outcome per other live
/// victim with a possible `Knockdown` beside it. A buffer reserved once to
/// this capacity never reallocates on the tick path.
pub const MAX_EVENTS_PER_TICK: usize = 2_048;

/// Chip damage dealt through a held block under the frozen V1 row above.
/// New rows use [`Tables::chip`]; the step core never calls this.
pub const fn block_chip(damage: u16) -> u16 {
    saturating_u32_to_u16((damage as u32 * BLOCK_CHIP_NUM) / BLOCK_CHIP_DEN)
}

/// Tunable fight-rules row: every number the step core reads, in one place.
/// Consumer: live tuning hands a replacement [`Tables`] to the arena as an
/// immutable swap input. Units match the consts above: damage in permille,
/// costs in stamina points, timing in fixed ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tables {
    /// Damage of one landed attack in permille.
    pub attack_damage: u16,
    /// Ticks from attack press to impact.
    pub attack_startup_ticks: u32,
    /// Ticks the attacker is locked after impact.
    pub attack_recovery_ticks: u32,
    /// Stamina cost to start an attack.
    pub attack_stamina_cost: u16,
    /// Stamina cost to start a dodge.
    pub dodge_stamina_cost: u16,
    /// Stamina drained per tick while block is held.
    pub block_hold_cost_per_tick: u16,
    /// Stamina regained per tick by every fighter.
    pub stamina_regen_per_tick: u16,
    /// Ticks a dodge lasts in total.
    pub dodge_duration_ticks: u32,
    /// Leading ticks of a dodge that negate hits.
    pub dodge_iframes_ticks: u32,
    /// Block chip numerator: chip = damage * num / den.
    pub block_chip_num: u32,
    /// Block chip denominator: chip = damage * num / den.
    pub block_chip_den: u32,
    /// Ticks after a block press during which an incoming hit is parried.
    pub parry_window_ticks: u32,
    /// Ticks the attacker is staggered after being parried.
    pub parry_stagger_ticks: u32,
    /// Ticks the victim cannot act after a landed hit.
    pub hit_react_ticks: u32,
    /// Ticks the whole fight freezes after a landed hit or parry.
    pub hitstop_ticks: u32,
    /// Ticks a knocked-down fighter stays down.
    pub knockdown_ticks: u32,
}

impl Tables {
    /// The shipped base rules: every field reads its module const above.
    pub const fn v1() -> Self {
        Self {
            attack_damage: ATTACK_DAMAGE,
            attack_startup_ticks: ATTACK_STARTUP_TICKS,
            attack_recovery_ticks: ATTACK_RECOVERY_TICKS,
            attack_stamina_cost: ATTACK_STAMINA_COST,
            dodge_stamina_cost: DODGE_STAMINA_COST,
            block_hold_cost_per_tick: BLOCK_HOLD_COST_PER_TICK,
            stamina_regen_per_tick: STAMINA_REGEN_PER_TICK,
            dodge_duration_ticks: DODGE_DURATION_TICKS,
            dodge_iframes_ticks: DODGE_IFRAMES_TICKS,
            block_chip_num: BLOCK_CHIP_NUM,
            block_chip_den: BLOCK_CHIP_DEN,
            parry_window_ticks: PARRY_WINDOW_TICKS,
            parry_stagger_ticks: PARRY_STAGGER_TICKS,
            hit_react_ticks: HIT_REACT_TICKS,
            hitstop_ticks: HITSTOP_TICKS,
            knockdown_ticks: KNOCKDOWN_TICKS,
        }
    }

    /// Chip damage dealt through a held block under this table row.
    /// Saturating and panic-free, so unvalidated rows still step safely.
    pub const fn chip(self, damage: u16) -> u16 {
        match (damage as u32).checked_mul(self.block_chip_num) {
            None => u16::MAX,
            Some(product) => match product.checked_div(self.block_chip_den) {
                None => 0,
                Some(chip) => {
                    if chip > u16::MAX as u32 {
                        u16::MAX
                    } else {
                        saturating_u32_to_u16(chip)
                    }
                }
            },
        }
    }
}

/// The shipped base rules as one row. Old tapes replay through this table;
/// replacement sets travel versioned and never mutate this row.
pub const TABLES_V1: Tables = Tables::v1();

/// The readable-timing row (version 2): startup 15 ticks, parry window 7.
/// The literals live once in [`v2_rows`](crate::tables::v2_rows); the arena
/// steps this row through [`try_step_v2_into`].
pub const TABLES_V2: Tables = crate::tables::v2_rows();

/// [`Tables`] for one Ludus Magnus class over [`TABLES_V2`]. Production
/// caller for [`classes`](crate::classes): the arena adopts it at match build.
pub fn class_tables(class: crate::classes::Class) -> Tables {
    crate::classes::tables(class, TABLES_V2)
}

/// Poise damage for one strike kind under explicit rows, in poise points.
/// Caller: `DepthSet::apply_poise_kind` (test builds only); the fight step
/// never writes poise, so V1 replays stay byte-identical. V1 rows and
/// unknown kinds give 0, never panic.
pub fn poise_damage_for_kind(tables: &Tables, kind: u8) -> u16 {
    use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
    if *tables != TABLES_V2 {
        return 0;
    }
    if kind == INPUT_LIGHT {
        crate::tables::LIGHT_POISE_DAMAGE_V2
    } else if kind == INPUT_HEAVY {
        crate::tables::HEAVY_POISE_DAMAGE_V2
    } else {
        0
    }
}

/// Stamina cost for one strike kind under explicit rows, in stamina points.
/// Caller: `DepthSet::spend_stamina_kind` (test builds only); the fight
/// intake keeps its single [`ATTACK_STAMINA_COST`] row, so V1 bouts never
/// fork. Unknown kinds give 0 under V2 and the attack cost under V1.
pub fn stamina_cost_for_kind(tables: &Tables, kind: u8) -> u16 {
    use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
    if *tables != TABLES_V2 {
        return tables.attack_stamina_cost;
    }
    if kind == INPUT_LIGHT {
        crate::tables::LIGHT_STAMINA_COST_V2
    } else if kind == INPUT_HEAVY {
        crate::tables::HEAVY_STAMINA_COST_V2
    } else {
        0
    }
}

/// Quiet ticks before poise regens under explicit rows, at 60 Hz. Caller:
/// [`DepthSet::new`](crate::depth::DepthSet::new). V2 gives 30, other rows 90.
pub fn poise_regen_delay_for_tables(tables: &Tables) -> u32 {
    if *tables == TABLES_V2 {
        crate::tables::POISE_REGEN_DELAY_TICKS_V2
    } else {
        crate::depth::DEFAULT_POISE_DELAY_TICKS
    }
}

/// Freeze ticks for one impacting strike under explicit rows, at 60 Hz.
/// Caller: [`resolve_strike`], so the two freeze sites never disagree. V2
/// delegates to the single home [`crate::hitstop_kind::for_impact`]; every
/// other row gives the single row, so V1 replays stay byte-identical.
#[must_use]
pub fn hitstop_for_impact(tables: &Tables, kind: u8, parried: bool) -> u32 {
    if tables.attack_startup_ticks != TABLES_V2.attack_startup_ticks
        || tables.parry_window_ticks != TABLES_V2.parry_window_ticks
    {
        return tables.hitstop_ticks;
    }
    crate::hitstop_kind::for_impact(kind, parried)
}

// ---- state ----------------------------------------------------------------

/// Per-fighter combat state. Plain integers/bools only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FighterState {
    /// Health in permille, `0..=1000`. Zero means out of the fight.
    pub hp_permille: u16,
    /// Stamina points, `0..=MAX_STAMINA`.
    pub stamina: u16,
    /// Ticks since the current attack started; meaningful when attacking.
    pub attack_age: u32,
    /// Whether an attack is in flight (startup through recovery).
    pub is_attacking: bool,
    /// Strike kind of the in-flight attack (`INPUT_LIGHT` or `INPUT_HEAVY` in
    /// `crate::depth`); the hit branch reads it for per-kind hit-stop. Under
    /// the shield-line rules a weapon holder makes the heavy chord the boss
    /// push, its own kind.
    pub attack_kind: u8,
    /// Queued attack presses awaiting a cancel window or the first ready
    /// tick. V2 rows queue taps while a swing is locked; V1 rows never queue
    /// one, so V1 bouts replay byte-identical.
    pub attack_buffer: StreamBuffer,
    /// Ticks since the current block press; `u32::MAX` when not blocking.
    pub block_age: u32,
    /// Whether block is currently held.
    pub is_blocking: bool,
    /// Ticks since the current dodge started; meaningful when dodging.
    pub dodge_age: u32,
    /// Whether a dodge is in flight.
    pub is_dodging: bool,
    /// Ticks remaining where the fighter cannot start actions (hit/parry stagger).
    pub react: u32,
    /// Ticks remaining knocked down; zero means up.
    pub knockdown: u32,
    /// Consecutive clean hits taken while already in hitstun (the AIN6 juggle
    /// budget). Reset when hitstun ends, a block lands or the budget converts.
    pub juggle: u8,
    /// Weapon row under the shield-line rules: 0 bare hands, 1 pugio,
    /// 2 gladius, 3 spatha. Zero keeps the legacy numbers.
    pub weapon: u8,
    /// The weight of the steel this fighter swings (FEEL1, [`crate::heft`]):
    /// its own startup, recovery, bare-row damage and the stagger it deals.
    /// `None` swings the table row, so a match that never arms one steps
    /// exactly as before.
    pub heft: Option<crate::heft::Heft>,
    /// The balance impulse the last push fed this fighter, in Q8 units. The
    /// arena hands it to its balance state. The channel carries one tick:
    /// every tick starts clear, so a push feeds it once.
    pub balance_impulse_q8: u16,
}

impl Default for FighterState {
    fn default() -> Self {
        Self {
            hp_permille: MAX_HP_PERMILLE,
            stamina: MAX_STAMINA,
            attack_age: 0,
            is_attacking: false,
            attack_kind: crate::depth::INPUT_LIGHT,
            attack_buffer: StreamBuffer::new(),
            block_age: u32::MAX,
            is_blocking: false,
            dodge_age: 0,
            is_dodging: false,
            react: 0,
            knockdown: 0,
            juggle: 0,
            weapon: 0,
            heft: None,
            balance_impulse_q8: 0,
        }
    }
}

impl FighterState {
    /// True while the fighter can start attacks/blocks/dodges.
    pub const fn can_act(self) -> bool {
        self.hp_permille > 0 && self.react == 0 && self.knockdown == 0
    }

    /// True during dodge invulnerability frames under explicit table rows.
    pub const fn invulnerable_with(self, tables: &Tables) -> bool {
        self.is_dodging && self.dodge_age < tables.dodge_iframes_ticks
    }

    /// True while a held block still parries under the shipped V1 row
    /// (pressed within the window).
    pub const fn parry_open(self) -> bool {
        self.parry_open_with(&TABLES_V1)
    }

    /// True while a held block still parries under explicit table rows.
    pub const fn parry_open_with(self, tables: &Tables) -> bool {
        self.is_blocking && self.block_age <= tables.parry_window_ticks
    }
}

/// Whole-fight state for up to [`MAX_PARTICIPANTS`] fighters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FightState {
    /// One slot per fighter ID; only `0..fighter_count` take part.
    pub fighters: [FighterState; SLOT_COUNT],
    /// Active fighter count, always `1..=MAX_PARTICIPANTS`.
    pub fighter_count: u8,
    /// Event ticks elapsed (frozen while hit-stop is held).
    pub tick: u32,
    /// Remaining frozen ticks; inputs are ignored while nonzero.
    pub hitstop: u32,
}

impl FightState {
    /// Fresh fight with `count` active fighters at full hp/stamina.
    pub fn new(count: u8) -> Result<Self, CombatError> {
        if count == 0 || count > MAX_PARTICIPANTS {
            return Err(CombatError::BadFighterCount(count));
        }
        Ok(Self {
            fighters: [FighterState::default(); SLOT_COUNT],
            fighter_count: count,
            tick: 0,
            hitstop: 0,
        })
    }

    /// Checked read of one fighter slot; `None` when out of the fight.
    pub const fn fighter(self, id: u8) -> Option<FighterState> {
        if id < self.fighter_count {
            Some(self.fighters[id as usize])
        } else {
            None
        }
    }
}

// ---- events and errors ----------------------------------------------------

/// Deterministic combat outcomes for one tick, in fighter-ID order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FightEvent {
    /// Full damage landed.
    Hit {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
        /// Damage in permille.
        damage: u16,
    },
    /// Held block soaked the hit down to chip damage.
    Blocked {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
        /// Chip damage in permille.
        chip: u16,
    },
    /// Block inside the parry window: no damage, attacker staggered.
    Parried {
        /// Attacker slot (staggered).
        attacker: u8,
        /// Victim slot (parried).
        victim: u8,
    },
    /// Dodge i-frames negated the hit.
    Dodged {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot (dodged).
        victim: u8,
    },
    /// Attack/dodge pressed without enough stamina; nothing started.
    StaminaOut {
        /// Fighter slot.
        fighter: u8,
    },
    /// Victim dropped to zero hp.
    Knockdown {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot (now at zero hp).
        victim: u8,
    },
}

/// Hard bound on press-tick attack cues per tick: at most one per fighter.
pub const MAX_ATTACK_STARTED_PER_TICK: usize = SLOT_COUNT;

/// Press-tick attack cue under readable-timing rows (EG19b slice 1).
/// Consumer: the game maps each cue into an audio `FightCue` once the audio
/// side lands; this crate only reports presses. Cues leave in fighter-ID
/// order on the press tick under [`TABLES_V2`] rows and stay empty under
/// [`TABLES_V1`], so old tapes replay. Never hashed: presentation only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AttackStarted {
    /// Fighter slot that began an attack this tick.
    pub attacker: u8,
}

/// Bounded combat failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CombatError {
    /// Fighter count outside `1..=MAX_PARTICIPANTS`, culprit carried.
    BadFighterCount(u8),
    /// Too many inputs for one tick, length carried.
    TooManyInputs(usize),
    /// One input failed [`FighterInput::validate`], index and source carried.
    BadInput {
        /// Index into the input slice.
        index: usize,
        /// Underlying fighter-contract failure.
        source: FighterError,
    },
    /// Input names a fighter outside the active `0..fighter_count` range.
    InactiveFighter {
        /// Index into the input slice.
        index: usize,
        /// Culprit slot.
        fighter: u8,
    },
    /// Same fighter twice in one tick, index and slot carried.
    DuplicateInput {
        /// Index of the repeat.
        index: usize,
        /// Culprit slot.
        fighter: u8,
    },
    /// Replacement table set carries version 0, which is reserved for
    /// legacy untagged replays and never valid on a new set.
    BadTableVersion(u32),
    /// One tunable row is outside its sane range; field name and culprit
    /// value carried. First failure wins, deterministically.
    BadTableValue {
        /// Table field name, e.g. `"attack_damage"`.
        field: &'static str,
        /// Culprit value widened to `u32`.
        value: u32,
    },
    /// Replay or swap references a table version with no known rows.
    UnknownTableVersion(u32),
    /// Rules-v2 strike geometry is missing, out of order or out of range for
    /// this slot (wrong pose count, pose or blade naming another slot, or a
    /// sweep bound broken); culprit slot carried.
    BadStrikePose(u8),
}

// ---- validation -----------------------------------------------------------

/// Range-check the tick's inputs against the active fighter count. Checks
/// slice bound, per-input validity, active range and one input per fighter;
/// first failure wins, deterministically.
pub fn validate_inputs(inputs: &[FighterInput], fighter_count: u8) -> Result<(), CombatError> {
    if fighter_count == 0 || fighter_count > MAX_PARTICIPANTS {
        return Err(CombatError::BadFighterCount(fighter_count));
    }
    if inputs.len() > MAX_INPUTS_PER_TICK {
        return Err(CombatError::TooManyInputs(inputs.len()));
    }
    let mut seen = [false; SLOT_COUNT];
    for (index, input) in inputs.iter().enumerate() {
        if let Err(source) = input.validate() {
            return Err(CombatError::BadInput { index, source });
        }
        let slot = input.fighter.0;
        if slot >= fighter_count {
            return Err(CombatError::InactiveFighter {
                index,
                fighter: slot,
            });
        }
        // Slot is < fighter_count <= MAX_PARTICIPANTS, so indexing is in bounds.
        let flag = &mut seen[usize::from(slot)];
        if *flag {
            return Err(CombatError::DuplicateInput {
                index,
                fighter: slot,
            });
        }
        *flag = true;
    }
    Ok(())
}

// ---- rules v2: hit geometry -------------------------------------------------

/// Fight rules version 1: the frozen hit rule. An impact resolves against
/// every living foe at any range; old tapes replay through it unchanged.
pub const FIGHT_RULES_V1: u32 = 1;
/// Fight rules version 2: an impact needs geometry. The attacker's blade path
/// must touch the victim's hurt capsule in front of the attacker, teammates
/// are never struck, and a block or parry counts only inside the shield cone.
pub const FIGHT_RULES_V2: u32 = 2;

/// One slot's strike geometry for a rules-v2 step: integer millimetres and
/// scale-free integer facing only (no floats, no trig). Consumer: the arena
/// builds one per active slot each tick; combat reads it inside impact
/// resolution.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StrikePose {
    /// Hurt capsule, facing and shield cone; `body.id` is the slot.
    /// `shield_up` is ignored: the fight state's held block decides.
    pub body: SweepParticipant,
    /// Blade path this slot strikes along if its impact lands this tick;
    /// `blade.attacker` is the slot.
    pub blade: WeaponSweep,
    /// Team id: equal ids never strike each other.
    pub team: u8,
}

/// Range-check rules-v2 geometry: one pose per active slot in slot order,
/// each pose and blade naming its own slot and inside the sweep bounds.
pub fn validate_strike_poses(poses: &[StrikePose], fighter_count: u8) -> Result<(), CombatError> {
    if usize::from(fighter_count) != poses.len() {
        return Err(CombatError::BadStrikePose(fighter_count));
    }
    for (slot, pose) in poses.iter().enumerate() {
        let id = u8::try_from(slot).map_err(|_| CombatError::BadStrikePose(u8::MAX))?;
        if pose.body.id != id
            || pose.blade.attacker != id
            || validate_fighter(&pose.body, fighter_count).is_err()
            || validate_sweep(&pose.blade, fighter_count, &SWEEP_V1).is_err()
        {
            return Err(CombatError::BadStrikePose(id));
        }
    }
    Ok(())
}

/// How one impacting attacker meets one victim under rules v2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StrikeGate {
    /// Teammate, out of reach or behind the attacker: no outcome at all.
    Miss,
    /// Blade contact; the victim's guard faces away from the attacker.
    Open,
    /// Blade contact inside the victim's shield cone: block and parry count.
    Covered,
}

/// The v2 gate for one pair: team first, then the reach, then the sweep
/// capsule test with the victim's shield cone raised.
fn strike_gate(
    poses: &[StrikePose],
    attacker: usize,
    victim: usize,
    reach: Option<u32>,
) -> StrikeGate {
    let (Some(a), Some(v)) = (poses.get(attacker), poses.get(victim)) else {
        return StrikeGate::Miss;
    };
    if a.team == v.team {
        return StrikeGate::Miss;
    }
    if !within_reach(&a.body, &v.body, reach) {
        return StrikeGate::Miss;
    }
    let guard = SweepParticipant {
        shield_up: true,
        ..v.body
    };
    match sweep_one(&a.body, &guard, &a.blade, &SWEEP_V1) {
        SweepOutcome::Hit { .. } => StrikeGate::Open,
        SweepOutcome::Blocked { .. } => StrikeGate::Covered,
        SweepOutcome::Miss(_) => StrikeGate::Miss,
    }
}

/// Shared rules-v2 intake: validate inputs then geometry and fold the slice
/// into per-slot button intents. The caller clears its buffers first.
fn v2_intents(
    inputs: &[FighterInput],
    poses: &[StrikePose],
    fighter_count: u8,
) -> Result<[Buttons; SLOT_COUNT], CombatError> {
    validate_inputs(inputs, fighter_count)?;
    validate_strike_poses(poses, fighter_count)?;
    let mut intents = [Buttons::NONE; SLOT_COUNT];
    for input in inputs.iter() {
        // Validated above: slot is an active fighter, unique per tick.
        intents[usize::from(input.fighter.0)] = input.buttons;
    }
    Ok(intents)
}

/// Strict rules-v2 step into a reused event buffer: [`try_step_with_tables_into`]
/// plus hit geometry. Impacts resolve only against foes the blade reaches in
/// front of the attacker, and a block or parry from behind the shield cone
/// fails. A rejected tick leaves `out` empty and `state` untouched. A
/// reserved buffer makes the tick allocate nothing.
pub fn try_step_v2_into(
    inputs: &[FighterInput],
    poses: &[StrikePose],
    state: &mut FightState,
    tables: &Tables,
    out: &mut Vec<FightEvent>,
) -> Result<(), CombatError> {
    out.clear();
    let intents = v2_intents(inputs, poses, state.fighter_count)?;
    core_step_into(&intents, state, tables, Some(poses), out, None);
    Ok(())
}

/// Strict rules-v2 step with press-tick attack cues (EG19b slice 1).
/// Caller: `try_step_v2_into` shares this core; the arena adopts this entry
/// once the game maps [`AttackStarted`] into an audio cue. Same validation
/// and [`FightEvent`] stream as [`try_step_v2_into`]; `cues` holds one
/// [`AttackStarted`] per fresh attack start in fighter-ID order under
/// [`TABLES_V2`] rows and stays empty otherwise.
pub fn try_step_v2_with_cues_into(
    inputs: &[FighterInput],
    poses: &[StrikePose],
    state: &mut FightState,
    tables: &Tables,
    out: &mut Vec<FightEvent>,
    cues: &mut Vec<AttackStarted>,
) -> Result<(), CombatError> {
    out.clear();
    cues.clear();
    let intents = v2_intents(inputs, poses, state.fighter_count)?;
    core_step_into(&intents, state, tables, Some(poses), out, Some(cues));
    Ok(())
}

// ---- rules v2: the shield line ---------------------------------------------

mod shield_line;
use shield_line::{
    strike_damage, strike_reach, within_reach, KIND_PUSH, PUSH_HITSTOP_TICKS, PUSH_IMPULSE_Q8,
    PUSH_RECOVERY_TICKS, PUSH_STAGGER_TICKS, PUSH_STARTUP_TICKS, ROW_BARE,
};

/// The startup and the recovery of one strike of `f`, in ticks. The push is
/// a v2 action: a short shove. Every other kind swings the fighter's armed
/// weight ([`crate::heft`]) or the table's row, so v1 rows keep their own
/// numbers.
fn kind_timing(tables: &Tables, f: &FighterState) -> (u32, u32) {
    if f.attack_kind == KIND_PUSH && *tables == TABLES_V2 {
        (PUSH_STARTUP_TICKS, PUSH_RECOVERY_TICKS)
    } else {
        (
            crate::heft::startup(f.heft, tables),
            crate::heft::recovery(f.heft, tables),
        )
    }
}

// ---- step -----------------------------------------------------------------

/// Lenient pure step under the shipped V1 row: same inputs twice give same
/// events. Invalid inputs are skipped deterministically; valid ones resolve
/// in fighter-ID order. Never panics on user data. Budget: O(events) time
/// plus one heap allocation per event tick; hot callers use [`step_into`].
pub fn step(inputs: &[FighterInput], state: &mut FightState) -> Vec<FightEvent> {
    step_with_tables(inputs, state, &TABLES_V1)
}

/// Lenient pure step into a reused event buffer under the shipped V1 row.
/// Same events as [`step`]; `out` is cleared first, so a buffer reserved once
/// to [`MAX_EVENTS_PER_TICK`] never reallocates and the tick allocates nothing.
pub fn step_into(inputs: &[FighterInput], state: &mut FightState, out: &mut Vec<FightEvent>) {
    step_with_tables_into(inputs, state, &TABLES_V1, out);
}

/// Strict step under the shipped V1 row: same as [`step`] but rejects bad
/// inputs with [`CombatError`]. Hot callers use [`try_step_into`].
pub fn try_step(
    inputs: &[FighterInput],
    state: &mut FightState,
) -> Result<Vec<FightEvent>, CombatError> {
    let mut events = Vec::new();
    try_step_into(inputs, state, &mut events)?;
    Ok(events)
}

/// Strict step into a reused event buffer under the shipped V1 row. Same
/// events and errors as [`try_step`]: a rejected tick leaves `out` empty and
/// `state` untouched. A reserved buffer makes the tick allocate nothing.
pub fn try_step_into(
    inputs: &[FighterInput],
    state: &mut FightState,
    out: &mut Vec<FightEvent>,
) -> Result<(), CombatError> {
    try_step_with_tables_into(inputs, state, &TABLES_V1, out)
}

/// Lenient pure step under explicit table rows: same tables plus same inputs
/// twice give the same events. Hot callers use [`step_with_tables_into`].
pub fn step_with_tables(
    inputs: &[FighterInput],
    state: &mut FightState,
    tables: &Tables,
) -> Vec<FightEvent> {
    let mut events = Vec::new();
    step_with_tables_into(inputs, state, tables, &mut events);
    events
}

/// Lenient pure step into a reused event buffer under explicit table rows.
/// Same events as [`step_with_tables`]; `out` is cleared first, so a reserved
/// buffer makes the tick allocate nothing.
pub fn step_with_tables_into(
    inputs: &[FighterInput],
    state: &mut FightState,
    tables: &Tables,
    out: &mut Vec<FightEvent>,
) {
    let intents = lenient_intents(inputs, state.fighter_count);
    core_step_into(&intents, state, tables, None, out, None);
}

/// Strict step under explicit table rows: same as [`step_with_tables`] but
/// rejects bad inputs with [`CombatError`]. Hot callers use
/// [`try_step_with_tables_into`].
pub fn try_step_with_tables(
    inputs: &[FighterInput],
    state: &mut FightState,
    tables: &Tables,
) -> Result<Vec<FightEvent>, CombatError> {
    let mut events = Vec::new();
    try_step_with_tables_into(inputs, state, tables, &mut events)?;
    Ok(events)
}

/// Strict step into a reused event buffer under explicit table rows. Same
/// events and errors as [`try_step_with_tables`]: a rejected tick leaves `out`
/// empty and `state` untouched. A reserved buffer allocates nothing.
pub fn try_step_with_tables_into(
    inputs: &[FighterInput],
    state: &mut FightState,
    tables: &Tables,
    out: &mut Vec<FightEvent>,
) -> Result<(), CombatError> {
    out.clear();
    validate_inputs(inputs, state.fighter_count)?;
    let mut intents = [Buttons::NONE; SLOT_COUNT];
    for input in inputs.iter() {
        // Validated above: slot is an active fighter, unique per tick.
        intents[usize::from(input.fighter.0)] = input.buttons;
    }
    core_step_into(&intents, state, tables, None, out, None);
    Ok(())
}

/// Fold an input slice into per-fighter button intents, skipping anything
/// invalid. The first input per fighter wins.
fn lenient_intents(inputs: &[FighterInput], fighter_count: u8) -> [Buttons; SLOT_COUNT] {
    let mut intents = [Buttons::NONE; SLOT_COUNT];
    if fighter_count == 0 || fighter_count > MAX_PARTICIPANTS {
        return intents;
    }
    // Copy at most the bound; extras are ignored deterministically.
    let take = inputs.len().min(MAX_INPUTS_PER_TICK);
    let mut order: [usize; MAX_INPUTS_PER_TICK] = [0; MAX_INPUTS_PER_TICK];
    for (sorted_pos, slot) in order.iter_mut().take(take).enumerate() {
        *slot = sorted_pos;
    }
    // Stable insertion sort by fighter slot.
    for i in 1..take {
        let key = order[i];
        let key_slot = inputs[key].fighter.0;
        let mut j = i;
        while j > 0 && inputs[order[j - 1]].fighter.0 > key_slot {
            order[j] = order[j - 1];
            j -= 1;
        }
        order[j] = key;
    }
    let mut seen = [false; SLOT_COUNT];
    for pos in order.iter().take(take) {
        let input = inputs[*pos];
        if input.validate().is_err() {
            continue;
        }
        let slot = input.fighter.0;
        if slot >= fighter_count || slot >= MAX_PARTICIPANTS {
            continue;
        }
        let idx = usize::from(slot);
        if seen[idx] {
            continue;
        }
        seen[idx] = true;
        intents[idx] = input.buttons;
    }
    intents
}

/// Shared deterministic core into a reused event buffer: intents (indexed by
/// fighter slot) plus state plus explicit table rows give the tick's events.
/// Hit-stop freezes the tick: it counts down and leaves `events` empty
/// without touching fighters or the clock. `events` is cleared first so
/// quiet ticks never allocate. Integer math and fighter-ID order only, so the
/// same tables plus the same inputs give the same events. `geometry` is
/// `None` under rules v1 and the validated per-slot poses under rules v2.
fn core_step_into(
    intents: &[Buttons; SLOT_COUNT],
    state: &mut FightState,
    tables: &Tables,
    geometry: Option<&[StrikePose]>,
    events: &mut Vec<FightEvent>,
    mut cues: Option<&mut Vec<AttackStarted>>,
) {
    events.clear();
    if let Some(cues) = cues.as_mut() {
        cues.clear();
    }
    let count = usize::from(state.fighter_count.min(MAX_PARTICIPANTS));
    // The balance channel carries one tick: every fighter starts the tick
    // with no impulse, so the arena reads one push per tick.
    for id in 0..count {
        state.fighters[id].balance_impulse_q8 = 0;
    }
    if state.hitstop > 0 {
        state.hitstop -= 1;
        return;
    }
    let commit = geometry.map(|_| StreamliningParams::v1());
    latch_block_holds(intents, state, tables, commit, count);
    start_actions(intents, state, tables, events, cues, commit, count);
    // GP31 prod heavy: V2 chord ATTACK+BLOCK (no DODGE) marks a just-started swing heavy.
    // A queued heavy keeps its press-time kind when the chain tick is idle:
    // live chord marks heavy, live light marks light, otherwise the kind set
    // at queue-start survives so a released block does not lighten the swing.
    if *tables == TABLES_V2 {
        for (id, intent) in intents.iter().enumerate().take(count) {
            let f = &mut state.fighters[id];
            if f.is_attacking && f.attack_age == 0 && attacking(*intent) && !dodging(*intent) {
                f.attack_kind = kind_for_buttons(intent.0, f.weapon);
            }
        }
    }
    resolve_impacts(state, tables, events, count, geometry);
    advance_clocks(state, tables, count);
    state.tick = state.tick.saturating_add(1);
}

const fn attacking(b: Buttons) -> bool {
    b.0 & ATTACK != 0
}

const fn blocking(b: Buttons) -> bool {
    b.0 & BLOCK != 0
}

const fn dodging(b: Buttons) -> bool {
    b.0 & DODGE != 0
}

/// Strike kind for one button byte: the V2 chord ATTACK+BLOCK (no DODGE) is
/// heavy, every other press is light. A fighter with a weapon row turns the
/// heavy chord into the boss push, the shove with the scutum.
const fn kind_for_buttons(buttons: u8, weapon: u8) -> u8 {
    if (buttons & ATTACK != 0) && (buttons & BLOCK != 0) && (buttons & DODGE == 0) {
        if weapon > ROW_BARE {
            KIND_PUSH
        } else {
            crate::depth::INPUT_HEAVY
        }
    } else {
        crate::depth::INPUT_LIGHT
    }
}

/// Slot index as the fighter-id byte carried by events and cues. An
/// out-of-range index maps to `None` instead of truncating.
fn slot_id_u8(id: usize) -> Option<u8> {
    u8::try_from(id).ok()
}

/// Step 1: latch block holds (a fresh press sets the parry age to zero).
/// Under the v2 commit lock an in-flight attack past its feint window refuses
/// a fresh block. Under V2 rows a release starts block coyote: the guard
/// keeps soaking for the forgive window; V1 rows reset as before.
fn latch_block_holds(
    intents: &[Buttons; SLOT_COUNT],
    state: &mut FightState,
    tables: &Tables,
    commit: Option<StreamliningParams>,
    count: usize,
) {
    let coyote = crate::feel::coyote_window(tables);
    for (id, intent) in intents.iter().enumerate().take(count) {
        let f = &mut state.fighters[id];
        if f.hp_permille == 0 || f.knockdown > 0 || f.react > 0 {
            f.is_blocking = false;
            continue;
        }
        if let Some(streamlining) = commit {
            if f.is_attacking
                && !streamlining.cancel_allowed(f.attack_age, crate::heft::startup(f.heft, tables))
            {
                f.is_blocking = false;
                f.block_age = u32::MAX;
                continue;
            }
        }
        if blocking(*intent) {
            if f.is_blocking {
                f.block_age = f.block_age.saturating_add(1);
            } else {
                f.is_blocking = true;
                f.block_age = 0;
            }
        } else if coyote > 0 {
            if f.is_blocking {
                f.is_blocking = false;
                f.block_age = 0;
            } else if f.block_age != u32::MAX {
                f.block_age = f.block_age.saturating_add(1);
                if f.block_age > coyote {
                    f.block_age = u32::MAX;
                }
            }
        } else {
            f.is_blocking = false;
            f.block_age = u32::MAX;
        }
    }
}

/// Step 2: start dodges (i-frames) or attacks; failures emit `StaminaOut`.
/// Under the v2 commit lock a dodge past the feint window is refused
/// silently, a fresh attack press inside the feint window restarts the swing
/// as a feint and a press inside the recovery chain window restarts it as a
/// chain. Fresh attack starts report one [`AttackStarted`] cue per starter
/// in fighter-ID order under [`TABLES_V2`] rows; feint restarts stay silent
/// so one swing cues exactly once. A mid-swing press past both windows is
/// queued and fires on the first chain or ready tick; V1 rows never queue
/// one, so V1 bouts replay byte-identical.
fn start_actions(
    intents: &[Buttons; SLOT_COUNT],
    state: &mut FightState,
    tables: &Tables,
    events: &mut Vec<FightEvent>,
    mut cues: Option<&mut Vec<AttackStarted>>,
    commit: Option<StreamliningParams>,
    count: usize,
) {
    for (id, intent) in intents.iter().enumerate().take(count) {
        let f = &mut state.fighters[id];
        if !f.can_act() {
            continue;
        }
        let tick = state.tick;
        if dodge_step(*intent, &mut *f, tables, &mut *events, id, commit) {
            continue;
        }
        let window = crate::feel::buffer_window(tables);
        let fresh = attacking(*intent) && !f.is_dodging && !blocking(*intent);
        // V2 heavy chord presses like a fresh tap but keeps its block bit in
        // the queue; V1 (window 0) never takes this leg so tapes replay.
        let heavy_press = window > 0
            && attacking(*intent)
            && blocking(*intent)
            && !dodging(*intent)
            && !f.is_dodging;
        if f.is_attacking {
            mid_swing_step(MidSwingStep {
                intent: *intent,
                f: &mut *f,
                tables,
                events: &mut *events,
                cues: &mut cues,
                commit,
                window,
                fresh,
                heavy_press,
                tick,
                id,
            });
            continue;
        }
        let queued_buttons = if window > 0 && !f.is_dodging && !blocking(*intent) {
            f.attack_buffer.pop_ready(tick, window)
        } else {
            None
        };
        let queued = queued_buttons.is_some();
        if fresh || heavy_press || queued {
            // Queued-only start keeps its press-time kind for the idle
            // post-step mark; live presses re-mark there.
            let queued_kind = if queued && !fresh && !heavy_press {
                queued_buttons.map(|buttons| kind_for_buttons(buttons, f.weapon))
            } else {
                None
            };
            f.attack_buffer.clear();
            if f.stamina >= tables.attack_stamina_cost {
                f.stamina -= tables.attack_stamina_cost;
                f.is_attacking = true;
                f.attack_age = 0;
                if let Some(kind) = queued_kind {
                    f.attack_kind = kind;
                }
                if *tables == TABLES_V2 {
                    if let Some(cues) = cues.as_mut() {
                        if let Some(attacker) = slot_id_u8(id) {
                            cues.push(AttackStarted { attacker });
                        }
                    }
                }
            } else if let Some(fighter) = slot_id_u8(id) {
                events.push(FightEvent::StaminaOut { fighter });
            }
        }
    }
}

/// Dodge leg of [`start_actions`]: spends stamina to start a dodge, reports
/// `StaminaOut` when broke, or bounces off the cancel rules.
fn dodge_step(
    intent: Buttons,
    f: &mut FighterState,
    tables: &Tables,
    events: &mut Vec<FightEvent>,
    id: usize,
    commit: Option<StreamliningParams>,
) -> bool {
    if !(dodging(intent) && !f.is_dodging) {
        return false;
    }
    if let Some(streamlining) = commit {
        if f.is_attacking
            && !streamlining.cancel_allowed(f.attack_age, crate::heft::startup(f.heft, tables))
        {
            return true;
        }
    }
    if f.stamina >= tables.dodge_stamina_cost {
        f.stamina -= tables.dodge_stamina_cost;
        f.is_dodging = true;
        f.dodge_age = 0;
        // A dodge cancels its own in-flight attack.
        f.is_attacking = false;
        f.attack_age = 0;
    } else if let Some(fighter) = slot_id_u8(id) {
        events.push(FightEvent::StaminaOut { fighter });
    }
    true
}

/// Grouped mid-swing inputs for [`mid_swing_step`]: one struct so the helper
/// stays under the argument-count lint. Pure grouping, no behavior change.
struct MidSwingStep<'a, 'b> {
    intent: Buttons,
    f: &'a mut FighterState,
    tables: &'a Tables,
    events: &'a mut Vec<FightEvent>,
    cues: &'a mut Option<&'b mut Vec<AttackStarted>>,
    commit: Option<StreamliningParams>,
    window: u32,
    fresh: bool,
    heavy_press: bool,
    tick: u32,
    id: usize,
}

/// Mid-swing leg of [`start_actions`]: feint/chain restarts inside their
/// windows, else buffers the press. Always consumes the tick.
fn mid_swing_step(args: MidSwingStep<'_, '_>) {
    let MidSwingStep {
        intent,
        f,
        tables,
        events,
        cues,
        commit,
        window,
        fresh,
        heavy_press,
        tick,
        id,
    } = args;
    if let Some(streamlining) = commit {
        let feint = streamlining.cancel_allowed(f.attack_age, crate::heft::startup(f.heft, tables));
        let chain = streamlining.recovery_cancel_allowed(
            f.attack_age,
            crate::heft::startup(f.heft, tables),
            crate::heft::recovery(f.heft, tables),
        );
        if feint || chain {
            let queued_buttons = if window > 0 && !f.is_dodging && !blocking(intent) {
                f.attack_buffer.peek_ready(tick, window)
            } else {
                None
            };
            let queued = queued_buttons.is_some();
            if fresh || heavy_press || queued {
                if f.stamina >= tables.attack_stamina_cost {
                    f.stamina -= tables.attack_stamina_cost;
                    f.attack_age = 0;
                    // Queued-only chain keeps its press-time kind for
                    // the idle post-step mark; live presses re-mark.
                    if queued && !fresh && !heavy_press {
                        if let Some(b) = queued_buttons {
                            f.attack_kind = kind_for_buttons(b, f.weapon);
                        }
                    }
                    f.attack_buffer.clear();
                    if chain && *tables == TABLES_V2 {
                        if let Some(cues) = cues.as_mut() {
                            if let Some(attacker) = slot_id_u8(id) {
                                cues.push(AttackStarted { attacker });
                            }
                        }
                    }
                } else if let Some(fighter) = slot_id_u8(id) {
                    events.push(FightEvent::StaminaOut { fighter });
                }
                // Restart handled (or refused for stamina): the press is
                // consumed, never rebuffered. Matches the old `continue`.
                return;
            }
        }
    }
    // Queue mid-swing taps with their real button bits so a heavy chord
    // survives until the chain tick; V1 never queues (window 0).
    if window > 0 && (fresh || heavy_press) {
        let _ = f.attack_buffer.push(tick, intent.0);
    }
}

/// Step 3: resolve impacts whose startup completes this tick, in ID order,
/// after cancelling in-flight attacks owned by staggered fighters. Rules v1
/// strikes every living foe with the guard always covering; rules v2 gates
/// each pair through [`strike_gate`] and resolves symmetrically, so slot
/// order never decides a simultaneous exchange.
fn resolve_impacts(
    state: &mut FightState,
    tables: &Tables,
    events: &mut Vec<FightEvent>,
    count: usize,
    geometry: Option<&[StrikePose]>,
) {
    for id in 0..count {
        let owner_stunned = {
            let f = state.fighters[id];
            f.hp_permille == 0 || f.react > 0 || f.knockdown > 0
        };
        if owner_stunned && state.fighters[id].is_attacking {
            state.fighters[id].is_attacking = false;
            state.fighters[id].attack_age = 0;
        }
    }
    let symmetric = geometry.is_some();
    // Rules v2 reads each guard from the state before the first same-tick
    // strike; snapshot the fields it reads once, in place (no heap copy).
    let pre = geometry.map(|_| {
        let mut snap = [PreState::default(); SLOT_COUNT];
        let mut i = 0;
        while i < count {
            snap[i] = PreState::of(&state.fighters[i]);
            i += 1;
        }
        snap
    });
    for attacker in 0..count {
        let f = pre.map_or_else(|| PreState::of(&state.fighters[attacker]), |p| p[attacker]);
        let swinger = FighterState {
            attack_kind: f.attack_kind,
            ..state.fighters[attacker]
        };
        let (startup, _recovery) = kind_timing(tables, &swinger);
        let impact = f.is_attacking
            && f.attack_age == startup
            && f.hp_permille > 0
            && f.react == 0
            && f.knockdown == 0;
        if !impact {
            continue;
        }
        let reach = strike_reach(f.weapon, f.attack_kind);
        for victim in 0..count {
            if victim == attacker {
                continue;
            }
            let guard = pre.map_or_else(|| PreState::of(&state.fighters[victim]), |p| p[victim]);
            if guard.hp_permille == 0 {
                continue;
            }
            let covered = match geometry.map(|poses| strike_gate(poses, attacker, victim, reach)) {
                None | Some(StrikeGate::Covered) => true,
                Some(StrikeGate::Open) => false,
                Some(StrikeGate::Miss) => continue,
            };
            let strike = Strike {
                attacker,
                victim,
                kind: f.attack_kind,
                weapon: f.weapon,
                covered,
                guard,
                symmetric,
            };
            if resolve_strike(state, tables, events, strike) {
                break;
            }
        }
    }
}

/// Compact pre-resolution view of the [`FighterState`] fields
/// [`resolve_impacts`] reads before any same-tick strike lands. Rules v2
/// snapshots once per tick; this view is a quarter the size of
/// [`FighterState`], so every read keeps the exact pre-resolution value.
#[derive(Clone, Copy, Default)]
struct PreState {
    is_attacking: bool,
    attack_age: u32,
    attack_kind: u8,
    weapon: u8,
    hp_permille: u16,
    react: u32,
    knockdown: u32,
    is_dodging: bool,
    dodge_age: u32,
    is_blocking: bool,
    block_age: u32,
}

impl PreState {
    /// The impact and guard fields of one fighter, read in place.
    fn of(f: &FighterState) -> Self {
        Self {
            is_attacking: f.is_attacking,
            attack_age: f.attack_age,
            attack_kind: f.attack_kind,
            weapon: f.weapon,
            hp_permille: f.hp_permille,
            react: f.react,
            knockdown: f.knockdown,
            is_dodging: f.is_dodging,
            dodge_age: f.dodge_age,
            is_blocking: f.is_blocking,
            block_age: f.block_age,
        }
    }

    /// True during dodge invulnerability frames, exactly as
    /// [`FighterState::invulnerable_with`] reads them.
    #[must_use]
    fn invulnerable_with(self, tables: &Tables) -> bool {
        self.is_dodging && self.dodge_age < tables.dodge_iframes_ticks
    }

    /// True while a held block still parries, exactly as
    /// [`FighterState::parry_open_with`] reads it.
    #[must_use]
    fn parry_open_with(self, tables: &Tables) -> bool {
        self.is_blocking && self.block_age <= tables.parry_window_ticks
    }
}

/// One impacting attacker against one victim, as [`resolve_strike`] reads it.
#[derive(Clone, Copy)]
struct Strike {
    attacker: usize,
    victim: usize,
    /// Strike kind of the in-flight attack; the hit branch freezes per kind.
    kind: u8,
    /// Weapon row of the attacker: the damage and the reach come from it.
    weapon: u8,
    /// Guard faces the attacker (always under v1; shield cone under v2).
    covered: bool,
    /// Victim guard fields the dodge, parry and block checks read: live under
    /// v1, pre-resolution under v2.
    guard: PreState,
    /// Rules v2: stun clocks only ever grow (`max`), so the order two
    /// same-tick strikes land in cannot change the result.
    symmetric: bool,
}

/// One impacting attacker meets one living victim: dodge, parry, block or hit
/// in that priority, read from `strike.guard`. Returns true when the attack
/// was parried or when a push landed, which ends the attacker's remaining
/// victims for this tick.
fn resolve_strike(
    state: &mut FightState,
    tables: &Tables,
    events: &mut Vec<FightEvent>,
    strike: Strike,
) -> bool {
    let Strike {
        attacker,
        victim,
        kind,
        weapon,
        covered,
        guard,
        symmetric,
    } = strike;
    let a = u8::try_from(attacker).unwrap_or(u8::MAX);
    let v = u8::try_from(victim).unwrap_or(u8::MAX);
    let stun = |current: u32, ticks: u32| if symmetric { current.max(ticks) } else { ticks };
    if guard.invulnerable_with(tables) {
        state.fighters[victim].juggle = 0;
        events.push(FightEvent::Dodged {
            attacker: a,
            victim: v,
        });
        return false;
    }
    // The boss push: a shove with the scutum. The steel never touches the
    // man, so a parry of the steel does not catch it; the shove breaks his
    // guard, staggers him and feeds his balance.
    if kind == KIND_PUSH {
        push_strike(state, victim, symmetric);
        return true;
    }
    if covered && guard.parry_open_with(tables) {
        state.fighters[victim].juggle = 0;
        events.push(FightEvent::Parried {
            attacker: a,
            victim: v,
        });
        let owner = &mut state.fighters[attacker];
        owner.react = stun(owner.react, tables.parry_stagger_ticks);
        owner.is_attacking = false;
        owner.attack_age = 0;
        state.hitstop = hitstop_for_impact(tables, kind, true);
        return true;
    }
    let blocked = blocked_by_guard(tables, covered, guard);
    let raw = strike_damage(
        crate::heft::damage(state.fighters[attacker].heft, tables),
        weapon,
    );
    let damage = if blocked { tables.chip(raw) } else { raw };
    let was_up = state.fighters[victim].hp_permille > 0;
    apply_damage(&mut state.fighters[victim], damage);
    events.push(if blocked {
        FightEvent::Blocked {
            attacker: a,
            victim: v,
            chip: damage,
        }
    } else {
        FightEvent::Hit {
            attacker: a,
            victim: v,
            damage,
        }
    });
    let clean = {
        let target = &mut state.fighters[victim];
        if target.hp_permille == 0 {
            target.juggle = 0;
            if was_up {
                target.knockdown = tables.knockdown_ticks;
                target.is_attacking = false;
                target.is_dodging = false;
                events.push(FightEvent::Knockdown {
                    attacker: a,
                    victim: v,
                });
            }
            false
        } else if blocked {
            target.juggle = 0;
            target.react = target.react.max(1);
            false
        } else {
            true
        }
    };
    if clean {
        apply_juggle_hit(state, tables, events, a, victim, v, symmetric);
    }
    if !blocked {
        state.hitstop = hitstop_for_impact(tables, kind, false);
    }
    false
}

/// True when the victim's guard soaks the strike: a held block, or the v2
/// coyote window after a release. V1 rows grant no forgive window.
fn blocked_by_guard(tables: &Tables, covered: bool, guard: PreState) -> bool {
    let forgive = crate::feel::coyote_window(tables);
    let coyote = forgive > 0
        && !guard.is_blocking
        && guard.block_age != u32::MAX
        && guard.block_age <= forgive;
    covered && (guard.is_blocking || coyote)
}

/// The boss push lands: the shove breaks the victim's guard, staggers him
/// and feeds his balance the impulse a body push feeds.
fn push_strike(state: &mut FightState, victim: usize, symmetric: bool) {
    let target = &mut state.fighters[victim];
    target.react = if symmetric {
        target.react.max(PUSH_STAGGER_TICKS)
    } else {
        PUSH_STAGGER_TICKS
    };
    target.is_blocking = false;
    target.block_age = u32::MAX;
    target.balance_impulse_q8 = PUSH_IMPULSE_Q8;
    state.hitstop = PUSH_HITSTOP_TICKS;
}

/// The AIN6 juggle budget on one clean hit: the first hit starts the count,
/// each further hit while the victim is still in hitstun adds one, and the
/// first hit past [`JUGGLE_BUDGET`] turns into a knockdown instead of more
/// stun.
fn apply_juggle_hit(
    state: &mut FightState,
    tables: &Tables,
    events: &mut Vec<FightEvent>,
    attacker: u8,
    victim: usize,
    victim_id: u8,
    symmetric: bool,
) {
    let react = state
        .fighters
        .get(usize::from(attacker))
        .map_or(tables.hit_react_ticks, |swinger| {
            crate::heft::react(swinger.heft, tables)
        });
    let target = &mut state.fighters[victim];
    if target.react > 0 {
        target.juggle = target.juggle.saturating_add(1);
    } else {
        target.juggle = 1;
    }
    if target.juggle > JUGGLE_BUDGET {
        target.juggle = 0;
        target.knockdown = tables.knockdown_ticks;
        target.is_attacking = false;
        target.is_dodging = false;
        events.push(FightEvent::Knockdown {
            attacker,
            victim: victim_id,
        });
    } else if symmetric {
        target.react = target.react.max(react);
    } else {
        target.react = react;
    }
}

/// Step 4: advance attack, dodge, react and knockdown clocks and regen
/// stamina. A swing in flight is exertion, so an attacking fighter does not
/// regen; idle, blocking and post-swing ticks do.
fn advance_clocks(state: &mut FightState, tables: &Tables, count: usize) {
    for id in 0..count {
        let f = &mut state.fighters[id];
        if f.is_attacking {
            f.attack_age = f.attack_age.saturating_add(1);
            let (startup, recovery) = kind_timing(tables, f);
            if f.attack_age > startup + recovery {
                f.is_attacking = false;
                f.attack_age = 0;
            }
        }
        if f.is_dodging {
            f.dodge_age = f.dodge_age.saturating_add(1);
            if f.dodge_age >= tables.dodge_duration_ticks {
                f.is_dodging = false;
                f.dodge_age = 0;
            }
        }
        if f.react > 0 {
            f.react -= 1;
            if f.react == 0 {
                f.juggle = 0;
            }
        }
        if f.knockdown > 0 && f.hp_permille > 0 {
            // Knockdown counts down only after a revive; the out fighter
            // stays down. (Revives are out of scope: nothing heals here.)
            f.knockdown -= 1;
        }
        if f.is_blocking {
            f.stamina = f.stamina.saturating_sub(tables.block_hold_cost_per_tick);
        }
        // A swing in flight is exertion: no stamina comes back while the
        // attack clock runs, so a mash cadence drains the bar.
        if !f.is_attacking {
            f.stamina = (f.stamina.saturating_add(tables.stamina_regen_per_tick)).min(MAX_STAMINA);
        }
        if f.hp_permille > MAX_HP_PERMILLE {
            f.hp_permille = MAX_HP_PERMILLE;
        }
    }
}

/// Subtract damage with an explicit floor at zero (no wrap, no panic).
fn apply_damage(fighter: &mut FighterState, damage: u16) {
    fighter.hp_permille = fighter.hp_permille.saturating_sub(damage);
}

#[cfg(test)]
mod tests {
    use super::shield_line::{SHIELD_DAMAGE, SHIELD_KINDS};
    use super::*;
    use engine_core::alloc_probe::measure;
    use engine_input::{Buttons, FighterId};

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

    fn idle_pair() -> [FighterInput; 2] {
        [none(0), none(1)]
    }

    fn tick(state: &mut FightState, inputs: &[FighterInput]) -> Vec<FightEvent> {
        step(inputs, state)
    }

    /// Rules-v2 pose: standard arena hurt capsule (800 mm half, 150 mm
    /// radius, 60 degree shield) at `(x, z)` mm facing `(fx, fz)`, blade from
    /// 200 mm to 1500 mm ahead along the facing signs.
    fn pose(slot: u8, x: i32, z: i32, facing: (i32, i32), team: u8) -> StrikePose {
        let (sx, sz) = (facing.0.signum(), facing.1.signum());
        StrikePose {
            body: SweepParticipant {
                id: slot,
                pos_mm: [x, 0, z],
                facing_x: facing.0,
                facing_z: facing.1,
                hurt_half_height_mm: 800,
                hurt_radius_mm: 150,
                shield_half_deg: 60,
                shield_up: false,
            },
            blade: WeaponSweep {
                attacker: slot,
                p0_mm: [x + sx * 200, 0, z + sz * 200],
                p1_mm: [x + sx * 1500, 0, z + sz * 1500],
                blade_half_mm: 10,
            },
            team,
        }
    }

    /// Slot 0 presses attack on tick 0 while slot 1 does `victim_buttons`
    /// every tick; returns every event through the impact tick under v2.
    fn v2_exchange(poses: &[StrikePose; 2], victim_buttons: u8) -> Vec<FightEvent> {
        let mut state = FightState::new(2).unwrap();
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut all = Vec::new();
        for t in 0..=TABLES_V1.attack_startup_ticks {
            let attack = if t == 0 { ATTACK } else { 0 };
            let inputs = [input(0, attack), input(1, victim_buttons)];
            try_step_v2_into(&inputs, poses, &mut state, &TABLES_V1, &mut out).unwrap();
            all.extend_from_slice(&out);
        }
        all
    }

    fn hit_v2() -> FightEvent {
        FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: ATTACK_DAMAGE,
        }
    }

    #[test]
    fn v2_foe_ten_metres_away_is_not_hit() {
        // Slot 0 at the origin faces +x; the foe stands 10 m down that line.
        let far = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 10_000, 0, (-1000, 0), 1),
        ];
        assert!(v2_exchange(&far, 0).is_empty(), "no hit across the arena");
        // The same foe at 1 m takes the hit.
        let near = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        assert_eq!(v2_exchange(&near, 0), vec![hit_v2()]);
        // F2P: the frozen v1 rule hits at any range (it never reads poses).
        let mut state = FightState::new(2).unwrap();
        let mut events = tick(&mut state, &[input(0, ATTACK), none(1)]);
        for _ in 0..TABLES_V1.attack_startup_ticks {
            events.extend(tick(&mut state, &idle_pair()));
        }
        assert_eq!(events, vec![hit_v2()]);
    }

    #[test]
    fn v2_foe_behind_the_attacker_is_not_hit() {
        let behind = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, -1_000, 0, (1000, 0), 1),
        ];
        assert!(v2_exchange(&behind, 0).is_empty());
    }

    #[test]
    fn v2_block_from_behind_fails_and_from_the_front_holds() {
        // Victim 1 m ahead of the attacker, guard held from tick 0.
        // Facing the attacker: the parry window catches the impact.
        let facing = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        assert_eq!(
            v2_exchange(&facing, BLOCK),
            vec![FightEvent::Parried {
                attacker: 0,
                victim: 1
            }]
        );
        // Facing away (attacker at its back): neither parry nor block count.
        let turned = [pose(0, 0, 0, (1000, 0), 0), pose(1, 1_000, 0, (1000, 0), 1)];
        assert_eq!(v2_exchange(&turned, BLOCK), vec![hit_v2()]);
    }

    #[test]
    fn v2_teammate_is_not_hit() {
        let mates = [
            pose(0, 0, 0, (1000, 0), 4),
            pose(1, 1_000, 0, (-1000, 0), 4),
        ];
        assert!(v2_exchange(&mates, 0).is_empty());
        let foes = [
            pose(0, 0, 0, (1000, 0), 4),
            pose(1, 1_000, 0, (-1000, 0), 5),
        ];
        assert_eq!(v2_exchange(&foes, 0), vec![hit_v2()]);
    }

    /// Rules-v2 poses with the foe 10 m down the blade line: every impact
    /// misses, so one swing runs its full clock with no hit-stop, reaction
    /// or damage to disturb the cancel assertions.
    fn v2_far() -> [StrikePose; 2] {
        [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 10_000, 0, (-1000, 0), 1),
        ]
    }

    /// Step `n` idle ticks under the v2 leg; returns every event seen.
    fn idle_v2(
        state: &mut FightState,
        poses: &[StrikePose; 2],
        tables: &Tables,
        n: u32,
    ) -> Vec<FightEvent> {
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut all = Vec::new();
        for _ in 0..n {
            try_step_v2_into(&idle_pair(), poses, state, tables, &mut out).unwrap();
            all.extend_from_slice(&out);
        }
        all
    }

    /// GP29 commit lock, v2 only. Oracle: startup 15 and grace 2, so the
    /// feint window is ages 0..=12; age 16 (hit tick + 1) is locked and the
    /// swing ages out through recovery (past 23) on its own clock.
    #[test]
    fn commit_lock_refuses_late_dodge_under_v2_only() {
        let tables = &TABLES_V2;
        let poses = v2_far();
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);

        // A dodge 2 ticks into startup still feints under v2.
        let mut state = FightState::new(2).unwrap();
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert_eq!(state.fighters[0].attack_age, 2);
        try_step_v2_into(
            &[input(0, DODGE), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(
            !state.fighters[0].is_attacking,
            "early dodge cancels the swing"
        );
        assert!(state.fighters[0].is_dodging, "early dodge starts");

        // A dodge at hit tick + 1 is refused under v2 and recovery holds.
        let mut state = FightState::new(2).unwrap();
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(
            idle_v2(&mut state, &poses, tables, 15).is_empty(),
            "far foe: no impacts"
        );
        assert_eq!(state.fighters[0].attack_age, 16);
        try_step_v2_into(
            &[input(0, DODGE), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(state.fighters[0].is_attacking, "locked swing runs on");
        assert!(!state.fighters[0].is_dodging, "late dodge never starts");
        assert!(out.is_empty(), "a refused cancel emits nothing");
        assert!(idle_v2(&mut state, &poses, tables, 8).is_empty());
        assert!(
            !state.fighters[0].is_attacking,
            "recovery ends by the clock"
        );
        assert!(!state.fighters[0].is_dodging);
        assert_eq!(state.fighters[0].attack_age, 0);

        // The same late dodge under v1 cancels as it always has: impact on
        // the 4-tick startup, 4 frozen ticks, then the dodge lands.
        let mut old = FightState::new(2).unwrap();
        tick(&mut old, &[input(0, ATTACK), none(1)]);
        for _ in 0..8 {
            tick(&mut old, &idle_pair());
        }
        assert_eq!(old.fighters[0].attack_age, 5);
        assert_eq!(old.hitstop, 0, "dodge pressed after the freeze drains");
        tick(&mut old, &[input(0, DODGE), none(1)]);
        assert!(
            !old.fighters[0].is_attacking,
            "v1 still cancels at any tick"
        );
        assert!(old.fighters[0].is_dodging);

        // The v2 late-dodge run twice gives the same state and events.
        let run = || {
            let mut s = FightState::new(2).unwrap();
            let mut o = Vec::with_capacity(MAX_EVENTS_PER_TICK);
            let mut e = Vec::new();
            try_step_v2_into(&[input(0, ATTACK), none(1)], &poses, &mut s, tables, &mut o).unwrap();
            e.extend_from_slice(&o);
            for _ in 0..15 {
                try_step_v2_into(&idle_pair(), &poses, &mut s, tables, &mut o).unwrap();
                e.extend_from_slice(&o);
            }
            try_step_v2_into(&[input(0, DODGE), none(1)], &poses, &mut s, tables, &mut o).unwrap();
            e.extend_from_slice(&o);
            (s, e)
        };
        assert_eq!(run(), run(), "commit lock replays exactly");
    }

    /// GP29 feint, v2 only: a fresh attack press inside the window restarts
    /// the swing; under v1 the same press is ignored and the clock runs on.
    #[test]
    fn commit_lock_feint_restarts_inside_the_window_under_v2() {
        let tables = &TABLES_V2;
        let poses = v2_far();
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);

        let mut state = FightState::new(2).unwrap();
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert_eq!(state.fighters[0].attack_age, 2);
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(
            state.fighters[0].is_attacking,
            "feint keeps the swing alive"
        );
        assert_eq!(
            state.fighters[0].attack_age, 1,
            "feint restarts, then the clock advances"
        );

        let mut old = FightState::new(2).unwrap();
        tick(&mut old, &[input(0, ATTACK), none(1)]);
        tick(&mut old, &idle_pair());
        assert_eq!(old.fighters[0].attack_age, 2);
        tick(&mut old, &[input(0, ATTACK), none(1)]);
        assert!(old.fighters[0].is_attacking);
        assert_eq!(
            old.fighters[0].attack_age, 3,
            "v1 ignores a second press mid-swing"
        );
    }

    /// GP29 commit lock, v2 only. Oracle: startup 15, grace 2 and recovery 8,
    /// so age 16 (hit tick + 1) is locked past the feint window while the
    /// chain window opens at 18. A block at 16 never latches; a fresh attack
    /// there queues and fires as a chain once the window opens. V1 keeps its
    /// old latch and drops every mid-swing tap.
    #[test]
    fn commit_lock_refuses_late_block_and_chains_queued_attack_under_v2() {
        let tables = &TABLES_V2;
        let poses = v2_far();
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);

        // A block at hit tick + 1 is refused under v2 and recovery holds.
        let mut state = FightState::new(2).unwrap();
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(
            idle_v2(&mut state, &poses, tables, 15).is_empty(),
            "far foe: no impacts"
        );
        assert_eq!(state.fighters[0].attack_age, 16);
        try_step_v2_into(
            &[input(0, BLOCK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(state.fighters[0].is_attacking, "locked swing runs on");
        assert!(!state.fighters[0].is_blocking, "late block never latches");
        assert!(out.is_empty(), "a refused block emits nothing");
        assert!(idle_v2(&mut state, &poses, tables, 8).is_empty());
        assert!(
            !state.fighters[0].is_attacking,
            "recovery ends by the clock"
        );

        // A fresh attack at hit tick + 1 is locked past the feint window, so
        // the tap queues; the queue fires as a chain once the recovery
        // window opens at age 18.
        let mut state = FightState::new(2).unwrap();
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(
            idle_v2(&mut state, &poses, tables, 15).is_empty(),
            "far foe: no impacts"
        );
        assert_eq!(state.fighters[0].attack_age, 16);
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(state.fighters[0].is_attacking, "locked swing runs on");
        assert_eq!(
            state.fighters[0].attack_age, 17,
            "a press past the feint window never restarts the clock at once"
        );
        assert_eq!(
            state.fighters[0].attack_buffer.len(),
            1,
            "late tap queues in the v2 input buffer"
        );
        assert!(out.is_empty(), "a queued attack emits nothing");
        // Age 17 is still inside the post-hit grace, so the queue waits; at
        // age 18 the chain window opens and the queued tap restarts the swing.
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert_eq!(state.fighters[0].attack_age, 18);
        assert!(state.fighters[0].is_attacking);
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert_eq!(
            state.fighters[0].attack_age, 1,
            "queued tap chains once the recovery window opens"
        );
        assert!(state.fighters[0].attack_buffer.is_empty());

        // The same late block under v1 latches as it always has.
        let mut old = FightState::new(2).unwrap();
        tick(&mut old, &[input(0, ATTACK), none(1)]);
        for _ in 0..8 {
            tick(&mut old, &idle_pair());
        }
        tick(&mut old, &[input(0, BLOCK), none(1)]);
        assert!(
            old.fighters[0].is_blocking,
            "v1 still latches a block mid-swing"
        );
    }

    /// Recovery chain, v2 only: a fresh attack press inside the chain window
    /// restarts the swing at once and cues the new swing exactly once.
    #[test]
    fn recovery_chain_restarts_and_cues_once_under_v2() {
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK};
        let tables = &TABLES_V2;
        let poses = v2_far();
        let mut state = FightState::new(2).unwrap();
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
            &mut cues,
        )
        .unwrap();
        assert_eq!(cues.len(), 1);
        assert!(idle_v2(&mut state, &poses, tables, 17).is_empty());
        assert_eq!(state.fighters[0].attack_age, 18);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            tables,
            &mut out,
            &mut cues,
        )
        .unwrap();
        assert!(state.fighters[0].is_attacking, "chain keeps swinging");
        assert_eq!(state.fighters[0].attack_age, 1, "chain restarts the clock");
        assert_eq!(
            cues,
            vec![AttackStarted { attacker: 0 }],
            "chain cues its new swing once"
        );
        assert!(out.is_empty(), "chain tick carries no impact yet");
        assert!(state.fighters[0].attack_buffer.is_empty());
    }

    /// Block coyote, v2 only: a block released inside the forgive window
    /// still soaks as a block; past it the full hit lands. V1 never forgives.
    #[test]
    fn block_coyote_soaks_chip_after_release_under_v2() {
        let tables = &TABLES_V2;
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        // Defender holds, then releases one tick before the impact.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = tables.attack_startup_ticks - 1;
        state.fighters[1].is_blocking = true;
        state.fighters[1].block_age = 5;
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert!(!state.fighters[1].is_blocking, "release unlatches the hold");
        assert_eq!(state.fighters[1].block_age, 0);
        assert!(out.is_empty(), "no impact yet");
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert!(
            out.iter().any(|e| matches!(
                e,
                FightEvent::Blocked {
                    attacker: 0,
                    victim: 1,
                    ..
                }
            )),
            "coyote soaks the hit down to chip"
        );
        assert_eq!(
            state.fighters[1].hp_permille,
            MAX_HP_PERMILLE - tables.chip(tables.attack_damage)
        );
        // Past the window the same release takes the full hit.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = tables.attack_startup_ticks;
        state.fighters[1].is_blocking = false;
        state.fighters[1].block_age = 3;
        try_step_v2_into(&idle_pair(), &poses, &mut state, tables, &mut out).unwrap();
        assert!(
            out.iter().any(|e| matches!(
                e,
                FightEvent::Hit {
                    attacker: 0,
                    victim: 1,
                    ..
                }
            )),
            "a stale release takes the full hit"
        );
        // V1 never forgives: a release one tick before impact takes the hit.
        let mut old = FightState::new(2).unwrap();
        old.fighters[0].is_attacking = true;
        old.fighters[0].attack_age = ATTACK_STARTUP_TICKS - 1;
        old.fighters[1].is_blocking = true;
        old.fighters[1].block_age = 5;
        let v1_events = tick(&mut old, &idle_pair());
        assert!(v1_events.is_empty());
        assert_eq!(old.fighters[1].block_age, u32::MAX);
        let v1_events = tick(&mut old, &idle_pair());
        assert!(
            v1_events.iter().any(|e| matches!(
                e,
                FightEvent::Hit {
                    attacker: 0,
                    victim: 1,
                    ..
                }
            )),
            "v1 releases never soak"
        );
    }

    #[test]
    fn v2_simultaneous_strikes_trade_symmetrically() {
        // Both swing on tick 0, 1 m apart, facing each other. v1 resolves in
        // slot order (slot 0's hit staggers slot 1 and cancels its impact);
        // v2 reads the pre-resolution state, so both hits land.
        let duel = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let run = |v2: bool| {
            let mut state = FightState::new(2).unwrap();
            let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
            let mut all = Vec::new();
            for t in 0..=TABLES_V1.attack_startup_ticks {
                let buttons = if t == 0 { ATTACK } else { 0 };
                let inputs = [input(0, buttons), input(1, buttons)];
                if v2 {
                    try_step_v2_into(&inputs, &duel, &mut state, &TABLES_V1, &mut out).unwrap();
                } else {
                    try_step_with_tables_into(&inputs, &mut state, &TABLES_V1, &mut out).unwrap();
                }
                all.extend_from_slice(&out);
            }
            (all, state)
        };
        let (v1_events, _) = run(false);
        assert_eq!(v1_events, vec![hit_v2()], "v1 favours slot 0");
        let (v2_events, state) = run(true);
        assert_eq!(
            v2_events,
            vec![
                hit_v2(),
                FightEvent::Hit {
                    attacker: 1,
                    victim: 0,
                    damage: ATTACK_DAMAGE,
                },
            ]
        );
        assert_eq!(state.fighters[0].hp_permille, state.fighters[1].hp_permille);
        assert_eq!(state.fighters[0].react, state.fighters[1].react);
    }
    #[test]
    fn v2_rejects_bad_geometry_untouched() {
        let mut state = FightState::new(2).unwrap();
        let before = state;
        let mut out = vec![hit_v2()];
        let one = [pose(0, 0, 0, (1000, 0), 0)];
        assert_eq!(
            try_step_v2_into(&idle_pair(), &one, &mut state, &TABLES_V1, &mut out),
            Err(CombatError::BadStrikePose(2))
        );
        let swapped = [
            pose(1, 0, 0, (1000, 0), 0),
            pose(0, 1_000, 0, (-1000, 0), 1),
        ];
        assert_eq!(
            try_step_v2_into(&idle_pair(), &swapped, &mut state, &TABLES_V1, &mut out),
            Err(CombatError::BadStrikePose(0))
        );
        let mut still = pose(1, 1_000, 0, (-1000, 0), 1);
        still.body.facing_x = 0;
        let no_facing = [pose(0, 0, 0, (1000, 0), 0), still];
        assert_eq!(
            try_step_v2_into(&idle_pair(), &no_facing, &mut state, &TABLES_V1, &mut out),
            Err(CombatError::BadStrikePose(1))
        );
        assert!(out.is_empty());
        assert_eq!(state, before);
    }
    #[test]
    fn hit_react_ticks_matches_anim_window() {
        // The window is single-owned here; anim reads this const through the
        // documented anim→combat edge, so no sync pin is needed — the
        // shipped table wiring below proves the plumbing.
        assert_eq!(TABLES_V1.hit_react_ticks, HIT_REACT_TICKS);
    }

    #[test]
    fn tables_have_expected_rows() {
        assert_eq!(ATTACK_DAMAGE, 120);
        assert_eq!((ATTACK_STARTUP_TICKS, ATTACK_RECOVERY_TICKS), (4, 8));
        assert_eq!(block_chip(ATTACK_DAMAGE), 24);
        assert_eq!((PARRY_WINDOW_TICKS, DODGE_IFRAMES_TICKS), (4, 6));
        assert_eq!(HITSTOP_TICKS, 4);
        assert_eq!((ATTACK_STAMINA_COST, STAMINA_REGEN_PER_TICK), (25, 2));
    }

    #[test]
    fn tables_v2_startup_in_readable_range() {
        let v2 = TABLES_V2;
        assert!(
            (12..=18).contains(&v2.attack_startup_ticks),
            "v2 startup {} outside 12-18",
            v2.attack_startup_ticks
        );
        assert!(
            (6..=8).contains(&v2.parry_window_ticks),
            "v2 parry {} outside 6-8",
            v2.parry_window_ticks
        );
        assert!(
            v2.parry_window_ticks < v2.attack_startup_ticks,
            "parry must sit under startup"
        );
        assert_ne!(v2, TABLES_V1, "v2 must differ from frozen v1");
        // Only timing moves; every other row matches the frozen base.
        let mut expected = TABLES_V1;
        expected.attack_startup_ticks = v2.attack_startup_ticks;
        expected.parry_window_ticks = v2.parry_window_ticks;
        assert_eq!(v2, expected);
        assert_eq!(v2, crate::tables::v2_rows());
    }

    #[test]
    fn tables_v2_validates_and_resolves() {
        use crate::tables::{tables_for_version, validate_tables, MoveTableSet};
        assert_eq!(validate_tables(&MoveTableSet::v2()), Ok(()));
        // Resolver half: v2 has no static replay row yet, so a legacy
        // resolver returning `None` skips; a v2-aware resolver must match.
        if let Some(resolved) = tables_for_version(2) {
            assert_eq!(*resolved, TABLES_V2);
        }
        // Two builds give equal rows twice: deterministic construction.
        let first = MoveTableSet::v2();
        let second = MoveTableSet::v2();
        assert_eq!(first, second);
        assert_eq!(first.tables, TABLES_V2);
        assert_eq!(second.tables, TABLES_V2);
    }

    #[test]
    fn parry_inside_window_negates_and_staggers_attacker() {
        let mut state = FightState::new(2).unwrap();
        // Attacker presses on tick 0, defender holds block from tick 0.
        tick(&mut state, &[input(0, ATTACK), input(1, BLOCK)]);
        for _ in 0..3 {
            tick(&mut state, &[none(0), input(1, BLOCK)]);
        }
        // Impact tick: block age 4 is the last open window tick.
        let events = tick(&mut state, &[none(0), input(1, BLOCK)]);
        assert!(events.contains(&FightEvent::Parried {
            attacker: 0,
            victim: 1
        }));
        assert_eq!(state.fighters[1].hp_permille, MAX_HP_PERMILLE);
        assert!(state.fighters[0].react > 0);
        assert_eq!(state.hitstop, HITSTOP_TICKS);
    }

    #[test]
    fn hitstop_kinds() {
        use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
        // Migrated pins: the V2 kind map lives in `crate::hitstop_kind`;
        // V1 keeps the single row through the live `hitstop_for_impact`.
        assert_eq!(
            hitstop_for_impact(&TABLES_V1, INPUT_LIGHT, false),
            HITSTOP_TICKS
        );
        assert_eq!(
            hitstop_for_impact(&TABLES_V1, INPUT_HEAVY, false),
            HITSTOP_TICKS
        );
        assert_eq!(
            hitstop_for_impact(&TABLES_V1, INPUT_LIGHT, true),
            HITSTOP_TICKS
        );
        assert_eq!(crate::hitstop_kind::for_kind(INPUT_LIGHT), 3);
        assert_eq!(crate::hitstop_kind::for_kind(INPUT_HEAVY), 6);
        assert_eq!(crate::hitstop_kind::for_parry(), 8);
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_LIGHT, false), 3);
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_HEAVY, false), 6);
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_LIGHT, true), 8);
        // Failure case: unknown kind never freezes under V2.
        assert_eq!(crate::hitstop_kind::for_kind(9), 0);
        assert_eq!(hitstop_for_impact(&TABLES_V2, 9, false), 0);
        // Wired caller: V1 parry still freezes the single row.
        let mut v1 = FightState::new(2).unwrap();
        v1.fighters[0].is_attacking = true;
        v1.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        let v1_events = tick(&mut v1, &[none(0), input(1, BLOCK)]);
        assert!(v1_events.contains(&FightEvent::Parried {
            attacker: 0,
            victim: 1
        }));
        assert_eq!(v1.hitstop, HITSTOP_TICKS);
        // Wired caller: V2 parry freezes 8 through resolve_strike.
        let tables = &TABLES_V2;
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut v2 = FightState::new(2).unwrap();
        v2.fighters[0].is_attacking = true;
        v2.fighters[0].attack_age = tables.attack_startup_ticks;
        v2.fighters[1].is_blocking = true;
        v2.fighters[1].block_age = 0;
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        try_step_v2_into(
            &[none(0), input(1, BLOCK)],
            &poses,
            &mut v2,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(out.contains(&FightEvent::Parried {
            attacker: 0,
            victim: 1
        }));
        assert_eq!(v2.hitstop, 8);
        // Wired caller: V2 light hit freezes 3 and heavy hit freezes 6
        // through the strike kind.
        let mut v2hit = FightState::new(2).unwrap();
        let mut hit_out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut v2hit,
            tables,
            &mut hit_out,
        )
        .unwrap();
        for _ in 0..tables.attack_startup_ticks {
            try_step_v2_into(
                &[none(0), none(1)],
                &poses,
                &mut v2hit,
                tables,
                &mut hit_out,
            )
            .unwrap();
            if hit_out.iter().any(|e| matches!(e, FightEvent::Hit { .. })) {
                break;
            }
        }
        assert!(hit_out.iter().any(|e| matches!(e, FightEvent::Hit { .. })));
        assert_eq!(v2hit.hitstop, 3);
    }

    #[test]
    fn hitstop_kinds_class_v2_gives_per_kind() {
        use crate::classes::Class;
        use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
        // Class rows over V2 keep V2 timing with scaled damage; they freeze
        // per-kind like V2, not the single row, so feel surveys read the same
        // freeze as the base timing. Kind rows are pinned in
        // `crate::hitstop_kind`; class rows prove it through the live impact.
        for class in Class::all() {
            let row = class_tables(class);
            assert_eq!(
                crate::hitstop_kind::for_kind(INPUT_LIGHT),
                3,
                "{class:?} light map"
            );
            assert_eq!(
                crate::hitstop_kind::for_kind(INPUT_HEAVY),
                6,
                "{class:?} heavy map"
            );
            assert_eq!(crate::hitstop_kind::for_parry(), 8, "{class:?} parry map");
            assert_eq!(
                hitstop_for_impact(&row, INPUT_LIGHT, false),
                3,
                "{class:?} impact light"
            );
            assert_eq!(
                hitstop_for_impact(&row, INPUT_HEAVY, false),
                6,
                "{class:?} impact heavy"
            );
            assert_eq!(
                hitstop_for_impact(&row, INPUT_LIGHT, true),
                8,
                "{class:?} impact parry"
            );
        }
        // Class rows over V1 keep V1 timing; they keep the single row.
        for class in Class::all() {
            let row = crate::classes::tables(class, TABLES_V1);
            assert_eq!(
                hitstop_for_impact(&row, INPUT_LIGHT, false),
                HITSTOP_TICKS,
                "{class:?} v1 light"
            );
            assert_eq!(
                hitstop_for_impact(&row, INPUT_HEAVY, false),
                HITSTOP_TICKS,
                "{class:?} v1 heavy"
            );
            assert_eq!(
                hitstop_for_impact(&row, INPUT_LIGHT, true),
                HITSTOP_TICKS,
                "{class:?} v1 parry"
            );
        }
    }

    #[test]
    fn heavy_hit_freezes_six_ticks_v2() {
        use crate::depth::INPUT_HEAVY;
        // GP31: a v2 heavy hit freezes 6 ticks through the wired strike path.
        let tables = &TABLES_V2;
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut heavy = FightState::new(2).unwrap();
        heavy.fighters[0].is_attacking = true;
        heavy.fighters[0].attack_age = tables.attack_startup_ticks;
        heavy.fighters[0].attack_kind = INPUT_HEAVY;
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        try_step_v2_into(&[none(0), none(1)], &poses, &mut heavy, tables, &mut out).unwrap();
        assert!(out.iter().any(|e| matches!(e, FightEvent::Hit { .. })));
        assert_eq!(heavy.hitstop, 6);
    }

    /// Buffered heavy, v2 only: a heavy chord pressed mid-swing queues with
    /// its block bit and chains heavy after release, freezing 6 through the
    /// wired strike path. F2P oracle: pressed heavy, chained heavy.
    #[test]
    fn buffered_heavy_chord_chains_heavy_and_freezes_six_v2() {
        use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
        // F2P: far poses miss, so the clock runs with no hit-stop to disturb
        // the cancel assertions; the near pose lands the chained heavy.
        let tables = &TABLES_V2;
        let far = v2_far();
        let near = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut state = FightState::new(2).unwrap();
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        // Start a light swing far away.
        try_step_v2_into(
            &[input(0, ATTACK), none(1)],
            &far,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert_eq!(
            state.fighters[0].attack_kind, INPUT_LIGHT,
            "fresh light marks light"
        );
        // Run to the locked tick past feint (age 16).
        assert!(idle_v2(&mut state, &far, tables, 15).is_empty());
        assert_eq!(state.fighters[0].attack_age, 16);
        // Press heavy chord mid-swing: queues, never restarts at once.
        try_step_v2_into(
            &[input(0, ATTACK | BLOCK), none(1)],
            &far,
            &mut state,
            tables,
            &mut out,
        )
        .unwrap();
        assert!(state.fighters[0].is_attacking, "locked swing runs on");
        assert_eq!(
            state.fighters[0].attack_age, 17,
            "a locked press never restarts the clock at once"
        );
        assert_eq!(
            state.fighters[0].attack_buffer.len(),
            1,
            "heavy chord queues with its block bit"
        );
        assert!(out.is_empty(), "a queued press emits nothing");
        // Release block, wait for the chain window: age 17 still locked.
        try_step_v2_into(&idle_pair(), &far, &mut state, tables, &mut out).unwrap();
        assert_eq!(state.fighters[0].attack_age, 18);
        assert!(state.fighters[0].is_attacking);
        // Chain tick idle: the queued heavy fires as a new swing.
        try_step_v2_into(&idle_pair(), &far, &mut state, tables, &mut out).unwrap();
        assert_eq!(
            state.fighters[0].attack_age, 1,
            "queued tap chains once the recovery window opens"
        );
        assert!(state.fighters[0].attack_buffer.is_empty());
        assert_eq!(
            state.fighters[0].attack_kind, INPUT_HEAVY,
            "press-time kind survives the queue"
        );
        // Wired strike path: run the chained heavy to impact near.
        assert!(idle_v2(&mut state, &far, tables, 14).is_empty());
        assert_eq!(state.fighters[0].attack_age, 15);
        try_step_v2_into(&idle_pair(), &near, &mut state, tables, &mut out).unwrap();
        assert!(
            out.iter().any(|e| matches!(e, FightEvent::Hit { .. })),
            "chained heavy must hit near"
        );
        assert_eq!(state.hitstop, 6, "heavy hit freezes six ticks");
    }

    #[test]
    fn hitstop_impact_unifies_parry_and_kind() {
        use crate::depth::{INPUT_HEAVY, INPUT_LIGHT};
        // GP31 slice: one owner for the impact freeze. Parry wins over kind;
        // V1 keeps the single row so old tapes replay.
        assert_eq!(
            hitstop_for_impact(&TABLES_V1, INPUT_LIGHT, false),
            HITSTOP_TICKS
        );
        assert_eq!(
            hitstop_for_impact(&TABLES_V1, INPUT_HEAVY, false),
            HITSTOP_TICKS
        );
        assert_eq!(
            hitstop_for_impact(&TABLES_V1, INPUT_LIGHT, true),
            HITSTOP_TICKS
        );
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_LIGHT, false), 3);
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_HEAVY, false), 6);
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_LIGHT, true), 8);
        assert_eq!(hitstop_for_impact(&TABLES_V2, INPUT_HEAVY, true), 8);
        assert_eq!(hitstop_for_impact(&TABLES_V2, 9, false), 0);
    }

    #[test]
    fn parry_window_edges_match_table() {
        // Fresh block press on the impact tick parries (age 0 at resolve).
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        let events = tick(&mut state, &[none(0), input(1, BLOCK)]);
        assert!(events.contains(&FightEvent::Parried {
            attacker: 0,
            victim: 1
        }));
        // Held block resolving at the last open tick (age 4) still parries:
        // the latch ages 3 -> 4 before resolution.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        state.fighters[1].is_blocking = true;
        state.fighters[1].block_age = PARRY_WINDOW_TICKS - 1;
        let events = tick(&mut state, &[none(0), input(1, BLOCK)]);
        assert!(events.contains(&FightEvent::Parried {
            attacker: 0,
            victim: 1
        }));
        // One tick later (age 5) the same held block only chips.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        state.fighters[1].is_blocking = true;
        state.fighters[1].block_age = PARRY_WINDOW_TICKS;
        let events = tick(&mut state, &[none(0), input(1, BLOCK)]);
        assert!(events.iter().any(|e| matches!(
            e,
            FightEvent::Blocked { attacker: 0, victim: 1, chip } if *chip == block_chip(ATTACK_DAMAGE)
        )));
        assert_eq!(
            state.fighters[1].hp_permille,
            MAX_HP_PERMILLE - block_chip(ATTACK_DAMAGE)
        );
    }

    #[test]
    fn late_parry_fails_to_block_or_chip_per_table() {
        let mut state = FightState::new(2).unwrap();
        // Defender blocks long before the attack: held age 5 is past the window.
        state.fighters[1].is_blocking = true;
        state.fighters[1].block_age = PARRY_WINDOW_TICKS + 1;
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        let events = tick(&mut state, &[none(0), input(1, BLOCK)]);
        assert!(!events
            .iter()
            .any(|e| matches!(e, FightEvent::Parried { .. })));
        assert!(events.iter().any(|e| matches!(
            e,
            FightEvent::Blocked { chip, .. } if *chip == block_chip(ATTACK_DAMAGE)
        )));
        assert_eq!(
            state.fighters[1].hp_permille,
            MAX_HP_PERMILLE - block_chip(ATTACK_DAMAGE)
        );
    }

    #[test]
    fn dodge_iframes_negate_and_late_dodge_takes_hit() {
        // Inside i-frames: dodge age 0 negates.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        state.fighters[1].is_dodging = true;
        state.fighters[1].dodge_age = 0;
        let events = tick(&mut state, &idle_pair());
        assert!(events.contains(&FightEvent::Dodged {
            attacker: 0,
            victim: 1
        }));
        assert_eq!(state.fighters[1].hp_permille, MAX_HP_PERMILLE);
        // Past i-frames: same dodge takes the full hit.
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        state.fighters[1].is_dodging = true;
        state.fighters[1].dodge_age = DODGE_IFRAMES_TICKS;
        let events = tick(&mut state, &idle_pair());
        assert!(events.iter().any(|e| matches!(e, FightEvent::Hit { .. })));
    }

    #[test]
    fn stamina_exhaustion_blocks_attacks_until_regen() {
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].stamina = ATTACK_STAMINA_COST - 1;
        let events = tick(&mut state, &[input(0, ATTACK), none(1)]);
        assert!(events.contains(&FightEvent::StaminaOut { fighter: 0 }));
        assert!(!state.fighters[0].is_attacking);
        // Regen 2/tick: idle until affordable again, then the attack starts.
        let mut started = false;
        for _ in 0..64 {
            let events = tick(&mut state, &[input(0, ATTACK), none(1)]);
            if state.fighters[0].is_attacking {
                started = true;
                assert!(!events.contains(&FightEvent::StaminaOut { fighter: 0 }));
                break;
            }
        }
        assert!(started, "attack never restarted after regen");
    }

    /// The C5 spam drain: a fighter that presses ATTACK at a mash cadence
    /// drains its own bar to empty inside one 600-tick bout, and the presses
    /// the empty bar refuses report `StaminaOut`. Red before the regen gate:
    /// the 25-point cost landed every 13th tick while 2 regen per tick
    /// refilled 26, so the bar never left full and no cue ever fired.
    #[test]
    fn spam_cadence_drains_the_bar_and_reports_refusals() {
        // Three real cadences: a mash every tick, the staged frames tape
        // (attack every 5 ticks), and the staged binary tape (attack on
        // cycle 0 and 35, block otherwise with a dropped bit every fifth
        // tick). Each is pure over the tick.
        let cadences: [fn(u32) -> u8; 3] = [
            |_| ATTACK,
            |tick| {
                if tick % 5 == 0 {
                    ATTACK
                } else {
                    Buttons::NONE.0
                }
            },
            |tick| match tick % 50 {
                0 | 35 => ATTACK,
                _ if tick % 5 == 4 => Buttons::NONE.0,
                _ => BLOCK,
            },
        ];
        for (index, cadence) in cadences.iter().enumerate() {
            let mut state = FightState::new(2).unwrap();
            let mut outs = 0usize;
            let mut lowest = MAX_STAMINA;
            for at in 0..600u32 {
                let events = tick(&mut state, &[input(0, cadence(at)), none(1)]);
                outs += events
                    .iter()
                    .filter(|e| matches!(e, FightEvent::StaminaOut { fighter: 0 }))
                    .count();
                lowest = lowest.min(state.fighters[0].stamina);
            }
            assert!(
                outs > 0,
                "cadence {index} never refused a press (lowest stamina {lowest})"
            );
            assert_eq!(lowest, 0, "cadence {index} never emptied the bar");
        }
    }

    #[test]
    fn hit_stop_freezes_event_ticks() {
        let mut state = FightState::new(2).unwrap();
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        let tick_before = state.tick;
        let events = tick(&mut state, &idle_pair());
        assert!(events.iter().any(|e| matches!(e, FightEvent::Hit { .. })));
        assert_eq!(state.hitstop, HITSTOP_TICKS);
        let frozen_tick = state.tick;
        assert_eq!(frozen_tick, tick_before + 1);
        // While frozen: no events, no clock advance, no age advance.
        let attack_age = state.fighters[0].attack_age;
        for _ in 0..HITSTOP_TICKS {
            let events = tick(&mut state, &[input(0, ATTACK), input(1, ATTACK)]);
            assert!(events.is_empty());
            assert_eq!(state.tick, frozen_tick);
            assert_eq!(state.fighters[0].attack_age, attack_age);
        }
        // After the freeze the clock runs again.
        let events = tick(&mut state, &idle_pair());
        assert!(events.is_empty());
        assert_eq!(state.tick, frozen_tick + 1);
    }

    #[test]
    fn fatal_hit_knocks_down() {
        let mut state = FightState::new(2).unwrap();
        state.fighters[1].hp_permille = ATTACK_DAMAGE;
        state.fighters[0].is_attacking = true;
        state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
        let events = tick(&mut state, &idle_pair());
        assert!(events.iter().any(|e| matches!(
            e,
            FightEvent::Hit {
                attacker: 0,
                victim: 1,
                damage: ATTACK_DAMAGE
            }
        )));
        assert!(events.contains(&FightEvent::Knockdown {
            attacker: 0,
            victim: 1
        }));
        assert_eq!(state.fighters[1].hp_permille, 0);
    }

    #[test]
    fn determinism_same_script_twice_identical() {
        let script: Vec<Vec<FighterInput>> = vec![
            vec![input(0, ATTACK), none(1)],
            vec![none(0), input(1, BLOCK)],
            vec![input(0, ATTACK), input(1, BLOCK)],
            vec![none(0), input(1, DODGE)],
            vec![input(0, ATTACK), input(1, ATTACK)],
            vec![none(0), none(1)],
        ];
        let run = || {
            let mut state = FightState::new(2).unwrap();
            let mut all = Vec::new();
            for tick_inputs in &script {
                all.push(tick(&mut state, tick_inputs));
            }
            // Drain any trailing hit-stop deterministically.
            for _ in 0..HITSTOP_TICKS {
                all.push(tick(&mut state, &idle_pair()));
            }
            (all, state)
        };
        let (a_events, a_state) = run();
        let (b_events, b_state) = run();
        assert_eq!(a_events, b_events);
        assert_eq!(a_state, b_state);
    }

    #[test]
    fn ranges_validated_with_typed_errors_never_panic() {
        assert_eq!(FightState::new(0), Err(CombatError::BadFighterCount(0)));
        assert_eq!(
            FightState::new(MAX_PARTICIPANTS + 1),
            Err(CombatError::BadFighterCount(MAX_PARTICIPANTS + 1))
        );
        let mut state = FightState::new(2).unwrap();
        // Unknown button bits fail instead of panicking.
        let bad = FighterInput {
            tick: 0,
            fighter: FighterId::new(0).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(0x08),
        };
        let err = try_step(&[bad], &mut state).unwrap_err();
        assert!(matches!(err, CombatError::BadInput { index: 0, .. }));
        // Inactive slot fails.
        let err = try_step(&[input(3, ATTACK)], &mut state).unwrap_err();
        assert_eq!(
            err,
            CombatError::InactiveFighter {
                index: 0,
                fighter: 3
            }
        );
        // Duplicate slot fails.
        let err = try_step(&[none(0), none(0)], &mut state).unwrap_err();
        assert_eq!(
            err,
            CombatError::DuplicateInput {
                index: 1,
                fighter: 0
            }
        );
        // Lenient step skips all three without panicking.
        let before = state;
        let events = step(&[bad, input(3, ATTACK), none(0)], &mut state);
        assert!(!events
            .iter()
            .any(|e| matches!(e, FightEvent::StaminaOut { fighter: 3 })));
        // Only fighter 0's idle intent applied; state still advances sanely.
        assert!(state.tick == before.tick + 1 || state.hitstop < before.hitstop + 1);
    }

    #[test]
    fn juggle_budget_knocks_down_on_the_sixteenth_stun_hit() {
        let mut tables = TABLES_V1;
        tables.attack_damage = 0;
        let mut state = FightState::new(2).expect("two fighters");
        let mut events = Vec::new();
        for hit in 1..=15u8 {
            let guard = PreState::of(&state.fighters[1]);
            let parried = resolve_strike(
                &mut state,
                &tables,
                &mut events,
                Strike {
                    attacker: 0,
                    victim: 1,
                    kind: crate::depth::INPUT_LIGHT,
                    weapon: 0,
                    covered: false,
                    guard,
                    symmetric: false,
                },
            );
            assert!(!parried);
            assert_eq!(state.fighters[1].juggle, hit, "hit {hit} stays in stun");
            assert_eq!(state.fighters[1].knockdown, 0, "hit {hit} is not a fall");
        }
        let guard = PreState::of(&state.fighters[1]);
        resolve_strike(
            &mut state,
            &tables,
            &mut events,
            Strike {
                attacker: 0,
                victim: 1,
                kind: crate::depth::INPUT_LIGHT,
                weapon: 0,
                covered: false,
                guard,
                symmetric: false,
            },
        );
        assert_eq!(state.fighters[1].juggle, 0, "the fall resets the budget");
        assert_eq!(state.fighters[1].knockdown, tables.knockdown_ticks);
        assert!(matches!(
            events.last(),
            Some(FightEvent::Knockdown {
                attacker: 0,
                victim: 1
            })
        ));
    }

    #[test]
    fn juggle_budget_resets_when_hitstun_expires() {
        let mut state = FightState::new(2).expect("two fighters");
        state.fighters[1].react = 1;
        state.fighters[1].juggle = 4;
        step(&[none(0), none(1)], &mut state);
        assert_eq!(state.fighters[1].react, 0);
        assert_eq!(state.fighters[1].juggle, 0, "a clean stun break resets it");
    }

    #[test]
    fn juggle_budget_fires_before_retiarius_kills() {
        // Retiarius damage is half: 60 permille a hit, so the victim is at
        // 100 hp after hit 15 and the 16th hit knocks down instead of killing.
        let tables = crate::classes::tables(crate::classes::Class::Retiarius, TABLES_V2);
        assert_eq!(tables.attack_damage, 60, "the cap needs a surviving victim");
        let mut state = FightState::new(2).expect("two fighters");
        let mut events = Vec::new();
        for _ in 0..16 {
            let guard = PreState::of(&state.fighters[1]);
            resolve_strike(
                &mut state,
                &tables,
                &mut events,
                Strike {
                    attacker: 0,
                    victim: 1,
                    kind: crate::depth::INPUT_LIGHT,
                    weapon: 0,
                    covered: false,
                    guard,
                    symmetric: false,
                },
            );
        }
        assert_eq!(state.fighters[1].hp_permille, 40);
        assert_eq!(state.fighters[1].knockdown, tables.knockdown_ticks);
        assert_eq!(state.fighters[1].juggle, 0);
    }

    #[test]
    fn input_order_does_not_change_events() {
        let run = |order: &[FighterInput]| {
            let mut state = FightState::new(2).unwrap();
            state.fighters[0].is_attacking = true;
            state.fighters[0].attack_age = ATTACK_STARTUP_TICKS;
            state.fighters[1].is_blocking = true;
            state.fighters[1].block_age = PARRY_WINDOW_TICKS + 1;
            step(order, &mut state)
        };
        let a = run(&[none(0), input(1, BLOCK)]);
        let b = run(&[input(1, BLOCK), none(0)]);
        assert_eq!(a, b);
    }

    fn tape_hash(all: &[Vec<FightEvent>], state: &FightState) -> u64 {
        const FNV_OFF: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let mut h = FNV_OFF;
        let mut mix = |w: u64| {
            h ^= w;
            h = h.wrapping_mul(FNV_PRIME);
        };
        mix(u64::from(state.fighter_count));
        mix(u64::from(state.tick));
        mix(u64::from(state.hitstop));
        for f in state.fighters.iter().take(state.fighter_count as usize) {
            mix(u64::from(f.hp_permille));
            mix(u64::from(f.stamina));
            mix(u64::from(f.attack_age));
            mix(u64::from(f.is_attacking));
            mix(u64::from(f.block_age));
            mix(u64::from(f.is_blocking));
            mix(u64::from(f.dodge_age));
            mix(u64::from(f.is_dodging));
            mix(u64::from(f.react));
            mix(u64::from(f.knockdown));
            mix(u64::from(f.juggle));
        }
        for tick_events in all {
            for e in tick_events {
                match *e {
                    FightEvent::Hit {
                        attacker,
                        victim,
                        damage,
                    } => {
                        mix(1);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                        mix(u64::from(damage));
                    }
                    FightEvent::Blocked {
                        attacker,
                        victim,
                        chip,
                    } => {
                        mix(2);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                        mix(u64::from(chip));
                    }
                    FightEvent::Parried { attacker, victim } => {
                        mix(3);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                    }
                    FightEvent::Dodged { attacker, victim } => {
                        mix(4);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                    }
                    FightEvent::StaminaOut { fighter } => {
                        mix(5);
                        mix(u64::from(fighter));
                    }
                    FightEvent::Knockdown { attacker, victim } => {
                        mix(6);
                        mix(u64::from(attacker));
                        mix(u64::from(victim));
                    }
                }
            }
        }
        h
    }

    #[test]
    fn shipped_tape_hashes_identically_twice() {
        // The shipped V1 rows replay identically twice through the strict
        // entry the arena match steps (`try_step` over V1).
        let script: Vec<Vec<FighterInput>> = vec![
            vec![input(0, ATTACK), none(1), none(2), none(3)],
            vec![none(0), input(1, BLOCK), none(2), input(3, DODGE)],
            vec![input(0, ATTACK), input(1, BLOCK), none(2), none(3)],
            vec![none(0), input(1, BLOCK), input(2, ATTACK), input(3, ATTACK)],
            vec![
                input(0, ATTACK),
                input(1, ATTACK),
                input(2, ATTACK),
                input(3, ATTACK),
            ],
            vec![none(0), none(1), none(2), none(3)],
        ];
        let run = || {
            let mut state = FightState::new(4).unwrap();
            let mut all = Vec::new();
            for tick_inputs in &script {
                all.push(try_step(tick_inputs, &mut state).expect("script inputs valid"));
            }
            for _ in 0..TABLES_V1.hitstop_ticks {
                all.push(
                    try_step(&[none(0), none(1), none(2), none(3)], &mut state)
                        .expect("idle valid"),
                );
            }
            (all, state)
        };
        let (a_events, a_state) = run();
        let (b_events, b_state) = run();
        assert_eq!(
            tape_hash(&a_events, &a_state),
            tape_hash(&b_events, &b_state)
        );
        assert!(a_events.iter().any(|tick| !tick.is_empty()));
    }

    #[test]
    fn slots_follow_canonical_cap_beyond_four() {
        // Generic slots: slot 4 (past the old 4-cap) fights under V1,
        // sixteen validate, seventeen refuse (canonical cap, no private cap).
        assert_eq!(
            FightState::new(MAX_PARTICIPANTS).unwrap().fighter_count,
            MAX_PARTICIPANTS
        );
        let run = || {
            let mut state = FightState::new(5).unwrap();
            state.fighters[4].is_attacking = true;
            state.fighters[4].attack_age = TABLES_V1.attack_startup_ticks;
            let idle: Vec<FighterInput> = (0..5).map(none).collect();
            let events = step(&idle, &mut state);
            (events, state)
        };
        let (a_events, a_state) = run();
        let (b_events, b_state) = run();
        // One attacker meets four victims: four hits in victim-ID order.
        assert_eq!(a_events.len(), 4);
        for (i, e) in a_events.iter().enumerate() {
            assert_eq!(
                *e,
                FightEvent::Hit {
                    attacker: 4,
                    victim: u8::try_from(i).unwrap_or(u8::MAX),
                    damage: TABLES_V1.attack_damage,
                }
            );
        }
        assert_eq!(a_events, b_events);
        assert_eq!(a_state, b_state);
        assert_eq!(a_state.hitstop, TABLES_V1.hitstop_ticks);
    }

    #[test]
    fn fixture_table_set_resolves_without_engine_edits() {
        // Tables own tuning: a versioned fixture set with different numbers
        // resolves through the same core, no engine edits.
        use crate::tables::{validate_tables, MoveTableSet};
        let mut rows = TABLES_V1;
        rows.attack_damage = 200;
        rows.attack_startup_ticks = 6;
        rows.parry_window_ticks = 2;
        rows.attack_stamina_cost = 10;
        let set = MoveTableSet::new(2, rows);
        assert_eq!(validate_tables(&set), Ok(()));
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
            let drain = (tables.attack_startup_ticks + tables.hitstop_ticks + 2) as usize;
            for _ in 0..drain {
                all.push(step_with_tables(&[none(0), none(1)], &mut state, tables));
            }
            (all, state)
        };
        let (v1_events, v1_state) = bout(&TABLES_V1);
        let (v2_events, v2_state) = bout(&rows);
        let hit_damage = |all: &[Vec<FightEvent>]| {
            all.iter().flatten().find_map(|e| match *e {
                FightEvent::Hit { damage, .. } => Some(damage),
                _ => None,
            })
        };
        assert_eq!(hit_damage(&v1_events), Some(TABLES_V1.attack_damage));
        assert_eq!(hit_damage(&v2_events), Some(200));
        assert_eq!(
            v1_state.fighters[1].hp_permille,
            MAX_HP_PERMILLE - TABLES_V1.attack_damage
        );
        assert_eq!(v2_state.fighters[1].hp_permille, MAX_HP_PERMILLE - 200);
        assert_ne!(v1_state, v2_state);
        // Each row replays identically twice.
        let (a_events, a_state) = bout(&rows);
        assert_eq!(
            tape_hash(&v2_events, &v2_state),
            tape_hash(&a_events, &a_state)
        );
        let (b_events, b_state) = bout(&TABLES_V1);
        assert_eq!(
            tape_hash(&v1_events, &v1_state),
            tape_hash(&b_events, &b_state)
        );
    }

    /// Four-fighter all-attack row: every slot presses attack when idle, so
    /// impacts, blocks, dodges, stamina-outs and knockdowns all fire across
    /// the bout. Shared by the parity and allocation tests below.
    fn brawl_inputs() -> [FighterInput; 4] {
        [
            input(0, ATTACK),
            input(1, ATTACK),
            input(2, ATTACK),
            input(3, ATTACK),
        ]
    }

    /// One-word fold of an event for allocation-free comparison inside the
    /// counting window (retaining `Vec`s there would allocate and fake the
    /// reading).
    fn event_word(event: FightEvent) -> u64 {
        const PRIME: u64 = 0x100000001b3;
        let (tag, a, b, c) = match event {
            FightEvent::Hit {
                attacker,
                victim,
                damage,
            } => (
                1u64,
                u64::from(attacker),
                u64::from(victim),
                u64::from(damage),
            ),
            FightEvent::Blocked {
                attacker,
                victim,
                chip,
            } => (
                2u64,
                u64::from(attacker),
                u64::from(victim),
                u64::from(chip),
            ),
            FightEvent::Parried { attacker, victim } => {
                (3u64, u64::from(attacker), u64::from(victim), 0)
            }
            FightEvent::Dodged { attacker, victim } => {
                (4u64, u64::from(attacker), u64::from(victim), 0)
            }
            FightEvent::StaminaOut { fighter } => (5u64, u64::from(fighter), 0, 0),
            FightEvent::Knockdown { attacker, victim } => {
                (6u64, u64::from(attacker), u64::from(victim), 0)
            }
        };
        tag.wrapping_mul(PRIME)
            .wrapping_add(a)
            .wrapping_mul(PRIME)
            .wrapping_add(b)
            .wrapping_mul(PRIME)
            .wrapping_add(c)
    }

    #[test]
    fn into_matches_owned_across_event_bout() {
        // SL20: the reused buffer is the same events the owned entries
        // return, tick by tick, across hits, hit-stop freezes, blocks,
        // parries, dodges, stamina-outs and knockdowns — and the end state
        // is identical, so the golden cannot move.
        let mut owned_state = FightState::new(4).unwrap();
        let mut into_state = owned_state;
        let mut buf: Vec<FightEvent> = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut all_owned = Vec::new();
        let mut all_into = Vec::new();
        // Seed one mid-bout parry: fighter 1 holds block from the start.
        let mut held = brawl_inputs();
        held[1] = input(1, BLOCK);
        for tick in 0..48 {
            let row: &[FighterInput] = if tick < 8 { &held } else { &brawl_inputs() };
            let owned = try_step(row, &mut owned_state).expect("brawl inputs valid");
            try_step_into(row, &mut into_state, &mut buf).expect("brawl inputs valid");
            assert_eq!(buf, owned, "tick {tick}: into diverges from owned");
            assert_eq!(into_state, owned_state, "tick {tick}: state diverges");
            all_owned.push(owned);
            all_into.push(buf.clone());
        }
        assert!(
            all_owned.iter().any(|tick| !tick.is_empty()),
            "bout must emit events or the parity proves nothing"
        );
        assert_eq!(all_into, all_owned);
        assert_eq!(
            tape_hash(&all_into, &into_state),
            tape_hash(&all_owned, &owned_state)
        );
        // The bound holds with room: the whole 4-fighter bout peaks far
        // below the 32-fighter worst case the const covers.
        let event_total: usize = all_into.iter().map(Vec::len).sum();
        assert!(
            event_total > 0,
            "bout must emit events or the parity proves nothing"
        );
        assert!(all_into
            .iter()
            .all(|tick| tick.len() <= MAX_EVENTS_PER_TICK));
        assert_eq!(buf.capacity(), MAX_EVENTS_PER_TICK);
        // Lenient twins agree on hostile input order (dupes, bad slots,
        // unknown buttons): first-valid-per-fighter wins in both.
        let hostile = [
            input(1, BLOCK),
            input(0, ATTACK),
            input(0, DODGE),
            input(9, ATTACK),
            FighterInput {
                tick: 0,
                fighter: FighterId::new(2).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(0x08),
            },
        ];
        let mut a = FightState::new(4).unwrap();
        let mut b = a;
        let owned = step(&hostile, &mut a);
        let mut into_buf = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        step_into(&hostile, &mut b, &mut into_buf);
        assert_eq!(into_buf, owned);
        assert_eq!(a, b);
        // Tables twins agree under a custom row.
        let mut rows = TABLES_V1;
        rows.attack_damage = 200;
        rows.attack_startup_ticks = 6;
        let mut a = FightState::new(2).unwrap();
        let mut b = a;
        let owned = try_step_with_tables(&brawl_inputs()[..2], &mut a, &rows).expect("valid pair");
        let mut into_buf = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        try_step_with_tables_into(&brawl_inputs()[..2], &mut b, &rows, &mut into_buf)
            .expect("valid pair");
        assert_eq!(into_buf, owned);
        assert_eq!(a, b);
        let owned = step_with_tables(&brawl_inputs()[..2], &mut a, &rows);
        step_with_tables_into(&brawl_inputs()[..2], &mut b, &rows, &mut into_buf);
        assert_eq!(into_buf, owned);
        assert_eq!(a, b);
    }

    #[test]
    fn into_rejects_like_owned_leaving_no_stale_events() {
        // SL20 failure case: every validation error the owned entry reports,
        // the into entry reports identically — with state untouched and the
        // reused buffer empty, never carrying the previous tick's events.
        let bad_button = FighterInput {
            tick: 0,
            fighter: FighterId::new(0).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(0x08),
        };
        let cases: Vec<Vec<FighterInput>> = vec![
            vec![bad_button],
            vec![input(3, ATTACK)],
            vec![none(0), none(0)],
            vec![none(0); MAX_INPUTS_PER_TICK + 1],
        ];
        for (case_index, inputs) in cases.iter().enumerate() {
            let mut owned_state = FightState::new(2).unwrap();
            let mut into_state = owned_state;
            let expected = try_step(inputs, &mut owned_state).unwrap_err();
            // Stale sentinel: a rejected tick must not leak it to the reader.
            let mut buf = vec![
                FightEvent::Hit {
                    attacker: 7,
                    victim: 7,
                    damage: 999,
                };
                3
            ];
            let err = try_step_into(inputs, &mut into_state, &mut buf).unwrap_err();
            assert_eq!(err, expected, "case {case_index}: error diverges");
            assert_eq!(into_state, owned_state, "case {case_index}: state moved");
            assert!(buf.is_empty(), "case {case_index}: stale events survive");
        }
        // A zero fighter count (unbuildable via `new`, reachable through a
        // restored snapshot) rejects the same way in both entries.
        let mut zero = FightState::new(2).unwrap();
        zero.fighter_count = 0;
        let mut zero_owned = zero;
        let mut zero_into = zero;
        let expected = try_step(&[], &mut zero_owned).unwrap_err();
        assert_eq!(expected, CombatError::BadFighterCount(0));
        let mut buf = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let err = try_step_into(&[], &mut zero_into, &mut buf).unwrap_err();
        assert_eq!(err, expected);
        assert_eq!(zero_into, zero_owned);
    }

    #[test]
    fn quiet_ticks_allocate_nothing_even_with_fresh_vec() {
        // SL20 floor: quiet ticks already read 0 — a fresh owned `Vec` per
        // tick allocates nothing when no event fires, so reuse only removes
        // the event share.
        let config_states = [2, 4];
        for fighters in config_states {
            let mut state = FightState::new(fighters).unwrap();
            let idle: Vec<FighterInput> = (0..fighters).map(none).collect();
            // Warmup outside the window (first-tick paths excluded).
            for _ in 0..10 {
                let _ = try_step(&idle, &mut state).expect("idle valid");
            }
            let ((), heap) = measure(|| {
                for _ in 0..600 {
                    let events = try_step(&idle, &mut state).expect("idle valid");
                    assert!(events.is_empty());
                    std::hint::black_box(&events);
                }
            });
            assert_eq!(
                heap.calls, 0,
                "{fighters}f quiet ticks must allocate nothing"
            );
        }
    }

    #[test]
    fn event_ticks_reuse_zero_alloc() {
        // SL20 gate: 600 ticks where every tick emits events (all four
        // fighters press attack with empty stamina, so each tick reports
        // four `StaminaOut`s and the clock keeps advancing). The owned
        // entries allocate once per event tick (the packet's event share),
        // while a buffer reserved once to MAX_EVENTS_PER_TICK allocates
        // nothing and returns identical events and end state. Live impact
        // paths (hits, blocks, parries, knockdowns, hit-stop) are covered
        // tick-by-tick in `into_matches_owned_across_event_bout`.
        fn starved() -> FightState {
            let mut state = FightState::new(4).unwrap();
            for f in state.fighters.iter_mut().take(4) {
                f.stamina = 0;
            }
            state
        }
        let run_owned = || {
            let mut state = starved();
            let mut total = 0usize;
            for _ in 0..600 {
                let events = try_step(&brawl_inputs(), &mut state).expect("brawl valid");
                total += events.len();
                std::hint::black_box(&events);
            }
            (total, state)
        };
        let ((fresh_total, _), fresh_heap) = measure(|| std::hint::black_box(run_owned()));
        assert!(
            fresh_total > 0,
            "bout must emit events or the gate is blind"
        );
        assert!(
            fresh_heap.calls > 0,
            "bout must allocate through owned entries or the gate is blind"
        );
        let mut buf: Vec<FightEvent> = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut state = starved();
        // Warmup outside the window, then restart from the same starved
        // state the owned comparison below uses.
        for _ in 0..12 {
            try_step_into(&brawl_inputs(), &mut state, &mut buf).expect("brawl valid");
        }
        state = starved();
        // Per-tick (length, event hash) retained in reserved `Vec`s: pushing
        // into reserved capacity allocates nothing, while cloning the event
        // `Vec`s here would fake the reading.
        let mut lens: Vec<usize> = Vec::with_capacity(600);
        let mut hashes: Vec<u64> = Vec::with_capacity(600);
        let ((reuse_state, event_total), reuse_heap) = measure(|| {
            let mut total = 0usize;
            for _ in 0..600 {
                try_step_into(&brawl_inputs(), &mut state, &mut buf).expect("brawl valid");
                assert!(buf.len() <= MAX_EVENTS_PER_TICK, "event bound blown");
                total += buf.len();
                let mut tick_hash = 0xcbf29ce484222325u64;
                for event in buf.iter() {
                    tick_hash ^= event_word(*event);
                    tick_hash = tick_hash.wrapping_mul(0x100000001b3);
                }
                lens.push(buf.len());
                hashes.push(tick_hash);
                std::hint::black_box(buf.len());
            }
            (state, total)
        });
        assert_eq!(
            reuse_heap.calls, 0,
            "reused buffer must allocate nothing on event ticks"
        );
        // Identical inputs give identical per-tick events and end state
        // either way.
        let mut owned_state = starved();
        let mut owned_total = 0usize;
        for (tick, (lens_tick, hash_tick)) in lens.iter().zip(hashes.iter()).enumerate() {
            let owned = try_step(&brawl_inputs(), &mut owned_state).expect("brawl valid");
            owned_total += owned.len();
            assert_eq!(&owned.len(), lens_tick, "tick {tick}: length diverges");
            let mut tick_hash = 0xcbf29ce484222325u64;
            for event in owned.iter() {
                tick_hash ^= event_word(*event);
                tick_hash = tick_hash.wrapping_mul(0x100000001b3);
            }
            assert_eq!(&tick_hash, hash_tick, "tick {tick}: events diverge");
        }
        assert_eq!(owned_state, reuse_state);
        assert_eq!(owned_total, event_total);
        assert_eq!(event_total, fresh_total);
        assert!(
            event_total > 0,
            "bout must emit events or the gate is blind"
        );
    }

    #[test]
    fn attack_started_v2_press_emits_one_on_press_tick() {
        // EG19b: a press emits exactly one cue on its tick under V2 rows.
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK, TABLES_V2};
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut state = FightState::new(2).unwrap();
        let mut events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            &TABLES_V2,
            &mut events,
            &mut cues,
        )
        .expect("press valid");
        assert_eq!(cues, vec![AttackStarted { attacker: 0 }]);
        assert!(events.is_empty(), "press tick carries no impact yet");
        assert!(cues.len() <= MAX_ATTACK_STARTED_PER_TICK);
    }

    #[test]
    fn attack_started_v1_rows_emit_none() {
        // EG19b: the V1 stream stays frozen — same press under V1 rows cues nothing.
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK, TABLES_V1};
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut state = FightState::new(2).unwrap();
        let mut events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            &TABLES_V1,
            &mut events,
            &mut cues,
        )
        .expect("press valid");
        assert!(cues.is_empty(), "V1 rows must cue nothing");
    }

    #[test]
    fn attack_started_second_press_while_attacking_emits_none() {
        // EG19b failure case: holding attack through startup cues once only.
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK, TABLES_V2};
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut state = FightState::new(2).unwrap();
        let mut events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            &TABLES_V2,
            &mut events,
            &mut cues,
        )
        .expect("first press valid");
        assert_eq!(cues.len(), 1);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), none(1)],
            &poses,
            &mut state,
            &TABLES_V2,
            &mut events,
            &mut cues,
        )
        .expect("second press valid");
        assert!(cues.is_empty(), "in-flight attack must not cue twice");
    }

    #[test]
    fn attack_started_two_presses_leave_in_fighter_id_order() {
        // EG19b: simultaneous presses cue once per starter in slot order.
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK, TABLES_V2};
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut state = FightState::new(2).unwrap();
        let mut events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
        try_step_v2_with_cues_into(
            &[input(0, ATTACK), input(1, ATTACK)],
            &poses,
            &mut state,
            &TABLES_V2,
            &mut events,
            &mut cues,
        )
        .expect("presses valid");
        assert_eq!(
            cues,
            vec![AttackStarted { attacker: 0 }, AttackStarted { attacker: 1 }]
        );
        assert!(events.is_empty(), "press tick carries no impact yet");
        assert!(cues.len() <= MAX_ATTACK_STARTED_PER_TICK);
    }

    #[test]
    fn attack_started_rejected_tick_leaves_cues_empty() {
        // EG19b failure case: a rejected tick clears cues and touches no state.
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK, TABLES_V2};
        let poses = [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
        ];
        let mut state = FightState::new(2).unwrap();
        let before = state;
        let mut events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
        let err = try_step_v2_with_cues_into(
            &[input(0, ATTACK), input(0, ATTACK)],
            &poses,
            &mut state,
            &TABLES_V2,
            &mut events,
            &mut cues,
        )
        .expect_err("duplicate input must reject");
        assert!(matches!(
            err,
            CombatError::DuplicateInput {
                index: 1,
                fighter: 0
            }
        ));
        assert!(cues.is_empty(), "rejected tick leaves cues empty");
        assert!(events.is_empty(), "rejected tick leaves events empty");
        assert_eq!(state, before, "rejected tick touches no state");
    }

    #[test]
    fn attack_started_cues_leave_event_stream_identical() {
        // EG19b: cues are presentation-only; the event stream and state stay
        // byte-identical with cues on or off, under both row sets.
        use super::{AttackStarted, MAX_ATTACK_STARTED_PER_TICK, TABLES_V1, TABLES_V2};
        for tables in [TABLES_V1, TABLES_V2] {
            let poses = [
                pose(0, 0, 0, (1000, 0), 0),
                pose(1, 1_000, 0, (-1000, 0), 1),
            ];
            let mut plain = FightState::new(2).unwrap();
            let mut cued = FightState::new(2).unwrap();
            let mut plain_out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
            let mut cued_out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
            let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
            for t in 0..=tables.attack_startup_ticks + 1 {
                let attack = if t == 0 { ATTACK } else { 0 };
                let inputs = [input(0, attack), none(1)];
                try_step_v2_into(&inputs, &poses, &mut plain, &tables, &mut plain_out)
                    .expect("plain step valid");
                try_step_v2_with_cues_into(
                    &inputs,
                    &poses,
                    &mut cued,
                    &tables,
                    &mut cued_out,
                    &mut cues,
                )
                .expect("cued step valid");
                assert_eq!(cued_out, plain_out, "cue collection must not move events");
                assert_eq!(cued, plain, "cue collection must not move state");
                if t == 0 {
                    if tables == TABLES_V2 {
                        assert_eq!(cues, vec![AttackStarted { attacker: 0 }]);
                    } else {
                        assert!(cues.is_empty(), "V1 rows cue nothing");
                    }
                }
            }
        }
    }

    #[test]
    fn slot_id_byte_matches_slot_and_rejects_overflow() {
        // Ratchet pin: slot indexes reach events and cues as id bytes
        // without truncation; an out-of-range index converts to nothing
        // instead of forging another fighter's id.
        assert_eq!(slot_id_u8(0), Some(0));
        assert_eq!(
            slot_id_u8(usize::from(MAX_PARTICIPANTS) - 1),
            Some(MAX_PARTICIPANTS - 1)
        );
        assert_eq!(slot_id_u8(usize::from(u8::MAX) + 1), None);
    }

    /// Input buffer at intake, v2 only: a tap 5 ticks before recovery ends
    /// is kept and fires on the first ready tick (the promise, not a drop).
    #[test]
    fn input_buffer_keeps_press_inside_window_under_v2() {
        // Single fighter, no victim: no impact, no hit-stop, no feint, so the
        // clock runs its full startup plus recovery and the intake alone decides.
        let tables = &TABLES_V2;
        let total = tables.attack_startup_ticks + tables.attack_recovery_ticks + 1;
        let press_at = total - 5;
        let run = || {
            let mut state = FightState::new(1).unwrap();
            let mut clock: u32 = 0;
            while clock <= total {
                let attack = if clock == 0 || clock == press_at {
                    ATTACK
                } else {
                    0
                };
                let _ = step_with_tables(&[input(0, attack)], &mut state, tables);
                clock += 1;
            }
            state
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "buffered bout replays exactly");
        assert!(
            first.fighters[0].is_attacking,
            "press 5 before ready must fire on the first ready tick"
        );
        assert_eq!(
            first.fighters[0].attack_age, 1,
            "fired swing advances one tick on the ready tick"
        );
    }

    /// Input buffer at intake, v2 only: a tap 12 ticks before recovery ends
    /// is stale and dropped, so the fighter rests on the first ready tick.
    #[test]
    fn input_buffer_drops_press_outside_window_under_v2() {
        let tables = &TABLES_V2;
        let total = tables.attack_startup_ticks + tables.attack_recovery_ticks + 1;
        let press_at = total - 12;
        let run = || {
            let mut state = FightState::new(1).unwrap();
            let mut clock: u32 = 0;
            while clock <= total {
                let attack = if clock == 0 || clock == press_at {
                    ATTACK
                } else {
                    0
                };
                let _ = step_with_tables(&[input(0, attack)], &mut state, tables);
                clock += 1;
            }
            state
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "stale bout replays exactly");
        assert!(
            !first.fighters[0].is_attacking,
            "press 12 before ready must be dropped"
        );
        assert_eq!(first.fighters[0].attack_age, 0);
    }

    /// Input buffer at intake, v1 only: both the 5-inside and the 12-outside
    /// taps are dropped as today, so v1 bouts stay byte-identical.
    #[test]
    fn input_buffer_drops_all_presses_under_v1() {
        let total = TABLES_V1.attack_startup_ticks + TABLES_V1.attack_recovery_ticks + 1;
        for press_at in [total - 5, total - 12] {
            let run = || {
                let mut state = FightState::new(1).unwrap();
                let mut clock: u32 = 0;
                while clock <= total {
                    let attack = if clock == 0 || clock == press_at {
                        ATTACK
                    } else {
                        0
                    };
                    let _ = tick(&mut state, &[input(0, attack)]);
                    clock += 1;
                }
                state
            };
            let first = run();
            let second = run();
            assert_eq!(first, second, "v1 bout replays exactly");
            assert!(
                !first.fighters[0].is_attacking,
                "v1 must drop a mid-recovery press"
            );
            assert_eq!(first.fighters[0].attack_age, 0);
        }
    }

    /// The rules-v2 impact resolver as it stood before the compact snapshot:
    /// it cloned the whole fighter array every tick and handed whole
    /// [`FighterState`] guards to [`resolve_strike`]. Kept as the equality
    /// oracle for [`resolve_impacts`]; it converts the cloned guard through
    /// [`PreState::of`] only because the shared strike step reads the compact
    /// view, so both paths read exactly the same fields.
    fn resolve_impacts_full_clone(
        state: &mut FightState,
        tables: &Tables,
        events: &mut Vec<FightEvent>,
        count: usize,
        geometry: Option<&[StrikePose]>,
    ) {
        for id in 0..count {
            let owner_stunned = {
                let f = state.fighters[id];
                f.hp_permille == 0 || f.react > 0 || f.knockdown > 0
            };
            if owner_stunned && state.fighters[id].is_attacking {
                state.fighters[id].is_attacking = false;
                state.fighters[id].attack_age = 0;
            }
        }
        let pre: Option<[FighterState; SLOT_COUNT]> = geometry.map(|_| state.fighters);
        for attacker in 0..count {
            let f: FighterState = pre.map_or(state.fighters[attacker], |p| p[attacker]);
            let (startup, _recovery) = kind_timing(tables, &f);
            let impact = f.is_attacking
                && f.attack_age == startup
                && f.hp_permille > 0
                && f.react == 0
                && f.knockdown == 0;
            if !impact {
                continue;
            }
            let reach = strike_reach(f.weapon, f.attack_kind);
            for victim in 0..count {
                if victim == attacker {
                    continue;
                }
                let guard: FighterState = pre.map_or(state.fighters[victim], |p| p[victim]);
                if guard.hp_permille == 0 {
                    continue;
                }
                let covered =
                    match geometry.map(|poses| strike_gate(poses, attacker, victim, reach)) {
                        None | Some(StrikeGate::Covered) => true,
                        Some(StrikeGate::Open) => false,
                        Some(StrikeGate::Miss) => continue,
                    };
                let strike = Strike {
                    attacker,
                    victim,
                    kind: f.attack_kind,
                    weapon: f.weapon,
                    covered,
                    guard: PreState::of(&guard),
                    symmetric: pre.is_some(),
                };
                if resolve_strike(state, tables, events, strike) {
                    break;
                }
            }
        }
    }

    /// [`core_step_into`] with the pre-snapshot resolver: the whole-tick
    /// equality oracle, identical in every other phase.
    fn core_step_into_full_clone(
        intents: &[Buttons; SLOT_COUNT],
        state: &mut FightState,
        tables: &Tables,
        geometry: Option<&[StrikePose]>,
        events: &mut Vec<FightEvent>,
        mut cues: Option<&mut Vec<AttackStarted>>,
    ) {
        events.clear();
        if let Some(cues) = cues.as_mut() {
            cues.clear();
        }
        let count = usize::from(state.fighter_count.min(MAX_PARTICIPANTS));
        // The balance channel carries one tick: every fighter starts the tick
        // with no impulse, so the arena reads one push per tick.
        for id in 0..count {
            state.fighters[id].balance_impulse_q8 = 0;
        }
        if state.hitstop > 0 {
            state.hitstop -= 1;
            return;
        }
        let commit = geometry.map(|_| StreamliningParams::v1());
        latch_block_holds(intents, state, tables, commit, count);
        start_actions(intents, state, tables, events, cues, commit, count);
        if *tables == TABLES_V2 {
            for (id, intent) in intents.iter().enumerate().take(count) {
                let f = &mut state.fighters[id];
                if f.is_attacking && f.attack_age == 0 && attacking(*intent) && !dodging(*intent) {
                    f.attack_kind = kind_for_buttons(intent.0, f.weapon);
                }
            }
        }
        resolve_impacts_full_clone(state, tables, events, count, geometry);
        advance_clocks(state, tables, count);
        state.tick = state.tick.saturating_add(1);
    }

    /// Four-duelist poses with drifting reach, two teams, so the tape reaches
    /// teammate misses, in-reach hits, blocks, parries and dodges.
    fn brawl4_poses(t: u32) -> [StrikePose; 4] {
        let reach = i32::try_from(t % 4).unwrap() * 900 + 400;
        [
            pose(0, 0, 0, (1000, 0), 0),
            pose(1, 1_000, 0, (-1000, 0), 1),
            pose(2, 4_000, 0, (0, 1000), 2),
            pose(3, 4_000, reach, (0, -1000), 2),
        ]
    }

    /// One tick of scripted presses over four duellists.
    fn brawl4_inputs(t: u32) -> [FighterInput; 4] {
        let mut v = [none(0), none(1), none(2), none(3)];
        if t.is_multiple_of(24) {
            v[0] = input(0, ATTACK);
        }
        if t % 24 == 4 {
            v[1] = input(1, BLOCK);
        }
        if t % 30 == 7 {
            v[2] = input(2, ATTACK);
        }
        if t % 30 == 12 {
            v[3] = input(3, BLOCK);
        }
        if t % 37 == 3 {
            v[1] = input(1, DODGE);
        }
        if t % 41 == 9 {
            v[0] = input(0, ATTACK | BLOCK);
        }
        v
    }

    #[test]
    fn compact_snapshot_matches_the_full_clone_across_a_v2_brawl() {
        let tables = &TABLES_V2;
        // Rules v2 (geometry on) and rules v1 (geometry off) both run the
        // same tape; events and end state must match the pre-snapshot path.
        for geometry_on in [true, false] {
            let mut new_state = FightState::new(4).unwrap();
            let mut old_state = new_state;
            let mut new_events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
            let mut old_events = Vec::with_capacity(MAX_EVENTS_PER_TICK);
            let mut total = 0usize;
            for t in 0..240u32 {
                let poses = brawl4_poses(t);
                let inputs = brawl4_inputs(t);
                let intents = v2_intents(&inputs, &poses, 4).unwrap();
                let geometry = if geometry_on { Some(&poses[..]) } else { None };
                core_step_into(
                    &intents,
                    &mut new_state,
                    tables,
                    geometry,
                    &mut new_events,
                    None,
                );
                core_step_into_full_clone(
                    &intents,
                    &mut old_state,
                    tables,
                    geometry,
                    &mut old_events,
                    None,
                );
                assert_eq!(
                    new_events, old_events,
                    "events differ at tick {t} (geometry {geometry_on})"
                );
                assert_eq!(
                    new_state, old_state,
                    "state differs at tick {t} (geometry {geometry_on})"
                );
                total += old_events.len();
            }
            assert_eq!(new_state, old_state, "end state (geometry {geometry_on})");
            // The tape really resolves contacts, so the oracle is not vacuous.
            assert!(total > 0, "the brawl tape emitted no events");
        }
    }

    /// Timing receipt: p50 microseconds of one [`resolve_impacts`] over a
    /// 16-fighter fixture, the compact pre-snapshot against the full-clone
    /// oracle it replaced, interleaved so one load sits under both sides. Two
    /// fixtures: an in-reach brawl (geometry dominates) and a teammate bout
    /// (geometry skipped, so the snapshot and the reads show through). The
    /// compact side stays only while the pair holds and it wins.
    /// Run: cargo test -p engine-combat --release --lib compact_snapshot_paired -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn compact_snapshot_paired_receipt() {
        const SAMPLES: usize = 201;
        const BATCH: usize = 64;
        const COUNT: u8 = 16;
        let count = usize::from(COUNT);
        let batch = f64::from(u32::try_from(BATCH).unwrap());
        let tables = &TABLES_V2;
        let mut brawl = [pose(0, 0, 0, (1000, 0), 0); 16];
        for (i, p) in brawl.iter_mut().enumerate() {
            *p = pose(
                u8::try_from(i).unwrap(),
                i32::try_from(i).unwrap() * 1_000,
                0,
                (1000, 0),
                u8::try_from(i % 2).unwrap(),
            );
        }
        // All one team: every pair is a one-compare Miss, isolating the
        // per-tick snapshot and the per-slot reads from the geometry.
        let mates = [pose(0, 0, 0, (1000, 0), 0); 16];
        let mut template = FightState::new(COUNT).unwrap();
        template.fighters[0].is_attacking = true;
        template.fighters[0].attack_age = tables.attack_startup_ticks;

        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        for (name, poses) in [("brawl", &brawl[..]), ("mates", &mates[..])] {
            let mut new_us = Vec::with_capacity(SAMPLES);
            let mut old_us = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let begin = std::time::Instant::now();
                for _ in 0..BATCH {
                    let mut s = template;
                    resolve_impacts(&mut s, tables, &mut out, count, Some(poses));
                }
                new_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / batch);
                let begin = std::time::Instant::now();
                for _ in 0..BATCH {
                    let mut s = template;
                    resolve_impacts_full_clone(&mut s, tables, &mut out, count, Some(poses));
                }
                old_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / batch);
            }
            new_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
            old_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let (new_p50, old_p50) = (new_us[SAMPLES / 2], old_us[SAMPLES / 2]);
            eprintln!(
                "resolve_impacts_snapshot fixture={name} full_p50_us={old_p50:.4} compact_p50_us={new_p50:.4} ratio_x={:.3} count={count}",
                old_p50 / new_p50
            );
            assert!(
                new_p50 <= old_p50,
                "the compact snapshot must not be the slower one on {name}: {new_p50:.4} vs {old_p50:.4}"
            );
        }
    }

    // ---- the shield line ----------------------------------------------------

    /// One row of `content/design/weapons.rows` as the file test reads it.
    struct WeaponFileRow {
        /// Row id, e.g. `gladius`.
        id: String,
        /// The strike kind: 1 thrust, 2 cut, 3 throw.
        kind: u32,
        /// Damage of one landed strike, in permille.
        damage_permille: u32,
        /// Hands to wield: 1 or 2.
        hands: u32,
    }

    /// The design file's weapon rows: the id, the strike kind, the damage
    /// and the hands. Whole numbers only; a bad line panics with its number.
    fn weapon_file_rows() -> Vec<WeaponFileRow> {
        let text = include_str!("../../../content/design/weapons.rows");
        let mut rows = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with("table ") {
                continue;
            }
            let mut fields = line.split_whitespace();
            let id = fields.next().unwrap_or("").to_string();
            let mut kind = 0;
            let mut damage_permille = 0;
            let mut hands = 0;
            for field in fields {
                let (key, value) = field.split_once('=').unwrap_or(("", ""));
                let number: u32 = value.parse().unwrap_or(0);
                match key {
                    "type" => kind = number,
                    "damage_permille" => damage_permille = number,
                    "hands" => hands = number,
                    _ => {}
                }
            }
            assert!(kind > 0, "line {}: the row needs a type", index + 1);
            rows.push(WeaponFileRow {
                id,
                kind,
                damage_permille,
                hands,
            });
        }
        rows
    }

    /// A rules-v2 bout with the attacker's weapon row set: the attacker
    /// presses the heavy chord on tick 0 and the foe holds his guard.
    /// Returns the end state, the events and the balance impulse the foe
    /// carried after each tick.
    fn shield_line_push(poses: &[StrikePose; 2]) -> (FightState, Vec<FightEvent>, Vec<u16>) {
        let mut state = FightState::new(2).expect("two fighters fit");
        state.fighters[0].weapon = 2;
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        let mut all = Vec::new();
        let mut impulses = Vec::new();
        for tick in 0..=TABLES_V2.attack_startup_ticks {
            let press = if tick == 0 { ATTACK | BLOCK } else { BLOCK };
            let inputs = [input(0, press), input(1, BLOCK)];
            try_step_v2_into(&inputs, poses, &mut state, &TABLES_V2, &mut out)
                .expect("the v2 step takes these inputs");
            all.extend_from_slice(&out);
            impulses.push(state.fighters[1].balance_impulse_q8);
        }
        (state, all, impulses)
    }

    /// A rules-v2 bout with the attacker's weapon row set: the attacker
    /// strikes on tick 0 and the foe stands unguarded. Returns the damage
    /// the strike did, or `None` when it fell short.
    fn shield_line_strike(poses: &[StrikePose; 2], weapon: u8) -> Option<u16> {
        let mut state = FightState::new(2).expect("two fighters fit");
        state.fighters[0].weapon = weapon;
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_TICK);
        for tick in 0..=TABLES_V2.attack_startup_ticks {
            let press = if tick == 0 { ATTACK } else { 0 };
            let inputs = [input(0, press), none(1)];
            try_step_v2_into(&inputs, poses, &mut state, &TABLES_V2, &mut out)
                .expect("the v2 step takes these inputs");
            for event in &out {
                if let FightEvent::Hit { damage, .. } = *event {
                    return Some(damage);
                }
            }
        }
        None
    }

    #[test]
    fn shield_line_boss_push_staggers_a_guarded_foe() {
        // The foe holds his guard: the shove with the scutum breaks it,
        // staggers him and feeds his balance, and the steel cuts nothing.
        let poses = [pose(0, 0, 0, (1000, 0), 0), pose(1, 700, 0, (-1000, 0), 1)];
        let (state, events, impulses) = shield_line_push(&poses);
        let foe = state.fighters[1];
        assert!(foe.react > 0, "the push must stagger the guarded foe");
        assert!(!foe.is_blocking, "the push breaks the guard");
        assert_eq!(foe.hp_permille, MAX_HP_PERMILLE, "the shove cuts nothing");
        // The balance channel carries one tick: the shove feeds it once and
        // the next tick clears it.
        let fed: Vec<usize> = impulses
            .iter()
            .enumerate()
            .filter(|(_, impulse)| **impulse != 0)
            .map(|(tick, _)| tick)
            .collect();
        assert_eq!(fed.len(), 1, "one push feeds the balance: {impulses:?}");
        assert_eq!(impulses[fed[0]], PUSH_IMPULSE_Q8, "the shove impulse");
        assert_eq!(
            foe.balance_impulse_q8, 0,
            "the channel is clear on the tick after the push"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, FightEvent::Hit { .. } | FightEvent::Blocked { .. })),
            "the shove has no steel event: {events:?}"
        );
    }

    #[test]
    fn shield_line_thrust_beats_a_cut_on_damage_at_the_same_reach() {
        // The same foe at the same distance: the gladius thrust bites
        // deeper than the spatha cut.
        let poses = [pose(0, 0, 0, (1000, 0), 0), pose(1, 700, 0, (-1000, 0), 1)];
        let thrust = shield_line_strike(&poses, 2).expect("the thrust lands");
        let cut = shield_line_strike(&poses, 3).expect("the cut lands");
        assert!(thrust > cut, "the thrust {thrust} must beat the cut {cut}");
        assert_eq!(thrust, 120);
        assert_eq!(cut, 75);
    }

    #[test]
    fn shield_line_thrust_reaches_past_the_cut() {
        // At 800 mm the cut falls short and the thrust still lands.
        let poses = [pose(0, 0, 0, (1000, 0), 0), pose(1, 800, 0, (-1000, 0), 1)];
        assert_eq!(
            shield_line_strike(&poses, 2),
            Some(120),
            "the thrust reaches under the rim"
        );
        assert_eq!(shield_line_strike(&poses, 3), None, "the cut falls short");
        // The bare hand keeps the arena's own reach: it lands at any range.
        assert_eq!(shield_line_strike(&poses, 0), Some(TABLES_V2.attack_damage));
    }

    #[test]
    fn shield_line_pilum_row_is_a_throw() {
        let rows = weapon_file_rows();
        let pilum = rows
            .iter()
            .find(|row| row.id == "pilum")
            .expect("the pilum row ships");
        assert_eq!(pilum.kind, 3, "the pilum is a throw, not a thrust");
        assert_eq!(pilum.hands, 1, "a thrown pilum is not a two-hand thrust");
    }

    #[test]
    fn shield_line_weapon_rows_match_the_design_file() {
        let rows = weapon_file_rows();
        let by_id: [(&str, u8); 3] = [("pugio", 1), ("gladius", 2), ("spatha", 3)];
        for (id, row) in by_id {
            let file = rows.iter().find(|r| r.id == id).expect("the row ships");
            let index = usize::from(row);
            assert_eq!(file.kind, u32::from(SHIELD_KINDS[index]), "{id} kind");
            assert_eq!(
                file.damage_permille,
                u32::from(SHIELD_DAMAGE[index]),
                "{id} damage"
            );
        }
    }

    #[test]
    fn shield_line_replay_twice_equal() {
        let poses = [pose(0, 0, 0, (1000, 0), 0), pose(1, 700, 0, (-1000, 0), 1)];
        let (state, events, impulses) = shield_line_push(&poses);
        let (again, events_again, impulses_again) = shield_line_push(&poses);
        assert_eq!(state, again, "the same bout gives the same state");
        assert_eq!(events, events_again, "the same bout gives the same events");
        assert_eq!(
            impulses, impulses_again,
            "the same bout feeds the balance the same way"
        );
    }
}
