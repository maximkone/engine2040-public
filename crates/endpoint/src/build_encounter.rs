//! build_encounter: brief text to validated encounter plan (AB3).
//! Owner: `engine-endpoint`; consumer: the endpoint library helpers
//! (`build_encounter`, not an MCP tool; `sim.build_encounter` is a reserved
//! name not registered in `mcp.rs`/`tool_names` today).
//! Units: chars, permille, fixed ticks, ladder rungs `0..=7`. Pure and
//! std-only: integer math, stable order, seeded expansion, no wall clock.
//! Errors are typed: [`BriefError`], [`TableError`], [`ForkBreach`],
//! [`BuildRefusal`] (conflict set after [`MAX_REPAIRS`] typed repairs).

// PQ2 P3 bars, frozen here so the default (dependency-free) endpoint build
// can oracle them without a new crate edge. Values mirror
// `crates/playtest/src/tournament.rs` (`WIN_MIN/MAX_PERMILLE`,
// `KILL_SHARE_MAX_PERMILLE`) and the GA1 ladder (`LADDER_RUNGS` in
// `crates/ai/src/depth.rs`); the orchestrator may promote them to a shared
// contract later. HR3 table fences mirror `crates/combat/src/tables.rs`.
// FK1 fork asserts are consumed through [`ForkOracle`], which the MCP door
// binds to real worker-thread forks; [`DeterministicOracle`] is the pure
// test oracle with the same refusal shape.

use super::brief::{BriefEnvelope, BriefStage};
use engine_core::{hash::FNV64_OFFSET, splitmix64, Fnv64};

/// Reserved door name for this stage; not registered in `mcp.rs`/`tool_names` today.
pub const MCP_TOOL_NAME: &str = "sim.build_encounter";
/// One-line summary for this stage (not advertised by the MCP door today).
pub const MCP_TOOL_SUMMARY: &str =
    "brief text to validated encounter plan: goals, move rows, difficulty rung";
/// Brief bound: chars of free text per encounter request.
pub const BRIEF_MAX_CHARS: usize = 1024;
/// Repair budget: typed repairs per brief, then refuse with the conflict set.
pub const MAX_REPAIRS: usize = 5;
/// P3 bar (PQ2 oracle): lowest acceptable fighter-0 win rate, permille.
pub const WIN_MIN_PERMILLE: u16 = 400;
/// P3 bar (PQ2 oracle): highest acceptable fighter-0 win rate, permille.
pub const WIN_MAX_PERMILLE: u16 = 600;
/// P3 bar (PQ2 oracle): no single move takes more than this kill share.
pub const KILL_SHARE_MAX_PERMILLE: u16 = 350;
/// P3 bar: a stuck fighter holds below fork stuck speed this many window
/// ticks before the window counts as degenerate.
pub const STUCK_WINDOW_TICKS: u32 = 180;
/// Floor (FK1/EA1c shape): a kill faster than this many fork ticks breaches.
pub const MIN_TICKS_TO_KILL: u32 = 450;
/// GA1 ladder size: rungs `0..=MAX_RUNG`.
pub const LADDER_RUNGS: u8 = 8;
/// Highest difficulty rung.
pub const MAX_RUNG: u8 = LADDER_RUNGS - 1;
/// Dwell: consecutive beyond-band reports before the rung moves.
pub const DWELL_REPORTS: u32 = 2;
/// HR3 fence: damage `1..=1000` (full health in permille).
pub const DAMAGE_MAX: u16 = 1000;
/// HR3 fence: startup `1..=60` ticks.
pub const STARTUP_MAX_TICKS: u32 = 60;
/// HR3 fence: recovery `0..=120` ticks.
pub const RECOVERY_MAX_TICKS: u32 = 120;
/// HR3 fence: stamina rows `0..=100`.
pub const STAMINA_MAX: u16 = 100;
/// HR3 fence: dodge duration `1..=120` ticks.
pub const DODGE_MAX_TICKS: u32 = 120;
/// HR3 fence: chip denominator `1..=16`, numerator inside it.
pub const CHIP_DEN_MAX: u32 = 16;
/// HR3 fence: parry window `0..=60` ticks and strictly below startup.
pub const PARRY_MAX_TICKS: u32 = 60;
/// HR3 fence: stagger and hit-react `0..=120` ticks.
pub const STAGGER_MAX_TICKS: u32 = 120;
/// HR3 fence: hit-stop `0..=60` ticks.
pub const HITSTOP_MAX_TICKS: u32 = 60;
/// HR3 fence: knockdown `0..=600` ticks.
pub const KNOCKDOWN_MAX_TICKS: u32 = 600;
/// Canonical fork window the oracle judges, in fixed ticks.
pub const FORK_WINDOW_TICKS: u32 = 600;
/// Shipped table version carried by every expanded draft.
pub const TABLES_VERSION: u32 = 2;

/// Closed goal kinds: the model translates a brief into one of these, never
/// plans in prose. Each maps to a PQ2 morale pair (pacing pressure only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoalKind {
    /// Even duel: symmetric morale, any deviation is mirror-match bias.
    Duel,
    /// Rivalry: one brave fighter against one cautious fighter.
    Rivalry,
    /// Cauldron: maximal morale split, same rules.
    Cauldron,
}

impl GoalKind {
    /// Stable lowercase tag for receipts and digests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duel => "duel",
            Self::Rivalry => "rivalry",
            Self::Cauldron => "cauldron",
        }
    }

    /// PQ2 morale pair for this goal (fighter 0 first).
    pub const fn morale(self) -> [u16; 2] {
        match self {
            Self::Duel => [500, 500],
            Self::Rivalry => [700, 300],
            Self::Cauldron => [1000, 0],
        }
    }
}

