//! mind: NPC mind contract — memory stream, goals, relations, async asks.
//!
//! Consumer: the memory store aggregates per-NPC [`MindContext`] snapshots,
//! ludus NPCs pull tick-stamped output with [`ActionLog::drain_due`], and
//! the AB brief chain feeds [`Mind::recall_into`] text. Data owner: [`Mind`]
//! owns one NPC's stream, goals and relations; movement brains keep their own
//! state and never see provider output. Units: ticks are fixed steps,
//! affinity `-1000..=1000`, salience/priority/trust `0..=1000`, budgets in
//! simulated ms. Determinism: integer math and stable [`Vec`] order only,
//! FNV-1a checkpoints, simulated latency never wall clock. Errors:
//! [`MindError`]; invalid input never panics. Budget: the stream holds at
//! most [`MAX_EVENTS`] events; recall fills a caller buffer and allocates
//! nothing; provider work runs off-tick and only recorded actions apply.
//!
//! Why a second contract: `engine-provider` owns the serving contract the
//! adapters enforce, while this module owns the NPC mind shape the game
//! ticks. The two must agree on budgets and proposal bounds, so any edit
//! here risks drift against `engine-provider` until a later packet retargets
//! this mind onto that contract; this change attempts no retarget.

use crate::reaction_delay::{delay_ticks, ready_or_last, ObsRing};
use engine_core::Fnv64;

/// Most events one mind keeps; older observations evict oldest-first.
pub const MAX_EVENTS: usize = 512;
/// Most goals one mind tracks at once.
pub const MAX_GOALS: usize = 16;
/// Most relations one mind tracks at once.
pub const MAX_RELATIONS: usize = 64;
/// Longest memory/action text, in characters.
pub const MAX_TEXT: usize = 280;
/// Most actions one provider proposal may carry.
pub const MAX_ACTIONS_PER_ASK: usize = 4;
/// Memories offered to an adapter per ask.
pub const CTX_MEMORIES: usize = 4;
/// Goals offered to an adapter per ask.
pub const CTX_GOALS: usize = 2;
/// Simulated success latency per backend, in ms (mock < local < remote).
pub const MOCK_LATENCY_MS: u64 = 1;
/// Simulated success latency per backend, in ms (mock < local < remote).
pub const LOCAL_LATENCY_MS: u64 = 20;
/// Simulated success latency per backend, in ms (mock < local < remote).
pub const REMOTE_LATENCY_MS: u64 = 150;
/// Simulated remote cost in minor units; mock/local cost stays unknown.
pub const REMOTE_COST: u64 = 7;

/// NPC identity. Stable `u32` handle owned by the spawner, never reused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NpcId(pub u32);

/// What kind of memory an event is. Fixed set so recall stays comparable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryKind {
    /// Witnessed something in the world.
    Seen,
    /// Was told something.
    Heard,
    /// Did something itself.
    Did,
    /// Felt something (mood, morale shift).
    Felt,
}

/// One memory-stream event. Tick-stamped at observation; text is bounded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MemoryEvent {
    /// Fixed tick the event was observed at.
    pub tick: u64,
    /// Mind that owns this memory.
    pub npc: NpcId,
    /// What kind of memory this is.
    pub kind: MemoryKind,
    /// Bounded recollection text (`1..=MAX_TEXT` chars).
    pub text: String,
    /// Importance `0..=1000`; drives recall ranking.
    pub salience: u32,
}

/// Goal lifecycle state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoalState {
    /// Pursued now.
    Active,
    /// Achieved; kept for reflection, never re-offered.
    Done,
    /// Abandoned; kept for reflection, never re-offered.
    Dropped,
}

/// One NPC goal. Only [`GoalState::Active`] goals steer asks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Goal {
    /// Mind-local goal id, monotonic from 1.
    pub id: u64,
    /// Mind that owns this goal.
    pub npc: NpcId,
    /// Bounded label (`1..=MAX_TEXT` chars).
    pub label: String,
    /// Urgency `0..=1000`.
    pub priority: u32,
    /// Lifecycle state.
    pub state: GoalState,
}

/// One directed relationship: how `npc` regards `other`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Relation {
    /// Mind that holds this view.
    pub npc: NpcId,
    /// The other NPC this view concerns.
    pub other: NpcId,
    /// Regard `-1000..=1000` (hostile..warm).
    pub affinity: i32,
    /// Reliance `0..=1000`.
    pub trust: u32,
}

/// Privacy bound on one ask. Local-only never becomes remote silently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Privacy {
    /// No provider call; scripted fallback only.
    Offline,
    /// Mock/local adapters only.
    LocalOnly,
    /// Any adapter, including remote.
    RemoteAllowed,
}

/// One async mind ask: identity, tick clock, deadline and privacy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MindRequest {
    /// Ask identity; repeats replay the recorded answer, never re-ask.
    pub id: u64,
    /// Mind being asked.
    pub npc: NpcId,
    /// Fixed tick the ask was issued at.
    pub issued_tick: u64,
    /// Fixed tick by which stamped actions must land (`>= issued_tick`).
    pub deadline_tick: u64,
    /// Simulated-ms budget; slower answers time out into fallback.
    pub budget_ms: u64,
    /// World revision the ask was built against; moved revisions go stale.
    pub base_revision: u64,
    /// Privacy bound for this ask.
    pub privacy: Privacy,
}

/// What a recorded action does. Presentation and movement both read this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind {
    /// Speak a line (presentation only).
    Say,
    /// Play a gesture (presentation only).
    Gesture,
    /// Move toward a named place or NPC.
    Approach,
    /// Withdraw from the scene.
    Leave,
    /// Hold position, stay quiet.
    Idle,
}

/// Where a recorded action came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionSource {
    /// Deterministic scripted table (fallback or offline).
    Scripted,
    /// Validated provider proposal.
    Provider,
}

/// One accepted, tick-stamped output. Only these enter the tick, never raw
/// provider text: drafts are validated, stamped and recorded first.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MindAction {
    /// Fixed tick this action applies at (`issued..=deadline` of its ask).
    pub tick_apply: u64,
    /// Mind this action belongs to.
    pub npc: NpcId,
    /// What the action does.
    pub kind: ActionKind,
    /// Bounded line or label (`1..=MAX_TEXT` chars).
    pub text: String,
    /// Scripted fallback or validated provider output.
    pub source: ActionSource,
    /// Ask that produced this action.
    pub request: u64,
}

/// Typed mind failures. Provider-side failures map to scripted fallback;
/// request-side failures record nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MindError {
    /// Text empty or whitespace-only.
    EmptyText,
    /// Text longer than [`MAX_TEXT`] characters.
    TextTooLong,
    /// Salience above 1000.
    BadSalience,
    /// Priority above 1000.
    BadPriority,
    /// Affinity outside `-1000..=1000`.
    BadAffinity,
    /// Trust above 1000.
    BadTrust,
    /// Ask names a different mind.
    UnknownNpc,
    /// Deadline before the issue tick.
    PastDeadline,
    /// World moved under the ask.
    StaleContext,
    /// Ask cancelled before it ran.
    Cancelled,
    /// Simulated latency blew the ms budget.
    Timeout,
    /// Provider output failed validation.
    Malformed,
    /// Adapter saturated or unreachable.
    Unavailable,
    /// Capability or privacy the adapter cannot serve.
    UnsupportedCapability,
    /// Spend or quota refusal.
    Quota,
    /// Providers disabled; scripted only.
    Offline,
    /// Nothing recorded yet to replay from.
    NothingToReflect,
    /// Proposal carried no usable action or too many.
    TooManyActions,
}

impl core::fmt::Display for MindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            MindError::EmptyText => "mind: empty text",
            MindError::TextTooLong => "mind: text over cap",
            MindError::BadSalience => "mind: salience out of range",
            MindError::BadPriority => "mind: priority out of range",
            MindError::BadAffinity => "mind: affinity out of range",
            MindError::BadTrust => "mind: trust out of range",
            MindError::UnknownNpc => "mind: ask names another npc",
            MindError::PastDeadline => "mind: deadline before issue",
            MindError::StaleContext => "mind: stale revision",
            MindError::Cancelled => "mind: cancelled",
            MindError::Timeout => "mind: deadline exceeded",
            MindError::Malformed => "mind: malformed proposal",
            MindError::Unavailable => "mind: adapter unavailable",
            MindError::UnsupportedCapability => "mind: unsupported capability",
            MindError::Quota => "mind: quota refused",
            MindError::Offline => "mind: providers offline",
            MindError::NothingToReflect => "mind: stream empty",
            MindError::TooManyActions => "mind: bad action count",
        };
        f.write_str(label)
    }
}

