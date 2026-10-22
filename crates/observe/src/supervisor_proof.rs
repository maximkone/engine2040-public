//! The heal proof rules of the [`Supervisor`]: the recovery proof, the
//! budget reset gate, the rollback and the rolling heal cap.
//!
//! Consumer: the supervisor and the subsystems that report what their heal
//! did. Data owner: [`Supervisor`] owns the tape, the budgets and the counts;
//! this module owns the rules. Units: ticks (`u64`), digests (`u64`), counts
//! (`u32`). Determinism: integer arithmetic only, no clock, no float.
//! Errors: [`crate::heal::HealError`]; a heal without a proof is refused.

use crate::heal::HealError;
use crate::recovery::{
    HealCap, HealObservation, HealScope, HealValidation, ResetOutcome, RollbackProof, RunHealth,
    VerifiedRecovery,
};
use crate::supervisor::{HealTapeRecord, Supervisor};
use crate::{Event, EventCode, RingBuffer};

impl Supervisor {
    /// Clear the attempt budget and escalation for `code` after a verified
    /// recovery. Deferred heals and tape history are kept: history is
    /// evidence, not budget. The breaker and hedge budget are kept too:
    /// they bound load and retries, not health.
    ///
    /// Refused without a proof: the budget of `code` is only cleared when a
    /// verified recovery or a proved rollback is on file for it (see
    /// [`Self::verify_heal`] and [`Self::rollback_heal`]). A refusal is loud:
    /// it lands in [`Self::events`] and the budget stays spent.
    pub fn reset(&mut self, code: EventCode) -> ResetOutcome {
        if !self.has_proof(code) {
            let tick = self.last_tick(code);
            self.events.push(Event::new(
                EventCode::HEAL_FAILED,
                tick,
                &format!(
                    "heal.failed cause={} budget reset refused without proof",
                    code.name().unwrap_or("unknown")
                ),
            ));
            return ResetOutcome::Refused;
        }
        self.clear_budget(code);
        ResetOutcome::Cleared
    }

    /// Clear the budget for `code` with an explicit proof. The proof must
    /// carry a green verdict (a proved recovery or a proved rollback); a
    /// frozen frame or a failed attempt proves nothing and is refused.
    /// Deferred heals and tape history are kept.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::UnverifiedRecovery`] when the proof does not
    /// prove a recovery. The budget stays spent.
    pub fn reset_verified(&mut self, proof: &VerifiedRecovery) -> Result<(), HealError> {
        if !proof.proves_recovery() {
            let tick = self.last_tick(proof.code);
            self.events.push(Event::new(
                EventCode::HEAL_FAILED,
                tick,
                &format!(
                    "heal.failed cause={} budget reset refused: proof is {}",
                    proof.code.name().unwrap_or("unknown"),
                    proof.validation.label()
                ),
            ));
            return Err(HealError::UnverifiedRecovery { code: proof.code });
        }
        self.write_proof(proof);
        self.clear_budget(proof.code);
        Ok(())
    }

    /// Clear the budget for `code` without a proof. This is the operator
    /// escape hatch: it works, but it is loud. The event lands in
    /// [`Self::events`] and names the code.
    pub fn reset_unverified(&mut self, code: EventCode) {
        let tick = self.last_tick(code);
        self.events.push(Event::new(
            EventCode::HEAL_FAILED,
            tick,
            &format!(
                "heal.failed cause={} budget reset without proof",
                code.name().unwrap_or("unknown")
            ),
        ));
        self.clear_budget(code);
    }

    /// True when a verified recovery or a proved rollback is on file for
    /// `code`.
    #[must_use]
    pub fn has_proof(&self, code: EventCode) -> bool {
        self.tape
            .iter()
            .rev()
            .find(|row| row.code == code)
            .is_some_and(|row| row.validation.is_green())
    }

    /// Supervisor-level events: refused resets, a blown heal cap, a frozen
    /// frame. Never dropped silently.
    #[must_use]
    pub fn events(&self) -> &RingBuffer {
        &self.events
    }

    /// How the run reads. Red when the heal cap is blown, when a
    /// presentation heal only held the last good frame, or when a fault code
    /// is quarantined. Degraded while a fault is being handled. Green only
    /// when nothing is wrong.
    #[must_use]
    pub fn run_health(&self) -> RunHealth {
        if self.heal_cap.exceeded() || self.frozen_frames > 0 || self.any_quarantined() {
            return RunHealth::Red;
        }
        if !self.escalation.is_empty() {
            return RunHealth::Degraded;
        }
        RunHealth::Green
    }

