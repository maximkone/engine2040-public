//! goap: A* action planner over a 64-fact integer world state.
//! Owner: callers bake [`Action`] rows (preconditions, effects, costs) once and
//! call [`plan`] when the state or the goal changes; an executor walks the
//! returned step list one action per tick.
//! Units: facts are bits of a `u64`, costs are integer ticks.
//! Determinism: integer math only; open nodes order by `(f, insertion)`, the
//! closed set is a sorted map, no `HashMap`, no wall clock; the search stops
//! at a node budget, never at a timer.
//! Errors: [`PlanError`]; an unreachable goal or a spent budget is a value.
//! PORT(victorb/dogoap, MIT): planner shape only (precondition/effect/cost
//! actions, A* over the fact state); reimplemented, no crate dependency.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

/// Longest plan returned; deeper searches refuse. Units: steps.
pub const MAX_PLAN: usize = 16;

/// One planner action over bit facts: it applies when every `pre_mask` bit of
/// the state equals `pre_value`, then sets every `set_mask` bit to `set_value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    /// Fact bits the precondition looks at.
    pub pre_mask: u64,
    /// Required values for those bits (bits outside `pre_mask` are ignored).
    pub pre_value: u64,
    /// Fact bits the effect writes.
    pub set_mask: u64,
    /// Written values for those bits (bits outside `set_mask` are ignored).
    pub set_value: u64,
    /// Cost of taking the action. Units: ticks.
    pub cost: u32,
}

impl Action {
    /// True when `state` meets the precondition.
    pub const fn applies(&self, state: u64) -> bool {
        (state ^ self.pre_value) & self.pre_mask == 0
    }
    /// State after the effect.
    pub const fn apply(&self, state: u64) -> u64 {
        (state & !self.set_mask) | (self.set_value & self.set_mask)
    }
}

/// Goal: the state must match `value` on every `mask` bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoalState {
    /// Fact bits the goal cares about.
    pub mask: u64,
    /// Wanted values for those bits.
    pub value: u64,
}

impl GoalState {
    /// True when `state` satisfies the goal.
    pub const fn met(&self, state: u64) -> bool {
        (state ^ self.value) & self.mask == 0
    }
}

/// Why [`plan`] gave no plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Every reachable state was searched and none meets the goal.
    NoPlan,
    /// The node budget ran out first; raise it or simplify the goal.
    Budget,
    /// The cheapest plan is longer than [`MAX_PLAN`] steps.
    TooLong,
}

/// A cheapest valid plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Indices into the action slice, in the order to take them.
    pub steps: Vec<u16>,
    /// Summed action cost. Units: ticks.
    pub cost: u32,
    /// Nodes expanded to find it (the tick-budget measure).
    pub expanded: u32,
}

struct Node {
    state: u64,
    g: u32,
    parent: u32,
    action: u16,
}

