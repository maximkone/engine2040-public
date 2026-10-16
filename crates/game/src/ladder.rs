//! ladder: arena ladder table, glory ranks, per-rung beats, repair hook (LIGHT slice).
//!
//! Consumer: display modes (`modes.rs`) reads [`LadderRecord`]; the
//! session loop folds match outcomes into [`Ladder`] and stores the record
//! beside the versioned progression payload. Data owner: this module owns
//! rung, glory and repair counters. Units: wins, losses and glory are
//! counts; `last_tick` is a fixed-tick cursor. Determinism: integer-only
//! fold over the outcome script in order; same script gives the same state,
//! no clock, no allocation. Errors: [`LadderError`] on bad rung, outcome,
//! length or version. Budget: `apply` is O(1) time and memory; the state is
//! a 26-byte record on the stack.

/// Payload version written by [`LadderRecord::encode`].
pub const LADDER_VERSION: u8 = 1;
/// Highest rung index; rungs run `0..=MAX_RUNG`.
pub const MAX_RUNG: u8 = 7;
/// Glory per win, plus [`GLORY_RUNG_BONUS`] per rung held.
pub const GLORY_PER_WIN: u32 = 100;
/// Extra glory per win per rung held (rewards climbing).
pub const GLORY_RUNG_BONUS: u32 = 25;
/// Glory granted on completing a loop at the top rung.
pub const GLORY_PER_LOOP: u32 = 500;
/// Glory floors for ranks `0..GLORY_RANKS.len()`; the rank is the highest
/// index whose floor the glory counter reaches.
pub const GLORY_RANKS: [u32; 5] = [0, 300, 900, 1800, 3600];
/// Display names per rank index, Roman ladder flavor.
pub const RANK_NAMES: [&str; 5] = ["Tiro", "Gladiator", "Primus", "Champion", "Legend"];
/// Exact record length: version (1) + wins (4) + losses (4) + glory (4) +
/// rung (1) + loops (4) + pending repairs (4) + last tick (4).
pub const RECORD_BYTES: usize = 26;
/// Rungs on the table: `0..=MAX_RUNG`, so this many beat rows.
pub const RUNG_COUNT: usize = MAX_RUNG as usize + 1;
/// Enemy tier per rung, 1 recruit through 4 boss. Never falls; boss is last.
pub const RUNG_ENEMY_TIER: [u8; RUNG_COUNT] = [1, 1, 2, 2, 3, 3, 3, 4];
/// New elements (a move, a rule, an arena variant) taught per rung. At most
/// [`MAX_NEW_ELEMENTS_PER_RUNG`] each, so no rung clumps.
pub const RUNG_NEW_ELEMENTS: [u8; RUNG_COUNT] = [2, 1, 2, 1, 2, 1, 2, 1];
/// Expected minutes per rung bout, counts.
pub const RUNG_MINUTES: [u32; RUNG_COUNT] = [2, 2, 3, 3, 4, 4, 5, 6];
/// Boss tier id, always the last rung.
pub const BOSS_TIER: u8 = 4;
/// Most new elements one rung may teach.
pub const MAX_NEW_ELEMENTS_PER_RUNG: u8 = 3;
/// Seconds from launch to the first bout start.
pub const FIRST_BOUT_START_SECS: u32 = 45;
/// Last legal second for the first bout start.
pub const FIRST_BOUT_START_LIMIT_SECS: u32 = 60;

/// Ladder failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LadderError {
    /// Rung above [`MAX_RUNG`], culprit carried.
    BadRung(u8),
    /// Outcome byte outside `0..=1`, byte carried.
    BadOutcome(u8),
    /// Record length is not [`RECORD_BYTES`], length carried.
    BadLength(usize),
    /// Record version is newer than [`LADDER_VERSION`], version carried.
    FutureVersion(u8),
}

impl core::fmt::Display for LadderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadRung(rung) => write!(f, "ladder rung {rung} above {MAX_RUNG}"),
            Self::BadOutcome(byte) => write!(f, "ladder outcome byte {byte}"),
            Self::BadLength(len) => write!(f, "ladder record length {len}"),
            Self::FutureVersion(version) => {
                write!(
                    f,
                    "ladder record version {version} is newer than v{LADDER_VERSION}"
                )
            }
        }
    }
}

impl std::error::Error for LadderError {}

