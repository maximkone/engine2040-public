//! Heal scope, the recovery proof and the rolling heal cap.
//!
//! Consumer: the supervisor (planning and the heal tape), the subsystems that
//! execute a heal (they report what they observed) and the gates that read
//! the run health (G1 never-broken, A9 chaos).
//! Data owner: this module owns the types; the supervisor owns the counts.
//! Units: ticks (`u64`), digests (`u64`), counts (`u32`).
//! Determinism: integer arithmetic only, no clock, no float.
//! Errors: [`crate::heal::HealError`]; a heal without a proof is refused,
//! never assumed good.
//!
//! A heal is only a recovery when it proves itself. The proof is the digest
//! of the healed state against the digest of the last good state: equal means
//! recovered, different means not recovered. A presentation heal that only
//! held the last good frame is a frozen frame (the G1 frozen-pair failure) and
//! never counts as green.

use crate::EventCode;
use std::collections::VecDeque;

/// The engine's fixed tick rate in hertz. The engine ticks at 60 Hz
/// (`crates/ai/src/gladiator.rs`: "180 ticks = 3 s", "3_600 ticks cover a 60 s
/// bout at 60 Hz"), so one minute of ticks is 3,600.
pub const TICKS_PER_SECOND: u64 = 60;

/// One minute of ticks at 60 Hz. The rolling window of the heal cap.
pub const HEAL_CAP_WINDOW_TICKS: u64 = 60 * TICKS_PER_SECOND;

/// What a heal touches. A presentation heal never changes simulation or scene
/// state; a state heal does and must land at a tick boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealScope {
    /// Presentation or service heal: pipeline, device, asset, provider,
    /// command, editor, save. It never enters the simulation hash.
    Presentation,
    /// Authoritative state heal: physics, brain, net desync. It runs at a
    /// tick boundary from the same inputs on every machine.
    State,
}

impl HealScope {
    /// True for a state heal.
    #[must_use]
    pub fn is_state(self) -> bool {
        matches!(self, Self::State)
    }
}

/// The verdict of a heal proof. Every heal tape row carries one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealValidation {
    /// The heal was planned; nobody checked the result yet.
    Unproven,
    /// The healed state hashes to the last good state. Green.
    Recovered,
    /// The heal ran but the state hashes to something else. A failed attempt.
    NotRecovered,
    /// A presentation heal that only held the last good frame. Never green:
    /// two byte-equal frames in a row are the G1 frozen-picture failure.
    FrozenFrame,
    /// A wrong heal was rolled back and the restored state hashes to the last
    /// good state. The state is healthy again.
    RolledBack,
}

impl HealValidation {
    /// True only when the verdict proves a recovery. A frozen frame and an
    /// unproven heal never pass.
    #[must_use]
    pub fn is_recovered(self) -> bool {
        matches!(self, Self::Recovered)
    }

    /// True when the run may count as healthy after this verdict: a proved
    /// recovery or a proved rollback. A frozen frame and a failed attempt are
    /// never green.
    #[must_use]
    pub fn is_green(self) -> bool {
        matches!(self, Self::Recovered | Self::RolledBack)
    }

    /// Short label for events and notices.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Unproven => "unproven",
            Self::Recovered => "recovered",
            Self::NotRecovered => "not_recovered",
            Self::FrozenFrame => "frozen_frame",
            Self::RolledBack => "rolled_back",
        }
    }
}

/// What the caller observed after the heal ran. The caller reports numbers
/// only; the verdict is decided here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealObservation {
    /// A state heal ran: the digest of the last good state and the digest of
    /// the state after the heal. Equal digests mean recovered.
    State {
        /// Digest of the last good state before the heal.
        before_digest: u64,
        /// Digest of the state after the heal.
        after_digest: u64,
    },
    /// A presentation heal presented a fresh frame again. The renderer is
    /// live; the pipeline is not held.
    Presented {
        /// Digest of the fresh frame.
        digest: u64,
    },
    /// A presentation heal only held the last good frame. Never green.
    HeldLastGood {
        /// Frames of the last good picture held while down.
        frames: u64,
        /// Digest of the held picture.
        digest: u64,
    },
}

/// Proof that a recovery happened. A budget reset needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedRecovery {
    /// Fault code that was healed.
    pub code: EventCode,
    /// One-based attempt the proof belongs to.
    pub attempt: u32,
    /// The verdict that proves the recovery.
    pub validation: HealValidation,
    /// Digest of the last good state before the heal.
    pub before_digest: u64,
    /// Digest of the state after the heal.
    pub after_digest: u64,
}

impl VerifiedRecovery {
    /// Build a proof.
    #[must_use]
    pub fn new(
        code: EventCode,
        attempt: u32,
        validation: HealValidation,
        before_digest: u64,
        after_digest: u64,
    ) -> Self {
        Self {
            code,
            attempt,
            validation,
            before_digest,
            after_digest,
        }
    }

    /// True when the verdict proves a recovery (a proved recovery or a proved
    /// rollback).
    #[must_use]
    pub fn proves_recovery(self) -> bool {
        self.validation.is_green()
    }
}

/// Proof that a wrong heal was rolled back to the last good snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollbackProof {
    /// Fault code whose heal was rolled back.
    pub code: EventCode,
    /// One-based attempt the rollback belongs to.
    pub attempt: u32,
    /// Tick the rollback ran at.
    pub tick: u64,
    /// Digest of the last good state before the bad heal.
    pub last_good_digest: u64,
    /// Digest of the restored state after the rollback.
    pub restored_digest: u64,
}