/// Plan from `start` to `goal`, expanding at most `budget` nodes.
///
/// A* with an admissible bound: each unmet goal bit needs an action that
/// writes it, so `ceil(unmet / most goal bits one action writes) * cheapest
/// such action` never overestimates. Ties pop in insertion order, so equal
/// inputs give equal plans; among equal-cost plans the one found first by
/// ascending action index wins.
pub fn plan(
    actions: &[Action],
    start: u64,
    goal: GoalState,
    budget: u32,
) -> Result<Plan, PlanError> {
    let writes = |a: &Action| (a.set_mask & goal.mask).count_ones();
    let per_action = actions.iter().map(writes).max().unwrap_or(0).max(1);
    let cheapest = actions
        .iter()
        .filter(|a| writes(a) > 0)
        .map(|a| a.cost)
        .min()
        .unwrap_or(0);
    let h = |s: u64| {
        ((s ^ goal.value) & goal.mask)
            .count_ones()
            .div_ceil(per_action)
            * cheapest
    };

    let mut nodes = vec![Node {
        state: start,
        g: 0,
        parent: u32::MAX,
        action: 0,
    }];
    let mut best: BTreeMap<u64, u32> = BTreeMap::from([(start, 0)]);
    let mut open = BinaryHeap::from([Reverse((h(start), 0u32))]);
    let mut expanded = 0u32;
    while let Some(Reverse((_, id))) = open.pop() {
        let (state, g) = (nodes[id as usize].state, nodes[id as usize].g);
        if best.get(&state).is_some_and(|&b| g > b) {
            continue; // a cheaper route to this state was found after the push
        }
        if goal.met(state) {
            return unwind(&nodes, id, expanded);
        }
        if expanded >= budget {
            return Err(PlanError::Budget);
        }
        expanded += 1;
        for (i, a) in actions.iter().enumerate().take(usize::from(u16::MAX)) {
            let next = a.apply(state);
            if !a.applies(state) || next == state {
                continue;
            }
            let ng = g.saturating_add(a.cost);
            if best.get(&next).is_some_and(|&b| b <= ng) {
                continue;
            }
            best.insert(next, ng);
            nodes.push(Node {
                state: next,
                g: ng,
                parent: id,
                action: u16::try_from(i).expect("take bounds the index"),
            });
            let nid = u32::try_from(nodes.len() - 1).map_err(|_| PlanError::Budget)?;
            open.push(Reverse((ng.saturating_add(h(next)), nid)));
        }
    }
    Err(PlanError::NoPlan)
}

