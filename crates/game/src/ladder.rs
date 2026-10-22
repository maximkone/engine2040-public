//! ladder: arena ladder table, glory ranks, per-rung beats, repair hook (LIGHT slice).
//!
//! Consumer: the session loop folds match outcomes into [`Ladder`] and
//! stores the record beside the versioned progression payload. Data owner: this module owns
//! rung, glory and repair counters. Units: wins, losses and glory are
//! counts; `last_tick` is a fixed-tick cursor. Determinism: integer-only
//! fold over the outcome script in order; same script gives the same state,
//! no clock, no allocation in the fold. Errors: [`LadderError`] on bad rung,
//! outcome, length or version. Budget: `apply` is O(1) time and memory; the
//! state is a 26-byte record on the stack.
//!
//! Rung 0 is the drill at the post (see [`palus`]): the armatura's first
//! rung is 30 s of thrusts at a fixed post with three marked targets, and
//! the duel rungs follow it.
//!
//! The career ranks (Tiro, Miles, Immunis, Optio, Centurio) and the
//! service record line are presentation: words and text over the counters,
//! never part of the hashed state. The data seat is the rank table in
//! `content/design/loop.rows`; this module holds the code seat, and its
//! test reads the file so the two cannot drift.

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
/// Named glory drain: glory paid on a loss, half the base win. A loss
/// subtracts it with a saturating floor at zero, so the counter never
/// falls below zero. Mirrored in `content/design/loop.rows` as
/// `glory_loss_penalty`, so a designer tunes the drain without a Rust edit.
pub const GLORY_LOSS_PENALTY: u32 = 50;
/// Glory floors for ranks `0..GLORY_RANKS.len()`; the rank is the highest
/// index whose floor the glory counter reaches.
pub const GLORY_RANKS: [u32; 5] = [0, 300, 900, 1800, 3600];
/// Career rank names per rank index: the Latin ranks of the legion, Tiro,
/// Miles, Immunis, Optio and Centurio. Words, so they stay out of the
/// hashed state. Source: `docs/plan/rome.md` section 0 decision 4 and
/// section 8; the data seat is the rank table in `content/design/loop.rows`.
pub const RANK_NAMES: [&str; 5] = ["Tiro", "Miles", "Immunis", "Optio", "Centurio"];
/// What each rank does, one word: a recruit, a line soldier, an untrained
/// hand for the shops, the second of a century, its centurion. The
/// officers optio and centurio are Vegetius 2.7-9 and 2.14
/// (`docs/plan/rome.md` section 5).
pub const RANK_DUTIES: [&str; 5] = ["recruit", "soldier", "untrained", "sergeant", "centurion"];
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
/// What every rung asks for: the drill at the post first, then the duels
/// (ROM4, the armatura at the post).
pub const RUNG_KINDS: [RungKind; RUNG_COUNT] = [
    RungKind::Drill,
    RungKind::Duel,
    RungKind::Duel,
    RungKind::Duel,
    RungKind::Duel,
    RungKind::Duel,
    RungKind::Duel,
    RungKind::Duel,
];
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
    /// losses retreat with the floor pinned at zero and drain
    /// [`GLORY_LOSS_PENALTY`] glory. Counters and glory saturate; the cursor
    /// always advances to `tick`.
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
                self.glory = self.glory.saturating_sub(GLORY_LOSS_PENALTY);
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
                self.glory = self.glory.saturating_sub(tables.glory_loss_penalty);
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

    /// Fold one outcome using designer rows text at a cold progression
    /// boundary, never per tick.
    ///
    /// Caller: `crates/game/src/modes_world.rs`
    /// (`ModeSession::settle_with_rows`, progression boundary only).
    /// Parses `text` via `crate::loop_text::parse_loop` and folds the
    /// header-version row via `apply_with_tables`, returning the rows rank.
    /// Units are counts (glory awards, rank floors). A refusal leaves
    /// `self` unchanged, so last-good stays. Same text plus same outcome
    /// always folds the same way. Golden unchanged: `Ladder::apply`
    /// still owns the compiled constants.
    pub fn apply_rows_text(
        &mut self,
        outcome: Outcome,
        tick: u32,
        text: &str,
    ) -> Result<u8, crate::loop_text::LoopRowsError> {
        let set = crate::loop_text::parse_loop(text)?;
        self.apply_with_tables(outcome, tick, &set.tables);
        Ok(self.rank_with_tables(&set.tables))
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

    /// Duty word for a rank index; out-of-range pins to the top duty. Pure.
    #[must_use]
    pub fn rank_duty(rank: u8) -> &'static str {
        RANK_DUTIES
            .get(usize::from(rank))
            .copied()
            .unwrap_or(RANK_DUTIES[RANK_DUTIES.len() - 1])
    }

    /// The service record: the career line the HUD shows instead of a bare
    /// glory counter. Presentation over the counters, never hashed. Pure.
    #[must_use]
    pub fn service_record(&self) -> ServiceRecord {
        let rank = self.rank();
        ServiceRecord {
            rank,
            name: Self::rank_name(rank),
            duty: Self::rank_duty(rank),
            glory: self.glory,
            wins: self.wins,
            losses: self.losses,
            rung: self.rung,
            loops: self.loops,
        }
    }

    /// Beat row for the current rung: enemy tier, new elements, minutes.
    /// The rung pins at [`MAX_RUNG`], so this always reads a chart row. Pure.
    #[must_use]
    pub fn beat(&self) -> RungBeat {
        let index = usize::from(self.rung.min(MAX_RUNG));
        RungBeat {
            kind: RUNG_KINDS[index],
            enemy_tier: RUNG_ENEMY_TIER[index],
            new_elements: RUNG_NEW_ELEMENTS[index],
            minutes: RUNG_MINUTES[index],
        }
    }

    /// What this rung asks for: the drill at the post or a duel. Rung 0 is
    /// the drill; the duel rungs follow it. Pure.
    #[must_use]
    pub fn rung_kind(&self) -> RungKind {
        RUNG_KINDS[usize::from(self.rung.min(MAX_RUNG))]
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

/// The service record: the career line the HUD reads instead of a glory
/// counter. Plain integers plus the rank words, so the line is pure text
/// and never enters the hashed state.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ServiceRecord {
    /// Rank index, `0..=4`.
    pub rank: u8,
    /// Rank name: Tiro, Miles, Immunis, Optio or Centurio.
    pub name: &'static str,
    /// Duty word of the rank: recruit, soldier, untrained, sergeant or
    /// centurion.
    pub duty: &'static str,
    /// Glory counts; the service record's number.
    pub glory: u32,
    /// Bouts won.
    pub wins: u32,
    /// Bouts lost.
    pub losses: u32,
    /// Rung on the beat chart.
    pub rung: u8,
    /// Completed top-rung loops.
    pub loops: u32,
}

