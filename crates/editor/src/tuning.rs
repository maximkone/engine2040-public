//! Live tuning panel: combat and AI numbers as previewed batches.
//!
//! Consumer is the editor session balancing fights live (shell wiring
//! plus the `shell_ui` Tuning tab). The data owner is [`TuningPanel`]:
//! combat/AI drafts, one staged [`StagedTune`] batch, the [`TuneTape`]
//! journal and the last [`TuneReceipt`]. Units are fixed ticks (`u32`) and
//! rules versions (`u32`): the panel never touches fight inputs, only the
//! numbers plus when they take effect. Determinism scope: same drafts plus
//! same recorded commands give the same staged versions, the same
//! [`rules timeline`](TuneTape::replay_rules) and the same
//! [`AI comparison`](TuneTape::ai_comparison), twice in a row. Errors are
//! typed [`TuneError`]: an invalid value is refused with its legal range and
//! changes nothing, a stale version is refused with last-good kept, and a
//! moved child cursor refuses the apply until the caller refreshes and
//! re-stages.
//!
//! Swap discipline (combat and AI swaps in, practice out, data only): drafts
//! validate through the real combat [`SwapInput`] constructor and the real AI
//! [`AiConfig::try_new`](engine_ai::AiConfig::try_new); the staged batch is
//! handed to the [`LoopbackChild`] — the headless practice-faithful child
//! (command logs, `at_tick <= t` effective rule, stale refusal) — with
//! `at_tick` equal to the child cursor, so swaps take effect at the next
//! tick boundary, never mid-tick. A second child (the editor process
//! transport, `SwapInput` and `AiConfig` crossing unchanged) re-extracts a
//! trait when it lands; until then the concrete child is the boundary. The tape is append-only:
//! old tapes replay through the pinned V1 rules forever, never rewritten.
//! Combat stepping determinism over resolved rows is the combat crate's proven property;
//! this panel proves the timeline it hands the child replays exactly plus
//! what the AI decisions do under it (via the AI
//! [`compare_curves`](engine_ai::compare_curves)).

use std::fmt;

use engine_ai::{
    compare_curves, AiConfig, AiCurveError, AiCurves, Brain, Comparison, Percept, BIAS_MAX,
    CURVE_LANES, DEFAULT_MORALE, ENGAGE_RANGE_MAX_CM, ENGAGE_RANGE_MIN_CM,
};
use engine_combat::{
    fight::{MAX_HP_PERMILLE, MAX_STAMINA},
    CombatError, SwapInput, Tables, TABLES_V1,
};
use engine_core::Fnv64;

/// Shipped combat rules version pinned for tapes without commands.
pub const COMBAT_VERSION_V1: u32 = 1;
/// Shipped AI curve version pinned for tapes without commands.
pub const AI_VERSION_V1: u32 = 1;
/// First replacement version minted by a fresh panel.
pub const FIRST_TUNE_VERSION: u32 = 2;
/// Cap on recorded rules commands per tape.
pub const MAX_TAPE_COMMANDS: usize = 256;
/// Cap on rules commands held by the loopback child.
pub const MAX_CHILD_COMMANDS: usize = 256;
/// Ticks covered by the replay-exactness check.
pub const REPLAY_CHECK_TICKS: u32 = 64;

/// One tunable combat number, in swap-table field order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CombatField {
    /// Damage of one landed attack in permille.
    AttackDamage,
    /// Ticks from attack press to impact.
    AttackStartupTicks,
    /// Ticks the attacker is locked after impact.
    AttackRecoveryTicks,
    /// Stamina cost to start an attack.
    AttackStaminaCost,
    /// Stamina cost to start a dodge.
    DodgeStaminaCost,
    /// Stamina drained per tick while block is held.
    BlockHoldCostPerTick,
    /// Stamina regained per tick by every fighter.
    StaminaRegenPerTick,
    /// Ticks a dodge lasts in total.
    DodgeDurationTicks,
    /// Leading ticks of a dodge that negate hits.
    DodgeIframesTicks,
    /// Block chip numerator: chip = damage * num / den.
    BlockChipNum,
    /// Block chip denominator: chip = damage * num / den.
    BlockChipDen,
    /// Ticks after a block press during which an incoming hit is parried.
    ParryWindowTicks,
    /// Ticks the attacker is staggered after being parried.
    ParryStaggerTicks,
    /// Ticks the victim cannot act after a landed hit.
    HitReactTicks,
    /// Ticks the whole fight freezes after a landed hit or parry.
    HitstopTicks,
    /// Ticks a knocked-down fighter stays down.
    KnockdownTicks,
}

impl CombatField {
    /// Every tunable combat number, in swap-table field order.
    pub const fn all() -> [Self; 16] {
        [
            Self::AttackDamage,
            Self::AttackStartupTicks,
            Self::AttackRecoveryTicks,
            Self::AttackStaminaCost,
            Self::DodgeStaminaCost,
            Self::BlockHoldCostPerTick,
            Self::StaminaRegenPerTick,
            Self::DodgeDurationTicks,
            Self::DodgeIframesTicks,
            Self::BlockChipNum,
            Self::BlockChipDen,
            Self::ParryWindowTicks,
            Self::ParryStaggerTicks,
            Self::HitReactTicks,
            Self::HitstopTicks,
            Self::KnockdownTicks,
        ]
    }

