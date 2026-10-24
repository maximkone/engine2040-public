//! agent_loop: bounded perceive-plan-act loop over door tools.
//! Owner: provider harness. Units: steps as counts, tokens as estimates.
//! Determinism: same goal plus same planner and door answers always yield
//! the same transcript and checkpoint; no wall clock, no thread order.
//! Errors: [`LoopError`] names budget, stuck, tool and fold refusals.
//! Compaction: a loop built with [`AgentLoop::compacting`] folds its oldest
//! completed steps into one [`SUMMARY_TOOL`] step when a bound would stop it,
//! keeps the last `keep` steps verbatim, never folds a step whose door write
//! is still open, and reports each fold in [`LoopOutcome::folds`]. An unfolded
//! run over the same budget is byte-identical to a run built with
//! [`AgentLoop::new`].
//! Status: the consumer is `engine-endpoint::audit::agent_run`: a scripted
//! task runs read -> preview -> apply over the in-process door and every call
//! lands in the audit chain with its args, digest and both revisions.
//! PORT(0xPlaygrounds/rig agent shape, reimplemented std-only, no dep)

use engine_core::Fnv64;

/// Largest steps one loop may take before it stops and reports.
pub const MAX_STEPS: u32 = 16;
/// Largest estimated tokens one loop may spend before it stops.
pub const MAX_TOKENS: u32 = 4096;
/// Longest tool name accepted, in bytes.
pub const MAX_TOOL_NAME_LEN: usize = 64;
/// Longest tool input, observation or answer accepted, in bytes.
pub const MAX_TOOL_TEXT_LEN: usize = 2048;
/// Most bytes of tool names one fold summary lists.
pub const MAX_SUMMARY_TOOLS_BYTES: usize = 512;
/// Tool name on the summary step a fold writes.
pub const SUMMARY_TOOL: &str = "loop.compact";

/// Bounded budget for one loop run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoopBudget {
    /// Most tool calls before the loop stops.
    pub max_steps: u32,
    /// Most estimated tokens before the loop stops.
    pub max_tokens: u32,
}

impl LoopBudget {
    /// Clamped budget: steps to `1..=MAX_STEPS`, tokens to `1..=MAX_TOKENS`.
    /// Clamping keeps every loop bounded even when callers misconfigure.
    #[must_use]
    pub fn new(max_steps: u32, max_tokens: u32) -> Self {
        Self {
            max_steps: max_steps.clamp(1, MAX_STEPS),
            max_tokens: max_tokens.clamp(1, MAX_TOKENS),
        }
    }

    /// Default offline budget: eight steps and two thousand tokens.
    #[must_use]
    pub fn default_budget() -> Self {
        Self {
            max_steps: 8,
            max_tokens: 2000,
        }
    }
}

/// Which bound stopped the loop.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BudgetKind {
    /// Tool-call count reached `max_steps`.
    Steps,
    /// Estimated token spend reached `max_tokens`.
    Tokens,
}

impl core::fmt::Display for BudgetKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BudgetKind::Steps => f.write_str("steps"),
            BudgetKind::Tokens => f.write_str("tokens"),
        }
    }
}

/// Typed loop failure.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LoopError {
    /// A bound stopped the loop; `used` reached `limit`.
    BudgetOverrun {
        /// Which bound stopped the loop.
        kind: BudgetKind,
        /// Steps or tokens used when the loop stopped.
        used: u32,
        /// The bound that was set.
        limit: u32,
    },
    /// The same tool plus input repeated without progress.
    Stuck {
        /// Tool that repeated.
        tool: String,
    },
    /// No tool under this name exists on the door.
    UnknownTool {
        /// Requested tool name.
        tool: String,
    },
    /// The door refused the call without running it.
    ToolRefused {
        /// Tool that refused.
        tool: String,
        /// Short machine reason, bounded.
        reason: String,
    },
    /// The planner returned nothing to do.
    EmptyPlan,
    /// The goal text exceeds the loop text bound.
    GoalTooLong {
        /// Goal byte length.
        len: usize,
    },
    /// Compaction would have to fold a step whose door write is still open.
    CannotFold {
        /// Index of the step that blocks the fold.
        step: u32,
        /// Idempotency key of that write.
        key: u64,
    },
}

impl LoopError {
    /// Short stable name for logs and receipts, never caller text.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            LoopError::BudgetOverrun { .. } => "budget_overrun",
            LoopError::Stuck { .. } => "stuck",
            LoopError::UnknownTool { .. } => "unknown_tool",
            LoopError::ToolRefused { .. } => "tool_refused",
            LoopError::EmptyPlan => "empty_plan",
            LoopError::GoalTooLong { .. } => "goal_too_long",
            LoopError::CannotFold { .. } => "cannot_fold",
        }
    }
}

