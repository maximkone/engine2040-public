//! plan_trace: one call that plans and shows the trace.
//!
//! Owner: this crate. The GOAP planner ([`crate::goap`]) and the shared
//! state ([`crate::blackboard`]) were two islands with no caller outside
//! this crate. This module is the one seam that puts them behind a single
//! testable API: [`plan_and_trace`] returns the plan, the goal it chased,
//! the reason that goal won, what every other goal cost and the blackboard
//! the plan wrote. [`plan_for_percept`] does the same for a gladiator
//! [`Percept`](crate::gladiator::Percept), so a brain can be asked what it
//! would do and why.
//!
//! Units: facts are bits of a `u64`, costs are integer ticks, separations
//! are millimetres. Determinism: integer math only; goals are tried in id
//! order and a tie goes to the lowest goal id, so the same facts give the
//! same trace. Errors: none; an unreachable goal is a value
//! ([`PlanReason::Unreachable`]). Budget: one A* per goal over the baked
//! action table; a plan runs once per round, never per tick.
//!
//! The arena consumer (one tree per fighter, one plan per round) is a
//! patch-ready spec in
//! `docs/research/powerups-2026-10-20/proposed-rows-14-smart-ai-native-engine.md`
//! (row SMX-01): `crates/arena` is held by another lane.

use crate::blackboard::Blackboard;
use crate::goap::{self, Action, GoalState, PlanError};
use crate::melee::StrikePhase;
use crate::{Brain, Percept};

/// The foe is inside strike range.
const F_IN_RANGE: u64 = 1 << 0;
/// The foe is swinging.
const F_THREAT: u64 = 1 << 1;
/// The foe is beyond the far ring.
const F_FAR: u64 = 1 << 2;
/// Own hp is under the wounded line.
const F_WOUNDED: u64 = 1 << 3;
/// The fighter closed the distance (planner-side fact).
const F_CLOSED: u64 = 1 << 4;
/// The fighter holds guard (planner-side fact).
const F_GUARDED: u64 = 1 << 5;
/// The fighter landed a blow (planner-side fact).
const F_STRUCK: u64 = 1 << 6;
/// The fighter gave ground (planner-side fact).
const F_BACK: u64 = 1 << 7;

/// Cost of closing the distance: about 1.2 m at the arena march speed.
const COST_CLOSE: u32 = 70;
/// Cost of a strike: startup plus recovery.
const COST_STRIKE: u32 = 15;
/// Cost of holding guard.
const COST_GUARD: u32 = 6;
/// Cost of giving ground.
const COST_BACK: u32 = 12;

/// Goal preference added to every plan that reaches it, in ticks. A bias is
/// what makes the gladiator prefer the strike over the cheaper close.
const BIAS_STRIKE: u32 = 0;
/// Guard bias: guard is a fallback, so it only wins when nothing else can.
const BIAS_GUARD: u32 = 10;
/// Close bias: closing is a means, not an end.
const BIAS_CLOSE: u32 = 30;
/// Back bias: giving ground is as good as striking.
const BIAS_BACK: u32 = 0;

/// Strike range in whole millimetres, as a float, so the percept comparison
/// needs no numeric cast. [`Brain::ATTACK_MM`] is the integer source of
/// truth; `plan_trace_metre_thresholds_match_the_brain_thresholds` pins the
/// pair, so a moved threshold fails the test instead of drifting.
const ATTACK_MM: f32 = 2_500.0;

/// Far ring in whole millimetres, paired with [`Brain::FAR_MM`] the same way.
const FAR_MM: f32 = 8_000.0;

/// Metres to whole millimetres, as a float: `trunc` reproduces the `as i32`
/// truncation the crate uses at the integer edge, so the thresholds can stay
/// integral without a numeric cast.
fn mm_from_m(dist_m: f32) -> f32 {
    (dist_m * 1000.0).trunc()
}

/// What the planner knows about the fight before it plans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The foe is inside strike range.
    pub in_range: bool,
    /// The foe is swinging.
    pub threat: bool,
    /// The foe is beyond the far ring.
    pub far: bool,
    /// Own hp is under the wounded line.
    pub wounded: bool,
}