    /// True when the rolling heal cap is blown: the run reads red.
    #[must_use]
    pub fn heal_cap_exceeded(&self) -> bool {
        self.heal_cap.exceeded()
    }

    /// Heals inside the rolling window that ends at `tick`.
    #[must_use]
    pub fn heal_count_in_window(&self, tick: u64) -> u32 {
        self.heal_cap.heals_in_window(tick)
    }

    /// Presentation heals that only held the last good frame. Each one is a
    /// frozen frame and never counts as green.
    #[must_use]
    pub fn frozen_frame_count(&self) -> u32 {
        self.frozen_frames
    }

    /// Set the rolling heal cap: at most `max_heals` heals inside
    /// `window_ticks` ticks. Past the cap the run reads red.
    pub fn set_heal_cap(&mut self, max_heals: u32, window_ticks: u64) {
        self.heal_cap = HealCap::new(max_heals, window_ticks);
    }

    /// Verify one applied heal. The caller reports what it observed; the
    /// supervisor decides the verdict, keeps the proof on the tape and moves
    /// the budget.
    ///
    /// A state heal is recovered when the healed state hashes to the last
    /// good state; a different digest is a failed attempt and spends the
    /// budget. A presentation heal that only held the last good frame is a
    /// frozen frame: never green, and the run reads red.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::NotRegistered`] for an unknown code,
    /// [`HealError::ScopeMismatch`] when the observation does not match the
    /// row scope, and [`HealError::UnknownHeal`] when no tape row holds that
    /// attempt.
    pub fn verify_heal(
        &mut self,
        code: EventCode,
        attempt: u32,
        observation: &HealObservation,
        ring: &mut RingBuffer,
    ) -> Result<HealValidation, HealError> {
        let policy = self.policy(code).ok_or(HealError::NotRegistered(code))?;
        let verdict = match (policy.scope, observation) {
            (
                HealScope::State,
                HealObservation::State {
                    before_digest,
                    after_digest,
                },
            ) => {
                if before_digest == after_digest {
                    HealValidation::Recovered
                } else {
                    HealValidation::NotRecovered
                }
            }
            (HealScope::Presentation, HealObservation::Presented { .. }) => {
                HealValidation::Recovered
            }
            (HealScope::Presentation, HealObservation::HeldLastGood { .. }) => {
                HealValidation::FrozenFrame
            }
            _ => return Err(HealError::ScopeMismatch { code }),
        };
        let (before, after) = match observation {
            HealObservation::State {
                before_digest,
                after_digest,
            } => (*before_digest, *after_digest),
            HealObservation::Presented { digest } => (0, *digest),
            HealObservation::HeldLastGood { digest, .. } => (*digest, *digest),
        };
        let index = self
            .tape
            .iter()
            .rposition(|row| row.code == code && row.attempt == attempt)
            .ok_or(HealError::UnknownHeal { code, attempt })?;
        let tick = self.tape[index]
            .applied_tick
            .unwrap_or(self.tape[index].fault_tick);
        self.tape[index].before_digest = Some(before);
        self.tape[index].after_digest = Some(after);
        self.tape[index].validation = verdict;
        let label = code.name().unwrap_or("unknown");
        match verdict {
            HealValidation::Recovered => {
                // The proof carries the budget reset: equal digests mean the
                // state is the last good state again.
                self.clear_budget(code);
                ring.push(Event::new(
                    EventCode::HEAL_APPLIED,
                    tick,
                    &format!("heal.applied cause={label} attempt={attempt} verified"),
                ));
            }
            HealValidation::NotRecovered => {
                self.fail_attempt(code, tick, attempt, ring);
            }
            HealValidation::FrozenFrame => {
                self.frozen_frames = self.frozen_frames.saturating_add(1);
                let frames = match observation {
                    HealObservation::HeldLastGood { frames, .. } => *frames,
                    _ => 0,
                };
                ring.push(Event::new(
                    EventCode::HEAL_FAILED,
                    tick,
                    &format!(
                        "heal.failed cause={label} attempt={attempt} frozen frame held {frames} (gate G1)"
                    ),
                ));
            }
            HealValidation::Unproven | HealValidation::RolledBack => {}
        }
        Ok(verdict)
    }