/// One decided match from the human fighter's side.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Outcome {
    /// Human scored the knockdown: climb one rung, a top-rung win loops.
    Win,
    /// Human suffered the knockdown: retreat one rung, floor pins at zero.
    Loss,
}

impl Outcome {
    /// Decode `0` (win) or `1` (loss); anything else fails. Pure.
    pub fn decode(byte: u8) -> Result<Self, LadderError> {
        match byte {
            0 => Ok(Self::Win),
            1 => Ok(Self::Loss),
            other => Err(LadderError::BadOutcome(other)),
        }
    }

    /// Encode to the byte [`Outcome::decode`] reads. Pure.
    #[must_use]
    pub fn encode(self) -> u8 {
        match self {
            Self::Win => 0,
            Self::Loss => 1,
        }
    }
}

/// Arena ladder state: rung table position, glory-rank counters and the
/// repair hook backlog. Plain integers only, so folds replay exactly.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Ladder {
    /// Knockdowns scored by the human fighter.
    pub wins: u32,
    /// Knockdowns suffered by the human fighter.
    pub losses: u32,
    /// Accumulated glory; [`Ladder::rank`] derives the rank from it.
    pub glory: u32,
    /// Table position, always `0..=MAX_RUNG`.
    pub rung: u8,
    /// Completed top-rung loops.
    pub loops: u32,
    /// Fixed-tick cursor of the last applied outcome.
    pub last_tick: u32,
    /// Repairs owed by completed loops, drained via [`Ladder::take_repair`].
    pending_repairs: u32,
}

impl Ladder {
    /// Empty ladder at rung zero with no glory and no repairs owed. Pure.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one decided match at fixed tick `tick`: wins climb (a win on the
    /// top rung completes a loop, resets to rung zero and arms one repair),
    /// losses retreat with the floor pinned at zero. Counters and glory
    /// saturate; the cursor always advances to `tick`.
    pub fn apply(&mut self, outcome: Outcome, tick: u32) {
        self.last_tick = tick;
        match outcome {
            Outcome::Win => {
                self.wins = self.wins.saturating_add(1);
                self.glory = self.glory.saturating_add(
                    GLORY_PER_WIN
                        .saturating_add(GLORY_RUNG_BONUS.saturating_mul(u32::from(self.rung))),
                );
                if self.rung >= MAX_RUNG {
                    self.loops = self.loops.saturating_add(1);
                    self.glory = self.glory.saturating_add(GLORY_PER_LOOP);
                    self.rung = 0;
                    self.pending_repairs = self.pending_repairs.saturating_add(1);
                } else {
                    self.rung += 1;
                }
            }
            Outcome::Loss => {
                self.losses = self.losses.saturating_add(1);
                if self.rung > 0 {
                    self.rung -= 1;
                }
            }
        }
    }

    /// Fold one outcome using explicit designer rows: cold progression
    /// boundary only, never per tick.
    ///
    /// Consumer: the mode framework reads a [`LoopSet`](crate::loop_text::LoopSet)
    /// via [`parse_loop`](crate::loop_text::parse_loop) at a progression
    /// boundary and folds with this; this module never reads a file. Same
    /// discipline as [`Ladder::apply`], but glory numbers come from `tables`,
    /// not the compiled constants. `MAX_RUNG` and rank names stay in code.
    /// Pure aside from `&mut self`; integer math only, no allocation. Golden
    /// unchanged: [`Ladder::apply`] still uses the compiled constants.
    pub fn apply_with_tables(
        &mut self,
        outcome: Outcome,
        tick: u32,
        tables: &crate::loop_text::LoopTables,
    ) {
        self.last_tick = tick;
        match outcome {
            Outcome::Win => {
                self.wins = self.wins.saturating_add(1);
                self.glory =
                    self.glory
                        .saturating_add(tables.glory_per_win.saturating_add(
                            tables.glory_rung_bonus.saturating_mul(u32::from(self.rung)),
                        ));
                if self.rung >= MAX_RUNG {
                    self.loops = self.loops.saturating_add(1);
                    self.glory = self.glory.saturating_add(tables.glory_per_loop);
                    self.rung = 0;
                    self.pending_repairs = self.pending_repairs.saturating_add(1);
                } else {
                    self.rung += 1;
                }
            }
            Outcome::Loss => {
                self.losses = self.losses.saturating_add(1);
                if self.rung > 0 {
                    self.rung -= 1;
                }
            }
        }
    }