impl Facts {
    /// The bit mask the planner reads. The four planner-side facts (closed,
    /// guarded, struck, given ground) stay inside the planner.
    #[must_use]
    pub const fn mask(self) -> u64 {
        let mut bits = 0u64;
        if self.in_range {
            bits |= F_IN_RANGE;
        }
        if self.threat {
            bits |= F_THREAT;
        }
        if self.far {
            bits |= F_FAR;
        }
        if self.wounded {
            bits |= F_WOUNDED;
        }
        bits
    }
}

/// The goal a plan chased.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanGoalId {
    /// Land a blow.
    Strike,
    /// Hold guard.
    Guard,
    /// Close the distance.
    Close,
    /// Give ground.
    Back,
}

impl PlanGoalId {
    /// Stable numeric id, so a tape can pin the goal.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            PlanGoalId::Strike => 0,
            PlanGoalId::Guard => 1,
            PlanGoalId::Close => 2,
            PlanGoalId::Back => 3,
        }
    }

    /// The goal with that numeric id, or `None` when the id is unknown.
    #[must_use]
    pub const fn from_u16(id: u16) -> Option<Self> {
        match id {
            0 => Some(PlanGoalId::Strike),
            1 => Some(PlanGoalId::Guard),
            2 => Some(PlanGoalId::Close),
            3 => Some(PlanGoalId::Back),
            _ => None,
        }
    }

    /// Short name for a receipt or a log line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PlanGoalId::Strike => "strike",
            PlanGoalId::Guard => "guard",
            PlanGoalId::Close => "close",
            PlanGoalId::Back => "back",
        }
    }
}

/// One thing a plan does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanAction {
    /// Close the distance.
    Close,
    /// Land a blow.
    Strike,
    /// Hold guard.
    Guard,
    /// Give ground.
    Back,
}

impl PlanAction {
    /// Index into the action table.
    #[must_use]
    pub const fn as_index(self) -> usize {
        match self {
            PlanAction::Close => 0,
            PlanAction::Strike => 1,
            PlanAction::Guard => 2,
            PlanAction::Back => 3,
        }
    }

    /// The action with that index, or `None` when the index is unknown.
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(PlanAction::Close),
            1 => Some(PlanAction::Strike),
            2 => Some(PlanAction::Guard),
            3 => Some(PlanAction::Back),
            _ => None,
        }
    }
}

/// Why the trace chose what it chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanReason {
    /// The goal was already met in the start facts: no step was needed.
    AlreadyMet,
    /// The goal was the cheapest reachable one. `runner_up` names the next
    /// cheapest reachable goal, or `None` when this was the only one.
    Cheapest {
        /// The next cheapest reachable goal.
        runner_up: Option<PlanGoalId>,
    },
    /// No goal is reachable from the start facts inside the budget.
    Unreachable,
}

impl PlanReason {
    /// Stable numeric code, so the reason can ride in the blackboard.
    #[must_use]
    pub const fn code(self) -> u64 {
        match self {
            PlanReason::AlreadyMet => 1,
            PlanReason::Cheapest { .. } => 2,
            PlanReason::Unreachable => 3,
        }
    }
}

/// One planned step: the action and the facts before and after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanStep {
    /// What the step does.
    pub action: PlanAction,
    /// Facts before the step (bit mask).
    pub before: u64,
    /// Facts after the step (bit mask).
    pub after: u64,
}

/// What one goal cost, or why it could not be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanGoalOutcome {
    /// The goal.
    pub goal: PlanGoalId,
    /// Total cost of the cheapest plan to this goal, bias included, or
    /// `None` when the goal is unreachable.
    pub cost: Option<u32>,
    /// The typed refusal when the goal is unreachable.
    pub refusal: Option<PlanError>,
}