impl ServiceRecord {
    /// Append the HUD line to `out`: `{name} ({duty}) - {glory} glory -
    /// {wins} won, {losses} lost`. The window clears one buffer per frame
    /// and pushes this line, so a frame costs no new allocation after
    /// warmup. Pure text.
    pub fn push_line(&self, out: &mut String) {
        out.push_str(self.name);
        out.push_str(" (");
        out.push_str(self.duty);
        out.push_str(") - ");
        push_count(self.glory, out);
        out.push_str(" glory - ");
        push_count(self.wins, out);
        out.push_str(" won, ");
        push_count(self.losses, out);
        out.push_str(" lost");
    }

    /// The same line as its own string, for a one-shot draw or a test.
    #[must_use]
    pub fn line(&self) -> String {
        let mut out = String::new();
        self.push_line(&mut out);
        out
    }
}

/// Decimal text of `value`, pushed digit by digit: no temp string, so a
/// caller that reuses its buffer allocates nothing. Pure.
fn push_count(value: u32, out: &mut String) {
    let mut digits = [0u8; 10];
    let mut index = digits.len();
    let mut rest = value;
    loop {
        index -= 1;
        digits[index] = b'0' + u8::try_from(rest % 10).unwrap_or(0);
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    for slot in digits[index..].iter() {
        out.push(char::from(*slot));
    }
}

/// What a rung asks the trainee to do.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RungKind {
    /// The drill at the post: 30 s of thrusts at the palus, no opponent.
    Drill,
    /// A duel against the tier foe.
    Duel,
}

