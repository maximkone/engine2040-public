//! Difficulty-curve sweep: one knob, a range of values, one tournament per
//! value.
//!
//! Consumer: the door's `tune.sweep` verb and the balance log. The caller
//! owns the knob and the tournament shape; this module runs the curve and
//! renders it. Data owner: the caller owns the config, so a knob the
//! tournament cannot see still gets a row. Units: fixed ticks, permille win
//! rates, raw button bytes for kill shares. Determinism: seeded and
//! replay-exact (the same points twice give the same report and the same
//! CSV). Errors: [`Failure`] rows; a bad point list fails before any
//! simulation.

use crate::winrates::HumanError;
use crate::Failure;

use super::{
    fail, run_difficulty, run_with_error, DifficultyReport, KillShare, TournamentConfig,
    MAX_MATCH_TICKS,
};

/// Difficulty-curve knob: the ONE [`BalanceConfig`](crate::BalanceConfig)
/// field the sweep varies (`match_ticks`, fight length in fixed ticks).
/// Both configs carry it; longer fights let skill decide while short fights
/// compress to draws, so the curve proves the win-rate bands hold across
/// fight lengths instead of at one shape only.
pub const SWEEP_KNOB: &str = "match_ticks";
/// Standard sweep values: short, standard and long fights.
pub const SWEEP_VALUES: [u32; 3] = [300, 600, 1_200];
/// Upper bound on knob values per sweep (the knob cap): one short run each.
pub const SWEEP_MAX_VALUES: usize = 8;
/// Player label for the perfect tape in the curve CSV.
pub const SWEEP_PERFECT: &str = "perfect";
/// Player label for the human-error tape in the curve CSV.
pub const SWEEP_HUMAN: &str = "human";

/// Difficulty-curve sweep configuration: one 200-match tournament per knob
/// value, perfect player first, then the seeded human-like error when
/// enabled (default off). Owned by the enemy designer; the curve CSV is its
/// report to the balance log.
#[derive(Clone, PartialEq, Debug)]
pub struct SweepConfig {
    /// Base tournament; `matches` stays at the proof size, only the knob
    /// (`match_ticks`) varies per value.
    pub base: TournamentConfig,
    /// Knob values (`1..=MAX_MATCH_TICKS`, `1..=SWEEP_MAX_VALUES` entries).
    pub values: Vec<u32>,
    /// Seeded human-like error on slot zero; off is the perfect player.
    pub error: HumanError,
}

impl SweepConfig {
    /// Standard curve: the P3 proof shape at short, standard and long fight
    /// lengths with the perfect player.
    pub fn standard() -> Self {
        Self {
            base: TournamentConfig::standard(),
            values: SWEEP_VALUES.to_vec(),
            error: HumanError::off(),
        }
    }
}

/// One finished curve point: a difficulty's outcome at one knob value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SweepRow {
    /// Knob value (fight length in ticks).
    pub value: u32,
    /// Difficulty name, in config order.
    pub difficulty: String,
    /// True for the human-error replay, false for the perfect player.
    pub human: bool,
    /// Matches fighter 0 won outright.
    pub wins: u32,
    /// Matches tied for top damage.
    pub draws: u32,
    /// Matches another fighter won outright.
    pub losses: u32,
    /// `(2 * wins + draws) * 1000 / (2 * trials)`.
    pub rate_permille: u16,
    /// Kill attribution per observed killing byte, sorted by byte.
    pub kill_shares: Vec<KillShare>,
    /// Matches with a fighter slow 180+ ticks, summed over both slots.
    pub stuck_count: u32,
    /// Matches played (== the tournament size).
    pub trials: u32,
}

/// One skipped curve point: a variant nobody finished (zero kills), so it
/// carries no win-rate signal and the curve skips it instead of printing a
/// bloodless row as difficulty data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SweepSkip {
    /// Knob value (fight length in ticks).
    pub value: u32,
    /// Difficulty name, in config order.
    pub difficulty: String,
    /// Why the variant was skipped (always names the empty kill count).
    pub reason: String,
}

/// Whole difficulty curve: finished rows plus skips, in sweep order.
/// Deterministic: the same config twice gives the same report.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SweepReport {
    /// The knob the sweep varied; its name rides in the CSV so a reader
    /// knows which number the values belong to.
    pub knob: String,
    /// Finished curve points, in `(value, difficulty, player)` order.
    pub rows: Vec<SweepRow>,
    /// Skipped variants, in `(value, difficulty)` order.
    pub skipped: Vec<SweepSkip>,
}

/// One swept point: the knob value and the tournament it runs. The caller
/// owns the knob, so a knob the tournament cannot see still gets a row
/// (with its own value) instead of a silent skip.
#[derive(Clone, PartialEq, Debug)]
pub struct SweepPoint {
    /// The knob value, in the knob's own unit.
    pub value: u32,
    /// The tournament that runs at this value.
    pub config: TournamentConfig,
}

