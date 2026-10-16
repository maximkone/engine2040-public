//! Startup config: seed, fixed tick, quality tier, worker/journal bounds.
//!
//! Units: seed hash-law u64, tick_hz fixed steps per second, deadlines and
//! costs in milliseconds, journal capacity in entries. Validation runs
//! before anything starts, so a bad config never half-starts an app.

use crate::error::AppError;
use engine_core::TICK_HZ;

/// Render/quality tier requested at startup; extraction honors it later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QualityTier {
    /// Lowest cost presentation.
    Low,
    /// Reference presentation.
    #[default]
    Medium,
    /// Highest cost presentation.
    High,
}

/// What one `App` boots with. Thread the seed through; no globals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppConfig {
    /// Deterministic seed in, hashes out.
    pub seed: u64,
    /// Fixed steps per second; must equal the core 60Hz tick.
    pub tick_hz: u64,
    /// Presentation tier handed to the window loop or editor shell.
    pub quality: QualityTier,
    /// Per-worker shutdown drain deadline in milliseconds.
    pub worker_deadline_ms: u64,
    /// Bounded journal capacity in entries.
    pub journal_capacity: usize,
}

impl Default for AppConfig {
    /// Hash-law defaults: seed 2040, 60Hz, 2s drain deadline, 256 entries.
    fn default() -> Self {
        Self {
            seed: 2040,
            tick_hz: TICK_HZ,
            quality: QualityTier::Medium,
            worker_deadline_ms: 2000,
            journal_capacity: 256,
        }
    }
}

impl AppConfig {
    /// Smallest journal that still holds a full startup plus shutdown.
    pub const MIN_JOURNAL: usize = 16;
    /// Largest journal before the bound stops meaning anything.
    pub const MAX_JOURNAL: usize = 4096;
    /// Longest shutdown drain deadline one worker may claim.
    pub const MAX_DEADLINE_MS: u64 = 30_000;

    /// Build with an explicit seed, hash-law defaults otherwise.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }

    /// Range-check every field; any failure names the culprit.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.tick_hz != TICK_HZ {
            return Err(AppError::BadConfig {
                reason: format!("tick_hz {} must be {TICK_HZ}", self.tick_hz),
            });
        }
        if self.worker_deadline_ms == 0 || self.worker_deadline_ms > Self::MAX_DEADLINE_MS {
            return Err(AppError::BadConfig {
                reason: format!(
                    "worker_deadline_ms {} outside 1..={}",
                    self.worker_deadline_ms,
                    Self::MAX_DEADLINE_MS
                ),
            });
        }
        if self.journal_capacity < Self::MIN_JOURNAL || self.journal_capacity > Self::MAX_JOURNAL {
            return Err(AppError::BadConfig {
                reason: format!(
                    "journal_capacity {} outside {}..={}",
                    self.journal_capacity,
                    Self::MIN_JOURNAL,
                    Self::MAX_JOURNAL
                ),
            });
        }
        Ok(())
    }

    /// The core sim config this app ticks under.
    pub fn core(&self) -> engine_core::config::Config {
        engine_core::config::Config {
            tick_hz: self.tick_hz,
            seed: self.seed,
            ..engine_core::config::Config::default()
        }
    }

    /// Seconds per fixed tick.
    pub fn dt(&self) -> f32 {
        1.0 / self.tick_hz as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_hash_law() {
        let config = AppConfig::default();
        assert_eq!(config.seed, 2040);
        assert_eq!(config.tick_hz, TICK_HZ);
        assert_eq!(config.quality, QualityTier::Medium);
        config.validate().unwrap();
        assert!((config.dt() - 1.0 / 60.0).abs() < f32::EPSILON);
    }

    #[test]
    fn each_field_refuses_its_own_range() {
        assert!(matches!(
            AppConfig {
                tick_hz: 59,
                ..AppConfig::default()
            }
            .validate(),
            Err(AppError::BadConfig { .. })
        ));
        assert!(matches!(
            AppConfig {
                worker_deadline_ms: 0,
                ..AppConfig::default()
            }
            .validate(),
            Err(AppError::BadConfig { .. })
        ));
        assert!(AppConfig {
            journal_capacity: 4,
            ..AppConfig::default()
        }
        .validate()
        .is_err());
        assert!(AppConfig {
            journal_capacity: AppConfig::MAX_JOURNAL + 1,
            ..AppConfig::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn core_config_carries_seed_and_hz() {
        let config = AppConfig::new(7);
        let core = config.core();
        assert_eq!(core.seed, 7);
        assert_eq!(core.tick_hz, TICK_HZ);
    }
}