impl core::fmt::Display for LoopError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoopError::BudgetOverrun { kind, used, limit } => {
                write!(
                    f,
                    "agent loop budget overrun ({kind}): used {used} of {limit}"
                )
            }
            LoopError::Stuck { tool } => {
                write!(f, "agent loop stuck: tool {tool} repeated without progress")
            }
            LoopError::UnknownTool { tool } => {
                write!(f, "agent loop unknown tool: {tool}")
            }
            LoopError::ToolRefused { tool, reason } => {
                write!(f, "agent loop tool refused: {tool} ({reason})")
            }
            LoopError::EmptyPlan => f.write_str("agent loop planner returned no plan"),
            LoopError::GoalTooLong { len } => {
                write!(f, "agent loop goal too long: {len} bytes")
            }
            LoopError::CannotFold { step, key } => write!(
                f,
                "agent loop cannot fold: step {step} is a pending write (key {key})"
            ),
        }
    }
}

impl std::error::Error for LoopError {}

/// One planner decision: call a door tool or finish with an answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Plan {
    /// Call one door tool once under an idempotency key.
    ToolCall {
        /// Door verb to run.
        tool: String,
        /// Bounded input for the verb.
        input: String,
        /// Caller-chosen idempotency key; repeats replay, never rerun.
        idempotency_key: u64,
    },
    /// Stop with the final answer.
    Done {
        /// Bounded final answer.
        answer: String,
    },
}

/// One recorded tool step: tick-stamped before the door runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StepRecord {
    /// Zero-based step index.
    pub step: u32,
    /// Door verb that ran.
    pub tool: String,
    /// Input the verb ran with.
    pub input: String,
    /// Observation the verb returned.
    pub observation: String,
    /// Estimated tokens for this step.
    pub tokens: u32,
    /// Idempotency key the step ran under.
    pub idempotency_key: u64,
}

/// One fold the loop made: how much it folded and what it bought back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FoldRecord {
    /// Steps folded into the summary step.
    pub folded: u32,
    /// Steps in the transcript after the fold, the summary included.
    pub kept: u32,
    /// Tokens the fold gave back to the budget.
    pub reclaimed: u32,
    /// Transcript checkpoint right after the fold.
    pub checkpoint: u64,
}

/// Completed loop: final answer plus its replayable transcript.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LoopOutcome {
    /// Final answer the planner returned.
    pub answer: String,
    /// Recorded tool steps in run order.
    pub steps: Vec<StepRecord>,
    /// Estimated tokens spent including the goal and answer.
    pub tokens_used: u32,
    /// Folds made to stay inside the budget, in run order.
    pub folds: Vec<FoldRecord>,
}

impl LoopOutcome {
    /// Deterministic checkpoint over the transcript; same steps always give
    /// the same value, any changed step changes it.
    #[must_use]
    pub fn checkpoint(&self) -> u64 {
        transcript_checkpoint(&self.steps)
    }

    /// Recorded steps in run order.
    #[must_use]
    pub fn steps(&self) -> &[StepRecord] {
        &self.steps
    }
}

/// Planner role: fixed perceive-plan half of the loop. Sees the transcript
/// so far and either names the next door tool or finishes.
pub trait Planner {
    /// Plan the next step from the transcript so far.
    /// Errors: [`LoopError::EmptyPlan`] when there is nothing to do.
    fn plan(&mut self, transcript: &[StepRecord]) -> Result<Plan, LoopError>;
}

/// Blanket adapter so closures serve as offline planners in tests.
impl<F> Planner for F
where
    F: FnMut(&[StepRecord]) -> Result<Plan, LoopError>,
{
    fn plan(&mut self, transcript: &[StepRecord]) -> Result<Plan, LoopError> {
        self(transcript)
    }
}

/// Door role: fixed act half of the loop. Runs one named verb under an
/// idempotency key, states its cost up front and can undo a keyed call.
pub trait Door {
    /// Names of the verbs this door serves.
    fn tool_names(&self) -> Vec<String>;

    /// Estimated token cost of one verb before it runs.
    fn cost_of(&self, tool: &str) -> u64;

    /// Run one verb once. Repeating a finished key replays its observation.
    /// Errors: [`LoopError::UnknownTool`] or [`LoopError::ToolRefused`].
    fn act(&mut self, tool: &str, input: &str, key: u64) -> Result<String, LoopError>;

    /// Undo the call recorded under `key`; true when something was undone.
    fn undo(&mut self, key: u64) -> bool;

    /// True when `key` is a write this door can still undo, so compaction
    /// must never fold that step into a summary. Default: the door reports
    /// no open writes and every step is foldable.
    fn pending_write(&self, tool: &str, key: u64) -> bool {
        let _ = (tool, key);
        false
    }
}

/// Offline scripted planner: plays a fixed queue, then reports empty.
/// Deterministic stand-in for a model adapter with no network.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScriptedPlanner {
    /// Queued plans in play order.
    plans: Vec<Plan>,
    /// Plans already served.
    cursor: usize,
}

