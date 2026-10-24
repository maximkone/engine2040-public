//! combo: the shared string counter — one cursor of three stance positions,
//! advanced by every attack and by movement.
//! Owner: engine-combat (code and data).
//! Consumer: the arena match step advances the cursor on an attack press and
//! on dash/shuffle, then reads [`ComboCursor::next`] to pick which move of a
//! stance's string plays; the string position hashes beside the rest of the
//! fighter state.
//! Units: [`Stance`] and stance positions are plain `u8` move indices; no
//! ticks, no floats, nothing to convert.
//! Determinism: pure integer math over fixed arrays, no heap and no clock, so
//! the same operation sequence gives the same positions and the same
//! [`ComboCursor::digest`] on every PC. Stance order is fixed
//! ([`Stance::all`]) and every operation walks it the same way.
//! Errors: [`ComboError`] for a table whose counts are zero or past
//! [`MAX_STRING`]; the cursor itself is total and never panics.
//! Budget: every operation is `O(STANCE_COUNT)` (three) with zero heap; the
//! cursor and the table are `Copy` and fit in a register pair.
//!
//! Source: `web: Blade Symphony official guide + ModDB MotY posts 4 and 7,
//! read 2026-10-19 (ideas only)`. The whole steal is one cursor shared across
//! stances plus a count per stance: a combo is a path through a shared
//! automaton, so an AI authors it as a table and the player still gets depth.

/// Wire version of the shipped combo table.
pub const COMBO_VERSION_V1: u32 = 1;
/// Number of stances; the cursor carries one position per stance.
pub const STANCE_COUNT: usize = 3;
/// Longest string a table may ask for, in moves.
pub const MAX_STRING: u8 = 16;

/// The three sword stances. Fixes the order every table and cursor uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    /// Fast (reverse) stance: the shortest string.
    Fast,
    /// Balanced stance.
    Balanced,
    /// Heavy stance: the longest string.
    Heavy,
}

impl Stance {
    /// Canonical index, `0..STANCE_COUNT`; the cursor and tables index by it.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Fast => 0,
            Self::Balanced => 1,
            Self::Heavy => 2,
        }
    }

    /// Every stance in canonical order, for table walks and tests.
    #[must_use]
    pub const fn all() -> [Stance; STANCE_COUNT] {
        [Self::Fast, Self::Balanced, Self::Heavy]
    }

    /// Short lowercase name for tables, logs and UI.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Heavy => "heavy",
        }
    }
}

/// The last blow's outcome. A [`ComboStep`]'s optional conditional asks for
/// one of these: a blocked swing chains into a different follow-up, a clean
/// hit (or a dodge) takes the base, and a whiff resets the string (SW46).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboReq {
    /// The blow landed clean (damage or a block's chip).
    AnyDamage,
    /// The blow was blocked: the follow-up is the bash slot.
    Blocked,
    /// The blow missed everything: the string resets.
    Whiffed,
}

/// One step of a stance's string: the move that plays by default plus an
/// optional (move, outcome) that plays instead when the last blow's outcome
/// matches. Rows carry these; a v1 row has `conditional: None` everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComboStep {
    /// Move index that plays after a clean hit (the default).
    pub base: u8,
    /// `(move, outcome)`: the follow-up that plays instead when the last
    /// blow's outcome equals the required one.
    pub conditional: Option<(u8, ComboReq)>,
}

impl ComboStep {
    /// A step with no conditional: the same move whatever the last outcome.
    #[must_use]
    pub const fn plain(base: u8) -> Self {
        Self {
            base,
            conditional: None,
        }
    }

    /// The move this step plays after `last`: the conditional's move when its
    /// outcome matches, otherwise the base.
    #[must_use]
    pub const fn pick(self, last: ComboReq) -> u8 {
        match self.conditional {
            Some((follow_up, req)) => {
                if req.same(last) {
                    follow_up
                } else {
                    self.base
                }
            }
            None => self.base,
        }
    }
}

impl ComboReq {
    /// True when both name the same outcome (const compare, no `as` cast).
    #[must_use]
    pub const fn same(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::AnyDamage, Self::AnyDamage)
                | (Self::Blocked, Self::Blocked)
                | (Self::Whiffed, Self::Whiffed)
        )
    }
}

