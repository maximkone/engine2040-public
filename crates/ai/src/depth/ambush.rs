//! ambush: the hold a fighter keeps on a remembered target.
//!
//! Consumer: an arena brain keeps one [`AmbushHold`] per ambusher and calls
//! [`AmbushHold::step`] once a tick. Entry needs a remembered target
//! ([`AmbushHold::enter`] refuses live contact) and the ambusher role, so a
//! fighter that lost sight hides on its memory; the budget runs out into
//! [`AmbushExpiry::Timeout`] and it rejoins the fight instead of waiting
//! forever. Data owner: this module; the arena owns the tick call and the
//! event. Units: ticks at 60 Hz, ids as the arena spells them. Determinism:
//! integer only, no clock, no map order. Errors: none; a refused entry is
//! `None`. Budget: constant time, no heap.

/// Ticks an ambush waits on its nest before giving up: 120 = 2 s at 60 Hz.
pub const AMBUSH_TIMEOUT_TICKS: u32 = 120;

/// Why an ambush hold ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmbushExpiry {
    /// The budget ran out with no contact, so the ambusher rejoins the fight.
    Timeout,
}

impl AmbushExpiry {
    /// Stable reason code for an arena receipt.
    pub const fn reason(self) -> &'static str {
        match self {
            AmbushExpiry::Timeout => "ambush_timeout",
        }
    }
}

/// One ambush hold: the nest a fighter waits on, its target and the budget.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AmbushHold {
    /// Ticks left before the hold gives up.
    pub ticks_left: u32,
    /// The fighter the ambush waits for.
    pub target_id: u32,
    /// The nest node the ambush query chose.
    pub node_id: u32,
    /// True when the hold came from memory, never a live sighting.
    pub from_memory: bool,
}

impl AmbushHold {
    /// Enter a hold: a remembered target and the ambusher role, or nothing.
    ///
    /// `evidence_from_memory` is the last evidence kind: live contact this
    /// window refuses, so an ambush never starts on a sighting it already has.
    #[must_use]
    pub fn enter(
        target_id: u32,
        node_id: u32,
        evidence_from_memory: bool,
        is_ambusher: bool,
    ) -> Option<Self> {
        if !evidence_from_memory || !is_ambusher {
            return None;
        }
        Some(Self {
            ticks_left: AMBUSH_TIMEOUT_TICKS,
            target_id,
            node_id,
            from_memory: true,
        })
    }

    /// One tick of waiting. Live contact ends the hold with no reason (the
    /// fight resumes); an empty budget ends it with the timeout reason.
    pub fn step(&mut self, contact: bool) -> Option<AmbushExpiry> {
        if contact {
            self.ticks_left = 0;
            return None;
        }
        self.ticks_left = self.ticks_left.saturating_sub(1);
        if self.ticks_left == 0 {
            return Some(AmbushExpiry::Timeout);
        }
        None
    }

    /// True while the hold still waits on its nest.
    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.ticks_left > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ambush waits on a memory, never on a live sighting, and the budget
    /// ends it with the timeout reason.
    #[test]
    fn ambush_holds_on_memory_and_clears_on_timeout() {
        // Memory evidence plus the ambusher role enters, on full budget.
        let mut hold = AmbushHold::enter(7, 21, true, true).expect("memory enters");
        assert_eq!(hold.ticks_left, AMBUSH_TIMEOUT_TICKS);
        assert_eq!(
            (hold.target_id, hold.node_id, hold.from_memory),
            (7, 21, true)
        );
        assert!(hold.is_holding());

        // Live contact this window refuses: the ambush would be a sighting.
        assert_eq!(AmbushHold::enter(7, 21, false, true), None);
        // A fighter outside the ambusher role refuses too.
        assert_eq!(AmbushHold::enter(7, 21, true, false), None);

        // Nothing arrives: the last tick of the budget names the reason.
        for _ in 1..AMBUSH_TIMEOUT_TICKS {
            assert_eq!(hold.step(false), None, "no early give-up");
            assert!(hold.is_holding());
        }
        assert_eq!(hold.step(false), Some(AmbushExpiry::Timeout));
        assert!(!hold.is_holding());
        assert_eq!(AmbushExpiry::Timeout.reason(), "ambush_timeout");

        // Contact ends the hold at once, with no timeout reason.
        let mut seen = AmbushHold::enter(9, 4, true, true).expect("memory enters");
        assert_eq!(seen.step(true), None);
        assert!(!seen.is_holding());
        assert_eq!(seen.ticks_left, 0);
    }
}