    /// Swap table field name, matching [`CombatError::BadTableValue`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::AttackDamage => "attack_damage",
            Self::AttackStartupTicks => "attack_startup_ticks",
            Self::AttackRecoveryTicks => "attack_recovery_ticks",
            Self::AttackStaminaCost => "attack_stamina_cost",
            Self::DodgeStaminaCost => "dodge_stamina_cost",
            Self::BlockHoldCostPerTick => "block_hold_cost_per_tick",
            Self::StaminaRegenPerTick => "stamina_regen_per_tick",
            Self::DodgeDurationTicks => "dodge_duration_ticks",
            Self::DodgeIframesTicks => "dodge_iframes_ticks",
            Self::BlockChipNum => "block_chip_num",
            Self::BlockChipDen => "block_chip_den",
            Self::ParryWindowTicks => "parry_window_ticks",
            Self::ParryStaggerTicks => "parry_stagger_ticks",
            Self::HitReactTicks => "hit_react_ticks",
            Self::HitstopTicks => "hitstop_ticks",
            Self::KnockdownTicks => "knockdown_ticks",
        }
    }

    /// Legal inclusive range for this field, mirroring the combat crate's
    /// [`validate_tables`](engine_combat::validate_tables). The chip and
    /// i-frame ceilings read the live draft (numerator inside the
    /// denominator, i-frames inside the dodge); the parry-strictly-below-
    /// startup rule is enforced at preview/apply time by [`SwapInput`].
    pub fn range(self, draft: &Tables) -> (u32, u32) {
        let stamina = u32::from(MAX_STAMINA);
        match self {
            Self::AttackDamage => (1, u32::from(MAX_HP_PERMILLE)),
            Self::AttackStartupTicks => (1, 60),
            Self::AttackRecoveryTicks => (0, 120),
            Self::AttackStaminaCost => (0, stamina),
            Self::DodgeStaminaCost => (0, stamina),
            Self::BlockHoldCostPerTick => (0, stamina),
            Self::StaminaRegenPerTick => (0, stamina),
            Self::DodgeDurationTicks => (1, 120),
            Self::DodgeIframesTicks => (0, draft.dodge_duration_ticks),
            Self::BlockChipNum => (0, draft.block_chip_den),
            Self::BlockChipDen => (1, 16),
            Self::ParryWindowTicks => (0, 60),
            Self::ParryStaggerTicks => (0, 120),
            Self::HitReactTicks => (0, 120),
            Self::HitstopTicks => (0, 60),
            Self::KnockdownTicks => (0, 600),
        }
    }

    /// Current draft value for this field.
    pub fn get(self, draft: &Tables) -> u32 {
        match self {
            Self::AttackDamage => u32::from(draft.attack_damage),
            Self::AttackStartupTicks => draft.attack_startup_ticks,
            Self::AttackRecoveryTicks => draft.attack_recovery_ticks,
            Self::AttackStaminaCost => u32::from(draft.attack_stamina_cost),
            Self::DodgeStaminaCost => u32::from(draft.dodge_stamina_cost),
            Self::BlockHoldCostPerTick => u32::from(draft.block_hold_cost_per_tick),
            Self::StaminaRegenPerTick => u32::from(draft.stamina_regen_per_tick),
            Self::DodgeDurationTicks => draft.dodge_duration_ticks,
            Self::DodgeIframesTicks => draft.dodge_iframes_ticks,
            Self::BlockChipNum => draft.block_chip_num,
            Self::BlockChipDen => draft.block_chip_den,
            Self::ParryWindowTicks => draft.parry_window_ticks,
            Self::ParryStaggerTicks => draft.parry_stagger_ticks,
            Self::HitReactTicks => draft.hit_react_ticks,
            Self::HitstopTicks => draft.hitstop_ticks,
            Self::KnockdownTicks => draft.knockdown_ticks,
        }
    }

    /// Write a range-checked value into the draft (the caller checked
    /// [`range`](Self::range) first).
    fn put(self, draft: &mut Tables, value: u32) {
        match self {
            Self::AttackDamage => draft.attack_damage = value.min(u32::from(u16::MAX)) as u16,
            Self::AttackStartupTicks => draft.attack_startup_ticks = value,
            Self::AttackRecoveryTicks => draft.attack_recovery_ticks = value,
            Self::AttackStaminaCost => draft.attack_stamina_cost = value.min(stamina_max()) as u16,
            Self::DodgeStaminaCost => draft.dodge_stamina_cost = value.min(stamina_max()) as u16,
            Self::BlockHoldCostPerTick => {
                draft.block_hold_cost_per_tick = value.min(stamina_max()) as u16;
            }
            Self::StaminaRegenPerTick => {
                draft.stamina_regen_per_tick = value.min(stamina_max()) as u16;
            }
            Self::DodgeDurationTicks => draft.dodge_duration_ticks = value,
            Self::DodgeIframesTicks => draft.dodge_iframes_ticks = value,
            Self::BlockChipNum => draft.block_chip_num = value,
            Self::BlockChipDen => draft.block_chip_den = value,
            Self::ParryWindowTicks => draft.parry_window_ticks = value,
            Self::ParryStaggerTicks => draft.parry_stagger_ticks = value,
            Self::HitReactTicks => draft.hit_react_ticks = value,
            Self::HitstopTicks => draft.hitstop_ticks = value,
            Self::KnockdownTicks => draft.knockdown_ticks = value,
        }
    }
}

impl fmt::Display for CombatField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Full stamina as `u32` for the checked writes in [`CombatField::put`].
fn stamina_max() -> u32 {
    u32::from(MAX_STAMINA)
}

/// What rejected a tuning action. Refusals change neither drafts, child logs
/// nor the tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuneError {
    /// A combat number outside its legal range; the range is carried so the
    /// panel can quote it. The draft is unchanged.
    CombatRange {
        /// Table field name, e.g. `"attack_damage"`.
        field: &'static str,
        /// The refused value.
        value: u32,
        /// Legal minimum, inclusive.
        min: u32,
        /// Legal maximum, inclusive.
        max: u32,
    },
    /// The staged drafts failed combat validation (e.g. the parry window at or
    /// above startup); nothing was staged.
    Combat(CombatError),
    /// An AI number failed AI validation; the cause text carries the legal
    /// range (e.g. `outside 0..=1000`). Nothing was staged.
    Ai(AiCurveError),
    /// No AI curve lane carries this index; lanes are `0..=7`.
    UnknownLane {
        /// The requested lane.
        index: usize,
    },
    /// A combat swap version is not newer than the effective one: last-good
    /// kept, log untouched.
    StaleCombat {
        /// Effective version at the cursor.
        current: u32,
        /// Offered version.
        offered: u32,
    },
    /// An AI swap version is not newer than the effective one.
    StaleAi {
        /// Effective version at the cursor.
        current: u32,
        /// Offered version.
        offered: u32,
    },
    /// The child cursor moved past the staged base: refresh (drop the staged
    /// batch) and re-stage instead of applying.
    StaleChild {
        /// Current child cursor.
        expected: u32,
        /// Cursor the batch was staged against.
        got: u32,
    },
    /// A command names a tick below the last recorded one; logs stay ordered.
    OutOfOrderTick {
        /// Last recorded tick.
        last: u32,
        /// Offered tick.
        offered: u32,
    },
    /// Apply needs a staged batch first.
    NothingStaged,
    /// The log holds no more rules commands.
    TooManyCommands,
}

impl fmt::Display for TuneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CombatRange {
                field,
                value,
                min,
                max,
            } => write!(
                f,
                "combat {field} value {value} outside {min}..={max}: legal range refused nothing"
            ),
            Self::Combat(error) => write!(f, "combat tables refused: {error:?}"),
            Self::Ai(error) => write!(f, "ai curves refused: {error}"),
            Self::UnknownLane { index } => {
                write!(f, "ai curve lane {index} outside 0..=7: legal range refused nothing")
            }
            Self::StaleCombat { current, offered } => write!(
                f,
                "combat version {offered} is not newer than effective {current}"
            ),
            Self::StaleAi { current, offered } => write!(
                f,
                "ai version {offered} is not newer than effective {current}"
            ),
            Self::StaleChild { expected, got } => write!(
                f,
                "stale tune batch: child is at tick {expected}, batch staged at {got}; refresh first"
            ),
            Self::OutOfOrderTick { last, offered } => write!(
                f,
                "tune command at tick {offered} is below last recorded tick {last}"
            ),
            Self::NothingStaged => f.write_str("no tune batch staged"),
            Self::TooManyCommands => write!(f, "tune log is full ({MAX_TAPE_COMMANDS} commands)"),
        }
    }
}

impl std::error::Error for TuneError {}

impl From<CombatError> for TuneError {
    fn from(error: CombatError) -> Self {
        Self::Combat(error)
    }
}

impl From<AiCurveError> for TuneError {
    fn from(error: AiCurveError) -> Self {
        Self::Ai(error)
    }
}

/// One previewed tune batch: validated combat and AI snapshots plus the child
/// cursor they were staged against. Snapshots are owned values, so later
/// draft edits never mutate a staged batch.
#[derive(Clone, Debug)]
pub struct StagedTune {
    base_cursor: u32,
    combat: SwapInput,
    ai: AiConfig,
}

impl StagedTune {
    /// Child cursor this batch was staged against.
    pub const fn base_cursor(&self) -> u32 {
        self.base_cursor
    }

    /// Validated combat snapshot, by value: a snapshot, never an alias.
    pub fn combat(&self) -> SwapInput {
        self.combat.clone()
    }

    /// Validated AI snapshot.
    pub const fn ai(&self) -> AiConfig {
        self.ai
    }