/// What stopped a brief parse. Messages name the legal values.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BriefError {
    /// Empty brief: submit 1..=`BRIEF_MAX_CHARS` chars.
    Empty,
    /// Overlong brief: length carried.
    TooLong { len: usize },
}

impl BriefError {
    /// Machine code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Empty => "empty_brief",
            Self::TooLong { .. } => "brief_too_long",
        }
    }
}

impl core::fmt::Display for BriefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty_brief: submit 1..={BRIEF_MAX_CHARS} chars"),
            Self::TooLong { len } => write!(
                f,
                "brief_too_long: brief holds {len} chars; at most {BRIEF_MAX_CHARS}"
            ),
        }
    }
}

impl std::error::Error for BriefError {}

/// Validated brief: bounded text plus the seed it expands under.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncounterBrief {
    text: String,
    seed: u64,
}

impl EncounterBrief {
    /// Validate bounds; overlong or empty briefs refuse with [`BriefError`].
    pub fn new(text: &str, seed: u64) -> Result<Self, BriefError> {
        let len = text.chars().count();
        if len == 0 {
            return Err(BriefError::Empty);
        }
        if len > BRIEF_MAX_CHARS {
            return Err(BriefError::TooLong { len });
        }
        Ok(Self {
            text: text.to_owned(),
            seed,
        })
    }

    /// Bounded brief text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Seed this brief expands under.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Shared LW6 envelope record line for this brief (`BriefEnvelope`
    /// under [`BriefStage::Encounter`]). Every AB3 write carries this
    /// line so the audit traces the brief end to end.
    pub fn record_line(&self) -> String {
        BriefEnvelope::new(&self.text, self.seed, BriefStage::Encounter)
            .map(|env| env.record_line())
            .unwrap_or_default()
    }
}

/// Tunable move rows for one encounter. Same fields and units as the HR3
/// `Tables` row (damage permille, costs in stamina points, timing in fixed
/// ticks); validated by [`validate_tables`] before any fork assert runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveRows {
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

impl MoveRows {
    /// Balanced rows: the shipped-shape middle every repair converges to.
    /// Startup 4 beats parry 3, so the strict HR3 rule holds.
    pub const fn balanced() -> Self {
        Self {
            attack_damage: 120,
            attack_startup_ticks: 4,
            attack_recovery_ticks: 8,
            attack_stamina_cost: 25,
            dodge_stamina_cost: 20,
            block_hold_cost_per_tick: 1,
            stamina_regen_per_tick: 2,
            dodge_duration_ticks: 10,
            dodge_iframes_ticks: 6,
            block_chip_num: 1,
            block_chip_den: 5,
            parry_window_ticks: 3,
            parry_stagger_ticks: 12,
            hit_react_ticks: 6,
            hitstop_ticks: 4,
            knockdown_ticks: 30,
        }
    }
}

/// What stopped table validation. The field name rides along, so a repair
/// (or a refusal) always quotes the legal range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TableError {
    /// Field holding the bad value (`version` for tag 0).
    pub field: &'static str,
    /// The rejected value.
    pub value: u32,
}

impl TableError {
    /// Machine code for this failure.
    pub const fn code(self) -> &'static str {
        "bad_table_value"
    }

    /// Legal range text for the culprit field.
    pub fn legal(self) -> &'static str {
        match self.field {
            "version" => "nonzero",
            "attack_damage" => "1..=1000",
            "attack_startup_ticks" => "1..=60",
            "attack_recovery_ticks" => "0..=120",
            "stamina" => "0..=100",
            "dodge_duration_ticks" => "1..=120",
            "dodge_iframes_ticks" => "0..=dodge_duration",
            "block_chip_den" => "1..=16",
            "block_chip_num" => "0..=den",
            "parry_window_ticks" => "0..=60 and below startup",
            "stagger" => "0..=120",
            "hitstop_ticks" => "0..=60",
            "knockdown_ticks" => "0..=600",
            _ => "in fence",
        }
    }
}

impl core::fmt::Display for TableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "bad_table_value: {} is {} (legal {})",
            self.field,
            self.value,
            self.legal()
        )
    }
}

impl std::error::Error for TableError {}

fn range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), TableError> {
    if value < min || value > max {
        return Err(TableError { field, value });
    }
    Ok(())
}

/// Range-check one versioned table set. Mirrors the HR3 `validate_tables`
/// rules: nonzero version, fenced rows, parry strictly below startup so a
/// held block ages out of the parry window before the next impact. First
/// failure wins, deterministically.
pub fn validate_tables(version: u32, rows: &MoveRows) -> Result<(), TableError> {
    if version == 0 {
        return Err(TableError {
            field: "version",
            value: 0,
        });
    }
    range(
        "attack_damage",
        u32::from(rows.attack_damage),
        1,
        u32::from(DAMAGE_MAX),
    )?;
    range(
        "attack_startup_ticks",
        rows.attack_startup_ticks,
        1,
        STARTUP_MAX_TICKS,
    )?;
    range(
        "attack_recovery_ticks",
        rows.attack_recovery_ticks,
        0,
        RECOVERY_MAX_TICKS,
    )?;
    range(
        "attack_stamina_cost",
        u32::from(rows.attack_stamina_cost),
        0,
        u32::from(STAMINA_MAX),
    )?;
    range(
        "dodge_stamina_cost",
        u32::from(rows.dodge_stamina_cost),
        0,
        u32::from(STAMINA_MAX),
    )?;
    range(
        "block_hold_cost_per_tick",
        u32::from(rows.block_hold_cost_per_tick),
        0,
        u32::from(STAMINA_MAX),
    )?;
    range(
        "stamina_regen_per_tick",
        u32::from(rows.stamina_regen_per_tick),
        0,
        u32::from(STAMINA_MAX),
    )?;
    range(
        "dodge_duration_ticks",
        rows.dodge_duration_ticks,
        1,
        DODGE_MAX_TICKS,
    )?;
    range(
        "dodge_iframes_ticks",
        rows.dodge_iframes_ticks,
        0,
        rows.dodge_duration_ticks,
    )?;
    range("block_chip_den", rows.block_chip_den, 1, CHIP_DEN_MAX)?;
    range(
        "block_chip_num",
        rows.block_chip_num,
        0,
        rows.block_chip_den,
    )?;
    range(
        "parry_window_ticks",
        rows.parry_window_ticks,
        0,
        PARRY_MAX_TICKS,
    )?;
    if rows.parry_window_ticks >= rows.attack_startup_ticks {
        return Err(TableError {
            field: "parry_window_ticks",
            value: rows.parry_window_ticks,
        });
    }
    range("stagger", rows.parry_stagger_ticks, 0, STAGGER_MAX_TICKS)?;
    range("stagger", rows.hit_react_ticks, 0, STAGGER_MAX_TICKS)?;
    range("hitstop_ticks", rows.hitstop_ticks, 0, HITSTOP_MAX_TICKS)?;
    range(
        "knockdown_ticks",
        rows.knockdown_ticks,
        0,
        KNOCKDOWN_MAX_TICKS,
    )?;
    Ok(())
}

