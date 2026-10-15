//! plan_goals: brief to goals in a small planning language, solved by a
//! classical planner in plain Rust. Owner is `engine-endpoint`; the consumer
//! is the MCP door (`sim.plan_goals`, see [`hook_name`]). Units are chars of
//! brief text, permille, fixed ticks and ladder rungs `0..=7`. Pure and
//! std-only: integer math, stable order, seeded expansion, no wall clock.
//! The model translates a brief into the closed [`Goal`] set only; search
//! owns the operator sequence and the fork check owns refusal. AB3 shapes
//! (`BuiltEncounter`, `BuildRefusal`, `Repair`) are reused, never mirrored.

use std::collections::{BTreeSet, VecDeque};

use engine_core::{hash::FNV64_OFFSET, Fnv64};

use crate::brief::{BriefEnvelope, BriefStage};
use crate::build_encounter::{
    expand, validate_tables, BuildRefusal, BuiltEncounter, DraftEncounter, EncounterBrief,
    ForkOracle, ForkSample, GoalKind, Repair, FORK_WINDOW_TICKS, KILL_SHARE_MAX_PERMILLE,
    MIN_TICKS_TO_KILL, STUCK_WINDOW_TICKS, TABLES_VERSION, WIN_MAX_PERMILLE, WIN_MIN_PERMILLE,
};

/// MCP door hook: register this module under this tool name.
pub const MCP_TOOL_NAME: &str = "sim.plan_goals";
/// One-line summary the MCP door shows for this tool.
pub const MCP_TOOL_SUMMARY: &str =
    "brief to planned encounter: closed goals, classical search, fork-checked";
/// Attempt budget: at most this many typed operators per brief, then refuse
/// with the conflict set. Matches the AB3 typed-repair budget by construction
/// (see `attempts_match_ab3_budget`).
pub const MAX_ATTEMPTS: usize = 5;

/// One-line router hook for the orchestrator: register the plan-goals tool on
/// the nineteen-tool router in `mcp.rs` and bump `tool_names` plus the
/// tool-list budget test. Returned as data so the hook is nameable from tests
/// without touching the router in this packet.
pub fn hook_name() -> &'static str {
    "mcp.rs ToolRouter::with_sync_tool::<PlanGoalsTool>() + tool_names 19->20"
}

/// Closed planning language: every brief maps to these five goals, no more.
/// The model picks the encounter flavor (goal kind plus rung, carried by the
/// draft); it can never invent a goal outside this enum.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Goal {
    /// Move rows validate against the shipped table fences.
    TablesValid,
    /// Slot-0 win rate sits inside the PQ2 P3 band.
    WinRateBand,
    /// No single move takes more than the kill-share cap.
    KillShareCap,
    /// No fighter holds still for the stuck window.
    NoStuck,
    /// No kill lands faster than the fork floor.
    NoFastKill,
}

impl Goal {
    /// Stable lowercase tag for receipts and refusals.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TablesValid => "tables_valid",
            Self::WinRateBand => "win_rate_band",
            Self::KillShareCap => "kill_share_cap",
            Self::NoStuck => "no_stuck",
            Self::NoFastKill => "no_fast_kill",
        }
    }

    /// Every goal in judge order: tables first, then the fork asserts in the
    /// same win-rate, kill-share, stuck, fast-kill order `check_fork` uses.
    pub const fn all() -> [Self; 5] {
        [
            Self::TablesValid,
            Self::WinRateBand,
            Self::KillShareCap,
            Self::NoStuck,
            Self::NoFastKill,
        ]
    }

    /// Judge one goal over a draft plus its window sample. `None` holds;
    /// `Some` is the conflict line quoting the violated bound.
    pub fn check(self, draft: &DraftEncounter, sample: &ForkSample) -> Option<String> {
        match self {
            Self::TablesValid => validate_tables(TABLES_VERSION, &draft.rows)
                .err()
                .map(|error| format!("table {error}")),
            Self::WinRateBand => {
                if sample.win_rate_permille < WIN_MIN_PERMILLE
                    || sample.win_rate_permille > WIN_MAX_PERMILLE
                {
                    Some(format!(
                        "fork win_rate {} permille outside [{WIN_MIN_PERMILLE}, {WIN_MAX_PERMILLE}]",
                        sample.win_rate_permille,
                    ))
                } else {
                    None
                }
            }
            Self::KillShareCap => {
                if sample.top_kill_share_permille > KILL_SHARE_MAX_PERMILLE {
                    Some(format!(
                        "fork kill_share {} permille over the {KILL_SHARE_MAX_PERMILLE} cap",
                        sample.top_kill_share_permille,
                    ))
                } else {
                    None
                }
            }
            Self::NoStuck => {
                if sample.stuck_ticks >= STUCK_WINDOW_TICKS {
                    Some(format!(
                        "fork stuck_ticks {} of the window still (bound {STUCK_WINDOW_TICKS})",
                        sample.stuck_ticks,
                    ))
                } else {
                    None
                }
            }
            Self::NoFastKill => match sample.ticks_to_kill {
                Some(kill) if kill < MIN_TICKS_TO_KILL => Some(format!(
                    "fork ticks_to_kill killed in {kill} ticks, faster than the \
                     {MIN_TICKS_TO_KILL}-tick floor",
                )),
                _ => None,
            },
        }
    }
}

