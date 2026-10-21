//! Supervisor over the [`heal`](super::heal) registry: fault catalog,
//! escalation with backoff, tick-boundary sim heals and a heal tape.
//!
//! Consumer: the real-fault wiring in render, audio, net, save and provider,
//! and the chaos suite.
//! Data owner: [`Supervisor`] owns policies, attempt budgets, escalation,
//! the deferred queue and the tape; the caller owns the [`RingBuffer`].
//! Units: ticks (`u64`), retries (`u32`), backoff (`u64` ms). Determinism:
//! `BTreeMap` order, saturating counters, tick-derived backoff, no clock.
//! Error type: [`HealError`](super::heal::HealError); over-budget faults
//! quarantine loudly, never heal silently.
//!
//! Planning (which kind, which attempt, how long to wait) stays here.
//! Execution stays with the caller: service heals apply at once, sim heals
//! wait for [`Supervisor::apply_tick_boundary`] and land in the tape the
//! simulation copies into its own action log for replay.

use super::heal::{applies_at_tick, HealError, HealKind, Registry};
use super::recovery::{HealCap, HealScope, HealValidation, HEAL_CAP_WINDOW_TICKS};
use super::{Event, EventCode, RingBuffer};
use engine_core::hash::{mix64, GOLDEN_GAMMA};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Maximum sim heals waiting for a tick boundary. Past this the supervisor
/// returns [`HealError::PendingFull`] so the caller drains first.
pub const MAX_PENDING_HEALS: usize = 64;

/// Maximum heal tape rows. Past this the supervisor returns
/// [`HealError::TapeFull`] so the caller exports first; rows are never
/// dropped silently.
pub const MAX_TAPE_RECORDS: usize = 1024;

// Quarantine cool-down plus half-open single-probe re-admit, ported fresh from donor mizcausevic-dev/reliability-toolkit-rs (MIT, https://github.com/mizcausevic-dev/reliability-toolkit-rs) Closed/Open/HalfOpen with cool_down and half_open_max_calls.
/// Ticks a quarantined code stays closed before one half-open probe is admitted.
pub const QUARANTINE_COOLDOWN_TICKS: u64 = 10;

/// Maximum half-open probe calls admitted per cool-down expiry.
pub const HALF_OPEN_MAX_CALLS: u32 = 1;

/// Quarantine entry: closed until until_tick, then one half-open probe when probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuarantineEntry {
    until_tick: u64,
    pub(crate) probing: bool,
}

/// Bounded heal policy for one fault code: which runnable recovery, how
/// many attempts, and how long to wait between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultPolicy {
    /// Fault code this policy heals.
    pub code: EventCode,
    /// Caller-executed recovery while within budget.
    pub kind: HealKind,
    /// Maximum heal attempts before the fault quarantines.
    pub attempts_max: u32,
    /// First escalation backoff in milliseconds; doubled per attempt.
    pub backoff_base_ms: u64,
    /// Backoff ceiling in milliseconds.
    pub backoff_cap_ms: u64,
    /// Ticks between the fault and the next attempt; sim heals also wait
    /// at least one tick for the next boundary.
    pub retry_after_ticks: u64,
    /// What the heal touches: presentation (no simulation or scene state) or
    /// state (changes it, so it waits for a tick boundary).
    pub scope: HealScope,
}

impl FaultPolicy {
    /// Build one catalog row.
    #[must_use]
    pub fn new(
        code: EventCode,
        kind: HealKind,
        attempts_max: u32,
        backoff_base_ms: u64,
        backoff_cap_ms: u64,
        retry_after_ticks: u64,
    ) -> Self {
        Self {
            code,
            kind,
            attempts_max,
            backoff_base_ms,
            backoff_cap_ms,
            retry_after_ticks,
            // The shared rule owns the default: the three authoritative
            // simulation faults are state heals, everything else is a
            // presentation heal.
            scope: if applies_at_tick(code) {
                HealScope::State
            } else {
                HealScope::Presentation
            },
        }
    }

    /// Set the scope of this row: [`HealScope::Presentation`] (the default)
    /// or [`HealScope::State`]. A state row always waits for a tick boundary.
    #[must_use]
    pub fn with_scope(mut self, scope: HealScope) -> Self {
        self.scope = scope;
        self
    }

    /// Seeded jittered backoff for `attempt` with `seed` over this row's
    /// base and cap. Units: ms (`u64`), attempt (`u32`), seed (`u64`).
    /// Determinism: same inputs give same output, no clock.
    #[must_use]
    pub fn jittered_backoff(self, attempt: u32, seed: u64) -> u64 {
        backoff_jittered_ms(self.backoff_base_ms, self.backoff_cap_ms, attempt, seed)
    }
}

/// The shipped fault catalog: every auto-heal table row with its bounded
/// policy, in table order. Budgets (retries / backoff base to cap):
/// nonfinite restore 3, stuck retry 3, desync AI-takeover 4 at 100-1600 ms,
/// device reconnect 3 at 100-2000 ms, pipeline last-good 2 at 50-500 ms,
/// assets retry 2 at 50-800 ms, provider-timeout degrade 2 at 200-2000 ms,
/// provider-malformed reset 2 at 200-2000 ms, stale escalate 2, crash
/// restore 2 at 200-2000 ms, save last-good 1, frame degrade 3.
#[must_use]
pub fn catalog() -> [FaultPolicy; 12] {
    [
        FaultPolicy::new(
            EventCode::PHYSICS_BODY_NONFINITE,
            HealKind::Restore,
            3,
            0,
            0,
            1,
        )
        .with_scope(HealScope::State),
        FaultPolicy::new(
            EventCode::BRAIN_FIGHTER_STUCK,
            HealKind::RetryTick,
            3,
            0,
            0,
            1,
        )
        .with_scope(HealScope::State),
        FaultPolicy::new(
            EventCode::NET_CLIENT_DESYNC,
            HealKind::AiTakeover,
            4,
            100,
            1600,
            1,
        )
        .with_scope(HealScope::State),
        FaultPolicy::new(
            EventCode::RENDER_DEVICE_LOST,
            HealKind::Reconnect,
            3,
            100,
            2000,
            0,
        ),
        FaultPolicy::new(
            EventCode::RENDER_PIPELINE_FAILED,
            HealKind::LastGood,
            2,
            50,
            500,
            0,
        ),
        FaultPolicy::new(
            EventCode::ASSETS_MISSING,
            HealKind::RetryTick,
            2,
            50,
            800,
            0,
        ),
        FaultPolicy::new(
            EventCode::PROVIDER_TIMEOUT,
            HealKind::Degrade,
            2,
            200,
            2000,
            0,
        ),
        FaultPolicy::new(
            EventCode::PROVIDER_MALFORMED,
            HealKind::ResetBudget,
            2,
            200,
            2000,
            0,
        ),
        FaultPolicy::new(EventCode::COMMANDS_STALE, HealKind::Escalate, 2, 0, 0, 0),
        FaultPolicy::new(EventCode::EDITOR_CRASH, HealKind::Restore, 2, 200, 2000, 0),
        FaultPolicy::new(EventCode::SAVE_CORRUPT, HealKind::LastGood, 1, 0, 0, 0),
        FaultPolicy::new(
            EventCode::RENDER_FRAME_OVER_BUDGET,
            HealKind::Degrade,
            3,
            0,
            0,
            0,
        ),
    ]
}

/// Deterministic exponential backoff in milliseconds: `base` doubled per
/// attempt past the first, capped at `cap`. Attempt 0 and base 0 wait
/// nothing; arithmetic saturates instead of wrapping.
#[must_use]
pub fn backoff_ms(base_ms: u64, cap_ms: u64, attempt: u32) -> u64 {
    if attempt == 0 || base_ms == 0 {
        return 0;
    }
    let shift = attempt.saturating_sub(1).min(63);
    base_ms
        .saturating_mul(1_u64.checked_shl(shift).unwrap_or(u64::MAX))
        .min(cap_ms)
}

// PORT(Xuanwo/backon@60d03488fd8fdeadcdcf2c9b1a6cb69c6c07f3c0:backon/src/backoff/exponential.rs) Apache-2.0 (verified live 2026-10-09 via gh api license + LICENSE text): seeded jitter (with_jitter/with_jitter_seed, cur + cur*rand) adapted to symmetric [exp/2, exp+ceil(exp/2)] to preserve mean for herd avoidance, plus total-delay gate (with_total_delay, cumulative + next <= total). Integer port, std-only, deterministic, no float; mixer reuses engine-core SplitMix64 (`GOLDEN_GAMMA` + `mix64`), no new dependency.
///
/// Seeded jittered backoff in milliseconds: exponential window `exp` from
/// [`backoff_ms`] spread symmetrically around `exp` so the mean stays `exp`
/// while same-tick clients desynchronize. Units: ms (`u64`), attempt
/// (`u32`), seed (`u64`). Determinism: SplitMix64 diffusion over (`seed`,
/// `attempt`), no clock, same inputs give same output. Attempt 0 and `exp`
/// 0 wait nothing; the jittered value is clamped to `cap`; arithmetic
/// saturates.
#[must_use]
pub fn backoff_jittered_ms(base_ms: u64, cap_ms: u64, attempt: u32, seed: u64) -> u64 {
    let exp = backoff_ms(base_ms, cap_ms, attempt);
    if exp == 0 {
        return 0;
    }
    let low = exp / 2;
    let high = exp.saturating_add(exp.saturating_sub(low));
    let span = high.saturating_sub(low).saturating_add(1);
    if span == 0 {
        return exp.min(cap_ms);
    }
    low.saturating_add(jitter_word(seed, attempt) % span)
        .min(cap_ms)
}

/// Fixed SplitMix64 seed for replay-stable backon jitter. Units: seed (`u64`).
/// Determinism: same seed plus attempt gives same delay.
pub const SPLITMIX_JITTER_SEED: u64 = 2040;

/// Deterministic mixer over `seed` and `attempt`: one SplitMix64 golden-step
/// fork (`GOLDEN_GAMMA` from `engine-core`) plus the canonical SplitMix64
/// finalizer (`mix64` from `engine-core`). Determinism: pure integer mix,
/// identical on every platform, no clock.
fn jitter_word(seed: u64, attempt: u32) -> u64 {
    mix64(seed.wrapping_add(u64::from(attempt).wrapping_mul(GOLDEN_GAMMA)))
}