/// One fork window sample: the metrics the FK1 asserts judge. Integer
/// metrics only, so checks are exact on every machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ForkSample {
    /// Fighter-0 win rate in permille over the window seeds.
    pub win_rate_permille: u16,
    /// Largest single-move kill share in permille.
    pub top_kill_share_permille: u16,
    /// Ticks the stillest fighter held below stuck speed.
    pub stuck_ticks: u32,
    /// Fork-relative ticks to the first kill, `None` when bloodless.
    pub ticks_to_kill: Option<u32>,
}

/// Balance rejection: which metric breached, measured against which bound.
/// The fork id travels along so reviewers can rerun the exact window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ForkBreach {
    /// Breaching metric (`win_rate`, `kill_share`, `stuck_ticks`,
    /// `ticks_to_kill`).
    pub metric: &'static str,
    /// Measured value in metric units.
    pub value: u64,
    /// The breached bound in the same units.
    pub bound: u64,
    /// Human detail quoting metric, value, bound and fork id.
    pub reason: String,
}

impl ForkBreach {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self.metric {
            "win_rate" => "win_rate_breach",
            "kill_share" => "kill_share_breach",
            "stuck_ticks" => "stuck_breach",
            _ => "fast_kill_breach",
        }
    }
}

impl core::fmt::Display for ForkBreach {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.code(), self.reason)
    }
}

impl std::error::Error for ForkBreach {}

/// FK1 fork asserts over one window sample: the PQ2 P3 bars. First breach
/// wins in win-rate, kill-share, stuck, fast-kill order. A bloodless window
/// with no stuck fighter passes; a degenerate or dominant one is refused.
pub fn check_fork(fork_id: u64, sample: &ForkSample) -> Result<(), ForkBreach> {
    if sample.win_rate_permille < WIN_MIN_PERMILLE || sample.win_rate_permille > WIN_MAX_PERMILLE {
        return Err(ForkBreach {
            metric: "win_rate",
            value: u64::from(sample.win_rate_permille),
            bound: u64::from(WIN_MAX_PERMILLE),
            reason: format!(
                "win_rate {} permille outside [{WIN_MIN_PERMILLE}, {WIN_MAX_PERMILLE}] \
                 (fork {fork_id}; even duels sit near 500, splits stay inside)",
                sample.win_rate_permille,
            ),
        });
    }
    if sample.top_kill_share_permille > KILL_SHARE_MAX_PERMILLE {
        return Err(ForkBreach {
            metric: "kill_share",
            value: u64::from(sample.top_kill_share_permille),
            bound: u64::from(KILL_SHARE_MAX_PERMILLE),
            reason: format!(
                "kill_share {} permille over the {KILL_SHARE_MAX_PERMILLE} cap \
                 (fork {fork_id}; no move takes more than 35% of kills)",
                sample.top_kill_share_permille,
            ),
        });
    }
    if sample.stuck_ticks >= STUCK_WINDOW_TICKS {
        return Err(ForkBreach {
            metric: "stuck_ticks",
            value: u64::from(sample.stuck_ticks),
            bound: u64::from(STUCK_WINDOW_TICKS),
            reason: format!(
                "stuck_ticks {} of the window still (fork {fork_id}; \
                 zero stuck fighters, post-kill holds excluded)",
                sample.stuck_ticks,
            ),
        });
    }
    if let Some(kill) = sample.ticks_to_kill {
        if kill < MIN_TICKS_TO_KILL {
            return Err(ForkBreach {
                metric: "ticks_to_kill",
                value: u64::from(kill),
                bound: u64::from(MIN_TICKS_TO_KILL),
                reason: format!(
                    "ticks_to_kill killed in {kill} ticks, faster than the \
                     {MIN_TICKS_TO_KILL}-tick floor (fork {fork_id})",
                ),
            });
        }
    }
    Ok(())
}

/// Draft encounter: closed goal plus explicit rows and rung, before gates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DraftEncounter {
    /// Translated goal kind.
    pub goal: GoalKind,
    /// Per-fighter morale in permille, fighter 0 first.
    pub morale: [u16; 2],
    /// Move rows under test.
    pub rows: MoveRows,
    /// Requested difficulty rung `0..=MAX_RUNG`.
    pub rung: u8,
    /// True when the brief asked for a frozen tableau (stuck by construction).
    pub stuck_risk: bool,
}

/// Case-insensitive whole-word hit for one keyword.
fn wants(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token.eq_ignore_ascii_case(word))
}

