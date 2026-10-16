//! config: typed sim config + budgets + feature flags, no globals. Defaults: 60Hz, seed 2040.
//! Owner: engine-core; Config/Budgets/Flags built per run, never global.
//! Units: tick_hz (Hz, u64), seed (u64), budgets (counts: ticks, actors), dt seconds (f32).
//! Determinism: plain data; same fields give the same tick stream and hash.
//! Errors: infallible (defaults valid; dt defined for nonzero tick_hz from Default/new).
//! Budget: Copy structs, no allocation; consumers enforce max_ticks/max_actors.

use std::fmt;

use crate::time::{IntAcc, NS_PER_SEC};

/// Typed config errors; invalid external input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// Tick rate was zero, so seconds-per-tick is undefined.
    ZeroTickHz,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTickHz => write!(f, "tick_hz must be non-zero"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Per-run resource budgets (counts, never bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    /// Max ticks per run.
    pub max_ticks: u64,
    /// Max live actors/bodies.
    pub max_actors: u32,
}

impl Default for Budgets {
    /// Default budgets: 600 ticks, 1024 actors.
    fn default() -> Self {
        Self {
            max_ticks: 600,
            max_actors: 1024,
        }
    }
}

/// Explicit per-run feature flags, never global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    /// Enforce fixed-step determinism gate.
    pub strict_determinism: bool,
    /// Collect debug overlay data.
    pub debug_overlay: bool,
}

impl Default for Flags {
    /// Default flags: strict on, overlay off.
    fn default() -> Self {
        Self {
            strict_determinism: true,
            debug_overlay: false,
        }
    }
}

/// Typed sim config. Thread seed + tick through; no globals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Ticks per second.
    pub tick_hz: u64,
    /// Deterministic seed in, hash out.
    pub seed: u64,
    /// Resource budgets.
    pub budgets: Budgets,
    /// Feature flags.
    pub flags: Flags,
}

impl Default for Config {
    /// Hash-law defaults: 60Hz, seed 2040.
    fn default() -> Self {
        Self {
            tick_hz: crate::TICK_HZ,
            seed: 2040,
            budgets: Budgets::default(),
            flags: Flags::default(),
        }
    }
}

impl Config {
    /// Construct with explicit seed, hash-law defaults otherwise.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }

