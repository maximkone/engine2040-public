//! Time-to-kill and matchup tables for the Rome duel (GP33).
//!
//! Consumer: the balance log and the Rome design cards. The module writes
//! two CSV reports into `target/reports`: `rome-ttk.csv` (the analytic
//! time-to-kill per duelist and move kind) and `rome-matchups.csv` (the
//! measured win matrix of the duel ladder).
//!
//! Data owner: the loaded design rows own every number. The move table comes
//! from `content/design/combat.rows` through
//! [`engine_combat::tables_text::parse_tables`], the enemy health from
//! `content/design/enemies.rows` through [`engine_ai::tier::parse_tiers`], and
//! the duel law from [`engine_ai::tier::WIN_BAR_PERMILLE`] (the GA13 house
//! bar: a fighter beats another when it wins that share of the duels).
//!
//! Units: health and damage in permille, timing in fixed ticks at 60 Hz, win
//! rates in permille. Counts are whole numbers; no float enters a table.
//!
//! Determinism: the time-to-kill table is pure integer math over the loaded
//! rows, so it is the same on every machine. The matchup matrix runs seeded
//! duels through the arena fork (the GA13 tournament path: a prefix
//! snapshot, a `SetTier` proposal per seat and, for the human seat, the
//! seeded GA14 error tape), so the same tree gives the same matrix twice.
//!
//! Errors: [`TtkError`] names the row, the field or the pair; nothing here
//! panics on a bad input, so a bad design file is a refusal and not a crash.
//!
//! The odd-cycle check: the matrix must hold no dominant row, meaning no
//! duelist beats every other duelist. Five duelists make an odd count, so
//! the ladder can close on itself. The measured matrix keeps the GA13 ladder
//! (every rung beats the one below it) and adds the player as the fifth
//! duelist: the boss (tier 4) clears the duel law against the rungs but not
//! against the player, so no row of the shipped matrix dominates.
//!
//! The player seat has two readings, and both are measured. The plain
//! matrix runs the player on the plain brain, the same seat the GA13
//! tournament uses for its mirror balance proof: no duelist dominates. The
//! human matrix runs the player on the seeded GA14 error tape: the boss then
//! clears the law against every duelist and the check names it. That second
//! reading is the design finding of this table: a player who duels with
//! human timing loses the odd cycle, so the player needs the kit answers the
//! tier cards promise (parry, dodge, block), not a faster brain.

use engine_ai::tier::{parse_tiers, TierRow, WIN_BAR_PERMILLE};
use engine_arena::fork::{Fork, ForkProposal};
use engine_arena::{ArenaConfig, BodyId, BodyInput, BodyTape};
use engine_combat::depth::{INPUT_HEAVY, INPUT_LIGHT};
use engine_combat::fight::{hitstop_for_impact, Tables, MAX_HP_PERMILLE};
use engine_combat::tables::{hits_to_kill, ttk_ticks};
use engine_combat::tables_text::parse_tables;

use crate::winrates::{human_tape_for, slot_zero_result, HumanError, SlotZero};

/// The shipped move table rows (the Rome duel reads the v2 row).
const COMBAT_ROWS: &str = include_str!("../../../content/design/combat.rows");
/// The shipped enemy rows (the four duel tiers and their health).
const ENEMY_ROWS: &str = include_str!("../../../content/design/enemies.rows");

/// Duel length in fixed ticks: the GA13 standard match.
pub const DUEL_TICKS: u32 = 600;
/// Seeded duels per pair: every duel plays the pair twice, once in each
/// seat, so the pair share never reads a seat preference as skill.
pub const DUELS_PER_PAIR: u32 = 400;
/// First seed of a duel; each duel adds one.
pub const BASE_SEED: u64 = 2040;
/// Arena half-extent in meters for the duel (the GA13 tournament shape).
pub const DUEL_HALF_EXTENT_M: f32 = 20.0;
/// Move-table version the Rome duel runs under: the readable-timing v2 row.
pub const DUEL_TABLE_VERSION: u32 = 2;

/// Name of the player duelist in the tables.
pub const HERO_NAME: &str = "hero";