impl std::error::Error for MindError {}

/// Bounds every stored or applied text: non-blank, at most [`MAX_TEXT`] chars.
pub(crate) fn check_text(text: &str) -> Result<(), MindError> {
    if text.trim().is_empty() {
        return Err(MindError::EmptyText);
    }
    if text.chars().count() > MAX_TEXT {
        return Err(MindError::TextTooLong);
    }
    Ok(())
}

/// One NPC mind: memory stream, goals and relations for a single [`NpcId`].
#[derive(Clone, Debug)]
pub struct Mind {
    npc: NpcId,
    seed: u64,
    events: Vec<MemoryEvent>,
    goals: Vec<Goal>,
    relations: Vec<Relation>,
    next_goal: u64,
    reaction: ObsRing<u32, 32>,
}

impl Mind {
    /// Empty mind for `npc` with a fallback seed for scripted lines.
    pub fn new(npc: NpcId, seed: u64) -> Self {
        Self {
            npc,
            seed,
            events: Vec::new(),
            goals: Vec::new(),
            relations: Vec::new(),
            next_goal: 1,
            reaction: ObsRing::new(),
        }
    }

    /// Owner id.
    pub const fn npc(&self) -> NpcId {
        self.npc
    }

    /// Events held.
    pub fn events(&self) -> &[MemoryEvent] {
        &self.events
    }

    /// Goals held, including done/dropped history.
    pub fn goals(&self) -> &[Goal] {
        &self.goals
    }

    /// Relations held.
    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    /// Observe one event. Validates bounds, then pushes; when the stream is
    /// full the oldest entry evicts so memory stays bounded and ordered.
    pub fn observe(
        &mut self,
        tick: u64,
        kind: MemoryKind,
        text: &str,
        salience: u32,
    ) -> Result<(), MindError> {
        check_text(text)?;
        if salience > 1000 {
            return Err(MindError::BadSalience);
        }
        if self.events.len() >= MAX_EVENTS {
            self.events.remove(0);
        }
        self.events.push(MemoryEvent {
            tick,
            npc: self.npc,
            kind,
            text: text.to_owned(),
            salience,
        });
        // Feed the reaction ring one sample per tick for the delayed view.
        self.reaction.push(salience);
        Ok(())
    }

    /// Recall ranking score: salience weighted by recency, integer only.
    /// `salience * 1024 / (1 + age)`; older ticks saturate, never wrap.
    fn score(salience: u32, now_tick: u64, at_tick: u64) -> u64 {
        let age = now_tick.saturating_sub(at_tick);
        let decay = age.saturating_add(1);
        u64::from(salience).saturating_mul(1024) / decay.max(1)
    }

    /// Fill `out` with indices of the top-ranked memories in rank order and
    /// return how many were written. Strict `>` keeps the earlier event on
    /// ties, so the same stream always recalls the same order. Writes into a
    /// caller buffer and allocates nothing.
    pub fn recall_into(&self, now_tick: u64, out: &mut [usize]) -> usize {
        let mut picked = 0usize;
        let slots = out.len().min(self.events.len());
        // Selection pass per slot over the not-yet-picked set; O(n*k) with
        // n <= MAX_EVENTS and tiny k, and no allocation on the recall path.
        let mut used = [false; MAX_EVENTS];
        for slot in out.iter_mut().take(slots) {
            let mut best: Option<usize> = None;
            let mut best_score = 0u64;
            for (index, event) in self.events.iter().enumerate() {
                if used[index] {
                    continue;
                }
                let score = Self::score(event.salience, now_tick, event.tick);
                let wins = match best {
                    None => true,
                    Some(_) => score > best_score,
                };
                if wins {
                    best = Some(index);
                    best_score = score;
                }
            }
            match best {
                Some(index) => {
                    used[index] = true;
                    *slot = index;
                    picked += 1;
                }
                None => break,
            }
        }
        picked
    }

    /// Delayed reflection into `Felt` at `tick`: mood from the ring (mid
    /// skill, 10 back; not ready repeats top; same observes give same `Felt`).
    /// Needs no provider; empty streams report [`MindError::NothingToReflect`].
    pub fn reflect(&mut self, tick: u64) -> Result<(), MindError> {
        if self.events.is_empty() {
            return Err(MindError::NothingToReflect);
        }
        let mut slot = [0usize; 1];
        let found = self.recall_into(tick, &mut slot);
        if found == 0 {
            return Err(MindError::NothingToReflect);
        }
        let top_text = self.events[slot[0]].text.clone();
        let top_salience = self.events[slot[0]].salience;
        let delayed = ready_or_last(&self.reaction, delay_ticks(500), top_salience);
        let mood = if delayed >= 700 {
            "restless"
        } else if delayed >= 400 {
            "watchful"
        } else {
            "calm"
        };
        let count = self.events.len();
        let text = format!("Feels {mood} after {count} memories; foremost: {top_text}");
        // Reflection text embeds a bounded memory line, so it always fits:
        // both halves are bounded and the template is short. Truncate defensively.
        let bounded: String = text.chars().take(MAX_TEXT).collect();
        self.observe(tick, MemoryKind::Felt, &bounded, delayed.min(1000))
    }

    /// Add a goal; returns its mind-local id. History (done/dropped) counts
    /// toward [`MAX_GOALS`] so reflection stays bounded too.
    pub fn add_goal(&mut self, label: &str, priority: u32) -> Result<u64, MindError> {
        check_text(label)?;
        if priority > 1000 {
            return Err(MindError::BadPriority);
        }
        if self.goals.len() >= MAX_GOALS {
            // Evict the oldest finished goal first; an all-active set refuses.
            if let Some(pos) = self.goals.iter().position(|g| g.state != GoalState::Active) {
                self.goals.remove(pos);
            } else {
                return Err(MindError::TooManyActions);
            }
        }
        let id = self.next_goal;
        self.next_goal = self.next_goal.saturating_add(1).max(1);
        self.goals.push(Goal {
            id,
            npc: self.npc,
            label: label.to_owned(),
            priority,
            state: GoalState::Active,
        });
        Ok(id)
    }

    /// Drop a goal; unknown ids report [`MindError::UnknownNpc`].
    pub fn drop_goal(&mut self, id: u64) -> Result<(), MindError> {
        match self.goals.iter_mut().find(|g| g.id == id) {
            Some(goal) => {
                goal.state = GoalState::Dropped;
                Ok(())
            }
            None => Err(MindError::UnknownNpc),
        }
    }

    /// Upsert how this mind regards `other`. Self-regard is refused.
    pub fn relate(&mut self, other: NpcId, affinity: i32, trust: u32) -> Result<(), MindError> {
        if other == self.npc {
            return Err(MindError::UnknownNpc);
        }
        if !(-1000..=1000).contains(&affinity) {
            return Err(MindError::BadAffinity);
        }
        if trust > 1000 {
            return Err(MindError::BadTrust);
        }
        if let Some(rel) = self.relations.iter_mut().find(|r| r.other == other) {
            rel.affinity = affinity;
            rel.trust = trust;
            return Ok(());
        }
        if self.relations.len() >= MAX_RELATIONS {
            return Err(MindError::TooManyActions);
        }
        self.relations.push(Relation {
            npc: self.npc,
            other,
            affinity,
            trust,
        });
        Ok(())
    }

    /// Regard for `other`; strangers read neutral 0.
    pub fn affinity(&self, other: NpcId) -> i32 {
        self.relations
            .iter()
            .find(|r| r.other == other)
            .map_or(0, |r| r.affinity)
    }