    /// Glory rank under explicit designer rows: highest index whose
    /// `tables.glory_ranks` floor is reached. Pure.
    #[must_use]
    pub fn rank_with_tables(&self, tables: &crate::loop_text::LoopTables) -> u8 {
        let mut rank: u8 = 0;
        let mut index: u8 = 0;
        for floor in tables.glory_ranks.iter() {
            if self.glory >= *floor {
                rank = index;
            }
            index = index.saturating_add(1);
        }
        rank
    }

    /// Replay an outcome script from rung zero with the cursor set to each
    /// outcome's index; equals stepwise [`Ladder::apply`] with
    /// `tick = index`. Pure.
    #[must_use]
    pub fn run_script(outcomes: &[Outcome]) -> Self {
        let mut ladder = Self::new();
        for (index, outcome) in outcomes.iter().enumerate() {
            ladder.apply(*outcome, index as u32);
        }
        ladder
    }

    /// Glory rank: highest index whose [`GLORY_RANKS`] floor is reached. Pure.
    #[must_use]
    pub fn rank(&self) -> u8 {
        let mut rank = 0u8;
        for (index, floor) in GLORY_RANKS.iter().enumerate() {
            if self.glory >= *floor {
                rank = index as u8;
            }
        }
        rank
    }

    /// Display name for a rank index; out-of-range pins to the top name. Pure.
    #[must_use]
    pub fn rank_name(rank: u8) -> &'static str {
        RANK_NAMES
            .get(rank as usize)
            .copied()
            .unwrap_or(RANK_NAMES[RANK_NAMES.len() - 1])
    }

    /// Beat row for the current rung: enemy tier, new elements, minutes.
    /// The rung pins at [`MAX_RUNG`], so this always reads a chart row. Pure.
    #[must_use]
    pub fn beat(&self) -> RungBeat {
        let index = usize::from(self.rung.min(MAX_RUNG));
        RungBeat {
            enemy_tier: RUNG_ENEMY_TIER[index],
            new_elements: RUNG_NEW_ELEMENTS[index],
            minutes: RUNG_MINUTES[index],
        }
    }

    /// Repairs owed by completed loops.
    #[must_use]
    pub fn pending_repairs(&self) -> u32 {
        self.pending_repairs
    }

    /// Drain one owed repair: true exactly once per completed loop, false
    /// when nothing is owed. Each loop arms one firing, never more.
    pub fn take_repair(&mut self) -> bool {
        if self.pending_repairs > 0 {
            self.pending_repairs -= 1;
            true
        } else {
            false
        }
    }

    /// Snapshot the counters for storage beside the progression payload. Pure.
    #[must_use]
    pub fn record(&self) -> LadderRecord {
        LadderRecord {
            wins: self.wins,
            losses: self.losses,
            glory: self.glory,
            rung: self.rung,
            loops: self.loops,
            pending_repairs: self.pending_repairs,
            last_tick: self.last_tick,
        }
    }

    /// Rebuild from a snapshot: a rung above [`MAX_RUNG`] fails, anything
    /// else restores exactly (repairs owed survive the roundtrip). Pure.
    pub fn restore(record: &LadderRecord) -> Result<Self, LadderError> {
        if record.rung > MAX_RUNG {
            return Err(LadderError::BadRung(record.rung));
        }
        Ok(Self {
            wins: record.wins,
            losses: record.losses,
            glory: record.glory,
            rung: record.rung,
            loops: record.loops,
            pending_repairs: record.pending_repairs,
            last_tick: record.last_tick,
        })
    }
}

/// One rung beat: who stands there, what is new, how long it runs.
/// Plain integers only, so the chart replays exactly.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RungBeat {
    /// Enemy tier id for this rung.
    pub enemy_tier: u8,
    /// New elements taught on this rung, at most 3.
    pub new_elements: u8,
    /// Expected minutes for the bout, counts.
    pub minutes: u32,
}

/// Beat row for `rung`; `None` past [`MAX_RUNG`]. Pure.
#[must_use]
pub fn rung_beat(rung: u8) -> Option<RungBeat> {
    if rung > MAX_RUNG {
        return None;
    }
    let index = usize::from(rung);
    Some(RungBeat {
        enemy_tier: RUNG_ENEMY_TIER[index],
        new_elements: RUNG_NEW_ELEMENTS[index],
        minutes: RUNG_MINUTES[index],
    })
}

