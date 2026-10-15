//! Auto-heal registry (MAPE-K) over [`super`] observe types.
//!
//! Consumer: the chaos suite and future tick integration; the caller owns
//! event storage and timing. Data owner: [`Registry`] owns the fault-code
//! rows and per-code attempt counters; the caller owns the [`RingBuffer`]
//! that heal records are pushed into. Units: ticks (`u64`), attempt counts
//! (`u32`). Determinism scope: rows and counters live in [`BTreeMap`]s so
//! iteration order is stable, counters saturate instead of wrapping, and
//! event messages are built from inputs only (no wall clock). Error type:
//! [`HealError`]; unknown codes fail, never panic.
//!
//! The registry only advises: [`HealDecision`] carries the [`HealKind`] the
//! caller must execute plus `applies_at_tick`, which tells a simulation
//! caller to defer application to a tick boundary (the sim owns timing and
//! recording; observe never touches the sim hash). Every successful heal
//! pushes a `heal.applied` event with the attempt count; a fault past its
//! retry budget pushes `heal.failed` and advises [`HealKind::Quarantine`].

use super::{Event, EventCode, ObserveError, RingBuffer};
use std::collections::BTreeMap;
use std::fmt;

/// Bounded heal action the caller executes. This enum is the runnable advice
/// (the `HealFn` of the contract): no closures cross the API, the caller
/// matches on the value and runs the corresponding scripted recovery.
/// Together the variants cover the supervisor catalog: retry, reset,
/// last-good, degrade, reconnect, AI takeover, restore and quarantine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealKind {
    /// Retry the bounded tick-scoped work (replan, re-import, resimulate).
    RetryTick,
    /// Reset a presentation/service budget (quality level, request quota).
    ResetBudget,
    /// Drop the failing job and keep the last-good artifact or pipeline.
    DropJob,
    /// Keep serving from the last validated good state: last-good pipeline,
    /// asset bytes or save generation. Never a guess.
    LastGood,
    /// Lower presentation or service quality within a floor (fewer lights,
    /// scripted fallback content) and record the new level.
    Degrade,
    /// Reopen a lost device, surface or session with backoff; the mixer or
    /// match keeps running on the last-good path meanwhile.
    Reconnect,
    /// Hand the slot to the scripted AI policy while the client or fighter
    /// recovers; authority stays with the simulation.
    AiTakeover,
    /// Restore a validated full snapshot (authority snapshot plus
    /// resimulation, journal document, previous save generation) and never
    /// forge missing state.
    Restore,
    /// Escalate to the supervising agent or operator (fresh brief plus
    /// conflict set); still bounded by the same attempt budget.
    Escalate,
    /// Quarantine the object or task and fail loudly. Never registered as a
    /// first action by convention; returned when a fault exceeds its budget.
    Quarantine,
}

/// One registered recovery row: fault code to bounded heal action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealAction {
    /// Fault code this row heals.
    pub code: EventCode,
    /// Maximum heal attempts before the fault quarantines. Zero means the
    /// first fault fails loudly at once.
    pub attempts_max: u32,
    /// Caller-executed recovery while within budget.
    pub kind: HealKind,
}

impl HealAction {
    /// Build a recovery row for `code`.
    #[must_use]
    pub fn new(code: EventCode, attempts_max: u32, kind: HealKind) -> Self {
        Self {
            code,
            attempts_max,
            kind,
        }
    }
}

/// Advice returned for one fault occurrence. The caller executes `kind`;
/// when `applies_at_tick` is set, a simulation caller must defer that
/// execution to a tick boundary and record it in the tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealDecision {
    /// Recovery to execute, or [`HealKind::Quarantine`] past the budget.
    pub kind: HealKind,
    /// One-based fault count for this code (attempts so far, saturating).
    pub attempt: u32,
    /// True for authoritative-simulation faults: apply only at a tick
    /// boundary from the same inputs on every machine.
    pub applies_at_tick: bool,
}

impl HealDecision {
    /// True when the budget is spent and the caller must quarantine the
    /// object or task and fail loudly.
    #[must_use]
    pub fn is_quarantine(self) -> bool {
        self.kind == HealKind::Quarantine
    }
}

/// Typed failure for heal registration and fault reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealError {
    /// Raw code is not in the stable [`EventCode`] registry.
    UnknownCode(u32),
    /// Code is known but has no registered heal row.
    NotRegistered(EventCode),
    /// The tick-boundary queue is full; the caller must drain it with the
    /// supervisor before reporting more simulation faults.
    PendingFull,
    /// The heal tape is full; the caller must export and clear it before
    /// reporting more faults. History is never dropped silently.
    TapeFull,
}