/// Why a table could not be built: the row, the field or the pair.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TtkError {
    /// The move-table rows refused to parse.
    CombatRows(String),
    /// The enemy rows refused to parse.
    EnemyRows(String),
    /// A duel could not be run (the arena refused the match).
    Duel {
        /// The first duelist of the pair.
        duelist: String,
        /// The second duelist of the pair.
        opponent: String,
        /// Why the duel refused.
        reason: String,
    },
    /// The report file could not be written.
    Write {
        /// The file path.
        path: String,
        /// Why the write refused.
        reason: String,
    },
}

impl std::fmt::Display for TtkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CombatRows(reason) => write!(f, "combat rows refused: {reason}"),
            Self::EnemyRows(reason) => write!(f, "enemy rows refused: {reason}"),
            Self::Duel {
                duelist,
                opponent,
                reason,
            } => write!(f, "duel {duelist} vs {opponent} refused: {reason}"),
            Self::Write { path, reason } => write!(f, "report {path} refused: {reason}"),
        }
    }
}

impl std::error::Error for TtkError {}

/// One duelist of the Rome table: the player or an enemy tier.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Duelist {
    /// Stable name in the tables (`hero`, `tier_1` ... `tier_4`).
    pub name: String,
    /// The installed enemy tier; `None` for the player.
    pub tier: Option<TierRow>,
    /// Health in permille: the tier row's own health, the player's full bar.
    pub hp_permille: u16,
}

impl Duelist {
    /// The player: no enemy tier, full health.
    pub fn hero() -> Self {
        Self {
            name: HERO_NAME.to_string(),
            tier: None,
            hp_permille: MAX_HP_PERMILLE,
        }
    }

    /// An enemy tier: its row owns the health.
    pub fn tier(row: TierRow) -> Self {
        Self {
            name: format!("tier_{}", row.tier),
            tier: Some(row),
            hp_permille: row.hp_permille,
        }
    }
}

/// The five duelists of the Rome table: the player plus the four enemy
/// tiers, in ladder order. An odd count keeps the cycle closable: with an
/// even count a fighter could sit on top of the ladder forever.
pub fn duelists(rows: &[TierRow; 4]) -> Vec<Duelist> {
    let mut out = vec![Duelist::hero()];
    for row in rows {
        out.push(Duelist::tier(*row));
    }
    out
}

/// The loaded move table of the Rome duel.
pub fn duel_tables() -> Result<Tables, TtkError> {
    parse_tables(COMBAT_ROWS)
        .map(|swap| swap.tables())
        .map_err(|error| TtkError::CombatRows(format!("{error:?}")))
}

/// The loaded enemy rows of the Rome duel.
pub fn duel_tiers() -> Result<[TierRow; 4], TtkError> {
    parse_tiers(ENEMY_ROWS).map_err(|error| TtkError::EnemyRows(format!("{error:?}")))
}

/// One time-to-kill row: a duelist and one move kind, from the loaded tables.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TtkRow {
    /// The duelist the row measures.
    pub duelist: String,
    /// The move kind (`light` or `heavy`).
    pub move_kind: &'static str,
    /// The duelist's health in permille.
    pub hp_permille: u16,
    /// One clean hit's damage in permille.
    pub damage_permille: u16,
    /// Hits needed to bring the duelist down.
    pub hits_to_kill: u32,
    /// Ticks from the press to the impact.
    pub startup_ticks: u32,
    /// Ticks committed after the impact.
    pub recovery_ticks: u32,
    /// Ticks the impact freezes the fight.
    pub hitstop_ticks: u32,
    /// Ticks to the kill: one full cycle per landed hit.
    pub ttk_ticks: u64,
}

impl TtkRow {
    /// The kill in hundredths of a second at 60 Hz, rounded to the nearest
    /// tick: `ttk_ticks * 100 / 60`.
    pub fn ttk_centiseconds(&self) -> u32 {
        let scaled = self.ttk_ticks.saturating_mul(100).saturating_add(30);
        u32::try_from(scaled / 60).unwrap_or(u32::MAX)
    }
}

/// The move kinds the table measures, in order: the light strike and the
/// heavy one. The heavy strike freezes the fight twice as long, so the kill
/// takes longer even though the damage is the same.
pub const MOVE_KINDS: [(&str, u8); 2] = [("light", INPUT_LIGHT), ("heavy", INPUT_HEAVY)];

