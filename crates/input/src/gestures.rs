//! gestures: tick-counted tap, double-tap and hold detector.
//!
//! Consumer: future action-set mapping feeds one bool per fixed tick and
//! turns emissions into action bits; this slice only detects, mapping is out
//! of scope. Data owner: [`GestureDetector`] owns its tick counters. Units:
//! ticks (integer fixed steps, e.g. 12 ticks ~200 ms at 60 Hz). Determinism:
//! pure tick function, no alloc, no clock, same script gives same emissions.
//! Errors: never fails; out-of-window presses yield `None`.

/// Minimum held ticks for a tap (shorter is noise, ignored).
pub const TAP_MIN_HELD_TICKS: u32 = 2;
/// Maximum held ticks for a tap (longer is a hold candidate or dead zone).
pub const TAP_MAX_HELD_TICKS: u32 = 12;
/// Maximum released ticks between two taps for a double-tap (inclusive).
pub const DOUBLE_TAP_MAX_GAP_TICKS: u32 = 18;
/// Minimum held ticks for a hold, emitted while still held.
pub const HOLD_MIN_HELD_TICKS: u32 = 30;

const _: () = assert!(
    TAP_MIN_HELD_TICKS <= TAP_MAX_HELD_TICKS
        && TAP_MAX_HELD_TICKS < HOLD_MIN_HELD_TICKS
        && TAP_MAX_HELD_TICKS > 0
);

/// Tick-window gesture emitted by [`GestureDetector`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Gesture {
    /// One short press confirmed after the double-tap window expires.
    Tap,
    /// Two short presses with a gap inside [`DOUBLE_TAP_MAX_GAP_TICKS`].
    DoubleTap,
    /// One long press, emitted on the tick held reaches [`HOLD_MIN_HELD_TICKS`].
    Hold,
}

/// Tick-counted detector for tap, double-tap and hold.
///
/// Feed one `pressed` bool per fixed tick with [`GestureDetector::tick`];
/// it returns one emission at most per tick. A first tap waits for the
/// double-tap window: a second tap in time becomes [`Gesture::DoubleTap`],
/// otherwise the first becomes [`Gesture::Tap`] on window expiry. A press
/// that reaches [`HOLD_MIN_HELD_TICKS`] emits [`Gesture::Hold`] while held
/// and cancels any pending tap. Budget: O(1) per tick, no heap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GestureDetector {
    held: u32,
    gap: u32,
    pending: bool,
    hold_fired: bool,
}

impl GestureDetector {
    /// Blank detector: nothing held, no pending tap.
    pub fn new() -> Self {
        Self {
            held: 0,
            gap: 0,
            pending: false,
            hold_fired: false,
        }
    }

    /// Clear all counters to the blank state.
    pub fn reset(&mut self) {
        self.held = 0;
        self.gap = 0;
        self.pending = false;
        self.hold_fired = false;
    }

    /// Advance one fixed tick; `pressed` is held this tick.
    ///
    /// Returns one gesture at most: [`Gesture::Hold`] on the tick held
    /// reaches [`HOLD_MIN_HELD_TICKS`], [`Gesture::DoubleTap`] on the second
    /// release inside the gap window, [`Gesture::Tap`] when a single tap's
    /// window expires with no second press. Noise (shorter than
    /// [`TAP_MIN_HELD_TICKS`]) and dead-zone presses (longer than
    /// [`TAP_MAX_HELD_TICKS`] but shorter than [`HOLD_MIN_HELD_TICKS`])
    /// return `None`.
    pub fn tick(&mut self, pressed: bool) -> Option<Gesture> {
        if pressed {
            if self.held == 0 {
                self.held = 1;
            } else {
                self.held = self.held.saturating_add(1);
            }
            if self.held == HOLD_MIN_HELD_TICKS && !self.hold_fired {
                self.hold_fired = true;
                self.pending = false;
                self.gap = 0;
                return Some(Gesture::Hold);
            }
            return None;
        }
        if self.held > 0 {
            let held = self.held;
            self.held = 0;
            if self.hold_fired {
                self.hold_fired = false;
                self.pending = false;
                self.gap = 0;
                return None;
            }
            if self.pending {
                self.pending = false;
                self.gap = 0;
                if (TAP_MIN_HELD_TICKS..=TAP_MAX_HELD_TICKS).contains(&held) {
                    return Some(Gesture::DoubleTap);
                }
                return Some(Gesture::Tap);
            }
            if (TAP_MIN_HELD_TICKS..=TAP_MAX_HELD_TICKS).contains(&held) {
                self.pending = true;
                self.gap = 0;
            }
            return None;
        }
        if self.pending {
            self.gap = self.gap.saturating_add(1);
            if self.gap > DOUBLE_TAP_MAX_GAP_TICKS {
                self.pending = false;
                self.gap = 0;
                return Some(Gesture::Tap);
            }
        }
        None
    }
}