    /// Bounded snapshot for one ask: top memories plus top active goals.
    /// Stable order (recall rank, then goal priority) so the same mind plus
    /// the same tick always builds the same context.
    pub fn snapshot(&self, tick: u64) -> MindContext {
        let mut slots = [0usize; CTX_MEMORIES];
        let found = self.recall_into(tick, &mut slots);
        let mut memories = Vec::with_capacity(found);
        for slot in slots.iter().take(found) {
            memories.push(self.events[*slot].text.clone());
        }
        let mut active: Vec<&Goal> = self
            .goals
            .iter()
            .filter(|g| g.state == GoalState::Active)
            .collect();
        // Stable insertion-style order by priority desc, id asc on ties.
        active.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
        let goals = active
            .iter()
            .take(CTX_GOALS)
            .map(|g| g.label.clone())
            .collect();
        MindContext {
            npc: self.npc,
            tick,
            memories,
            goals,
        }
    }

    /// Deterministic scripted fallback action for `req`. Table-picked by
    /// issue tick plus seed, stamped at the issue tick, always [`ActionKind::Idle`]
    /// or [`ActionKind::Say`] so it never moves the body on its own.
    pub fn scripted_action(&self, req: &MindRequest) -> MindAction {
        const TABLE: &[&str] = &[
            "watches the sand in silence.",
            "murmurs an old arena proverb.",
            "nods to no one in particular.",
            "studies the crowd, saying nothing.",
        ];
        let pick = TABLE[(req.issued_tick.wrapping_add(self.seed) as usize) % TABLE.len()];
        MindAction {
            tick_apply: req.issued_tick,
            npc: self.npc,
            kind: ActionKind::Say,
            text: format!("npc{} {pick}", self.npc.0),
            source: ActionSource::Scripted,
            request: req.id,
        }
    }
}

/// Bounded ask context handed to adapters: top memory lines plus active
/// goal labels. Plain data; adapters propose, the runtime validates.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MindContext {
    /// Mind being asked.
    pub npc: NpcId,
    /// Tick the snapshot was taken at.
    pub tick: u64,
    /// Top memory lines, recall rank order (`<= CTX_MEMORIES`).
    pub memories: Vec<String>,
    /// Top active goal labels, priority order (`<= CTX_GOALS`).
    pub goals: Vec<String>,
}

/// One provider-proposed action draft, pre-stamp. The runtime validates text
/// and delay before stamping `issued + delay` onto it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActionDraft {
    /// What the action does.
    pub kind: ActionKind,
    /// Bounded line or label.
    pub text: String,
    /// Ticks after issue this action lands; must fit inside the deadline.
    pub delay_ticks: u64,
}

/// Validated provider proposal for one ask.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Proposal {
    /// Ask this proposal answers.
    pub request: u64,
    /// One to [`MAX_ACTIONS_PER_ASK`] drafts.
    pub drafts: Vec<ActionDraft>,
    /// Provider note (diagnostics, bounded).
    pub note: String,
}

/// Adapter answer envelope: result plus simulated latency/cost rows.
/// Unknown cost stays unknown (`None`), never zero-claimed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AdapterOutcome {
    /// Proposal or typed failure.
    pub result: Result<Proposal, MindError>,
    /// Simulated latency in ms; over budget reads as timeout.
    pub latency_ms: u64,
    /// Simulated cost in minor units, or `None` when unknown.
    pub cost: Option<u64>,
    /// True only when a remote endpoint served this ask.
    pub dispatched_remote: bool,
    /// True when a post-cancel answer was dropped before dispatch.
    pub discarded: bool,
}

/// Late-delivery report. Delivery never mutates the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LateOutcome {
    /// True when the answer was dropped (cancelled, done or unknown job).
    pub discarded: bool,
    /// Always false: offering output is never a world write.
    pub mutated: bool,
}

/// Scripted backend behavior. Every failure mode the conformance suite
/// covers has an arm; guards (cancel, privacy, load) run before the script.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScriptStep {
    /// Complete a canned proposal.
    Succeed,
    /// Simulated latency blows the ms budget.
    Timeout {
        /// Pretend elapsed ms for the attempt.
        simulated_ms: u64,
    },
    /// Output fails draft validation.
    Malformed,
    /// Job cancelled mid-ask.
    Cancelled,
    /// Base revision moved under the ask.
    Stale,
    /// Capability the adapter does not offer.
    Unsupported,
    /// Spend cap or quota refusal.
    Quota,
    /// In-flight load at the concurrency cap.
    Overflow,
    /// Answer arriving after cancel: discarded, never applied.
    Late,
}

/// Adapter contract every backend honors: propose under a script, track
/// cancel/revision/load guards, count calls and mutations.
pub trait MindAdapter {
    /// Backend label for latency/cost rows.
    fn name(&self) -> &'static str;
    /// Simulated success latency in ms.
    fn success_latency_ms(&self) -> u64;
    /// Simulated success cost, or `None` when unknown.
    fn success_cost(&self) -> Option<u64>;
    /// Ask the backend for one proposal.
    fn call(&mut self, req: &MindRequest, ctx: &MindContext) -> AdapterOutcome;
    /// Cancel an ask id.
    fn cancel(&mut self, id: u64);
    /// True when `id` was cancelled.
    fn is_cancelled(&self, id: u64) -> bool;
    /// Track a job as running so late delivery has a live job.
    fn begin_job(&mut self, id: u64);
    /// Offer one answer after the fact; only running jobs accept.
    fn deliver_late(&self, id: u64) -> LateOutcome;
    /// Pin the live world revision for staleness guards.
    fn set_revision(&mut self, revision: u64);
    /// Pretend `count` asks are already in flight.
    fn set_in_flight(&mut self, count: u32);
    /// Script arms not yet consumed.
    fn script_remaining(&self) -> usize;
    /// Asks that newly produced a proposal.
    fn mutations(&self) -> u64;
    /// Total calls served (success plus refusals).
    fn calls(&self) -> u64;
    /// Remote endpoint exchanges completed.
    fn remote_calls(&self) -> u32;
}

/// Backend selector.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// Deterministic mock for CI and tests.
    Mock,
    /// Local sidecar loopback.
    Local,
    /// Remote endpoint (explicit privacy only).
    Remote,
}

impl Backend {
    const fn label(self) -> &'static str {
        match self {
            Backend::Mock => "mock",
            Backend::Local => "local",
            Backend::Remote => "remote",
        }
    }

    const fn latency_ms(self) -> u64 {
        match self {
            Backend::Mock => MOCK_LATENCY_MS,
            Backend::Local => LOCAL_LATENCY_MS,
            Backend::Remote => REMOTE_LATENCY_MS,
        }
    }

    const fn cost(self) -> Option<u64> {
        match self {
            Backend::Mock | Backend::Local => None,
            Backend::Remote => Some(REMOTE_COST),
        }
    }
}

/// Shared script/playback core behind the three backend adapters. State
/// (script, cancels, jobs, revision, load, counters) is explicit, so the
/// same script plus the same ask sequence always yields the same outcomes.
#[derive(Clone, Debug)]
struct AdapterCore {
    backend: Backend,
    script: Vec<ScriptStep>,
    cursor: usize,
    cancelled: Vec<u64>,
    jobs: Vec<(u64, bool)>,
    revision: Option<u64>,
    in_flight: u32,
    max_concurrency: u32,
    mutations: u64,
    calls: u64,
    remote_calls: u32,
}

impl AdapterCore {
    fn new(backend: Backend, script: Vec<ScriptStep>) -> Self {
        Self {
            backend,
            script,
            cursor: 0,
            cancelled: Vec::new(),
            jobs: Vec::new(),
            revision: None,
            in_flight: 0,
            max_concurrency: 4,
            mutations: 0,
            calls: 0,
            remote_calls: 0,
        }
    }

    fn next_step(&mut self) -> ScriptStep {
        if self.cursor < self.script.len() {
            let step = self.script[self.cursor].clone();
            self.cursor += 1;
            step
        } else {
            ScriptStep::Succeed
        }
    }

    fn fail(&mut self, latency_ms: u64, error: MindError, remote: bool) -> AdapterOutcome {
        self.calls += 1;
        AdapterOutcome {
            result: Err(error),
            latency_ms,
            cost: None,
            dispatched_remote: remote,
            discarded: false,
        }
    }