/// Total-delay budget gate: true while `attempt` stays within `max_times`
/// and `elapsed_ms` stays within `total_ms`. Units: attempts (`u32`),
/// delays (`u64` ms). Determinism: pure comparison, no clock.
#[must_use]
pub fn backoff_within_budget(attempt: u32, max_times: u32, elapsed_ms: u64, total_ms: u64) -> bool {
    attempt <= max_times && elapsed_ms <= total_ms
}

/// One sim heal waiting for its tick boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingHeal {
    /// Fault code being healed.
    pub code: EventCode,
    /// Tick the fault was reported.
    pub fault_tick: u64,
    /// Boundary tick the sim must apply it at.
    pub applies_at: u64,
    /// Recovery the sim executes there.
    pub kind: HealKind,
    /// One-based attempt count.
    pub attempt: u32,
    /// Backoff that was advised, in milliseconds.
    pub backoff_ms: u64,
    /// Index of this heal's row in the supervisor tape.
    pub tape_index: usize,
}

/// One tape row: fault to applied heal, or loud quarantine. The simulation
/// copies these rows into its own action log so replay reproduces the same
/// checkpoints with logging on or off. A row also carries the proof of the
/// heal: the digest of the last good state, the digest after the heal and the
/// verdict. An unproven row is evidence that the heal ran, not that it worked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealTapeRecord {
    /// Fault code that was reported.
    pub code: EventCode,
    /// Tick the fault was reported.
    pub fault_tick: u64,
    /// Tick the heal applied (`None` while deferred to a boundary).
    pub applied_tick: Option<u64>,
    /// Recovery executed, or [`HealKind::Quarantine`] past the budget.
    pub kind: HealKind,
    /// One-based attempt count.
    pub attempt: u32,
    /// Backoff that was advised, in milliseconds.
    pub backoff_ms: u64,
    /// True when the budget is spent: quarantine loudly, never silently.
    pub quarantined: bool,
    /// What the heal touches: presentation or state.
    pub scope: HealScope,
    /// Digest of the last good state before the heal (`None` while unproven).
    pub before_digest: Option<u64>,
    /// Digest of the state after the heal (`None` while unproven).
    pub after_digest: Option<u64>,
    /// The verdict of the proof: unproven, recovered, not recovered, frozen
    /// frame or rolled back.
    pub validation: HealValidation,
}

/// Escalation state for one fault code after its latest report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscalationState {
    /// Zero-based level: 0 on the first attempt, rising per retry.
    pub level: u32,
    /// Backoff advised for the next attempt, in milliseconds.
    pub backoff_ms: u64,
    /// Earliest tick a retry may run.
    pub not_before_tick: u64,
}

/// Supervised advice for one fault occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisedDecision {
    /// Recovery to execute, or [`HealKind::Quarantine`] past the budget.
    pub kind: HealKind,
    /// One-based attempt count.
    pub attempt: u32,
    /// Backoff before the next attempt, in milliseconds.
    pub backoff_ms: u64,
    /// True for sim faults: execute only at [`Self::applies_at`].
    pub deferred: bool,
    /// Boundary tick for deferred heals, else the fault tick.
    pub applies_at: u64,
    /// True when the budget is spent: quarantine loudly.
    pub quarantined: bool,
    /// True when a late attempt spent hedge budget: run this retry now as
    /// the backup, alongside the still-running primary.
    pub hedged: bool,
}

impl SupervisedDecision {
    /// True when the budget is spent and the caller must quarantine the
    /// object or task and fail loudly.
    #[must_use]
    pub fn is_quarantine(self) -> bool {
        self.quarantined
    }
}

/// One deferred heal the sim just applied at a tick boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedHeal {
    /// Fault code that was healed.
    pub code: EventCode,
    /// Tick the fault was reported.
    pub fault_tick: u64,
    /// Boundary tick it applied at.
    pub applied_tick: u64,
    /// Recovery the sim executed.
    pub kind: HealKind,
    /// One-based attempt count.
    pub attempt: u32,
}

/// Calls in one rolling breaker window before its error percent is judged.
const BREAKER_WINDOW_CALLS: usize = 10;

/// Error percent at or above which a full window opens the breaker.
const BREAKER_ERROR_PERCENT: u32 = 50;

/// Breaker position. Closed admits retries, open rejects them fast,
/// half-open admits one recovery probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerState {
    /// Retries flow; outcomes fill the rolling window.
    Closed,
    /// Retry storm guard: retries stop here without touching the budget.
    Open,
    /// One probe is admitted to test recovery.
    HalfOpen,
}

/// Typed breaker rejection: open means stop retrying, not try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerError {
    /// Breaker is open, or its one probe is already spent.
    Open,
}

impl fmt::Display for BreakerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "circuit is open"),
        }
    }
}

/// Rolling error-percent breaker: opens at 50% or more errors over the last
/// 10 calls, then one half-open probe closes it on success or reopens it.
/// No clock: `opened_at` counts calls, so the same outcomes give the same state.
#[derive(Debug, Clone)]
pub struct RollingBreaker {
    /// Ring of the last [`BREAKER_WINDOW_CALLS`] outcomes (`true` is an error).
    window: [bool; BREAKER_WINDOW_CALLS],
    /// Next ring slot to overwrite.
    next: usize,
    /// Outcomes recorded in the current window (caps at the window size).
    filled: usize,
    /// Total calls observed (1-based, saturating).
    attempt: u64,
    /// Position of the breaker.
    state: BreakerState,
    /// 1-based call index where the breaker last opened (`None` if never).
    opened_at: Option<u64>,
    /// Probe calls consumed while half-open (caps at [`HALF_OPEN_MAX_CALLS`]).
    probes: u32,
}

impl RollingBreaker {
    /// Closed breaker with an empty window.
    pub fn new() -> Self {
        Self {
            window: [false; BREAKER_WINDOW_CALLS],
            next: 0,
            filled: 0,
            attempt: 0,
            state: BreakerState::Closed,
            opened_at: None,
            probes: 0,
        }
    }

    /// Total calls observed so far.
    #[must_use]
    pub fn attempt(&self) -> u64 {
        self.attempt
    }

    /// True while new retries must reject fast.
    pub fn is_open(&self) -> bool {
        self.state == BreakerState::Open
    }

    /// True while the single recovery probe is admitted.
    pub fn is_half_open(&self) -> bool {
        self.state == BreakerState::HalfOpen
    }

    /// 1-based call index where the breaker last opened.
    pub fn opened_at(&self) -> Option<u64> {
        self.opened_at
    }

    /// Gate one retry: `Ok` admits it, `Err` stops it without using window
    /// budget. Half-open admits here; `half_open_probe` is what spends the
    /// single probe.
    pub fn admit(&self) -> Result<(), BreakerError> {
        match self.state {
            BreakerState::Closed | BreakerState::HalfOpen => Ok(()),
            BreakerState::Open => Err(BreakerError::Open),
        }
    }

    /// Record one retry outcome (`ok` is success). A closed window trips at
    /// 50% or more errors once full; a half-open probe closes on success or
    /// reopens on failure; an open breaker ignores outcomes.
    pub fn observe(&mut self, ok: bool) {
        match self.state {
            BreakerState::HalfOpen => self.complete_probe(ok),
            BreakerState::Open => {}
            BreakerState::Closed => {
                self.push(!ok);
                if self.filled == BREAKER_WINDOW_CALLS
                    && self.error_percent() >= BREAKER_ERROR_PERCENT
                {
                    self.state = BreakerState::Open;
                    self.opened_at = Some(self.attempt);
                }
            }
        }
    }

    /// Move an open breaker to half-open and spend its one probe. A closed
    /// breaker admits without moving; a second probe in one half-open period
    /// is refused.
    pub fn half_open_probe(&mut self) -> Result<(), BreakerError> {
        match self.state {
            BreakerState::Closed => Ok(()),
            BreakerState::HalfOpen => {
                if self.probes >= HALF_OPEN_MAX_CALLS {
                    Err(BreakerError::Open)
                } else {
                    self.probes = self.probes.saturating_add(1);
                    Ok(())
                }
            }
            BreakerState::Open => {
                self.state = BreakerState::HalfOpen;
                self.probes = 1;
                Ok(())
            }
        }
    }

    /// Finish the half-open probe: success closes and clears the window,
    /// failure reopens at once.
    fn complete_probe(&mut self, ok: bool) {
        if ok {
            self.state = BreakerState::Closed;
            self.window = [false; BREAKER_WINDOW_CALLS];
            self.next = 0;
            self.filled = 0;
            self.probes = 0;
        } else {
            self.state = BreakerState::Open;
            self.attempt = self.attempt.saturating_add(1);
            self.opened_at = Some(self.attempt);
            self.probes = 0;
        }
    }

    /// Push one error flag into the ring and count the call.
    fn push(&mut self, is_error: bool) {
        self.window[self.next] = is_error;
        self.next = (self.next + 1) % BREAKER_WINDOW_CALLS;
        if self.filled < BREAKER_WINDOW_CALLS {
            self.filled = self.filled.saturating_add(1);
        }
        self.attempt = self.attempt.saturating_add(1);
    }

    /// Error percent over the filled window (0 while empty).
    pub fn error_percent(&self) -> u32 {
        if self.filled == 0 {
            return 0;
        }
        let errors =
            u32::try_from(self.window.iter().filter(|hit| **hit).count()).unwrap_or(u32::MAX);
        let filled = u32::try_from(self.filled).unwrap_or(u32::MAX);
        errors.saturating_mul(100) / filled.max(1)
    }
}

impl Default for RollingBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-code hedge budget: a retry that faults past `hedge_after_ms` may run
/// one backup at once, while backups stay within `max_extra_percent` of the
/// reported primaries. Units: ms (`u64`), counts (`u64`), percent (`u32`).
/// Determinism: integer counts only, no clock, same latencies give the same
/// decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HedgeBudget {
    /// Latency past which a late retry owes a backup, in milliseconds.
    hedge_after_ms: u64,
    /// Extra-load cap as integer percent of reported primaries.
    max_extra_percent: u32,
    /// Reported primary attempts counted against the cap.
    primaries: u64,
    /// Backups spent so far, never above the cap.
    backups: u64,
}