impl Default for GestureDetector {
    /// Blank detector (see [`GestureDetector::new`]).
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(script: &[bool]) -> Vec<Gesture> {
        let mut detector = GestureDetector::new();
        let mut out = Vec::new();
        for &pressed in script {
            if let Some(gesture) = detector.tick(pressed) {
                out.push(gesture);
            }
        }
        out
    }

    fn pressed(n: usize) -> Vec<bool> {
        vec![true; n]
    }

    fn released(n: usize) -> Vec<bool> {
        vec![false; n]
    }

    fn script(chunks: &[&[bool]]) -> Vec<bool> {
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend_from_slice(chunk);
        }
        out
    }

    #[test]
    fn tap_emits_single_after_window_expiry() {
        let press = pressed(5);
        let idle = released(25);
        let emissions = run(&script(&[&press, &released(1), &idle]));
        assert_eq!(emissions, vec![Gesture::Tap]);
        // Green held: same script replays identically.
        assert_eq!(run(&script(&[&press, &released(1), &idle])), emissions);
    }

    #[test]
    fn tap_boundaries_hold_and_gap_are_inclusive() {
        // Shortest tap (min) still counts.
        let min = run(&script(&[&pressed(2), &released(20)]));
        assert_eq!(min, vec![Gesture::Tap]);
        // Longest tap (max) still counts.
        let max = run(&script(&[&pressed(12), &released(20)]));
        assert_eq!(max, vec![Gesture::Tap]);
        // Gap exactly at max still doubles.
        let gap_max = run(&script(&[
            &pressed(5),
            &released(1),
            &released(18),
            &pressed(5),
            &released(20),
        ]));
        assert_eq!(gap_max, vec![Gesture::DoubleTap]);
    }

    #[test]
    fn double_tap_within_window_emits_double_only() {
        let emissions = run(&script(&[
            &pressed(5),
            &released(1),
            &released(10),
            &pressed(6),
            &released(25),
        ]));
        assert_eq!(emissions, vec![Gesture::DoubleTap]);
        assert_eq!(emissions.len(), 1);
        assert!(!emissions.contains(&Gesture::Tap));
        assert!(!emissions.contains(&Gesture::Hold));
    }

    #[test]
    fn hold_emits_once_at_threshold_while_held() {
        let mut detector = GestureDetector::new();
        let mut emissions: Vec<(usize, Gesture)> = Vec::new();
        for tick in 0..35usize {
            if let Some(gesture) = detector.tick(true) {
                emissions.push((tick, gesture));
            }
        }
        assert_eq!(emissions.len(), 1);
        assert_eq!(emissions[0].1, Gesture::Hold);
        assert_eq!(emissions[0].0, 29);
        // Release after a hold is silent, and idle stays silent.
        assert_eq!(detector.tick(false), None);
        for _ in 0..25 {
            assert_eq!(detector.tick(false), None);
        }
    }

    #[test]
    fn sub_threshold_noise_and_dead_zone_emit_nothing() {
        // Single-tick blips are noise: shorter than TAP_MIN.
        let blips = run(&script(&[
            &pressed(1),
            &released(5),
            &pressed(1),
            &released(25),
        ]));
        assert!(blips.is_empty(), "noise must emit nothing, got {blips:?}");
        // Dead zone: longer than TAP_MAX but shorter than HOLD_MIN.
        let dead = run(&script(&[&pressed(15), &released(30)]));
        assert!(dead.is_empty(), "dead zone must emit nothing, got {dead:?}");
        // Longest dead-zone press (one tick short of hold) still silent.
        let edge = run(&script(&[&pressed(29), &released(30)]));
        assert!(
            edge.is_empty(),
            "29-tick press must emit nothing, got {edge:?}"
        );
    }

    #[test]
    fn window_overflow_splits_double_into_two_taps() {
        // Gap of 25 exceeds DOUBLE_TAP_MAX_GAP_TICKS (18): two singles.
        let emissions = run(&script(&[
            &pressed(5),
            &released(1),
            &released(25),
            &pressed(5),
            &released(25),
        ]));
        assert_eq!(emissions, vec![Gesture::Tap, Gesture::Tap]);
        assert!(!emissions.contains(&Gesture::DoubleTap));
    }

    #[test]
    fn same_script_replays_identically_x2() {
        let tape = script(&[
            &pressed(5),
            &released(10),
            &pressed(6),
            &released(25),
            &pressed(32),
            &released(5),
        ]);
        assert_eq!(run(&tape), run(&tape));
        assert_eq!(run(&tape), vec![Gesture::DoubleTap, Gesture::Hold]);
    }
}