/// The trace: the plan, the goal, the reason and the blackboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTrace {
    /// The chosen goal, or `None` when nothing was reachable.
    pub goal: Option<PlanGoalId>,
    /// Why that goal.
    pub reason: PlanReason,
    /// The steps, in the order to take them.
    pub steps: Vec<PlanStep>,
    /// Total cost of the chosen plan, bias included. Units: ticks.
    pub cost: u32,
    /// Nodes the planner expanded for the chosen goal.
    pub expanded: u32,
    /// Every goal's outcome, in goal order.
    pub goals: Vec<PlanGoalOutcome>,
    /// The blackboard the plan wrote.
    pub blackboard: Blackboard,
}

/// The baked duel tables: close, strike, guard, give ground.
///
/// Costs are the arena's own numbers: closing walks about 1.2 m at the march
/// speed, a strike pays startup plus recovery, a guard pays the block
/// window, giving ground is one quick step. The biases make the gladiator
/// strike when it can, guard when the strike is out of reach and otherwise
/// close.
fn shipped_actions() -> Vec<Action> {
    vec![
        Action {
            pre_mask: F_FAR,
            pre_value: F_FAR,
            // Closing the distance puts the foe in range and clears "far".
            set_mask: F_CLOSED | F_IN_RANGE | F_FAR,
            set_value: F_CLOSED | F_IN_RANGE,
            cost: COST_CLOSE,
        },
        Action {
            pre_mask: F_IN_RANGE,
            pre_value: F_IN_RANGE,
            set_mask: F_STRUCK,
            set_value: F_STRUCK,
            cost: COST_STRIKE,
        },
        Action {
            pre_mask: F_THREAT,
            pre_value: F_THREAT,
            set_mask: F_GUARDED,
            set_value: F_GUARDED,
            cost: COST_GUARD,
        },
        Action {
            pre_mask: F_WOUNDED,
            pre_value: F_WOUNDED,
            set_mask: F_BACK,
            set_value: F_BACK,
            cost: COST_BACK,
        },
    ]
}

/// One goal: the fact it wants, and what it costs to want it.
fn goal(id: PlanGoalId, fact: u64, bias: u32) -> (PlanGoalId, GoalState, u32) {
    (
        id,
        GoalState {
            mask: fact,
            value: fact,
        },
        bias,
    )
}

/// The goal rows, in the order they are tried.
fn goals() -> [(PlanGoalId, GoalState, u32); 4] {
    [
        goal(PlanGoalId::Strike, F_STRUCK, BIAS_STRIKE),
        goal(PlanGoalId::Guard, F_GUARDED, BIAS_GUARD),
        goal(PlanGoalId::Close, F_CLOSED, BIAS_CLOSE),
        goal(PlanGoalId::Back, F_BACK, BIAS_BACK),
    ]
}

/// Facts from a gladiator percept.
///
/// The melee view wins when it is live, because it is already quantized to
/// whole centimetres at the edge, and that path compares integers. Otherwise
/// the percept separation is truncated to whole millimetres as a float and
/// compared against the same integral thresholds. A separation that is not
/// finite or not positive reads as unknown: then neither the in-range nor
/// the far fact is set, so the trace refuses to guess. Own hp comes from the
/// melee view, so a legacy percept (empty view) is never wounded.
#[must_use]
pub fn facts_from_percept(percept: &Percept) -> Facts {
    let mut facts = Facts::default();
    let melee = &percept.melee;
    if melee.is_live() {
        // Whole centimetres at the edge: integer in, integer compare.
        let mm = melee.target().dist_mm();
        facts.in_range = mm <= Brain::ATTACK_MM;
        facts.far = mm >= Brain::FAR_MM;
    } else if percept.dist_m.is_finite() && percept.dist_m > 0.0 {
        // Truncated to whole millimetres exactly like the integer read, so
        // both paths classify the same separation the same way.
        let mm = mm_from_m(percept.dist_m);
        facts.in_range = mm <= ATTACK_MM;
        facts.far = mm >= FAR_MM;
    }
    facts.threat = percept.foe_attacking
        || melee
            .foes()
            .iter()
            .any(|foe| foe.phase.is_threat() || foe.phase == StrikePhase::Active);
    facts.wounded = melee.own_hp_permille() < crate::melee::WOUNDED_PERMILLE;
    facts
}