/// One rung beat: who stands there, what is new, how long it runs.
/// Plain integers only, so the chart replays exactly.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RungBeat {
    /// What the rung asks for: the drill at the post or a duel.
    pub kind: RungKind,
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
        kind: RUNG_KINDS[index],
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
/// (16 wins) reach 4000 glory past the 3600 floor, so the rank is
/// Centurio.
pub const START_TO_RANK_WINS: usize = 16;

/// Fresh ladder driven start to rank with [`START_TO_RANK_WINS`] wins. Pure.
#[must_use]
pub fn start_to_rank() -> Ladder {
    Ladder::run_script(&[Outcome::Win; START_TO_RANK_WINS])
}

/// The drill at the post: the armatura's first rung, before the duels.
///
/// Consumer: the mode loop stages the drill at rung 0 and folds the score
/// into the ladder through [`Ladder::apply_drill`]. Data owner: this module
/// owns the drill rules; the post itself is stage geometry owned by
/// `engine_arena::world::kit`, and the mark heights come from there, so the
/// post and the scoring cannot drift. Units: ticks at 60 Hz, points as
/// counts, mark heights in millimeters. Determinism: integer-only fold over
/// the action script in order; the same script always scores the same.
/// Errors: none; a blow that lands on nothing scores nothing. Budget:
/// O(actions) time, no allocation.
pub mod palus {
    use super::{Ladder, Outcome};

    /// The drill at the post runs 30 seconds.
    pub const DRILL_SECS: u32 = 30;
    /// The drill length in ticks at 60 Hz.
    pub const DRILL_TICKS: u32 = DRILL_SECS * engine_core::TICK_HZ_U32;
    /// The wooden sword of the drill weighs twice the steel one
    /// (Vegetius 1.11), so every blow needs a full second to come back.
    pub const DRILL_SWORD_WEIGHT_MULTIPLIER: u32 = 2;
    /// Ticks a blow needs to come back with the double-weight wooden sword.
    pub const DRILL_RECOVERY_TICKS: u32 = 60;
    /// Points a clean thrust on the head mark scores.
    pub const HEAD_POINTS: u32 = 6;
    /// Points a clean thrust on the body mark scores.
    pub const BODY_POINTS: u32 = 4;
    /// Points a clean thrust on the legs mark scores.
    pub const LEGS_POINTS: u32 = 2;
    /// Points a cut scores, whatever mark it lands on: a cut with the heavy
    /// wooden sword is a wild blow, never a clean thrust.
    pub const CUT_POINTS: u32 = 1;
    /// Half height of a mark band in millimeters: a blade tip inside the
    /// band lands on the mark.
    pub const MARK_BAND_MM: u32 = 150;
    /// Points the drill must score to leave the drill rung: ten clean
    /// thrusts on the head mark.
    pub const PALUS_BAR: u32 = 60;

