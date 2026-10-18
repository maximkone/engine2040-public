//! virt: virtual time for the window's tick accumulator (GP45).
//!
//! Owner: game lane. Consumer: `bin/arena.rs` (`ticks_owed`) passes every wall
//! frame's milliseconds through [`VirtualClock::advance`] before the fixed-tick
//! accumulator sees them, so slow-mo, fast-forward and the dev console's `slow`
//! verb change how many ticks a wall second owes and never what a tick
//! computes. PATTERN(Bevy `Time<Virtual>`: a relative speed that scales the
//! frame delta ahead of the fixed step).
//! Units: wall milliseconds in, virtual milliseconds out; speeds in permille
//! of real time (1000 is real time). The accumulator still owes at most two
//! ticks a frame, so a fast-forward past that is its guard's to cap. Determinism: float math at the frame
//! boundary only; the sim is fed whole ticks, never reads the clock, and the
//! tick hash never moves. A net match locks the clock at real time
//! ([`VirtualClock::lock_real_time`]): peers cannot agree on a private speed.
//! Errors: [`VirtError`] for a speed out of range or a locked clock; nothing
//! panics. Budget: a few float ops per frame, no heap.

use core::fmt;

/// Real time: the speed with no scale in effect, permille.
pub const REAL_SPEED_PERMILLE: u32 = 1000;
/// Slowest speed the clock accepts, permille: a hundredth of real time.
pub const MIN_SPEED_PERMILLE: u32 = 10;
/// Fastest speed the clock accepts, permille: four times real time.
pub const MAX_SPEED_PERMILLE: u32 = 4000;
/// Permille per whole ratio, for the speed ratio the clock multiplies by.
const PERMILLE: f64 = 1000.0;
/// Milliseconds per second, for ramp rates kept in ratio per second.
const MS_PER_S: f64 = 1000.0;
/// Speeds the window's slow-mo keys step through, permille, slowest first. The
/// fastest is what the accumulator's two-ticks-a-frame spiral guard carries at 60 fps.
pub const SPEED_STEPS: [u32; 4] = [250, 500, REAL_SPEED_PERMILLE, 2000];
/// Half a permille: speeds closer than this are the same speed.
const SPEED_EPS_PERMILLE: f64 = 0.5;
/// Wall milliseconds a key-driven speed change eases over: long enough to read
/// as a ramp, short enough to answer the key at once.
pub const KEY_RAMP_MS: u32 = 250;

/// The preset one step faster (`faster`) or slower than `current_permille`,
/// held at the ends of [`SPEED_STEPS`]. A speed between presets (a ramp in
/// flight reads as one) steps to the nearest preset in that direction.
#[must_use]
pub fn step_speed(current_permille: f64, faster: bool) -> u32 {
    let first = SPEED_STEPS[0];
    let last = SPEED_STEPS[SPEED_STEPS.len() - 1];
    if faster {
        SPEED_STEPS
            .into_iter()
            .find(|step| f64::from(*step) > current_permille + SPEED_EPS_PERMILLE)
            .unwrap_or(last)
    } else {
        SPEED_STEPS
            .into_iter()
            .rev()
            .find(|step| f64::from(*step) < current_permille - SPEED_EPS_PERMILLE)
            .unwrap_or(first)
    }
}

/// Title-bar note for a speed other than real time (`speed 50%`), `None` at
/// real time so a normal run shows nothing.
#[must_use]
pub fn speed_label(speed_permille: f64) -> Option<String> {
    ((speed_permille - f64::from(REAL_SPEED_PERMILLE)).abs() > SPEED_EPS_PERMILLE)
        .then(|| format!("speed {:.0}%", speed_permille / 10.0))
}

/// Why the clock refused a speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtError {
    /// The speed (permille, carried) is outside
    /// [`MIN_SPEED_PERMILLE`]`..=`[`MAX_SPEED_PERMILLE`].
    SpeedOutOfRange(u32),
    /// The clock is locked at real time (a net match).
    Locked,
}

impl fmt::Display for VirtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpeedOutOfRange(permille) => write!(
                f,
                "speed {permille} permille is outside {MIN_SPEED_PERMILLE}..={MAX_SPEED_PERMILLE}"
            ),
            Self::Locked => write!(f, "time is locked at real speed in a net match"),
        }
    }
}

impl std::error::Error for VirtError {}

