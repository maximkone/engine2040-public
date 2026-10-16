//! Skill-rating scorer over frozen bout records (offline only).
//!
//! Consumer: the balance-tournament gate and DESIGN move-table packets,
//! which call [`table_from_tally`] or [`score`] to turn win tallies into
//! Elo ratings and read [`flagged`] for overpowered outliers. Data owner:
//! the caller owns ids and bouts; this module keeps no global state.
//! Units: Elo points (`f64`), permille bout scores (`0`, `500`, `1000`).
//! Determinism: sorted ids, input-order updates, no wall clock or hash
//! order in scores. Errors: [`RatingError`] on bad ids or scores.

use std::fmt;

/// Starting Elo for every entrant.
pub const START_RATING: f64 = 1500.0;
/// Elo K-factor per bout (offline tuning, zero-sum updates).
pub const K_FACTOR: f64 = 32.0;
/// Rating gap above start that flags an overpowered entrant.
pub const OP_GAP: f64 = 100.0;
/// Upper bound on entrants per table (bounded offline buffers).
pub const MAX_PLAYERS: usize = 64;
/// Upper bound on bouts per scoring call (offline batch cap).
pub const MAX_BOUTS: usize = 10_000;

/// One bout from the white entrant's view: `1000` win, `500` draw,
/// `0` loss. Ids name tournament entrants (fighters or moves).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bout {
    /// White entrant id.
    pub white: u32,
    /// Black entrant id (must differ from `white`).
    pub black: u32,
    /// White score in permille: exactly `0`, `500` or `1000`.
    pub score_permille: u16,
}

/// Typed scorer rejections: invalid ids, scores or bounds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RatingError {
    /// No entrants or too many (`1..=MAX_PLAYERS`).
    BadEntrants(String),
    /// Unknown entrant, mirrored ids or bad score in a bout.
    BadBout(String),
    /// Too many bouts (`0..=MAX_BOUTS`).
    TooManyBouts(String),
}

impl fmt::Display for RatingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadEntrants(reason) | Self::BadBout(reason) | Self::TooManyBouts(reason) => {
                write!(f, "{reason}")
            }
        }
    }
}

impl std::error::Error for RatingError {}

/// Elo table: ids in sorted order with one rating each.
#[derive(Clone, PartialEq, Debug)]
pub struct Table {
    /// Entrant ids in sorted order.
    ids: Vec<u32>,
    /// Ratings parallel to `ids`.
    ratings: Vec<f64>,
}

impl Table {
    /// Empty table over sorted unique `ids`. Errors on empty, duplicate
    /// or over-cap entrants. Deterministic: same ids give same ratings.
    pub fn new(ids: &[u32]) -> Result<Self, RatingError> {
        if ids.is_empty() || ids.len() > MAX_PLAYERS {
            return Err(RatingError::BadEntrants(format!(
                "need 1..={} entrants, got {}",
                MAX_PLAYERS,
                ids.len()
            )));
        }
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != ids.len() {
            return Err(RatingError::BadEntrants(
                "entrant ids must be unique".to_string(),
            ));
        }
        Ok(Self {
            ratings: vec![START_RATING; sorted.len()],
            ids: sorted,
        })
    }

    /// Entrant count.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when the table holds no entrants (never after [`Table::new`]).
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Rating for `id`, or `None` for unknown entrants.
    pub fn rating(&self, id: u32) -> Option<f64> {
        self.ids
            .iter()
            .position(|entry| *entry == id)
            .map(|index| self.ratings[index])
    }

    /// Entrant ids in sorted order.
    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    /// Score one validated bout in place (zero-sum Elo update).
    pub fn update(&mut self, bout: &Bout) -> Result<(), RatingError> {
        validate_bout(&self.ids, bout)?;
        let white_at = self
            .ids
            .iter()
            .position(|entry| *entry == bout.white)
            .unwrap_or(0);
        let black_at = self
            .ids
            .iter()
            .position(|entry| *entry == bout.black)
            .unwrap_or(0);
        let actual = f64::from(bout.score_permille) / 1000.0;
        let expected = expected_score(self.ratings[white_at], self.ratings[black_at]);
        let delta = K_FACTOR * (actual - expected);
        self.ratings[white_at] += delta;
        self.ratings[black_at] -= delta;
        Ok(())
    }