/// Failing goals over a draft plus its sample, in [`Goal::all`] order.
fn failing_goals(draft: &DraftEncounter, sample: &ForkSample) -> Vec<(Goal, String)> {
    let mut out = Vec::new();
    for goal in Goal::all() {
        if let Some(conflict) = goal.check(draft, sample) {
            out.push((goal, conflict));
        }
    }
    out
}

/// Translated brief: the closed goal flavor the model may propose. The full
/// goal set is always [`Goal::all`]; this carries the encounter kind plus the
/// requested rung the search starts from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TranslatedBrief {
    /// Translated goal kind (duel, rivalry or cauldron).
    pub goal: GoalKind,
    /// Requested difficulty rung `0..=7`.
    pub rung: u8,
    /// True when the brief asked for a frozen tableau.
    pub stuck_risk: bool,
}

/// Translate a brief into its closed goal flavor. Bounds-checked through the
/// AB3 brief type, so overlong or empty briefs refuse with the legal values;
/// unknown prose falls back to a duel at rung 3, never to a new goal.
pub fn translate(
    text: &str,
    seed: u64,
) -> Result<TranslatedBrief, crate::build_encounter::BriefError> {
    let brief = EncounterBrief::new(text, seed)?;
    let draft = expand(&brief);
    Ok(TranslatedBrief {
        goal: draft.goal,
        rung: draft.rung,
        stuck_risk: draft.stuck_risk,
    })
}

/// Planner operators in fixed expansion order. These are the AB3 typed
/// repairs reused as STRIPS-style operators: each fits every draft, so search
/// never stalls on an inapplicable fix.
const OPERATORS: [Repair; 5] = [
    Repair::ClampDamage,
    Repair::FixParryWindow,
    Repair::SoftenBlows,
    Repair::Unstick,
    Repair::DropRung,
];

/// Apply one operator to a draft. Same effects as the AB3 repairs over the
/// public draft fields (balanced damage 120, parry one tick below startup);
/// pure and total.
fn apply_operator(draft: &mut DraftEncounter, operator: Repair) {
    match operator {
        Repair::ClampDamage => {
            draft.rows.attack_damage = 120;
        }
        Repair::FixParryWindow => {
            draft.rows.parry_window_ticks = draft.rows.attack_startup_ticks.saturating_sub(1);
        }
        Repair::SoftenBlows => {
            draft.rows.attack_damage = draft.rows.attack_damage.min(120);
            draft.rows.attack_recovery_ticks = draft
                .rows
                .attack_recovery_ticks
                .saturating_add(12)
                .min(crate::build_encounter::RECOVERY_MAX_TICKS);
        }
        Repair::Unstick => {
            draft.stuck_risk = false;
        }
        Repair::DropRung => {
            draft.rung = draft.rung.saturating_sub(2);
        }
    }
}

/// Search signature: every mutable draft lane, in field order. Stable and
/// ordered, so the visited set is deterministic on every machine.
type Signature = (u16, u32, u32, bool, u8);

/// Signature of one draft: damage, recovery, parry window, stuck risk, rung.
fn signature(draft: &DraftEncounter) -> Signature {
    (
        draft.rows.attack_damage,
        draft.rows.attack_recovery_ticks,
        draft.rows.parry_window_ticks,
        draft.stuck_risk,
        draft.rung,
    )
}

/// Deterministic plan digest: FNV-1a over brief text, seed, goal tag, morale,
/// every row field and the rung. Same inputs, same digest on every machine.
/// Domain-separated from the AB3 plan ids by the `AB5` tag word.
fn plan_digest(brief: &EncounterBrief, draft: &DraftEncounter) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(b"AB5-plan-goals");
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

/// Deterministic fork id: FNV-1a over the plan digest plus the window length,
/// domain-separated from AB3 fork ids by construction.
fn fork_id(plan: u64, ticks: u32) -> u64 {
    let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ 0x060a_1ab5_0000_0005);
    hasher.write(&plan.to_le_bytes());
    hasher.write(&ticks.to_le_bytes());
    hasher.finish()
}