impl HedgeBudget {
    /// Empty budget: no backups spent, cap applies to future reports.
    #[must_use]
    pub fn new(hedge_after_ms: u64, max_extra_percent: u32) -> Self {
        Self {
            hedge_after_ms,
            max_extra_percent,
            primaries: 0,
            backups: 0,
        }
    }

    /// Latency past which a late retry owes a backup, in milliseconds.
    #[must_use]
    pub fn hedge_after_ms(&self) -> u64 {
        self.hedge_after_ms
    }

    /// Extra-load cap as integer percent of reported primaries.
    #[must_use]
    pub fn max_extra_percent(&self) -> u32 {
        self.max_extra_percent
    }

    /// Reported primaries counted against the cap.
    #[must_use]
    pub fn primaries(&self) -> u64 {
        self.primaries
    }

    /// Backups spent so far.
    #[must_use]
    pub fn backups(&self) -> u64 {
        self.backups
    }

    /// Count one reported primary at `latency_ms`; `true` spends one backup
    /// when the sample is past the threshold and the cap still allows it.
    pub fn observe(&mut self, latency_ms: u64) -> bool {
        self.primaries = self.primaries.saturating_add(1);
        let allowed = self
            .primaries
            .saturating_mul(u64::from(self.max_extra_percent))
            / 100;
        let hedge = latency_ms > self.hedge_after_ms && self.backups < allowed;
        if hedge {
            self.backups = self.backups.saturating_add(1);
        }
        hedge
    }
}

/// MAPE-K supervisor: catalog policies with attempt budgets, escalation
/// with backoff, a tick-boundary queue for sim heals and a heal tape.
///
/// Monitor/Analyze/Execute stay with the caller. The supervisor plans
/// (which kind, which attempt, how long to wait, which boundary) and
/// records (`heal.applied` / `heal.failed` into the caller's ring plus one
/// tape row per report). Budget and capacity errors precede any state
/// change: a failed report consumes no attempt.
#[derive(Debug, Clone)]
pub struct Supervisor {
    pub(crate) registry: Registry,
    pub(crate) policies: BTreeMap<u32, FaultPolicy>,
    pending: VecDeque<PendingHeal>,
    pub(crate) tape: Vec<HealTapeRecord>,
    pub(crate) escalation: BTreeMap<u32, EscalationState>,
    pub(crate) quarantine: BTreeMap<u32, QuarantineEntry>,
    /// Opt-in rolling breakers per code. Empty by default, so codes without
    /// one keep the plain attempt-budget path.
    breakers: BTreeMap<u32, RollingBreaker>,
    /// Opt-in hedge budgets per code. Empty by default, so codes without one
    /// keep the plain backoff path.
    hedges: BTreeMap<u32, HedgeBudget>,
    /// Supervisor-level events: a refused budget reset, a blown heal cap, a
    /// frozen frame. These have no caller ring to go into, so the supervisor
    /// keeps its own and never drops them silently.
    pub(crate) events: RingBuffer,
    /// Rolling heal cap: how many heals may fire inside one rolling minute
    /// of ticks. Past the cap the run reads red.
    pub(crate) heal_cap: HealCap,
    /// Presentation heals that only held the last good frame. Never green.
    pub(crate) frozen_frames: u32,
}

/// Capacity of the supervisor-level event ring.
pub const SUPERVISOR_EVENT_CAP: usize = 64;

/// Default heal cap: one heal per catalog row per rolling minute. The number
/// comes from the shipped catalog, so it is measured, not guessed.
#[must_use]
pub fn default_heal_cap_max() -> u32 {
    u32::try_from(catalog().len()).unwrap_or(u32::MAX)
}

impl Supervisor {
    /// Supervisor preloaded with [`catalog`].
    #[must_use]
    pub fn new() -> Self {
        let mut supervisor = Self {
            registry: Registry::new(),
            policies: BTreeMap::new(),
            pending: VecDeque::new(),
            tape: Vec::new(),
            escalation: BTreeMap::new(),
            quarantine: BTreeMap::new(),
            breakers: BTreeMap::new(),
            hedges: BTreeMap::new(),
            events: RingBuffer::new(SUPERVISOR_EVENT_CAP),
            heal_cap: HealCap::new(default_heal_cap_max(), HEAL_CAP_WINDOW_TICKS),
            frozen_frames: 0,
        };
        for policy in catalog() {
            supervisor.register_policy(policy);
        }
        supervisor
    }

    /// Register (or replace) one policy row. Re-registering keeps that code's
    /// attempt budget, quarantine and escalation: new knowledge may change the
    /// policy, never the budget. A heal cannot reopen its own budget.
    /// In-flight deferred heals and tape history are kept.
    pub fn register_policy(&mut self, policy: FaultPolicy) {
        self.registry
            .register_keep_budget(policy.code, policy.attempts_max, policy.kind);
        self.insert_policy(policy);
    }

    /// Copy of the policy row for `code`, if any.
    #[must_use]
    pub fn policy(&self, code: EventCode) -> Option<FaultPolicy> {
        self.policies.get(&code.raw()).copied()
    }

    /// Seeded jittered advice for `code` at `attempt`, gated by the row's
    /// attempt budget plus the donor total-delay stop (`elapsed_ms` plus the
    /// next jittered wait must stay within `total_ms`). Returns `None` for
    /// unregistered codes or when over budget. Units: ms (`u64`), attempt
    /// (`u32`), seed (`u64`). Determinism: same inputs give same output, no
    /// clock. Consumer: render, audio, net and provider heal wiring calls
    /// this when spreading same-tick retries; `report_fault` stays lockstep.
    #[must_use]
    pub fn jittered_backoff_for(
        &self,
        code: EventCode,
        attempt: u32,
        seed: u64,
        elapsed_ms: u64,
        total_ms: u64,
    ) -> Option<u64> {
        let policy = self.policy(code)?;
        if !backoff_within_budget(attempt, policy.attempts_max, elapsed_ms, total_ms) {
            return None;
        }
        let next = policy.jittered_backoff(attempt, seed);
        if elapsed_ms.saturating_add(next) > total_ms {
            return None;
        }
        Some(next)
    }