    fn succeed(&mut self, req: &MindRequest, ctx: &MindContext) -> AdapterOutcome {
        let latency_ms = self.backend.latency_ms();
        // Canned proposal names its backend and echoes the top memory line,
        // so rows stay distinguishable without any network.
        let line = match ctx.memories.first() {
            Some(top) => {
                let short: String = top.chars().take(64).collect();
                format!(
                    "{}:ok npc{} recalls {short}",
                    self.backend.label(),
                    req.npc.0
                )
            }
            None => format!("{}:ok npc{} idles", self.backend.label(), req.npc.0),
        };
        let proposal = Proposal {
            request: req.id,
            drafts: vec![ActionDraft {
                kind: ActionKind::Say,
                text: line,
                delay_ticks: 0,
            }],
            note: format!("{}:receipt rev{}", self.backend.label(), req.base_revision),
        };
        if validate_proposal(&proposal).is_err() {
            return self.fail(latency_ms, MindError::Malformed, false);
        }
        self.calls += 1;
        self.mutations += 1;
        let remote = self.backend == Backend::Remote;
        if remote {
            self.remote_calls += 1;
        }
        AdapterOutcome {
            result: Ok(proposal),
            latency_ms,
            cost: self.backend.cost(),
            dispatched_remote: remote,
            discarded: false,
        }
    }

    fn call(&mut self, req: &MindRequest, ctx: &MindContext) -> AdapterOutcome {
        let latency = self.backend.latency_ms();
        if self.cancelled.contains(&req.id) {
            return self.fail(latency, MindError::Cancelled, false);
        }
        if let Some(current) = self.revision {
            if req.base_revision != current {
                return self.fail(latency, MindError::StaleContext, false);
            }
        }
        if self.in_flight >= self.max_concurrency.max(1) {
            return self.fail(latency, MindError::Unavailable, false);
        }
        if self.backend == Backend::Remote && req.privacy != Privacy::RemoteAllowed {
            return self.fail(latency, MindError::UnsupportedCapability, false);
        }
        match self.next_step() {
            ScriptStep::Succeed => self.succeed(req, ctx),
            ScriptStep::Timeout { simulated_ms } => {
                self.fail(simulated_ms, MindError::Timeout, false)
            }
            ScriptStep::Malformed => self.fail(latency, MindError::Malformed, false),
            ScriptStep::Cancelled => {
                if !self.cancelled.contains(&req.id) {
                    self.cancelled.push(req.id);
                }
                self.fail(latency, MindError::Cancelled, false)
            }
            ScriptStep::Stale => self.fail(latency, MindError::StaleContext, false),
            ScriptStep::Unsupported => self.fail(latency, MindError::UnsupportedCapability, false),
            ScriptStep::Quota => self.fail(latency, MindError::Quota, false),
            ScriptStep::Overflow => self.fail(latency, MindError::Unavailable, false),
            ScriptStep::Late => {
                self.calls += 1;
                AdapterOutcome {
                    result: Err(MindError::Cancelled),
                    latency_ms: latency,
                    cost: None,
                    dispatched_remote: false,
                    discarded: true,
                }
            }
        }
    }
}

/// Validate a full proposal the way the consume path must before stamping:
/// one to [`MAX_ACTIONS_PER_ASK`] drafts, bounded text, bounded note.
pub fn validate_proposal(proposal: &Proposal) -> Result<(), MindError> {
    if proposal.drafts.is_empty() || proposal.drafts.len() > MAX_ACTIONS_PER_ASK {
        return Err(MindError::TooManyActions);
    }
    for draft in &proposal.drafts {
        check_text(&draft.text)?;
    }
    if proposal.note.chars().count() > MAX_TEXT {
        return Err(MindError::TextTooLong);
    }
    Ok(())
}

macro_rules! backend_adapter {
    ($name:ident, $backend:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Clone, Debug)]
        pub struct $name {
            core: AdapterCore,
        }

        impl $name {
            /// Play `script` in order; asks past the end succeed.
            pub fn new(script: Vec<ScriptStep>) -> Self {
                Self {
                    core: AdapterCore::new($backend, script),
                }
            }
        }

        impl MindAdapter for $name {
            fn name(&self) -> &'static str {
                self.core.backend.label()
            }
            fn success_latency_ms(&self) -> u64 {
                self.core.backend.latency_ms()
            }
            fn success_cost(&self) -> Option<u64> {
                self.core.backend.cost()
            }
            fn call(&mut self, req: &MindRequest, ctx: &MindContext) -> AdapterOutcome {
                self.core.call(req, ctx)
            }
            fn cancel(&mut self, id: u64) {
                if !self.core.cancelled.contains(&id) {
                    self.core.cancelled.push(id);
                }
            }
            fn is_cancelled(&self, id: u64) -> bool {
                self.core.cancelled.contains(&id)
            }
            fn begin_job(&mut self, id: u64) {
                if !self.core.jobs.iter().any(|(job, _)| *job == id) {
                    self.core.jobs.push((id, true));
                }
            }
            fn deliver_late(&self, id: u64) -> LateOutcome {
                match self.core.jobs.iter().find(|(job, _)| *job == id) {
                    Some((_, true)) if !self.core.cancelled.contains(&id) => LateOutcome {
                        discarded: false,
                        mutated: false,
                    },
                    _ => LateOutcome {
                        discarded: true,
                        mutated: false,
                    },
                }
            }
            fn set_revision(&mut self, revision: u64) {
                self.core.revision = Some(revision);
            }
            fn set_in_flight(&mut self, count: u32) {
                self.core.in_flight = count;
            }
            fn script_remaining(&self) -> usize {
                self.core.script.len().saturating_sub(self.core.cursor)
            }
            fn mutations(&self) -> u64 {
                self.core.mutations
            }
            fn calls(&self) -> u64 {
                self.core.calls
            }
            fn remote_calls(&self) -> u32 {
                self.core.remote_calls
            }
        }
    };
}

backend_adapter!(
    MockAdapter,
    Backend::Mock,
    "Deterministic mock backend: canned proposals, 1 ms latency, unknown cost."
);
backend_adapter!(
    LocalAdapter,
    Backend::Local,
    "Local sidecar backend: canned loopback proposals, 20 ms latency, unknown cost."
);
backend_adapter!(
    RemoteAdapter,
    Backend::Remote,
    "Remote endpoint backend: needs `Privacy::RemoteAllowed`, 150 ms latency, metered cost."
);

/// How one ask settled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AskStatus {
    /// Validated provider proposal recorded.
    Provider,
    /// Provider failed with this cause; scripted fallback recorded instead.
    Fallback(MindError),
    /// Ask id already served; recorded answer replayed, no new call.
    Duplicate,
}

/// Settled ask: recorded actions plus how they settled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AskOutcome {
    /// Actions recorded for this ask (acceptance order).
    pub actions: Vec<MindAction>,
    /// How the ask settled.
    pub status: AskStatus,
}

/// Recorded accepted actions. The tick applies these; replay iterates them
/// verbatim and never calls a provider.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ActionLog {
    entries: Vec<MindAction>,
}

impl ActionLog {
    /// Empty log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record one accepted action. Rejects actions with empty or over-long
    /// text; tick order is the caller's contract, acceptance order is kept.
    pub fn push(&mut self, action: MindAction) -> Result<(), MindError> {
        check_text(&action.text)?;
        self.entries.push(action);
        Ok(())
    }

    /// Recorded entries in acceptance order.
    pub fn entries(&self) -> &[MindAction] {
        &self.entries
    }

    /// Replay recorded entries verbatim: no provider call, no new entry.
    pub fn replay(&self) -> impl Iterator<Item = &MindAction> {
        self.entries.iter()
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Actions due at `tick` (stamp `<= tick`), acceptance order.
    pub fn due_at(&self, tick: u64) -> Vec<&MindAction> {
        self.entries
            .iter()
            .filter(|a| a.tick_apply <= tick)
            .collect()
    }

    /// Remove and return actions due at `tick`, acceptance order. Ludus
    /// NPCs drain on the tick boundary; late provider answers for a drained
    /// ask stay recorded history and are never re-applied.
    pub fn drain_due(&mut self, tick: u64) -> Vec<MindAction> {
        let mut due = Vec::new();
        let mut kept = Vec::with_capacity(self.entries.len());
        for action in self.entries.drain(..) {
            if action.tick_apply <= tick {
                due.push(action);
            } else {
                kept.push(action);
            }
        }
        self.entries = kept;
        due
    }

    /// Deterministic FNV-1a checkpoint over stamps, owners, kinds, sources
    /// and text bytes in acceptance order. Same recorded sequence always
    /// yields the same checkpoint; any changed entry changes it.
    pub fn checkpoint(&self) -> u64 {
        let mut hash = Fnv64::new();
        for entry in &self.entries {
            hash.write_u64(entry.tick_apply);
            hash.write_u32(entry.npc.0);
            hash.write_u8(entry.kind as u8);
            hash.write_u8(entry.source as u8);
            hash.write_u64(entry.request);
            hash.write_u64(entry.text.len() as u64);
            hash.write_str(&entry.text);
        }
        hash.finish()
    }
}

/// Per-backend latency/cost row for the conformance report. Unknown cost
/// stays unknown (`None`), never zero-claimed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BackendRow {
    /// Backend label (`mock`, `local`, `remote`).
    pub backend: &'static str,
    /// Simulated success latency in ms.
    pub latency_ms: u64,
    /// Simulated success cost, or `None` when unknown.
    pub cost: Option<u64>,
    /// Asks served while running the suite.
    pub served: u64,
    /// Asks that fell back to the scripted table.
    pub scripted: u64,
}