/// Stance order as a table: how many moves each stance's string holds, plus
/// one [`ComboStep`] per move.
///
/// The finisher is the last index of a string (position `count - 1`), so the
/// count is part of the design, not a buffer size. Blade Symphony's strings
/// run three to four moves; [`ComboTable::v1`] is one such row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComboTable {
    counts: [u8; STANCE_COUNT],
    steps: [[ComboStep; MAX_STRING as usize]; STANCE_COUNT],
}

impl ComboTable {
    /// The shipped V1 row: three fast moves, four balanced, three heavy, every
    /// step plain, so v1 strings and their digests are unchanged.
    #[must_use]
    pub const fn v1() -> Self {
        Self::from_counts([3, 4, 3])
    }

    /// A table from move counts whose steps are all plain `base = index`
    /// steps: the v1 shape, also used by v2 rows that add no conditional.
    #[must_use]
    pub const fn from_counts(counts: [u8; STANCE_COUNT]) -> Self {
        let mut steps = [[ComboStep::plain(0); MAX_STRING as usize]; STANCE_COUNT];
        let mut stance = 0;
        while stance < STANCE_COUNT {
            let mut index = 0;
            while index < MAX_STRING as usize {
                // Padding past the count stays base 0 and is never read.
                steps[stance][index] = ComboStep::plain(u8::try_from(index).unwrap_or(0));
                index += 1;
            }
            stance += 1;
        }
        Self { counts, steps }
    }

    /// This table with one step replaced: rows that add a conditional build
    /// theirs from [`ComboTable::v1`] through this so validation still reads
    /// the counts.
    #[must_use]
    pub const fn with_step(mut self, stance: Stance, position: usize, step: ComboStep) -> Self {
        let index = stance.index();
        if position < MAX_STRING as usize {
            self.steps[index][position] = step;
        }
        self
    }

    /// Move counts in [`Stance`] order.
    #[must_use]
    pub fn counts(&self) -> [u8; STANCE_COUNT] {
        self.counts
    }

    /// The step at `position` of this stance's string; an out-of-range
    /// position reads the plain move 0 rather than panicking.
    #[must_use]
    pub fn step(&self, stance: Stance, position: u8) -> ComboStep {
        let index = usize::from(position);
        if index < usize::from(self.counts[stance.index()]) {
            self.steps[stance.index()][index]
        } else {
            ComboStep::plain(0)
        }
    }

    /// Range-check the row: every count in `1..=MAX_STRING`.
    ///
    /// Deterministic: fixed stance order, first bad count wins.
    pub fn validate(&self) -> Result<(), ComboError> {
        let stances = Stance::all();
        let mut i = 0;
        while i < STANCE_COUNT {
            let count = self.counts[i];
            let stance = stances[i];
            if count == 0 {
                return Err(ComboError::ZeroCount(stance));
            }
            if count > MAX_STRING {
                return Err(ComboError::CountTooLarge(stance));
            }
            i += 1;
        }
        Ok(())
    }
}

/// One fighter's string cursor: one position per stance, shared by all of
/// them, so every attack moves all three strings at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ComboCursor {
    positions: [u8; STANCE_COUNT],
}

impl ComboCursor {
    /// A cursor at the start of every string.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Positions in [`Stance`] order.
    #[must_use]
    pub fn positions(&self) -> [u8; STANCE_COUNT] {
        self.positions
    }

    /// The move of this stance's string that plays now: the step's base, no
    /// outcome read.
    #[must_use]
    pub fn next(&self, stance: Stance) -> u8 {
        self.positions[stance.index()]
    }

    /// The move that plays now once the last blow's outcome is read: the step
    /// at this stance's position resolves through [`ComboStep::pick`], so a
    /// blocked swing chains into its conditional follow-up and a clean hit
    /// takes the base.
    #[must_use]
    pub fn next_for(&self, table: &ComboTable, stance: Stance, last: ComboReq) -> u8 {
        table
            .step(stance, self.positions[stance.index()])
            .pick(last)
    }

    /// One attack: a whiff resets every string; any other outcome advances
    /// every stance's string, wrapping each by its count.
    pub fn attack(&mut self, table: &ComboTable, last: ComboReq) {
        if matches!(last, ComboReq::Whiffed) {
            self.reset();
            return;
        }
        self.advance(table);
    }

    /// One dash: advances every string exactly as an attack does, so movement
    /// spends the same counter.
    pub fn dash(&mut self, table: &ComboTable) {
        self.advance(table);
    }