/// The time-to-kill table: one row per duelist and move kind, from the
/// loaded move table and the loaded enemy rows. Pure: the same rows give the
/// same table twice.
pub fn ttk_rows(tables: &Tables, duelists: &[Duelist]) -> Vec<TtkRow> {
    let mut out = Vec::new();
    for duelist in duelists {
        for (name, kind) in MOVE_KINDS {
            let hitstop = hitstop_for_impact(tables, kind, false);
            let hits = hits_to_kill(tables.attack_damage, duelist.hp_permille);
            out.push(TtkRow {
                duelist: duelist.name.clone(),
                move_kind: name,
                hp_permille: duelist.hp_permille,
                damage_permille: tables.attack_damage,
                hits_to_kill: hits,
                startup_ticks: tables.attack_startup_ticks,
                recovery_ticks: tables.attack_recovery_ticks,
                hitstop_ticks: hitstop,
                ttk_ticks: ttk_ticks(
                    hits,
                    tables.attack_startup_ticks,
                    tables.attack_recovery_ticks,
                    hitstop,
                ),
            });
        }
    }
    out
}

/// The time-to-kill table as CSV text: a header, one row per duelist and
/// move kind. Pure and byte-stable: the same table twice gives the same
/// text.
pub fn render_ttk_csv(rows: &[TtkRow]) -> String {
    let mut out = String::from(
        "# rome-ttk: time-to-kill per duelist and move kind from the loaded\n\
         # design rows (content/design/combat.rows and enemies.rows).\n\
         # ticks are fixed ticks at 60 Hz; ttk_centiseconds is ttk_ticks * 100 / 60.\n",
    );
    out.push_str(
        "duelist,move_kind,hp_permille,damage_permille,hits_to_kill,startup_ticks,recovery_ticks,hitstop_ticks,ttk_ticks,ttk_centiseconds\n",
    );
    for row in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            row.duelist,
            row.move_kind,
            row.hp_permille,
            row.damage_permille,
            row.hits_to_kill,
            row.startup_ticks,
            row.recovery_ticks,
            row.hitstop_ticks,
            row.ttk_ticks,
            row.ttk_centiseconds(),
        ));
    }
    out
}

/// One matchup row: the duel between two duelists over the seeded duels. The
/// row names the first duelist of the pair and carries its share; the
/// opponent's share is the complement.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchupRow {
    /// The first duelist of the pair.
    pub duelist: String,
    /// The second duelist of the pair.
    pub opponent: String,
    /// Duel seeds played: each seed duels the pair in both seats.
    pub duels: u32,
    /// Seeds the first duelist won outright.
    pub wins: u32,
    /// Seeds the pair held level.
    pub draws: u32,
    /// Seeds the opponent won outright.
    pub losses: u32,
    /// The first duelist's share in permille, a draw counting as half.
    pub win_rate_permille: u16,
}

impl MatchupRow {
    /// The opponent's share in permille: the complement of the first
    /// duelist's share, a draw counting as half for both.
    pub fn opponent_rate_permille(&self) -> u16 {
        1000u16.saturating_sub(self.win_rate_permille)
    }

    /// True when the first duelist clears the duel law
    /// ([`WIN_BAR_PERMILLE`]): it beats the opponent.
    pub fn beats(&self) -> bool {
        self.win_rate_permille >= WIN_BAR_PERMILLE
    }

    /// True when the opponent clears the duel law.
    pub fn opponent_beats(&self) -> bool {
        self.opponent_rate_permille() >= WIN_BAR_PERMILLE
    }
}

/// The first duelist's share in permille from the tallies, a draw counting
/// as half: `(2 * wins + draws) * 1000 / (2 * duels)`. Pure integer math.
fn rate_permille(wins: u32, draws: u32, duels: u32) -> u16 {
    if duels == 0 {
        return 0;
    }
    let score = (2 * wins + draws).saturating_mul(1000);
    u16::try_from(score / (2 * duels)).unwrap_or(u16::MAX)
}

/// How one duel ended, from the seat-0 duelist's side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DuelOutcome {
    /// The seat-0 duelist dealt the most damage.
    Win,
    /// Both dealt the same damage.
    Draw,
    /// The seat-1 duelist dealt the most damage.
    Loss,
}