/// Refusal helper: the conflict set plus a reason quoting the violated
/// bounds, so authors can fix the brief.
fn refuse(conflicts: Vec<String>, repairs_used: Vec<Repair>, reason: String) -> BuildRefusal {
    BuildRefusal {
        conflicts,
        reason,
        repairs_used,
    }
}

/// Classical planner: breadth-first search over operator sequences, shortest
/// plan first, fixed operator order, no repeated operator on one path. The
/// first state passing every [`Goal`] (tables plus the fork asserts) lands as
/// the AB3 encounter shape; an exhausted budget refuses with the conflict set
/// of the closest state (fewest failing goals, then shortest path). At most
/// `budget` operators on the returned plan, clamped to [`MAX_ATTEMPTS`].
pub fn plan_with_budget<O: ForkOracle>(
    text: &str,
    seed: u64,
    budget: usize,
    oracle: &O,
) -> Result<BuiltEncounter, BuildRefusal> {
    let brief = EncounterBrief::new(text, seed).map_err(|error| {
        refuse(
            vec![format!("brief {error}")],
            Vec::new(),
            format!("brief refused before any attempt: {error}"),
        )
    })?;
    let budget = budget.min(MAX_ATTEMPTS);
    let start = expand(&brief);

    let landed = |draft: &DraftEncounter, sample: &ForkSample, path: &[Repair]| BuiltEncounter {
        plan_id: plan_digest(&brief, draft),
        goal: draft.goal,
        rows: draft.rows,
        rung: draft.rung,
        fork_id: fork_id(plan_digest(&brief, draft), FORK_WINDOW_TICKS),
        sample: *sample,
        repairs_used: path.to_vec(),
    };

    let start_sample = oracle.sample(&start);
    if failing_goals(&start, &start_sample).is_empty() {
        return Ok(landed(&start, &start_sample, &[]));
    }
    let start_conflicts: Vec<String> = failing_goals(&start, &start_sample)
        .into_iter()
        .map(|(_, conflict)| conflict)
        .collect();
    // Closest state so far: fewest failing goals, then the shortest path.
    let mut best: (usize, Vec<String>, Vec<Repair>) =
        (start_conflicts.len(), start_conflicts, Vec::new());

    let mut visited: BTreeSet<Signature> = BTreeSet::from([signature(&start)]);
    let mut queue: VecDeque<(DraftEncounter, Vec<Repair>)> = VecDeque::from([(start, Vec::new())]);
    while let Some((draft, path)) = queue.pop_front() {
        if path.len() >= budget {
            continue;
        }
        for operator in OPERATORS {
            if path.contains(&operator) {
                continue;
            }
            let mut next = draft;
            apply_operator(&mut next, operator);
            if !visited.insert(signature(&next)) {
                continue;
            }
            let mut next_path = path.clone();
            next_path.push(operator);
            let sample = oracle.sample(&next);
            let failing = failing_goals(&next, &sample);
            if failing.is_empty() {
                return Ok(landed(&next, &sample, &next_path));
            }
            if failing.len() < best.0 {
                best = (
                    failing.len(),
                    failing.into_iter().map(|(_, conflict)| conflict).collect(),
                    next_path.clone(),
                );
            }
            queue.push_back((next, next_path));
        }
    }
    let (fails, conflicts, path) = best;
    let first = conflicts.first().cloned().unwrap_or_default();
    Err(refuse(
        conflicts,
        path,
        format!(
            "no plan within {budget} attempts ({fails} goal(s) still failing, first: {first}; \
             win-rate band [{WIN_MIN_PERMILLE}, {WIN_MAX_PERMILLE}], \
             kill-share cap {KILL_SHARE_MAX_PERMILLE})",
        ),
    ))
}

/// Envelope entry: plan one encounter under the envelope budget. The stage
/// must be `encounter`; any other stage refuses with the legal stage named.
/// The search spends at most the envelope repair budget (never more than
/// [`MAX_ATTEMPTS` operators), then refuses with the conflict set.
pub fn plan_encounter<O: ForkOracle>(
    env: &BriefEnvelope,
    oracle: &O,
) -> Result<BuiltEncounter, BuildRefusal> {
    if env.stage() != BriefStage::Encounter {
        return Err(refuse(
            vec![format!(
                "stage `{}` is not an encounter brief",
                env.stage().as_str()
            )],
            Vec::new(),
            format!(
                "stage `{}` refused: submit stage `encounter` (legal: {})",
                env.stage().as_str(),
                BriefStage::all_tags().join(", "),
            ),
        ));
    }
    let budget = usize::from(env.repair_budget());
    plan_with_budget(env.text(), env.seed(), budget, oracle)
}

/// Staged plan: an AB1 level plus its AB5 planned encounter under one seed.
/// Available with the `mcp` feature, where the AB1 level chain lives.
#[cfg(feature = "mcp")]
pub mod staged {
    use super::{plan_with_budget, MAX_ATTEMPTS};
    use crate::build_encounter::{BuildRefusal, BuiltEncounter, ForkOracle};