    /// Combat rules version this batch carries.
    pub fn combat_version(&self) -> u32 {
        self.combat.version()
    }

    /// AI rules version this batch carries.
    pub const fn ai_version(&self) -> u32 {
        self.ai.curves.version
    }
}

/// One recorded rules command: owned snapshots plus the tick they take
/// effect at. Later tuning never mutates a recorded command.
#[derive(Clone, Debug)]
pub struct TuneCommand {
    /// First tick the snapshots govern (`at_tick <= t` wins).
    pub at_tick: u32,
    /// Validated combat snapshot.
    pub combat: SwapInput,
    /// Validated AI snapshot.
    pub ai: AiConfig,
}

/// Receipt for one applied tune batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuneReceipt {
    /// Tick the batch took effect at (the child cursor at apply time).
    pub at_tick: u32,
    /// Applied combat rules version.
    pub combat_version: u32,
    /// Applied AI rules version.
    pub ai_version: u32,
}

impl fmt::Display for TuneReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tuned at tick {}: combat v{}, ai v{}",
            self.at_tick, self.combat_version, self.ai_version
        )
    }
}

/// Append-only tune tape: the recorded rules commands in cursor order. Ticks
/// and versions only — fight inputs stay with the child authority; this tape
/// journals which rules governed which ticks. Old tapes replay through the
/// pinned V1 rules forever; recording new commands never rewrites recorded
/// commands.
#[derive(Clone, Debug, Default)]
pub struct TuneTape {
    commands: Vec<TuneCommand>,
}

impl TuneTape {
    /// Empty tape: no commands, V1 rules effective everywhere.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recorded command count.
    pub fn command_len(&self) -> usize {
        self.commands.len()
    }

    /// True when no command is recorded.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Record one rules command. Ticks stay ordered: `at_tick` must not fall
    /// below the last recorded command.
    pub fn record_command(&mut self, command: TuneCommand) -> Result<(), TuneError> {
        if self.commands.len() >= MAX_TAPE_COMMANDS {
            return Err(TuneError::TooManyCommands);
        }
        if let Some(last) = self.commands.last() {
            if command.at_tick < last.at_tick {
                return Err(TuneError::OutOfOrderTick {
                    last: last.at_tick,
                    offered: command.at_tick,
                });
            }
        }
        self.commands.push(command);
        Ok(())
    }

    /// Combat rules version effective for `tick`: the latest command with
    /// `at_tick <= tick`, or [`COMBAT_VERSION_V1`] when none recorded. Pure.
    pub fn combat_version_for_tick(&self, tick: u32) -> u32 {
        let mut version = COMBAT_VERSION_V1;
        for command in &self.commands {
            if command.at_tick <= tick {
                version = command.combat.version();
            }
        }
        version
    }

    /// AI rules version effective for `tick`. Pure.
    pub fn ai_version_for_tick(&self, tick: u32) -> u32 {
        let mut version = AI_VERSION_V1;
        for command in &self.commands {
            if command.at_tick <= tick {
                version = command.ai.curves.version;
            }
        }
        version
    }

    /// Combat rows effective for `tick`: owned rows (replacement) or the
    /// pinned V1 row when no command covers the tick. Pure.
    pub fn combat_tables_for_tick(&self, tick: u32) -> Tables {
        let mut tables = TABLES_V1;
        for command in &self.commands {
            if command.at_tick <= tick {
                tables = command.combat.tables();
            }
        }
        tables
    }

    /// AI snapshot effective for `tick`. Pure.
    pub fn ai_for_tick(&self, tick: u32) -> AiConfig {
        let mut config = AiConfig::scripted_defaults();
        for command in &self.commands {
            if command.at_tick <= tick {
                config = command.ai;
            }
        }
        config
    }

    /// Replay the rules timeline: the effective `(combat rows, AI snapshot)`
    /// per tick for `ticks` ticks. Same tape twice gives the same timeline;
    /// a tape with no commands replays the V1 timeline. Pure.
    pub fn replay_rules(&self, ticks: u32) -> Vec<(Tables, AiConfig)> {
        (0..ticks)
            .map(|tick| (self.combat_tables_for_tick(tick), self.ai_for_tick(tick)))
            .collect()
    }

    /// Fork-compare the AI snapshots effective at two ticks over a fixed
    /// percept script ([`compare_curves`](engine_ai::compare_curves)):
    /// what the tuning changed, as data. Same tape plus same script twice
    /// gives the same report. Pure.
    pub fn ai_comparison(
        &self,
        before_tick: u32,
        after_tick: u32,
        script: &[Percept],
    ) -> Comparison {
        compare_curves(
            &self.ai_for_tick(before_tick),
            &self.ai_for_tick(after_tick),
            script,
        )
    }
}

/// Headless practice-faithful child: cursor, two ordered command logs, the
/// `at_tick <= t` effective rule and stale refusal. Ticks advance only
/// through [`tick`](Self::tick) — the frame boundary — never mid-step. This
/// concrete child is the publication boundary the panel applies through; a
/// second child re-extracts a trait when it lands.
#[derive(Clone, Debug, Default)]
pub struct LoopbackChild {
    cursor: u32,
    combat_log: Vec<(u32, SwapInput)>,
    ai_log: Vec<(u32, AiConfig)>,
}

impl LoopbackChild {
    /// Fresh child at tick zero with V1 rules effective.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance one fixed tick (the frame boundary callers drive between
    /// proposals, never mid-tick).
    pub fn tick(&mut self) {
        self.cursor = self.cursor.saturating_add(1);
    }

    /// Recorded combat command count.
    pub fn combat_commands(&self) -> usize {
        self.combat_log.len()
    }

    /// Recorded AI command count.
    pub fn ai_commands(&self) -> usize {
        self.ai_log.len()
    }

    /// Combat version effective for `tick`. Pure.
    fn effective_combat(&self, tick: u32) -> u32 {
        let mut version = COMBAT_VERSION_V1;
        for (at_tick, swap) in &self.combat_log {
            if *at_tick <= tick {
                version = swap.version();
            }
        }
        version
    }

    /// AI version effective for `tick`. Pure.
    fn effective_ai(&self, tick: u32) -> u32 {
        let mut version = AI_VERSION_V1;
        for (at_tick, config) in &self.ai_log {
            if *at_tick <= tick {
                version = config.curves.version;
            }
        }
        version
    }

    /// Last recorded tick across both logs, for the ordering invariant.
    fn last_tick(&self) -> Option<u32> {
        self.combat_log
            .last()
            .map(|(tick, _)| *tick)
            .into_iter()
            .chain(self.ai_log.last().map(|(tick, _)| *tick))
            .max()
    }

    /// Fixed-tick cursor: ticks simulated so far.
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Record a combat swap taking effect at `at_tick`. Stale versions
    /// (not newer than effective) are refused with last-good kept.
    pub fn propose_combat(&mut self, at_tick: u32, swap: SwapInput) -> Result<u32, TuneError> {
        if self.combat_log.len() + self.ai_log.len() >= MAX_CHILD_COMMANDS {
            return Err(TuneError::TooManyCommands);
        }
        if let Some(last) = self.last_tick() {
            if at_tick < last {
                return Err(TuneError::OutOfOrderTick {
                    last,
                    offered: at_tick,
                });
            }
        }
        let offered = swap.version();
        let current = self.effective_combat(at_tick);
        if offered <= current {
            return Err(TuneError::StaleCombat { current, offered });
        }
        self.combat_log.push((at_tick, swap));
        Ok(offered)
    }