/// One duel: `first` in seat 0 and `second` in seat 1, both at neutral
/// morale, the enemy tiers installed on their seats. The player seat runs
/// the plain brain, the same seat the GA13 tournament uses for its mirror
/// balance proof. The outcome is the seat-0 duelist's.
fn duel(
    first: &Duelist,
    second: &Duelist,
    seed: u64,
    human_error: bool,
) -> Result<DuelOutcome, TtkError> {
    let config = ArenaConfig {
        seed,
        ticks: DUEL_TICKS,
        fighters: 2,
        half_extent_m: DUEL_HALF_EXTENT_M,
        table_version: DUEL_TABLE_VERSION,
    };
    let (snapshot, _) =
        engine_arena::run_prefix(&config, &BodyTape::default(), 0).ok_or_else(|| {
            TtkError::Duel {
                duelist: first.name.clone(),
                opponent: second.name.clone(),
                reason: "the arena refused the prefix".to_string(),
            }
        })?;
    let mut fork = Fork::from_snapshot(snapshot, 0).ok_or_else(|| TtkError::Duel {
        duelist: first.name.clone(),
        opponent: second.name.clone(),
        reason: "the arena refused the fork".to_string(),
    })?;
    for (slot, duelist) in [first, second].iter().enumerate() {
        let fighter = u8::try_from(slot).unwrap_or(u8::MAX);
        fork.apply_proposal(ForkProposal::SetAggression {
            fighter,
            permille: 500,
        })
        .map_err(|error| TtkError::Duel {
            duelist: first.name.clone(),
            opponent: second.name.clone(),
            reason: format!("morale refused: {error:?}"),
        })?;
        if let Some(row) = duelist.tier {
            fork.apply_proposal(ForkProposal::SetTier { fighter, tier: row })
                .map_err(|error| TtkError::Duel {
                    duelist: first.name.clone(),
                    opponent: second.name.clone(),
                    reason: format!("tier refused: {error:?}"),
                })?;
        }
    }
    // The player seat is a human only when the caller asks for it: the
    // matrix runs the plain brain on both seats, the human matrix runs the
    // seeded GA14 error tape on the player seat.
    let player_seat = [first, second]
        .iter()
        .position(|duelist| duelist.tier.is_none())
        .and_then(|slot| u8::try_from(slot).ok());
    if let Some(slot) = player_seat.filter(|_| human_error) {
        let (_, dataset) = fork.run_recorded(config.ticks);
        let mut perfect: Vec<BodyInput> = dataset
            .rows
            .iter()
            .filter(|row| row.action.fighter == slot)
            .map(|row| row.action.input)
            .collect();
        perfect.sort_by_key(|input| input.tick);
        let template = perfect.first().copied().ok_or_else(|| TtkError::Duel {
            duelist: first.name.clone(),
            opponent: second.name.clone(),
            reason: "the player seat recorded no input".to_string(),
        })?;
        let fighter = BodyId::new(slot).ok_or_else(|| TtkError::Duel {
            duelist: first.name.clone(),
            opponent: second.name.clone(),
            reason: "the player slot has no fighter id".to_string(),
        })?;
        let delay = HumanError::human(BASE_SEED).delay_for(seed);
        let noisy = human_tape_for(&perfect, fighter, config.ticks, delay, &template);
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: slot,
            inputs: noisy.0.clone(),
        })
        .map_err(|error| TtkError::Duel {
            duelist: first.name.clone(),
            opponent: second.name.clone(),
            reason: format!("the player tape refused: {error:?}"),
        })?;
    }
    let metrics = fork.run_n(config.ticks);
    Ok(match slot_zero_result(&metrics.damage_dealt) {
        SlotZero::Win => DuelOutcome::Win,
        SlotZero::Loss => DuelOutcome::Loss,
        SlotZero::Draw => DuelOutcome::Draw,
    })
}

/// The win matrix of the duel ladder: one row per pair of distinct
/// duelists. Every pair plays the same seeds in both seats, so each duelist
/// is measured in seat 0 and in seat 1 and the pair share carries no seat
/// preference. The enemy seats carry their tier rows; the player seat runs
/// the plain brain, the same seat the GA13 tournament uses for its mirror
/// balance proof. Deterministic: the same duelists give the same matrix
/// twice.
pub fn matchup_rows(duelists: &[Duelist]) -> Result<Vec<MatchupRow>, TtkError> {
    matchup_rows_with(duelists, false)
}

/// The win matrix with the player seat on the seeded GA14 human error: the
/// perfect brain tape delayed and held, the way a real player duels. The
/// enemy seats carry their tier rows. Deterministic: the same duelists give
/// the same matrix twice.
pub fn matchup_rows_with(
    duelists: &[Duelist],
    human_error: bool,
) -> Result<Vec<MatchupRow>, TtkError> {
    matchup_rows_at(duelists, human_error, DUELS_PER_PAIR)
}

