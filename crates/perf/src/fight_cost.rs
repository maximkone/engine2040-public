//! fight_cost: frame-cost percentiles behind the FC3 30 fps gate.
//!
//! Status: test-only (no production caller reads `check_gate` yet; exercised
//! by the unit tests in this file and the wiring vectors). Tracked by FC1
//! (frame cost table for the windowed arena). Data owner: this module owns
//! the percentile math plus the gate check; the runner owns the samples.
//! Units: whole microseconds (`u64`), integer math only, no wall clock.
//! Determinism: pure functions over caller-sorted samples, same inputs give
//! same outputs. Errors: [`FightCostError`] at every boundary, never panics.
//!
//! The bars mirror the retired `engine-game::eight_fight` ceilings
//! (`33_300` / `50_000` us); they are copied, not imported, because this
//! crate must not depend on the game crate (acyclic layers). Nothing here
//! enters any sim hash.

/// Frame p50 ceiling in microseconds (30 fps). Mirrors the game gate bar.
pub const FIGHT_P50_BUDGET_US: u64 = 33_300;

/// Frame p99 ceiling in microseconds. Mirrors the game gate bar.
pub const FIGHT_P99_BUDGET_US: u64 = 50_000;

/// Fighters in the FC3 windowed arena run.
pub const FIGHT_FIGHTERS: u8 = 8;

/// Gate failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FightCostError {
    /// Fewer than two samples; the gate needs a real run.
    TooFewSamples(usize),
    /// Percentile `0..=100` out of range, culprit carried.
    BadPercentile(u32),
    /// Measured p50 above [`FIGHT_P50_BUDGET_US`]; carries measured plus bar.
    P50Blown { measured_us: u64, bar_us: u64 },
    /// Measured p99 above [`FIGHT_P99_BUDGET_US`]; carries measured plus bar.
    P99Blown { measured_us: u64, bar_us: u64 },
}

impl core::fmt::Display for FightCostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::TooFewSamples(n) => write!(f, "fight-cost gate needs samples, got {n}"),
            Self::BadPercentile(p) => write!(f, "bad fight-cost percentile ({p})"),
            Self::P50Blown {
                measured_us,
                bar_us,
            } => write!(
                f,
                "fight-cost p50 blew bar ({measured_us}us over {bar_us}us)"
            ),
            Self::P99Blown {
                measured_us,
                bar_us,
            } => write!(
                f,
                "fight-cost p99 blew bar ({measured_us}us over {bar_us}us)"
            ),
        }
    }
}

impl std::error::Error for FightCostError {}

/// One frame's cost split, all in microseconds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FightFrame {
    /// Sim step cost.
    pub sim_us: u64,
    /// Present cost.
    pub present_us: u64,
    /// Fighter draw/pose cost.
    pub fighters_us: u64,
}

impl FightFrame {
    /// Whole-frame cost: the three splits summed, saturating.
    #[must_use]
    pub fn total_us(self) -> u64 {
        self.sim_us
            .saturating_add(self.present_us)
            .saturating_add(self.fighters_us)
    }
}

/// Percentile of ascending `sorted` at `pct` (`0..=100`): rank
/// `len * pct / 100` clamped to the last sample. Empty gives `None`.
/// Integer math only. `sorted` must be ascending.
/// Differs deliberately from [`crate::latency::nearest_rank_sorted`] (float
/// `round(p / 100 * (n - 1))` over ms) and from the ceil-ranked JS gate
/// (`tools/proof.mjs` `ceil(p * n) - 1`): this gate needs whole microseconds
/// with no float, plus `None` on empty or `pct > 100` so a bad rank fails
/// loudly instead of printing zeros. Caller: `p50_sorted`, `p99_sorted` and
/// `check_gate` only.
#[must_use]
pub fn percentile_sorted(sorted: &[u64], pct: u32) -> Option<u64> {
    if sorted.is_empty() || pct > 100 {
        return None;
    }
    let len = sorted.len();
    let mut rank = len * usize::try_from(pct).unwrap_or(100) / 100;
    if rank >= len {
        rank = len - 1;
    }
    sorted.get(rank).copied()
}

/// Median frame cost of ascending `sorted`. `None` when empty.
#[must_use]
pub fn p50_sorted(sorted: &[u64]) -> Option<u64> {
    percentile_sorted(sorted, 50)
}