/// The winning goal, its plan and its total cost.
struct Best {
    /// The goal that won.
    goal: PlanGoalId,
    /// The cheapest plan that reaches it.
    plan: goap::Plan,
    /// Plan cost plus the goal bias. Units: ticks.
    total: u32,
}

/// Plan every goal from the same start facts and keep the cheapest total
/// (plan cost plus the goal bias); a tie goes to the lower goal id. Returns
/// every goal's outcome in goal order, so the trace can say what the losers
/// cost or refused with.
fn plan_every_goal(
    actions: &[Action],
    start: u64,
    budget: u32,
) -> (Vec<PlanGoalOutcome>, Option<Best>) {
    let mut outcomes: Vec<PlanGoalOutcome> = Vec::with_capacity(goals().len());
    let mut best: Option<Best> = None;
    for (id, state, bias) in goals() {
        match goap::plan(actions, start, state, budget) {
            Ok(plan) => {
                let total = plan.cost.saturating_add(bias);
                outcomes.push(PlanGoalOutcome {
                    goal: id,
                    cost: Some(total),
                    refusal: None,
                });
                let better = match &best {
                    None => true,
                    Some(winner) => {
                        total < winner.total
                            || (total == winner.total && id.as_u16() < winner.goal.as_u16())
                    }
                };
                if better {
                    best = Some(Best {
                        goal: id,
                        plan,
                        total,
                    });
                }
            }
            Err(refusal) => outcomes.push(PlanGoalOutcome {
                goal: id,
                cost: None,
                refusal: Some(refusal),
            }),
        }
    }
    (outcomes, best)
}

/// The next cheapest reachable goal after the winner, or `None` when the
/// winner was the only reachable goal.
fn runner_up(outcomes: &[PlanGoalOutcome], winner: PlanGoalId) -> Option<PlanGoalId> {
    outcomes
        .iter()
        .filter(|outcome| outcome.cost.is_some() && outcome.goal != winner)
        .min_by_key(|outcome| (outcome.cost.unwrap_or(u32::MAX), outcome.goal.as_u16()))
        .map(|outcome| outcome.goal)
}

/// Walk the plan: one step per action, with the facts before and after it.
/// Returns the steps and the final facts.
fn trace_steps(actions: &[Action], start: u64, plan: &goap::Plan) -> (Vec<PlanStep>, u64) {
    let mut steps: Vec<PlanStep> = Vec::with_capacity(plan.steps.len());
    let mut mask = start;
    for index in &plan.steps {
        let action = actions[*index];
        let before = mask;
        mask = action.apply(mask);
        steps.push(PlanStep {
            action: PlanAction::from_index(*index).unwrap_or(PlanAction::Close),
            before,
            after: mask,
        });
    }
    (steps, mask)
}

/// The blackboard a trace writes: the goal, the reason, the costs, the step
/// count, the expanded node count, the start facts, the final facts and the
/// runner-up when there is one.
fn trace_blackboard(
    best: &Best,
    reason: PlanReason,
    start: u64,
    final_facts: u64,
    steps: usize,
    next: Option<PlanGoalId>,
) -> Blackboard {
    let mut blackboard = Blackboard::new();
    blackboard.set("goal", u64::from(best.goal.as_u16()));
    blackboard.set("reason", reason.code());
    blackboard.set("cost", u64::from(best.total));
    blackboard.set("plan_cost", u64::from(best.plan.cost));
    blackboard.set("steps", u64::try_from(steps).unwrap_or(u64::MAX));
    blackboard.set("expanded", u64::from(best.plan.expanded));
    blackboard.set("facts", start);
    blackboard.set("final_facts", final_facts);
    if let Some(next) = next {
        blackboard.set("runner_up", u64::from(next.as_u16()));
    }
    blackboard
}