/// Virtual time: a speed that eases toward a target, scaling wall time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualClock {
    /// Speed now, as a ratio of real time.
    speed: f64,
    /// Speed the clock is easing toward.
    target: f64,
    /// Ratio change per wall second while easing; infinite jumps at once.
    ramp_per_s: f64,
    /// A net match: always real time.
    locked: bool,
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualClock {
    /// A clock at real time with no scale pending.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            speed: 1.0,
            target: 1.0,
            ramp_per_s: f64::INFINITY,
            locked: false,
        }
    }

    /// Ease toward `speed_permille` over `ramp_ms` wall milliseconds (zero
    /// jumps at once). A running ease restarts from the speed it has reached.
    /// Errors: [`VirtError::SpeedOutOfRange`] outside the accepted range,
    /// [`VirtError::Locked`] on a locked clock; the clock is unchanged.
    pub fn set_speed(&mut self, speed_permille: u32, ramp_ms: u32) -> Result<(), VirtError> {
        if self.locked {
            return Err(VirtError::Locked);
        }
        if !(MIN_SPEED_PERMILLE..=MAX_SPEED_PERMILLE).contains(&speed_permille) {
            return Err(VirtError::SpeedOutOfRange(speed_permille));
        }
        self.target = f64::from(speed_permille) / PERMILLE;
        if ramp_ms == 0 {
            self.speed = self.target;
            self.ramp_per_s = f64::INFINITY;
        } else {
            self.ramp_per_s = (self.target - self.speed).abs() * MS_PER_S / f64::from(ramp_ms);
        }
        Ok(())
    }

    /// Pin the clock at real time for the rest of its life: a net match.
    pub fn lock_real_time(&mut self) {
        *self = Self {
            locked: true,
            ..Self::new()
        };
    }

    /// Speed now in permille of real time: fractional while an ease runs.
    #[must_use]
    pub fn speed_permille(&self) -> f64 {
        self.speed * PERMILLE
    }

    /// True when the clock is locked at real time.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Scale `wall_ms` of wall time into virtual milliseconds, easing the
    /// speed toward its target over the interval: the result is the exact
    /// integral of the linear ease, so any frame rate covers the same virtual
    /// time. A non-finite or non-positive input is no time at all.
    pub fn advance(&mut self, wall_ms: f64) -> f64 {
        if !wall_ms.is_finite() || wall_ms <= 0.0 {
            return 0.0;
        }
        if self.locked {
            return wall_ms;
        }
        let from = self.speed;
        let reach = self.ramp_per_s * wall_ms / MS_PER_S;
        let delta = self.target - from;
        // Land on the target exactly, never a rounding step short of it.
        let to = if delta.abs() <= reach {
            self.target
        } else {
            from + delta.signum() * reach
        };
        self.speed = to;
        wall_ms * 0.5 * (from + to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windowed::TickAccumulator;
    use crate::{App, GameConfig, HUMAN_SLOT};
    use engine_input::{FighterId, FighterInput};

    /// One 60 Hz wall frame, milliseconds.
    const FRAME_MS: f64 = 1000.0 / 60.0;

    fn idle(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).expect("human slot is a fighter"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons(0),
        }
    }

    /// Ticks the accumulator owes for `frames` wall frames of `frame_ms`
    /// through `clock`, each one run on `app` as it is owed.
    fn run_scaled(app: &mut App, clock: &mut VirtualClock, frames: u32, frame_ms: f64) -> u32 {
        let mut acc = TickAccumulator::new();
        let mut ticks = 0;
        for _ in 0..frames {
            for _ in 0..acc.push_elapsed_ms(clock.advance(frame_ms)) {
                app.tick(idle(app.cursor()));
                ticks += 1;
            }
        }
        ticks
    }

    fn app() -> App {
        App::new(GameConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
        })
        .expect("session builds")
    }

    #[test]
    fn time_scale_quarter_speed_gives_30_ticks_and_the_same_digest() {
        let mut slow = app();
        let mut clock = VirtualClock::new();
        clock.set_speed(250, 0).expect("a quarter is in range");
        // 120 wall frames is two seconds; at a quarter that is half a second.
        let ticks = run_scaled(&mut slow, &mut clock, 120, FRAME_MS);
        assert!(
            (29..=31).contains(&ticks),
            "owed {ticks} ticks, want 30 +-1"
        );
        // The same ticks at real speed leave the same digest at every tick: the
        // scale changed how many ticks were owed, never what a tick computes.
        let mut real = app();
        for _ in 0..ticks {
            real.tick(idle(real.cursor()));
        }
        assert_eq!(slow.cursor(), ticks);
        assert_eq!(slow.checkpoints(), real.checkpoints(), "digests match");
        let mut again = app();
        let mut clock = VirtualClock::new();
        clock.set_speed(250, 0).expect("a quarter is in range");
        run_scaled(&mut again, &mut clock, 120, FRAME_MS);
        assert_eq!(
            slow.checkpoints(),
            again.checkpoints(),
            "the scaled run replays x2"
        );
    }

    #[test]
    fn time_scale_ramp_eases_and_covers_the_same_time_at_any_frame_rate() {
        // Real time down to a quarter over 500 ms: the ease is linear, so the
        // virtual time over 2 s is the ramp's trapezoid plus the quarter tail.
        let trapezoid = 500.0 * 0.5 * (1.0 + 0.25) + 1500.0 * 0.25;
        for (frame_ms, frames) in [
            (1000.0 / 30.0, 60),
            (1000.0 / 60.0, 120),
            (1000.0 / 144.0, 288),
        ] {
            let mut clock = VirtualClock::new();
            clock.set_speed(250, 500).expect("in range");
            let total: f64 = (0..frames).map(|_| clock.advance(frame_ms)).sum();
            assert!(
                (total - trapezoid).abs() < 1.0e-6,
                "{frame_ms} ms frames: {total} virtual ms, want {trapezoid}"
            );
            assert_eq!(clock.speed_permille(), 250.0, "the ease has landed");
        }
        // Half way through the ease the speed is half way between the two.
        let mut clock = VirtualClock::new();
        clock.set_speed(250, 500).expect("in range");
        clock.advance(250.0);
        assert!((clock.speed_permille() - 625.0).abs() < 1.0e-9);
    }

    #[test]
    fn time_scale_a_net_match_is_locked_at_real_time() {
        let mut clock = VirtualClock::new();
        clock.set_speed(250, 0).expect("in range");
        clock.lock_real_time();
        assert!(clock.is_locked());
        assert_eq!(clock.speed_permille(), 1000.0);
        assert_eq!(clock.advance(16.0), 16.0, "a locked clock never scales");
        assert_eq!(clock.set_speed(500, 0), Err(VirtError::Locked));
        assert_eq!(
            clock.advance(16.0),
            16.0,
            "and a refused speed changes nothing"
        );
    }

    #[test]
    fn time_scale_refuses_bad_speeds_and_gives_no_time_for_bad_deltas() {
        let mut clock = VirtualClock::new();
        for bad in [0, MIN_SPEED_PERMILLE - 1, MAX_SPEED_PERMILLE + 1, u32::MAX] {
            assert_eq!(
                clock.set_speed(bad, 0),
                Err(VirtError::SpeedOutOfRange(bad))
            );
        }
        assert_eq!(clock.speed_permille(), 1000.0, "refusals change nothing");
        for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
            assert_eq!(clock.advance(bad), 0.0, "{bad} ms is no time");
        }
        assert_eq!(clock.advance(16.0), 16.0, "still real time");
        assert!(VirtError::Locked.to_string().contains("locked"));
        assert!(VirtError::SpeedOutOfRange(5).to_string().contains('5'));
    }

    #[test]
    fn time_scale_keys_step_through_the_presets_and_hold_at_the_ends() {
        assert_eq!(step_speed(1000.0, false), 500);
        assert_eq!(step_speed(500.0, false), 250);
        assert_eq!(step_speed(250.0, false), 250, "slowest holds");
        assert_eq!(step_speed(1000.0, true), 2000);
        assert_eq!(step_speed(2000.0, true), 2000, "fastest holds");
        // A ramp in flight lands between presets and steps to the neighbor.
        assert_eq!(step_speed(625.0, false), 500);
        assert_eq!(step_speed(625.0, true), 1000);
        // Every preset is a speed the clock accepts.
        for step in SPEED_STEPS {
            assert!(VirtualClock::new().set_speed(step, KEY_RAMP_MS).is_ok());
        }
        assert_eq!(speed_label(1000.0), None);
        assert_eq!(speed_label(250.0).as_deref(), Some("speed 25%"));
    }

    #[test]
    fn time_scale_fast_forward_is_capped_by_the_two_ticks_a_frame_guard() {
        // Twice real speed owes two seconds of ticks in one wall second ...
        let mut clock = VirtualClock::new();
        clock.set_speed(2000, 0).expect("in range");
        let twice = run_scaled(&mut app(), &mut clock, 60, FRAME_MS);
        assert!(
            (119..=121).contains(&twice),
            "two seconds of ticks, got {twice}"
        );
        // ... and four times real speed is held to two ticks a frame.
        let mut clock = VirtualClock::new();
        clock.set_speed(MAX_SPEED_PERMILLE, 0).expect("in range");
        let capped = run_scaled(&mut app(), &mut clock, 60, FRAME_MS);
        assert!(
            (119..=121).contains(&capped),
            "capped at 2 a frame, got {capped}"
        );
    }
}