    /// Number of policy rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.policies.len()
    }

    /// True when no policy rows are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Faults seen so far for `code` (zero when unregistered).
    #[must_use]
    pub fn attempts(&self, code: EventCode) -> u32 {
        self.registry.attempts(code)
    }

    /// True once `code` has spent its budget.
    #[must_use]
    pub fn is_quarantined(&self, code: EventCode) -> bool {
        if let Some(entry) = self.quarantine.get(&code.raw()) {
            return !entry.probing;
        }
        self.registry.is_quarantined(code)
    }

    /// Tick until which code stays quarantined, if quarantined.
    #[must_use]
    pub fn quarantined_until(&self, code: EventCode) -> Option<u64> {
        self.quarantine
            .get(&code.raw())
            .map(|entry| entry.until_tick)
    }

    /// True while code admits its single half-open probe.
    #[must_use]
    pub fn is_half_open(&self, code: EventCode) -> bool {
        self.quarantine
            .get(&code.raw())
            .map(|entry| entry.probing)
            .unwrap_or(false)
    }

    /// Latest escalation state for `code`, if it faulted.
    #[must_use]
    pub fn escalation_of(&self, code: EventCode) -> Option<EscalationState> {
        self.escalation.get(&code.raw()).copied()
    }

    /// Sim heals waiting for a tick boundary, oldest first.
    #[must_use]
    pub fn pending(&self) -> &VecDeque<PendingHeal> {
        &self.pending
    }

    /// Number of deferred heals waiting for a tick boundary.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Heal tape rows in report order.
    #[must_use]
    pub fn tape(&self) -> &[HealTapeRecord] {
        &self.tape
    }

    /// Number of heal tape rows.
    #[must_use]
    pub fn tape_len(&self) -> usize {
        self.tape.len()
    }


    /// Turn on the rolling error-percent breaker for `code`. Opt-in: codes
    /// without one keep the plain attempt-budget path. Re-enabling keeps the
    /// existing window. `reset` leaves the breaker alone; only a probe
    /// success closes it.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::NotRegistered`] when `code` has no policy row.
    pub fn enable_breaker(&mut self, code: EventCode) -> Result<(), HealError> {
        if !self.policies.contains_key(&code.raw()) {
            return Err(HealError::NotRegistered(code));
        }
        self.breakers.entry(code.raw()).or_default();
        Ok(())
    }

    /// True while the code's breaker is open and its retries reject fast.
    #[must_use]
    pub fn breaker_is_open(&self, code: EventCode) -> bool {
        self.breakers
            .get(&code.raw())
            .is_some_and(RollingBreaker::is_open)
    }

    /// True while the code's breaker admits its single half-open probe.
    #[must_use]
    pub fn breaker_is_half_open(&self, code: EventCode) -> bool {
        self.breakers
            .get(&code.raw())
            .is_some_and(RollingBreaker::is_half_open)
    }

    /// Call index where the code's breaker last opened, if it ever did.
    #[must_use]
    pub fn breaker_opened_at(&self, code: EventCode) -> Option<u64> {
        self.breakers
            .get(&code.raw())
            .and_then(RollingBreaker::opened_at)
    }

    /// Feed one retry outcome (`ok` is success) into the code's breaker.
    /// Codes without a breaker are a no-op.
    pub fn record_attempt_outcome(&mut self, code: EventCode, ok: bool) {
        if let Some(breaker) = self.breakers.get_mut(&code.raw()) {
            breaker.observe(ok);
        }
    }

    /// Move an open breaker to half-open and spend its one probe. Returns
    /// `false` when the probe is refused; codes without a breaker return
    /// `true`. The caller owns the cool-down: this never reads a tick.
    pub fn breaker_probe(&mut self, code: EventCode) -> bool {
        self.breakers
            .get_mut(&code.raw())
            .is_none_or(|breaker| breaker.half_open_probe().is_ok())
    }

    /// Turn on the hedge budget for `code`. Opt-in: codes without one keep
    /// the plain backoff path. Re-enabling replaces the row and its counts:
    /// new knowledge, fresh budget. `reset` leaves it alone; the cap bounds
    /// load, not health.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::NotRegistered`] when `code` has no policy row.
    pub fn enable_hedge(
        &mut self,
        code: EventCode,
        hedge_after_ms: u64,
        max_extra_percent: u32,
    ) -> Result<(), HealError> {
        if !self.policies.contains_key(&code.raw()) {
            return Err(HealError::NotRegistered(code));
        }
        self.hedges.insert(
            code.raw(),
            HedgeBudget::new(hedge_after_ms, max_extra_percent),
        );
        Ok(())
    }

    /// Copy of the code's hedge budget, if enabled.
    #[must_use]
    pub fn hedge_budget(&self, code: EventCode) -> Option<HedgeBudget> {
        self.hedges.get(&code.raw()).copied()
    }

    /// Count one reported primary at `latency_ms`; `true` spends one backup
    /// when the sample is past the threshold and the cap allows it. Codes
    /// without a hedge budget return `false`.
    fn hedge_admits(&mut self, code: EventCode, latency_ms: u64) -> bool {
        self.hedges
            .get_mut(&code.raw())
            .is_some_and(|budget| budget.observe(latency_ms))
    }

    /// True when the code's breaker is open, so the retry stops here.
    fn breaker_rejects(&self, code: EventCode) -> bool {
        self.breakers
            .get(&code.raw())
            .is_some_and(|breaker| breaker.admit().is_err())
    }

    /// Breaker reject: a quarantine decision with no budget, ring or tape
    /// writes, the same shape as the quarantine fast reject.
    fn breaker_reject(fault_tick: u64, attempts_max: u32) -> SupervisedDecision {
        SupervisedDecision {
            kind: HealKind::Quarantine,
            attempt: attempts_max.saturating_add(1),
            backoff_ms: 0,
            deferred: false,
            applies_at: fault_tick,
            quarantined: true,
            hedged: false,
        }
    }

    /// Fast reject while quarantined: no budget, ring or tape writes.
    fn reject_fast(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        until_tick: u64,
        attempts_max: u32,
    ) -> SupervisedDecision {
        if let Some(state) = self.escalation.get_mut(&code.raw()) {
            state.not_before_tick = until_tick;
        } else {
            self.escalation.insert(
                code.raw(),
                EscalationState {
                    level: attempts_max,
                    backoff_ms: 0,
                    not_before_tick: until_tick,
                },
            );
        }
        SupervisedDecision {
            kind: HealKind::Quarantine,
            attempt: attempts_max.saturating_add(1),
            backoff_ms: 0,
            deferred: false,
            applies_at: fault_tick,
            quarantined: true,
            hedged: false,
        }
    }

    fn fail_half_open_probe(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        ring: &mut RingBuffer,
        attempts_max: u32,
    ) -> SupervisedDecision {
        let until = fault_tick.saturating_add(QUARANTINE_COOLDOWN_TICKS);
        if let Some(entry) = self.quarantine.get_mut(&code.raw()) {
            entry.until_tick = until;
            entry.probing = false;
        }
        let attempt_q = attempts_max.saturating_add(1);
        self.tape.push(HealTapeRecord {
            code,
            fault_tick,
            applied_tick: Some(fault_tick),
            kind: HealKind::Quarantine,
            attempt: attempt_q,
            backoff_ms: 0,
            quarantined: true,
            scope: self.policy(code).map_or(HealScope::Presentation, |row| row.scope),
            before_digest: None,
            after_digest: None,
            validation: HealValidation::Unproven,
        });
        let label = code.name().unwrap_or("unknown");
        ring.push(Event::new(
            EventCode::HEAL_FAILED,
            fault_tick,
            &format!("heal.failed cause={label} attempt={attempt_q}/{attempts_max} quarantined"),
        ));
        if let Some(state) = self.escalation.get_mut(&code.raw()) {
            state.not_before_tick = until;
        } else {
            self.escalation.insert(
                code.raw(),
                EscalationState {
                    level: attempts_max,
                    backoff_ms: 0,
                    not_before_tick: until,
                },
            );
        }
        SupervisedDecision {
            kind: HealKind::Quarantine,
            attempt: attempt_q,
            backoff_ms: 0,
            deferred: false,
            applies_at: fault_tick,
            quarantined: true,
            hedged: false,
        }
    }

    /// Quarantine gate for a fault report: fast-reject while closed,
    /// fail the single half-open probe, or admit the probe after cool-down.
    /// Returns a decision when the report ends here, else `None` to continue
    /// with a fresh attempt.
    fn admit_quarantined_report(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        ring: &mut RingBuffer,
        attempts_max: u32,
    ) -> Option<SupervisedDecision> {
        let entry = self.quarantine.get(&code.raw()).copied()?;
        if fault_tick < entry.until_tick {
            return Some(self.reject_fast(code, fault_tick, entry.until_tick, attempts_max));
        }
        if entry.probing {
            return Some(self.fail_half_open_probe(code, fault_tick, ring, attempts_max));
        }
        self.registry.reset(code);
        if let Some(slot) = self.quarantine.get_mut(&code.raw()) {
            slot.probing = true;
        }
        debug_assert_eq!(HALF_OPEN_MAX_CALLS, 1);
        None
    }

    /// Tape and quarantine rows for an over-budget fault. Callers insert the
    /// escalation row first; this moves its retry gate to the cool-down end.
    pub(crate) fn record_quarantine(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        attempt: u32,
        backoff: u64,
    ) -> SupervisedDecision {
        let until = fault_tick.saturating_add(QUARANTINE_COOLDOWN_TICKS);
        self.quarantine.insert(
            code.raw(),
            QuarantineEntry {
                until_tick: until,
                probing: false,
            },
        );
        if let Some(state) = self.escalation.get_mut(&code.raw()) {
            state.not_before_tick = until;
        }
        self.tape.push(HealTapeRecord {
            code,
            fault_tick,
            applied_tick: Some(fault_tick),
            kind: HealKind::Quarantine,
            attempt,
            backoff_ms: backoff,
            quarantined: true,
            scope: self.policy(code).map_or(HealScope::Presentation, |row| row.scope),
            before_digest: None,
            after_digest: None,
            validation: HealValidation::Unproven,
        });
        SupervisedDecision {
            kind: HealKind::Quarantine,
            attempt,
            backoff_ms: backoff,
            deferred: false,
            applies_at: fault_tick,
            quarantined: true,
            hedged: false,
        }
    }

    /// Tape (and deferred queue) rows for a within-budget heal: deferred to
    /// the tick boundary for sim faults, applied at once otherwise.
    #[expect(
        clippy::too_many_arguments,
        reason = "eight args are the tape-row fields; a struct would only move them"
    )]
    fn record_heal(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        attempt: u32,
        backoff: u64,
        kind: HealKind,
        boundary: bool,
        retry_after_ticks: u64,
        scope: HealScope,
    ) -> SupervisedDecision {
        if boundary {
            let applies_at = fault_tick.saturating_add(retry_after_ticks.max(1));
            self.tape.push(HealTapeRecord {
                code,
                fault_tick,
                applied_tick: None,
                kind,
                attempt,
                backoff_ms: backoff,
                quarantined: false,
                scope,
                before_digest: None,
                after_digest: None,
                validation: HealValidation::Unproven,
            });
            let tape_index = self.tape.len().saturating_sub(1);
            self.pending.push_back(PendingHeal {
                code,
                fault_tick,
                applies_at,
                kind,
                attempt,
                backoff_ms: backoff,
                tape_index,
            });
            SupervisedDecision {
                kind,
                attempt,
                backoff_ms: backoff,
                deferred: true,
                applies_at,
                quarantined: false,
                hedged: false,
            }
        } else {
            self.tape.push(HealTapeRecord {
                code,
                fault_tick,
                applied_tick: Some(fault_tick),
                kind,
                attempt,
                backoff_ms: backoff,
                quarantined: false,
                scope,
                before_digest: None,
                after_digest: None,
                validation: HealValidation::Unproven,
            });
            SupervisedDecision {
                kind,
                attempt,
                backoff_ms: backoff,
                deferred: false,
                applies_at: fault_tick,
                quarantined: false,
                hedged: false,
            }
        }
    }

    /// Report one fault at fault_tick, pushing heal records into the ring.
    ///
    /// # Errors
    ///
    /// Returns NotRegistered, TapeFull and PendingFull without consuming budget.
    pub fn report_fault(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        ring: &mut RingBuffer,
    ) -> Result<SupervisedDecision, HealError> {
        self.plan_fault(code, fault_tick, None, ring)
    }

    /// Report one fault whose attempt took `latency_ms`. With a hedge budget
    /// on the code, a late attempt spends one backup and the retry runs at
    /// once (`hedged`) instead of after its backoff. Units: ms (`u64`).
    ///
    /// # Errors
    ///
    /// Same as [`Self::report_fault`].
    pub fn report_fault_with_latency(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        latency_ms: u64,
        ring: &mut RingBuffer,
    ) -> Result<SupervisedDecision, HealError> {
        self.plan_fault(code, fault_tick, Some(latency_ms), ring)
    }

    fn plan_fault(
        &mut self,
        code: EventCode,
        fault_tick: u64,
        latency_ms: Option<u64>,
        ring: &mut RingBuffer,
    ) -> Result<SupervisedDecision, HealError> {
        let policy = self
            .policies
            .get(&code.raw())
            .copied()
            .ok_or(HealError::NotRegistered(code))?;
        if self.tape.len() >= MAX_TAPE_RECORDS {
            return Err(HealError::TapeFull);
        }
        if self.breaker_rejects(code) {
            return Ok(Self::breaker_reject(fault_tick, policy.attempts_max));
        }
        if let Some(decision) =
            self.admit_quarantined_report(code, fault_tick, ring, policy.attempts_max)
        {
            return Ok(decision);
        }
        let attempt = self.registry.attempts(code).saturating_add(1);
        let backoff = backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, attempt);
        // A state heal changes simulation or scene state, so it waits for a
        // tick boundary; a presentation heal may apply at once.
        let boundary = policy.scope.is_state();
        let would_heal = attempt <= policy.attempts_max;
        if would_heal && boundary && self.pending.len() >= MAX_PENDING_HEALS {
            if self.is_half_open(code) {
                if let Some(slot) = self.quarantine.get_mut(&code.raw()) {
                    slot.probing = false;
                }
            }
            return Err(HealError::PendingFull);
        }
        let decision = self.registry.on_fault(code, fault_tick, ring)?;
        let quarantined = decision.is_quarantine();
        self.escalation.insert(
            code.raw(),
            EscalationState {
                level: attempt.saturating_sub(1),
                backoff_ms: backoff,
                not_before_tick: fault_tick.saturating_add(policy.retry_after_ticks),
            },
        );
        if quarantined {
            return Ok(self.record_quarantine(code, fault_tick, attempt, backoff));
        }
        let hedged =
            !boundary && latency_ms.is_some_and(|latency| self.hedge_admits(code, latency));
        let mut decision = self.record_heal(
            code,
            fault_tick,
            attempt,
            if hedged { 0 } else { backoff },
            decision.kind,
            boundary,
            policy.retry_after_ticks,
            policy.scope,
        );
        decision.hedged = hedged;
        // The rolling heal cap: one heal counts once. Past the cap the run
        // reads red, never degraded.
        if self.heal_cap.observe(fault_tick) {
            ring.push(Event::new(
                EventCode::HEAL_FAILED,
                fault_tick,
                &format!(
                    "heal.failed cause=heal.cap heals in window={} max={} run=red",
                    self.heal_cap.heals_in_window(fault_tick),
                    self.heal_cap.max_heals()
                ),
            ));
        }
        Ok(decision)
    }

    /// Report one fault from a raw code, rejecting unregistered values
    /// before touching any budget counter.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::UnknownCode`] for unregistered codes and the
    /// same capacity errors as [`Self::report_fault`].
    pub fn report_raw(
        &mut self,
        raw: u32,
        fault_tick: u64,
        ring: &mut RingBuffer,
    ) -> Result<SupervisedDecision, HealError> {
        let code = EventCode::from_raw(raw).map_err(HealError::from)?;
        self.report_fault(code, fault_tick, ring)
    }

    /// Apply every deferred heal due at or before `boundary_tick`, oldest
    /// first, and stamp their tape rows with the boundary tick. Returns
    /// the applied heals for the sim to execute; empty when nothing is
    /// due, allocating nothing in that case.
    pub fn apply_tick_boundary(&mut self, boundary_tick: u64) -> Vec<AppliedHeal> {
        let mut applied = Vec::new();
        let mut kept = VecDeque::with_capacity(self.pending.len());
        for waiting in self.pending.drain(..) {
            if waiting.applies_at <= boundary_tick {
                if let Some(row) = self.tape.get_mut(waiting.tape_index) {
                    if row.code == waiting.code && row.fault_tick == waiting.fault_tick {
                        row.applied_tick = Some(boundary_tick);
                    }
                }
                applied.push(AppliedHeal {
                    code: waiting.code,
                    fault_tick: waiting.fault_tick,
                    applied_tick: boundary_tick,
                    kind: waiting.kind,
                    attempt: waiting.attempt,
                });
            } else {
                kept.push_back(waiting);
            }
        }
        self.pending = kept;
        applied
    }

    fn insert_policy(&mut self, policy: FaultPolicy) {
        self.policies.insert(policy.code.raw(), policy);
    }
}