/// First rung index breaking the chart rules over the given columns: new
/// elements above the cap, or a tier below the rung before. A length
/// mismatch fails too. `None` when the columns hold. Pure.
#[must_use]
pub fn first_chart_issue(tiers: &[u8], new_elements: &[u8]) -> Option<usize> {
    let mut prev_tier: u8 = 0;
    let mut index: usize = 0;
    for (tier, fresh) in tiers.iter().zip(new_elements.iter()) {
        if *fresh > MAX_NEW_ELEMENTS_PER_RUNG {
            return Some(index);
        }
        if *tier < prev_tier {
            return Some(index);
        }
        prev_tier = *tier;
        index += 1;
    }
    if tiers.len() != new_elements.len() {
        return Some(index);
    }
    None
}

/// True when the compiled beat chart holds: no clumped rung, the tier
/// never falls, the boss is last, the first bout starts within 60 s. Pure.
#[must_use]
pub fn is_beat_chart_clean() -> bool {
    first_chart_issue(&RUNG_ENEMY_TIER, &RUNG_NEW_ELEMENTS).is_none()
        && RUNG_ENEMY_TIER[RUNG_COUNT - 1] == BOSS_TIER
        && FIRST_BOUT_START_SECS <= FIRST_BOUT_START_LIMIT_SECS
}

/// Storable ladder snapshot: plain integers in a fixed little-endian layout
/// so the caller can persist it beside the versioned progression payload and
/// rebuild the identical [`Ladder`] later. No pointers, no padding.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct LadderRecord {
    /// Knockdowns scored by the human fighter.
    pub wins: u32,
    /// Knockdowns suffered by the human fighter.
    pub losses: u32,
    /// Accumulated glory at snapshot time.
    pub glory: u32,
    /// Table position, always `0..=MAX_RUNG`.
    pub rung: u8,
    /// Completed top-rung loops.
    pub loops: u32,
    /// Repairs owed at snapshot time.
    pub pending_repairs: u32,
    /// Fixed-tick cursor of the last applied outcome.
    pub last_tick: u32,
}

impl LadderRecord {
    /// Fixed-layout little-endian encode. Pure.
    #[must_use]
    pub fn encode(self) -> [u8; RECORD_BYTES] {
        let mut out = [0u8; RECORD_BYTES];
        out[0] = LADDER_VERSION;
        out[1..5].copy_from_slice(&self.wins.to_le_bytes());
        out[5..9].copy_from_slice(&self.losses.to_le_bytes());
        out[9..13].copy_from_slice(&self.glory.to_le_bytes());
        out[13] = self.rung;
        out[14..18].copy_from_slice(&self.loops.to_le_bytes());
        out[18..22].copy_from_slice(&self.pending_repairs.to_le_bytes());
        out[22..26].copy_from_slice(&self.last_tick.to_le_bytes());
        out
    }

    /// Decode exactly [`RECORD_BYTES`] at version [`LADDER_VERSION`]; a rung
    /// above [`MAX_RUNG`] fails. Pure.
    pub fn decode(bytes: &[u8]) -> Result<Self, LadderError> {
        if bytes.len() != RECORD_BYTES {
            return Err(LadderError::BadLength(bytes.len()));
        }
        if bytes[0] != LADDER_VERSION {
            return Err(LadderError::FutureVersion(bytes[0]));
        }
        let mut quad = [0u8; 4];
        quad.copy_from_slice(&bytes[1..5]);
        let wins = u32::from_le_bytes(quad);
        quad.copy_from_slice(&bytes[5..9]);
        let losses = u32::from_le_bytes(quad);
        quad.copy_from_slice(&bytes[9..13]);
        let glory = u32::from_le_bytes(quad);
        let rung = bytes[13];
        if rung > MAX_RUNG {
            return Err(LadderError::BadRung(rung));
        }
        quad.copy_from_slice(&bytes[14..18]);
        let loops = u32::from_le_bytes(quad);
        quad.copy_from_slice(&bytes[18..22]);
        let pending_repairs = u32::from_le_bytes(quad);
        quad.copy_from_slice(&bytes[22..26]);
        let last_tick = u32::from_le_bytes(quad);
        Ok(Self {
            wins,
            losses,
            glory,
            rung,
            loops,
            pending_repairs,
            last_tick,
        })
    }
}