    /// Refusing constructor: rejects `tick_hz == 0` with `ZeroTickHz`.
    pub fn try_new(seed: u64, tick_hz: u64) -> Result<Self, ConfigError> {
        let cfg = Self {
            seed,
            tick_hz,
            ..Self::default()
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Range-check every field; zero tick rate refuses with `ZeroTickHz`.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.tick_hz == 0 {
            return Err(ConfigError::ZeroTickHz);
        }
        Ok(())
    }

    /// Frozen legacy step in seconds (1.0 / tick_hz as f32, never panics).
    /// At 60Hz the bits equal `tick::LEGACY_DT`; f32 division by zero yields
    /// infinity, never a panic. This holds the single f32 conversion site.
    pub fn legacy_dt(&self) -> f32 {
        1.0 / self.tick_hz as f32
    }

    /// Refusing seconds-per-tick; errors on `tick_hz == 0`.
    pub fn try_dt(&self) -> Result<f32, ConfigError> {
        if self.tick_hz == 0 {
            return Err(ConfigError::ZeroTickHz);
        }
        Ok(self.legacy_dt())
    }

    /// Seconds per tick; saturates to crate `DT` on `tick_hz == 0`, never inf.
    /// `legacy_dt` keeps the frozen inf form for receipts.
    pub fn dt(&self) -> f32 {
        self.try_dt().unwrap_or(crate::DT)
    }

    /// Nanoseconds per tick, saturating, never panics.
    /// At 60Hz equals the integer step whole-ns part (16_666_666);
    /// zero Hz saturates at `u64::MAX`.
    pub fn dt_ns(&self) -> u64 {
        NS_PER_SEC.checked_div(self.tick_hz).unwrap_or(u64::MAX)
    }

    /// Default integer accumulator; `Some` only at 60Hz, else `None`.
    /// Saturating and never panics; the `IntAcc` is empty (tick 0).
    pub fn int_acc(&self) -> Option<IntAcc> {
        if self.tick_hz == crate::TICK_HZ {
            Some(IntAcc::new())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_hash_law() {
        let c = Config::default();
        assert_eq!(c.tick_hz, 60);
        assert_eq!(c.tick_hz, crate::TICK_HZ);
        assert_eq!(c.seed, 2040);
    }

    #[test]
    fn new_keeps_defaults_except_seed() {
        let c = Config::new(7);
        assert_eq!(c.seed, 7);
        assert_eq!(c.tick_hz, Config::default().tick_hz);
        assert_eq!(c.budgets, Budgets::default());
        assert_eq!(c.flags, Flags::default());
    }

    #[test]
    fn dt_matches_hz() {
        let dt = Config::default().dt();
        assert!((dt - 1.0 / 60.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_hz_refuses_typed() {
        assert_eq!(Config::try_new(2040, 0), Err(ConfigError::ZeroTickHz));
        let zero = Config {
            tick_hz: 0,
            ..Config::default()
        };
        assert_eq!(zero.validate(), Err(ConfigError::ZeroTickHz));
        assert_eq!(zero.try_dt(), Err(ConfigError::ZeroTickHz));
        assert_eq!(
            format!("{}", ConfigError::ZeroTickHz),
            "tick_hz must be non-zero"
        );
        Config::try_new(2040, 60).expect("60Hz builds");
        Config::default().validate().expect("default valid");
    }

    #[test]
    fn dt_saturates_on_zero_never_inf() {
        let zero = Config {
            tick_hz: 0,
            ..Config::default()
        };
        let dt = zero.dt();
        assert!(dt.is_finite(), "dt on zero hz must not be inf, got {dt}");
        assert_eq!(dt, crate::DT);
    }

    #[test]
    fn dt_delegates_same_bits_as_legacy() {
        for hz in [1u64, 30, 60, 120, 240] {
            let c = Config {
                tick_hz: hz,
                ..Config::default()
            };
            assert_eq!(
                c.dt().to_bits(),
                c.legacy_dt().to_bits(),
                "dt must delegate with identical bits at {hz}Hz"
            );
        }
        assert_eq!(
            Config::default().legacy_dt().to_bits(),
            crate::tick::LEGACY_DT.to_bits()
        );
    }

    #[test]
    fn dt_ns_saturating_never_panics() {
        assert_eq!(Config::default().dt_ns(), 16_666_666);
        assert_eq!(Config::default().dt_ns(), crate::time::STEP_NS);
        let hz30 = Config {
            tick_hz: 30,
            ..Config::default()
        };
        assert_eq!(hz30.dt_ns(), 33_333_333);
        let hz0 = Config {
            tick_hz: 0,
            ..Config::default()
        };
        assert_eq!(hz0.dt_ns(), u64::MAX);
        assert_eq!(hz0.legacy_dt(), f32::INFINITY);
        // dt saturates to DT on zero hz while legacy_dt keeps frozen inf.
        assert_eq!(hz0.dt(), crate::DT);
        assert_ne!(hz0.dt().to_bits(), hz0.legacy_dt().to_bits());
        let hzmax = Config {
            tick_hz: u64::MAX,
            ..Config::default()
        };
        assert_eq!(hzmax.dt_ns(), 0);
        assert!(hzmax.int_acc().is_none());
    }

    #[test]
    fn int_acc_some_only_at_60hz() {
        let at60 = Config::default().int_acc().expect("60Hz must give Some");
        assert_eq!(at60.tick(), 0);
        assert_eq!(at60.buffered_ns(), 0);
        let hz30 = Config {
            tick_hz: 30,
            ..Config::default()
        };
        assert!(hz30.int_acc().is_none());
        let hz0 = Config {
            tick_hz: 0,
            ..Config::default()
        };
        assert!(hz0.int_acc().is_none());
    }

    #[test]
    fn int_acc_default_path_takes_60_per_second() {
        let c = Config::default();
        let mut acc = c.int_acc().expect("60Hz must give Some");
        let mut steps: u64 = 0;
        for _ in 0..1000 {
            steps += u64::from(acc.push_ns(1_000_000).expect("1ms bounded"));
        }
        assert_eq!(steps, 60);
        assert_eq!(acc.tick(), 60);
    }
}