impl ScriptedPlanner {
    /// Queue `plans` in play order.
    #[must_use]
    pub fn new(plans: Vec<Plan>) -> Self {
        Self { plans, cursor: 0 }
    }

    /// Plans not yet served.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.plans.len().saturating_sub(self.cursor)
    }
}

impl Planner for ScriptedPlanner {
    fn plan(&mut self, _transcript: &[StepRecord]) -> Result<Plan, LoopError> {
        if self.cursor < self.plans.len() {
            let next = self.plans[self.cursor].clone();
            self.cursor = self.cursor.saturating_add(1);
            Ok(next)
        } else {
            Err(LoopError::EmptyPlan)
        }
    }
}

/// In-memory door for offline runs: canned observations per verb name.
/// Records every keyed call so repeats replay and undo works.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MemoryDoor {
    /// Canned `(tool, observation)` pairs.
    table: Vec<(String, String)>,
    /// `(tool, input, key)` calls already served, in order.
    log: Vec<(String, String, u64)>,
    /// Verbs that write: a logged call to one of them stays undoable, so
    /// compaction must not fold it.
    writes: Vec<String>,
}

impl MemoryDoor {
    /// Serve `entries` canned `(tool, observation)` pairs.
    #[must_use]
    pub fn new(entries: Vec<(String, String)>) -> Self {
        Self {
            table: entries,
            log: Vec::new(),
            writes: Vec::new(),
        }
    }

    /// Mark `writes` as verbs that change state: their logged calls report
    /// [`Door::pending_write`] until [`Door::undo`] removes them.
    #[must_use]
    pub fn with_writes(entries: Vec<(String, String)>, writes: Vec<String>) -> Self {
        Self {
            table: entries,
            log: Vec::new(),
            writes,
        }
    }

    /// True when a verb under this name exists.
    #[must_use]
    pub fn has_tool(&self, tool: &str) -> bool {
        self.table.iter().any(|(name, _)| name == tool)
    }

    /// Keyed calls served so far.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.log.len()
    }

    fn canned(&self, tool: &str) -> Option<String> {
        self.table
            .iter()
            .find(|(name, _)| name == tool)
            .map(|(_, observation)| observation.clone())
    }
}

impl Door for MemoryDoor {
    fn tool_names(&self) -> Vec<String> {
        self.table
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<String>>()
    }

    fn cost_of(&self, tool: &str) -> u64 {
        match self.canned(tool) {
            Some(observation) => u64::from(estimate_tokens(&observation)),
            None => 0,
        }
    }

    fn act(&mut self, tool: &str, input: &str, key: u64) -> Result<String, LoopError> {
        if tool.is_empty() || tool.len() > MAX_TOOL_NAME_LEN {
            return Err(LoopError::UnknownTool {
                tool: tool.to_owned(),
            });
        }
        if input.len() > MAX_TOOL_TEXT_LEN {
            return Err(LoopError::ToolRefused {
                tool: tool.to_owned(),
                reason: String::from("input too long"),
            });
        }
        if let Some(prior) = self
            .log
            .iter()
            .find(|(_, _, logged_key)| *logged_key == key)
        {
            let replay_tool = prior.0.clone();
            match self.canned(&replay_tool) {
                Some(observation) => return Ok(observation),
                None => return Err(LoopError::UnknownTool { tool: replay_tool }),
            }
        }
        match self.canned(tool) {
            Some(observation) => {
                self.log.push((tool.to_owned(), input.to_owned(), key));
                Ok(observation)
            }
            None => Err(LoopError::UnknownTool {
                tool: tool.to_owned(),
            }),
        }
    }

    fn undo(&mut self, key: u64) -> bool {
        match self.log.iter().position(|(_, _, logged)| *logged == key) {
            Some(index) => {
                self.log.remove(index);
                true
            }
            None => false,
        }
    }

    fn pending_write(&self, tool: &str, key: u64) -> bool {
        self.writes.iter().any(|write| write == tool)
            && self
                .log
                .iter()
                .any(|(logged_tool, _, logged_key)| logged_tool == tool && *logged_key == key)
    }
}

/// Rough token estimate for loop budgets: one token per four bytes.
/// Pure integer math, identical on every platform.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    let len = u64::try_from(text.len()).unwrap_or(u64::MAX);
    let estimate = len.saturating_add(3) / 4;
    u32::try_from(estimate).unwrap_or(u32::MAX)
}

/// Deterministic checkpoint over a transcript in run order.
/// Lengths prefix each text so adjacent fields cannot collide.
#[must_use]
pub fn transcript_checkpoint(steps: &[StepRecord]) -> u64 {
    let mut hash = Fnv64::new();
    for record in steps {
        hash.write_u32(record.step);
        hash.write_u64(u64::try_from(record.tool.len()).unwrap_or(u64::MAX));
        hash.write_str(&record.tool);
        hash.write_u64(u64::try_from(record.input.len()).unwrap_or(u64::MAX));
        hash.write_str(&record.input);
        hash.write_u64(u64::try_from(record.observation.len()).unwrap_or(u64::MAX));
        hash.write_str(&record.observation);
        hash.write_u64(record.idempotency_key);
        hash.write_u32(record.tokens);
    }
    hash.finish()
}