/// CUT scope markers refused by the LIGHT gate (gameplay plan section
/// 8). The full-scope halves never land in this LIGHT slice; the bare words
/// (`economy`, `progression`, `survival`, `building`) stay legal because the
/// versioned payload beside this table is the base game progression. Each entry is split with
/// `concat!` so this very definition never names a joined marker in the
/// scanned head. Pure data; checked in tests via `include_str!`.
pub const CUT_TOKENS: &[&str] = &[
    concat!("skilltrees-", "full"),
    concat!("skilltree-", "full"),
    concat!("crafting-", "full"),
    concat!("economy-", "full"),
    concat!("progression-", "full"),
    concat!("survival-", "full"),
    concat!("building-", "full"),
    concat!("social-", "full"),
    concat!("battle-", "royale"),
    concat!("tower-", "defense"),
];

/// First CUT scope marker named by `src`, if any. Pure.
#[must_use]
pub fn first_cut_token(src: &str) -> Option<&'static str> {
    CUT_TOKENS.iter().copied().find(|token| src.contains(token))
}

/// True when `src` names no CUT scope marker (the LIGHT slice holds). Pure.
#[must_use]
pub fn is_light_clean(src: &str) -> bool {
    first_cut_token(src).is_none()
}

/// Fixture: wins from a fresh ladder to the top rank. Two top-rung loops
/// (16 wins) reach 4000 glory past the 3600 floor, so the rank is Legend.
pub const START_TO_RANK_WINS: usize = 16;