/// Translate a brief into its closed goal enum. Unknown words fall back to
/// [`GoalKind::Duel`]: the plan language stays closed no matter what prose
/// arrives.
fn translate_goal(text: &str) -> GoalKind {
    if wants(text, "cauldron") || wants(text, "brutal") {
        GoalKind::Cauldron
    } else if wants(text, "rivalry")
        || wants(text, "rival")
        || wants(text, "pressure")
        || wants(text, "grudge")
    {
        GoalKind::Rivalry
    } else {
        GoalKind::Duel
    }
}

/// Translate a brief into its difficulty rung. Unknown words sit at rung 3,
/// the middle of the GA1 ladder.
fn translate_rung(text: &str) -> u8 {
    if wants(text, "easy") || wants(text, "training") || wants(text, "quiet") {
        0
    } else if wants(text, "hard")
        || wants(text, "brutal")
        || wants(text, "cauldron")
        || wants(text, "final")
    {
        7
    } else if wants(text, "advanced") || wants(text, "pressure") || wants(text, "noon") {
        5
    } else {
        3
    }
}

/// Pure expander: brief plus seed to draft. Same brief plus same seed always
/// yields the same draft on every machine. Adversarial keywords
/// (`overpowered`, `broken`, `freeze`) are honored literally so the gates
/// have something to refuse or repair: the translator never silently heals.
pub fn expand(brief: &EncounterBrief) -> DraftEncounter {
    let text = brief.text();
    let goal = translate_goal(text);
    let rung = translate_rung(text);
    // Non-aliasing stream fork: each field draws its own SplitMix64 lane off
    // the brief seed, so fields never share a draw.
    let lane = brief.seed().wrapping_add(0xab3u64);
    let damage_draw = splitmix64(lane);
    let mut rows = MoveRows::balanced();
    // Seeded damage wobble inside the balanced band: 80..=160 permille.
    rows.attack_damage = 80 + (damage_draw % 81) as u16;
    if wants(text, "overpowered") || wants(text, "oneshot") || wants(text, "cursed") {
        rows.attack_damage = 900;
    }
    if wants(text, "broken") || wants(text, "unblockable") {
        // Parry at startup: trips the strict parry-below-startup rule.
        rows.parry_window_ticks = rows.attack_startup_ticks;
    }
    if wants(text, "exhausting") || wants(text, "marathon") {
        rows.attack_recovery_ticks = 120;
    }
    DraftEncounter {
        goal,
        morale: goal.morale(),
        rows,
        rung,
        stuck_risk: wants(text, "freeze") || wants(text, "frozen") || wants(text, "statue"),
    }
}

/// Fork oracle: turns a draft into the window sample the FK1 asserts judge.
/// The planner binds this to real worker-thread forks; [`build_with_budget`]
/// and [`build_encounter`] take the pure [`DeterministicOracle`] directly
/// until a second oracle lands.
pub trait ForkOracle {
    /// Sample one fork window for this draft.
    fn sample(&self, draft: &DraftEncounter) -> ForkSample;
}

/// Pure test oracle with the PQ2 refusal shape. Integer-only estimator:
/// morale split tilts the rate gently (the measured symmetric band), damage
/// past 200 permille dominates it, kill share tracks damage plus rung, fast
/// kills arrive with one-shot damage, stuck windows with frozen tableaux.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DeterministicOracle;

impl ForkOracle for DeterministicOracle {
    fn sample(&self, draft: &DraftEncounter) -> ForkSample {
        let diff = i32::from(draft.morale[0]) - i32::from(draft.morale[1]);
        let morale_term = diff / 16;
        let over = i32::from(draft.rows.attack_damage) - 200;
        let damage_term = if over > 0 { over / 2 } else { 0 };
        let rate = (500 + morale_term + damage_term).clamp(0, 1000) as u16;
        let share = (180 + i32::from(draft.rows.attack_damage) / 2 + i32::from(draft.rung) * 4)
            .clamp(0, 1000) as u16;
        ForkSample {
            win_rate_permille: rate,
            top_kill_share_permille: share,
            stuck_ticks: if draft.stuck_risk {
                FORK_WINDOW_TICKS
            } else {
                0
            },
            ticks_to_kill: if draft.rows.attack_damage >= 400 {
                Some(120)
            } else {
                None
            },
        }
    }
}

/// Typed repairs, applied at most once each per brief, in failure order.
/// Every variant names its own fix, so the receipt (or refusal) says what
/// the loop did instead of chatting forever.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repair {
    /// Set attack damage back to the balanced 120 permille.
    ClampDamage,
    /// Set the parry window one tick below startup.
    FixParryWindow,
    /// Cut damage to the balanced band and lengthen recovery.
    SoftenBlows,
    /// Drop the frozen tableau: the window plays again.
    Unstick,
    /// Step the requested rung down two (bottom saturates).
    DropRung,
}

impl Repair {
    /// Stable lowercase tag for receipts and refusals.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClampDamage => "clamp_damage",
            Self::FixParryWindow => "fix_parry_window",
            Self::SoftenBlows => "soften_blows",
            Self::Unstick => "unstick",
            Self::DropRung => "drop_rung",
        }
    }
}

/// Apply one typed repair to a draft. Pure and total: every repair fits
/// every draft, so the loop never stalls on an inapplicable fix.
fn apply_repair(draft: &mut DraftEncounter, repair: Repair) {
    match repair {
        Repair::ClampDamage => {
            draft.rows.attack_damage = 120;
        }
        Repair::FixParryWindow => {
            draft.rows.parry_window_ticks = draft.rows.attack_startup_ticks.saturating_sub(1);
        }
        Repair::SoftenBlows => {
            draft.rows.attack_damage = draft.rows.attack_damage.min(120);
            draft.rows.attack_recovery_ticks =
                (draft.rows.attack_recovery_ticks + 12).min(RECOVERY_MAX_TICKS);
        }
        Repair::Unstick => {
            draft.stuck_risk = false;
        }
        Repair::DropRung => {
            draft.rung = draft.rung.saturating_sub(2);
        }
    }
}