impl fmt::Display for HealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCode(code) => write!(f, "unknown event code: {code}"),
            Self::NotRegistered(code) => {
                let label = code.name().unwrap_or("unknown");
                write!(f, "no heal registered for: {label}")
            }
            Self::PendingFull => write!(f, "tick-boundary heal queue is full"),
            Self::TapeFull => write!(f, "heal tape is full"),
        }
    }
}

impl std::error::Error for HealError {}

impl From<ObserveError> for HealError {
    fn from(err: ObserveError) -> Self {
        match err {
            ObserveError::UnknownCode(code) => Self::UnknownCode(code),
        }
    }
}

/// Advise whether a fault's heal must wait for a tick boundary.
///
/// Authoritative-simulation faults (non-finite physics state, stuck fighter
/// brain, desynced net client) resimulate or replan from shared snapshots,
/// so they apply only at tick boundaries from identical inputs.
/// Presentation and service heals (render, assets, provider, commands,
/// editor, saves) may apply immediately; the caller still owns timing.
#[must_use]
pub fn applies_at_tick(code: EventCode) -> bool {
    code == EventCode::PHYSICS_BODY_NONFINITE
        || code == EventCode::BRAIN_FIGHTER_STUCK
        || code == EventCode::NET_CLIENT_DESYNC
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    action: HealAction,
    attempts: u32,
}

/// MAPE-K knowledge base: fault codes to bounded heal actions with
/// per-code attempt counters, escalation and quarantine.
///
/// Monitor/Analyze/Plan/Execute all stay with the caller: the registry
/// plans (which kind, which attempt, tick-boundary or not) and records
/// (`heal.applied` / `heal.failed` into the caller's ring); the caller
/// executes and owns authoritative timing.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    rows: BTreeMap<u32, Row>,
}

impl Registry {
    /// Empty registry with no heal rows.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
        }
    }

    /// Register (or replace) the heal row for `code`. Re-registering resets
    /// that code's attempt counter to zero: new knowledge, fresh budget.
    pub fn register(&mut self, code: EventCode, attempts_max: u32, kind: HealKind) {
        self.rows.insert(
            code.raw(),
            Row {
                action: HealAction::new(code, attempts_max, kind),
                attempts: 0,
            },
        );
    }

    /// Register from a raw code, rejecting unregistered values.
    ///
    /// # Errors
    ///
    /// Returns [`ObserveError::UnknownCode`] when `raw` is not registered.
    pub fn register_raw(
        &mut self,
        raw: u32,
        attempts_max: u32,
        kind: HealKind,
    ) -> Result<(), ObserveError> {
        let code = EventCode::from_raw(raw)?;
        self.register(code, attempts_max, kind);
        Ok(())
    }

    /// True when `code` has a heal row.
    #[must_use]
    pub fn registered(&self, code: EventCode) -> bool {
        self.rows.contains_key(&code.raw())
    }

    /// Copy of the heal row for `code`, if any.
    #[must_use]
    pub fn get(&self, code: EventCode) -> Option<HealAction> {
        self.rows.get(&code.raw()).map(|row| row.action)
    }

    /// Number of registered heal rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no heal rows are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Faults seen so far for `code` (zero when unregistered).
    #[must_use]
    pub fn attempts(&self, code: EventCode) -> u32 {
        self.rows.get(&code.raw()).map_or(0, |row| row.attempts)
    }

    /// True once `code` has spent its budget (attempts past `attempts_max`).
    /// Unregistered codes are never quarantined by this registry.
    #[must_use]
    pub fn is_quarantined(&self, code: EventCode) -> bool {
        self.rows
            .get(&code.raw())
            .is_some_and(|row| row.attempts > row.action.attempts_max)
    }

    /// Clear the attempt counter for `code` after a verified recovery, so a
    /// later unrelated fault starts from a fresh budget. No-op when `code`
    /// has no row.
    pub fn reset(&mut self, code: EventCode) {
        if let Some(row) = self.rows.get_mut(&code.raw()) {
            row.attempts = 0;
        }
    }

    /// Report one fault occurrence at `tick`, pushing the `heal.applied` or
    /// `heal.failed` record into the caller's `ring` and returning the
    /// advice the caller must execute.
    ///
    /// Within budget the decision echoes the registered kind with the
    /// one-based attempt count. Past the budget the decision is
    /// [`HealKind::Quarantine`] with a `heal.failed` record: quarantine the
    /// object or task and fail loudly, never heal silently.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::NotRegistered`] when `code` has no heal row.
    pub fn on_fault(
        &mut self,
        code: EventCode,
        tick: u64,
        ring: &mut RingBuffer,
    ) -> Result<HealDecision, HealError> {
        let row = self
            .rows
            .get_mut(&code.raw())
            .ok_or(HealError::NotRegistered(code))?;
        row.attempts = row.attempts.saturating_add(1);
        let attempt = row.attempts;
        let attempts_max = row.action.attempts_max;
        let registered_kind = row.action.kind;
        let label = code.name().unwrap_or("unknown");
        let boundary = applies_at_tick(code);
        if attempt <= attempts_max {
            ring.push(Event::new(
                EventCode::HEAL_APPLIED,
                tick,
                &format!("heal.applied cause={label} attempt={attempt}/{attempts_max} kind={registered_kind:?}"),
            ));
            Ok(HealDecision {
                kind: registered_kind,
                attempt,
                applies_at_tick: boundary,
            })
        } else {
            ring.push(Event::new(
                EventCode::HEAL_FAILED,
                tick,
                &format!("heal.failed cause={label} attempt={attempt}/{attempts_max} quarantined"),
            ));
            Ok(HealDecision {
                kind: HealKind::Quarantine,
                attempt,
                applies_at_tick: boundary,
            })
        }
    }

    /// Report one fault from a raw code, rejecting unregistered values
    /// before touching any budget counter.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::UnknownCode`] for unregistered codes and
    /// [`HealError::NotRegistered`] for known codes with no heal row.
    pub fn on_fault_raw(
        &mut self,
        raw: u32,
        tick: u64,
        ring: &mut RingBuffer,
    ) -> Result<HealDecision, HealError> {
        let code = EventCode::from_raw(raw)?;
        self.on_fault(code, tick, ring)
    }
}