    /// The three marked targets of the post, top to bottom.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    pub enum Target {
        /// The head mark.
        Head,
        /// The body mark.
        Body,
        /// The legs mark.
        Legs,
    }

    impl Target {
        /// The marked targets in post order: head, body, legs.
        pub const TARGETS: [Target; 3] = [Target::Head, Target::Body, Target::Legs];

        /// Index of the mark in the post's mark table: head 0, body 1, legs 2.
        #[must_use]
        pub const fn index(self) -> usize {
            match self {
                Self::Head => 0,
                Self::Body => 1,
                Self::Legs => 2,
            }
        }

        /// Height of the mark above the floor in millimeters, read from the
        /// staged post, so the post and the scoring cannot drift.
        #[must_use]
        pub fn height_mm(self) -> u32 {
            engine_arena::world::kit::PALUS_POST.marks_mm[self.index()]
        }

        /// Points a clean thrust on this mark scores.
        #[must_use]
        pub const fn points(self) -> u32 {
            match self {
                Self::Head => HEAD_POINTS,
                Self::Body => BODY_POINTS,
                Self::Legs => LEGS_POINTS,
            }
        }
    }

    /// The marked target a blade tip at `height_mm` lands on: the mark whose
    /// band holds the height. `None` when the tip is off every band.
    #[must_use]
    pub fn target_at(height_mm: u32) -> Option<Target> {
        for target in Target::TARGETS {
            let mark = target.height_mm();
            if height_mm >= mark.saturating_sub(MARK_BAND_MM)
                && height_mm <= mark.saturating_add(MARK_BAND_MM)
            {
                return Some(target);
            }
        }
        None
    }

    /// Height of the post top above the floor in millimeters: the drill's
    /// wooden sword cannot reach past it.
    #[must_use]
    pub fn post_height_mm() -> u32 {
        engine_arena::world::kit::PALUS_POST.height_mm
    }

    /// What the trainee does with the wooden sword.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    pub enum Attack {
        /// A thrust: the drill's scoring blow.
        Thrust,
        /// A cut: a wild swing with the double-weight wooden sword.
        Cut,
    }

    /// One blow of the drill script: when it lands, what it is and the
    /// height of the blade tip at the landing, in millimeters.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    pub struct DrillAction {
        /// Tick the blow lands on.
        pub tick: u32,
        /// What the blow is.
        pub attack: Attack,
        /// Height of the blade tip at the landing, millimeters.
        pub height_mm: u32,
    }

    impl DrillAction {
        /// A thrust whose tip lands at `height_mm` on `tick`.
        #[must_use]
        pub const fn thrust(tick: u32, height_mm: u32) -> Self {
            Self {
                tick,
                attack: Attack::Thrust,
                height_mm,
            }
        }

        /// A cut whose tip lands at `height_mm` on `tick`.
        #[must_use]
        pub const fn cut(tick: u32, height_mm: u32) -> Self {
            Self {
                tick,
                attack: Attack::Cut,
                height_mm,
            }
        }
    }

    /// The score of one drill at the post.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct PalusScore {
        /// Points scored by the drill.
        pub points: u32,
        /// Clean thrusts that landed on a mark.
        pub clean_thrusts: u32,
        /// Cuts that landed on the post.
        pub cuts: u32,
        /// Blows that hit nothing: rushed, off the marks or past the post.
        pub misses: u32,
    }

    /// Score a scripted drill: the same script always scores the same.
    ///
    /// A blow inside the recovery gap of the previous one is rushed and
    /// scores nothing: the double-weight wooden sword needs a full second
    /// back. A blow at or past the drill's 30 s never lands. A thrust scores
    /// the points of the mark its tip lands on; a thrust off every mark
    /// scores nothing; a cut scores [`CUT_POINTS`] when it lands on the
    /// post at all.
    #[must_use]
    pub fn score_drill(actions: &[DrillAction]) -> PalusScore {
        let mut score = PalusScore::default();
        let mut last_tick: Option<u32> = None;
        for action in actions {
            if action.tick >= DRILL_TICKS {
                break;
            }
            let rushed = match last_tick {
                Some(last) => action.tick.saturating_sub(last) < DRILL_RECOVERY_TICKS,
                None => false,
            };
            last_tick = Some(action.tick);
            if rushed {
                score.misses = score.misses.saturating_add(1);
                continue;
            }
            match action.attack {
                Attack::Thrust => match target_at(action.height_mm) {
                    Some(target) => {
                        score.points = score.points.saturating_add(target.points());
                        score.clean_thrusts = score.clean_thrusts.saturating_add(1);
                    }
                    None => score.misses = score.misses.saturating_add(1),
                },
                Attack::Cut => {
                    if action.height_mm <= post_height_mm() {
                        score.points = score.points.saturating_add(CUT_POINTS);
                        score.cuts = score.cuts.saturating_add(1);
                    } else {
                        score.misses = score.misses.saturating_add(1);
                    }
                }
            }
        }
        score
    }

    impl Ladder {
        /// Fold a finished drill into the ladder: a drill that scored the
        /// bar is a win and the rung advances to the first duel, a drill
        /// below the bar is a loss and the rung stays. Returns true when the
        /// rung moved. Pure aside from the fold.
        pub fn apply_drill(&mut self, drill: &PalusScore, tick: u32) -> bool {
            if drill.points >= PALUS_BAR {
                self.apply(Outcome::Win, tick);
                true
            } else {
                self.apply(Outcome::Loss, tick);
                false
            }
        }
    }
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
    fn loss_drains_glory_and_floors_at_zero() {
        let mut ladder = Ladder::new();
        ladder.apply(Outcome::Win, 0);
        assert_eq!(ladder.glory, GLORY_PER_WIN);
        ladder.apply(Outcome::Loss, 1);
        assert_eq!(ladder.glory, GLORY_PER_WIN - GLORY_LOSS_PENALTY);
        // The drain saturates: a loss at zero glory never goes negative.
        let mut broke = Ladder::new();
        broke.apply(Outcome::Loss, 0);
        assert_eq!(broke.glory, 0);
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
        assert_eq!(Ladder::rank_name(ladder.rank()), "Miles");
    }

    #[test]
    fn rank_name_pins_out_of_range_to_top() {
        assert_eq!(Ladder::rank_name(u8::MAX), "Centurio");
        assert_eq!(Ladder::rank_duty(u8::MAX), "centurion");
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
        const { assert!(FIRST_BOUT_START_SECS <= FIRST_BOUT_START_LIMIT_SECS) };
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
        assert_eq!(GLORY_LOSS_PENALTY, 50);
        assert_eq!(GLORY_RANKS, [0, 300, 900, 1800, 3600]);
        assert_eq!(
            RANK_NAMES,
            ["Tiro", "Miles", "Immunis", "Optio", "Centurio"]
        );
        assert_eq!(
            RANK_DUTIES,
            ["recruit", "soldier", "untrained", "sergeant", "centurion"]
        );
        assert_eq!(RECORD_BYTES, 26);
        assert_eq!(START_TO_RANK_WINS, 16);
    }

    #[test]
    fn light_gate_holds_over_owned_sources() {
        const LADDER_SRC: &str = include_str!("ladder.rs");
        let head = LADDER_SRC
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
    fn start_to_rank_reaches_centurio() {
        let ladder = start_to_rank();
        assert_eq!(ladder.wins, START_TO_RANK_WINS as u32);
        assert_eq!(ladder.losses, 0);
        assert_eq!((ladder.rung, ladder.loops), (0, 2));
        assert_eq!(ladder.glory, 4000);
        assert_eq!(ladder.rank(), 4);
        assert_eq!(Ladder::rank_name(ladder.rank()), "Centurio");
        // Rank floors on the way: 2 wins stay Tiro, 3 wins step up, one
        // full loop (8 wins) reaches Optio at 2000 glory.
        let two = Ladder::run_script(&wins(2));
        assert_eq!((two.glory, two.rank()), (225, 0));
        let three = Ladder::run_script(&wins(3));
        assert_eq!((three.glory, three.rank()), (375, 1));
        assert_eq!(Ladder::rank_name(three.rank()), "Miles");
        let one_loop = Ladder::run_script(&wins(8));
        assert_eq!((one_loop.glory, one_loop.loops), (2000, 1));
        assert_eq!(one_loop.rank(), 3);
        assert_eq!(Ladder::rank_name(one_loop.rank()), "Optio");
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
        assert_eq!(set.tables.glory_loss_penalty, GLORY_LOSS_PENALTY);
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

    #[test]
    fn rows_text_entry_folds_like_compiled() {
        let text = include_str!("../../../content/design/loop.rows");
        let mut from_text = Ladder::new();
        let rank = from_text
            .apply_rows_text(Outcome::Win, 0, text)
            .expect("shipped rows fold");
        let mut compiled = Ladder::new();
        compiled.apply(Outcome::Win, 0);
        assert_eq!(from_text, compiled);
        assert_eq!(rank, compiled.rank());
    }

    #[test]
    fn rows_text_tuned_moves_glory_and_bad_keeps_last_good() {
        let text = include_str!("../../../content/design/loop.rows");
        let tuned = text
            .replace("table loop version 1", "table loop version 2")
            .replace("v1 glory_per_win=100", "v2 glory_per_win=150");
        let mut modded = Ladder::new();
        let rank = modded
            .apply_rows_text(Outcome::Win, 0, &tuned)
            .expect("tuned rows fold");
        let mut base = Ladder::new();
        base.apply(Outcome::Win, 0);
        assert_eq!(modded.glory, base.glory.saturating_add(50));
        assert_eq!(rank, base.rank());
        let before = modded;
        assert!(modded
            .apply_rows_text(Outcome::Win, 1, "table loop version 0\nv0 glory_per_win=1")
            .is_err());
        assert_eq!(modded, before, "refused rows never fold");
    }

    #[test]
    fn shipped_rows_carry_the_loss_drain() {
        // F2P: the rows file must carry the glory drain key, so a designer
        // tunes a lost bout without a Rust edit.
        let text = include_str!("../../../content/design/loop.rows");
        assert!(
            text.contains("glory_loss_penalty="),
            "loop.rows must carry the glory loss drain key"
        );
        let set = crate::loop_text::parse_loop(text).expect("shipped loop.rows parses");
        assert_eq!(set.tables.glory_loss_penalty, GLORY_LOSS_PENALTY);
        let mut from_rows = Ladder::new();
        from_rows
            .apply_rows_text(Outcome::Win, 0, text)
            .expect("shipped win folds");
        from_rows
            .apply_rows_text(Outcome::Loss, 1, text)
            .expect("shipped loss folds");
        let mut compiled = Ladder::new();
        compiled.apply(Outcome::Win, 0);
        compiled.apply(Outcome::Loss, 1);
        assert_eq!(from_rows, compiled, "shipped rows drain like compiled");
    }

    #[test]
    fn rows_loss_penalty_tunes_the_drain() {
        // One number in the rows text, no Rust edit: the drain follows the row.
        let text = include_str!("../../../content/design/loop.rows");
        let tuned = text
            .replace("table loop version 1", "table loop version 2")
            .replace("v1 glory_per_win", "v2 glory_per_win")
            .replace("glory_loss_penalty=50", "glory_loss_penalty=80");
        let mut base = Ladder::new();
        base.apply_rows_text(Outcome::Win, 0, text)
            .expect("shipped win folds");
        base.apply_rows_text(Outcome::Loss, 1, text)
            .expect("shipped loss folds");
        let mut modded = Ladder::new();
        modded
            .apply_rows_text(Outcome::Win, 0, &tuned)
            .expect("tuned win folds");
        modded
            .apply_rows_text(Outcome::Loss, 1, &tuned)
            .expect("tuned loss folds");
        assert_eq!(base.glory, GLORY_PER_WIN.saturating_sub(GLORY_LOSS_PENALTY));
        assert_eq!(modded.glory, GLORY_PER_WIN.saturating_sub(80));
        assert_eq!(modded.wins, base.wins);
        assert_eq!(modded.rung, base.rung);
        // A drain the ranges refuse folds nothing: last-good stays.
        let bad = text.replace("glory_loss_penalty=50", "glory_loss_penalty=20000");
        let before = modded;
        assert!(modded.apply_rows_text(Outcome::Loss, 2, &bad).is_err());
        assert_eq!(modded, before);
    }

    /// Rank table of a loop rows text: the `ranks Name=floor ...` line of
    /// the header comment, in file order.
    fn rank_table(text: &str) -> Vec<(String, u32)> {
        let mut out = Vec::new();
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("# ranks ") else {
                continue;
            };
            for pair in rest.split_whitespace() {
                let (name, floor) = pair.split_once('=').expect("a rank pair holds =");
                let floor = floor.parse::<u32>().expect("a rank floor is a number");
                out.push((name.to_string(), floor));
            }
        }
        out
    }

    /// Rank names of a ladder rows text: every rung row's `rank=` cell and
    /// its trailing `# rank <n> <name>` comment.
    fn rung_rank_names(text: &str) -> Vec<(usize, usize, String)> {
        let mut out = Vec::new();
        for line in text.lines() {
            let body = line.trim();
            if !body.starts_with("rung_") {
                continue;
            }
            let (cells, comment) = body.split_once('#').expect("a rung row names its rank");
            let cell = cells
                .split_whitespace()
                .find_map(|part| part.strip_prefix("rank="))
                .expect("a rung row holds a rank cell")
                .parse::<usize>()
                .expect("a rank cell is a number");
            let mut words = comment.split_whitespace();
            assert_eq!(
                words.next(),
                Some("rank"),
                "the rung comment names the rank"
            );
            let named = words
                .next()
                .expect("the rung comment holds the rank index")
                .parse::<usize>()
                .expect("the named rank index is a number");
            let name = words.next().expect("the rung comment holds the rank name");
            out.push((cell, named, name.to_string()));
        }
        out
    }

    #[test]
    fn rank_names_are_the_roman_career_ranks() {
        // The career replaces the gladiator ranks; the floors stay put.
        assert_eq!(
            RANK_NAMES,
            ["Tiro", "Miles", "Immunis", "Optio", "Centurio"]
        );
        for (index, floor) in GLORY_RANKS.iter().enumerate() {
            let mut ladder = Ladder::new();
            ladder.glory = *floor;
            assert_eq!(
                usize::from(ladder.rank()),
                index,
                "floor {floor} holds rank {index}"
            );
            assert_eq!(Ladder::rank_name(ladder.rank()), RANK_NAMES[index]);
        }
    }

    #[test]
    fn shipped_rows_carry_the_rank_table() {
        // The data seat: the rank names and their floors live in the file.
        let text = include_str!("../../../content/design/loop.rows");
        let table = rank_table(text);
        assert_eq!(table.len(), RANK_NAMES.len(), "one name per rank");
        for (index, (name, floor)) in table.iter().enumerate() {
            assert_eq!(name, RANK_NAMES[index], "rank {index} keeps its name");
            assert_eq!(*floor, GLORY_RANKS[index], "rank {index} keeps its floor");
        }
    }

    #[test]
    fn shipped_ladder_rows_name_every_rank() {
        // The rung chart names the rank each rung reaches, and the name
        // agrees with the code seat.
        let text = include_str!("../../../content/design/ladder.rows");
        let names = rung_rank_names(text);
        assert_eq!(names.len(), RUNG_COUNT, "every rung names its rank");
        for (cell, named, name) in names {
            assert_eq!(cell, named, "the rank cell and the comment agree");
            assert_eq!(name, RANK_NAMES[named]);
        }
    }

    /// A script of 30 thrusts, one per second: the whole drill.
    fn palus_thrust_script() -> Vec<palus::DrillAction> {
        (0..palus::DRILL_SECS)
            .map(|index| {
                palus::DrillAction::thrust(
                    index * palus::DRILL_RECOVERY_TICKS,
                    palus::Target::Head.height_mm(),
                )
            })
            .collect()
    }

    #[test]
    fn palus_drill_scores_the_same_twice() {
        let script = palus_thrust_script();
        let first = palus::score_drill(&script);
        let second = palus::score_drill(&script);
        assert_eq!(first, second, "the same script scores the same");
        assert_eq!(first.points, palus::DRILL_SECS * palus::HEAD_POINTS);
        assert_eq!(first.clean_thrusts, palus::DRILL_SECS);
        assert_eq!(first.misses, 0);
    }

    #[test]
    fn palus_cut_scores_less_than_a_thrust() {
        let head = palus::Target::Head.height_mm();
        let thrust = palus::score_drill(&[palus::DrillAction::thrust(0, head)]);
        let cut = palus::score_drill(&[palus::DrillAction::cut(0, head)]);
        assert_eq!(thrust.points, palus::HEAD_POINTS);
        assert_eq!(cut.points, palus::CUT_POINTS);
        assert!(
            cut.points < thrust.points,
            "a cut scores less than a thrust"
        );
        for target in palus::Target::TARGETS {
            let thrust = palus::score_drill(&[palus::DrillAction::thrust(0, target.height_mm())]);
            let cut = palus::score_drill(&[palus::DrillAction::cut(0, target.height_mm())]);
            assert!(
                cut.points < thrust.points,
                "a cut scores less than a thrust on {target:?}"
            );
        }
    }

    #[test]
    fn palus_rung_advances_at_the_bar() {
        let mut ladder = Ladder::new();
        assert_eq!(ladder.rung, 0);
        let short = palus::score_drill(&[palus::DrillAction::thrust(
            0,
            palus::Target::Body.height_mm(),
        )]);
        assert!(short.points < palus::PALUS_BAR);
        assert!(
            !ladder.apply_drill(&short, 10),
            "a short drill keeps the rung"
        );
        assert_eq!(ladder.rung, 0);
        assert_eq!(ladder.losses, 1);
        let at_bar = palus::PalusScore {
            points: palus::PALUS_BAR,
            ..palus::PalusScore::default()
        };
        assert!(ladder.apply_drill(&at_bar, 20), "the bar advances the rung");
        assert_eq!(ladder.rung, 1);
        assert_eq!(ladder.wins, 1);
        assert_eq!(ladder.rung_kind(), RungKind::Duel);
    }

    #[test]
    fn palus_rung_zero_is_the_drill_and_the_duels_follow() {
        let fresh = Ladder::new();
        assert_eq!(fresh.rung_kind(), RungKind::Drill, "rung 0 is the drill");
        assert_eq!(fresh.beat().kind, RungKind::Drill);
        for rung in 1..=MAX_RUNG {
            let mut ladder = Ladder::new();
            ladder.rung = rung;
            assert_eq!(ladder.rung_kind(), RungKind::Duel, "rung {rung} is a duel");
        }
    }

    #[test]
    fn palus_marks_are_the_staged_post_marks() {
        let post = engine_arena::world::kit::PALUS_POST;
        assert_eq!(post.marks_mm.len(), palus::Target::TARGETS.len());
        for target in palus::Target::TARGETS {
            assert_eq!(target.height_mm(), post.marks_mm[target.index()]);
            assert_eq!(
                palus::target_at(target.height_mm()),
                Some(target),
                "a tip at the mark lands on it"
            );
        }
        assert_eq!(palus::target_at(0), None, "the floor is off every band");
        assert_eq!(
            palus::target_at(post.height_mm + palus::MARK_BAND_MM + 1),
            None,
            "past the post top is off every band"
        );
    }

    #[test]
    fn palus_rushed_blow_scores_nothing() {
        let head = palus::Target::Head.height_mm();
        let script = [
            palus::DrillAction::thrust(0, head),
            palus::DrillAction::thrust(palus::DRILL_RECOVERY_TICKS - 1, head),
        ];
        let score = palus::score_drill(&script);
        assert_eq!(score.clean_thrusts, 1);
        assert_eq!(score.misses, 1, "the second blow was rushed");
        assert_eq!(score.points, palus::HEAD_POINTS);
    }

    #[test]
    fn palus_drill_ends_after_thirty_seconds() {
        let head = palus::Target::Head.height_mm();
        let late = palus::score_drill(&[palus::DrillAction::thrust(palus::DRILL_TICKS, head)]);
        assert_eq!(late.points, 0, "the drill is over");
        assert_eq!(late.misses, 0, "a blow past the end never lands");
        assert_eq!(
            palus::DRILL_TICKS,
            palus::DRILL_SECS * engine_core::TICK_HZ_U32
        );
    }

    #[test]
    fn service_record_reads_the_career() {
        let fresh = Ladder::new().service_record();
        assert_eq!((fresh.rank, fresh.name, fresh.duty), (0, "Tiro", "recruit"));
        assert_eq!(fresh.line(), "Tiro (recruit) - 0 glory - 0 won, 0 lost");
        let ladder = Ladder::run_script(&wins(3));
        let record = ladder.service_record();
        assert_eq!(
            (record.rank, record.name, record.duty),
            (1, "Miles", "soldier")
        );
        assert_eq!(record.glory, 375);
        assert_eq!((record.wins, record.losses), (3, 0));
        assert_eq!(record.line(), "Miles (soldier) - 375 glory - 3 won, 0 lost");
        // The window reuses one buffer: the line is the same, no new string.
        let mut buffer = String::new();
        record.push_line(&mut buffer);
        assert_eq!(buffer, record.line());
    }
}