impl Default for Supervisor {
    /// Supervisor preloaded with [`catalog`].
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::heal::{applies_at_tick, HealDecision};
    use super::super::recovery::{
        HealObservation, ResetOutcome, RollbackProof, RunHealth, VerifiedRecovery,
    };
    use super::*;

    fn ring(cap: usize) -> RingBuffer {
        RingBuffer::new(cap)
    }

    #[test]
    fn catalog_covers_every_fault_row_once() {
        let rows = catalog();
        assert_eq!(rows.len(), 12);
        let codes: Vec<EventCode> = rows.iter().map(|row| row.code).collect();
        assert_eq!(
            codes,
            vec![
                EventCode::PHYSICS_BODY_NONFINITE,
                EventCode::BRAIN_FIGHTER_STUCK,
                EventCode::NET_CLIENT_DESYNC,
                EventCode::RENDER_DEVICE_LOST,
                EventCode::RENDER_PIPELINE_FAILED,
                EventCode::ASSETS_MISSING,
                EventCode::PROVIDER_TIMEOUT,
                EventCode::PROVIDER_MALFORMED,
                EventCode::COMMANDS_STALE,
                EventCode::EDITOR_CRASH,
                EventCode::SAVE_CORRUPT,
                EventCode::RENDER_FRAME_OVER_BUDGET,
            ]
        );
        // Budgets are positive where a heal exists; caps cover bases.
        for row in &rows {
            assert!(row.attempts_max >= 1, "code {:?}", row.code);
            assert!(
                row.backoff_cap_ms >= row.backoff_base_ms,
                "code {:?}",
                row.code
            );
        }
        // All eight catalog policies are represented.
        let mut kinds: Vec<HealKind> = rows.iter().map(|row| row.kind).collect();
        kinds.sort_by_key(|kind| *kind as u8);
        kinds.dedup();
        for required in [
            HealKind::RetryTick,
            HealKind::ResetBudget,
            HealKind::LastGood,
            HealKind::Degrade,
            HealKind::Reconnect,
            HealKind::AiTakeover,
            HealKind::Restore,
            HealKind::Escalate,
        ] {
            assert!(kinds.contains(&required), "missing {required:?}");
        }
    }

    #[test]
    fn backoff_doubles_then_caps() {
        assert_eq!(backoff_ms(100, 2000, 0), 0);
        assert_eq!(backoff_ms(0, 2000, 3), 0);
        assert_eq!(backoff_ms(100, 2000, 1), 100);
        assert_eq!(backoff_ms(100, 2000, 2), 200);
        assert_eq!(backoff_ms(100, 2000, 3), 400);
        assert_eq!(backoff_ms(100, 2000, 4), 800);
        assert_eq!(backoff_ms(100, 2000, 5), 1600);
        assert_eq!(backoff_ms(100, 2000, 6), 2000);
        assert_eq!(backoff_ms(100, 2000, 7), 2000);
        assert_eq!(backoff_ms(100, 2000, u32::MAX), 2000);
        // Saturation never wraps past the cap.
        assert_eq!(backoff_ms(u64::MAX, u64::MAX, u32::MAX), u64::MAX);
        assert_eq!(backoff_ms(u64::MAX, 7, 200), 7);
    }

    #[test]
    fn jittered_spread_keeps_lockstep_mean_and_budget_gates() {
        // Lockstep baseline: 100, 200, 400, 800, 1600 (mean 620).
        let lockstep: Vec<u64> = (1..=5_u32).map(|a| backoff_ms(100, 10_000, a)).collect();
        assert_eq!(lockstep, vec![100, 200, 400, 800, 1600]);
        let lockstep_sum: u64 = lockstep.iter().sum();
        assert_eq!(lockstep_sum, 3_100);

        // Seeded jitter spreads each attempt but stays in its symmetric
        // window and under the cap; same seed replays bit-identically.
        let jittered: Vec<u64> = (1..=5_u32)
            .map(|a| backoff_jittered_ms(100, 10_000, a, SPLITMIX_JITTER_SEED))
            .collect();
        let replay: Vec<u64> = (1..=5_u32)
            .map(|a| backoff_jittered_ms(100, 10_000, a, SPLITMIX_JITTER_SEED))
            .collect();
        assert_eq!(jittered, replay);
        for (index, next) in jittered.iter().enumerate() {
            let attempt = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
            let exp = backoff_ms(100, 10_000, attempt);
            let low = exp / 2;
            let high = exp.saturating_add(exp.saturating_sub(low));
            assert!(
                (low..=high).contains(next),
                "attempt {attempt}: jittered {next} outside [{low}, {high}]"
            );
            assert!(*next <= 10_000, "jittered {next} exceeds cap");
        }
        // Spread: the seeded run must not be lockstep; different seeds differ.
        assert_ne!(jittered, lockstep, "seeded jitter must move the number");
        assert_ne!(
            backoff_jittered_ms(100, 10_000, 3, 7),
            backoff_jittered_ms(100, 10_000, 3, 8),
            "adjacent seeds must usually differ"
        );
        // Same mean: average over 200 seeds for attempt 3 (exp 400) stays
        // near 400; the symmetric window preserves the mean by construction.
        let mut total: u64 = 0;
        for seed in 0..200_u64 {
            total = total.saturating_add(backoff_jittered_ms(100, 10_000, 3, seed));
        }
        let mean = total / 200;
        assert!(
            (300..=500).contains(&mean),
            "200-seed mean {mean} drifts from exp 400"
        );
        // Five-attempt seeded mean stays within half-to-one-and-half of 620.
        let jittered_sum: u64 = jittered.iter().sum();
        assert!(
            (1_550..=4_650).contains(&jittered_sum),
            "seeded total {jittered_sum} drifts from lockstep 3100"
        );

        // Edge gates: zero attempt, base and cap all wait nothing.
        assert_eq!(backoff_jittered_ms(100, 10_000, 0, 7), 0);
        assert_eq!(backoff_jittered_ms(0, 10_000, 3, 7), 0);
        assert_eq!(backoff_jittered_ms(100, 0, 3, 7), 0);
        // Policy helper matches the free function.
        let policy = FaultPolicy::new(
            EventCode::NET_CLIENT_DESYNC,
            HealKind::AiTakeover,
            4,
            100,
            1_600,
            1,
        );
        assert_eq!(
            policy.jittered_backoff(3, SPLITMIX_JITTER_SEED),
            backoff_jittered_ms(100, 1_600, 3, SPLITMIX_JITTER_SEED)
        );

        // Total-delay budget: attempt and elapsed gates plus the donor
        // cumulative stop (elapsed + next > total refuses).
        assert!(backoff_within_budget(2, 3, 100, 1_000));
        assert!(!backoff_within_budget(4, 3, 100, 1_000));
        assert!(!backoff_within_budget(2, 3, 2_001, 2_000));
        let supervisor = Supervisor::new();
        let advised = supervisor
            .jittered_backoff_for(
                EventCode::NET_CLIENT_DESYNC,
                1,
                SPLITMIX_JITTER_SEED,
                0,
                10_000,
            )
            .expect("within budget must advise");
        assert!(
            (50..=150).contains(&advised),
            "desync attempt 1 jittered {advised} outside [50, 150]"
        );
        assert_eq!(
            supervisor.jittered_backoff_for(EventCode::NET_CLIENT_DESYNC, 9, 7, 0, 10_000),
            None,
            "past max_times must refuse"
        );
        assert_eq!(
            supervisor.jittered_backoff_for(EventCode::NET_CLIENT_DESYNC, 1, 7, 9_999, 10_000),
            None,
            "cumulative over total must refuse"
        );
        assert_eq!(
            supervisor.jittered_backoff_for(EventCode::PROVIDER_TIMEOUT, 1, 7, 0, 0),
            None,
            "zero total must refuse any wait"
        );
    }