    /// Roll back a wrong heal to the last good snapshot. The rollback is
    /// proven by the restored digest: equal digests mean the state is back
    /// to the last good state and the budget is cleared; different digests
    /// mean the rollback failed, and the code is quarantined loudly.
    ///
    /// # Errors
    ///
    /// Returns [`HealError::NotRegistered`] when the code has no policy row.
    pub fn rollback_heal(
        &mut self,
        proof: &RollbackProof,
        ring: &mut RingBuffer,
    ) -> Result<HealValidation, HealError> {
        let policy = self
            .policy(proof.code)
            .ok_or(HealError::NotRegistered(proof.code))?;
        let label = proof.code.name().unwrap_or("unknown");
        if !proof.restored() {
            ring.push(Event::new(
                EventCode::HEAL_FAILED,
                proof.tick,
                &format!(
                    "heal.failed cause={label} attempt={} rollback failed: restored digest differs from the last good state",
                    proof.attempt
                ),
            ));
            let attempts = self.registry.burn_attempt(proof.code);
            let _ = self.record_quarantine(proof.code, proof.tick, attempts, 0);
            return Ok(HealValidation::NotRecovered);
        }
        self.tape.push(HealTapeRecord {
            code: proof.code,
            fault_tick: proof.tick,
            applied_tick: Some(proof.tick),
            kind: policy.kind,
            attempt: proof.attempt,
            backoff_ms: 0,
            quarantined: false,
            scope: policy.scope,
            before_digest: Some(proof.last_good_digest),
            after_digest: Some(proof.restored_digest),
            validation: HealValidation::RolledBack,
        });
        self.clear_budget(proof.code);
        ring.push(Event::new(
            EventCode::HEAL_APPLIED,
            proof.tick,
            &format!(
                "heal.applied cause={label} attempt={} rolled back to the last good state",
                proof.attempt
            ),
        ));
        Ok(HealValidation::RolledBack)
    }

    /// Clear the attempt budget, the escalation row and the quarantine entry
    /// for `code`. Tape history stays: history is evidence, not budget.
    fn clear_budget(&mut self, code: EventCode) {
        self.registry.reset(code);
        self.escalation.remove(&code.raw());
        self.quarantine.remove(&code.raw());
    }

    /// A heal that ran but could not prove itself is a failed attempt: it
    /// spends the budget and quarantines the code when the budget is gone.
    fn fail_attempt(&mut self, code: EventCode, tick: u64, attempt: u32, ring: &mut RingBuffer) {
        let attempts = self.registry.burn_attempt(code);
        let max = self.policy(code).map_or(0, |policy| policy.attempts_max);
        let label = code.name().unwrap_or("unknown");
        ring.push(Event::new(
            EventCode::HEAL_FAILED,
            tick,
            &format!("heal.failed cause={label} attempt={attempt} unproven"),
        ));
        if attempts > max {
            let _ = self.record_quarantine(code, tick, attempts, 0);
        }
    }

    /// Write a proof onto the tape row it belongs to, when there is one.
    fn write_proof(&mut self, proof: &VerifiedRecovery) {
        if let Some(row) = self
            .tape
            .iter_mut()
            .rev()
            .find(|row| row.code == proof.code && row.attempt == proof.attempt)
        {
            row.before_digest = Some(proof.before_digest);
            row.after_digest = Some(proof.after_digest);
            row.validation = proof.validation;
        }
    }

    /// Last tick seen for `code` (zero when the code never faulted).
    fn last_tick(&self, code: EventCode) -> u64 {
        self.tape
            .iter()
            .rev()
            .find(|row| row.code == code)
            .map_or(0, |row| row.applied_tick.unwrap_or(row.fault_tick))
    }