/// Near-worst frame cost of ascending `sorted`. `None` when empty.
#[must_use]
pub fn p99_sorted(sorted: &[u64]) -> Option<u64> {
    percentile_sorted(sorted, 99)
}

/// One fighter's even share of a frame total. `fighters == 0` gives `0`.
#[must_use]
pub fn per_fighter_us(total_us: u64, fighters: u8) -> u64 {
    if fighters == 0 {
        return 0;
    }
    total_us / u64::from(fighters)
}

/// Check one quiet run's sorted frame totals against the gate bars.
/// Returns the `(p50, p99)` that passed. At the bar passes.
pub fn check_gate(sorted_us: &[u64]) -> Result<(u64, u64), FightCostError> {
    if sorted_us.len() < 2 {
        return Err(FightCostError::TooFewSamples(sorted_us.len()));
    }
    let p50 = p50_sorted(sorted_us).ok_or(FightCostError::TooFewSamples(sorted_us.len()))?;
    let p99 = p99_sorted(sorted_us).ok_or(FightCostError::TooFewSamples(sorted_us.len()))?;
    if p50 > FIGHT_P50_BUDGET_US {
        return Err(FightCostError::P50Blown {
            measured_us: p50,
            bar_us: FIGHT_P50_BUDGET_US,
        });
    }
    if p99 > FIGHT_P99_BUDGET_US {
        return Err(FightCostError::P99Blown {
            measured_us: p99,
            bar_us: FIGHT_P99_BUDGET_US,
        });
    }
    Ok((p50, p99))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ladder() -> Vec<u64> {
        // Ascending fixture: 10 frames, 1 ms apart.
        vec![
            28_000, 29_000, 30_000, 31_000, 32_000, 33_000, 34_000, 35_000, 36_000, 49_000,
        ]
    }

    #[test]
    fn percentiles_rank_a_known_ladder() {
        let run = ladder();
        assert_eq!(percentile_sorted(&run, 50), Some(33_000));
        assert_eq!(percentile_sorted(&run, 99), Some(49_000));
        assert_eq!(percentile_sorted(&run, 0), Some(28_000));
        assert_eq!(percentile_sorted(&run, 100), Some(49_000));
        assert_eq!(p50_sorted(&run), Some(33_000));
        assert_eq!(p99_sorted(&run), Some(49_000));
        assert_eq!(percentile_sorted(&[], 50), None);
        assert_eq!(percentile_sorted(&run, 101), None);
        assert_eq!(percentile_sorted(&[7], 50), Some(7));
    }

    #[test]
    fn gate_passes_headroom_and_trips_over() {
        let (p50, p99) = check_gate(&ladder()).expect("ladder fits the bars");
        assert_eq!((p50, p99), (33_000, 49_000));
        assert_eq!(check_gate(&[]), Err(FightCostError::TooFewSamples(0)));
        assert_eq!(check_gate(&[1]), Err(FightCostError::TooFewSamples(1)));
        let slow_p50 = vec![FIGHT_P50_BUDGET_US + 1; 4];
        assert_eq!(
            check_gate(&slow_p50),
            Err(FightCostError::P50Blown {
                measured_us: FIGHT_P50_BUDGET_US + 1,
                bar_us: FIGHT_P50_BUDGET_US,
            })
        );
        let slow_p99 = vec![1, 1, 1, FIGHT_P99_BUDGET_US + 1];
        assert_eq!(
            check_gate(&slow_p99),
            Err(FightCostError::P99Blown {
                measured_us: FIGHT_P99_BUDGET_US + 1,
                bar_us: FIGHT_P99_BUDGET_US,
            })
        );
    }

    #[test]
    fn frame_split_totals_and_shares() {
        let frame = FightFrame {
            sim_us: 2_000,
            present_us: 8_000,
            fighters_us: 20_000,
        };
        assert_eq!(frame.total_us(), 30_000);
        assert_eq!(per_fighter_us(frame.total_us(), FIGHT_FIGHTERS), 3_750);
        assert_eq!(per_fighter_us(100, 0), 0);
        assert_eq!(FightFrame::default().total_us(), 0);
        // Same inputs twice give the same gate answer.
        assert_eq!(check_gate(&ladder()), check_gate(&ladder()));
    }
}
