//! defence: one resolver answers "did the defence hold?" for both men (SW82,
//! rules v3). Perilous, unparryable, hyper-armour and a held guard that bleeds
//! slower all read the same query, so a block is no longer one `Blocked` event
//! decided in two places.
//!
//! The rule, in integer ticks:
//!
//! - a blow the move marks **unblockable** is not answered by anything, not
//!   even by a parry inside its window: [`DefenceOutcome::None`];
//! - a blow the move marks **unparryable** degrades a live parry to a plain
//!   block when the defender can block, and to nothing when he cannot;
//! - a defence older than `deflect_window` ticks is a held guard: it blocks,
//!   it does not deflect;
//! - **hyper-armour** refuses stagger in the move's Startup and Active, and
//!   never in the recovery tail.
//!
//! Units: integer ticks at the fixed step; no floats, no clock, so the same
//! query answers the same twice. Deliberately no allocation and no hash map:
//! the resolve runs inside the fixed tick.

use crate::exchange::MovePhase;

/// What the defence came to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefenceOutcome {
    /// Nothing held: the blow lands.
    None,
    /// A parry inside its window turned the blow.
    Deflected,
    /// The guard stopped the blow but did not turn it.
    Blocked,
}

/// One defender's state and one blow's own marks, all integer ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DefenceQuery {
    /// The defender is in a defence right now.
    pub is_defending: bool,
    /// The defender's move can deflect a blow (a parry, not a shield).
    pub can_deflect: bool,
    /// The defender's move can stop a blow without turning it.
    pub can_block: bool,
    /// Ticks spent in the current defence.
    pub time_in_defence: u32,
    /// Ticks after the defence starts in which a parry deflects.
    pub deflect_window: u32,
    /// The blow's own row says no block answers it.
    pub unblockable: bool,
    /// The blow's own row says no parry answers it.
    pub unparryable: bool,
}

impl DefenceQuery {
    /// A defender doing nothing against a plainly answerable blow.
    #[must_use]
    pub const fn idle() -> Self {
        Self {
            is_defending: false,
            can_deflect: false,
            can_block: false,
            time_in_defence: 0,
            deflect_window: 0,
            unblockable: false,
            unparryable: false,
        }
    }

    /// True when the defence is still inside its deflect window.
    #[must_use]
    pub const fn inside_window(&self) -> bool {
        self.time_in_defence < self.deflect_window
    }
}

/// Did the defence hold? Integer ticks in, one outcome out.
///
/// A blow the move marks unblockable is not answered at all, even by a parry
/// inside its window: the flag outranks the geometry, because a perilous blow
/// exists to punish the guard. An unparryable blow is stopped by a guard that
/// can block, and passes a defender who can only parry.
#[must_use]
pub const fn resolve_defence(query: &DefenceQuery) -> DefenceOutcome {
    if !query.is_defending {
        return DefenceOutcome::None;
    }
    if query.unblockable {
        return DefenceOutcome::None;
    }
    if query.can_deflect && !query.unparryable && query.inside_window() {
        return DefenceOutcome::Deflected;
    }
    if query.can_block {
        return DefenceOutcome::Blocked;
    }
    DefenceOutcome::None
}

/// Does the blow's own row hold the body through a stagger in this phase?
///
/// Hyper-armour refuses stagger while the move starts up and runs live, and
/// never in the recovery tail: a fighter who has swung through cannot also
/// ignore the answer of the man he swung at.
#[must_use]
pub const fn resists_stagger(phase: MovePhase) -> bool {
    matches!(phase, MovePhase::Startup | MovePhase::Active)
}

/// The defence's outcome and whether the body held through it.
#[must_use]
pub const fn resolve_defence_phased(
    query: &DefenceQuery,
    phase: MovePhase,
) -> (DefenceOutcome, bool) {
    (resolve_defence(query), resists_stagger(phase))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parry inside its window.
    fn live_parry(window: u32, spent: u32) -> DefenceQuery {
        DefenceQuery {
            is_defending: true,
            can_deflect: true,
            can_block: false,
            time_in_defence: spent,
            deflect_window: window,
            unblockable: false,
            unparryable: false,
        }
    }

    #[test]
    fn defence_unblockable_through_a_live_parry_is_none() {
        let mut query = live_parry(3, 1);
        assert_eq!(resolve_defence(&query), DefenceOutcome::Deflected);
        query.unblockable = true;
        assert_eq!(
            resolve_defence(&query),
            DefenceOutcome::None,
            "a perilous blow is not answered, even inside the window"
        );
    }

    #[test]
    fn defence_unparryable_against_a_blocker_is_blocked() {
        let mut query = live_parry(3, 1);
        query.can_block = true;
        query.unparryable = true;
        assert_eq!(
            resolve_defence(&query),
            DefenceOutcome::Blocked,
            "a parry-only answer degrades to the guard it also has"
        );
        query.can_block = false;
        assert_eq!(
            resolve_defence(&query),
            DefenceOutcome::None,
            "a defender who can only parry has no answer to it"
        );
    }

    #[test]
    fn defence_hyper_armour_holds_in_active_and_fails_in_recovery() {
        assert!(resists_stagger(MovePhase::Startup), "startup holds");
        assert!(resists_stagger(MovePhase::Active), "the live phase holds");
        assert!(!resists_stagger(MovePhase::Endlag), "recovery never holds");
        assert!(
            !resists_stagger(MovePhase::Done),
            "a finished move holds nothing"
        );
        // The phased door answers both questions at once.
        let query = live_parry(3, 1);
        assert_eq!(
            resolve_defence_phased(&query, MovePhase::Active),
            (DefenceOutcome::Deflected, true)
        );
        assert_eq!(
            resolve_defence_phased(&query, MovePhase::Endlag),
            (DefenceOutcome::Deflected, false)
        );
    }

    #[test]
    fn defence_a_four_tick_old_guard_is_blocked_at_window_three() {
        let mut query = live_parry(3, 4);
        query.can_block = true;
        assert!(!query.inside_window(), "4 ticks is past a 3-tick window");
        assert_eq!(
            resolve_defence(&query),
            DefenceOutcome::Blocked,
            "a held guard bleeds slower, it does not turn the blow"
        );
        // The same defender at 2 ticks still deflects.
        assert_eq!(
            resolve_defence(&live_parry(3, 2)),
            DefenceOutcome::Deflected
        );
    }

    #[test]
    fn defence_is_the_same_twice() {
        for query in [
            DefenceQuery::idle(),
            live_parry(3, 1),
            live_parry(3, 4),
            DefenceQuery {
                is_defending: true,
                can_deflect: false,
                can_block: true,
                time_in_defence: 0,
                deflect_window: 3,
                unblockable: false,
                unparryable: true,
            },
        ] {
            let first = resolve_defence(&query);
            let second = resolve_defence(&query);
            assert_eq!(first, second, "the same query answers the same twice");
            assert_eq!(
                resolve_defence_phased(&query, MovePhase::Active),
                resolve_defence_phased(&query, MovePhase::Active),
                "the phased door is stable too"
            );
        }
    }
}