    /// Record an AI swap taking effect at `at_tick`. Same discipline:
    /// invalid snapshots and stale versions are refused, last-good kept.
    pub fn propose_ai(&mut self, at_tick: u32, config: AiConfig) -> Result<u32, TuneError> {
        if self.combat_log.len() + self.ai_log.len() >= MAX_CHILD_COMMANDS {
            return Err(TuneError::TooManyCommands);
        }
        if let Some(last) = self.last_tick() {
            if at_tick < last {
                return Err(TuneError::OutOfOrderTick {
                    last,
                    offered: at_tick,
                });
            }
        }
        if let Err(error) = AiConfig::try_new(config.curves, config.morale_default) {
            return Err(TuneError::Ai(error));
        }
        let offered = config.curves.version;
        let current = self.effective_ai(at_tick);
        if offered <= current {
            return Err(TuneError::StaleAi { current, offered });
        }
        self.ai_log.push((at_tick, config));
        Ok(offered)
    }
}

/// Live tuning panel over the combat and AI swap schemas and the practice child seam.
///
/// Drafts start at the shipped V1 rules; every setter range-checks before
/// mutating, so an invalid value leaves the draft it found.
/// [`stage`](Self::stage) validates both drafts (combat first,
/// deterministically) and mints the next versions; [`apply`](Self::apply)
/// hands the staged batch to the child at its cursor and journals the
/// command into the tape. The tape and the child logs are append-only:
/// applies never rewrite earlier commands.
pub struct TuningPanel {
    combat_draft: Tables,
    ai_curves: AiCurves,
    ai_morale: i32,
    next_combat: u32,
    next_ai: u32,
    staged: Option<StagedTune>,
    tape: TuneTape,
    last_receipt: Option<TuneReceipt>,
}

impl TuningPanel {
    /// Fresh panel: V1 drafts, first replacements mint version 2, empty tape.
    pub fn new() -> Self {
        Self {
            combat_draft: TABLES_V1,
            ai_curves: AiCurves::default_curves(),
            ai_morale: DEFAULT_MORALE,
            next_combat: FIRST_TUNE_VERSION,
            next_ai: FIRST_TUNE_VERSION,
            staged: None,
            tape: TuneTape::new(),
            last_receipt: None,
        }
    }

    /// Current combat draft (the numbers the panel edits).
    pub const fn combat_draft(&self) -> Tables {
        self.combat_draft
    }

    /// Current draft value for one combat field.
    pub fn combat_value(&self, field: CombatField) -> u32 {
        field.get(&self.combat_draft)
    }

    /// Legal range for one combat field against the live draft.
    pub fn combat_range(&self, field: CombatField) -> (u32, u32) {
        field.range(&self.combat_draft)
    }

    /// Edit one combat number. Range-checked first: an invalid value is
    /// refused with its legal range and the draft is unchanged.
    pub fn set_combat(&mut self, field: CombatField, value: u32) -> Result<(), TuneError> {
        let (min, max) = field.range(&self.combat_draft);
        if value < min || value > max {
            return Err(TuneError::CombatRange {
                field: field.name(),
                value,
                min,
                max,
            });
        }
        field.put(&mut self.combat_draft, value);
        Ok(())
    }

    /// Current AI curve draft.
    pub const fn ai_curves(&self) -> AiCurves {
        self.ai_curves
    }

    /// Current AI morale default draft.
    pub const fn ai_morale(&self) -> i32 {
        self.ai_morale
    }

    /// Edit one aggression lane (`0..=7`, values `0..=1000`, checked now;
    /// monotonicity is checked at preview). Refusals change nothing.
    pub fn set_aggression(&mut self, lane: usize, value: u16) -> Result<(), TuneError> {
        if lane >= CURVE_LANES {
            return Err(TuneError::UnknownLane { index: lane });
        }
        if value > BIAS_MAX {
            return Err(TuneError::Ai(AiCurveError::AggressionOutOfRange {
                index: lane,
                value,
            }));
        }
        self.ai_curves.aggression[lane] = value;
        Ok(())
    }

    /// Edit one caution lane (`0..=7`, values `0..=1000`, any shape).
    pub fn set_caution(&mut self, lane: usize, value: u16) -> Result<(), TuneError> {
        if lane >= CURVE_LANES {
            return Err(TuneError::UnknownLane { index: lane });
        }
        if value > BIAS_MAX {
            return Err(TuneError::Ai(AiCurveError::CautionOutOfRange {
                index: lane,
                value,
            }));
        }
        self.ai_curves.caution[lane] = value;
        Ok(())
    }

    /// Edit the strike-commit range in centimetres (`50..=6000`).
    pub fn set_engage_range(&mut self, cm: u32) -> Result<(), TuneError> {
        if !(ENGAGE_RANGE_MIN_CM..=ENGAGE_RANGE_MAX_CM).contains(&cm) {
            return Err(TuneError::Ai(AiCurveError::EngageRangeOutOfRange {
                value: cm,
            }));
        }
        self.ai_curves.engage_range_cm = cm;
        Ok(())
    }

    /// Edit the starting morale (`0..=1000`).
    pub fn set_morale(&mut self, morale: i32) -> Result<(), TuneError> {
        if !(Brain::MORALE_MIN..=Brain::MORALE_MAX).contains(&morale) {
            return Err(TuneError::Ai(AiCurveError::BadMoraleDefault {
                value: morale,
            }));
        }
        self.ai_morale = morale;
        Ok(())
    }

    /// Next combat version a staged batch would mint.
    pub const fn next_combat_version(&self) -> u32 {
        self.next_combat
    }

    /// Next AI version a staged batch would mint.
    pub const fn next_ai_version(&self) -> u32 {
        self.next_ai
    }

    /// The staged batch, if any.
    pub const fn staged(&self) -> Option<&StagedTune> {
        self.staged.as_ref()
    }

    /// True when a batch is staged against a cursor that moved on.
    pub fn is_stale(&self, cursor: u32) -> bool {
        self.staged
            .as_ref()
            .is_some_and(|staged| staged.base_cursor != cursor)
    }

    /// The tune tape journal.
    pub const fn tape(&self) -> &TuneTape {
        &self.tape
    }

    /// Last applied receipt, if any batch applied yet.
    pub const fn last_receipt(&self) -> Option<TuneReceipt> {
        self.last_receipt
    }

    /// Preview the combat draft without side effects: the validated combat swap
    /// input for the next version. Versions are not minted by previews.
    pub fn preview_combat(&self) -> Result<SwapInput, TuneError> {
        Ok(SwapInput::new(self.next_combat, self.combat_draft)?)
    }

    /// Preview the AI draft without side effects: the validated AI snapshot
    /// for the next version. The draft's lane table is copied under the
    /// minted version; the draft itself keeps version 1 until staged.
    pub fn preview_ai(&self) -> Result<AiConfig, TuneError> {
        let mut curves = self.ai_curves;
        curves.version = self.next_ai;
        Ok(AiConfig::try_new(curves, self.ai_morale)?)
    }

    /// Stage a previewed batch against the child cursor: validates both
    /// drafts (combat first, deterministically), mints the next versions and
    /// stores the batch. Drafts stay editable afterwards; the staged batch
    /// keeps its own snapshots.
    pub fn stage(&mut self, base_cursor: u32) -> Result<StagedTune, TuneError> {
        let combat = SwapInput::new(self.next_combat, self.combat_draft)?;
        let mut curves = self.ai_curves;
        curves.version = self.next_ai;
        let ai = AiConfig::try_new(curves, self.ai_morale)?;
        let staged = StagedTune {
            base_cursor,
            combat,
            ai,
        };
        self.next_combat = self.next_combat.saturating_add(1).max(FIRST_TUNE_VERSION);
        self.next_ai = self.next_ai.saturating_add(1).max(FIRST_TUNE_VERSION);
        self.staged = Some(staged.clone());
        Ok(staged)
    }