    /// Chaos sweep: every catalog fault heals within budget, sim faults
    /// defer to the boundary and land in the tape with both ticks.
    #[test]
    fn every_catalog_fault_heals_within_budget() {
        for policy in catalog() {
            let mut supervisor = Supervisor::new();
            let mut events = ring(32);
            let decision = supervisor
                .report_fault(policy.code, 10, &mut events)
                .expect("catalog fault must report");
            assert_eq!(decision.kind, policy.kind, "code {:?}", policy.code);
            assert_eq!(decision.attempt, 1);
            assert!(!decision.is_quarantine());
            assert_eq!(decision.deferred, applies_at_tick(policy.code));
            // Planning record is in the ring with the attempt count.
            assert_eq!(events.query(Some(EventCode::HEAL_APPLIED)).len(), 1);
            assert!(events.query(Some(EventCode::HEAL_FAILED)).is_empty());
            // Escalation tracks level 0 with the first-attempt backoff.
            let escalation = supervisor
                .escalation_of(policy.code)
                .expect("report must escalate");
            assert_eq!(escalation.level, 0);
            assert_eq!(
                escalation.backoff_ms,
                backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, 1)
            );
            if decision.deferred {
                assert_eq!(supervisor.pending_len(), 1);
                assert_eq!(decision.applies_at, 11);
                // Nothing applies before the boundary.
                assert!(supervisor.apply_tick_boundary(10).is_empty());
                assert_eq!(supervisor.tape()[0].applied_tick, None);
                let applied = supervisor.apply_tick_boundary(11);
                assert_eq!(applied.len(), 1);
                assert_eq!(applied[0].fault_tick, 10);
                assert_eq!(applied[0].applied_tick, 11);
                assert_eq!(applied[0].kind, policy.kind);
                assert!(supervisor.pending().is_empty());
                assert_eq!(supervisor.tape()[0].applied_tick, Some(11));
            } else {
                assert_eq!(decision.applies_at, 10);
                assert!(supervisor.pending().is_empty());
                assert_eq!(supervisor.tape()[0].applied_tick, Some(10));
            }
            assert_eq!(supervisor.tape_len(), 1);
            assert!(!supervisor.tape()[0].quarantined);
        }
    }

    /// Chaos sweep: every catalog fault quarantines loudly past its budget
    /// with a `heal.failed` record and a quarantined tape row.
    #[test]
    fn every_catalog_fault_quarantines_past_budget() {
        for policy in catalog() {
            let mut supervisor = Supervisor::new();
            let mut events = ring(64);
            let mut quarantined = None;
            for tick in 0..=policy.attempts_max {
                let tick = u64::from(tick) + 100;
                let decision = supervisor
                    .report_fault(policy.code, tick, &mut events)
                    .expect("report must return advice");
                // Sim heals still drain at their boundaries while healing.
                let _ = supervisor.apply_tick_boundary(tick + 1);
                if decision.is_quarantine() {
                    quarantined = Some((decision, tick));
                    break;
                }
                assert_eq!(decision.kind, policy.kind);
            }
            let (decision, tick) = quarantined.expect("fault must quarantine");
            assert_eq!(decision.kind, HealKind::Quarantine);
            assert_eq!(decision.attempt, policy.attempts_max + 1);
            assert!(supervisor.is_quarantined(policy.code));
            assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), 1);
            let row = supervisor
                .tape()
                .iter()
                .rfind(|row| row.code == policy.code)
                .expect("quarantine must be taped");
            assert!(row.quarantined);
            assert_eq!(row.applied_tick, Some(tick));
            assert_eq!(row.kind, HealKind::Quarantine);
        }
    }

    #[test]
    fn escalation_backoff_grows_per_attempt() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let policy = supervisor
            .policy(EventCode::NET_CLIENT_DESYNC)
            .expect("catalog must hold desync");
        let mut backoffs = Vec::new();
        for tick in 0..3_u64 {
            let decision = supervisor
                .report_fault(EventCode::NET_CLIENT_DESYNC, tick, &mut events)
                .expect("within budget");
            assert!(!decision.is_quarantine());
            assert_eq!(decision.kind, HealKind::AiTakeover);
            backoffs.push(decision.backoff_ms);
            let _ = supervisor.apply_tick_boundary(tick + 1);
        }
        assert_eq!(
            backoffs,
            vec![
                backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, 1),
                backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, 2),
                backoff_ms(policy.backoff_base_ms, policy.backoff_cap_ms, 3),
            ]
        );
        assert_eq!(backoffs, vec![100, 200, 400]);
        let escalation = supervisor
            .escalation_of(EventCode::NET_CLIENT_DESYNC)
            .expect("escalation tracked");
        assert_eq!(escalation.level, 2);
        assert_eq!(escalation.not_before_tick, 2 + policy.retry_after_ticks);
    }

    #[test]
    fn same_fault_sequence_twice_gives_same_tape() {
        fn sequence() -> Vec<HealTapeRecord> {
            let mut supervisor = Supervisor::new();
            let mut events = ring(64);
            let faults = [
                EventCode::PHYSICS_BODY_NONFINITE,
                EventCode::PROVIDER_TIMEOUT,
                EventCode::NET_CLIENT_DESYNC,
                EventCode::RENDER_FRAME_OVER_BUDGET,
            ];
            for (tick, code) in faults.into_iter().enumerate() {
                let tick = tick as u64;
                supervisor
                    .report_fault(code, tick, &mut events)
                    .expect("sequence fault must report");
                let _ = supervisor.apply_tick_boundary(tick + 1);
            }
            supervisor.tape().to_vec()
        }
        assert_eq!(sequence(), sequence());
    }

    #[test]
    fn unknown_and_unregistered_codes_rejected_without_budget() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(8);
        assert_eq!(
            supervisor.report_raw(9999, 0, &mut events),
            Err(HealError::UnknownCode(9999))
        );
        assert!(events.is_empty());
        assert!(supervisor.tape().is_empty());
        // A known code the supervisor never loaded is rejected, not healed.
        let mut bare = Supervisor {
            registry: Registry::new(),
            policies: BTreeMap::new(),
            pending: VecDeque::new(),
            tape: Vec::new(),
            escalation: BTreeMap::new(),
            quarantine: BTreeMap::new(),
            breakers: BTreeMap::new(),
            hedges: BTreeMap::new(),
            events: RingBuffer::new(SUPERVISOR_EVENT_CAP),
            heal_cap: HealCap::new(default_heal_cap_max(), HEAL_CAP_WINDOW_TICKS),
            frozen_frames: 0,
        };
        assert_eq!(
            bare.report_fault(EventCode::PROVIDER_TIMEOUT, 0, &mut events),
            Err(HealError::NotRegistered(EventCode::PROVIDER_TIMEOUT))
        );
        assert!(events.is_empty());
        let err = HealError::NotRegistered(EventCode::PROVIDER_TIMEOUT);
        assert_eq!(err.to_string(), "no heal registered for: provider.timeout");
        assert_eq!(
            HealError::PendingFull.to_string(),
            "tick-boundary heal queue is full"
        );
        assert_eq!(HealError::TapeFull.to_string(), "heal tape is full");
    }

    #[test]
    fn pending_full_errors_before_consuming_budget() {
        let mut supervisor = Supervisor::new();
        supervisor.register_policy(FaultPolicy::new(
            EventCode::PHYSICS_BODY_NONFINITE,
            HealKind::Restore,
            10_000,
            0,
            0,
            1,
        ));
        let mut events = RingBuffer::new(4096);
        for tick in 0..MAX_PENDING_HEALS as u64 {
            supervisor
                .report_fault(EventCode::PHYSICS_BODY_NONFINITE, tick, &mut events)
                .expect("queue has room");
        }
        assert_eq!(supervisor.pending_len(), MAX_PENDING_HEALS);
        assert_eq!(
            supervisor.report_fault(
                EventCode::PHYSICS_BODY_NONFINITE,
                MAX_PENDING_HEALS as u64,
                &mut events
            ),
            Err(HealError::PendingFull)
        );
        // The rejected report consumed no attempt.
        assert_eq!(
            supervisor.attempts(EventCode::PHYSICS_BODY_NONFINITE),
            MAX_PENDING_HEALS as u32
        );
        // Draining makes room again.
        let applied = supervisor.apply_tick_boundary(u64::MAX);
        assert_eq!(applied.len(), MAX_PENDING_HEALS);
        assert!(supervisor.pending().is_empty());
    }

    #[test]
    fn reset_clears_budget_and_escalation_but_keeps_tape() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(16);
        supervisor
            .report_fault(EventCode::PROVIDER_TIMEOUT, 1, &mut events)
            .expect("first fault heals");
        supervisor
            .report_fault(EventCode::PROVIDER_TIMEOUT, 2, &mut events)
            .expect("second fault heals");
        supervisor
            .report_fault(EventCode::PROVIDER_TIMEOUT, 3, &mut events)
            .expect("over-budget fault quarantines");
        assert!(supervisor.is_quarantined(EventCode::PROVIDER_TIMEOUT));
        assert!(supervisor
            .escalation_of(EventCode::PROVIDER_TIMEOUT)
            .is_some());

        supervisor.reset_unverified(EventCode::PROVIDER_TIMEOUT);
        assert_eq!(supervisor.attempts(EventCode::PROVIDER_TIMEOUT), 0);
        assert!(!supervisor.is_quarantined(EventCode::PROVIDER_TIMEOUT));
        assert_eq!(supervisor.escalation_of(EventCode::PROVIDER_TIMEOUT), None);
        // History stays: three rows taped.
        assert_eq!(supervisor.tape_len(), 3);
        // The unproven reset is loud: it lands in the supervisor events.
        assert!(supervisor
            .events()
            .query(Some(EventCode::HEAL_FAILED))
            .iter()
            .any(|event| event.message.contains("reset without proof")));

        let healed = supervisor
            .report_fault(EventCode::PROVIDER_TIMEOUT, 4, &mut events)
            .expect("fresh budget heals again");
        assert_eq!(healed.attempt, 1);
        assert_eq!(healed.kind, HealKind::Degrade);
    }

    #[test]
    fn supervisor_starts_with_catalog_and_default() {
        let supervisor = Supervisor::new();
        assert_eq!(supervisor.len(), 12);
        assert!(!supervisor.is_empty());
        assert!(supervisor.tape().is_empty());
        assert!(supervisor.pending().is_empty());
        let via_default = Supervisor::default();
        assert_eq!(via_default.len(), 12);
        // Registry advice and supervisor advice agree on the boundary.
        let decision = HealDecision {
            kind: HealKind::Restore,
            attempt: 1,
            applies_at_tick: true,
        };
        assert!(decision.applies_at_tick);
        assert!(!decision.is_quarantine());
    }

    #[test]
    fn quarantine_cools_down_then_single_probe_readmits() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::SAVE_CORRUPT;
        let first = supervisor.report_fault(code, 100, &mut events).unwrap();
        assert_eq!(first.attempt, 1);
        assert_eq!(first.kind, HealKind::LastGood);
        assert!(!first.is_quarantine());
        let second = supervisor.report_fault(code, 101, &mut events).unwrap();
        assert!(second.is_quarantine());
        assert_eq!(second.kind, HealKind::Quarantine);
        assert_eq!(second.attempt, 2);
        let until = 101 + QUARANTINE_COOLDOWN_TICKS;
        assert_eq!(supervisor.quarantined_until(code), Some(until));
        assert!(supervisor.is_quarantined(code));
        assert!(!supervisor.is_half_open(code));
        let escalation = supervisor.escalation_of(code).unwrap();
        assert_eq!(escalation.not_before_tick, until);
        let tape_len = supervisor.tape_len();
        assert_eq!(tape_len, 2);
        let failed = events.query(Some(EventCode::HEAL_FAILED)).len();
        assert_eq!(failed, 1);
        let fast = supervisor.report_fault(code, 102, &mut events).unwrap();
        assert!(fast.is_quarantine());
        assert_eq!(fast.kind, HealKind::Quarantine);
        assert_eq!(fast.attempt, 2);
        assert_eq!(supervisor.quarantined_until(code), Some(until));
        let escalation_fast = supervisor.escalation_of(code).unwrap();
        assert_eq!(escalation_fast.not_before_tick, until);
        assert_eq!(supervisor.tape_len(), tape_len);
        assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), failed);
        assert!(supervisor.is_quarantined(code));
        assert!(!supervisor.is_half_open(code));
        let probe = supervisor.report_fault(code, until, &mut events).unwrap();
        assert!(!probe.is_quarantine());
        assert_eq!(probe.attempt, 1);
        assert_eq!(probe.kind, HealKind::LastGood);
        assert!(supervisor.is_half_open(code));
        assert!(!supervisor.is_quarantined(code));
    }

    #[test]
    fn breaker_open_rejects_retry_without_budget() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::SAVE_CORRUPT;
        supervisor.enable_breaker(code).unwrap();
        // Five errors then five successes: 50% over a full window opens it.
        for ok in [
            false, false, false, false, false, true, true, true, true, true,
        ] {
            supervisor.record_attempt_outcome(code, ok);
        }
        assert!(supervisor.breaker_is_open(code));
        assert_eq!(supervisor.breaker_opened_at(code), Some(10));
        let attempts = supervisor.attempts(code);
        let tape_len = supervisor.tape_len();
        // Open: the retry stops with a quarantine decision and writes nothing.
        let rejected = supervisor.report_fault(code, 200, &mut events).unwrap();
        assert!(rejected.is_quarantine());
        assert_eq!(rejected.kind, HealKind::Quarantine);
        assert_eq!(supervisor.attempts(code), attempts);
        assert_eq!(supervisor.tape_len(), tape_len);
        assert!(events.query(Some(EventCode::HEAL_FAILED)).is_empty());
        // One probe is admitted per half-open period; its success closes.
        assert!(supervisor.breaker_probe(code));
        assert!(supervisor.breaker_is_half_open(code));
        assert!(!supervisor.breaker_probe(code));
        supervisor.record_attempt_outcome(code, true);
        assert!(!supervisor.breaker_is_open(code));
        assert!(!supervisor.breaker_is_half_open(code));
        let retry = supervisor.report_fault(code, 201, &mut events).unwrap();
        assert!(!retry.is_quarantine());
        assert_eq!(retry.kind, HealKind::LastGood);
    }

    #[test]
    fn breaker_is_opt_in_per_code() {
        let mut supervisor = Supervisor::new();
        let code = EventCode::SAVE_CORRUPT;
        // No breaker: outcomes are ignored and the probe is a pass-through.
        for _ in 0..10 {
            supervisor.record_attempt_outcome(code, false);
        }
        assert!(!supervisor.breaker_is_open(code));
        assert_eq!(supervisor.breaker_opened_at(code), None);
        assert!(supervisor.breaker_probe(code));
    }

    #[test]
    fn failed_half_open_probe_reopens_at_once() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::SAVE_CORRUPT;
        supervisor.report_fault(code, 100, &mut events).unwrap();
        supervisor.report_fault(code, 101, &mut events).unwrap();
        let until = 101 + QUARANTINE_COOLDOWN_TICKS;
        let probe = supervisor.report_fault(code, until, &mut events).unwrap();
        assert_eq!(probe.attempt, 1);
        assert!(supervisor.is_half_open(code));
        let reopen = supervisor
            .report_fault(code, until + 1, &mut events)
            .unwrap();
        assert!(reopen.is_quarantine());
        assert_eq!(reopen.kind, HealKind::Quarantine);
        let until2 = until + 1 + QUARANTINE_COOLDOWN_TICKS;
        assert_eq!(supervisor.quarantined_until(code), Some(until2));
        assert!(supervisor.is_quarantined(code));
        assert!(!supervisor.is_half_open(code));
        assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), 2);
        supervisor.reset_unverified(code);
        assert!(!supervisor.is_quarantined(code));
        assert!(!supervisor.is_half_open(code));
        assert_eq!(supervisor.quarantined_until(code), None);
        assert_eq!(supervisor.attempts(code), 0);
        let healed = supervisor
            .report_fault(code, until2 + 1, &mut events)
            .unwrap();
        assert_eq!(healed.attempt, 1);
        assert!(!healed.is_quarantine());
        assert_eq!(healed.kind, HealKind::LastGood);
    }

    #[test]
    fn late_fault_hedges_retry_within_budget() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PROVIDER_TIMEOUT;
        supervisor.enable_hedge(code, 15, 100).unwrap();
        // 20 ms is past the 15 ms hedge threshold: the retry is the backup
        // and runs at once, not after its backoff.
        let hedged = supervisor
            .report_fault_with_latency(code, 100, 20, &mut events)
            .unwrap();
        assert!(hedged.hedged);
        assert_eq!(hedged.backoff_ms, 0);
        assert_eq!(hedged.kind, HealKind::Degrade);
        assert_eq!(hedged.attempt, 1);
        assert!(!hedged.is_quarantine());
        assert_eq!(supervisor.hedge_budget(code).unwrap().backups(), 1);
        // 10 ms is the fast path: the retry keeps its normal backoff.
        let plain = supervisor
            .report_fault_with_latency(code, 101, 10, &mut events)
            .unwrap();
        assert!(!plain.hedged);
        assert!(plain.backoff_ms > 0);
        assert_eq!(supervisor.hedge_budget(code).unwrap().backups(), 1);
        // The tape keeps the instant retry at zero backoff.
        assert_eq!(supervisor.tape()[0].backoff_ms, 0);
    }

    #[test]
    fn hedge_is_opt_in_per_code() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PROVIDER_TIMEOUT;
        let plain = supervisor
            .report_fault_with_latency(code, 100, 500, &mut events)
            .unwrap();
        assert!(!plain.hedged, "no hedge row: latency changes nothing");
        assert_eq!(supervisor.hedge_budget(code), None);
    }

    #[test]
    fn hedge_budget_caps_extra_load_at_percent() {
        let mut budget = HedgeBudget::new(15, 10);
        for _ in 0..9 {
            assert!(!budget.observe(40), "9 of 10 reports allow no backup");
        }
        assert!(budget.observe(40), "the tenth report allows one backup");
        assert_eq!(budget.primaries(), 10);
        assert_eq!(budget.backups(), 1);
        for _ in 0..9 {
            assert!(!budget.observe(40), "cap already spent");
        }
        assert!(budget.observe(40), "the twentieth allows the second");
        assert_eq!(budget.backups(), 2);
    }

    /// A state heal that restores the last good digest is a proved recovery:
    /// the tape row carries both digests and the budget is cleared.
    #[test]
    fn heal_that_restores_the_digest_passes() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PHYSICS_BODY_NONFINITE;
        let decision = supervisor
            .report_fault(code, 10, &mut events)
            .expect("state fault reports");
        assert!(decision.deferred, "a state heal waits for the tick boundary");
        let applied = supervisor.apply_tick_boundary(11);
        assert_eq!(applied.len(), 1);

        let verdict = supervisor
            .verify_heal(
                code,
                1,
                &HealObservation::State {
                    before_digest: 0xABCD,
                    after_digest: 0xABCD,
                },
                &mut events,
            )
            .expect("the heal is proved");
        assert_eq!(verdict, HealValidation::Recovered);
        assert!(verdict.is_green());
        // The proof carries the budget reset.
        assert_eq!(supervisor.attempts(code), 0);

        let row = supervisor
            .tape()
            .iter()
            .rev()
            .find(|row| row.code == code)
            .expect("the tape holds the row");
        assert_eq!(row.before_digest, Some(0xABCD));
        assert_eq!(row.after_digest, Some(0xABCD));
        assert_eq!(row.validation, HealValidation::Recovered);
        assert_eq!(row.scope, HealScope::State);
        assert!(supervisor.run_health().is_green());
    }

    /// A state heal that changes the digest is a failed attempt: it spends
    /// the budget and the code is quarantined loudly.
    #[test]
    fn heal_that_changes_the_digest_fails_and_quarantines() {
        let mut supervisor = Supervisor::new();
        supervisor.register_policy(
            FaultPolicy::new(EventCode::PHYSICS_BODY_NONFINITE, HealKind::Restore, 1, 0, 0, 1)
                .with_scope(HealScope::State),
        );
        let mut events = ring(32);
        let code = EventCode::PHYSICS_BODY_NONFINITE;
        supervisor
            .report_fault(code, 10, &mut events)
            .expect("state fault reports");
        let _ = supervisor.apply_tick_boundary(11);

        let verdict = supervisor
            .verify_heal(
                code,
                1,
                &HealObservation::State {
                    before_digest: 7,
                    after_digest: 9,
                },
                &mut events,
            )
            .expect("the heal is checked");
        assert_eq!(verdict, HealValidation::NotRecovered);
        assert!(!verdict.is_green());
        assert!(supervisor.is_quarantined(code), "the budget is spent");
        assert!(supervisor.run_health().is_red());
        assert!(!events.query(Some(EventCode::HEAL_FAILED)).is_empty());
        let row = supervisor
            .tape()
            .iter()
            .find(|row| row.code == code && row.attempt == 1)
            .expect("the tape holds the failed row");
        assert_eq!(row.validation, HealValidation::NotRecovered);
    }

    /// A budget reset without a proof on file is refused and stays loud.
    #[test]
    fn reset_without_the_proof_is_refused() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PROVIDER_TIMEOUT;
        supervisor
            .report_fault(code, 5, &mut events)
            .expect("fault reports");
        assert_eq!(supervisor.attempts(code), 1);

        assert_eq!(
            supervisor.reset(code),
            ResetOutcome::Refused,
            "a heal cannot reopen its own budget"
        );
        assert_eq!(supervisor.attempts(code), 1, "the budget stays spent");
        assert!(supervisor
            .events()
            .query(Some(EventCode::HEAL_FAILED))
            .iter()
            .any(|event| event.message.contains("refused without proof")));

        let proof = VerifiedRecovery::new(code, 1, HealValidation::Recovered, 5, 5);
        assert_eq!(supervisor.reset_verified(&proof), Ok(()));
        assert_eq!(supervisor.attempts(code), 0);

        let frozen = VerifiedRecovery::new(code, 2, HealValidation::FrozenFrame, 5, 5);
        assert_eq!(
            supervisor.reset_verified(&frozen),
            Err(HealError::UnverifiedRecovery { code }),
            "a frozen frame proves nothing"
        );
    }

    /// Past the rolling heal cap the run reads red, never degraded.
    #[test]
    fn over_the_heal_cap_the_run_reads_red() {
        let mut supervisor = Supervisor::new();
        supervisor.set_heal_cap(2, HEAL_CAP_WINDOW_TICKS);
        let mut events = ring(64);
        let codes = [
            EventCode::PROVIDER_TIMEOUT,
            EventCode::ASSETS_MISSING,
            EventCode::RENDER_DEVICE_LOST,
        ];
        supervisor
            .report_fault(codes[0], 10, &mut events)
            .expect("fault reports");
        supervisor
            .report_fault(codes[1], 11, &mut events)
            .expect("fault reports");
        assert!(!supervisor.heal_cap_exceeded(), "two heals are inside the cap");
        assert_eq!(supervisor.heal_count_in_window(11), 2);

        supervisor
            .report_fault(codes[2], 12, &mut events)
            .expect("fault reports");
        assert!(supervisor.heal_cap_exceeded());
        assert_eq!(supervisor.heal_count_in_window(12), 3);
        assert_eq!(supervisor.run_health(), RunHealth::Red);
        assert!(events
            .query(Some(EventCode::HEAL_FAILED))
            .iter()
            .any(|event| event.message.contains("heal.cap")));
    }

    /// A wrong heal is rolled back to the last good snapshot; the restored
    /// digest proves the rollback.
    #[test]
    fn wrong_heal_rolls_back_and_the_restored_digest_matches() {
        let mut supervisor = Supervisor::new();
        supervisor.register_policy(
            FaultPolicy::new(EventCode::PHYSICS_BODY_NONFINITE, HealKind::Restore, 1, 0, 0, 1)
                .with_scope(HealScope::State),
        );
        let mut events = ring(32);
        let code = EventCode::PHYSICS_BODY_NONFINITE;
        supervisor
            .report_fault(code, 10, &mut events)
            .expect("state fault reports");
        let _ = supervisor.apply_tick_boundary(11);
        let verdict = supervisor
            .verify_heal(
                code,
                1,
                &HealObservation::State {
                    before_digest: 0x11,
                    after_digest: 0x22,
                },
                &mut events,
            )
            .expect("the heal is checked");
        assert_eq!(verdict, HealValidation::NotRecovered);
        assert!(supervisor.is_quarantined(code));

        let proof = RollbackProof::new(code, 1, 12, 0x11, 0x11);
        let rolled = supervisor
            .rollback_heal(&proof, &mut events)
            .expect("the rollback runs");
        assert_eq!(rolled, HealValidation::RolledBack);
        assert!(rolled.is_green());
        assert!(!supervisor.is_quarantined(code), "the state is good again");
        assert_eq!(supervisor.attempts(code), 0);
        let row = supervisor
            .tape()
            .iter()
            .rev()
            .find(|row| row.code == code)
            .expect("the tape holds the rollback row");
        assert_eq!(row.validation, HealValidation::RolledBack);
        assert_eq!(row.before_digest, Some(0x11));
        assert_eq!(row.after_digest, Some(0x11));
    }

    /// A rollback that does not restore the last good digest is not proven:
    /// the code stays quarantined.
    #[test]
    fn rollback_with_a_different_digest_does_not_prove_anything() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::RENDER_DEVICE_LOST;
        supervisor
            .report_fault(code, 10, &mut events)
            .expect("fault reports");
        let proof = RollbackProof::new(code, 1, 11, 0x11, 0x12);
        let verdict = supervisor
            .rollback_heal(&proof, &mut events)
            .expect("the rollback runs");
        assert_eq!(verdict, HealValidation::NotRecovered);
        assert!(supervisor.is_quarantined(code));
        assert!(events
            .query(Some(EventCode::HEAL_FAILED))
            .iter()
            .any(|event| event.message.contains("rollback failed")));
    }

    /// A presentation heal that only held the last good frame is a frozen
    /// frame: it never counts as green.
    #[test]
    fn presentation_heal_that_holds_the_last_good_frame_is_frozen_and_never_green() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::RENDER_DEVICE_LOST;
        supervisor
            .report_fault(code, 10, &mut events)
            .expect("fault reports");
        let verdict = supervisor
            .verify_heal(
                code,
                1,
                &HealObservation::HeldLastGood {
                    frames: 30,
                    digest: 0x55,
                },
                &mut events,
            )
            .expect("the heal is checked");
        assert_eq!(verdict, HealValidation::FrozenFrame);
        assert!(!verdict.is_green());
        assert_eq!(supervisor.frozen_frame_count(), 1);
        assert_eq!(supervisor.run_health(), RunHealth::Red);
        assert_eq!(
            supervisor.attempts(code),
            1,
            "a frozen frame is not a recovery: the budget stays"
        );
        assert!(events
            .query(Some(EventCode::HEAL_FAILED))
            .iter()
            .any(|event| event.message.contains("frozen frame")));
    }

    /// A presentation heal that presented a fresh frame is green.
    #[test]
    fn presentation_heal_that_presents_a_fresh_frame_is_green() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::RENDER_DEVICE_LOST;
        supervisor
            .report_fault(code, 10, &mut events)
            .expect("fault reports");
        let verdict = supervisor
            .verify_heal(
                code,
                1,
                &HealObservation::Presented { digest: 0x66 },
                &mut events,
            )
            .expect("the heal is checked");
        assert_eq!(verdict, HealValidation::Recovered);
        assert_eq!(supervisor.attempts(code), 0);
        assert_eq!(supervisor.frozen_frame_count(), 0);
    }

    /// Re-registering a policy row keeps the budget it already spent.
    #[test]
    fn re_registering_a_policy_keeps_the_budget() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PROVIDER_TIMEOUT;
        supervisor
            .report_fault(code, 1, &mut events)
            .expect("fault reports");
        assert_eq!(supervisor.attempts(code), 1);
        let policy = supervisor.policy(code).expect("the catalog row");
        supervisor.register_policy(policy);
        assert_eq!(
            supervisor.attempts(code),
            1,
            "new knowledge may change the policy, never the budget"
        );
    }

    /// A state heal waits for a tick boundary; a presentation heal does not.
    #[test]
    fn state_heal_waits_for_a_tick_boundary_and_presentation_heal_does_not() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let state = supervisor
            .report_fault(EventCode::NET_CLIENT_DESYNC, 10, &mut events)
            .expect("fault reports");
        assert!(state.deferred);
        assert_eq!(state.applies_at, 11);
        let presentation = supervisor
            .report_fault(EventCode::RENDER_DEVICE_LOST, 10, &mut events)
            .expect("fault reports");
        assert!(!presentation.deferred);
        assert_eq!(presentation.applies_at, 10);
    }

    /// An observation that does not match the row scope is refused.
    #[test]
    fn observation_must_match_the_row_scope() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PHYSICS_BODY_NONFINITE;
        supervisor
            .report_fault(code, 10, &mut events)
            .expect("fault reports");
        assert_eq!(
            supervisor.verify_heal(
                code,
                1,
                &HealObservation::Presented { digest: 1 },
                &mut events,
            ),
            Err(HealError::ScopeMismatch { code })
        );
    }
}