    /// True when a fault code sits in quarantine.
    fn any_quarantined(&self) -> bool {
        self.quarantine.values().any(|entry| !entry.probing)
            || self.policies.keys().any(|raw| {
                EventCode::from_raw(*raw).is_ok_and(|code| self.registry.is_quarantined(code))
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::breaker::RollingBreaker;
    use crate::heal::HealKind;
    use crate::recovery::{HealObservation, HealValidation, RollbackProof, VerifiedRecovery};
    use crate::supervisor::{FaultPolicy, Supervisor};
    use crate::{EventCode, RingBuffer};

    fn ring(cap: usize) -> RingBuffer {
        RingBuffer::new(cap)
    }

    /// The acceptance proof: a heal that "applies" while the read-back still
    /// faults is not a recovery. It spends the budget, quarantines the code
    /// and cannot reset itself.
    #[test]
    fn heal_that_applies_while_read_back_still_faults_ends_in_quarantine() {
        let mut supervisor = Supervisor::new();
        supervisor.register_policy(
            FaultPolicy::new(
                EventCode::PHYSICS_BODY_NONFINITE,
                HealKind::Restore,
                1,
                0,
                0,
                1,
            )
            .with_scope(crate::recovery::HealScope::State),
        );
        let mut events = ring(32);
        let code = EventCode::PHYSICS_BODY_NONFINITE;
        let decision = supervisor
            .report_fault(code, 10, &mut events)
            .expect("the state fault must report");
        assert!(decision.deferred);
        assert_eq!(supervisor.apply_tick_boundary(11).len(), 1);

        // The heal ran and was taped, but the read-back still faults.
        let verdict = supervisor
            .verify_heal(
                code,
                1,
                &HealObservation::State {
                    before_digest: 0xAA,
                    after_digest: 0xBB,
                },
                &mut events,
            )
            .expect("the applied heal must be checked");
        assert_eq!(verdict, HealValidation::NotRecovered);
        assert!(
            supervisor.is_quarantined(code),
            "a failed proof quarantines"
        );
        assert_eq!(supervisor.run_health(), crate::recovery::RunHealth::Red);

        // A heal cannot reopen its own budget: the reset without a proof is
        // refused and the budget stays spent.
        assert_eq!(
            supervisor.reset(code),
            crate::recovery::ResetOutcome::Refused
        );
        assert!(supervisor.is_quarantined(code));
        let fast = supervisor
            .report_fault(code, 12, &mut events)
            .expect("a quarantined fault still returns advice");
        assert!(fast.is_quarantine());
    }

    /// Re-registering a policy row keeps the attempts it already spent.
    #[test]
    fn re_registering_a_policy_does_not_reopen_the_budget() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(32);
        let code = EventCode::PROVIDER_TIMEOUT;
        supervisor
            .report_fault(code, 1, &mut events)
            .expect("first fault heals");
        supervisor
            .report_fault(code, 2, &mut events)
            .expect("second fault heals");
        supervisor
            .report_fault(code, 3, &mut events)
            .expect("over-budget fault quarantines");
        assert!(supervisor.is_quarantined(code));
        assert_eq!(supervisor.attempts(code), 3);

        // New knowledge may change the policy, never the budget.
        let policy = supervisor.policy(code).expect("the catalog row");
        supervisor.register_policy(policy);
        assert_eq!(supervisor.attempts(code), 3);
        assert!(
            supervisor.is_quarantined(code),
            "re-registering must not clear quarantine"
        );
        assert_eq!(
            supervisor.reset(code),
            crate::recovery::ResetOutcome::Refused
        );
        assert_eq!(supervisor.attempts(code), 3, "the budget stays spent");

        // Only an explicit proof clears it.
        let proof = VerifiedRecovery::new(code, 3, HealValidation::Recovered, 7, 7);
        assert_eq!(supervisor.reset_verified(&proof), Ok(()));
        assert_eq!(supervisor.attempts(code), 0);
        assert!(!supervisor.is_quarantined(code));
    }

    /// Every proven heal tape row carries both digests: before and after.
    #[test]
    fn tape_rows_carry_the_before_and_after_digests() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(64);
        let state = EventCode::PHYSICS_BODY_NONFINITE;
        let presentation = EventCode::RENDER_DEVICE_LOST;

        // A recovered state heal.
        supervisor
            .report_fault(state, 10, &mut events)
            .expect("state fault reports");
        let _ = supervisor.apply_tick_boundary(11);
        let recovered = supervisor
            .verify_heal(
                state,
                1,
                &HealObservation::State {
                    before_digest: 0x11,
                    after_digest: 0x11,
                },
                &mut events,
            )
            .expect("the heal is checked");
        assert_eq!(recovered, HealValidation::Recovered);

        // A presentation heal that only held the last good frame.
        supervisor
            .report_fault(presentation, 20, &mut events)
            .expect("presentation fault reports");
        let frozen = supervisor
            .verify_heal(
                presentation,
                1,
                &HealObservation::HeldLastGood {
                    frames: 30,
                    digest: 0x22,
                },
                &mut events,
            )
            .expect("the heal is checked");
        assert_eq!(frozen, HealValidation::FrozenFrame);

        // A wrong heal rolled back to the last good snapshot.
        supervisor
            .report_fault(state, 30, &mut events)
            .expect("state fault reports again");
        let _ = supervisor.apply_tick_boundary(31);
        let _ = supervisor
            .verify_heal(
                state,
                1,
                &HealObservation::State {
                    before_digest: 0x33,
                    after_digest: 0x44,
                },
                &mut events,
            )
            .expect("the heal is checked");
        let rolled = supervisor
            .rollback_heal(&RollbackProof::new(state, 2, 32, 0x33, 0x33), &mut events)
            .expect("the rollback runs");
        assert_eq!(rolled, HealValidation::RolledBack);

        let rows: Vec<(u64, u64, HealValidation)> = supervisor
            .tape()
            .iter()
            .filter(|row| {
                row.validation.is_green() || row.validation == HealValidation::FrozenFrame
            })
            .map(|row| {
                (
                    row.before_digest
                        .expect("every proven row has a before digest"),
                    row.after_digest
                        .expect("every proven row has an after digest"),
                    row.validation,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (0x11, 0x11, HealValidation::Recovered),
                (0x22, 0x22, HealValidation::FrozenFrame),
                (0x33, 0x33, HealValidation::RolledBack),
            ]
        );
        assert!(supervisor
            .tape()
            .iter()
            .all(|row| row.before_digest.is_some() && row.after_digest.is_some()));
    }

    /// The circuit vectors run against the src module, not a copy in tests.
    #[test]
    fn the_existing_vectors_run_against_the_src_breaker() {
        // Five errors then five successes: exactly 50% over a full window.
        let mut breaker = RollingBreaker::new();
        for at in 0..10_u64 {
            breaker.admit().expect("the closed breaker admits");
            breaker.observe(at >= 5);
        }
        assert!(breaker.is_open(), "50% errors over 10 must open");
        assert_eq!(breaker.opened_at(), Some(10));
        assert_eq!(breaker.attempt(), 10);
        assert_eq!(breaker.admit(), Err(crate::breaker::BreakerError::Open));

        // One probe admits; success closes; a second probe is refused.
        breaker.half_open_probe().expect("one probe admitted");
        assert!(breaker.is_half_open());
        assert_ne!(breaker.half_open_probe(), Ok(()));
        breaker.observe(true);
        assert!(!breaker.is_open());
        assert!(!breaker.is_half_open());

        // A fresh window holds 40% errors without opening.
        let mut low = RollingBreaker::new();
        for at in 0..10_u64 {
            low.observe(at >= 4);
        }
        assert!(!low.is_open());
        assert_eq!(low.error_percent(), 40);
        assert_eq!(low.opened_at(), None);

        // The same vector twice gives the same open revision.
        let mut replay = RollingBreaker::new();
        for at in 0..10_u64 {
            replay.observe(at >= 5);
        }
        assert_eq!(replay.opened_at(), breaker.opened_at());
    }

    /// A provider timeout storm opens the breaker, the next fault stops
    /// without budget, and exactly one half-open probe is admitted.
    #[test]
    fn a_timeout_storm_opens_the_breaker_and_admits_one_half_open_probe() {
        let mut supervisor = Supervisor::new();
        let mut events = ring(64);
        let code = EventCode::PROVIDER_TIMEOUT;
        supervisor
            .enable_breaker(code)
            .expect("the provider row is registered");
        // Ten timeouts clocked by tick: the open revision is the tick.
        for tick in 100..110_u64 {
            supervisor.record_attempt_outcome_at(code, tick, false);
        }
        assert!(supervisor.breaker_is_open(code));
        assert_eq!(supervisor.breaker_opened_at(code), Some(109));

        // Open: the next fault stops here, with no budget, tape or event.
        let attempts = supervisor.attempts(code);
        let tape_len = supervisor.tape_len();
        let rejected = supervisor
            .report_fault(code, 110, &mut events)
            .expect("an open breaker returns advice");
        assert!(rejected.is_quarantine());
        assert_eq!(rejected.kind, HealKind::Quarantine);
        assert_eq!(supervisor.attempts(code), attempts);
        assert_eq!(supervisor.tape_len(), tape_len);
        assert!(events.query(Some(EventCode::HEAL_FAILED)).is_empty());

        // One probe, then the recovery closes the breaker again.
        assert!(supervisor.breaker_probe(code));
        assert!(supervisor.breaker_is_half_open(code));
        assert!(!supervisor.breaker_probe(code), "one probe per period");
        supervisor.record_attempt_outcome_at(code, 111, true);
        assert!(!supervisor.breaker_is_open(code));
        assert!(!supervisor.breaker_is_half_open(code));
        let healed = supervisor
            .report_fault(code, 112, &mut events)
            .expect("a closed breaker heals again");
        assert!(!healed.is_quarantine());
        assert_eq!(healed.kind, HealKind::Degrade);
    }
}