#[cfg(test)]
mod tests {
    use super::super::digest;
    use super::*;

    fn ring(cap: usize) -> RingBuffer {
        RingBuffer::new(cap)
    }

    #[test]
    fn registered_fault_heals_and_records_applied() {
        let mut registry = Registry::new();
        let mut events = ring(16);
        registry.register(EventCode::PHYSICS_BODY_NONFINITE, 3, HealKind::RetryTick);

        let first = registry
            .on_fault(EventCode::PHYSICS_BODY_NONFINITE, 10, &mut events)
            .expect("registered fault must heal");
        assert_eq!(first.kind, HealKind::RetryTick);
        assert_eq!(first.attempt, 1);
        assert!(first.applies_at_tick);
        assert!(!first.is_quarantine());

        let second = registry
            .on_fault(EventCode::PHYSICS_BODY_NONFINITE, 11, &mut events)
            .expect("second fault within budget must heal");
        assert_eq!(second.attempt, 2);
        assert_eq!(registry.attempts(EventCode::PHYSICS_BODY_NONFINITE), 2);

        // Success path records heal.applied with the attempt count.
        let applied = events.query(Some(EventCode::HEAL_APPLIED));
        assert_eq!(applied.len(), 2);
        assert!(applied[0].message.contains("attempt=1/3"));
        assert!(applied[1].message.contains("attempt=2/3"));
        assert!(applied[0].message.contains("physics.body.nonfinite"));
        assert!(events.query(Some(EventCode::HEAL_FAILED)).is_empty());
    }

    #[test]
    fn over_budget_fault_quarantines_with_failed_event() {
        let mut registry = Registry::new();
        let mut events = ring(16);
        registry.register(EventCode::ASSETS_MISSING, 1, HealKind::DropJob);

        let healed = registry
            .on_fault(EventCode::ASSETS_MISSING, 4, &mut events)
            .expect("first fault within budget must heal");
        assert_eq!(healed.kind, HealKind::DropJob);
        assert!(!registry.is_quarantined(EventCode::ASSETS_MISSING));

        let quarantined = registry
            .on_fault(EventCode::ASSETS_MISSING, 5, &mut events)
            .expect("over-budget fault must return a decision, not an error");
        assert_eq!(quarantined.kind, HealKind::Quarantine);
        assert_eq!(quarantined.attempt, 2);
        assert!(quarantined.is_quarantine());
        assert!(registry.is_quarantined(EventCode::ASSETS_MISSING));

        let failed = events.query(Some(EventCode::HEAL_FAILED));
        assert_eq!(failed.len(), 1);
        assert!(failed[0].message.contains("attempt=2/1"));
        assert!(failed[0].message.contains("quarantined"));
    }

