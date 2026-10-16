//! eight_fight: the FC3 8-fighter 30 fps gate shape.
//!
//! Consumer: the FC3 quiet perf run (windowed arena, 8 fighters, 1080p).
//! Data owner: this module owns the gate numbers only (fighter count plus
//! frame ceilings); `engine-render` owns the passes that spend the budget.
//! Units: whole microseconds (`u64`), integer math only, no wall clock.
//! Determinism: pure functions, same inputs give same outputs.
//! Errors: [`EightFightError`] at every boundary, never panics.
//!
//! Std-only. Nothing here enters the sim hash (presentation gate only).

/// Fighters in the FC3 windowed arena run.
pub const EIGHT_FIGHTERS: u8 = 8;

/// Frame p50 ceiling: 33.3 ms at 30 fps, in microseconds.
pub const FRAME_P50_BUDGET_US: u64 = 33_300;

/// Frame p99 ceiling: 50 ms, in microseconds.
pub const FRAME_P99_BUDGET_US: u64 = 50_000;

/// Quiet perf runs the accept asks for (x2).
pub const QUIET_RUNS: usize = 2;

/// Smallest tick count a gate run accepts (one present cadence).
pub const MIN_TICKS: u32 = 1;

/// Largest tick count a gate run accepts (mirrors the session cap).
pub const MAX_TICKS: u32 = 100_000;

/// Arena half-extent bounds in meters (mirrors the session config).
pub const MIN_EXTENT_M: f32 = 5.0;
/// Arena half-extent ceiling in meters (mirrors the session config).
pub const MAX_EXTENT_M: f32 = 100.0;

/// Gate failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EightFightError {
    /// Fighter count is not the gate count, culprit carried.
    BadFighters(u8),
    /// Tick count `0` or above [`MAX_TICKS`], culprit carried.
    BadTicks(u32),
    /// Half-extent NaN or outside `MIN_EXTENT_M..=MAX_EXTENT_M`.
    BadExtent,
    /// Measured p50 above [`FRAME_P50_BUDGET_US`]; carries measured plus bar.
    P50Blown { measured_us: u64, bar_us: u64 },
    /// Measured p99 above [`FRAME_P99_BUDGET_US`]; carries measured plus bar.
    P99Blown { measured_us: u64, bar_us: u64 },
}

impl core::fmt::Display for EightFightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BadFighters(n) => write!(f, "eight-fight gate needs 8 fighters, got {n}"),
            Self::BadTicks(n) => write!(f, "bad eight-fight ticks ({n})"),
            Self::BadExtent => write!(f, "bad eight-fight half-extent"),
            Self::P50Blown {
                measured_us,
                bar_us,
            } => write!(
                f,
                "eight-fight p50 blew bar ({measured_us}us over {bar_us}us)"
            ),
            Self::P99Blown {
                measured_us,
                bar_us,
            } => write!(
                f,
                "eight-fight p99 blew bar ({measured_us}us over {bar_us}us)"
            ),
        }
    }
}

impl std::error::Error for EightFightError {}

/// One FC3 gate session: the windowed arena run with 8 fighters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EightFightConfig {
    /// Arena seed (same seed twice gives the same run).
    pub seed: u64,
    /// Fixed steps for the run, `MIN_TICKS..=MAX_TICKS`.
    pub ticks: u32,
    /// Fighters, always [`EIGHT_FIGHTERS`].
    pub fighters: u8,
    /// Arena half-extent in meters, `MIN_EXTENT_M..=MAX_EXTENT_M`.
    pub half_extent_m: f32,
}

impl EightFightConfig {
    /// The gate run: seed 2040, 600 ticks, 8 fighters, 20 m extent.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            seed: 2040,
            ticks: 600,
            fighters: EIGHT_FIGHTERS,
            half_extent_m: 20.0,
        }
    }

    /// Range-check every field; NaN or out-of-range fails with the culprit.
    pub fn validate(self) -> Result<(), EightFightError> {
        if self.fighters != EIGHT_FIGHTERS {
            return Err(EightFightError::BadFighters(self.fighters));
        }
        if self.ticks < MIN_TICKS || self.ticks > MAX_TICKS {
            return Err(EightFightError::BadTicks(self.ticks));
        }
        if !self.half_extent_m.is_finite()
            || self.half_extent_m < MIN_EXTENT_M
            || self.half_extent_m > MAX_EXTENT_M
        {
            return Err(EightFightError::BadExtent);
        }
        Ok(())
    }
}

/// One fighter's even share of a frame budget: `frame_us / fighters`.
/// Integer division, rounds down. `fighters == 0` gives `0`.
#[must_use]
pub fn per_fighter_share_us(frame_us: u64, fighters: u8) -> u64 {
    if fighters == 0 {
        return 0;
    }
    frame_us / u64::from(fighters)
}

/// Check one quiet run's frame percentiles against the gate bars.
/// At the bar passes, one microsecond over trips. Pure.
pub fn check_frame_us(p50_us: u64, p99_us: u64) -> Result<(), EightFightError> {
    if p50_us > FRAME_P50_BUDGET_US {
        return Err(EightFightError::P50Blown {
            measured_us: p50_us,
            bar_us: FRAME_P50_BUDGET_US,
        });
    }
    if p99_us > FRAME_P99_BUDGET_US {
        return Err(EightFightError::P99Blown {
            measured_us: p99_us,
            bar_us: FRAME_P99_BUDGET_US,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_config_validates() {
        EightFightConfig::standard()
            .validate()
            .expect("standard gate run validates");
    }

    #[test]
    fn wrong_fighter_count_fails() {
        let bad = EightFightConfig {
            fighters: 2,
            ..EightFightConfig::standard()
        };
        assert_eq!(bad.validate(), Err(EightFightError::BadFighters(2)));
        let bad_ticks = EightFightConfig {
            ticks: 0,
            ..EightFightConfig::standard()
        };
        assert_eq!(bad_ticks.validate(), Err(EightFightError::BadTicks(0)));
        let bad_extent = EightFightConfig {
            half_extent_m: f32::NAN,
            ..EightFightConfig::standard()
        };
        assert_eq!(bad_extent.validate(), Err(EightFightError::BadExtent));
    }

    #[test]
    fn gate_bars_hold_at_the_bar_and_trip_one_over() {
        check_frame_us(FRAME_P50_BUDGET_US, FRAME_P99_BUDGET_US).expect("at-bar passes");
        check_frame_us(0, 0).expect("headroom passes");
        assert_eq!(
            check_frame_us(FRAME_P50_BUDGET_US + 1, FRAME_P99_BUDGET_US),
            Err(EightFightError::P50Blown {
                measured_us: FRAME_P50_BUDGET_US + 1,
                bar_us: FRAME_P50_BUDGET_US,
            })
        );
        assert_eq!(
            check_frame_us(FRAME_P50_BUDGET_US, FRAME_P99_BUDGET_US + 1),
            Err(EightFightError::P99Blown {
                measured_us: FRAME_P99_BUDGET_US + 1,
                bar_us: FRAME_P99_BUDGET_US,
            })
        );
    }

    #[test]
    fn per_fighter_share_is_even_and_safe() {
        assert_eq!(
            per_fighter_share_us(FRAME_P50_BUDGET_US, EIGHT_FIGHTERS),
            4_162
        );
        assert_eq!(per_fighter_share_us(100, 0), 0);
        // Same inputs twice give the same share.
        assert_eq!(
            per_fighter_share_us(50_000, 8),
            per_fighter_share_us(50_000, 8)
        );
    }
}