    /// Drop the staged batch. Returns true when one was waiting.
    pub fn refresh(&mut self) -> bool {
        self.staged.take().is_some()
    }

    /// Apply the staged batch through the child at its cursor: refuses with
    /// [`TuneError::StaleChild`] when the child moved on (caller refreshes
    /// and re-stages), journals the command into the tape on success and
    /// consumes the staged batch.
    pub fn apply(&mut self, child: &mut LoopbackChild) -> Result<TuneReceipt, TuneError> {
        let staged = self.staged.clone().ok_or(TuneError::NothingStaged)?;
        let cursor = child.cursor();
        if cursor != staged.base_cursor {
            return Err(TuneError::StaleChild {
                expected: cursor,
                got: staged.base_cursor,
            });
        }
        let combat_version = child.propose_combat(cursor, staged.combat.clone())?;
        let ai_version = child.propose_ai(cursor, staged.ai)?;
        self.tape.record_command(TuneCommand {
            at_tick: cursor,
            combat: staged.combat.clone(),
            ai: staged.ai,
        })?;
        let receipt = TuneReceipt {
            at_tick: cursor,
            combat_version,
            ai_version,
        };
        self.last_receipt = Some(receipt);
        self.staged = None;
        Ok(receipt)
    }

    /// Replay-exactness check over the tape: the rules timeline replays
    /// identically twice, and the AI decisions under the first and last
    /// effective snapshots compare identically twice.
    pub fn replay_exact(&self) -> bool {
        if self.tape.replay_rules(REPLAY_CHECK_TICKS) != self.tape.replay_rules(REPLAY_CHECK_TICKS)
        {
            return false;
        }
        let script = canonical_percepts(32);
        let first = self.tape.ai_for_tick(0);
        let last = self.tape.ai_for_tick(u32::MAX);
        self.tape.ai_comparison(0, u32::MAX, &script) == compare_curves(&first, &last, &script)
    }

    /// Combat rows effective for `tick` under the recorded commands.
    pub fn combat_tables_for_tick(&self, tick: u32) -> Tables {
        self.tape.combat_tables_for_tick(tick)
    }

    /// AI snapshot effective for `tick` under the recorded commands.
    pub fn ai_for_tick(&self, tick: u32) -> AiConfig {
        self.tape.ai_for_tick(tick)
    }
}

impl Default for TuningPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Canonical percept script for replay-exactness checks: two fighters
/// closing from 2 m, the foe striking at tick 4. Fixed data, no randomness.
pub fn canonical_percepts(ticks: u32) -> Vec<Percept> {
    (0..ticks)
        .map(|tick| Percept::new([0.0, 0.0, 0.0], 0.0, [2.0, 0.0, 0.0], tick == 4, 2.0, tick))
        .collect()
}

/// Edit-to-game budget for the A10 fixture leg, in milliseconds. The leg
/// reports its wall time against this budget; the budget never gates state.
pub const EDIT_TO_GAME_BUDGET_MS: u64 = 1_000;
/// Child cursor the A10 fixture edit lands at.
pub const A10_CURSOR: u32 = 8;
/// Combat damage the fixture edit tunes to.
pub const A10_DAMAGE: u32 = 240;
/// AI engage range the fixture edit tunes to, in centimetres.
pub const A10_ENGAGE_CM: u32 = 300;

/// Receipt for one scripted edit-to-game leg: wall time plus replay proof.
/// Wall time is evidence only; the hash is the replay gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditToGameReceipt {
    /// Wall ms from first keystroke to live game plus replay check.
    pub elapsed_ms: u64,
    /// Tick the batch took effect at.
    pub at_tick: u32,
    /// Applied combat rules version.
    pub combat_version: u32,
    /// Applied AI rules version.
    pub ai_version: u32,
    /// Deterministic hash of the tuned rules timeline.
    pub replay_hash: u64,
    /// True when the tuned timeline replays exactly twice.
    pub replay_exact: bool,
    /// Bad edits refused with last-good kept.
    pub bad_edits_refused: u32,
}

impl fmt::Display for EditToGameReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "edit-to-game {} ms: tuned at tick {} combat v{} ai v{}, replay hash {:016x} replay-exact {} bad-edits-refused {}/3 last-good-kept budget {} ms",
            self.elapsed_ms,
            self.at_tick,
            self.combat_version,
            self.ai_version,
            self.replay_hash,
            if self.replay_exact { "yes" } else { "no" },
            self.bad_edits_refused,
            EDIT_TO_GAME_BUDGET_MS,
        )
    }
}

/// Deterministic hash of the tuned rules timeline: every combat row plus
/// every AI snapshot over [`REPLAY_CHECK_TICKS`] ticks, in tick order. Same
/// tape twice gives the same hash; wall clock never enters it. Pure.
pub fn tuned_fight_hash(panel: &TuningPanel) -> u64 {
    let mut hash = Fnv64::new();
    for (tables, ai) in panel.tape().replay_rules(REPLAY_CHECK_TICKS) {
        hash.write(&tables.attack_damage.to_le_bytes());
        hash.write(&tables.attack_startup_ticks.to_le_bytes());
        hash.write(&tables.attack_recovery_ticks.to_le_bytes());
        hash.write(&tables.attack_stamina_cost.to_le_bytes());
        hash.write(&tables.dodge_stamina_cost.to_le_bytes());
        hash.write(&tables.block_hold_cost_per_tick.to_le_bytes());
        hash.write(&tables.stamina_regen_per_tick.to_le_bytes());
        hash.write(&tables.dodge_duration_ticks.to_le_bytes());
        hash.write(&tables.dodge_iframes_ticks.to_le_bytes());
        hash.write(&tables.block_chip_num.to_le_bytes());
        hash.write(&tables.block_chip_den.to_le_bytes());
        hash.write(&tables.parry_window_ticks.to_le_bytes());
        hash.write(&tables.parry_stagger_ticks.to_le_bytes());
        hash.write(&tables.hit_react_ticks.to_le_bytes());
        hash.write(&tables.hitstop_ticks.to_le_bytes());
        hash.write(&tables.knockdown_ticks.to_le_bytes());
        hash.write(&ai.curves.version.to_le_bytes());
        for lane in ai.curves.aggression {
            hash.write(&lane.to_le_bytes());
        }
        for lane in ai.curves.caution {
            hash.write(&lane.to_le_bytes());
        }
        hash.write(&ai.curves.engage_range_cm.to_le_bytes());
        hash.write(&ai.morale_default.to_le_bytes());
    }
    hash.finish()
}