/// One curve row from a finished difficulty report. Pure.
fn sweep_row(value: u32, row: &DifficultyReport, human: bool) -> SweepRow {
    SweepRow {
        value,
        difficulty: row.name.clone(),
        human,
        wins: row.wins,
        draws: row.draws,
        losses: row.losses,
        rate_permille: row.rate_permille,
        kill_shares: row.kill_shares.clone(),
        stuck_count: row.stuck_trips.iter().map(|(_, n)| *n).sum(),
        trials: row.matches,
    }
}

/// Run the difficulty curve over prepared points: per point the perfect-player
/// tournament first; a difficulty nobody finished (`kills_total == 0`) is
/// skipped; finished difficulties also replay under the seeded human error
/// when enabled. Discovery, spotlight and exchange probes stay perfect brains
/// — only duel rows take the error tape. The knob name rides in the report,
/// so the CSV names the knob the caller swept. Deterministic: the same points
/// twice give the same report. Fails without simulating on an empty or
/// over-long point list; per-point tournaments validate like
/// [`super::run`].
pub fn sweep_points(
    knob: &str,
    points: &[SweepPoint],
    error: HumanError,
) -> Result<SweepReport, Failure> {
    if points.is_empty() || points.len() > SWEEP_MAX_VALUES {
        return Err(fail(format!(
            "invalid sweep values {}: must be 1..={SWEEP_MAX_VALUES}",
            points.len()
        )));
    }
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for point in points {
        let perfect = run_with_error(&point.config, HumanError::off())?;
        for (index, row) in perfect.per_difficulty.iter().enumerate() {
            if row.kills_total == 0 {
                skipped.push(SweepSkip {
                    value: point.value,
                    difficulty: row.name.clone(),
                    reason: "no-finisher (kills_total=0)".to_string(),
                });
                continue;
            }
            rows.push(sweep_row(point.value, row, false));
            if error.enabled {
                let difficulty = &point.config.difficulties[index];
                let erred = run_difficulty(&point.config, index, difficulty, error)?;
                rows.push(sweep_row(point.value, &erred, true));
            }
        }
    }
    Ok(SweepReport {
        knob: knob.to_string(),
        rows,
        skipped,
    })
}

/// Run the difficulty curve: per knob value the perfect-player tournament
/// first; a difficulty nobody finished (`kills_total == 0`) is skipped;
/// finished difficulties also replay under the seeded human error when
/// enabled. Discovery, spotlight and exchange probes stay perfect brains —
/// only duel rows take the error tape. Fails without simulating on a bad
/// knob list; per-value tournaments validate like [`super::run`].
pub fn sweep_difficulty_curve(sweep: &SweepConfig) -> Result<SweepReport, Failure> {
    if sweep.values.is_empty() || sweep.values.len() > SWEEP_MAX_VALUES {
        return Err(fail(format!(
            "invalid sweep values {}: must be 1..={SWEEP_MAX_VALUES}",
            sweep.values.len()
        )));
    }
    for value in &sweep.values {
        if *value == 0 || *value > MAX_MATCH_TICKS {
            return Err(fail(format!(
                "invalid sweep value {value}: must be 1..={MAX_MATCH_TICKS}"
            )));
        }
    }
    let points: Vec<SweepPoint> = sweep
        .values
        .iter()
        .map(|value| SweepPoint {
            value: *value,
            config: TournamentConfig {
                match_ticks: *value,
                ..sweep.base.clone()
            },
        })
        .collect();
    sweep_points(SWEEP_KNOB, &points, sweep.error)
}

/// Render the curve as CSV: `knob,value,difficulty,player,win_rate_permille,
/// kill_shares,stuck_count,trials` data rows plus `# skipped ...` comment
/// lines. Kill shares print `byte:share` pairs joined by `;` (`none` when a
/// finished row drew no blood attribution). Pure and byte-stable: the same
/// report twice gives the same text (no clock inside; the UTC stamp names
/// the file only).
pub fn render_sweep_csv(report: &SweepReport) -> String {
    let mut out = String::from(
        "knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials\n",
    );
    for row in &report.rows {
        let shares = if row.kill_shares.is_empty() {
            "none".to_string()
        } else {
            row.kill_shares
                .iter()
                .map(|share| format!("{}:{}", share.move_byte, share.share_permille))
                .collect::<Vec<_>>()
                .join(";")
        };
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            report.knob,
            row.value,
            row.difficulty,
            if row.human {
                SWEEP_HUMAN
            } else {
                SWEEP_PERFECT
            },
            row.rate_permille,
            shares,
            row.stuck_count,
            row.trials,
        ));
    }
    let knob = &report.knob;
    for skip in &report.skipped {
        out.push_str(&format!(
            "# skipped {knob}={} difficulty={} reason={}\n",
            skip.value, skip.difficulty, skip.reason,
        ));
    }
    out
}