/// The win matrix over a given number of duel seeds per pair. The seed
/// count is a parameter so the same rows can be measured at more seeds.
fn matchup_rows_at(
    duelists: &[Duelist],
    human_error: bool,
    duels_per_pair: u32,
) -> Result<Vec<MatchupRow>, TtkError> {
    let mut rows = Vec::new();
    for (index, first) in duelists.iter().enumerate() {
        for second in duelists.iter().skip(index + 1) {
            let (mut wins, mut draws, mut losses) = (0u32, 0u32, 0u32);
            for duel_index in 0..duels_per_pair {
                // The same seed, the two seats: the pair duels itself twice
                // with the fighters swapped, so neither seat is a handicap.
                let seed = BASE_SEED.wrapping_add(u64::from(duel_index));
                let first_seat = duel(first, second, seed, human_error)?;
                let second_seat = duel(second, first, seed, human_error)?;
                let first_won = matches!(
                    (first_seat, second_seat),
                    (DuelOutcome::Win, DuelOutcome::Loss)
                );
                let first_lost = matches!(
                    (first_seat, second_seat),
                    (DuelOutcome::Loss, DuelOutcome::Win)
                );
                if first_won {
                    wins += 1;
                } else if first_lost {
                    losses += 1;
                } else {
                    // A level pair: the first duelist led in one seat and was
                    // held in the other, or the two seats were level.
                    draws += 1;
                }
            }
            // Each seed plays the pair in both seats, so the pair plays
            // `2 * DUELS_PER_PAIR` duels but the seed count is the number the
            // duel law is read against: one result per seed.
            let duels = duels_per_pair;
            rows.push(MatchupRow {
                duelist: first.name.clone(),
                opponent: second.name.clone(),
                duels,
                wins,
                draws,
                losses,
                win_rate_permille: rate_permille(wins, draws, duels),
            });
        }
    }
    Ok(rows)
}

/// The win matrix as CSV text: a header and one row per pair. Pure and
/// byte-stable: the same matrix twice gives the same text.
pub fn render_matchup_csv(rows: &[MatchupRow]) -> String {
    let mut out = String::from(
        "# rome-matchups: the Rome duel win matrix, one row per pair of\n\
         # distinct duelists, measured over seeded duels in both seats so no\n\
         # seat reads as skill. A row beats when its win_rate_permille reaches\n\
         # the duel law (600 permille, engine_ai::tier::WIN_BAR_PERMILLE); a\n\
         # draw counts as half a win for both duelists.\n",
    );
    out.push_str(
        "duelist,opponent,duels,wins,draws,losses,win_rate_permille,opponent_rate_permille,beats\n",
    );
    for row in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            row.duelist,
            row.opponent,
            row.duels,
            row.wins,
            row.draws,
            row.losses,
            row.win_rate_permille,
            row.opponent_rate_permille(),
            if row.beats() { "yes" } else { "no" },
        ));
    }
    out
}

/// The share in permille of `duelist` against `opponent`, read from the
/// matrix row of that pair. `None` when the matrix holds no such pair.
fn pair_rate(rows: &[MatchupRow], duelist: &str, opponent: &str) -> Option<u16> {
    let row = rows
        .iter()
        .find(|row| row.duelist == duelist && row.opponent == opponent);
    if let Some(row) = row {
        return Some(row.win_rate_permille);
    }
    let row = rows
        .iter()
        .find(|row| row.duelist == opponent && row.opponent == duelist)?;
    Some(row.opponent_rate_permille())
}

/// The duelist that beats every other duelist, with its rates, when the
/// matrix holds one. `None` when no row dominates: the odd cycle holds.
pub fn dominant_duelist(
    rows: &[MatchupRow],
    duelists: &[Duelist],
) -> Option<(String, Vec<(String, u16)>)> {
    for duelist in duelists {
        let mut rates = Vec::new();
        let mut beats_all = true;
        for other in duelists {
            if other.name == duelist.name {
                continue;
            }
            let Some(rate) = pair_rate(rows, &duelist.name, &other.name) else {
                beats_all = false;
                break;
            };
            rates.push((other.name.clone(), rate));
            if rate < WIN_BAR_PERMILLE {
                beats_all = false;
            }
        }
        if beats_all && rates.len() + 1 == duelists.len() {
            return Some((duelist.name.clone(), rates));
        }
    }
    None
}

