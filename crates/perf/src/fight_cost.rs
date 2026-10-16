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
    /// One `arena frame-split` stdout line did not match the expected shape.
    BadSplitLine,
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
            Self::BadSplitLine => write!(f, "bad arena frame-split line"),
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

/// Parse one trimmed `arena frame-split` stdout line into frame costs.
///
/// Expected shape is `arena frame-split sim=<f>ms present=<f>ms
/// fighters=<f>ms` with one millisecond value per split. The live arena
/// receipt prints `arena frame-split:` with the same three tokens in order
/// plus trailing receipt tokens, which are ignored. Each value becomes
/// whole microseconds with `(ms * 1000.0).round()`. Any other shape, plus
/// NaN, negative or over-range values, gives
/// [`FightCostError::BadSplitLine`]. Pure and deterministic, never panics.
pub fn parse_frame_split(line: &str) -> Result<FightFrame, FightCostError> {
    let mut parts = line.trim().split_whitespace();
    let arena = parts.next().ok_or(FightCostError::BadSplitLine)?;
    let kind = parts.next().ok_or(FightCostError::BadSplitLine)?;
    let sim_part = parts.next().ok_or(FightCostError::BadSplitLine)?;
    let present_part = parts.next().ok_or(FightCostError::BadSplitLine)?;
    let fighters_part = parts.next().ok_or(FightCostError::BadSplitLine)?;
    if arena != "arena" || (kind != "frame-split" && kind != "frame-split:") {
        return Err(FightCostError::BadSplitLine);
    }
    Ok(FightFrame {
        sim_us: split_ms_to_us(sim_part, "sim=")?,
        present_us: split_ms_to_us(present_part, "present=")?,
        fighters_us: split_ms_to_us(fighters_part, "fighters=")?,
    })
}

/// Read one `name=<float>ms` token as whole microseconds.
///
/// The float parses as `f64`, then `(ms * 1000.0).round()` becomes the
/// integer. Over-range milliseconds become an over-range microsecond string
/// here, so the integer parse below rejects them without a float cast.
fn split_ms_to_us(token: &str, prefix: &str) -> Result<u64, FightCostError> {
    let inner = token
        .strip_prefix(prefix)
        .ok_or(FightCostError::BadSplitLine)?;
    let digits = inner
        .strip_suffix("ms")
        .ok_or(FightCostError::BadSplitLine)?;
    if digits.is_empty() {
        return Err(FightCostError::BadSplitLine);
    }
    let ms: f64 = digits.parse().map_err(|_| FightCostError::BadSplitLine)?;
    if !ms.is_finite() || ms < 0.0 {
        return Err(FightCostError::BadSplitLine);
    }
    let rounded = (ms * 1000.0).round();
    if !rounded.is_finite() || rounded < 0.0 {
        return Err(FightCostError::BadSplitLine);
    }
    rounded
        .to_string()
        .parse()
        .map_err(|_| FightCostError::BadSplitLine)
}

/// Each split's share of the frame total as integer permille.
///
/// Returns `(sim, present, fighters)` in `0..=1000` with sum `1000`. The
/// fighters share is `fighters_us * 1000 / total`; the present share absorbs
/// the integer remainder so the three sum to `1000`. Total `0` gives
/// `(0, 0, 0)`. Integer math only, deterministic.
#[must_use]
pub fn split_shares(frame: FightFrame) -> (u32, u32, u32) {
    let total_us = frame.total_us();
    if total_us == 0 {
        return (0, 0, 0);
    }
    let total = u128::from(total_us);
    let sim_part = u128::from(frame.sim_us) * 1000 / total;
    let fighters_part = u128::from(frame.fighters_us) * 1000 / total;
    let sim_share = match u32::try_from(sim_part) {
        Ok(v) => v,
        Err(_) => 0,
    };
    let fighters_share = match u32::try_from(fighters_part) {
        Ok(v) => v,
        Err(_) => 0,
    };
    let present_share = 1000_u32
        .saturating_sub(sim_share)
        .saturating_sub(fighters_share);
    (sim_share, present_share, fighters_share)
}

/// The costliest split with its permille share and owning file.
///
/// Owner is `crates/game/src/lib.rs` for sim,
/// `crates/render/src/present.rs` for present and
/// `crates/game/src/fighter_look.rs` for fighters. Ties break sim, then
/// present, then fighters. Integer math only, deterministic.
#[must_use]
pub fn top_split(frame: FightFrame) -> (&'static str, u32, &'static str) {
    let (sim_share, present_share, fighters_share) = split_shares(frame);
    if sim_share >= present_share && sim_share >= fighters_share {
        ("sim", sim_share, "crates/game/src/lib.rs")
    } else if present_share >= fighters_share {
        ("present", present_share, "crates/render/src/present.rs")
    } else {
        (
            "fighters",
            fighters_share,
            "crates/game/src/fighter_look.rs",
        )
    }
}

/// True when three frame totals agree within ten percent.
///
/// Compares `(max - min) * 10 <= min` in 128-bit math so the scale never
/// overflows. All zeros pass; any zero minimum with a nonzero span fails.
/// Integer math only, deterministic.
#[must_use]
pub fn spread_ok(first_us: u64, second_us: u64, third_us: u64) -> bool {
    let min_us = first_us.min(second_us).min(third_us);
    let max_us = first_us.max(second_us).max(third_us);
    let span = max_us - min_us;
    u128::from(span) * 10 <= u128::from(min_us)
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

    #[test]
    fn frame_split_parse_names_top_cost_and_spread() {
        let frame = parse_frame_split("arena frame-split sim=2.0ms present=8.0ms fighters=20.0ms")
            .expect("quiet line parses");
        assert_eq!(
            frame,
            FightFrame {
                sim_us: 2_000,
                present_us: 8_000,
                fighters_us: 20_000,
            }
        );
        let shares = split_shares(frame);
        assert!(shares.2 > shares.0 && shares.2 > shares.1);
        assert_eq!(
            top_split(frame),
            ("fighters", 666, "crates/game/src/fighter_look.rs")
        );
        assert!(spread_ok(30_000, 31_000, 32_000));
        assert!(!spread_ok(30_000, 30_000, 40_000));
        assert!(parse_frame_split("").is_err());
        assert!(parse_frame_split("arena frame-split sim=xms").is_err());
        assert!(
            parse_frame_split("arena frame-split sim=-1.0ms present=8.0ms fighters=20.0ms")
                .is_err()
        );
    }

    #[test]
    fn frame_split_parses_the_live_emit_line() {
        let line = "arena frame-split: sim=2.0ms present=8.0ms fighters=20.0ms per frame (mean over 300 frames) cpu_ms p50=1ms p99=2ms interval_ms p50=3ms p99=4ms gpu_ms=unsupported acquire=0ms present_mode=";
        let frame = parse_frame_split(line).expect("live emit line parses");
        assert_eq!(
            frame,
            FightFrame {
                sim_us: 2_000,
                present_us: 8_000,
                fighters_us: 20_000,
            }
        );
        assert!(parse_frame_split("").is_err());
        assert!(parse_frame_split("arena frame-split sim=xms").is_err());
        assert!(
            parse_frame_split("arena frame-split sim=-1.0ms present=8.0ms fighters=20.0ms")
                .is_err()
        );
        assert!(
            parse_frame_split("arena frame-split: sim=NaNms present=8.0ms fighters=20.0ms")
                .is_err()
        );
        assert!(parse_frame_split(
            "arena frame-split: sim=99999999999999999999999999.0ms present=8.0ms fighters=20.0ms"
        )
        .is_err());
    }
}