/// The trace when no goal is reachable: the refusal stays a value and the
/// blackboard says so.
fn unreachable_trace(outcomes: Vec<PlanGoalOutcome>, start: u64) -> PlanTrace {
    let mut blackboard = Blackboard::new();
    blackboard.set("goal", u64::MAX);
    blackboard.set("reason", PlanReason::Unreachable.code());
    blackboard.set("facts", start);
    blackboard.set("cost", 0);
    blackboard.set("steps", 0);
    blackboard.set("expanded", 0);
    PlanTrace {
        goal: None,
        reason: PlanReason::Unreachable,
        steps: Vec::new(),
        cost: 0,
        expanded: 0,
        goals: outcomes,
        blackboard,
    }
}

/// Plan from the facts and show the trace.
///
/// Every goal is planned against the same start facts; the cheapest total
/// (plan cost plus the goal bias) wins, and a tie goes to the lowest goal
/// id. The trace names the winner, the reason, the runner-up and what every
/// other goal cost or refused with. The blackboard records the goal, the
/// costs, the step count, the expanded node count, the start facts and the
/// reason code, so a tape can pin the plan by [`Blackboard::state_hash`].
///
/// Deterministic: integer math only, goals tried in id order. Never panics.
#[must_use]
pub fn plan_and_trace(facts: Facts, budget: u32) -> PlanTrace {
    let actions = shipped_actions();
    let start = facts.mask();
    let (outcomes, best) = plan_every_goal(&actions, start, budget);
    let Some(best) = best else {
        return unreachable_trace(outcomes, start);
    };
    let next = runner_up(&outcomes, best.goal);
    let reason = if best.plan.steps.is_empty() {
        PlanReason::AlreadyMet
    } else {
        PlanReason::Cheapest { runner_up: next }
    };
    let (steps, final_facts) = trace_steps(&actions, start, &best.plan);
    let blackboard = trace_blackboard(&best, reason, start, final_facts, steps.len(), next);
    PlanTrace {
        goal: Some(best.goal),
        reason,
        steps,
        cost: best.total,
        expanded: best.plan.expanded,
        goals: outcomes,
        blackboard,
    }
}