/// Bounded perceive-plan-act loop over door tools.
/// The planner never touches the door and the door never plans: the loop
/// owns the budget, the transcript and the stop rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AgentLoop {
    /// Budget this loop enforces.
    pub budget: LoopBudget,
    /// Steps kept verbatim by compaction, or `None` when the loop never folds.
    compact_keep: Option<u32>,
}

impl AgentLoop {
    /// Build a loop enforcing `budget`.
    #[must_use]
    pub fn new(budget: LoopBudget) -> Self {
        Self {
            budget,
            compact_keep: None,
        }
    }

    /// Build a loop that folds its oldest steps when a bound would stop it,
    /// keeping the last `keep` steps verbatim. `keep` is clamped to
    /// `1..=MAX_STEPS`, and a fold needs at least one more step than `keep`
    /// to gain anything.
    #[must_use]
    pub fn compacting(budget: LoopBudget, keep: u32) -> Self {
        Self {
            budget,
            compact_keep: Some(keep.clamp(1, MAX_STEPS)),
        }
    }

    /// Steps kept verbatim by compaction, or `None` when the loop never folds.
    #[must_use]
    pub const fn compaction(&self) -> Option<u32> {
        self.compact_keep
    }

    /// Run `goal` to done under the budget.
    /// Perceive reads the transcript, plan names the next tool, act runs it.
    /// Stops done on [`Plan::Done`], stuck on a repeated tool plus input and
    /// overrun when steps or tokens reach the budget.
    /// Errors: [`LoopError`] for budget, stuck, unknown or refused tools,
    /// empty plans and over-long goals.
    pub fn run(
        &self,
        goal: &str,
        planner: &mut impl Planner,
        door: &mut impl Door,
    ) -> Result<LoopOutcome, LoopError> {
        if goal.len() > MAX_TOOL_TEXT_LEN {
            return Err(LoopError::GoalTooLong { len: goal.len() });
        }
        let mut tokens_used = estimate_tokens(goal);
        if tokens_used > self.budget.max_tokens {
            return Err(LoopError::BudgetOverrun {
                kind: BudgetKind::Tokens,
                used: tokens_used,
                limit: self.budget.max_tokens,
            });
        }
        let mut steps: Vec<StepRecord> = Vec::new();
        let mut folds: Vec<FoldRecord> = Vec::new();
        let mut last_tool = String::new();
        let mut last_input = String::new();
        let mut repeats: u32 = 0;
        loop {
            let spent = u64::try_from(steps.len()).unwrap_or(u64::MAX);
            if spent >= u64::from(self.budget.max_steps) {
                if self.fold(&mut steps, door, &mut tokens_used, &mut folds)? {
                    continue;
                }
                let used = u32::try_from(steps.len()).unwrap_or(u32::MAX);
                return Err(LoopError::BudgetOverrun {
                    kind: BudgetKind::Steps,
                    used,
                    limit: self.budget.max_steps,
                });
            }
            if tokens_used > self.budget.max_tokens {
                if self.fold(&mut steps, door, &mut tokens_used, &mut folds)? {
                    continue;
                }
                return Err(LoopError::BudgetOverrun {
                    kind: BudgetKind::Tokens,
                    used: tokens_used,
                    limit: self.budget.max_tokens,
                });
            }
            match planner.plan(&steps)? {
                Plan::Done { answer } => {
                    return self.finish(goal, &steps, &answer, tokens_used, folds);
                }
                Plan::ToolCall {
                    tool,
                    input,
                    idempotency_key,
                } => {
                    repeats =
                        check_repeat(&tool, &input, &mut last_tool, &mut last_input, repeats)?;
                    let observation = door.act(&tool, &input, idempotency_key)?;
                    if observation.len() > MAX_TOOL_TEXT_LEN {
                        return Err(LoopError::ToolRefused {
                            tool,
                            reason: String::from("observation too long"),
                        });
                    }
                    let cost = step_cost(&tool, &input, &observation);
                    tokens_used = tokens_used.saturating_add(cost);
                    let index = u32::try_from(steps.len()).unwrap_or(u32::MAX);
                    steps.push(StepRecord {
                        step: index,
                        tool,
                        input,
                        observation,
                        tokens: cost,
                        idempotency_key,
                    });
                }
            }
        }
    }

