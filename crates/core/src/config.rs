//! config: typed sim config + budgets, no globals. Defaults: 60Hz, seed 2040.
//! Owner: engine-core; Config/Budgets built per run, never global.
//! Units: tick_hz (Hz, u64), seed (u64), budgets (counts: ticks), dt seconds (f32).
//! Determinism: plain data; same fields give the same tick stream and hash.
//! Errors: infallible (defaults valid; dt defined for nonzero tick_hz from Default/new).
//! Budget: Copy structs, no allocation; consumers enforce max_ticks.

use std::fmt;

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
}

impl Default for Budgets {
    /// Default budgets: 600 ticks.
    fn default() -> Self {
        Self { max_ticks: 600 }
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
}

impl Default for Config {
    /// Hash-law defaults: 60Hz, seed 2040.
    fn default() -> Self {
        Self {
            tick_hz: crate::TICK_HZ,
            seed: 2040,
            budgets: Budgets::default(),
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

    /// Seconds per tick; saturates to crate `DT` on `tick_hz == 0`, never inf.
    pub fn dt(&self) -> f32 {
        if self.tick_hz == 0 {
            crate::DT
        } else {
            1.0 / self.tick_hz as f32
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
}