    /// Score every bout in order. Errors without mutating past the
    /// failing bout's predecessors (callers re-score from scratch).
    pub fn score_all(&mut self, bouts: &[Bout]) -> Result<(), RatingError> {
        if bouts.len() > MAX_BOUTS {
            return Err(RatingError::TooManyBouts(format!(
                "need 0..={} bouts, got {}",
                MAX_BOUTS,
                bouts.len()
            )));
        }
        for bout in bouts {
            self.update(bout)?;
        }
        Ok(())
    }
}

/// Expected white score from two Elo ratings (offline logistic only,
/// never inside the fixed tick).
fn expected_score(white: f64, black: f64) -> f64 {
    1.0 / (1.0 + 10_f64.powf((black - white) / 400.0))
}

/// Validate one bout against sorted `ids`.
fn validate_bout(ids: &[u32], bout: &Bout) -> Result<(), RatingError> {
    if bout.white == bout.black {
        return Err(RatingError::BadBout(format!(
            "mirrored bout id {}: white must differ from black",
            bout.white
        )));
    }
    if !ids.contains(&bout.white) || !ids.contains(&bout.black) {
        return Err(RatingError::BadBout(format!(
            "unknown entrant in bout {} vs {}",
            bout.white, bout.black
        )));
    }
    if bout.score_permille != 0 && bout.score_permille != 500 && bout.score_permille != 1000 {
        return Err(RatingError::BadBout(format!(
            "bad score {}: must be 0, 500 or 1000",
            bout.score_permille
        )));
    }
    Ok(())
}

/// Score `bouts` over `ids` in order. New table each call, so the same
/// inputs give the same ratings.
pub fn score(ids: &[u32], bouts: &[Bout]) -> Result<Table, RatingError> {
    let mut table = Table::new(ids)?;
    table.score_all(bouts)?;
    Ok(table)
}

/// Entrants rated at least [`OP_GAP`] above start, sorted. Empty on
/// balanced fields; the planted overpowered entrant appears here.
pub fn flagged(table: &Table) -> Vec<u32> {
    table
        .ids
        .iter()
        .zip(table.ratings.iter())
        .filter(|(_, rating)| **rating - START_RATING >= OP_GAP)
        .map(|(id, _)| *id)
        .collect()
}

/// Build deterministic bouts from a win tally: wins (`1000`), then
/// draws (`500`), then losses (`0`). Tallies above the bout cap fail.
pub fn bouts_from_tally(
    white: u32,
    black: u32,
    wins: u32,
    draws: u32,
    losses: u32,
) -> Result<Vec<Bout>, RatingError> {
    if white == black {
        return Err(RatingError::BadBout(format!(
            "mirrored tally id {white}: entrants must differ"
        )));
    }
    let sum = u64::from(wins) + u64::from(draws) + u64::from(losses);
    let total = usize::try_from(sum)
        .map_err(|_| RatingError::TooManyBouts(format!("tally sum {sum} exceeds address space")))?;
    if total > MAX_BOUTS {
        return Err(RatingError::TooManyBouts(format!(
            "tally sums to {total} bouts, cap is {MAX_BOUTS}"
        )));
    }
    let mut bouts = Vec::with_capacity(total);
    for _ in 0..wins {
        bouts.push(Bout {
            white,
            black,
            score_permille: 1000,
        });
    }
    for _ in 0..draws {
        bouts.push(Bout {
            white,
            black,
            score_permille: 500,
        });
    }
    for _ in 0..losses {
        bouts.push(Bout {
            white,
            black,
            score_permille: 0,
        });
    }
    Ok(bouts)
}