/// Fresh ladder driven start to rank with [`START_TO_RANK_WINS`] wins. Pure.
#[must_use]
pub fn start_to_rank() -> Ladder {
    Ladder::run_script(&[Outcome::Win; START_TO_RANK_WINS])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wins(n: usize) -> Vec<Outcome> {
        vec![Outcome::Win; n]
    }

    #[test]
    fn wins_climb_and_losses_retreat() {
        let mut ladder = Ladder::new();
        ladder.apply(Outcome::Win, 0);
        ladder.apply(Outcome::Win, 1);
        assert_eq!(ladder.rung, 2);
        ladder.apply(Outcome::Loss, 2);
        assert_eq!(ladder.rung, 1);
        assert_eq!((ladder.wins, ladder.losses), (2, 1));
        assert_eq!(ladder.last_tick, 2);
    }

    #[test]
    fn loss_at_floor_pins_at_zero() {
        let mut ladder = Ladder::new();
        ladder.apply(Outcome::Loss, 4);
        assert_eq!((ladder.rung, ladder.losses), (0, 1));
    }

    #[test]
    fn top_rung_win_loops_and_arms_one_repair() {
        let mut ladder = Ladder::run_script(&wins(usize::from(MAX_RUNG)));
        assert_eq!(ladder.rung, MAX_RUNG);
        assert!(!ladder.take_repair());
        ladder.apply(Outcome::Win, 100);
        assert_eq!((ladder.rung, ladder.loops), (0, 1));
        assert!(ladder.take_repair());
        assert!(!ladder.take_repair());
    }

    #[test]
    fn mid_ladder_wins_arm_no_repair() {
        let ladder = Ladder::run_script(&wins(3));
        assert_eq!(ladder.pending_repairs(), 0);
        assert!(!Ladder::new().take_repair());
    }

    #[test]
    fn glory_accumulates_and_rank_steps() {
        let ladder = Ladder::new();
        assert_eq!(ladder.rank(), 0);
        assert_eq!(Ladder::rank_name(ladder.rank()), "Tiro");
        let ladder = Ladder::run_script(&wins(2));
        // 100 + (100 + 25) = 225: below the 300 floor, still rank 0.
        assert_eq!(ladder.glory, 225);
        assert_eq!(ladder.rank(), 0);
        let ladder = Ladder::run_script(&wins(3));
        // Third win at rung 2 adds 150, total 375: rank 1.
        assert_eq!(ladder.glory, 375);
        assert_eq!(ladder.rank(), 1);
        assert_eq!(Ladder::rank_name(ladder.rank()), "Gladiator");
    }

    #[test]
    fn rank_name_pins_out_of_range_to_top() {
        assert_eq!(Ladder::rank_name(u8::MAX), "Legend");
    }

    #[test]
    fn beat_chart_covers_every_rung() {
        assert_eq!(RUNG_COUNT, usize::from(MAX_RUNG) + 1);
        for rung in 0..=MAX_RUNG {
            let beat = rung_beat(rung).expect("every rung has a beat row");
            assert_eq!(beat.enemy_tier, RUNG_ENEMY_TIER[usize::from(rung)]);
            assert_eq!(beat.new_elements, RUNG_NEW_ELEMENTS[usize::from(rung)]);
            assert_eq!(beat.minutes, RUNG_MINUTES[usize::from(rung)]);
        }
        assert_eq!(rung_beat(MAX_RUNG + 1), None);
        assert_eq!(Ladder::new().beat(), rung_beat(0).unwrap());
        let top = Ladder::run_script(&wins(usize::from(MAX_RUNG)));
        assert_eq!(top.beat().enemy_tier, BOSS_TIER);
    }

    #[test]
    fn beat_chart_rules_hold_and_clumps_fail() {
        assert!(is_beat_chart_clean());
        assert_eq!(
            first_chart_issue(&RUNG_ENEMY_TIER, &RUNG_NEW_ELEMENTS),
            None
        );
        // Clumped rung: 6 new elements on one rung fails at that rung.
        let mut clumped = RUNG_NEW_ELEMENTS;
        clumped[2] = MAX_NEW_ELEMENTS_PER_RUNG + 3;
        assert_eq!(first_chart_issue(&RUNG_ENEMY_TIER, &clumped), Some(2));
        // Falling tier fails at the rung that drops.
        let mut fallen = RUNG_ENEMY_TIER;
        fallen[5] = 1;
        assert_eq!(first_chart_issue(&fallen, &RUNG_NEW_ELEMENTS), Some(5));
        // Boss is last and the first bout starts within 60 s.
        assert_eq!(RUNG_ENEMY_TIER[RUNG_COUNT - 1], BOSS_TIER);
        assert!(FIRST_BOUT_START_SECS <= FIRST_BOUT_START_LIMIT_SECS);
        assert_eq!(FIRST_BOUT_START_LIMIT_SECS, 60);
    }

    #[test]
    fn replay_matches_stepwise_and_order_matters() {
        let script = [Outcome::Win, Outcome::Win, Outcome::Loss, Outcome::Win];
        let replayed = Ladder::run_script(&script);
        let mut stepwise = Ladder::new();
        for (index, outcome) in script.iter().enumerate() {
            stepwise.apply(*outcome, index as u32);
        }
        assert_eq!(replayed, stepwise);
        let shuffled =
            Ladder::run_script(&[Outcome::Loss, Outcome::Win, Outcome::Win, Outcome::Win]);
        assert_ne!(replayed, shuffled);
    }

    #[test]
    fn record_roundtrip_is_byte_stable() {
        let ladder = Ladder::run_script(&wins(usize::from(MAX_RUNG) + 1));
        assert_eq!((ladder.rung, ladder.loops), (0, 1));
        let first = ladder.record().encode();
        assert_eq!(first.len(), RECORD_BYTES);
        assert_eq!(first[0], LADDER_VERSION);
        assert_eq!(first, ladder.record().encode());
        let record = LadderRecord::decode(&first).unwrap();
        assert_eq!(record, ladder.record());
        assert_eq!(Ladder::restore(&record).unwrap(), ladder);
    }

    #[test]
    fn corrupt_records_rejected() {
        let bytes = Ladder::new().record().encode();
        for len in [0, 1, RECORD_BYTES - 1, RECORD_BYTES + 1] {
            let mut clipped = bytes.to_vec();
            clipped.resize(len, 0xAB);
            assert_eq!(
                LadderRecord::decode(&clipped),
                Err(LadderError::BadLength(len))
            );
        }
        for version in [LADDER_VERSION + 1, u8::MAX] {
            let mut bad = bytes;
            bad[0] = version;
            assert_eq!(
                LadderRecord::decode(&bad),
                Err(LadderError::FutureVersion(version))
            );
        }
        let mut bad_rung = bytes;
        bad_rung[13] = MAX_RUNG + 1;
        assert_eq!(
            LadderRecord::decode(&bad_rung),
            Err(LadderError::BadRung(MAX_RUNG + 1))
        );
        let record = LadderRecord {
            rung: MAX_RUNG + 1,
            ..LadderRecord::default()
        };
        assert_eq!(
            Ladder::restore(&record),
            Err(LadderError::BadRung(MAX_RUNG + 1))
        );
    }

    #[test]
    fn bad_outcome_bytes_rejected() {
        assert_eq!(Outcome::decode(0), Ok(Outcome::Win));
        assert_eq!(Outcome::decode(1), Ok(Outcome::Loss));
        assert_eq!(Outcome::decode(2), Err(LadderError::BadOutcome(2)));
        assert_eq!(
            Outcome::decode(u8::MAX),
            Err(LadderError::BadOutcome(u8::MAX))
        );
        assert_eq!(Outcome::Win.encode(), 0);
        assert_eq!(Outcome::Loss.encode(), 1);
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let record = LadderRecord {
            wins: u32::MAX,
            glory: u32::MAX,
            ..LadderRecord::default()
        };
        let mut ladder = Ladder::restore(&record).unwrap();
        ladder.apply(Outcome::Win, 7);
        assert_eq!((ladder.wins, ladder.glory), (u32::MAX, u32::MAX));
        assert_eq!(ladder.rung, 1);
    }

    #[test]
    fn errors_name_the_culprit() {
        assert!(format!("{}", LadderError::BadRung(9)).contains('9'));
        assert!(format!("{}", LadderError::BadOutcome(7)).contains('7'));
        assert!(format!("{}", LadderError::BadLength(3)).contains('3'));
        assert!(format!("{}", LadderError::FutureVersion(9)).contains('9'));
    }

    #[test]
    fn locked_tables_match_receipt() {
        assert_eq!(LADDER_VERSION, 1);
        assert_eq!(MAX_RUNG, 7);
        assert_eq!(GLORY_PER_WIN, 100);
        assert_eq!(GLORY_RUNG_BONUS, 25);
        assert_eq!(GLORY_PER_LOOP, 500);
        assert_eq!(GLORY_RANKS, [0, 300, 900, 1800, 3600]);
        assert_eq!(
            RANK_NAMES,
            ["Tiro", "Gladiator", "Primus", "Champion", "Legend"]
        );
        assert_eq!(RECORD_BYTES, 26);
        assert_eq!(START_TO_RANK_WINS, 16);
    }

    #[test]
    fn light_gate_holds_over_owned_sources() {
        const LADDER_SRC: &str = include_str!("ladder.rs");
        const MODES_SRC: &str = include_str!("modes.rs");
        for src in [LADDER_SRC, MODES_SRC] {
            let head = src
                .split("#[cfg(test)]")
                .next()
                .expect("owned file keeps its cfg(test) marker");
            assert!(
                is_light_clean(head),
                "LIGHT slice must name no CUT scope: {:?}",
                first_cut_token(head)
            );
            assert_eq!(first_cut_token(head), None);
        }
    }

    #[test]
    fn cut_strings_fail_the_gate() {
        for token in CUT_TOKENS {
            assert!(
                !is_light_clean(token),
                "joined CUT marker must fail the gate: {token}"
            );
            assert_eq!(first_cut_token(token), Some(*token));
            let planted = format!("repair bench {token} ledger");
            assert!(!is_light_clean(&planted));
            assert_eq!(first_cut_token(&planted), Some(*token));
        }
        assert!(is_light_clean("repair the arena boards between bouts"));
        assert_eq!(
            first_cut_token("repair the arena boards between bouts"),
            None
        );
    }

    #[test]
    fn start_to_rank_reaches_legend() {
        let ladder = start_to_rank();
        assert_eq!(ladder.wins, START_TO_RANK_WINS as u32);
        assert_eq!(ladder.losses, 0);
        assert_eq!((ladder.rung, ladder.loops), (0, 2));
        assert_eq!(ladder.glory, 4000);
        assert_eq!(ladder.rank(), 4);
        assert_eq!(Ladder::rank_name(ladder.rank()), "Legend");
        // Rank floors on the way: 2 wins stay Tiro, 3 wins step up, one
        // full loop (8 wins) reaches Champion at 2000 glory.
        let two = Ladder::run_script(&wins(2));
        assert_eq!((two.glory, two.rank()), (225, 0));
        let three = Ladder::run_script(&wins(3));
        assert_eq!((three.glory, three.rank()), (375, 1));
        assert_eq!(Ladder::rank_name(three.rank()), "Gladiator");
        let one_loop = Ladder::run_script(&wins(8));
        assert_eq!((one_loop.glory, one_loop.loops), (2000, 1));
        assert_eq!(one_loop.rank(), 3);
        assert_eq!(Ladder::rank_name(one_loop.rank()), "Champion");
    }

    #[test]
    fn craft_hook_fires_once_per_loop() {
        let mut ladder = Ladder::new();
        assert!(!ladder.take_repair());
        for index in 0..3u32 {
            ladder.apply(Outcome::Win, index);
        }
        assert_eq!(ladder.pending_repairs(), 0);
        assert!(!ladder.take_repair());
        for index in 3..8 {
            ladder.apply(Outcome::Win, index as u32);
        }
        assert_eq!((ladder.loops, ladder.pending_repairs()), (1, 1));
        assert!(ladder.take_repair());
        assert!(!ladder.take_repair());
        for index in 8..16 {
            ladder.apply(Outcome::Win, index as u32);
        }
        assert_eq!((ladder.loops, ladder.pending_repairs()), (2, 1));
        assert!(ladder.take_repair());
        assert!(!ladder.take_repair());
    }

    #[test]
    fn roundtrip_identical_twice() {
        let ladder = start_to_rank();
        let first = ladder.record().encode();
        let second = ladder.record().encode();
        assert_eq!(first, second);
        let record = LadderRecord::decode(&first).unwrap();
        let again = LadderRecord::decode(&second).unwrap();
        assert_eq!(record, again);
        assert_eq!(record, ladder.record());
        let restored = Ladder::restore(&record).unwrap();
        let restored_again = Ladder::restore(&again).unwrap();
        assert_eq!(restored, ladder);
        assert_eq!(restored_again, ladder);
        assert_eq!(restored.record().encode(), first);
    }

    #[test]
    fn rows_v1_folds_like_compiled() {
        let text = include_str!("../../../content/design/loop.rows");
        let set = crate::loop_text::parse_loop(text).expect("shipped loop.rows parses");
        assert_eq!(set.version, 1);
        assert_eq!(set.tables.glory_per_win, GLORY_PER_WIN);
        assert_eq!(set.tables.glory_rung_bonus, GLORY_RUNG_BONUS);
        assert_eq!(set.tables.glory_per_loop, GLORY_PER_LOOP);
        assert_eq!(set.tables.glory_ranks, GLORY_RANKS);
        let script = [Outcome::Win, Outcome::Win, Outcome::Loss, Outcome::Win];
        let mut compiled = Ladder::new();
        let mut from_rows = Ladder::new();
        for (index, outcome) in script.iter().enumerate() {
            let tick = index as u32;
            compiled.apply(*outcome, tick);
            from_rows.apply_with_tables(*outcome, tick, &set.tables);
        }
        assert_eq!(from_rows, compiled);
        assert_eq!(from_rows.rank_with_tables(&set.tables), compiled.rank());
    }

    #[test]
    fn rows_text_swap_changes_glory_and_bad_refuses() {
        let text = include_str!("../../../content/design/loop.rows");
        let set = crate::loop_text::parse_loop(text).expect("v1 parses");
        let mut tuned = set.tables;
        tuned.glory_per_win = tuned.glory_per_win.saturating_add(50);
        let mut base = Ladder::new();
        let mut modded = Ladder::new();
        base.apply(Outcome::Win, 0);
        modded.apply_with_tables(Outcome::Win, 0, &tuned);
        assert_eq!(modded.glory, base.glory.saturating_add(50));
        assert_eq!(modded.wins, 1);
        let ladder = Ladder::new();
        let before = ladder;
        assert!(crate::loop_text::parse_loop("table loop version 0\nv0 glory_per_win=1").is_err());
        assert_eq!(ladder, before, "bad rows never fold, last-good stays");
        let set = crate::loop_text::parse_loop(text).expect("v1 text parses once");
        assert_eq!(set.version, 1);
        let mut landed = Ladder::new();
        landed.apply_with_tables(Outcome::Win, 0, &set.tables);
        assert_eq!(landed.wins, 1);
        assert_eq!(landed.glory, GLORY_PER_WIN);
    }
}