fn unwind(nodes: &[Node], goal_id: u32, expanded: u32) -> Result<Plan, PlanError> {
    let mut steps = Vec::new();
    let mut at = goal_id as usize;
    while nodes[at].parent != u32::MAX {
        steps.push(nodes[at].action);
        at = nodes[at].parent as usize;
    }
    if steps.len() > MAX_PLAN {
        return Err(PlanError::TooLong);
    }
    steps.reverse();
    Ok(Plan {
        steps,
        cost: nodes[goal_id as usize].g,
        expanded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::splitmix64;

    const FACTS: u32 = 10;

    /// Seeded rows: a guaranteed chain `bit i-1 -> bit i` plus random decoys
    /// that may clobber the chain. Goal is the first `reach + 1` chain bits.
    fn seeded(seed: u64) -> (Vec<Action>, GoalState) {
        let mut s = seed;
        let mut draw = |n: u64| {
            s = splitmix64(s);
            s % n
        };
        let reach = 2 + draw(3);
        let mut rows = vec![Action {
            pre_mask: 0,
            pre_value: 0,
            set_mask: 1,
            set_value: 1,
            cost: 1 + draw(5) as u32,
        }];
        for i in 1..=reach {
            rows.push(Action {
                pre_mask: 1 << (i - 1),
                pre_value: 1 << (i - 1),
                set_mask: 1 << i,
                set_value: 1 << i,
                cost: 1 + draw(9) as u32,
            });
        }
        for _ in 0..10 {
            let pre_mask = draw(1 << FACTS);
            let set_mask = 1 + draw((1 << FACTS) - 1);
            rows.push(Action {
                pre_mask,
                pre_value: draw(1 << FACTS) & pre_mask,
                set_mask,
                set_value: draw(1 << FACTS) & set_mask,
                cost: 1 + draw(9) as u32,
            });
        }
        let mask = (1 << (reach + 1)) - 1;
        (rows, GoalState { mask, value: mask })
    }

    /// Bellman-Ford over every state: the cheapest cost, independent of A*.
    fn cheapest(actions: &[Action], start: u64, goal: GoalState) -> Option<u32> {
        let mut dist = vec![u32::MAX; 1 << FACTS];
        dist[start as usize] = 0;
        for _ in 0..(1 << FACTS) {
            let mut moved = false;
            for st in 0..(1u64 << FACTS) {
                let d = dist[st as usize];
                if d == u32::MAX {
                    continue;
                }
                for a in actions.iter().filter(|a| a.applies(st)) {
                    let n = a.apply(st) as usize;
                    if d + a.cost < dist[n] {
                        dist[n] = d + a.cost;
                        moved = true;
                    }
                }
            }
            if !moved {
                break;
            }
        }
        (0..(1u64 << FACTS))
            .filter(|&st| goal.met(st))
            .map(|st| dist[st as usize])
            .min()
            .filter(|&d| d != u32::MAX)
    }

    fn walk(actions: &[Action], start: u64, steps: &[u16]) -> u64 {
        steps.iter().fold(start, |st, &i| {
            let a = &actions[i as usize];
            assert!(a.applies(st), "step {i} does not apply");
            a.apply(st)
        })
    }

    #[test]
    fn goap_plan_twenty_seeded_goal_sets_plan_cheapest_and_valid() {
        for seed in 0..20 {
            let (rows, goal) = seeded(seed);
            let p = plan(&rows, 0, goal, 4096).unwrap_or_else(|e| panic!("seed {seed}: {e:?}"));
            assert!(goal.met(walk(&rows, 0, &p.steps)), "seed {seed}");
            assert_eq!(Some(p.cost), cheapest(&rows, 0, goal), "seed {seed}");
            let paid: u32 = p.steps.iter().map(|&i| rows[i as usize].cost).sum();
            assert_eq!(paid, p.cost, "seed {seed}");
        }
    }

    #[test]
    fn goap_plan_picks_the_cheap_route_not_the_short_one() {
        let set = |bit: u32, pre: u64, cost| Action {
            pre_mask: pre,
            pre_value: pre,
            set_mask: 1 << bit,
            set_value: 1 << bit,
            cost,
        };
        // One step costs 10; two steps cost 2 + 3.
        let rows = [set(2, 0, 10), set(0, 0, 2), set(2, 1, 3)];
        let goal = GoalState { mask: 4, value: 4 };
        let p = plan(&rows, 0, goal, 64).unwrap();
        assert_eq!((p.steps, p.cost), (vec![1, 2], 5));
    }

    #[test]
    fn goap_plan_replan_after_a_state_change_stays_in_budget() {
        let (rows, goal) = seeded(7);
        let first = plan(&rows, 0, goal, 4096).unwrap();
        for flipped in 0..FACTS {
            let state = 1u64 << flipped;
            let p = plan(&rows, state, goal, 256)
                .unwrap_or_else(|e| panic!("flip {flipped}: {e:?} after {first:?}"));
            assert!(p.expanded <= 256);
            assert!(goal.met(walk(&rows, state, &p.steps)));
        }
    }

    #[test]
    fn goap_plan_refusals_are_values() {
        let (rows, goal) = seeded(3);
        assert_eq!(plan(&rows, 0, goal, 0), Err(PlanError::Budget));
        assert_eq!(plan(&[], 0, goal, 64), Err(PlanError::NoPlan));
        let met = plan(&rows, goal.value, goal, 0).unwrap();
        assert_eq!((met.steps.len(), met.cost), (0, 0));
        let long: Vec<Action> = (0..=MAX_PLAN as u32)
            .map(|i| Action {
                pre_mask: if i == 0 { 0 } else { 1 << (i - 1) },
                pre_value: if i == 0 { 0 } else { 1 << (i - 1) },
                set_mask: 1 << i,
                set_value: 1 << i,
                cost: 1,
            })
            .collect();
        let far = GoalState {
            mask: 1 << MAX_PLAN,
            value: 1 << MAX_PLAN,
        };
        assert_eq!(plan(&long, 0, far, 4096), Err(PlanError::TooLong));
    }

    #[test]
    fn goap_plan_replays_byte_equal_twice() {
        let run = || {
            (0..20)
                .map(|seed| {
                    let (rows, goal) = seeded(seed);
                    plan(&rows, 0, goal, 4096)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