    #[test]
    fn unknown_code_rejected() {
        let mut registry = Registry::new();
        let mut events = ring(8);

        // Raw registration rejects codes outside the stable registry.
        assert_eq!(
            registry.register_raw(9999, 3, HealKind::RetryTick),
            Err(ObserveError::UnknownCode(9999))
        );
        // Raw fault reports reject before touching any counter.
        assert_eq!(
            registry.on_fault_raw(9999, 0, &mut events),
            Err(HealError::UnknownCode(9999))
        );
        assert!(events.is_empty());

        // A known code with no heal row is rejected, not silently healed.
        assert_eq!(
            registry.on_fault(EventCode::PROVIDER_TIMEOUT, 0, &mut events),
            Err(HealError::NotRegistered(EventCode::PROVIDER_TIMEOUT))
        );
        assert_eq!(
            registry.on_fault_raw(EventCode::PROVIDER_TIMEOUT.raw(), 0, &mut events),
            Err(HealError::NotRegistered(EventCode::PROVIDER_TIMEOUT))
        );
        assert!(events.is_empty());

        let err = HealError::NotRegistered(EventCode::PROVIDER_TIMEOUT);
        assert_eq!(err.to_string(), "no heal registered for: provider.timeout");
        assert_eq!(
            HealError::UnknownCode(7_777).to_string(),
            "unknown event code: 7777"
        );
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn escalation_row_returns_escalate() {
        let mut registry = Registry::new();
        let mut events = ring(8);
        // Stale AI edit: return a fresh brief plus conflict set to the agent.
        registry.register(EventCode::COMMANDS_STALE, 2, HealKind::Escalate);

        let decision = registry
            .on_fault(EventCode::COMMANDS_STALE, 20, &mut events)
            .expect("escalation row must advise escalation");
        assert_eq!(decision.kind, HealKind::Escalate);
        assert_eq!(decision.attempt, 1);
        assert!(!decision.applies_at_tick);
        assert!(!decision.is_quarantine());

        let applied = events.query(Some(EventCode::HEAL_APPLIED));
        assert_eq!(applied.len(), 1);
        assert!(applied[0].message.contains("Escalate"));
    }

    #[test]
    fn retry_tick_row_heals_provider_timeout() {
        let mut registry = Registry::new();
        let mut events = ring(8);
        // Provider timeout: scripted fallback with a bounded retry.
        registry.register(EventCode::PROVIDER_TIMEOUT, 2, HealKind::RetryTick);

        let decision = registry
            .on_fault(EventCode::PROVIDER_TIMEOUT, 30, &mut events)
            .expect("provider row must heal");
        assert_eq!(decision.kind, HealKind::RetryTick);
        assert!(!decision.applies_at_tick);
        assert_eq!(events.query(Some(EventCode::HEAL_APPLIED)).len(), 1);
    }

    #[test]
    fn reset_budget_row_heals_frame_over_budget() {
        let mut registry = Registry::new();
        let mut events = ring(8);
        // Frame over budget: lower presentation quality within floor.
        registry.register(
            EventCode::RENDER_FRAME_OVER_BUDGET,
            2,
            HealKind::ResetBudget,
        );

        let decision = registry
            .on_fault(EventCode::RENDER_FRAME_OVER_BUDGET, 40, &mut events)
            .expect("frame-budget row must heal");
        assert_eq!(decision.kind, HealKind::ResetBudget);
        assert!(!decision.applies_at_tick);
        assert_eq!(events.query(Some(EventCode::HEAL_APPLIED)).len(), 1);
    }

    #[test]
    fn drop_job_row_heals_pipeline_failure() {
        let mut registry = Registry::new();
        let mut events = ring(8);
        // Shader or pipeline build fails: keep last good, drop the job.
        registry.register(EventCode::RENDER_PIPELINE_FAILED, 2, HealKind::DropJob);

        let decision = registry
            .on_fault(EventCode::RENDER_PIPELINE_FAILED, 50, &mut events)
            .expect("pipeline row must heal");
        assert_eq!(decision.kind, HealKind::DropJob);
        assert!(!decision.applies_at_tick);
        assert_eq!(events.query(Some(EventCode::HEAL_APPLIED)).len(), 1);
    }

    #[test]
    fn zero_budget_quarantines_first_fault() {
        let mut registry = Registry::new();
        let mut events = ring(8);
        // Corrupt save: preserve the original, never forge; no silent retry.
        registry.register(EventCode::SAVE_CORRUPT, 0, HealKind::RetryTick);

        let decision = registry
            .on_fault(EventCode::SAVE_CORRUPT, 60, &mut events)
            .expect("zero-budget fault must still return a decision");
        assert!(decision.is_quarantine());
        assert_eq!(decision.attempt, 1);
        assert!(registry.is_quarantined(EventCode::SAVE_CORRUPT));
        assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), 1);
        assert!(events.query(Some(EventCode::HEAL_APPLIED)).is_empty());
    }

    #[test]
    fn sim_faults_advise_tick_boundary() {
        // Authoritative simulation heals wait for a tick boundary.
        assert!(applies_at_tick(EventCode::PHYSICS_BODY_NONFINITE));
        assert!(applies_at_tick(EventCode::BRAIN_FIGHTER_STUCK));
        assert!(applies_at_tick(EventCode::NET_CLIENT_DESYNC));
        // Presentation and service heals may apply immediately.
        assert!(!applies_at_tick(EventCode::RENDER_DEVICE_LOST));
        assert!(!applies_at_tick(EventCode::RENDER_FRAME_OVER_BUDGET));
        assert!(!applies_at_tick(EventCode::PROVIDER_TIMEOUT));
        assert!(!applies_at_tick(EventCode::COMMANDS_STALE));
        assert!(!applies_at_tick(EventCode::EDITOR_CRASH));
    }

    #[test]
    fn reset_clears_budget_after_verified_recovery() {
        let mut registry = Registry::new();
        let mut events = ring(8);
        registry.register(EventCode::BRAIN_FIGHTER_STUCK, 1, HealKind::RetryTick);
        registry
            .on_fault(EventCode::BRAIN_FIGHTER_STUCK, 1, &mut events)
            .expect("first fault heals");
        registry
            .on_fault(EventCode::BRAIN_FIGHTER_STUCK, 2, &mut events)
            .expect("over-budget fault quarantines");
        assert!(registry.is_quarantined(EventCode::BRAIN_FIGHTER_STUCK));

        registry.reset(EventCode::BRAIN_FIGHTER_STUCK);
        assert_eq!(registry.attempts(EventCode::BRAIN_FIGHTER_STUCK), 0);
        assert!(!registry.is_quarantined(EventCode::BRAIN_FIGHTER_STUCK));
        let healed = registry
            .on_fault(EventCode::BRAIN_FIGHTER_STUCK, 3, &mut events)
            .expect("fresh budget heals again");
        assert_eq!(healed.attempt, 1);
        assert_eq!(healed.kind, HealKind::RetryTick);
    }

    #[test]
    fn digest_bounded_after_heal_traffic() {
        let mut registry = Registry::new();
        let mut events = ring(64);
        registry.register(EventCode::PHYSICS_BODY_NONFINITE, 2, HealKind::RetryTick);
        registry.register(EventCode::PROVIDER_TIMEOUT, 1, HealKind::RetryTick);
        registry.register(EventCode::COMMANDS_STALE, 1, HealKind::Escalate);
        registry.register(EventCode::ASSETS_MISSING, 0, HealKind::DropJob);

        for tick in 0..20_u64 {
            let code = match tick % 4 {
                0 => EventCode::PHYSICS_BODY_NONFINITE,
                1 => EventCode::PROVIDER_TIMEOUT,
                2 => EventCode::COMMANDS_STALE,
                _ => EventCode::ASSETS_MISSING,
            };
            events.push(Event::new(code, tick, "fault injected"));
            let _ = registry.on_fault(code, tick, &mut events);
        }

        let snapshot: Vec<Event> = events.iter().cloned().collect();
        for budget in [0_usize, 1, 16, 64, 256, 1024, 4096] {
            let out = digest(&snapshot, budget);
            assert!(
                out.len() <= budget,
                "digest len {} exceeds budget {budget}",
                out.len()
            );
        }
        let full = digest(&snapshot, 4096);
        assert!(full.contains("heal.applied"));
        assert!(full.contains("heal.failed"));
    }
}