/// Pick the next repair for a failure the gates just reported. Table
/// failures heal their own field; fork breaches heal their own metric, so
/// each round fixes what the findings name.
fn repair_for_table(error: &TableError) -> Repair {
    match error.field {
        "parry_window_ticks" => Repair::FixParryWindow,
        "attack_damage" => Repair::ClampDamage,
        _ => Repair::SoftenBlows,
    }
}

/// Pick the next repair for a fork breach. Win-rate and fast-kill breaches
/// come from dominant damage; kill-share breaches from hot moves; stuck
/// windows from frozen tableaux; a hot high rung cools two steps.
fn repair_for_breach(breach: &ForkBreach, draft: &DraftEncounter) -> Repair {
    match breach.metric {
        "win_rate" | "ticks_to_kill" => Repair::ClampDamage,
        "kill_share" => {
            if draft.rung >= 6 {
                Repair::DropRung
            } else {
                Repair::SoftenBlows
            }
        }
        _ => Repair::Unstick,
    }
}

/// Landed encounter plan: the validated draft plus its fork id, rung and
/// repair receipt.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BuiltEncounter {
    /// Deterministic plan id over brief text, seed, goal and rows.
    pub plan_id: u64,
    /// Translated goal kind.
    pub goal: GoalKind,
    /// Validated move rows.
    pub rows: MoveRows,
    /// Final difficulty rung.
    pub rung: u8,
    /// Fork id of the passing window sample.
    pub fork_id: u64,
    /// Passing window sample.
    pub sample: ForkSample,
    /// Repairs applied, in order (empty when the draft passed clean).
    pub repairs_used: Vec<Repair>,
}

/// Refusal: the conflict set after the repair budget ran out (or before it
/// started, for untranslatable briefs). Every entry quotes the violated
/// invariant plus one recovery, so authors can fix the brief.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BuildRefusal {
    /// Conflicting findings in gate order, each `metric: detail`.
    pub conflicts: Vec<String>,
    /// Why the plan is refused, with the violated bound quoted.
    pub reason: String,
    /// Repairs spent before refusing (`MAX_REPAIRS` at most).
    pub repairs_used: Vec<Repair>,
}

impl BuildRefusal {
    /// Machine code for this failure.
    pub const fn code(&self) -> &'static str {
        "encounter_refused"
    }
}

impl core::fmt::Display for BuildRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "encounter_refused: {}; conflicts: [{}]",
            self.reason,
            self.conflicts.join(", ")
        )
    }
}

impl std::error::Error for BuildRefusal {}

/// Deterministic plan id: FNV-1a over brief text, seed, goal tag, morale,
/// every row field and the rung. Same inputs, same id on every machine.
fn plan_id(brief: &EncounterBrief, draft: &DraftEncounter) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(brief.text().as_bytes());
    hasher.write(&brief.seed().to_le_bytes());
    hasher.write(draft.goal.as_str().as_bytes());
    for morale in draft.morale {
        hasher.write(&morale.to_le_bytes());
    }
    let rows = &draft.rows;
    hasher.write(&rows.attack_damage.to_le_bytes());
    hasher.write(&rows.attack_startup_ticks.to_le_bytes());
    hasher.write(&rows.attack_recovery_ticks.to_le_bytes());
    hasher.write(&rows.attack_stamina_cost.to_le_bytes());
    hasher.write(&rows.dodge_stamina_cost.to_le_bytes());
    hasher.write(&rows.block_hold_cost_per_tick.to_le_bytes());
    hasher.write(&rows.stamina_regen_per_tick.to_le_bytes());
    hasher.write(&rows.dodge_duration_ticks.to_le_bytes());
    hasher.write(&rows.dodge_iframes_ticks.to_le_bytes());
    hasher.write(&rows.block_chip_num.to_le_bytes());
    hasher.write(&rows.block_chip_den.to_le_bytes());
    hasher.write(&rows.parry_window_ticks.to_le_bytes());
    hasher.write(&rows.parry_stagger_ticks.to_le_bytes());
    hasher.write(&rows.hit_react_ticks.to_le_bytes());
    hasher.write(&rows.hitstop_ticks.to_le_bytes());
    hasher.write(&rows.knockdown_ticks.to_le_bytes());
    hasher.write(&[draft.rung]);
    hasher.finish()
}

/// Deterministic fork id: FNV-1a over the plan id plus the window length.
fn fork_id(plan: u64, ticks: u32) -> u64 {
    let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ ab3encounter_fork_id());
    hasher.write(&plan.to_le_bytes());
    hasher.write(&ticks.to_le_bytes());
    hasher.finish()
}

/// Domain separator for fork ids (kept apart from plan ids by construction).
const fn ab3encounter_fork_id() -> u64 {
    0x0e1c_0ab3_0000_0001
}

