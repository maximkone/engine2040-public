//! laws: physics laws any fixed-tick sim proves in a test, in a few lines.
//! Owner: engine-core (feature `test-util`). Each check steps the caller's state with the
//! caller's step closure and returns `Err(Break)` naming the law, the tick and the state when
//! the law fails, so a red test reads as physics, not as a diff.
//!
//! ```ignore
//! use engine_core::laws;
//! // A walker with no input on flat ground never moves, not by one millimetre.
//! laws::rest_stays_rest(parked, 600, |w| step_walker(w, &idle, &world, &cfg)).unwrap();
//! // Walking left is walking right, mirrored, tick for tick.
//! laws::mirror(walk_east, walk_west, 300, |w, _| step_walker(w, ...), |a, b| a.x == -b.x).unwrap();
//! ```
//!
//! What each law catches:
//! - rest stays rest: creep, jitter, bounce at rest, gravity leaking through a ground clamp;
//! - replay: hidden inputs (wall clock, hash-map order, a carry that two copies do not share);
//! - mirror: floor-rounding bias (`>>` on negatives), logic that handles one sign only (a slope
//!   solid when it rises along +x and hollow when it falls);
//! - energy never rises: explicit-Euler springs, damping past one per step, bounce gain;
//! - conserved: momentum, mass or count leaking in an exchange;
//! - steady rate: distance = speed x time at every speed (truncation dead zones: `v / 60` in
//!   integer millimetres stalls everything under 60 mm/s).
//!
//! Determinism: the checks only step and compare; they add no state to the sim.

use std::fmt;

/// A broken law: which, at what tick, and what the state looked like.
#[derive(Debug, Clone, PartialEq)]
pub struct Break {
    /// The law's name, as in the module list.
    pub law: &'static str,
    /// The first tick (1-based: after that many steps) where it failed; 0 means the start.
    pub tick: u32,
    /// The states or numbers involved, `{:?}`-printed.
    pub detail: String,
}

impl fmt::Display for Break {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "law '{}' broke at tick {}: {}",
            self.law, self.tick, self.detail
        )
    }
}

impl std::error::Error for Break {}

/// A state with nothing acting on it stays bit-identical for `ticks` steps.
pub fn rest_stays_rest<S: Clone + PartialEq + fmt::Debug>(
    start: S,
    ticks: u32,
    mut step: impl FnMut(&mut S),
) -> Result<(), Break> {
    let mut state = start.clone();
    for tick in 1..=ticks {
        step(&mut state);
        if state != start {
            return Err(Break {
                law: "rest stays rest",
                tick,
                detail: format!("started {start:?}, now {state:?}"),
            });
        }
    }
    Ok(())
}

/// Two copies of `start` stepped side by side stay equal every tick. `step` gets the tick index
/// (0-based) so scripted input can vary over time. Returns the final state.
pub fn replay<S: Clone + PartialEq + fmt::Debug>(
    start: S,
    ticks: u32,
    mut step: impl FnMut(&mut S, u32),
) -> Result<S, Break> {
    let mut a = start.clone();
    let mut b = start;
    for tick in 0..ticks {
        step(&mut a, tick);
        step(&mut b, tick);
        if a != b {
            return Err(Break {
                law: "replay",
                tick: tick + 1,
                detail: format!("first run {a:?}, second run {b:?}"),
            });
        }
    }
    Ok(a)
}

/// Mirrored starts stay mirrored: `mirrored(a, b)` holds at the start and after every step.
/// `step` gets the tick index (0-based); feed each side its mirrored input from it.
pub fn mirror<S: fmt::Debug>(
    mut a: S,
    mut b: S,
    ticks: u32,
    mut step: impl FnMut(&mut S, u32),
    mirrored: impl Fn(&S, &S) -> bool,
) -> Result<(), Break> {
    if !mirrored(&a, &b) {
        return Err(Break {
            law: "mirror",
            tick: 0,
            detail: format!("the starts are not mirrored: {a:?} vs {b:?}"),
        });
    }
    for tick in 0..ticks {
        step(&mut a, tick);
        step(&mut b, tick);
        if !mirrored(&a, &b) {
            return Err(Break {
                law: "mirror",
                tick: tick + 1,
                detail: format!("{a:?} vs {b:?}"),
            });
        }
    }
    Ok(())
}