    /// Fold the oldest completed steps into one summary step when a bound has
    /// been reached. Returns `true` when the transcript changed, so the caller
    /// can retry the same iteration on the same budget.
    ///
    /// A step whose door write is still open is never folded: the fold stops
    /// before it, and a fold that would have to start at one is refused with
    /// [`LoopError::CannotFold`].
    fn fold(
        &self,
        steps: &mut Vec<StepRecord>,
        door: &impl Door,
        tokens_used: &mut u32,
        folds: &mut Vec<FoldRecord>,
    ) -> Result<bool, LoopError> {
        let Some(keep) = self.compact_keep else {
            return Ok(false);
        };
        let keep = usize::try_from(keep).unwrap_or(usize::MAX);
        // A fold has to gain a step to be worth doing: keep + 1 is the floor.
        if steps.len() <= keep.saturating_add(1) {
            return Ok(false);
        }
        let mut cut = steps.len() - keep;
        let pending = steps
            .iter()
            .take(cut)
            .position(|step| door.pending_write(&step.tool, step.idempotency_key));
        if let Some(index) = pending {
            if index == 0 {
                let first = &steps[0];
                return Err(LoopError::CannotFold {
                    step: first.step,
                    key: first.idempotency_key,
                });
            }
            cut = index;
        }
        // A fold must give at least one step back: folding one step into one
        // summary leaves the length where it was, so it would never end.
        if cut < 2 {
            return Ok(false);
        }
        let folded = u32::try_from(cut).unwrap_or(u32::MAX);
        let mut list = String::new();
        {
            let mut names: Vec<&str> = Vec::new();
            for step in &steps[..cut] {
                if !names.contains(&step.tool.as_str()) {
                    names.push(&step.tool);
                }
            }
            for name in names {
                if list.len().saturating_add(name.len()) > MAX_SUMMARY_TOOLS_BYTES {
                    break;
                }
                if !list.is_empty() {
                    list.push(',');
                }
                list.push_str(name);
            }
        }
        let tool = String::from(SUMMARY_TOOL);
        let observation = format!("{folded} steps folded; tools: {list}");
        let before: u32 = steps[..cut]
            .iter()
            .fold(0u32, |sum, step| sum.saturating_add(step.tokens));
        // The reclaim number sits in the input at a fixed width, so the input
        // length does not depend on its value: costs stay exact and the fold
        // is settled in one pass. Sixteen steps cap `before` far below 1e8.
        let probe = format!("folded={folded} reclaimed=00000000 keep={keep}");
        let spent = step_cost(&tool, &probe, &observation);
        let reclaimed = before.saturating_sub(spent);
        let input = format!("folded={folded} reclaimed={reclaimed:08} keep={keep}");
        let summary = StepRecord {
            step: 0,
            tool,
            input,
            observation,
            tokens: spent,
            idempotency_key: 0,
        };
        let mut next: Vec<StepRecord> = Vec::with_capacity(steps.len() - cut + 1);
        next.push(summary);
        next.extend(steps.drain(cut..));
        for (index, step) in next.iter_mut().enumerate() {
            step.step = u32::try_from(index).unwrap_or(u32::MAX);
        }
        *steps = next;
        *tokens_used = tokens_used.saturating_sub(reclaimed);
        folds.push(FoldRecord {
            folded,
            kept: u32::try_from(steps.len()).unwrap_or(u32::MAX),
            reclaimed,
            checkpoint: transcript_checkpoint(steps),
        });
        Ok(true)
    }

    fn finish(
        &self,
        _goal: &str,
        steps: &[StepRecord],
        answer: &str,
        spent: u32,
        folds: Vec<FoldRecord>,
    ) -> Result<LoopOutcome, LoopError> {
        if answer.len() > MAX_TOOL_TEXT_LEN {
            return Err(LoopError::ToolRefused {
                tool: String::from("answer"),
                reason: String::from("answer too long"),
            });
        }
        let total = spent.saturating_add(estimate_tokens(answer));
        if total > self.budget.max_tokens {
            return Err(LoopError::BudgetOverrun {
                kind: BudgetKind::Tokens,
                used: total,
                limit: self.budget.max_tokens,
            });
        }
        Ok(LoopOutcome {
            answer: answer.to_owned(),
            steps: steps.to_vec(),
            tokens_used: total,
            folds,
        })
    }
}

/// Track consecutive identical tool plus input pairs.
/// The third identical call in a row reports stuck.
fn check_repeat(
    tool: &str,
    input: &str,
    last_tool: &mut String,
    last_input: &mut String,
    repeats: u32,
) -> Result<u32, LoopError> {
    if tool.is_empty() || tool.len() > MAX_TOOL_NAME_LEN {
        return Err(LoopError::UnknownTool {
            tool: tool.to_owned(),
        });
    }
    if input.len() > MAX_TOOL_TEXT_LEN {
        return Err(LoopError::ToolRefused {
            tool: tool.to_owned(),
            reason: String::from("input too long"),
        });
    }
    if tool == last_tool && input == last_input {
        let next = repeats.saturating_add(1);
        if next >= 2 {
            return Err(LoopError::Stuck {
                tool: tool.to_owned(),
            });
        }
        Ok(next)
    } else {
        tool.clone_into(last_tool);
        input.clone_into(last_input);
        Ok(0)
    }
}

