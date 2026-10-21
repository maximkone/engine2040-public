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