/// With no input, `energy` never rises by more than `slack` from one tick to the next, and
/// stays finite. Pick `slack` from the sim's rounding (one unit squared for integer lanes), not
/// from the size of the failure.
pub fn energy_never_rises<S: fmt::Debug>(
    mut state: S,
    ticks: u32,
    mut step: impl FnMut(&mut S),
    energy: impl Fn(&S) -> f64,
    slack: f64,
) -> Result<(), Break> {
    let mut before = energy(&state);
    for tick in 1..=ticks {
        step(&mut state);
        let now = energy(&state);
        if !now.is_finite() || now > before + slack {
            return Err(Break {
                law: "energy never rises",
                tick,
                detail: format!("energy {before} -> {now} (slack {slack}), state {state:?}"),
            });
        }
        before = now;
    }
    Ok(())
}

/// `quantity` (momentum, mass, a count) stays within `slack` of its start value every tick.
pub fn conserved<S: fmt::Debug>(
    mut state: S,
    ticks: u32,
    mut step: impl FnMut(&mut S),
    quantity: impl Fn(&S) -> f64,
    slack: f64,
) -> Result<(), Break> {
    let start = quantity(&state);
    for tick in 1..=ticks {
        step(&mut state);
        let now = quantity(&state);
        if !now.is_finite() || (now - start).abs() > slack {
            return Err(Break {
                law: "conserved",
                tick,
                detail: format!("{start} -> {now} (slack {slack}), state {state:?}"),
            });
        }
    }
    Ok(())
}