/// Build one encounter: translate, expand, validate tables, assert the fork,
/// repairing at most `budget` times before refusing with the conflict set.
/// The default entry is [`build_encounter`] with the default repair budget.
/// Monomorphized on [`DeterministicOracle`]: the only oracle binds here until
/// a second oracle lands, so no generic stays for one call shape.
pub fn build_with_budget(
    brief: &EncounterBrief,
    oracle: &DeterministicOracle,
    budget: usize,
) -> Result<BuiltEncounter, BuildRefusal> {
    let mut draft = expand(brief);
    let mut repairs_used: Vec<Repair> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    loop {
        if let Err(error) = validate_tables(TABLES_VERSION, &draft.rows) {
            let repair = repair_for_table(&error);
            conflicts.push(format!("table {error}"));
            if repairs_used.contains(&repair) || repairs_used.len() >= budget {
                return Err(BuildRefusal {
                    reason: format!(
                        "move rows invalid after {} repairs: {error} \
                         (legal {})",
                        repairs_used.len(),
                        error.legal(),
                    ),
                    conflicts,
                    repairs_used,
                });
            }
            apply_repair(&mut draft, repair);
            repairs_used.push(repair);
            continue;
        }
        let plan = plan_id(brief, &draft);
        let id = fork_id(plan, FORK_WINDOW_TICKS);
        let sample = oracle.sample(&draft);
        match check_fork(id, &sample) {
            Ok(()) => {
                return Ok(BuiltEncounter {
                    plan_id: plan,
                    goal: draft.goal,
                    rows: draft.rows,
                    rung: draft.rung,
                    fork_id: id,
                    sample,
                    repairs_used,
                });
            }
            Err(breach) => {
                let repair = repair_for_breach(&breach, &draft);
                conflicts.push(format!("fork {breach}"));
                if repairs_used.contains(&repair) || repairs_used.len() >= budget {
                    return Err(BuildRefusal {
                        reason: format!(
                            "fork still breaches after {} repairs: {breach} \
                             (win-rate band [{WIN_MIN_PERMILLE}, {WIN_MAX_PERMILLE}], \
                             kill-share cap {KILL_SHARE_MAX_PERMILLE})",
                            repairs_used.len(),
                        ),
                        conflicts,
                        repairs_used,
                    });
                }
                apply_repair(&mut draft, repair);
                repairs_used.push(repair);
            }
        }
    }
}

/// Build one encounter under the default repair budget: at most [`MAX_REPAIRS`]
/// typed repairs, then refuse with the conflict set.
/// Monomorphized on [`DeterministicOracle`], same as [`build_with_budget`].
pub fn build_encounter(
    brief: &EncounterBrief,
    oracle: &DeterministicOracle,
) -> Result<BuiltEncounter, BuildRefusal> {
    build_with_budget(brief, oracle, MAX_REPAIRS)
}

/// Difficulty signal for one judged window: player-side win rate in permille.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RungSignal {
    /// Player dominates: climb on dwell.
    Climb,
    /// Player struggles: descend on dwell.
    Descend,
    /// Inside the band: hold and reset dwell.
    Hold,
}

impl RungSignal {
    /// Map a win rate through the hysteresis band: above the P3 cap climbs,
    /// below the P3 floor descends, inside holds. The band (not a line) is
    /// what stops one noisy window from flapping the ladder.
    pub const fn from_rate(rate_permille: u16) -> Self {
        if rate_permille > WIN_MAX_PERMILLE {
            Self::Climb
        } else if rate_permille < WIN_MIN_PERMILLE {
            Self::Descend
        } else {
            Self::Hold
        }
    }
}

/// Difficulty rung with hysteresis and dwell: the GA1 ladder rung moves only
/// after [`DWELL_REPORTS`] consecutive beyond-band signals, and any in-band
/// report resets the streak. Rungs saturate at `0` and [`MAX_RUNG`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RungController {
    rung: u8,
    climbs: u32,
    descends: u32,
}

/// What stopped a rung bind. Messages name the legal values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RungError {
    /// Unknown rung: submit 0..=`MAX_RUNG`.
    Unknown { rung: u8 },
}

impl RungError {
    /// Machine code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unknown { .. } => "unknown_rung",
        }
    }
}

impl core::fmt::Display for RungError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown { rung } => {
                write!(f, "unknown rung {rung}: submit 0..={MAX_RUNG} (GA1 ladder)")
            }
        }
    }
}

impl std::error::Error for RungError {}

impl RungController {
    /// New controller at `rung`; unknown rungs refuse loudly.
    pub fn new(rung: u8) -> Result<Self, RungError> {
        if rung >= LADDER_RUNGS {
            return Err(RungError::Unknown { rung });
        }
        Ok(Self {
            rung,
            climbs: 0,
            descends: 0,
        })
    }

    /// Current rung.
    pub const fn rung(&self) -> u8 {
        self.rung
    }

    /// Observe one judged signal. Returns the rung after dwell and clamps.
    pub fn observe(&mut self, signal: RungSignal) -> u8 {
        match signal {
            RungSignal::Climb => {
                self.climbs += 1;
                self.descends = 0;
                if self.climbs >= DWELL_REPORTS {
                    self.rung = self.rung.saturating_add(1).min(MAX_RUNG);
                    self.climbs = 0;
                }
            }
            RungSignal::Descend => {
                self.descends += 1;
                self.climbs = 0;
                if self.descends >= DWELL_REPORTS {
                    self.rung = self.rung.saturating_sub(1);
                    self.descends = 0;
                }
            }
            RungSignal::Hold => {
                self.climbs = 0;
                self.descends = 0;
            }
        }
        self.rung
    }
}