/// The gladiator's plan for one percept: the facts come from the percept,
/// the tables are the shipped duel tables, and the trace says which goal
/// was chosen and why. Deterministic and integer-only; never panics.
#[must_use]
pub fn plan_for_percept(percept: &Percept) -> PlanTrace {
    plan_and_trace(facts_from_percept(percept), 256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::melee::{FoeView, MeleeView};

    fn far_percept(attacking: bool) -> Percept {
        Percept::new([0.0, 0.0, 0.0], 0.0, [10.0, 0.0, 0.0], attacking, 10.0, 7)
    }

    fn close_percept(attacking: bool) -> Percept {
        Percept::new([0.0, 0.0, 0.0], 0.0, [1.5, 0.0, 0.0], attacking, 1.5, 7)
    }

    #[test]
    fn plan_trace_a_gladiator_plans_and_the_trace_names_the_goal_and_the_reason() {
        let trace = plan_for_percept(&far_percept(false));
        assert_eq!(trace.goal, Some(PlanGoalId::Strike));
        assert_eq!(
            trace.reason,
            PlanReason::Cheapest {
                runner_up: Some(PlanGoalId::Close)
            }
        );
        assert_eq!(
            trace.steps.iter().map(|s| s.action).collect::<Vec<_>>(),
            vec![PlanAction::Close, PlanAction::Strike]
        );
        assert_eq!(trace.cost, COST_CLOSE + COST_STRIKE);
        assert_eq!(
            trace.blackboard.get("goal"),
            Some(u64::from(PlanGoalId::Strike.as_u16()))
        );
        assert_eq!(
            trace.blackboard.get("reason"),
            Some(PlanReason::Cheapest { runner_up: None }.code())
        );
        assert_eq!(trace.blackboard.get("steps"), Some(2));
        assert_eq!(
            trace.goals.iter().find(|g| g.goal == PlanGoalId::Guard),
            Some(&PlanGoalOutcome {
                goal: PlanGoalId::Guard,
                cost: None,
                refusal: Some(PlanError::NoPlan)
            })
        );
    }

    #[test]
    fn plan_trace_a_threat_from_far_chooses_guard_and_the_trace_says_why() {
        let trace = plan_for_percept(&far_percept(true));
        assert_eq!(trace.goal, Some(PlanGoalId::Guard));
        assert_eq!(
            trace.reason,
            PlanReason::Cheapest {
                runner_up: Some(PlanGoalId::Strike)
            }
        );
        assert_eq!(
            trace.steps.iter().map(|s| s.action).collect::<Vec<_>>(),
            vec![PlanAction::Guard]
        );
    }

    #[test]
    fn plan_trace_in_range_strikes_in_one_step() {
        let trace = plan_for_percept(&close_percept(false));
        assert_eq!(trace.goal, Some(PlanGoalId::Strike));
        assert_eq!(
            trace.steps.iter().map(|s| s.action).collect::<Vec<_>>(),
            vec![PlanAction::Strike]
        );
        assert_eq!(trace.cost, COST_STRIKE);
    }

    #[test]
    fn plan_trace_a_wounded_fighter_gives_ground() {
        let foe = FoeView::quantize(1, 1_500, 1_000, StrikePhase::Idle, 0);
        let melee = MeleeView::nearest(200, 1_000, &[foe]);
        let percept = close_percept(false).with_melee(melee);
        let trace = plan_for_percept(&percept);
        assert_eq!(trace.goal, Some(PlanGoalId::Back));
        assert_eq!(
            trace.steps.iter().map(|s| s.action).collect::<Vec<_>>(),
            vec![PlanAction::Back]
        );
    }

    #[test]
    fn plan_trace_steps_carry_the_fact_change() {
        let trace = plan_for_percept(&far_percept(false));
        let first = trace.steps[0];
        assert_eq!(first.action, PlanAction::Close);
        assert_eq!(first.before & F_FAR, F_FAR);
        assert_eq!(first.after & F_IN_RANGE, F_IN_RANGE);
        assert_eq!(first.after & F_FAR, 0);
    }

    #[test]
    fn plan_trace_replays_byte_equal_twice() {
        let run = || {
            let trace = plan_for_percept(&far_percept(true));
            (
                trace.goal,
                trace.reason,
                trace.steps.clone(),
                trace.blackboard.state_hash(),
            )
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn plan_trace_an_unknown_separation_guesses_nothing() {
        let percept = Percept::new([0.0, 0.0, 0.0], 0.0, [0.0, 0.0, 0.0], false, f32::NAN, 0);
        let facts = facts_from_percept(&percept);
        assert!(!facts.in_range);
        assert!(!facts.far);
        assert_eq!(facts.mask(), 0);
    }

    #[test]
    fn plan_trace_goal_ids_round_trip() {
        for goal in [
            PlanGoalId::Strike,
            PlanGoalId::Guard,
            PlanGoalId::Close,
            PlanGoalId::Back,
        ] {
            assert_eq!(PlanGoalId::from_u16(goal.as_u16()), Some(goal));
            assert!(!goal.name().is_empty());
        }
        assert_eq!(PlanGoalId::from_u16(99), None);
    }

    #[test]
    fn plan_trace_metre_thresholds_match_the_brain_thresholds() {
        // The percept path compares floats against integral thresholds; this
        // pins them to the brain's integers so a moved range fails here.
        let attack = u16::try_from(Brain::ATTACK_MM).expect("strike range fits u16");
        let far = u16::try_from(Brain::FAR_MM).expect("far ring fits u16");
        assert_eq!(ATTACK_MM, f32::from(attack));
        assert_eq!(FAR_MM, f32::from(far));
        // The truncating read classifies the boundary the same way both paths
        // do: 2 500 mm is in range, 2 501 mm is not.
        assert!(mm_from_m(2.5) <= ATTACK_MM);
        assert!(mm_from_m(2.501) > ATTACK_MM);
        assert!(mm_from_m(8.0) >= FAR_MM);
        assert!(mm_from_m(7.999) < FAR_MM);
    }
}