/// Distance = rate x time: after `warm` ticks of settling, the next `ticks` steps move
/// `measure` by `per_tick * ticks` within `slack`. Run it at slow, odd and fast rates: a
/// truncating integer step passes at round speeds and fails at the rest.
pub fn steady_rate<S: fmt::Debug>(
    mut state: S,
    warm: u32,
    ticks: u32,
    mut step: impl FnMut(&mut S),
    measure: impl Fn(&S) -> f64,
    per_tick: f64,
    slack: f64,
) -> Result<(), Break> {
    for _ in 0..warm {
        step(&mut state);
    }
    let from = measure(&state);
    for _ in 0..ticks {
        step(&mut state);
    }
    let moved = measure(&state) - from;
    let want = per_tick * f64::from(ticks);
    if !moved.is_finite() || (moved - want).abs() > slack {
        return Err(Break {
            law: "steady rate",
            tick: warm + ticks,
            detail: format!(
                "moved {moved} in {ticks} ticks, want {want} (slack {slack}), state {state:?}"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A point on a line in integer millimetres with a velocity in mm per second at 60 Hz.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Mover {
        x: i32,
        v: i32,
        carry: i32,
    }

    fn truncating(m: &mut Mover) {
        m.x += m.v / 60;
    }

    fn carrying(m: &mut Mover) {
        let total = m.v + m.carry;
        m.x += total / 60;
        m.carry = total % 60;
    }

    #[test]
    fn a_break_names_its_tick_and_an_exact_rise_is_inside_the_energy_law() {
        let broke = Break {
            law: "steady rate",
            tick: 7,
            detail: "moved 0".into(),
        };
        assert_eq!(
            broke.to_string(),
            "law 'steady rate' broke at tick 7: moved 0"
        );
        let mut calls = 0_i32;
        let hidden = replay(
            Mover {
                x: 0,
                v: 0,
                carry: 0,
            },
            3,
            |m, _| {
                calls += 1;
                m.x += calls;
            },
        )
        .expect_err("a step that is not a function of the state breaks replay");
        assert_eq!(
            (hidden.law, hidden.tick),
            ("replay", 1),
            "the first tick is named, not tick zero"
        );
        assert!(
            energy_never_rises(0u32, 3, |s| *s += 1, |s| f64::from(*s), 1.0).is_ok(),
            "a rise of exactly the slack is inside the law"
        );
        let mover = Mover {
            x: 100,
            v: 120,
            carry: 0,
        };
        assert!(
            steady_rate(mover, 10, 10, carrying, |m| f64::from(m.x), 2.0, 0.0).is_ok(),
            "20 mm in 10 ticks after the warm-up is exactly the rate"
        );
        let slow = Mover {
            x: 0,
            v: 59,
            carry: 0,
        };
        let dead = steady_rate(slow, 2, 3, truncating, |m| f64::from(m.x), 59.0 / 60.0, 0.0)
            .expect_err("the truncation dead zone breaks the rate");
        assert_eq!(
            (dead.law, dead.tick),
            ("steady rate", 5),
            "the tick counts the warm-up"
        );
    }

    #[test]
    fn steady_rate_catches_a_truncation_dead_zone() {
        let slow = Mover {
            x: 0,
            v: 59,
            carry: 0,
        };
        let broke = steady_rate(
            slow,
            0,
            600,
            truncating,
            |m| f64::from(m.x),
            59.0 / 60.0,
            1.0,
        );
        assert_eq!(broke.map_err(|b| b.law), Err("steady rate"));
        assert!(steady_rate(slow, 0, 600, carrying, |m| f64::from(m.x), 59.0 / 60.0, 1.0).is_ok());
    }

    #[test]
    fn rest_stays_rest_catches_creep() {
        let parked = Mover {
            x: 5,
            v: 0,
            carry: 0,
        };
        assert!(rest_stays_rest(parked, 100, carrying).is_ok());
        let creep = rest_stays_rest(parked, 100, |m| m.x += 1).expect_err("creep must break");
        assert_eq!((creep.law, creep.tick), ("rest stays rest", 1));
    }

    #[test]
    fn mirror_catches_floor_rounding_bias() {
        let east = Mover {
            x: 5,
            v: 0,
            carry: 0,
        };
        let west = Mover {
            x: -5,
            v: 0,
            carry: 0,
        };
        let same = |a: &Mover, b: &Mover| a.x == -b.x;
        // Truncation toward zero is sign-symmetric; an arithmetic shift floors and is not.
        assert!(mirror(east, west, 20, |m, _| m.x = m.x * 7 / 8, same).is_ok());
        let bias =
            mirror(east, west, 20, |m, _| m.x = (m.x * 7) >> 3, same).expect_err("floor bias");
        assert_eq!(bias.tick, 1);
    }

    #[test]
    fn energy_catches_an_explicit_euler_spring() {
        // x'' = -x. Explicit Euler (both updates from the old state) gains energy every step;
        // semi-implicit Euler with light damping never does.
        let energy = |s: &(f64, f64)| 0.5 * (s.0 * s.0 + s.1 * s.1);
        let h = 0.1;
        let explicit = |s: &mut (f64, f64)| *s = (s.0 + h * s.1, s.1 - h * s.0);
        let damped = |s: &mut (f64, f64)| {
            s.1 = (s.1 - h * s.0) * 0.99;
            s.0 += h * s.1;
        };
        let gained = energy_never_rises((1.0, 0.0), 100, explicit, energy, 1e-9);
        assert_eq!(gained.map_err(|b| b.law), Err("energy never rises"));
        // Semi-implicit energy wobbles inside a band of order h, so the slack is h-sized.
        assert!(energy_never_rises((1.0, 0.0), 1_000, damped, energy, h * 0.05).is_ok());
    }

    #[test]
    fn conserved_catches_a_leaking_exchange() {
        // Two bodies trade momentum; a fair trade conserves the sum, a lossy one does not.
        let momentum = |s: &(i32, i32)| f64::from(s.0 + s.1);
        assert!(conserved((10, -4), 50, |s| *s = (s.1, s.0), momentum, 0.0).is_ok());
        let leak = conserved((10, -4), 50, |s| *s = (s.1, s.0 / 2), momentum, 0.0);
        assert_eq!(leak.map_err(|b| b.tick), Err(1));
    }

    #[test]
    fn replay_catches_a_hidden_input() {
        let mut calls = 0_i32;
        let hidden = replay(
            Mover {
                x: 0,
                v: 0,
                carry: 0,
            },
            10,
            |m, _| {
                calls += 1;
                m.x += calls;
            },
        );
        assert_eq!(hidden.map_err(|b| b.law), Err("replay"));
        let fair = replay(
            Mover {
                x: 0,
                v: 120,
                carry: 0,
            },
            10,
            |m, _| carrying(m),
        );
        assert_eq!(fair.map(|m| m.x), Ok(20));
    }
}