impl RollbackProof {
    /// Build a rollback proof.
    #[must_use]
    pub fn new(
        code: EventCode,
        attempt: u32,
        tick: u64,
        last_good_digest: u64,
        restored_digest: u64,
    ) -> Self {
        Self {
            code,
            attempt,
            tick,
            last_good_digest,
            restored_digest,
        }
    }

    /// True when the restored state hashes to the last good state: the
    /// rollback is proven.
    #[must_use]
    pub fn restored(self) -> bool {
        self.last_good_digest == self.restored_digest
    }
}

/// The outcome of a budget reset. A reset without a proof is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetOutcome {
    /// The budget was cleared: a verified recovery or a proved rollback was
    /// on file.
    Cleared,
    /// The reset was refused: no proof was on file, so the budget stays
    /// spent. A heal cannot reopen its own budget.
    Refused,
}

/// Rolling heal cap: how many heals may fire inside one rolling window of
/// ticks. Past the cap the run reads red, never degraded.
#[derive(Debug, Clone)]
pub struct HealCap {
    max_heals: u32,
    window_ticks: u64,
    window: VecDeque<(u64, u32)>,
    total: u32,
    exceeded: bool,
}

impl HealCap {
    /// Cap of `max_heals` heals per `window_ticks` ticks.
    #[must_use]
    pub fn new(max_heals: u32, window_ticks: u64) -> Self {
        Self {
            max_heals,
            window_ticks,
            window: VecDeque::new(),
            total: 0,
            exceeded: false,
        }
    }

    /// Cap value in heals per window.
    #[must_use]
    pub fn max_heals(&self) -> u32 {
        self.max_heals
    }

    /// Window length in ticks.
    #[must_use]
    pub fn window_ticks(&self) -> u64 {
        self.window_ticks
    }

    /// Count one heal at `tick`; true when the cap is now blown. The cap
    /// stays blown until the window empties: a red run does not silently go
    /// green again.
    pub fn observe(&mut self, tick: u64) -> bool {
        self.prune(tick);
        self.window.push_back((tick, 1));
        self.total = self.total.saturating_add(1);
        if self.total > self.max_heals {
            self.exceeded = true;
        }
        self.exceeded
    }

    /// Heals inside the window that ends at `tick`.
    #[must_use]
    pub fn heals_in_window(&self, tick: u64) -> u32 {
        self.window
            .iter()
            .filter(|(at, _)| tick.saturating_sub(*at) < self.window_ticks)
            .map(|(_, count)| *count)
            .sum()
    }

    /// True once the cap was blown. Sticky until the window empties.
    #[must_use]
    pub fn exceeded(&self) -> bool {
        self.exceeded
    }

    /// Drop heals that fell out of the window. The red flag clears only when
    /// the window is empty again.
    fn prune(&mut self, tick: u64) {
        while let Some((at, count)) = self.window.front().copied() {
            if tick.saturating_sub(at) < self.window_ticks {
                break;
            }
            self.window.pop_front();
            self.total = self.total.saturating_sub(count);
        }
        if self.total == 0 {
            self.exceeded = false;
        }
    }
}

/// How the run reads. Green is the only state a gate may count as healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunHealth {
    /// Nothing wrong: no heal in flight, no red flag.
    Green,
    /// A fault is being handled (a heal is planned or in flight) but the run
    /// is not red.
    Degraded,
    /// The run is red: the heal cap is blown, a heal only held the last good
    /// frame, or a fault code is quarantined.
    Red,
}

impl RunHealth {
    /// True only for a green run.
    #[must_use]
    pub fn is_green(self) -> bool {
        matches!(self, Self::Green)
    }

    /// True when the run reads red.
    #[must_use]
    pub fn is_red(self) -> bool {
        matches!(self, Self::Red)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_counts_inside_the_rolling_window_only() {
        let mut cap = HealCap::new(2, 100);
        assert!(!cap.observe(0));
        assert!(!cap.observe(10));
        assert!(
            cap.observe(20),
            "the third heal in the window blows the cap"
        );
        assert!(cap.exceeded());
        assert_eq!(cap.heals_in_window(20), 3);
        // A heal one window later drops the old ones.
        assert!(!cap.observe(200));
        assert_eq!(cap.heals_in_window(200), 1);
        assert!(!cap.exceeded(), "an empty window clears the red flag");
    }

    #[test]
    fn one_minute_window_is_3600_ticks() {
        assert_eq!(TICKS_PER_SECOND, 60);
        assert_eq!(HEAL_CAP_WINDOW_TICKS, 3600);
    }

    #[test]
    fn only_a_proved_recovery_is_green() {
        assert!(HealValidation::Recovered.is_green());
        assert!(HealValidation::RolledBack.is_green());
        assert!(!HealValidation::Unproven.is_green());
        assert!(!HealValidation::NotRecovered.is_green());
        assert!(!HealValidation::FrozenFrame.is_green());
        assert!(HealValidation::Recovered.is_recovered());
        assert!(!HealValidation::RolledBack.is_recovered());
    }

    #[test]
    fn rollback_proof_needs_matching_digests() {
        let good = RollbackProof::new(EventCode::NET_CLIENT_DESYNC, 1, 10, 42, 42);
        assert!(good.restored());
        let bad = RollbackProof::new(EventCode::NET_CLIENT_DESYNC, 1, 10, 42, 43);
        assert!(!bad.restored());
    }
}