/// Estimated tokens one tool step costs: its verb, input and observation.
fn step_cost(tool: &str, input: &str, observation: &str) -> u32 {
    estimate_tokens(tool)
        .saturating_add(estimate_tokens(input))
        .saturating_add(estimate_tokens(observation))
}

/// Offline three-step demo wiring the loop, planner and door together.
/// Caller for the loop API inside this module: runs inspect, place and
/// verify verbs, then finishes. Fully offline and deterministic.
/// Errors: [`LoopError`] exactly as [`AgentLoop::run`] reports.
pub fn run_offline_three_step(budget: LoopBudget) -> Result<LoopOutcome, LoopError> {
    let plans = vec![
        Plan::ToolCall {
            tool: String::from("inspect"),
            input: String::from("room"),
            idempotency_key: 1,
        },
        Plan::ToolCall {
            tool: String::from("place"),
            input: String::from("brazier north"),
            idempotency_key: 2,
        },
        Plan::ToolCall {
            tool: String::from("verify"),
            input: String::from("brazier placed"),
            idempotency_key: 3,
        },
        Plan::Done {
            answer: String::from("brazier placed north"),
        },
    ];
    let mut planner = ScriptedPlanner::new(plans);
    let mut door = MemoryDoor::new(vec![
        (String::from("inspect"), String::from("room: bare sand")),
        (String::from("place"), String::from("placed brazier")),
        (String::from("verify"), String::from("brazier stands")),
    ]);
    AgentLoop::new(budget).run("offline three-step", &mut planner, &mut door)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget_steps(steps: u32) -> LoopBudget {
        LoopBudget::new(steps, MAX_TOKENS)
    }

    #[test]
    fn agent_loop_runs_three_step_tool_task_offline() {
        let outcome =
            run_offline_three_step(LoopBudget::default_budget()).expect("offline task finishes");
        assert_eq!(outcome.steps.len(), 3, "three tool steps ran");
        assert_eq!(outcome.answer, "brazier placed north");
        assert_eq!(outcome.steps[0].tool, "inspect");
        assert_eq!(outcome.steps[1].tool, "place");
        assert_eq!(outcome.steps[2].tool, "verify");
        assert_eq!(outcome.steps[0].step, 0);
        assert_eq!(outcome.steps[2].step, 2);
        let step_sum: u32 = outcome.steps.iter().map(|record| record.tokens).sum();
        assert!(step_sum > 0, "every step records its own cost");
        assert_eq!(
            outcome.tokens_used,
            estimate_tokens("offline three-step")
                + step_sum
                + estimate_tokens("brazier placed north"),
            "total is goal plus steps plus answer"
        );
        assert_eq!(
            outcome.steps[0].tokens,
            estimate_tokens("inspect")
                + estimate_tokens("room")
                + estimate_tokens("room: bare sand")
        );
        assert_eq!(outcome.checkpoint(), transcript_checkpoint(&outcome.steps));
    }

    #[test]
    fn agent_loop_budget_overrun_stops_with_named_error() {
        let short = run_offline_three_step(budget_steps(2));
        match short {
            Err(LoopError::BudgetOverrun { kind, used, limit }) => {
                assert_eq!(kind, BudgetKind::Steps);
                assert_eq!(limit, 2);
                assert_eq!(used, 2);
                let text = LoopError::BudgetOverrun { kind, used, limit }.to_string();
                assert!(text.contains("steps"), "named bound in text: {text}");
                assert!(text.contains("budget"), "named error in text: {text}");
            }
            other => panic!("two-step budget must overrun, got {other:?}"),
        }
        let tiny = run_offline_three_step(LoopBudget::new(MAX_STEPS, 1));
        match tiny {
            Err(LoopError::BudgetOverrun { kind, .. }) => {
                assert_eq!(kind, BudgetKind::Tokens);
            }
            other => panic!("one-token budget must overrun tokens, got {other:?}"),
        }
    }

    #[test]
    fn agent_loop_transcript_replays_twice() {
        let first =
            run_offline_three_step(LoopBudget::default_budget()).expect("first run finishes");
        let second =
            run_offline_three_step(LoopBudget::default_budget()).expect("second run finishes");
        assert_eq!(first.steps, second.steps, "same task replays same steps");
        assert_eq!(first.answer, second.answer);
        assert_eq!(first.tokens_used, second.tokens_used);
        assert_eq!(
            first.checkpoint(),
            second.checkpoint(),
            "replays share the checkpoint"
        );
        assert_eq!(
            format!("{:?}", first.steps),
            format!("{:?}", second.steps),
            "debug replay is byte-identical"
        );
    }

    #[test]
    fn agent_loop_stuck_loop_stops() {
        let plans = vec![
            Plan::ToolCall {
                tool: String::from("inspect"),
                input: String::from("room"),
                idempotency_key: 11,
            },
            Plan::ToolCall {
                tool: String::from("inspect"),
                input: String::from("room"),
                idempotency_key: 12,
            },
            Plan::ToolCall {
                tool: String::from("inspect"),
                input: String::from("room"),
                idempotency_key: 13,
            },
        ];
        let mut planner = ScriptedPlanner::new(plans);
        let mut door = MemoryDoor::new(vec![(
            String::from("inspect"),
            String::from("room: bare sand"),
        )]);
        let looped =
            AgentLoop::new(LoopBudget::default_budget()).run("stuck", &mut planner, &mut door);
        match looped {
            Err(LoopError::Stuck { tool }) => assert_eq!(tool, "inspect"),
            other => panic!("repeated tool must report stuck, got {other:?}"),
        }
    }

    #[test]
    fn agent_loop_unknown_tool_refuses() {
        let mut planner = ScriptedPlanner::new(vec![Plan::ToolCall {
            tool: String::from("shell.exec"),
            input: String::from("nope"),
            idempotency_key: 21,
        }]);
        let mut door = MemoryDoor::new(vec![(
            String::from("inspect"),
            String::from("room: bare sand"),
        )]);
        let refused =
            AgentLoop::new(LoopBudget::default_budget()).run("refused", &mut planner, &mut door);
        assert!(
            matches!(refused, Err(LoopError::UnknownTool { .. })),
            "missing verb refuses, got {refused:?}"
        );
        assert!(!door.undo(21), "refused call logs nothing to undo");
        let observed = door.act("inspect", "room", 22).expect("known verb runs");
        assert_eq!(observed, "room: bare sand");
        assert_eq!(door.call_count(), 1);
        assert!(door.undo(22), "logged call undoes once");
        assert!(!door.undo(22), "second undo finds nothing");
        assert!(door.has_tool("inspect"));
        assert_eq!(
            door.tool_names(),
            vec![String::from("inspect")],
            "door names its verbs"
        );
        assert!(door.cost_of("inspect") > 0);
        assert_eq!(door.cost_of("shell.exec"), 0);
    }

    #[test]
    fn agent_loop_empty_plan_and_goal_bounds() {
        let mut planner = ScriptedPlanner::new(Vec::new());
        let mut door = MemoryDoor::new(Vec::new());
        let empty =
            AgentLoop::new(LoopBudget::default_budget()).run("goal", &mut planner, &mut door);
        assert_eq!(empty, Err(LoopError::EmptyPlan));
        let long = "x".repeat(MAX_TOOL_TEXT_LEN + 1);
        let mut planner = ScriptedPlanner::new(Vec::new());
        let mut door = MemoryDoor::new(Vec::new());
        match AgentLoop::new(LoopBudget::default_budget()).run(&long, &mut planner, &mut door) {
            Err(LoopError::GoalTooLong { len }) => assert_eq!(len, long.len()),
            other => panic!("over-long goal must refuse, got {other:?}"),
        }
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(LoopBudget::new(0, 0).max_steps, 1);
        assert_eq!(LoopBudget::new(u32::MAX, u32::MAX).max_steps, MAX_STEPS);
        assert_eq!(BudgetKind::Tokens.to_string(), "tokens");
        let _: &dyn std::error::Error = &LoopError::EmptyPlan;
    }

    fn read_plans(count: usize) -> Vec<Plan> {
        let mut plans: Vec<Plan> = (0..count)
            .map(|index| Plan::ToolCall {
                tool: String::from("scene.read"),
                input: format!("entity {index}"),
                idempotency_key: u64::try_from(index).unwrap_or(0).saturating_add(1),
            })
            .collect();
        plans.push(Plan::Done {
            answer: String::from("read all"),
        });
        plans
    }

    fn read_door() -> MemoryDoor {
        MemoryDoor::new(vec![(
            String::from("scene.read"),
            String::from("entity at 10,20"),
        )])
    }

    fn write_door() -> MemoryDoor {
        MemoryDoor::with_writes(
            vec![
                (String::from("tx.apply"), String::from("applied wall 3")),
                (String::from("scene.read"), String::from("entity at 10,20")),
            ],
            vec![String::from("tx.apply")],
        )
    }

    /// F2P: the same budget that reports `BudgetOverrun` mid-task under
    /// [`AgentLoop::new`] finishes under [`AgentLoop::compacting`], and the
    /// fold is recorded with its step count and reclaimed tokens.
    #[test]
    fn agent_loop_compaction_finishes_where_the_plain_budget_overruns() {
        let budget = budget_steps(6);
        let mut planner = ScriptedPlanner::new(read_plans(7));
        let mut door = read_door();
        let plain = AgentLoop::new(budget).run("read seven", &mut planner, &mut door);
        assert_eq!(
            plain,
            Err(LoopError::BudgetOverrun {
                kind: BudgetKind::Steps,
                used: 6,
                limit: 6,
            }),
            "the unfolded run stops at the bound"
        );

        let mut planner = ScriptedPlanner::new(read_plans(7));
        let mut door = read_door();
        let folded = AgentLoop::compacting(budget, 3)
            .run("read seven", &mut planner, &mut door)
            .expect("the folded run finishes on the same budget");
        assert_eq!(folded.answer, "read all");
        assert_eq!(folded.folds.len(), 1, "one fold was enough");
        assert_eq!(folded.folds[0].folded, 3);
        assert_eq!(folded.folds[0].kept, 4, "summary plus three kept steps");
        assert!(folded.folds[0].reclaimed > 0, "the fold gave tokens back");
        assert_ne!(folded.folds[0].checkpoint, 0, "the fold has a checkpoint");
        assert_eq!(folded.steps[0].tool, SUMMARY_TOOL);
        assert_eq!(folded.steps[0].step, 0);
        assert!(folded.steps[0].input.contains("folded=3"));
        assert!(folded.steps[0].observation.contains("steps folded"));
        let indices: Vec<u32> = folded.steps.iter().map(|step| step.step).collect();
        assert_eq!(
            indices,
            vec![0, 1, 2, 3, 4],
            "steps are renumbered in order"
        );
        assert!(folded.steps[1..]
            .iter()
            .all(|step| step.tool == "scene.read"));
        assert_eq!(folded.steps.len(), 5, "the loop kept going to the answer");
    }

    /// P2P: with room to spare, compaction changes nothing at all.
    #[test]
    fn agent_loop_compaction_leaves_a_short_run_unchanged() {
        let budget = LoopBudget::default_budget();
        let mut planner = ScriptedPlanner::new(read_plans(3));
        let mut door = read_door();
        let plain = AgentLoop::new(budget)
            .run("read three", &mut planner, &mut door)
            .expect("plain run finishes");

        let mut planner = ScriptedPlanner::new(read_plans(3));
        let mut door = read_door();
        let compacting = AgentLoop::compacting(budget, 2)
            .run("read three", &mut planner, &mut door)
            .expect("compacting run finishes");
        assert!(compacting.folds.is_empty(), "nothing to fold");
        assert_eq!(compacting, plain, "same answer, steps, tokens and folds");
        assert_eq!(compacting.checkpoint(), plain.checkpoint());
    }

    /// A pending write at the head of the transcript stops the fold by name.
    #[test]
    fn agent_loop_compaction_refuses_a_pending_write_at_the_head() {
        let budget = budget_steps(5);
        let mut plans = vec![Plan::ToolCall {
            tool: String::from("tx.apply"),
            input: String::from("wall 3"),
            idempotency_key: 9,
        }];
        plans.extend(read_plans(6));
        let mut planner = ScriptedPlanner::new(plans);
        let mut door = write_door();
        let refused =
            AgentLoop::compacting(budget, 2).run("apply then read", &mut planner, &mut door);
        match refused {
            Err(error) => {
                assert_eq!(error.code(), "cannot_fold");
                assert_eq!(
                    error,
                    LoopError::CannotFold { step: 0, key: 9 },
                    "the oldest step is the pending write"
                );
                assert!(error.to_string().contains("pending write"), "{error}");
            }
            Ok(outcome) => panic!("a pending write must not fold, got {outcome:?}"),
        }
    }

    /// A write inside the fold window stays verbatim with its undo key.
    #[test]
    fn agent_loop_compaction_keeps_a_write_and_its_undo_verbatim() {
        let budget = budget_steps(5);
        let plans = vec![
            Plan::ToolCall {
                tool: String::from("scene.read"),
                input: String::from("entity a"),
                idempotency_key: 1,
            },
            Plan::ToolCall {
                tool: String::from("scene.read"),
                input: String::from("entity b"),
                idempotency_key: 2,
            },
            Plan::ToolCall {
                tool: String::from("tx.apply"),
                input: String::from("wall 3"),
                idempotency_key: 7,
            },
            Plan::ToolCall {
                tool: String::from("scene.read"),
                input: String::from("entity c"),
                idempotency_key: 4,
            },
            Plan::ToolCall {
                tool: String::from("scene.read"),
                input: String::from("entity d"),
                idempotency_key: 5,
            },
            Plan::Done {
                answer: String::from("read all"),
            },
        ];
        let mut planner = ScriptedPlanner::new(plans);
        let mut door = write_door();
        let outcome = AgentLoop::compacting(budget, 2)
            .run("read, write, read", &mut planner, &mut door)
            .expect("the fold stops before the write and the loop finishes");
        assert_eq!(outcome.folds[0].folded, 2, "only the two reads folded");
        let apply = outcome
            .steps
            .iter()
            .find(|step| step.tool == "tx.apply")
            .expect("the write survives the fold");
        assert_eq!(apply.idempotency_key, 7);
        assert_eq!(apply.input, "wall 3");
        assert!(
            door.pending_write("tx.apply", 7),
            "the write is still undoable"
        );
        assert!(door.undo(7), "and undo still finds it");
        assert!(!door.pending_write("tx.apply", 7));
    }
}