    /// One shuffle: step every string back, saturating at the start.
    pub fn shuffle(&mut self) {
        let mut i = 0;
        while i < STANCE_COUNT {
            self.positions[i] = self.positions[i].saturating_sub(1);
            i += 1;
        }
    }

    /// Back to the start of every string.
    pub fn reset(&mut self) {
        self.positions = [0; STANCE_COUNT];
    }

    /// FNV-1a 64 over the three positions only, in stance order.
    ///
    /// Shares `engine-core`'s FNV constants so the engine keeps one hash
    /// owner; the same positions give the same digest on every PC.
    #[must_use]
    pub const fn digest(&self) -> u64 {
        let mut hash = engine_core::hash::FNV64_OFFSET;
        let mut i = 0;
        while i < STANCE_COUNT {
            // One byte per position, widened without a cast.
            hash ^= u64::from_le_bytes([self.positions[i], 0, 0, 0, 0, 0, 0, 0]);
            hash = hash.wrapping_mul(engine_core::hash::FNV64_PRIME);
            i += 1;
        }
        hash
    }

    /// Shared `attack`/`dash` step: `+1` on all three, wrapping per count.
    ///
    /// Total even on an unvalidated table: a zero count wraps to zero and a
    /// position at the top of `u8` wraps to zero rather than overflowing.
    fn advance(&mut self, table: &ComboTable) {
        let mut i = 0;
        while i < STANCE_COUNT {
            let count = table.counts[i];
            let pos = self.positions[i];
            self.positions[i] = match pos.checked_add(1) {
                Some(next) if next < count => next,
                _ => 0,
            };
            i += 1;
        }
    }
}

/// Why a combo table was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboError {
    /// A stance string holds no moves.
    ZeroCount(Stance),
    /// A stance string is longer than [`MAX_STRING`].
    CountTooLarge(Stance),
    /// Unknown table version.
    VersionUnknown(u32),
}

impl core::fmt::Display for ComboError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroCount(stance) => write!(f, "combo string for {} has no moves", stance.name()),
            Self::CountTooLarge(stance) => {
                write!(
                    f,
                    "combo string for {} is past {MAX_STRING} moves",
                    stance.name()
                )
            }
            Self::VersionUnknown(version) => {
                write!(f, "combo table version {version} is not valid")
            }
        }
    }
}

impl std::error::Error for ComboError {}

/// The shipped combo table.
pub const COMBO_V1: ComboTable = ComboTable::v1();