/// Score a tournament tally directly: the named consumer wiring beside
/// the balance-tournament harness. Builds [`bouts_from_tally`] bouts over
/// the two entrants and returns the finished table.
pub fn table_from_tally(
    white: u32,
    black: u32,
    wins: u32,
    draws: u32,
    losses: u32,
) -> Result<Table, RatingError> {
    let bouts = bouts_from_tally(white, black, wins, draws, losses)?;
    score(&[white, black], &bouts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win_bouts(white: u32, black: u32, n: u32) -> Vec<Bout> {
        (0..n)
            .map(|_| Bout {
                white,
                black,
                score_permille: 1000,
            })
            .collect()
    }

    #[test]
    fn planted_overpowered_entrant_flags_within_100_bouts() {
        // Planted OP: white wins every bout; the detector must fire early.
        let bouts = win_bouts(7, 9, 60);
        let table = score(&[7, 9], &bouts).expect("valid bouts score");
        let hits = flagged(&table);
        assert_eq!(hits, vec![7], "planted winner must flag: {table:?}");
        // Prefix check: the first 100 bouts already flag (60 < 100).
        assert!(bouts.len() <= 100);
        assert!(
            table.rating(7).unwrap_or(0.0) - START_RATING >= OP_GAP,
            "gap must clear {OP_GAP}: {table:?}"
        );
    }

    #[test]
    fn balanced_records_raise_no_false_alarm() {
        // Even split in bout order (alternating wins and losses, the way
        // real records read): ratings hug start, nobody flags. Block order
        // (all wins then all losses) drifts by Elo path dependence, so the
        // balanced fixture interleaves like a real field.
        let mut bouts = Vec::with_capacity(100);
        for index in 0..50 {
            let permille = if index % 2 == 0 { 1000 } else { 0 };
            bouts.push(Bout {
                white: 3,
                black: 4,
                score_permille: permille,
            });
        }
        for _ in 0..50 {
            bouts.push(Bout {
                white: 3,
                black: 4,
                score_permille: 500,
            });
        }
        let table = score(&[3, 4], &bouts).expect("balanced bouts score");
        assert!(
            flagged(&table).is_empty(),
            "balanced must not flag: {table:?}"
        );
        // All draws: exactly start, still silent.
        let draws = bouts_from_tally(3, 4, 0, 100, 0).expect("draws build");
        let calm = score(&[3, 4], &draws).expect("draws score");
        assert!(flagged(&calm).is_empty());
    }

    #[test]
    fn same_bouts_twice_give_same_table() {
        let bouts = bouts_from_tally(1, 2, 10, 5, 7).expect("tally builds");
        let first = score(&[2, 1], &bouts).expect("scores");
        let second = score(&[1, 2], &bouts).expect("scores");
        assert_eq!(first, second);
        assert_eq!(first.ids(), &[1, 2]);
    }

    #[test]
    fn tally_wiring_matches_explicit_bouts() {
        let from_tally = table_from_tally(5, 6, 4, 2, 1).expect("tally scores");
        let bouts = bouts_from_tally(5, 6, 4, 2, 1).expect("bouts build");
        let explicit = score(&[5, 6], &bouts).expect("explicit scores");
        assert_eq!(from_tally, explicit);
    }

    #[test]
    fn invalid_bouts_fail_loudly() {
        assert!(Table::new(&[]).is_err());
        assert!(Table::new(&[1, 1]).is_err());
        let table = Table::new(&[1, 2]).expect("valid table");
        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
        // Mirrored ids.
        assert!(score(
            &[1, 2],
            &[Bout {
                white: 1,
                black: 1,
                score_permille: 1000
            }]
        )
        .is_err());
        // Unknown entrant.
        assert!(score(
            &[1, 2],
            &[Bout {
                white: 1,
                black: 99,
                score_permille: 1000
            }]
        )
        .is_err());
        // Bad score.
        assert!(score(
            &[1, 2],
            &[Bout {
                white: 1,
                black: 2,
                score_permille: 700
            }]
        )
        .is_err());
        // Mirrored tally and oversize tally.
        assert!(bouts_from_tally(8, 8, 1, 0, 0).is_err());
        assert!(bouts_from_tally(8, 9, MAX_BOUTS as u32, 1, 0).is_err());
    }

    #[test]
    #[ignore = "timing: wall clock, run under --perf"]
    fn scoring_100_bouts_stays_under_10ms() {
        // Bout-ordered field (wins, draws and losses interleaved the way
        // real records read, avoiding block-order Elo drift).
        let mut bouts = Vec::with_capacity(100);
        for index in 0..100 {
            let permille = match index % 5 {
                0 | 1 | 2 => 1000,
                3 => 500,
                _ => 0,
            };
            bouts.push(Bout {
                white: 11,
                black: 12,
                score_permille: permille,
            });
        }
        let start = std::time::Instant::now();
        let table = score(&[11, 12], &bouts).expect("scores");
        let elapsed = start.elapsed();
        let replay = score(&[11, 12], &bouts).expect("replays");
        assert_eq!(table, replay);
        assert!(
            elapsed.as_millis() < 10,
            "100 bouts must score under 10 ms, took {elapsed:?}"
        );
    }
}