/// Review-pipeline dedup binds (AB3): single owner in [`super::canon`];
/// re-exported here so `build_encounter::review_variants` keeps its path
/// with zero behavior change. Offline authoring only, never in the tick.
pub use super::canon::{collapse_for_review, review_variants};

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(text: &str) -> EncounterBrief {
        EncounterBrief::new(text, 2040).expect("fixture brief is valid")
    }

    #[test]
    fn ten_briefs_land_inside_win_rate_bounds() {
        let oracle = DeterministicOracle;
        let texts = [
            "a quiet duel at dawn in the training yard",
            "a rivalry grudge match under the noon sun",
            "a brutal cauldron final before the crowd",
            "an advanced duel with footwork drills",
            "a training duel for the new recruits",
            "a pressure rivalry between rival houses",
            "an easy sparring circle for beginners",
            "a hard final between champions",
            "a marathon duel of endurance",
            "an exhausting rivalry across the sand",
        ];
        assert_eq!(texts.len(), 10);
        for (index, text) in texts.iter().enumerate() {
            let brief =
                EncounterBrief::new(text, 2040 + index as u64).expect("ten-brief fixture is valid");
            let built = build_encounter(&brief, &oracle).expect("brief lands");
            assert!(
                (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&built.sample.win_rate_permille),
                "brief {index} rate {} outside the P3 band",
                built.sample.win_rate_permille,
            );
            assert!(
                built.sample.top_kill_share_permille <= KILL_SHARE_MAX_PERMILLE,
                "brief {index} share {} over the cap",
                built.sample.top_kill_share_permille,
            );
            assert_eq!(built.sample.stuck_ticks, 0, "brief {index} stuck");
            assert!(built.repairs_used.len() <= MAX_REPAIRS, "repairs bounded");
            // Same brief twice: identical plan (determinism scope).
            let again = build_encounter(&brief, &oracle).expect("replay lands");
            assert_eq!(built, again, "brief {index} replays exactly");
        }
    }

    #[test]
    fn fork_refuses_overpowered_plan() {
        let draft = expand(&brief("an overpowered duel with oneshot blades"));
        assert_eq!(draft.rows.attack_damage, 900);
        let oracle = DeterministicOracle;
        let sample = oracle.sample(&draft);
        let breach = check_fork(7, &sample).expect_err("one-shot damage breaches");
        assert_eq!(breach.metric, "win_rate");
        // Zero repair budget: the loop refuses with the conflict set.
        let refused = build_with_budget(&brief("an overpowered duel"), &oracle, 0)
            .expect_err("no budget means refuse");
        assert_eq!(refused.code(), "encounter_refused");
        assert!(refused.repairs_used.is_empty());
        assert!(!refused.conflicts.is_empty());
        assert!(refused.reason.contains("win-rate band"));
    }

    #[test]
    fn adversarial_brief_self_repairs_within_budget() {
        let oracle = DeterministicOracle;
        let brief = brief("an overpowered broken frozen duel of statues");
        let built = build_encounter(&brief, &oracle).expect("repairs converge");
        assert!(!built.repairs_used.is_empty(), "repairs were needed");
        assert!(built.repairs_used.len() <= MAX_REPAIRS, "repairs bounded");
        let tags: Vec<&str> = built.repairs_used.iter().map(|r| r.as_str()).collect();
        assert!(tags.contains(&"clamp_damage"), "damage repaired: {tags:?}");
        assert!(
            tags.contains(&"fix_parry_window"),
            "parry repaired: {tags:?}"
        );
        assert!(tags.contains(&"unstick"), "tableau plays: {tags:?}");
        assert!(
            (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&built.sample.win_rate_permille),
            "repaired rate {} lands",
            built.sample.win_rate_permille,
        );
        assert_eq!(built.sample.stuck_ticks, 0, "tableau plays again");
        assert!(
            validate_tables(TABLES_VERSION, &built.rows).is_ok(),
            "repaired rows validate"
        );
    }

    #[test]
    fn kill_share_and_fast_kill_breaches_name_their_metric() {
        let fast = ForkSample {
            win_rate_permille: 500,
            top_kill_share_permille: 100,
            stuck_ticks: 0,
            ticks_to_kill: Some(30),
        };
        let breach = check_fork(11, &fast).expect_err("fast kill breaches");
        assert_eq!(breach.metric, "ticks_to_kill");
        let hot = ForkSample {
            win_rate_permille: 500,
            top_kill_share_permille: 900,
            stuck_ticks: 0,
            ticks_to_kill: None,
        };
        let breach = check_fork(12, &hot).expect_err("hot move breaches");
        assert_eq!(breach.metric, "kill_share");
        let stuck = ForkSample {
            win_rate_permille: 500,
            top_kill_share_permille: 100,
            stuck_ticks: FORK_WINDOW_TICKS,
            ticks_to_kill: None,
        };
        let breach = check_fork(13, &stuck).expect_err("stuck window breaches");
        assert_eq!(breach.metric, "stuck_ticks");
        let clean = ForkSample {
            win_rate_permille: 500,
            top_kill_share_permille: 200,
            stuck_ticks: 0,
            ticks_to_kill: None,
        };
        assert!(check_fork(14, &clean).is_ok(), "clean window passes");
    }

    #[test]
    fn validate_tables_rejects_bad_rows_with_field_names() {
        let mut rows = MoveRows::balanced();
        assert!(validate_tables(TABLES_VERSION, &rows).is_ok());
        assert!(validate_tables(0, &rows).is_err(), "version 0 refuses");
        rows.attack_damage = 0;
        let error = validate_tables(TABLES_VERSION, &rows).expect_err("zero damage");
        assert_eq!(error.field, "attack_damage");
        assert_eq!(error.code(), "bad_table_value");
        rows = MoveRows::balanced();
        rows.parry_window_ticks = rows.attack_startup_ticks;
        let error = validate_tables(TABLES_VERSION, &rows).expect_err("parry at startup");
        assert_eq!(error.field, "parry_window_ticks");
        rows = MoveRows::balanced();
        rows.block_chip_num = rows.block_chip_den + 1;
        assert!(
            validate_tables(TABLES_VERSION, &rows).is_err(),
            "chip over den refuses"
        );
    }

    #[test]
    fn rung_holds_single_spikes_and_moves_on_dwell() {
        let mut ladder = RungController::new(3).expect("rung 3 ships");
        assert_eq!(ladder.rung(), 3);
        // One spike never moves the rung (dwell).
        assert_eq!(ladder.observe(RungSignal::Climb), 3);
        // Two consecutive climbs move exactly one rung.
        assert_eq!(ladder.observe(RungSignal::Climb), 4);
        // In-band reports reset dwell (hysteresis).
        assert_eq!(ladder.observe(RungSignal::Hold), 4);
        assert_eq!(ladder.observe(RungSignal::Climb), 4);
        // Flapping never moves: climb, descend, climb stays put.
        ladder.observe(RungSignal::Descend);
        assert_eq!(ladder.observe(RungSignal::Climb), 4);
        // Two descends move down one rung.
        ladder.observe(RungSignal::Descend);
        assert_eq!(ladder.observe(RungSignal::Descend), 3);
        // Band mapping: inside holds, outside climbs or descends.
        assert_eq!(RungSignal::from_rate(500), RungSignal::Hold);
        assert_eq!(RungSignal::from_rate(601), RungSignal::Climb);
        assert_eq!(RungSignal::from_rate(399), RungSignal::Descend);
    }

    #[test]
    fn rung_saturates_at_both_ends_and_refuses_unknown() {
        assert!(RungController::new(LADDER_RUNGS).is_err(), "rung 8 refuses");
        let mut top = RungController::new(MAX_RUNG).expect("top rung ships");
        top.observe(RungSignal::Climb);
        assert_eq!(top.observe(RungSignal::Climb), MAX_RUNG, "top saturates");
        let mut bottom = RungController::new(0).expect("bottom rung ships");
        bottom.observe(RungSignal::Descend);
        assert_eq!(bottom.observe(RungSignal::Descend), 0, "bottom saturates");
    }

    #[test]
    fn bad_briefs_refuse_before_any_repair() {
        assert!(EncounterBrief::new("", 1).is_err(), "empty refuses");
        let long = "x".repeat(BRIEF_MAX_CHARS + 1);
        let error = EncounterBrief::new(&long, 1).expect_err("overlong refuses");
        assert_eq!(error.code(), "brief_too_long");
    }

    #[test]
    fn translator_stays_closed_on_unknown_prose() {
        // Words the translator never heard still land on a closed goal.
        let draft = expand(&brief("quantum banquet of purple jurisprudence"));
        assert_eq!(draft.goal, GoalKind::Duel);
        assert_eq!(draft.morale, [500, 500]);
        assert_eq!(draft.rung, 3);
        let built = build_encounter(
            &brief("quantum banquet of purple jurisprudence"),
            &DeterministicOracle,
        )
        .expect("unknown prose still lands");
        assert_eq!(built.goal, GoalKind::Duel);
        assert!(built.repairs_used.is_empty(), "clean draft needs no repair");
    }

    #[test]
    fn mcp_hook_names_the_tool() {
        assert_eq!(MCP_TOOL_NAME, "sim.build_encounter");
        assert!(!MCP_TOOL_SUMMARY.is_empty());
    }

    #[test]
    fn review_pipeline_collapses_seeded_sets_by_thirty_percent() {
        let all = super::super::canon::thirty_sets();
        assert_eq!(all.len(), 240);
        let report = review_variants(&all).expect("review dedup lands");
        assert!(
            report.meets_bar,
            "need >=30% reduction, got {}/{} ({} permille)",
            report.candidate, report.baseline, report.reduction_permille
        );
        assert!(report.reduction_permille >= super::super::canon::REDUCTION_MIN_PERMILLE);
        let collapsed = collapse_for_review(&all).expect("collapse lands");
        assert_eq!(
            collapsed.len(),
            report.candidate,
            "one representative per key"
        );
        let again = collapse_for_review(&all).expect("replay lands");
        assert_eq!(collapsed, again);
    }

    #[test]
    fn review_pipeline_keeps_keypoint_distinct_scenes_split() {
        let mut distinct: Vec<String> = Vec::new();
        for i in 0..30u64 {
            distinct.push(format!("prop crate key_{i:02}\nspawn key_{i:02}"));
        }
        let report = review_variants(&distinct).expect("distinct dedup lands");
        assert_eq!(report.candidate, 30, "distinct scenes never merge");
        assert_eq!(report.reduction_permille, 0);
        let collapsed = collapse_for_review(&distinct).expect("collapse lands");
        assert_eq!(collapsed.len(), 30);
        let pair = [
            "a quiet duel at dawn".to_owned(),
            "a brutal cauldron final".to_owned(),
        ];
        let split = review_variants(&pair).expect("pair lands");
        assert_eq!((split.baseline, split.candidate), (2, 2));
        assert_eq!(collapse_for_review(&pair).expect("pair collapse").len(), 2);
    }

    #[test]
    fn review_pipeline_refuses_bad_inputs_with_codes() {
        assert_eq!(
            review_variants(&["   ".to_owned()])
                .expect_err("blank")
                .code(),
            "empty_variant"
        );
        assert_eq!(
            collapse_for_review(&["   ".to_owned()])
                .expect_err("blank collapse")
                .code(),
            "empty_variant"
        );
        let long = "x".repeat(super::super::canon::MAX_CHARS + 1);
        assert_eq!(
            review_variants(std::slice::from_ref(&long))
                .expect_err("overlong")
                .code(),
            "variant_too_long"
        );
        assert_eq!(
            collapse_for_review(&[long])
                .expect_err("overlong collapse")
                .code(),
            "variant_too_long"
        );
        let many = vec!["prop crate a\nspawn a".to_owned(); super::super::canon::MAX_VARIANTS + 1];
        assert_eq!(
            review_variants(&many).expect_err("too many").code(),
            "too_many_variants"
        );
        assert_eq!(
            collapse_for_review(&many)
                .expect_err("too many collapse")
                .code(),
            "too_many_variants"
        );
    }
}