/// Scripted A10 fixture leg: three bad keystrokes refused with last-good
/// kept, one good tune staged and applied at [`A10_CURSOR`], then the replay
/// hash checked twice. Returns the apply receipt, the timing receipt, the
/// timing log text and the capture text. Wall time covers the first
/// keystroke through the replay check; it is evidence only, never state.
pub fn scripted_edit_to_game() -> (TuneReceipt, EditToGameReceipt, String, String) {
    let start = std::time::Instant::now();
    let mut panel = TuningPanel::new();
    let mut child = LoopbackChild::new();
    // Three bad keystrokes: each refused, drafts and child logs unchanged.
    let combat_before = panel.combat_draft();
    let ai_before = (panel.ai_curves(), panel.ai_morale());
    assert!(panel.set_combat(CombatField::AttackDamage, 0).is_err());
    assert!(panel.set_aggression(CURVE_LANES, 500).is_err());
    assert!(panel.set_engage_range(49).is_err());
    assert_eq!(panel.combat_draft(), combat_before, "bad edits keep combat");
    assert_eq!(
        (panel.ai_curves(), panel.ai_morale()),
        ai_before,
        "bad edits keep ai"
    );
    assert_eq!(child.combat_commands() + child.ai_commands(), 0);
    // Good keystrokes on the fixture: strict parry row plus tuned numbers.
    panel
        .set_combat(CombatField::AttackDamage, A10_DAMAGE)
        .expect("fixture damage stages");
    panel
        .set_combat(CombatField::ParryWindowTicks, 3)
        .expect("fixture parry stages");
    panel
        .set_engage_range(A10_ENGAGE_CM)
        .expect("fixture engage stages");
    for _ in 0..A10_CURSOR {
        child.tick();
    }
    panel.stage(child.cursor()).expect("fixture stages");
    let receipt = panel.apply(&mut child).expect("fixture applies");
    let replay_exact = panel.replay_exact();
    let first = tuned_fight_hash(&panel);
    let second = tuned_fight_hash(&panel);
    assert_eq!(first, second, "tuned fight hashes twice");
    let elapsed_ms = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let timing = EditToGameReceipt {
        elapsed_ms,
        at_tick: receipt.at_tick,
        combat_version: receipt.combat_version,
        ai_version: receipt.ai_version,
        replay_hash: first,
        replay_exact,
        bad_edits_refused: 3,
    };
    let log = format!(
        "A10 edit-to-game timing log\ncommand: cargo test -p engine-editor --all-features tuning\nfixture: strict panel, cursor {A10_CURSOR}, damage {A10_DAMAGE}, engage {A10_ENGAGE_CM} cm\n{timing}\nbudget_ms: {EDIT_TO_GAME_BUDGET_MS}\nreplay_hash: {first:016x} x2\ncapture: target/evidence/a10/capture.txt\n"
    );
    let capture = format!(
        "A10 capture: tuned fixture fight\n{receipt}\n{timing}\ntape_commands: {}\ncombat_damage_at_tick_{A10_CURSOR}: {}\nreplay_hash: {first:016x}\n",
        panel.tape().command_len(),
        panel.combat_tables_for_tick(A10_CURSOR).attack_damage,
    );
    (receipt, timing, log, capture)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drafts that satisfy the strict combat preview rules (V1 is grandfathered
    /// for replays; new sets need parry strictly below startup).
    fn strict_panel() -> TuningPanel {
        let mut panel = TuningPanel::new();
        panel
            .set_combat(CombatField::ParryWindowTicks, 3)
            .expect("parry 3 below startup 4 stages");
        panel
    }

    /// Strict combat rows for direct swap construction in tape tests.
    fn strict_rows() -> Tables {
        let mut rows = TABLES_V1;
        rows.parry_window_ticks = 3;
        rows.attack_damage = 240;
        rows
    }

    #[test]
    fn drafts_start_at_v1_and_preview_mints_v2() {
        let panel = strict_panel();
        assert_eq!(panel.combat_draft(), {
            let mut rows = TABLES_V1;
            rows.parry_window_ticks = 3;
            rows
        });
        assert_eq!(panel.next_combat_version(), FIRST_TUNE_VERSION);
        assert_eq!(panel.next_ai_version(), FIRST_TUNE_VERSION);
        let combat = panel.preview_combat().unwrap();
        assert_eq!(combat.version(), 2);
        assert_eq!(combat.tables().parry_window_ticks, 3);
        let ai = panel.preview_ai().unwrap();
        assert_eq!(ai.curves.version, 2);
        assert!(ai.is_valid());
        // Previews mint nothing: a second preview offers the same versions.
        assert_eq!(panel.preview_combat().unwrap().version(), 2);
    }

    #[test]
    fn combat_edits_apply_and_preview_validates_strict_rules() {
        let mut panel = TuningPanel::new();
        panel
            .set_combat(CombatField::AttackDamage, 240)
            .expect("240 is in range");
        assert_eq!(panel.combat_value(CombatField::AttackDamage), 240);
        // V1 drafts still carry the grandfathered parry row, so the preview
        // refuses with the strict rule and stages nothing.
        assert_eq!(
            panel.preview_combat().unwrap_err(),
            TuneError::Combat(CombatError::BadTableValue {
                field: "parry_window_ticks",
                value: 4,
            })
        );
        assert!(panel.staged().is_none());
        // Tighten parry below startup and the same damage stages as v2.
        panel.set_combat(CombatField::ParryWindowTicks, 3).unwrap();
        let staged = panel.stage(8).unwrap();
        assert_eq!(staged.base_cursor(), 8);
        assert_eq!(staged.combat_version(), 2);
        assert_eq!(staged.combat().tables().attack_damage, 240);
        assert_eq!(panel.next_combat_version(), 3);
        // Later draft edits never mutate the staged snapshot.
        panel.set_combat(CombatField::AttackDamage, 300).unwrap();
        assert_eq!(staged.combat().tables().attack_damage, 240);
        assert_eq!(
            panel
                .staged()
                .expect("batch kept")
                .combat()
                .tables()
                .attack_damage,
            240
        );
    }

    #[test]
    fn invalid_combat_refused_with_legal_range_and_draft_kept() {
        let mut panel = strict_panel();
        // Zero damage and over-full damage both name 1..=1000.
        assert_eq!(
            panel.set_combat(CombatField::AttackDamage, 0).unwrap_err(),
            TuneError::CombatRange {
                field: "attack_damage",
                value: 0,
                min: 1,
                max: u32::from(MAX_HP_PERMILLE),
            }
        );
        assert_eq!(
            panel
                .set_combat(CombatField::AttackDamage, 1001)
                .unwrap_err(),
            TuneError::CombatRange {
                field: "attack_damage",
                value: 1001,
                min: 1,
                max: u32::from(MAX_HP_PERMILLE),
            }
        );
        // Startup needs at least one tick; chip denominator is never zero.
        assert_eq!(
            panel
                .set_combat(CombatField::AttackStartupTicks, 0)
                .unwrap_err(),
            TuneError::CombatRange {
                field: "attack_startup_ticks",
                value: 0,
                min: 1,
                max: 60,
            }
        );
        assert_eq!(
            panel.set_combat(CombatField::BlockChipDen, 0).unwrap_err(),
            TuneError::CombatRange {
                field: "block_chip_den",
                value: 0,
                min: 1,
                max: 16,
            }
        );
        assert_eq!(
            panel.combat_draft(),
            strict_panel().combat_draft(),
            "refusals keep the draft"
        );
        // The chip numerator ceiling follows the live denominator, and the
        // i-frame ceiling follows the live dodge duration.
        panel.set_combat(CombatField::BlockChipDen, 4).unwrap();
        let before = panel.combat_draft();
        assert_eq!(
            panel.set_combat(CombatField::BlockChipNum, 5).unwrap_err(),
            TuneError::CombatRange {
                field: "block_chip_num",
                value: 5,
                min: 0,
                max: 4,
            }
        );
        assert_eq!(
            panel
                .set_combat(CombatField::DodgeIframesTicks, 11)
                .unwrap_err(),
            TuneError::CombatRange {
                field: "dodge_iframes_ticks",
                value: 11,
                min: 0,
                max: 10,
            }
        );
        assert_eq!(panel.combat_draft(), before, "refusals keep the draft");
        // The refusal text quotes the legal range.
        let error = panel
            .set_combat(CombatField::KnockdownTicks, 601)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "combat knockdown_ticks value 601 outside 0..=600: legal range refused nothing"
        );
    }

    #[test]
    fn invalid_ai_refused_with_legal_range_and_draft_kept() {
        let mut panel = TuningPanel::new();
        let before = (panel.ai_curves(), panel.ai_morale());
        assert_eq!(
            panel.set_aggression(0, 1001).unwrap_err(),
            TuneError::Ai(AiCurveError::AggressionOutOfRange {
                index: 0,
                value: 1001,
            })
        );
        assert_eq!(
            panel.set_caution(7, 1001).unwrap_err(),
            TuneError::Ai(AiCurveError::CautionOutOfRange {
                index: 7,
                value: 1001,
            })
        );
        assert_eq!(
            panel.set_engage_range(49).unwrap_err(),
            TuneError::Ai(AiCurveError::EngageRangeOutOfRange { value: 49 })
        );
        assert_eq!(
            panel.set_engage_range(6001).unwrap_err(),
            TuneError::Ai(AiCurveError::EngageRangeOutOfRange { value: 6001 })
        );
        assert_eq!(
            panel.set_morale(-1).unwrap_err(),
            TuneError::Ai(AiCurveError::BadMoraleDefault { value: -1 })
        );
        assert_eq!(
            panel.set_aggression(8, 500).unwrap_err(),
            TuneError::UnknownLane { index: 8 }
        );
        assert_eq!(
            panel.set_caution(8, 500).unwrap_err(),
            TuneError::UnknownLane { index: 8 }
        );
        assert_eq!((panel.ai_curves(), panel.ai_morale()), before);
        // A dipped aggression lane passes the setter but fails the preview.
        panel.set_aggression(0, 600).unwrap();
        panel.set_aggression(1, 599).unwrap();
        assert_eq!(
            panel.preview_ai().unwrap_err(),
            TuneError::Ai(AiCurveError::AggressionNotMonotonic {
                index: 1,
                prev: 600,
                next: 599,
            })
        );
        assert!(panel.staged().is_none());
        // The AI refusal text carries the legal range.
        let error = panel.set_engage_range(49).unwrap_err();
        assert!(
            error.to_string().contains("50..=6000"),
            "refusal quotes range: {error}"
        );
    }

    #[test]
    fn hot_ai_tune_diverges_from_scripted_deterministically() {
        // A hot top-lane aggression tune validates and provably changes AI
        // decisions over the canonical script, identically twice.
        let mut panel = TuningPanel::new();
        for lane in 0..CURVE_LANES {
            panel.set_aggression(lane, 1000).unwrap();
        }
        panel.set_morale(1000).unwrap();
        let hot = panel.preview_ai().unwrap();
        let script = canonical_percepts(16);
        let v1 = AiConfig::scripted_defaults();
        let first = compare_curves(&v1, &hot, &script);
        let second = compare_curves(&v1, &hot, &script);
        assert_eq!(first, second);
        assert_eq!(first.total, 16);
        assert!(
            first.first_divergent_index.is_some(),
            "hot tune must diverge somewhere: {first:?}"
        );
        // Identical configs agree everywhere, vacuously or not.
        let same = compare_curves(&v1, &v1, &script);
        assert_eq!(same.agree, same.total);
        assert_eq!(same.first_divergent_index, None);
    }

    #[test]
    fn stage_apply_records_at_tick_boundary_and_replays_exactly() {
        let mut panel = strict_panel();
        panel.set_combat(CombatField::AttackDamage, 240).unwrap();
        panel.set_engage_range(300).unwrap();
        // Stage against the child cursor and apply: the batch lands at the
        // cursor tick, never mid-tick.
        let mut child = LoopbackChild::new();
        for _ in 0..8 {
            child.tick();
        }
        assert_eq!(child.cursor(), 8);
        let staged = panel.stage(child.cursor()).unwrap();
        assert_eq!((staged.combat_version(), staged.ai_version()), (2, 2));
        let receipt = panel.apply(&mut child).unwrap();
        assert_eq!(
            receipt,
            TuneReceipt {
                at_tick: 8,
                combat_version: 2,
                ai_version: 2,
            }
        );
        assert_eq!(receipt.to_string(), "tuned at tick 8: combat v2, ai v2");
        assert!(panel.staged().is_none(), "apply consumes the batch");
        assert_eq!(panel.last_receipt(), Some(receipt));
        // Ticks below the boundary pin V1; the boundary tick runs the swap.
        assert_eq!(panel.tape().combat_version_for_tick(7), COMBAT_VERSION_V1);
        assert_eq!(panel.tape().combat_version_for_tick(8), 2);
        assert_eq!(panel.tape().ai_version_for_tick(7), AI_VERSION_V1);
        assert_eq!(panel.tape().ai_version_for_tick(8), 2);
        assert_eq!(
            panel.combat_tables_for_tick(8).attack_damage,
            240,
            "recorded rows carry the tune"
        );
        // A tuned fight replays exactly: same rules timeline plus same AI
        // comparison, twice.
        assert!(panel.replay_exact());
        let script = canonical_percepts(16);
        assert_eq!(
            panel.tape().ai_comparison(0, u32::MAX, &script),
            panel.tape().ai_comparison(0, u32::MAX, &script)
        );
    }

    #[test]
    fn old_tapes_keep_old_rules_and_boundaries_hold() {
        let plain = TuneTape::new();
        assert!(plain.is_empty());
        // The same inputs with no commands replay the V1 timeline exactly.
        let v1_timeline = plain.replay_rules(16);
        assert_eq!(v1_timeline.len(), 16);
        assert!(
            v1_timeline
                .iter()
                .all(|(tables, ai)| *tables == TABLES_V1 && *ai == AiConfig::scripted_defaults()),
            "command-free tapes pin V1"
        );
        // Tuning a copy at tick 8 changes only ticks at or past the swap:
        // the prefix still runs the old rules, recorded ticks unrewritten.
        let mut tuned = plain.clone();
        tuned
            .record_command(TuneCommand {
                at_tick: 8,
                combat: SwapInput::new(2, strict_rows()).unwrap(),
                ai: AiConfig::scripted_defaults(),
            })
            .unwrap();
        assert_eq!(tuned.command_len(), 1);
        let tuned_timeline = tuned.replay_rules(16);
        assert_eq!(&tuned_timeline[..8], &v1_timeline[..8]);
        assert_ne!(tuned_timeline[8], v1_timeline[8]);
        assert_eq!(tuned_timeline[8].0.attack_damage, 240);
        assert_eq!(tuned_timeline[8].1, AiConfig::scripted_defaults());
        // The old tape is untouched by the copy's later commands.
        assert_eq!(plain.replay_rules(16), v1_timeline);
        assert_eq!(plain.command_len(), 0);
    }

    #[test]
    fn stale_child_refreshes_and_stale_versions_refuse() {
        let mut panel = strict_panel();
        panel.set_combat(CombatField::AttackDamage, 240).unwrap();
        let mut child = LoopbackChild::new();
        for _ in 0..5 {
            child.tick();
        }
        panel.stage(child.cursor()).unwrap();
        // The child ticks on before the apply: the batch is stale and the
        // apply writes nothing anywhere.
        child.tick();
        assert!(panel.is_stale(child.cursor()));
        assert_eq!(
            panel.apply(&mut child).unwrap_err(),
            TuneError::StaleChild {
                expected: 6,
                got: 5
            }
        );
        assert_eq!(child.combat_commands(), 0);
        assert_eq!(panel.tape().command_len(), 0);
        assert_eq!(panel.last_receipt(), None);
        // Refresh drops the stale batch; re-stage at the new cursor applies.
        assert!(panel.refresh());
        assert!(!panel.refresh(), "second refresh drops nothing");
        assert!(!panel.is_stale(child.cursor()));
        panel.stage(child.cursor()).unwrap();
        let receipt = panel.apply(&mut child).unwrap();
        assert_eq!(receipt.at_tick, 6);
        assert_eq!(receipt.combat_version, 3, "retry mints the next version");
        assert_eq!(receipt.ai_version, 3, "both lanes mint together");
        // The same versions offered again are stale: last-good kept.
        let mut replay = LoopbackChild::new();
        let swap = panel.preview_combat().unwrap();
        assert_eq!(replay.propose_combat(0, swap.clone()), Ok(4));
        assert_eq!(
            replay.propose_combat(0, swap),
            Err(TuneError::StaleCombat {
                current: 4,
                offered: 4
            })
        );
        let ai = panel.preview_ai().unwrap();
        assert_eq!(replay.propose_ai(0, ai), Ok(4));
        assert_eq!(
            replay.propose_ai(0, ai),
            Err(TuneError::StaleAi {
                current: 4,
                offered: 4
            })
        );
        assert_eq!(
            panel.apply(&mut child).unwrap_err(),
            TuneError::NothingStaged
        );
    }

    #[test]
    fn loopback_orders_commands_and_applies_at_boundaries() {
        let mut child = LoopbackChild::new();
        let swap = SwapInput::new(2, strict_rows()).unwrap();
        // A command below the last recorded tick is refused, log untouched.
        assert_eq!(child.propose_combat(4, swap.clone()), Ok(2));
        assert_eq!(
            child.propose_combat(2, swap.clone()).unwrap_err(),
            TuneError::OutOfOrderTick {
                last: 4,
                offered: 2
            }
        );
        assert_eq!(child.combat_commands(), 1);
        // Invalid AI snapshots never reach the log.
        let mut wild = AiConfig::scripted_defaults();
        wild.morale_default = 9000;
        assert_eq!(
            child.propose_ai(4, wild).unwrap_err(),
            TuneError::Ai(AiCurveError::BadMoraleDefault { value: 9000 })
        );
        assert_eq!(child.ai_commands(), 0);
    }

    #[test]
    fn tape_bounds_and_ordering_fail_loudly() {
        let mut tape = TuneTape::new();
        let mut rows = TABLES_V1;
        rows.parry_window_ticks = 3;
        for version in 2..=(MAX_TAPE_COMMANDS as u32 + 1) {
            let command = TuneCommand {
                at_tick: version,
                combat: SwapInput::new(version, rows).unwrap(),
                ai: AiConfig::scripted_defaults(),
            };
            tape.record_command(command).unwrap();
        }
        assert_eq!(tape.command_len(), MAX_TAPE_COMMANDS);
        let overflow = TuneCommand {
            at_tick: u32::MAX,
            combat: SwapInput::new(u32::MAX, rows).unwrap(),
            ai: AiConfig::scripted_defaults(),
        };
        assert_eq!(
            tape.record_command(overflow).unwrap_err(),
            TuneError::TooManyCommands
        );
        // Out-of-order ticks refuse even with room: order the fresh tape.
        let mut ordered = TuneTape::new();
        ordered
            .record_command(TuneCommand {
                at_tick: 9,
                combat: SwapInput::new(2, rows).unwrap(),
                ai: AiConfig::scripted_defaults(),
            })
            .unwrap();
        assert_eq!(
            ordered
                .record_command(TuneCommand {
                    at_tick: 3,
                    combat: SwapInput::new(3, rows).unwrap(),
                    ai: AiConfig::scripted_defaults(),
                })
                .unwrap_err(),
            TuneError::OutOfOrderTick {
                last: 9,
                offered: 3
            }
        );
        assert_eq!(ordered.command_len(), 1);
    }

    #[test]
    fn shell_roundtrip_stages_applies_refreshes_and_replays() {
        use crate::shell::{Panel, Shell};

        let mut shell = Shell::new();
        assert!(shell.is_open(Panel::Tuning));
        // Invalid edits are refused with their range and logged; the draft
        // they found is kept.
        assert_eq!(
            shell
                .tune_set_combat(CombatField::AttackDamage, 0)
                .unwrap_err(),
            TuneError::CombatRange {
                field: "attack_damage",
                value: 0,
                min: 1,
                max: u32::from(MAX_HP_PERMILLE),
            }
        );
        assert_eq!(
            shell.tune_set_aggression(8, 500).unwrap_err(),
            TuneError::UnknownLane { index: 8 }
        );
        assert_eq!(
            shell.tune_set_engage(49).unwrap_err(),
            TuneError::Ai(AiCurveError::EngageRangeOutOfRange { value: 49 })
        );
        // Tune both lanes, stage and apply at tick zero.
        shell
            .tune_set_combat(CombatField::AttackDamage, 240)
            .unwrap();
        shell
            .tune_set_combat(CombatField::ParryWindowTicks, 3)
            .unwrap();
        shell.tune_set_engage(300).unwrap();
        assert!(shell.tune_preview().is_ok());
        let staged = shell.tune_stage().unwrap();
        assert_eq!((staged.combat_version, staged.ai_version), (2, 2));
        let receipt = shell.tune_apply().unwrap();
        assert_eq!(receipt.at_tick, 0);
        assert_eq!(shell.tune_apply().unwrap_err(), TuneError::NothingStaged);
        assert!(shell.tune_replay_exact());
        // A stale apply refreshes instead of writing: tick on, stage, tick
        // on, apply refuses, refresh plus re-stage applies.
        shell.tune_tick();
        shell.tune_stage().unwrap();
        shell.tune_tick();
        assert!(shell.tune_stale());
        assert_eq!(
            shell.tune_apply().unwrap_err(),
            TuneError::StaleChild {
                expected: 2,
                got: 1
            }
        );
        assert!(shell.tune_refresh());
        shell.tune_stage().unwrap();
        let second = shell.tune_apply().unwrap();
        assert_eq!((second.at_tick, second.combat_version), (2, 4));
        assert!(shell.tune_replay_exact(), "later swaps keep replay exact");
    }

    #[test]
    fn a10_timed_edit_reports_receipt_and_writes_evidence() {
        let (receipt, timing, log, capture) = super::scripted_edit_to_game();
        assert_eq!(receipt.at_tick, super::A10_CURSOR);
        assert_eq!((receipt.combat_version, receipt.ai_version), (2, 2));
        assert!(timing.replay_exact, "tuned fight replays exactly");
        assert_eq!(timing.bad_edits_refused, 3, "bad edits refused");
        // Evidence lands inside the lane checkout only.
        let root = crate::a2_session::checkout_root();
        let dir = root.join("target").join("evidence").join("a10");
        std::fs::create_dir_all(&dir).expect("a10 evidence dir writes");
        std::fs::write(dir.join("timing.log"), &log).expect("timing log writes");
        std::fs::write(dir.join("capture.txt"), &capture).expect("capture writes");
        println!("{timing}");
        println!("{log}");
        assert!(
            timing.elapsed_ms <= 10_000,
            "fixture leg must finish quickly: {timing}"
        );
    }
}