/// Documented latency/cost rows, one per backend, for the report.
pub fn backend_rows() -> Vec<BackendRow> {
    vec![
        BackendRow {
            backend: "mock",
            latency_ms: MOCK_LATENCY_MS,
            cost: None,
            served: 0,
            scripted: 0,
        },
        BackendRow {
            backend: "local",
            latency_ms: LOCAL_LATENCY_MS,
            cost: None,
            served: 0,
            scripted: 0,
        },
        BackendRow {
            backend: "remote",
            latency_ms: REMOTE_LATENCY_MS,
            cost: Some(REMOTE_COST),
            served: 0,
            scripted: 0,
        },
    ]
}

/// The NPC mind runtime: one ask entry point with deadline plus scripted
/// fallback, validated stamping, duplicate suppression and an offline mode.
/// Models never run in the tick: this service records validated actions and
/// the tick only drains them via [`ActionLog::drain_due`].
#[derive(Clone, Debug)]
pub struct MindRuntime {
    log: ActionLog,
    seen: Vec<(u64, Vec<MindAction>)>,
    cancelled: Vec<u64>,
    revision: Option<u64>,
    online: bool,
    served: u64,
    scripted: u64,
    mutations: u64,
    record_dropped: u64,
}

impl MindRuntime {
    /// Empty runtime, providers on, no revision pin.
    pub fn new() -> Self {
        Self {
            log: ActionLog::new(),
            seen: Vec::new(),
            cancelled: Vec::new(),
            revision: None,
            online: true,
            served: 0,
            scripted: 0,
            mutations: 0,
            record_dropped: 0,
        }
    }

    /// Recorded log.
    pub fn recorded(&self) -> &ActionLog {
        &self.log
    }

    /// Remove and return actions due at `tick`, acceptance order.
    /// Tick-boundary only: the apply door drains through this; drained
    /// actions never re-apply, late answers stay history.
    pub fn drain_due(&mut self, tick: u64) -> Vec<MindAction> {
        self.log.drain_due(tick)
    }

    /// Asks served so far (provider plus fallback plus duplicate).
    pub fn served(&self) -> u64 {
        self.served
    }

    /// Asks that fell back to the scripted table.
    pub fn scripted_count(&self) -> u64 {
        self.scripted
    }

    /// Provider-sourced actions recorded (duplicates never increment this).
    pub fn mutations(&self) -> u64 {
        self.mutations
    }

    /// Recorded actions dropped for failing validation at record time.
    pub fn record_dropped(&self) -> u64 {
        self.record_dropped
    }

    /// Flip provider access. Off means scripted fallback without any adapter
    /// call; replay uses this so checkpoints reproduce with no provider.
    pub fn set_online(&mut self, online: bool) {
        self.online = online;
    }

    /// Providers currently allowed.
    pub fn online(&self) -> bool {
        self.online
    }

    /// Pin the live world revision; asks built elsewhere go stale.
    pub fn set_revision(&mut self, revision: u64) {
        self.revision = Some(revision);
    }

    /// Stop enforcing revision freshness.
    pub fn clear_revision(&mut self) {
        self.revision = None;
    }

    /// Cancel an ask id. Late answers for it are dropped, never applied.
    pub fn cancel(&mut self, id: u64) {
        if !self.cancelled.contains(&id) {
            self.cancelled.push(id);
        }
    }

    fn record(&mut self, id: u64, actions: Vec<MindAction>, provider: bool) {
        for action in actions.iter().cloned() {
            // Recorded actions passed validation before stamping, so a push
            // failure here is unreachable; count it instead of swallowing.
            if self.log.push(action).is_err() {
                self.record_dropped = self.record_dropped.saturating_add(1);
            }
        }
        if provider {
            self.mutations += u64::try_from(actions.len()).unwrap_or(u64::MAX);
        }
        self.seen.push((id, actions));
    }

    fn fallback(&mut self, mind: &Mind, req: &MindRequest, cause: MindError) -> AskOutcome {
        self.served += 1;
        self.scripted += 1;
        let action = mind.scripted_action(req);
        self.record(req.id, vec![action.clone()], false);
        AskOutcome {
            actions: vec![action],
            status: AskStatus::Fallback(cause),
        }
    }

    /// Ask a mind through an adapter with deadline plus scripted fallback.
    /// Request-side failures (wrong NPC, past deadline, stale revision,
    /// armed cancel, duplicate) record nothing new; provider-side failures
    /// record one scripted fallback action. Successful drafts are validated,
    /// stamped inside `[issued, deadline]` and recorded as provider actions.
    pub fn ask<A: MindAdapter>(
        &mut self,
        mind: &Mind,
        req: &MindRequest,
        adapter: &mut A,
    ) -> Result<AskOutcome, MindError> {
        if req.npc != mind.npc() {
            return Err(MindError::UnknownNpc);
        }
        if req.deadline_tick < req.issued_tick {
            return Err(MindError::PastDeadline);
        }
        if let Some(recorded) = self.seen.iter().find(|(id, _)| *id == req.id) {
            self.served += 1;
            return Ok(AskOutcome {
                actions: recorded.1.clone(),
                status: AskStatus::Duplicate,
            });
        }
        if self.cancelled.contains(&req.id) {
            return Err(MindError::Cancelled);
        }
        if let Some(current) = self.revision {
            if req.base_revision != current {
                return Err(MindError::StaleContext);
            }
        }
        if !self.online || req.privacy == Privacy::Offline {
            return Ok(self.fallback(mind, req, MindError::Offline));
        }
        let ctx = mind.snapshot(req.issued_tick);
        let outcome = adapter.call(req, &ctx);
        if outcome.discarded {
            return Ok(self.fallback(mind, req, MindError::Cancelled));
        }
        if outcome.latency_ms > req.budget_ms {
            return Ok(self.fallback(mind, req, MindError::Timeout));
        }
        match outcome.result {
            Ok(proposal) => {
                if proposal.request != req.id {
                    return Ok(self.fallback(mind, req, MindError::Malformed));
                }
                if validate_proposal(&proposal).is_err() {
                    return Ok(self.fallback(mind, req, MindError::Malformed));
                }
                let mut actions = Vec::with_capacity(proposal.drafts.len());
                for draft in &proposal.drafts {
                    let stamp = req.issued_tick.saturating_add(draft.delay_ticks);
                    if stamp > req.deadline_tick {
                        return Ok(self.fallback(mind, req, MindError::Malformed));
                    }
                    actions.push(MindAction {
                        tick_apply: stamp,
                        npc: mind.npc(),
                        kind: draft.kind,
                        text: draft.text.clone(),
                        source: ActionSource::Provider,
                        request: req.id,
                    });
                }
                self.served += 1;
                self.record(req.id, actions.clone(), true);
                Ok(AskOutcome {
                    actions,
                    status: AskStatus::Provider,
                })
            }
            Err(cause) => Ok(self.fallback(mind, req, cause)),
        }
    }

    /// Rebuild a log from recorded entries with no provider: validates each
    /// entry and returns the checkpoint. Providers-off replay reproduces the
    /// live checkpoint exactly.
    pub fn replay_offline(entries: &[MindAction]) -> Result<(ActionLog, u64), MindError> {
        let mut log = ActionLog::new();
        for entry in entries {
            log.push(entry.clone())?;
        }
        let checkpoint = log.checkpoint();
        Ok((log, checkpoint))
    }
}