/// Resolve a replay version tag to its static table: 1 gives V1, else unknown.
///
/// Determinism scope: pure lookup; the same tag gives the same table.
#[must_use]
pub fn combo_table_for_version(version: u32) -> Option<&'static ComboTable> {
    match version {
        COMBO_VERSION_V1 => Some(&COMBO_V1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(counts: [u8; STANCE_COUNT]) -> ComboTable {
        ComboTable { counts }
    }

    fn cursor(positions: [u8; STANCE_COUNT]) -> ComboCursor {
        ComboCursor { positions }
    }

    #[test]
    fn strings_are_three_to_four_moves() {
        // A Blade Symphony string is three or four moves; the last index is
        // the finisher, so the count is design data.
        let counts = COMBO_V1.counts();
        assert_eq!(counts, [3, 4, 3]);
        for count in counts {
            assert!(
                (3..=4).contains(&count),
                "count {count} is not a 3-4 move string"
            );
        }
        assert_eq!(COMBO_V1.validate(), Ok(()));
        assert_eq!(Stance::all().map(Stance::index), [0, 1, 2]);
        assert_eq!(Stance::Fast.name(), "fast");
        assert_eq!(Stance::Balanced.name(), "balanced");
        assert_eq!(Stance::Heavy.name(), "heavy");
        assert_eq!(COMBO_V1.counts().len(), STANCE_COUNT);
    }

    #[test]
    fn next_reads_the_position_for_its_stance() {
        let c = cursor([2, 3, 1]);
        assert_eq!(c.next(Stance::Fast), 2);
        assert_eq!(c.next(Stance::Balanced), 3);
        assert_eq!(c.next(Stance::Heavy), 1);
        assert_eq!(c.positions(), [2, 3, 1]);
    }

    #[test]
    fn attack_wraps_at_the_count_and_moves_all_three() {
        // Fast is at its finisher (2 of 3), balanced at its finisher (3 of 4),
        // heavy mid-string: one attack wraps the first two and steps the third.
        let mut c = cursor([2, 3, 2]);
        c.attack(&COMBO_V1, ComboReq::AnyDamage);
        assert_eq!(c.positions(), [0, 0, 0]);
        let mut d = cursor([2, 0, 1]);
        d.attack(&COMBO_V1, ComboReq::AnyDamage);
        assert_eq!(d.positions(), [0, 1, 2]);
    }

    #[test]
    fn dash_wraps_like_an_attack() {
        let mut c = cursor([2, 3, 2]);
        c.dash(&COMBO_V1);
        assert_eq!(c.positions(), [0, 0, 0]);
        let mut attack = cursor([2, 3, 2]);
        attack.attack(&COMBO_V1, ComboReq::AnyDamage);
        assert_eq!(c.positions(), attack.positions());
    }

    #[test]
    fn shuffle_saturates_at_zero() {
        let mut c = cursor([0, 1, 7]);
        c.shuffle();
        assert_eq!(c.positions(), [0, 0, 6]);
        c.shuffle();
        c.shuffle();
        assert_eq!(c.positions(), [0, 0, 4]);
        let mut at_zero = cursor([0, 0, 0]);
        for _ in 0..8 {
            at_zero.shuffle();
        }
        assert_eq!(at_zero.positions(), [0, 0, 0]);
    }

    #[test]
    fn reset_zeroes_every_stance() {
        let mut c = cursor([1, 2, 3]);
        c.reset();
        assert_eq!(c.positions(), [0, 0, 0]);
        assert_eq!(c, ComboCursor::new());
        assert_eq!(c, ComboCursor::default());
    }

    #[test]
    fn same_sequence_gives_same_positions_and_digest() {
        let mut a = ComboCursor::new();
        let mut b = ComboCursor::new();
        for step in 0..9 {
            if step % 3 == 0 {
                a.attack(&COMBO_V1, ComboReq::AnyDamage);
                b.attack(&COMBO_V1, ComboReq::AnyDamage);
            } else if step % 3 == 1 {
                a.dash(&COMBO_V1);
                b.dash(&COMBO_V1);
            } else {
                a.shuffle();
                b.shuffle();
            }
        }
        assert_eq!(a.positions(), b.positions());
        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.positions(), [0, 0, 0]);
    }

    #[test]
    fn digest_sees_all_three_positions() {
        let base = cursor([1, 2, 3]);
        let others = [cursor([2, 2, 3]), cursor([1, 3, 3]), cursor([1, 2, 4])];
        for other in others {
            assert_ne!(other.positions(), base.positions());
            assert_ne!(other.digest(), base.digest());
            assert_eq!(other.digest(), other.digest());
        }
        assert_eq!(base.digest(), cursor([1, 2, 3]).digest());
    }

    #[test]
    fn validate_rejects_zero_and_over_max() {
        assert_eq!(
            table([0, 3, 3]).validate(),
            Err(ComboError::ZeroCount(Stance::Fast))
        );
        assert_eq!(
            table([3, 0, 3]).validate(),
            Err(ComboError::ZeroCount(Stance::Balanced))
        );
        assert_eq!(
            table([3, 3, 0]).validate(),
            Err(ComboError::ZeroCount(Stance::Heavy))
        );
        assert_eq!(
            table([1, MAX_STRING + 1, 3]).validate(),
            Err(ComboError::CountTooLarge(Stance::Balanced))
        );
        assert_eq!(
            table([MAX_STRING, MAX_STRING, MAX_STRING]).validate(),
            Ok(())
        );
    }

    #[test]
    fn version_lookup() {
        assert_eq!(combo_table_for_version(COMBO_VERSION_V1), Some(&COMBO_V1));
        assert_eq!(combo_table_for_version(2), None);
        assert_eq!(combo_table_for_version(0), None);
        assert_eq!(COMBO_V1, ComboTable::v1());
    }

    #[test]
    fn errors_display() {
        assert_eq!(
            ComboError::ZeroCount(Stance::Heavy).to_string(),
            "combo string for heavy has no moves"
        );
        assert_eq!(
            ComboError::CountTooLarge(Stance::Fast).to_string(),
            "combo string for fast is past 16 moves"
        );
        assert_eq!(
            ComboError::VersionUnknown(7).to_string(),
            "combo table version 7 is not valid"
        );
    }
}
