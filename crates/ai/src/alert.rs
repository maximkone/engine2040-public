//! ai alert ladder: sight plus sound plus a closed door into five states.
//! Data owner: this module owns `Alert`, `Sense` and `AlertState` only.
//! Units: last-seen cells in centimetres, ticks in fixed-tick counts.
//! Determinism: pure integer step over the caller-passed tick; same inputs
//! give the same output, no wall clock. Errors: none (every input is valid).

/// Five-state awareness ladder, lowest to highest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alert {
    /// No evidence; the default state.
    Unaware,
    /// A glimpse: partial sight stamped the evidence tick.
    Suspicious,
    /// A sound with no sight stamped the evidence tick.
    Investigate,
    /// Lost contact inside the window; holds the fixed last-seen cell.
    Track,
    /// Confirmed sight; holds the fixed last-seen cell.
    Combat,
}

/// What the eyes report before the door is applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sight {
    /// Nothing visible.
    None,
    /// A glimpse, not a lock.
    Partial,
    /// A solid lock on the target.
    Confirmed,
}

/// One tick of raw sense input for the ladder.
/// Units: flag-only; the cell travels beside this struct in `step`.
/// Determinism: plain data; the door folds sight to [`Sight::None`].
/// Errors: none (every combination is valid).
#[derive(Clone, Copy)]
pub struct Sense {
    /// Eye report before the door is applied.
    pub sight: Sight,
    /// True when a sound was heard this tick.
    pub heard: bool,
    /// True when a closed door stands between eye and target.
    pub door_closed: bool,
}

/// Ladder state: the rung, the fixed last-seen cell, and its stamp.
/// Units: cell in centimetres, stamp in fixed-tick counts.
/// Determinism: integer ticks only, wrapping subtraction for expiry.
/// Errors: none (all fields are valid by construction).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AlertState {
    /// Current rung on the ladder.
    pub alert: Alert,
    /// Fixed cell kept while tracking; never follows the live position.
    pub last_seen_cm: Option<[i32; 2]>,
    /// Tick of the last stimulus; unchanged while quiet.
    pub evidence_tick: u64,
}

/// Ticks a stimulus stays live before the ladder falls to [`Alert::Unaware`].
const EVIDENCE_WINDOW_TICKS: u64 = 300;

/// Fresh ladder: [`Alert::Unaware`] with no cell and tick 0.
/// Units: none. Determinism: constant. Errors: none.
pub fn fresh() -> AlertState {
    AlertState {
        alert: Alert::Unaware,
        last_seen_cm: None,
        evidence_tick: 0,
    }
}

/// Fold one tick of sense into the next ladder state.
/// Units: `tick` in fixed-tick counts, cell in centimetres.
/// Determinism: pure; same inputs give the same output twice.
/// Errors: none (a closed door folds sight to [`Sight::None`]).
pub fn step(
    state: &AlertState,
    sense: Sense,
    tick: u64,
    seen_cell_cm: Option<[i32; 2]>,
) -> AlertState {
    let effective = if sense.door_closed {
        Sight::None
    } else {
        sense.sight
    };
    match effective {
        Sight::Confirmed => AlertState {
            alert: Alert::Combat,
            last_seen_cm: seen_cell_cm,
            evidence_tick: tick,
        },
        Sight::Partial => AlertState {
            alert: Alert::Suspicious,
            last_seen_cm: state.last_seen_cm,
            evidence_tick: tick,
        },
        Sight::None => {
            if sense.heard {
                return AlertState {
                    alert: Alert::Investigate,
                    last_seen_cm: state.last_seen_cm,
                    evidence_tick: tick,
                };
            }
            if tick.wrapping_sub(state.evidence_tick) >= EVIDENCE_WINDOW_TICKS {
                return AlertState {
                    alert: Alert::Unaware,
                    last_seen_cm: None,
                    evidence_tick: state.evidence_tick,
                };
            }
            match state.alert {
                Alert::Combat => AlertState {
                    alert: Alert::Track,
                    last_seen_cm: state.last_seen_cm,
                    evidence_tick: state.evidence_tick,
                },
                _ => *state,
            }
        }
    }
}

/// Fixed last-seen rule: the stored cell for every rung except
/// [`Alert::Unaware`], which reports `None`. Track never follows live input.
/// Units: cell in centimetres. Determinism: pure read. Errors: none.
pub fn last_seen(state: &AlertState) -> Option<[i32; 2]> {
    match state.alert {
        Alert::Unaware => None,
        _ => state.last_seen_cm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_partial_is_suspicious() {
        let state = fresh();
        let next = step(
            &state,
            Sense {
                sight: Sight::Partial,
                heard: false,
                door_closed: false,
            },
            5,
            None,
        );
        assert_eq!(next.alert, Alert::Suspicious);
        assert_eq!(next.evidence_tick, 5);
        assert_eq!(next.last_seen_cm, None);
    }

    #[test]
    fn alert_sound_only_is_investigate() {
        let state = fresh();
        let next = step(
            &state,
            Sense {
                sight: Sight::None,
                heard: true,
                door_closed: false,
            },
            7,
            None,
        );
        assert_eq!(next.alert, Alert::Investigate);
        assert_eq!(next.evidence_tick, 7);
        assert_eq!(next.last_seen_cm, None);
    }

    #[test]
    fn alert_confirmed_is_combat() {
        let state = fresh();
        let next = step(
            &state,
            Sense {
                sight: Sight::Confirmed,
                heard: false,
                door_closed: false,
            },
            0,
            Some([100, 200]),
        );
        assert_eq!(next.alert, Alert::Combat);
        assert_eq!(next.last_seen_cm, Some([100, 200]));
        assert_eq!(next.evidence_tick, 0);
        assert_eq!(last_seen(&next), Some([100, 200]));
    }

    #[test]
    fn alert_lost_goes_track_then_unaware() {
        let combat = step(
            &fresh(),
            Sense {
                sight: Sight::Confirmed,
                heard: false,
                door_closed: false,
            },
            0,
            Some([100, 200]),
        );
        let quiet = Sense {
            sight: Sight::None,
            heard: false,
            door_closed: false,
        };
        let track = step(&combat, quiet, 10, None);
        assert_eq!(track.alert, Alert::Track);
        assert_eq!(track.last_seen_cm, Some([100, 200]));
        assert_eq!(last_seen(&track), Some([100, 200]));
        let cold = step(&combat, quiet, 300, None);
        assert_eq!(cold.alert, Alert::Unaware);
        assert_eq!(cold.last_seen_cm, None);
        assert_eq!(last_seen(&cold), None);
    }

    #[test]
    fn alert_closed_door_blocks_sight() {
        let state = fresh();
        let next = step(
            &state,
            Sense {
                sight: Sight::Confirmed,
                heard: false,
                door_closed: true,
            },
            3,
            Some([100, 200]),
        );
        assert_ne!(next.alert, Alert::Combat);
        assert_eq!(next.alert, Alert::Unaware);
    }

    #[test]
    fn alert_stable_x2() {
        let combat = step(
            &fresh(),
            Sense {
                sight: Sight::Confirmed,
                heard: false,
                door_closed: false,
            },
            0,
            Some([100, 200]),
        );
        let quiet = Sense {
            sight: Sight::None,
            heard: false,
            door_closed: false,
        };
        let first = step(&combat, quiet, 10, None);
        let second = step(&combat, quiet, 10, None);
        assert_eq!(first, second);
        assert_eq!(first.last_seen_cm, Some([100, 200]));
    }
}