impl Default for MindRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// One conformance suite every backend adapter must pass: success, timeout,
/// malformed, armed cancel, scripted cancel, stale revision, duplicate
/// replay, unsupported capability, quota, queue overflow, late output,
/// privacy downgrade, offline fallback and deterministic replay. Panics name
/// the failing arm and backend.
pub fn conformance_suite<A: MindAdapter>(
    adapter: &mut A,
    new: &dyn Fn(Vec<ScriptStep>) -> A,
    make_req: &dyn Fn(u64, u64) -> MindRequest,
) {
    let backend = adapter.name();
    let mut mind = Mind::new(NpcId(3), 11);
    mind.observe(1, MemoryKind::Seen, "a stranger enters the ludus", 800)
        .expect("fixture observes");
    mind.observe(2, MemoryKind::Heard, "the crowd chants at dusk", 300)
        .expect("fixture observes");
    mind.add_goal("guard the gate", 700).expect("fixture goal");

    // Success completes with one validated provider action.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let req = make_req(1, 3);
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("success asks");
        assert_eq!(out.status, AskStatus::Provider, "{backend}: success status");
        assert_eq!(out.actions.len(), 1, "{backend}: one action");
        assert_eq!(
            out.actions[0].source,
            ActionSource::Provider,
            "{backend}: source"
        );
        assert_eq!(out.actions[0].request, req.id, "{backend}: request stamp");
        assert!(
            out.actions[0].text.contains(backend),
            "{backend}: receipt tags backend"
        );
        assert_eq!(backend_adapter.mutations(), 1, "{backend}: one mutation");
        assert_eq!(rt.mutations(), 1, "{backend}: runtime counts one");
    }
    // Timeout (simulated ms over budget) falls back scripted.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Timeout {
            simulated_ms: 9_000,
        }]);
        let req = make_req(2, 3);
        assert!(
            9_000 > req.budget_ms,
            "{backend}: fixture must blow the budget it claims"
        );
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("timeout asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Timeout),
            "{backend}: timeout status"
        );
        assert_eq!(
            out.actions[0].source,
            ActionSource::Scripted,
            "{backend}: fallback scripted"
        );
        assert_eq!(rt.mutations(), 0, "{backend}: no mutation on timeout");
    }
    // Malformed output fails validation into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Malformed]);
        let req = make_req(3, 3);
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("malformed asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Malformed),
            "{backend}: malformed status"
        );
        assert!(validate_proposal(&Proposal {
            request: req.id,
            drafts: Vec::new(),
            note: String::new(),
        })
        .is_err());
        let bad = ActionDraft {
            kind: ActionKind::Say,
            text: String::new(),
            delay_ticks: 0,
        };
        assert!(check_text(&bad.text).is_err());
    }
    // Armed cancel refuses before the script runs.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let req = make_req(4, 3);
        rt.cancel(req.id);
        let before = backend_adapter.script_remaining();
        assert_eq!(
            rt.ask(&mind, &req, &mut backend_adapter),
            Err(MindError::Cancelled)
        );
        assert_eq!(
            backend_adapter.script_remaining(),
            before,
            "{backend}: cancel consumes no script"
        );
        assert!(
            rt.recorded().is_empty(),
            "{backend}: cancel records nothing"
        );
    }
    // Scripted cancel reports cancelled into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Cancelled]);
        let out = rt
            .ask(&mind, &make_req(44, 3), &mut backend_adapter)
            .expect("cancel asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Cancelled),
            "{backend}: cancel status"
        );
        assert!(
            backend_adapter.is_cancelled(44),
            "{backend}: cancel tracked"
        );
    }
    // Stale revision on mismatch; fresh revision serves again.
    {
        let mut rt = MindRuntime::new();
        rt.set_revision(10);
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        assert_eq!(
            rt.ask(&mind, &make_req(5, 9), &mut backend_adapter),
            Err(MindError::StaleContext),
            "{backend}: stale refused"
        );
        let mut fresh = new(vec![ScriptStep::Succeed]);
        let out = rt
            .ask(&mind, &make_req(55, 10), &mut fresh)
            .expect("fresh asks");
        assert_eq!(out.status, AskStatus::Provider, "{backend}: fresh serves");
    }
    // Scripted stale arm reports stale into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Stale]);
        let out = rt
            .ask(&mind, &make_req(56, 3), &mut backend_adapter)
            .expect("stale asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::StaleContext),
            "{backend}: stale status"
        );
    }
    // Duplicate replays the recorded answer with no second mutation.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let req = make_req(6, 3);
        let first = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("first asks");
        assert_eq!(first.status, AskStatus::Provider);
        let calls = backend_adapter.calls();
        let second = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("replay asks");
        assert_eq!(
            second.status,
            AskStatus::Duplicate,
            "{backend}: duplicate status"
        );
        assert_eq!(
            second.actions, first.actions,
            "{backend}: duplicate replays answer"
        );
        assert_eq!(backend_adapter.calls(), calls, "{backend}: no second call");
        assert_eq!(
            rt.mutations(),
            1,
            "{backend}: mutation counter stays at one"
        );
    }
    // Unsupported capability falls back without serving.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Unsupported]);
        let out = rt
            .ask(&mind, &make_req(7, 3), &mut backend_adapter)
            .expect("unsupported asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::UnsupportedCapability),
            "{backend}: unsupported status"
        );
        assert_eq!(rt.mutations(), 0);
    }
    // Quota refusal falls back without serving.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Quota]);
        let out = rt
            .ask(&mind, &make_req(8, 3), &mut backend_adapter)
            .expect("quota asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Quota),
            "{backend}: quota status"
        );
    }
    // Queue overflow over max concurrency falls back; draining serves.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        backend_adapter.set_in_flight(4);
        let out = rt
            .ask(&mind, &make_req(9, 3), &mut backend_adapter)
            .expect("overflow asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Unavailable),
            "{backend}: overflow status"
        );
        backend_adapter.set_in_flight(0);
        let served = rt
            .ask(&mind, &make_req(99, 3), &mut backend_adapter)
            .expect("drained asks");
        assert_eq!(
            served.status,
            AskStatus::Provider,
            "{backend}: drained serves"
        );
    }
    // Scripted overflow arm is unavailable.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Overflow]);
        let out = rt
            .ask(&mind, &make_req(98, 3), &mut backend_adapter)
            .expect("overflow asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Unavailable),
            "{backend}: overflow status"
        );
    }
    // Late output after cancel is discarded; unknown jobs too.
    {
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        backend_adapter.begin_job(10);
        backend_adapter.cancel(10);
        let delivery = backend_adapter.deliver_late(10);
        assert!(
            delivery.discarded,
            "{backend}: event after cancel is discarded"
        );
        assert!(!delivery.mutated, "{backend}: delivery never mutates");
        assert!(
            backend_adapter.deliver_late(999).discarded,
            "{backend}: unknown job discarded"
        );
    }
    // Scripted late arm discards into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Late]);
        let out = rt
            .ask(&mind, &make_req(101, 3), &mut backend_adapter)
            .expect("late asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Cancelled),
            "{backend}: late status"
        );
        assert_eq!(out.actions[0].source, ActionSource::Scripted);
    }
    // Privacy downgrade: remote under local-only refuses without dispatch.
    // Mock/local backends ignore this arm (their contract has no remote leg).
    if backend == "remote" {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let mut req = make_req(11, 3);
        req.privacy = Privacy::LocalOnly;
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("downgrade asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::UnsupportedCapability),
            "{backend}: downgrade status"
        );
        assert_eq!(
            backend_adapter.remote_calls(),
            0,
            "{backend}: downgrade never dispatches"
        );
    }
    // Offline runtime never calls the adapter.
    {
        let mut rt = MindRuntime::new();
        rt.set_online(false);
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let calls = backend_adapter.calls();
        let out = rt
            .ask(&mind, &make_req(12, 3), &mut backend_adapter)
            .expect("offline asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Offline),
            "{backend}: offline status"
        );
        assert_eq!(
            backend_adapter.calls(),
            calls,
            "{backend}: offline performs no call"
        );
        assert!(out.actions[0].scripted_or_provider_is_scripted());
    }
    // Same script twice gives the same outcome sequence.
    {
        let script = vec![
            ScriptStep::Succeed,
            ScriptStep::Timeout {
                simulated_ms: 9_000,
            },
            ScriptStep::Malformed,
            ScriptStep::Quota,
        ];
        let run = |base: u64| {
            let mut rt = MindRuntime::new();
            let mut backend_adapter = new(script.clone());
            (20..24)
                .map(|id| {
                    rt.ask(&mind, &make_req(id, base), &mut backend_adapter)
                        .expect("suite asks")
                        .status
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3), "{backend}: deterministic fixtures replay");
    }
    let _ = adapter.calls();
}