    /// One staged plan: level digests plus the fork-checked encounter.
    /// Deterministic: same briefs plus seed replay identically.
    #[derive(Clone, PartialEq, Eq, Debug)]
    pub struct StagedPlan {
        /// FNV over the AB1 placement table.
        pub level_digest: u64,
        /// FNV over the AB1 cell bytes.
        pub cell_digest: u64,
        /// Props the level placed.
        pub props_placed: u32,
        /// AB1 repair rounds consumed.
        pub level_repairs: u8,
        /// The planned encounter.
        pub encounter: BuiltEncounter,
        /// Seed both stages expanded under.
        pub seed: u64,
    }

    /// Plan an encounter staged on an AB1 level: the level builds first
    /// (navigable routes required), then the classical planner solves the
    /// encounter under the same seed. Either side refuses with its conflict
    /// set; refusals stage nothing.
    pub fn plan_staged_encounter<O: ForkOracle>(
        level_brief: &str,
        encounter_text: &str,
        seed: u64,
        oracle: &O,
    ) -> Result<StagedPlan, BuildRefusal> {
        let level_env =
            crate::build_level::envelope(level_brief, seed).map_err(|error| BuildRefusal {
                conflicts: vec![format!("level {error}")],
                reason: format!("level brief refused before any attempt: {error}"),
                repairs_used: Vec::new(),
            })?;
        let level = match crate::build_level::build(&level_env).map_err(|error| BuildRefusal {
            conflicts: vec![format!("level {error}")],
            reason: format!("level brief refused before any attempt: {error}"),
            repairs_used: Vec::new(),
        })? {
            crate::build_level::BuildOutcome::Placed(level) => level,
            crate::build_level::BuildOutcome::Refused(refusal) => {
                let mut conflicts = refusal.conflicts;
                conflicts.extend(refusal.findings);
                return Err(BuildRefusal {
                    conflicts,
                    reason: format!("level refused, encounter never planned: {}", refusal.reason),
                    repairs_used: Vec::new(),
                });
            }
        };
        let encounter = plan_with_budget(encounter_text, seed, MAX_ATTEMPTS, oracle)?;
        Ok(StagedPlan {
            level_digest: level.report.level_digest,
            cell_digest: level.report.cell_digest,
            props_placed: level.report.props_placed,
            level_repairs: level.report.repairs_used,
            encounter,
            seed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_encounter::{check_fork, DeterministicOracle, MAX_REPAIRS};

    fn oracle() -> DeterministicOracle {
        DeterministicOracle
    }

    const TEN_BRIEFS: [&str; 10] = [
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

    #[test]
    fn ten_briefs_plan_inside_win_rate_bounds() {
        assert_eq!(TEN_BRIEFS.len(), 10);
        for (index, text) in TEN_BRIEFS.iter().enumerate() {
            let seed = 2040 + index as u64;
            let planned =
                plan_with_budget(text, seed, MAX_ATTEMPTS, &oracle()).expect("brief plans");
            assert!(
                (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&planned.sample.win_rate_permille),
                "brief {index} rate {} outside the band",
                planned.sample.win_rate_permille,
            );
            assert!(
                planned.sample.top_kill_share_permille <= KILL_SHARE_MAX_PERMILLE,
                "brief {index} share {} over the cap",
                planned.sample.top_kill_share_permille,
            );
            assert_eq!(planned.sample.stuck_ticks, 0, "brief {index} stuck");
            assert!(
                planned.repairs_used.len() <= MAX_ATTEMPTS,
                "brief {index} attempts bounded"
            );
            assert!(
                validate_tables(TABLES_VERSION, &planned.rows).is_ok(),
                "brief {index} rows validate"
            );
            let again =
                plan_with_budget(text, seed, MAX_ATTEMPTS, &oracle()).expect("replay plans");
            assert_eq!(planned, again, "brief {index} replays exactly");
        }
    }

    #[test]
    fn seeded_bad_plan_refused_by_fork_check() {
        let brief = EncounterBrief::new("an overpowered duel with oneshot blades", 7)
            .expect("brief bounds");
        let draft = expand(&brief);
        assert_eq!(draft.rows.attack_damage, 900);
        let sample = oracle().sample(&draft);
        let breach = check_fork(7, &sample).expect_err("one-shot damage breaches");
        assert_eq!(breach.metric, "win_rate");
        // Zero attempt budget: the planner refuses with the conflict set.
        let refused = plan_with_budget("an overpowered duel", 7, 0, &oracle())
            .expect_err("no budget means refuse");
        assert_eq!(refused.code(), "encounter_refused");
        assert!(refused.repairs_used.is_empty());
        assert!(!refused.conflicts.is_empty());
        assert!(
            refused
                .conflicts
                .iter()
                .any(|line| line.contains("win_rate")),
            "conflicts name the breach: {:?}",
            refused.conflicts,
        );
        assert!(
            refused.reason.contains("win-rate band"),
            "reason quotes the band: {}",
            refused.reason,
        );
    }

    #[test]
    fn adversarial_brief_converges_within_five_attempts() {
        let planned = plan_with_budget(
            "an overpowered broken frozen duel of statues",
            11,
            MAX_ATTEMPTS,
            &oracle(),
        )
        .expect("search converges");
        assert!(!planned.repairs_used.is_empty(), "repairs were needed");
        assert!(planned.repairs_used.len() <= MAX_ATTEMPTS);
        let tags: Vec<&str> = planned.repairs_used.iter().map(|r| r.as_str()).collect();
        assert!(tags.contains(&"clamp_damage"), "damage fixed: {tags:?}");
        assert!(tags.contains(&"fix_parry_window"), "parry fixed: {tags:?}");
        assert!(tags.contains(&"unstick"), "tableau plays: {tags:?}");
        assert!(
            (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&planned.sample.win_rate_permille),
            "repaired rate {} lands",
            planned.sample.win_rate_permille,
        );
        assert_eq!(planned.sample.stuck_ticks, 0);
        assert!(validate_tables(TABLES_VERSION, &planned.rows).is_ok());
        // No operator repeats on one plan: each fires at most once.
        let mut seen = Vec::new();
        for repair in &planned.repairs_used {
            assert!(!seen.contains(repair), "operator fires once: {tags:?}");
            seen.push(*repair);
        }
    }

    #[test]
    fn broken_parry_needs_one_typed_fix() {
        let planned =
            plan_with_budget("a broken duel at dawn", 3, MAX_ATTEMPTS, &oracle()).expect("plans");
        assert_eq!(
            planned.repairs_used,
            vec![Repair::FixParryWindow],
            "shortest plan first, fixed order"
        );
    }

    #[test]
    fn translator_stays_closed_on_unknown_prose() {
        let flavor = translate("quantum banquet of purple jurisprudence", 5).expect("translates");
        assert_eq!(flavor.goal, GoalKind::Duel);
        assert_eq!(flavor.rung, 3);
        assert!(!flavor.stuck_risk);
        let planned = plan_with_budget(
            "quantum banquet of purple jurisprudence",
            5,
            MAX_ATTEMPTS,
            &oracle(),
        )
        .expect("unknown prose still plans");
        assert_eq!(planned.goal, GoalKind::Duel);
        assert!(planned.repairs_used.is_empty(), "clean draft needs no fix");
    }

    #[test]
    fn bad_briefs_refuse_before_any_attempt() {
        let empty = plan_with_budget("", 1, MAX_ATTEMPTS, &oracle()).expect_err("empty refuses");
        assert_eq!(empty.code(), "encounter_refused");
        assert!(empty.repairs_used.is_empty());
        assert!(
            empty
                .conflicts
                .iter()
                .any(|line| line.contains("empty_brief")),
            "conflicts: {:?}",
            empty.conflicts,
        );
        let long = "x".repeat(1025);
        let over =
            plan_with_budget(&long, 1, MAX_ATTEMPTS, &oracle()).expect_err("overlong refuses");
        assert!(
            over.conflicts
                .iter()
                .any(|line| line.contains("brief_too_long")),
            "conflicts: {:?}",
            over.conflicts,
        );
        assert!(
            translate("", 1).is_err(),
            "translator refuses empty briefs too"
        );
    }

    #[test]
    fn envelope_entry_enforces_stage_and_budget() {
        let level =
            BriefEnvelope::new("prop crate probe_a", 1, BriefStage::Level).expect("envelope");
        let refused = plan_encounter(&level, &oracle()).expect_err("level stage refuses");
        assert_eq!(refused.code(), "encounter_refused");
        assert!(
            refused.reason.contains("`encounter`"),
            "reason names the legal stage: {}",
            refused.reason,
        );
        let strict = BriefEnvelope::with_budget("an overpowered duel", 9, BriefStage::Encounter, 0)
            .expect("zero budget is legal");
        let refused =
            plan_encounter(&strict, &oracle()).expect_err("zero budget refuses bad plans");
        assert!(refused.repairs_used.is_empty());
        assert!(
            refused
                .conflicts
                .iter()
                .any(|line| line.contains("win_rate")),
            "conflicts: {:?}",
            refused.conflicts,
        );
        let open =
            BriefEnvelope::new(TEN_BRIEFS[0], 2040, BriefStage::Encounter).expect("envelope");
        assert_eq!(open.repair_budget(), 5);
        let planned = plan_encounter(&open, &oracle()).expect("clean brief plans");
        assert!((WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&planned.sample.win_rate_permille));
    }

    #[test]
    fn attempts_match_ab3_budget() {
        assert_eq!(MAX_ATTEMPTS, MAX_REPAIRS, "one budget for both stages");
        assert_eq!(MAX_ATTEMPTS, 5);
    }

    #[test]
    fn goal_language_is_closed_and_ordered() {
        let tags: Vec<&str> = Goal::all().iter().map(|goal| goal.as_str()).collect();
        assert_eq!(
            tags,
            vec![
                "tables_valid",
                "win_rate_band",
                "kill_share_cap",
                "no_stuck",
                "no_fast_kill"
            ]
        );
        let clean = expand(&EncounterBrief::new("a quiet duel at dawn", 2).expect("brief bounds"));
        let sample = oracle().sample(&clean);
        assert!(
            failing_goals(&clean, &sample).is_empty(),
            "clean draft holds every goal"
        );
        let hot = expand(
            &EncounterBrief::new("an overpowered duel with oneshot blades", 2)
                .expect("brief bounds"),
        );
        let sample = oracle().sample(&hot);
        let failing = failing_goals(&hot, &sample);
        assert!(
            failing.iter().any(|(goal, _)| *goal == Goal::WinRateBand),
            "one-shot damage fails the band"
        );
        // Judge order follows the goal language order.
        let order: Vec<Goal> = failing.into_iter().map(|(goal, _)| goal).collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted, "conflicts ride in goal order");
    }

    #[test]
    fn plan_digests_are_deterministic_and_seed_sensitive() {
        let first = plan_with_budget(TEN_BRIEFS[1], 31, MAX_ATTEMPTS, &oracle()).expect("plans");
        let again = plan_with_budget(TEN_BRIEFS[1], 31, MAX_ATTEMPTS, &oracle()).expect("replays");
        assert_eq!(first.plan_id, again.plan_id);
        assert_eq!(first.fork_id, again.fork_id);
        let other = plan_with_budget(TEN_BRIEFS[1], 32, MAX_ATTEMPTS, &oracle()).expect("plans");
        assert_ne!(first.plan_id, other.plan_id, "the seed feeds the digest");
    }

    #[test]
    fn landed_plan_passes_ab3_gates_directly() {
        // The planner returns AB3 shapes, so the AB3 judges accept them as-is.
        let planned = plan_with_budget(
            "an overpowered broken frozen duel",
            13,
            MAX_ATTEMPTS,
            &oracle(),
        )
        .expect("plans");
        assert!(validate_tables(TABLES_VERSION, &planned.rows).is_ok());
        assert!(check_fork(planned.fork_id, &planned.sample).is_ok());
    }

    #[test]
    fn mcp_hook_names_the_tool() {
        assert_eq!(MCP_TOOL_NAME, "sim.plan_goals");
        assert!(!MCP_TOOL_SUMMARY.is_empty());
        assert!(MCP_TOOL_SUMMARY.contains("fork"));
        assert!(hook_name().contains("PlanGoalsTool"));
        assert!(hook_name().contains("tool_names"));
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn staged_level_plus_encounter_replays() {
        use super::staged::plan_staged_encounter;
        let level_brief = "prop crate crate_01\nwant crate_01 next_to wall:north [0,60]";
        let first =
            plan_staged_encounter(level_brief, TEN_BRIEFS[0], 77, &oracle()).expect("staged plans");
        assert!(first.props_placed >= 1, "the level places");
        assert!(
            (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE)
                .contains(&first.encounter.sample.win_rate_permille),
            "the encounter lands in the band"
        );
        assert_eq!(first.seed, 77);
        let again = plan_staged_encounter(level_brief, TEN_BRIEFS[0], 77, &oracle())
            .expect("staged replays");
        assert_eq!(first, again, "same briefs plus seed replay exactly");
        let doomed = "prop crate doomed\nwant doomed next_to wall:north [0,10]\n\
             want doomed next_to wall:south [0,10]\nwant doomed next_to wall:east [0,10]";
        let refused = plan_staged_encounter(doomed, TEN_BRIEFS[0], 78, &oracle())
            .expect_err("doomed levels refuse");
        assert_eq!(refused.code(), "encounter_refused");
        assert!(!refused.conflicts.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Door tool: sim.plan_goals over the classical planner.
// ---------------------------------------------------------------------------
//
// The door binds the pure `DeterministicOracle`: the planner is classical
// search in plain Rust (no GPU, no provider, no wall clock), so the
// deterministic oracle is the live path, not a stand-in. It samples the same
// window the fork asserts judge, and every answer replays byte-identically.
// DTO structs and fields carry no doc comments on purpose: every doc line
// inflates the discovery schemas, and the twenty-one-tool list is a gate.

/// Plan request: brief text plus the seed it expands under.
/// Units are brief chars in, plan ids out. Errors are typed brief refusals
/// naming the legal values. Budget is a bounded attempt count per call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlanGoalsArgs {
    pub brief: String,
    pub seed: u64,
}

/// Plan answer: solved flag, plan ids, goal, rung and fork sample.
/// Units are ids, permille rates and chars of reason text. No errors once
/// planned; unsolved answers zero the ids. Budget is one plan per call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlanGoalsOutput {
    pub solved: bool,
    pub plan_id: u64,
    pub goal: String,
    pub rung: u8,
    pub fork_id: u64,
    pub win_rate_permille: u16,
    pub top_kill_share_permille: u16,
    pub repairs: Vec<String>,
    pub conflicts: Vec<String>,
    pub reason: String,
}

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Structured refusal: the conflict set plus the repairs spent, so authors
/// can fix the brief. Zeroed ids mark an unsolved answer.
fn refusal_output(refusal: &crate::build_encounter::BuildRefusal) -> PlanGoalsOutput {
    PlanGoalsOutput {
        solved: false,
        plan_id: 0,
        goal: String::new(),
        rung: 0,
        fork_id: 0,
        win_rate_permille: 0,
        top_kill_share_permille: 0,
        repairs: refusal
            .repairs_used
            .iter()
            .map(|repair| repair.as_str().to_owned())
            .collect(),
        conflicts: refusal.conflicts.clone(),
        reason: refusal.reason.clone(),
    }
}

impl super::mcp::ServerState {
    /// Answer one `sim.plan_goals` call with the classical planner. Read-only:
    /// the document, journal, budget and ledger never move.
    pub fn plan_goals(
        &self,
        args: &PlanGoalsArgs,
    ) -> Result<PlanGoalsOutput, super::mcp::McpToolError> {
        if let Err(error) = crate::build_encounter::EncounterBrief::new(&args.brief, args.seed) {
            let code = error.clone().code();
            return Err(super::mcp::McpToolError::new(code, error.to_string()));
        }
        let oracle = crate::build_encounter::DeterministicOracle;
        match plan_with_budget(&args.brief, args.seed, MAX_ATTEMPTS, &oracle) {
            Ok(built) => Ok(PlanGoalsOutput {
                solved: true,
                plan_id: built.plan_id,
                goal: built.goal.as_str().to_owned(),
                rung: built.rung,
                fork_id: built.fork_id,
                win_rate_permille: built.sample.win_rate_permille,
                top_kill_share_permille: built.sample.top_kill_share_permille,
                repairs: built
                    .repairs_used
                    .iter()
                    .map(|repair| repair.as_str().to_owned())
                    .collect(),
                conflicts: Vec::new(),
                reason: String::new(),
            }),
            Err(refusal) => Ok(refusal_output(&refusal)),
        }
    }
}

/// Plan tool: classical planning over a brief, read-only over the document.
pub struct PlanGoalsTool;

impl ToolBase for PlanGoalsTool {
    type Parameter = PlanGoalsArgs;
    type Output = PlanGoalsOutput;
    type Error = super::mcp::McpToolError;

    fn name() -> Cow<'static, str> {
        MCP_TOOL_NAME.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(MCP_TOOL_SUMMARY.into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<super::mcp::McpServer> for PlanGoalsTool {
    fn invoke(
        service: &super::mcp::McpServer,
        param: Self::Parameter,
    ) -> Result<Self::Output, Self::Error> {
        service.dispatch(MCP_TOOL_NAME, |state| state.plan_goals(&param))
    }
}

#[cfg(test)]
mod door_tests {
    use super::{PlanGoalsArgs, PlanGoalsOutput, PlanGoalsTool, MAX_ATTEMPTS};
    use crate::build_encounter::{
        BuildRefusal, DeterministicOracle, BRIEF_MAX_CHARS, KILL_SHARE_MAX_PERMILLE,
        WIN_MAX_PERMILLE, WIN_MIN_PERMILLE,
    };
    use rmcp::handler::server::router::tool::{SyncTool, ToolBase};

    fn server() -> super::super::mcp::McpServer {
        super::super::mcp::McpServer::new(7, engine_editor::capability::Capability::unrestricted(7))
    }

    fn args(text: &str, seed: u64) -> PlanGoalsArgs {
        PlanGoalsArgs {
            brief: text.to_owned(),
            seed,
        }
    }

    fn revision(server: &super::super::mcp::McpServer) -> u64 {
        super::super::mcp::SceneReadTool::invoke(
            server,
            super::super::mcp::SceneReadArgs::default(),
        )
        .expect("scene reads")
        .revision
    }

    #[test]
    fn door_shape_is_one_line_and_readonly() {
        assert_eq!(PlanGoalsTool::name().as_ref(), "sim.plan_goals");
        let description = PlanGoalsTool::description().expect("tool carries a description");
        assert!(!description.is_empty(), "description names the tool");
        assert!(
            !description.contains('\n'),
            "one-line description for the token budget: {description}"
        );
        let annotations = PlanGoalsTool::annotations().expect("annotations ride along");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.open_world_hint, Some(false));
        assert!(
            PlanGoalsTool::input_schema().is_some(),
            "door advertises its input schema"
        );
        assert!(
            PlanGoalsTool::output_schema().is_some(),
            "door advertises its output schema"
        );
        assert!(super::hook_name().contains("PlanGoalsTool"));
    }

    #[test]
    fn clean_brief_solves_replays_and_writes_nothing() {
        let server = server();
        let before = revision(&server);
        let first = PlanGoalsTool::invoke(
            &server,
            args("a quiet duel at dawn in the training yard", 2040),
        )
        .expect("clean brief plans");
        assert!(first.solved);
        assert!(["duel", "rivalry", "cauldron"].contains(&first.goal.as_str()));
        assert!(
            (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&first.win_rate_permille),
            "rate {} lands in the band",
            first.win_rate_permille,
        );
        assert!(
            first.top_kill_share_permille <= KILL_SHARE_MAX_PERMILLE,
            "share {} under the cap",
            first.top_kill_share_permille,
        );
        assert!(first.repairs.len() <= MAX_ATTEMPTS);
        assert!(first.conflicts.is_empty());
        assert!(first.reason.is_empty());
        let again = PlanGoalsTool::invoke(
            &server,
            args("a quiet duel at dawn in the training yard", 2040),
        )
        .expect("replay plans");
        assert_eq!(first, again, "same brief plus seed replays exactly");
        assert_eq!(revision(&server), before, "the door writes nothing");
    }

    #[test]
    fn adversarial_brief_converges_with_typed_repairs() {
        let planned = PlanGoalsTool::invoke(
            &server(),
            args("an overpowered broken frozen duel of statues", 11),
        )
        .expect("search converges");
        assert!(planned.solved);
        assert!(!planned.repairs.is_empty(), "repairs were needed");
        assert!(planned.repairs.len() <= MAX_ATTEMPTS);
        assert!(planned.repairs.contains(&"clamp_damage".to_owned()));
        assert!(planned.repairs.contains(&"unstick".to_owned()));
        assert!(
            (WIN_MIN_PERMILLE..=WIN_MAX_PERMILLE).contains(&planned.win_rate_permille),
            "repaired rate {} lands",
            planned.win_rate_permille,
        );
    }

    #[test]
    fn bad_briefs_refuse_with_the_legal_values() {
        let server = server();
        let empty = PlanGoalsTool::invoke(&server, args("", 1)).expect_err("empty refuses");
        assert_eq!(empty.code, "empty_brief");
        assert!(
            empty.message.contains(&BRIEF_MAX_CHARS.to_string()),
            "refusal quotes the bound: {}",
            empty.message,
        );
        let long = "x".repeat(BRIEF_MAX_CHARS + 1);
        let over = PlanGoalsTool::invoke(&server, args(&long, 1)).expect_err("overlong refuses");
        assert_eq!(over.code, "brief_too_long");
        assert!(
            over.message.contains(&BRIEF_MAX_CHARS.to_string()),
            "refusal quotes the bound: {}",
            over.message,
        );
        assert_eq!(revision(&server), 0, "refusals write nothing");
    }

    #[test]
    fn planner_refusal_maps_to_structured_unsolved_output() {
        // Zero attempt budget refuses a breaching brief: the refusal arm maps
        // to `solved: false` with the conflict set, never to a transport error.
        let refused = super::plan_with_budget("an overpowered duel", 7, 0, &DeterministicOracle)
            .expect_err("no budget means refuse");
        assert_eq!(refused.code(), "encounter_refused");
        let out: PlanGoalsOutput = super::refusal_output(&refused);
        assert!(!out.solved);
        assert_eq!(out.plan_id, 0);
        assert!(out.goal.is_empty());
        assert!(!out.conflicts.is_empty());
        assert!(!out.reason.is_empty());
        // The synthetic shape round-trips the same fields the door serves.
        let direct = super::refusal_output(&BuildRefusal {
            conflicts: vec!["fork win_rate 850 permille outside [400, 600]".to_owned()],
            reason: "no plan within 0 attempts".to_owned(),
            repairs_used: vec![crate::build_encounter::Repair::ClampDamage],
        });
        assert!(!direct.solved);
        assert_eq!(direct.repairs, vec!["clamp_damage".to_owned()]);
        assert_eq!(direct.conflicts.len(), 1);
    }
}