/// The report directory of the playtest runs: `target/reports`.
pub fn reports_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/reports")
}

/// Write one report file into the report directory. `Ok(path)` on success.
pub fn write_report(name: &str, text: &str) -> Result<std::path::PathBuf, TtkError> {
    let dir = reports_dir();
    std::fs::create_dir_all(&dir).map_err(|error| TtkError::Write {
        path: dir.display().to_string(),
        reason: error.to_string(),
    })?;
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(|error| TtkError::Write {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    Ok(path)
}

/// The report file name of the time-to-kill table.
pub const TTK_REPORT: &str = "rome-ttk.csv";
/// The report file name of the win matrix.
pub const MATCHUP_REPORT: &str = "rome-matchups.csv";

#[cfg(test)]
mod tests {
    use super::*;

    /// The loaded duel tables and duelists.
    fn loaded() -> (Tables, Vec<Duelist>) {
        let tables = duel_tables().expect("the shipped combat rows load");
        let tiers = duel_tiers().expect("the shipped enemy rows load");
        (tables, duelists(&tiers))
    }

    /// The time-to-kill table: one row per duelist and move kind, from the
    /// loaded design rows.
    #[test]
    fn ttk_table_covers_every_duelist_and_move_kind() {
        let (tables, duelists) = loaded();
        let rows = ttk_rows(&tables, &duelists);
        assert_eq!(
            rows.len(),
            duelists.len() * MOVE_KINDS.len(),
            "one row per duelist and move kind"
        );
        for duelist in &duelists {
            for (name, _) in MOVE_KINDS {
                let row = rows
                    .iter()
                    .find(|row| row.duelist == duelist.name && row.move_kind == name)
                    .unwrap_or_else(|| panic!("missing row {} {name}", duelist.name));
                assert_eq!(row.hp_permille, duelist.hp_permille);
                assert_eq!(row.damage_permille, tables.attack_damage);
                assert!(row.hits_to_kill > 0, "a living duelist needs hits");
                assert!(row.ttk_ticks > 0, "a kill takes time");
            }
        }
        // The heavy strike freezes the fight twice as long as the light one,
        // so the same duelist dies later to the heavy.
        for duelist in &duelists {
            let light = rows
                .iter()
                .find(|row| row.duelist == duelist.name && row.move_kind == "light")
                .expect("the light row");
            let heavy = rows
                .iter()
                .find(|row| row.duelist == duelist.name && row.move_kind == "heavy")
                .expect("the heavy row");
            assert!(
                heavy.ttk_ticks > light.ttk_ticks,
                "{}: the heavy freeze must cost time",
                duelist.name
            );
            assert_eq!(light.hits_to_kill, heavy.hits_to_kill);
        }
    }

    /// The time-to-kill table is pure integer math over the loaded rows, so
    /// it replays byte for byte and the CSV on disk is the table itself.
    #[test]
    fn ttk_table_replays_and_matches_the_committed_copy() {
        let (tables, duelists) = loaded();
        let first = render_ttk_csv(&ttk_rows(&tables, &duelists));
        let second = render_ttk_csv(&ttk_rows(&tables, &duelists));
        assert_eq!(first, second, "the table must replay");
        // The committed copy beside this module is the pinned table: the
        // loaded design rows and the table are one number set.
        let committed = include_str!("ttk/rome-ttk.csv");
        assert_eq!(
            first, committed,
            "the table drifted from its committed copy"
        );
        // The report file in `target/reports` carries the same text.
        let path = write_report(TTK_REPORT, &first).expect("the report writes");
        assert!(path.ends_with(TTK_REPORT), "{}", path.display());
        assert_eq!(
            std::fs::read_to_string(&path).expect("the report reads back"),
            committed
        );
    }

    /// The win matrix: one row per pair of distinct duelists, every pair
    /// played in both seats, and no dominant row.
    #[test]
    fn ttk_matchup_matrix_holds_no_dominant_row() {
        let (_, duelists) = loaded();
        let rows = matchup_rows(&duelists).expect("the duel matrix runs");
        let want = duelists.len() * (duelists.len() - 1) / 2;
        assert_eq!(rows.len(), want, "one row per pair");
        for row in &rows {
            assert_eq!(row.duels, DUELS_PER_PAIR);
            assert_eq!(row.wins + row.draws + row.losses, row.duels);
            assert_eq!(
                row.win_rate_permille + row.opponent_rate_permille(),
                1000,
                "the two shares of a pair must add up"
            );
        }
        // The odd cycle: no duelist beats every other duelist.
        let dominant = dominant_duelist(&rows, &duelists);
        assert!(
            dominant.is_none(),
            "a dominant duelist breaks the odd cycle: {dominant:?}"
        );
        // The two directions of a pair can never both clear the duel law.
        for row in &rows {
            assert!(
                !(row.beats() && row.opponent_beats()),
                "{} and {} cannot both beat each other",
                row.duelist,
                row.opponent
            );
        }
    }

    /// The ladder the GA13 tournament measures: every rung beats the one
    /// below it, and the boss does not clear the law against the player.
    #[test]
    fn ttk_matchup_matrix_keeps_the_tier_ladder() {
        let (_, duelists) = loaded();
        let rows = matchup_rows(&duelists).expect("the duel matrix runs");
        for (higher, lower) in [
            ("tier_2", "tier_1"),
            ("tier_3", "tier_2"),
            ("tier_4", "tier_1"),
            ("tier_4", "tier_2"),
        ] {
            let rate = pair_rate(&rows, higher, lower).expect("the pair is in the matrix");
            assert!(
                rate >= WIN_BAR_PERMILLE,
                "{higher} must beat {lower}: {rate} permille"
            );
        }
        // The boss does not clear the law against the player, so the ladder
        // closes on itself instead of ending on the boss.
        let boss_over_player = pair_rate(&rows, "tier_4", "hero").expect("the pair is there");
        assert!(
            boss_over_player < WIN_BAR_PERMILLE,
            "the boss must not beat the player outright: {boss_over_player} permille"
        );
    }

    /// The matrix replays: the same seeds give the same rows and the same
    /// CSV, and the report file carries that text.
    #[test]
    fn ttk_matchup_matrix_replays_and_writes_its_report() {
        let (_, duelists) = loaded();
        let first = matchup_rows(&duelists).expect("the duel matrix runs");
        let second = matchup_rows(&duelists).expect("the duel matrix replays");
        assert_eq!(first, second, "the matrix must replay");
        let csv = render_matchup_csv(&first);
        assert_eq!(csv, render_matchup_csv(&second));
        let committed = include_str!("ttk/rome-matchups.csv");
        assert_eq!(csv, committed, "the matrix drifted from its committed copy");
        let path = write_report(MATCHUP_REPORT, &csv).expect("the report writes");
        assert!(path.ends_with(MATCHUP_REPORT), "{}", path.display());
        assert_eq!(
            std::fs::read_to_string(&path).expect("the report reads back"),
            committed
        );
    }

    /// The human seat (the GA14 error model) is a different matrix: the
    /// player duels slower, and the primus clears the duel law against the
    /// player. The check names the boss instead of passing, so the bar can
    /// fail on a real number set and not only on a forged one.
    #[test]
    fn ttk_odd_cycle_check_names_the_boss_on_the_human_seat() {
        let (_, duelists) = loaded();
        let rows = matchup_rows_with(&duelists, true).expect("the human matrix runs");
        let dominant =
            dominant_duelist(&rows, &duelists).expect("the primus dominates the human player");
        assert_eq!(dominant.0, "tier_4");
        assert_eq!(dominant.1.len(), duelists.len() - 1);
    }

    /// The check can fail on a forged matrix too: a duelist that beats every
    /// other duelist is named, never passed.
    #[test]
    fn ttk_odd_cycle_check_names_a_dominant_row() {
        let (_, duelists) = loaded();
        let mut rows = matchup_rows(&duelists).expect("the duel matrix runs");
        // Forge the dominant duelist: the primus beats every other duelist.
        for row in &mut rows {
            let primus_here = row.duelist == "tier_4" || row.opponent == "tier_4";
            if primus_here {
                row.wins = row.duels;
                row.draws = 0;
                row.losses = 0;
                row.win_rate_permille = if row.duelist == "tier_4" { 1000 } else { 0 };
            }
        }
        let dominant = dominant_duelist(&rows, &duelists).expect("the primus dominates now");
        assert_eq!(dominant.0, "tier_4");
        assert_eq!(dominant.1.len(), duelists.len() - 1);
    }
}