impl MindAction {
    fn scripted_or_provider_is_scripted(&self) -> bool {
        self.source == ActionSource::Scripted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mind() -> Mind {
        Mind::new(NpcId(3), 11)
    }

    fn req(id: u64, revision: u64) -> MindRequest {
        MindRequest {
            id,
            npc: NpcId(3),
            issued_tick: 100,
            deadline_tick: 108,
            // Above the slowest backend (remote, 150 ms) so success arms
            // serve everywhere; the timeout arm simulates 9 s against it.
            budget_ms: 500,
            base_revision: revision,
            privacy: Privacy::RemoteAllowed,
        }
    }

    fn seeded_mind() -> Mind {
        let mut npc = mind();
        npc.observe(1, MemoryKind::Seen, "a stranger enters the ludus", 800)
            .expect("seed observes");
        npc.observe(2, MemoryKind::Heard, "the crowd chants at dusk", 300)
            .expect("seed observes");
        npc.observe(3, MemoryKind::Did, "swept the training sand", 100)
            .expect("seed observes");
        npc.add_goal("guard the gate", 700).expect("seed goal");
        npc.relate(NpcId(5), 400, 600).expect("seed relation");
        npc
    }

    #[test]
    fn observe_recall_reflect_roundtrip() {
        let mut npc = seeded_mind();
        // Recall ranks the vivid stranger memory first, then chant, then chore.
        let mut slots = [0usize; 4];
        let found = npc.recall_into(10, &mut slots);
        assert_eq!(found, 3);
        assert_eq!(npc.events()[slots[0]].salience, 800);
        assert_eq!(npc.events()[slots[1]].salience, 300);
        assert_eq!(npc.events()[slots[2]].salience, 100);
        // Reflection appends a deterministic Felt summary of the top memory.
        let before = npc.events().len();
        npc.reflect(11).expect("reflects");
        assert_eq!(npc.events().len(), before + 1);
        let felt = npc.events().last().expect("felt appended");
        assert_eq!(felt.kind, MemoryKind::Felt);
        assert!(felt.text.contains("a stranger enters the ludus"));
        // Same stream twice recalls identically (after reflection too).
        let mut again = [0usize; 4];
        let found_again = npc.recall_into(10, &mut again);
        let mut third = [0usize; 4];
        assert_eq!(npc.recall_into(10, &mut third), found_again);
        assert_eq!(again, third);
        assert_eq!(found_again, found + 1, "reflection appended one memory");
    }

    #[test]
    fn stream_stays_bounded_oldest_evicts() {
        let mut npc = mind();
        for tick in 0..(MAX_EVENTS as u64 + 5) {
            npc.observe(tick, MemoryKind::Seen, "sand shifts", 10)
                .expect("observes");
        }
        assert_eq!(npc.events().len(), MAX_EVENTS);
        assert_eq!(npc.events()[0].tick, 5, "oldest evicted first");
        assert_eq!(npc.events()[MAX_EVENTS - 1].tick, MAX_EVENTS as u64 + 4);
    }

    #[test]
    fn invalid_memories_goals_relations_refused() {
        let mut npc = mind();
        assert_eq!(
            npc.observe(0, MemoryKind::Seen, "   ", 10),
            Err(MindError::EmptyText)
        );
        assert_eq!(
            npc.observe(0, MemoryKind::Seen, &"x".repeat(MAX_TEXT + 1), 10),
            Err(MindError::TextTooLong)
        );
        assert_eq!(
            npc.observe(0, MemoryKind::Seen, "fine", 1001),
            Err(MindError::BadSalience)
        );
        assert_eq!(npc.add_goal("", 10).map(|_| ()), Err(MindError::EmptyText));
        assert_eq!(
            npc.add_goal("fine", 1001).map(|_| ()),
            Err(MindError::BadPriority)
        );
        assert_eq!(npc.relate(NpcId(3), 0, 0), Err(MindError::UnknownNpc));
        assert_eq!(npc.relate(NpcId(4), 1001, 0), Err(MindError::BadAffinity));
        assert_eq!(npc.relate(NpcId(4), -1001, 0), Err(MindError::BadAffinity));
        assert_eq!(npc.relate(NpcId(4), 0, 1001), Err(MindError::BadTrust));
        assert_eq!(
            Mind::new(NpcId(1), 0).reflect(0),
            Err(MindError::NothingToReflect)
        );
        // Valid edges hold: stranger neutral, friend warm, updated in place.
        assert_eq!(npc.affinity(NpcId(9)), 0);
        npc.relate(NpcId(9), -200, 100).expect("relates");
        assert_eq!(npc.affinity(NpcId(9)), -200);
        npc.relate(NpcId(9), 500, 900).expect("updates");
        assert_eq!(npc.relations().len(), 1, "upsert keeps one row");
        assert_eq!(npc.affinity(NpcId(9)), 500);
    }

    #[test]
    fn request_validation_records_nothing() {
        let npc = seeded_mind();
        let mut rt = MindRuntime::new();
        let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
        // Wrong NPC, past deadline and stale revision all refuse cleanly.
        let mut other = req(1, 3);
        other.npc = NpcId(99);
        assert_eq!(rt.ask(&npc, &other, &mut mock), Err(MindError::UnknownNpc));
        let mut past = req(2, 3);
        past.deadline_tick = 99;
        assert_eq!(rt.ask(&npc, &past, &mut mock), Err(MindError::PastDeadline));
        rt.set_revision(10);
        assert_eq!(
            rt.ask(&npc, &req(3, 9), &mut mock),
            Err(MindError::StaleContext)
        );
        assert!(rt.recorded().is_empty(), "refused asks record nothing");
        assert_eq!(mock.calls(), 0, "refused asks never reach the adapter");
    }

    #[test]
    fn conformance_green_mock_local_remote() {
        let mut mock = MockAdapter::new(vec![]);
        conformance_suite(&mut mock, &MockAdapter::new, &req);
        let mut local = LocalAdapter::new(vec![]);
        conformance_suite(&mut local, &LocalAdapter::new, &req);
        let mut remote = RemoteAdapter::new(vec![]);
        conformance_suite(&mut remote, &RemoteAdapter::new, &req);
    }

    #[test]
    fn latency_cost_rows_cover_backends() {
        let rows = backend_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].backend, "mock");
        assert_eq!(rows[1].backend, "local");
        assert_eq!(rows[2].backend, "remote");
        assert!(
            rows[0].latency_ms < rows[1].latency_ms && rows[1].latency_ms < rows[2].latency_ms,
            "mock < local < remote: {rows:?}"
        );
        assert_eq!(rows[0].cost, None, "mock cost stays unknown");
        assert_eq!(rows[1].cost, None, "local cost stays unknown");
        assert_eq!(
            rows[2].cost,
            Some(REMOTE_COST),
            "remote cost metered, never zero"
        );
        // Adapters agree with the documented rows.
        assert_eq!(
            MockAdapter::new(vec![]).success_latency_ms(),
            MOCK_LATENCY_MS
        );
        assert_eq!(
            LocalAdapter::new(vec![]).success_latency_ms(),
            LOCAL_LATENCY_MS
        );
        assert_eq!(
            RemoteAdapter::new(vec![]).success_latency_ms(),
            REMOTE_LATENCY_MS
        );
        assert_eq!(MockAdapter::new(vec![]).success_cost(), None);
        assert_eq!(RemoteAdapter::new(vec![]).success_cost(), Some(REMOTE_COST));
    }

    #[test]
    fn slow_or_cancelled_asks_fall_back_scripted() {
        let npc = seeded_mind();
        // Simulated 9 s against a 500 ms budget: timeout, scripted line, tick kept.
        let mut rt = MindRuntime::new();
        let mut slow = MockAdapter::new(vec![ScriptStep::Timeout {
            simulated_ms: 9_000,
        }]);
        let out = rt.ask(&npc, &req(1, 3), &mut slow).expect("timeout asks");
        assert_eq!(out.status, AskStatus::Fallback(MindError::Timeout));
        assert_eq!(out.actions[0].source, ActionSource::Scripted);
        // Cancelled asks refuse with no record and no script consumed.
        let mut rt = MindRuntime::new();
        let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
        rt.cancel(2);
        assert_eq!(
            rt.ask(&npc, &req(2, 3), &mut mock),
            Err(MindError::Cancelled)
        );
        assert!(rt.recorded().is_empty());
        assert_eq!(mock.script_remaining(), 1);
    }

    #[test]
    fn duplicate_replays_without_second_mutation() {
        let npc = seeded_mind();
        let mut rt = MindRuntime::new();
        let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
        let first = rt.ask(&npc, &req(1, 3), &mut mock).expect("first asks");
        assert_eq!(first.status, AskStatus::Provider);
        let second = rt.ask(&npc, &req(1, 3), &mut mock).expect("replay asks");
        assert_eq!(second.status, AskStatus::Duplicate);
        assert_eq!(second.actions, first.actions);
        assert_eq!(mock.calls(), 1, "duplicate performs no second call");
        assert_eq!(rt.mutations(), 1, "duplicate never re-mutates");
        assert!(rt.recorded().replay().count() == 1);
    }

    #[test]
    fn runtime_drain_due_removes_only_due() {
        let npc = seeded_mind();
        let mut rt = MindRuntime::new();
        rt.set_online(false);
        let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
        let mut first = req(1, 3);
        first.issued_tick = 100;
        first.deadline_tick = 104;
        rt.ask(&npc, &first, &mut mock).expect("first asks");
        let mut second = req(2, 3);
        second.issued_tick = 200;
        second.deadline_tick = 204;
        rt.ask(&npc, &second, &mut mock).expect("second asks");
        assert_eq!(mock.calls(), 0, "offline performs no provider call");
        assert_eq!(rt.recorded().len(), 2);
        // Early drain: nothing due, the log is untouched.
        assert!(rt.drain_due(50).is_empty());
        assert_eq!(rt.recorded().len(), 2, "future stamps stay history");
        // Due drain removes the first line only.
        let due = rt.drain_due(104);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].request, 1);
        assert_eq!(rt.recorded().len(), 1);
        // Same tick again: nothing due, nothing re-applied.
        assert!(rt.drain_due(104).is_empty());
        assert_eq!(rt.recorded().len(), 1);
        // The future's own tick drains it.
        let rest = rt.drain_due(1_000);
        assert_eq!(rest.len(), 1);
        assert!(rt.recorded().is_empty());
    }

    #[test]
    fn providers_off_replay_reproduces_checkpoints_x2() {
        // Live run with providers disabled: scripted fallbacks only.
        let run_offline = || {
            let npc = seeded_mind();
            let mut rt = MindRuntime::new();
            rt.set_online(false);
            let mut mock = MockAdapter::new(vec![ScriptStep::Succeed]);
            for id in 1..=12u64 {
                let mut ask = req(id, 3);
                ask.issued_tick = 100 + id;
                ask.deadline_tick = 100 + id + 4;
                let out = rt.ask(&npc, &ask, &mut mock).expect("offline asks");
                assert_eq!(out.status, AskStatus::Fallback(MindError::Offline));
            }
            assert_eq!(mock.calls(), 0, "offline performs no provider call");
            (rt.recorded().entries().to_vec(), rt.recorded().checkpoint())
        };
        let (first_entries, first_checkpoint) = run_offline();
        let (second_entries, second_checkpoint) = run_offline();
        assert_eq!(
            first_entries, second_entries,
            "offline runs record identically"
        );
        assert_eq!(
            first_checkpoint, second_checkpoint,
            "offline checkpoints match x2"
        );
        // Stored replay with no adapter at all reproduces the checkpoint.
        let (_, replayed) = MindRuntime::replay_offline(&first_entries).expect("replays");
        assert_eq!(replayed, first_checkpoint, "recorded replay matches live");
        let (_, replayed_again) = MindRuntime::replay_offline(&first_entries).expect("replays");
        assert_eq!(
            replayed_again, first_checkpoint,
            "recorded replay matches x2"
        );
        // Divergence hygiene: one changed line changes the checkpoint.
        let mut changed = first_entries;
        changed[4].text.push('!');
        let (_, other) = MindRuntime::replay_offline(&changed).expect("replays");
        assert_ne!(
            other, first_checkpoint,
            "changed input changes the checkpoint"
        );
    }

    #[test]
    fn provider_run_replays_from_record_without_provider() {
        // Mixed provider + fallback tape, then replay with no adapter.
        let npc = seeded_mind();
        let mut rt = MindRuntime::new();
        let mut mock = MockAdapter::new(vec![
            ScriptStep::Succeed,
            ScriptStep::Timeout {
                simulated_ms: 9_000,
            },
            ScriptStep::Succeed,
        ]);
        let mut statuses = Vec::new();
        for id in 1..=3u64 {
            let out = rt.ask(&npc, &req(id, 3), &mut mock).expect("asks");
            statuses.push(out.status);
        }
        assert_eq!(
            statuses,
            vec![
                AskStatus::Provider,
                AskStatus::Fallback(MindError::Timeout),
                AskStatus::Provider
            ]
        );
        let live = rt.recorded().checkpoint();
        let entries = rt.recorded().entries().to_vec();
        let (_, replayed) = MindRuntime::replay_offline(&entries).expect("replays");
        assert_eq!(replayed, live, "mixed tape replays without any provider");
        // Ludus NPCs drain due actions on the tick boundary in acceptance order.
        let log = entries.clone();
        let mut drain_log = ActionLog::new();
        for action in log {
            drain_log.push(action).expect("records");
        }
        let due = drain_log.drain_due(100);
        assert_eq!(due.len(), 3, "all stamps land at the issue tick");
        assert!(drain_log.is_empty(), "drain removes applied actions");
        assert_eq!(due[0].request, 1);
        assert_eq!(due[1].request, 2);
    }

    #[test]
    fn remote_privacy_refusal_never_dispatches() {
        let npc = seeded_mind();
        let mut rt = MindRuntime::new();
        let mut remote = RemoteAdapter::new(vec![ScriptStep::Succeed]);
        let mut ask = req(1, 3);
        ask.privacy = Privacy::LocalOnly;
        let out = rt.ask(&npc, &ask, &mut remote).expect("downgrade asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::UnsupportedCapability)
        );
        assert_eq!(remote.remote_calls(), 0, "refusal records no dispatch");
        assert_eq!(out.actions[0].source, ActionSource::Scripted);
        // Explicit remote privacy serves and dispatches exactly once.
        let mut rt = MindRuntime::new();
        let mut remote = RemoteAdapter::new(vec![ScriptStep::Succeed]);
        let out = rt.ask(&npc, &req(2, 3), &mut remote).expect("remote asks");
        assert_eq!(out.status, AskStatus::Provider);
        assert_eq!(remote.remote_calls(), 1);
    }

    #[test]
    fn scripted_fallback_is_deterministic() {
        let npc = seeded_mind();
        let first = npc.scripted_action(&req(7, 3));
        let second = npc.scripted_action(&req(7, 3));
        assert_eq!(first, second, "same ask gives the same fallback");
        assert_eq!(first.tick_apply, 100);
        assert_eq!(first.source, ActionSource::Scripted);
        assert!(!first.text.is_empty());
    }

    fn delayed_fixture() -> Mind {
        let mut npc = mind();
        for tick in 0..30u64 {
            let sal = if tick < 19 { 800 } else { 100 };
            npc.observe(tick, MemoryKind::Seen, "flash", sal)
                .expect("observes");
        }
        npc
    }
    #[test]
    fn mind_tick_delayed_reflection_reads_ring_when_ready() {
        let mut npc = delayed_fixture();
        npc.reflect(30).expect("reflects");
        let felt = npc.events().last().expect("felt");
        assert_eq!(felt.salience, 100);
        let mut twin = delayed_fixture();
        twin.reflect(30).expect("twin");
        assert_eq!(felt, twin.events().last().expect("twin felt"));
    }
    #[test]
    fn mind_tick_delayed_reflection_refuses_empty() {
        let mut empty = mind();
        assert_eq!(empty.reflect(0), Err(MindError::NothingToReflect));
    }
}
