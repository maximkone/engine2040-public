//! impact_seq: the moment of the hit as one authored sequence per impact class
//! and weapon (`content/design/impact.rows`).
//!
//! A blow lands and the presentation answers on every channel at once: the
//! clock stops, the screen shakes, the fight camera takes trauma and a kick
//! along the blow, a guard nudges the view, a killing blow slows time. Today those numbers sat in three places (the
//! feel flow's shipped rows, two look constants, a gain in the audio map) and
//! knew nothing of the weapon. This module reads them from one table so a
//! designer tunes the whole hit of a gladius against a club in one row each
//! (the Sparkle Lite idea: feedback as a data-authored sequence the hit fires)
//! and offers [`StopStack`], hit-stop as a composable stack that restores the
//! scale it found (the Wallop Lite idea).
//!
//! Consumer: [`crate::feel_view::FeelView`] (stop, shake, kick, slow-mo, bump)
//! and the fighter look (trauma). The blow's sound level already follows the
//! weapon through the damage the cue carries (`combat_cues::strike_gain`). Data owner:
//! `content/design/impact.rows`, embedded at build time. Units: ticks and frames
//! of the 60 Hz presentation clock, millimeters, permille. Determinism: pure
//! integer rows and a fixed-order stack; presentation only, the sim and its
//! hash never read it. Errors: [`ImpactError`] names the row and the key; the
//! shipped table is checked by a test and the window falls back to the
//! built-in copy of the old numbers if it ever fails to parse.

use std::sync::OnceLock;

/// The shipped rows, embedded at build time so a blow never reads a file.
pub const IMPACT_ROWS: &str = include_str!("../../../content/design/impact.rows");

/// Sim hold ticks from which a landed blow reads as heavy (combat v2: light 3,
/// heavy 6, parry 8).
pub const HEAVY_HOLD_TICKS: u32 = 6;
/// Stop frames a row that follows the sim uses when no live count is known
/// (the reference game's `stopFrame`).
pub const DEFAULT_STOP_FRAMES: u32 = 4;
/// Most stops a [`StopStack`] holds at once.
pub const STACK_DEPTH: usize = 4;
/// Longest a stack entry may run, frames: a stray row never freezes the game.
pub const STOP_CAP_FRAMES: u32 = 30;

/// What kind of blow landed. The row table has one default row per class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImpactClass {
    /// A light blow.
    Light,
    /// A heavy blow.
    Heavy,
    /// A parry answered the blow.
    Parry,
    /// A held guard soaked the blow.
    Block,
    /// The victim dropped to zero.
    Knockdown,
}

/// Every class, in row order.
pub const CLASSES: [ImpactClass; 5] = [
    ImpactClass::Light,
    ImpactClass::Heavy,
    ImpactClass::Parry,
    ImpactClass::Block,
    ImpactClass::Knockdown,
];

impl ImpactClass {
    /// The class name a row starts with.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Heavy => "heavy",
            Self::Parry => "parry",
            Self::Block => "block",
            Self::Knockdown => "knockdown",
        }
    }

    /// The class of a clean hit by the sim's hold: a hold of
    /// [`HEAVY_HOLD_TICKS`] or more is a heavy blow, anything else (including
    /// no count at all) is light.
    #[must_use]
    pub const fn of_hit(hold_ticks: u32) -> Self {
        if hold_ticks >= HEAVY_HOLD_TICKS {
            Self::Heavy
        } else {
            Self::Light
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        CLASSES.iter().copied().find(|class| class.key() == key)
    }
}

/// One authored sequence: every channel of the hit moment. Zero ticks or zero
/// millimeters silence a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// Frames the clock stops; zero follows the sim's hold.
    pub stop_frames: u32,
    /// Clock rate while stopped, permille of real time.
    pub stop_rate_permille: u32,
    /// Screen shake length, frames.
    pub shake_ticks: u32,
    /// Screen shake power, permille of the reference shake.
    pub shake_power_permille: u32,
    /// Fight-camera trauma the blow adds, permille.
    pub trauma_permille: u32,
    /// Camera kick along the blow, millimeters.
    pub kick_mm: u32,
    /// Frames the kick eases back over.
    pub kick_ticks: u32,
    /// Screen nudge along x, milli pixels.
    pub bump_milli: u32,
    /// Slow-motion length after the blow, frames; zero is none.
    pub slowmo_ticks: u32,
    /// Slow-motion speed, permille.
    pub slowmo_permille: u32,
}

impl Sequence {
    /// A blow that does nothing: no stop, no shake, no kick. The base the
    /// built-in rows build on.
    pub const QUIET: Self = Self {
        stop_frames: 0,
        stop_rate_permille: 1000,
        shake_ticks: 0,
        shake_power_permille: 0,
        trauma_permille: 0,
        kick_mm: 0,
        kick_ticks: 0,
        bump_milli: 0,
        slowmo_ticks: 0,
        slowmo_permille: 1000,
    };

    /// True when the row stops the clock at all.
    #[must_use]
    pub const fn stops(&self) -> bool {
        self.stop_rate_permille < 1000
    }

    /// Frames this row stops the clock for, given the sim's hold when known:
    /// the row's own number, else the sim's count, else the reference four.
    #[must_use]
    pub const fn stop_for(&self, sim_hold: Option<u32>) -> u32 {
        if self.stop_frames > 0 {
            self.stop_frames
        } else if let Some(hold) = sim_hold {
            hold
        } else {
            DEFAULT_STOP_FRAMES
        }
    }

    /// The camera kick as a distance in meters.
    #[must_use]
    pub fn kick_m(&self) -> f32 {
        f32::from(u16::try_from(self.kick_mm).unwrap_or(u16::MAX)) / 1000.0
    }
}

/// Why a row was refused: the line, the key, never a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImpactError {
    /// The table header `table impact version 1` is missing.
    Header,
    /// A row names no known class.
    Class {
        /// One-based line number.
        line: usize,
        /// The word the row starts with.
        word: String,
    },
    /// A row lacks a key.
    Missing {
        /// One-based line number.
        line: usize,
        /// The missing key.
        key: &'static str,
    },
    /// A key is not a whole number inside its range.
    Bad {
        /// One-based line number.
        line: usize,
        /// The offending key.
        key: &'static str,
        /// The text the row holds.
        value: String,
        /// The range the table takes, `min..=max`.
        range: (u32, u32),
    },
    /// A class has no `weapon=*` default row.
    NoDefault(&'static str),
}

impl core::fmt::Display for ImpactError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Header => {
                f.write_str("impact rows: the `table impact version 1` header is missing")
            }
            Self::Class { line, word } => write!(
                f,
                "impact rows line {line}: `{word}` is not an impact class"
            ),
            Self::Missing { line, key } => {
                write!(f, "impact rows line {line}: key `{key}` is missing")
            }
            Self::Bad {
                line,
                key,
                value,
                range,
            } => write!(
                f,
                "impact rows line {line}: `{key}={value}` is not a whole number in {}..={}",
                range.0, range.1
            ),
            Self::NoDefault(class) => {
                write!(f, "impact rows: class `{class}` has no `weapon=*` row")
            }
        }
    }
}

impl std::error::Error for ImpactError {}

/// The keys of a row and their ranges, in [`Sequence`] field order.
const KEYS: [(&str, (u32, u32)); 10] = [
    ("stop_frames", (0, STOP_CAP_FRAMES)),
    ("stop_rate_permille", (0, 1000)),
    ("shake_ticks", (0, 120)),
    ("shake_power_permille", (0, 4000)),
    ("trauma_permille", (0, 1000)),
    ("kick_mm", (0, 150)),
    ("kick_ticks", (0, 60)),
    ("bump_milli", (0, 5000)),
    ("slowmo_ticks", (0, 120)),
    ("slowmo_permille", (100, 1000)),
];

impl Sequence {
    /// Set field `index` of [`KEYS`].
    fn set(&mut self, index: usize, value: u32) {
        let slot = match index {
            0 => &mut self.stop_frames,
            1 => &mut self.stop_rate_permille,
            2 => &mut self.shake_ticks,
            3 => &mut self.shake_power_permille,
            4 => &mut self.trauma_permille,
            5 => &mut self.kick_mm,
            6 => &mut self.kick_ticks,
            7 => &mut self.bump_milli,
            8 => &mut self.slowmo_ticks,
            9 => &mut self.slowmo_permille,
            _ => return,
        };
        *slot = value;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    class: ImpactClass,
    /// Index into the table's weapon names; `None` is the class default.
    weapon: Option<u8>,
    sequence: Sequence,
}

/// The parsed rows: a default per class plus any weapon overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImpactTable {
    rows: Vec<Row>,
    /// Weapon ids that have override rows, in first-seen order.
    names: Vec<String>,
}

impl ImpactTable {
    /// Parse rows text. Header first, then one row per line. A class default
    /// (`weapon=*`) names every key; a weapon row names only the keys it
    /// changes and takes the rest from its class default.
    ///
    /// # Errors
    ///
    /// [`ImpactError`] for a missing header, an unknown class, a missing or
    /// out-of-range key, or a class without its `weapon=*` row.
    pub fn parse(text: &str) -> Result<Self, ImpactError> {
        let mut raw_rows: Vec<(usize, ImpactClass, &str, &str)> = Vec::new();
        let mut header = false;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            if !header {
                if raw.split_whitespace().collect::<Vec<_>>() != ["table", "impact", "version", "1"]
                {
                    return Err(ImpactError::Header);
                }
                header = true;
                continue;
            }
            let first = raw.split_whitespace().next().unwrap_or_default();
            let class = ImpactClass::from_key(first).ok_or_else(|| ImpactError::Class {
                line,
                word: first.to_string(),
            })?;
            let weapon = raw
                .split_whitespace()
                .find_map(|part| part.strip_prefix("weapon="))
                .ok_or(ImpactError::Missing {
                    line,
                    key: "weapon",
                })?;
            raw_rows.push((line, class, weapon, raw));
        }
        if !header {
            return Err(ImpactError::Header);
        }
        // Class defaults first: a weapon row inherits from them.
        let mut rows: Vec<Row> = Vec::new();
        for class in CLASSES {
            let Some(&(line, _, _, raw)) = raw_rows
                .iter()
                .find(|(_, row_class, weapon, _)| *row_class == class && *weapon == "*")
            else {
                return Err(ImpactError::NoDefault(class.key()));
            };
            rows.push(Row {
                class,
                weapon: None,
                sequence: Self::read_row(line, raw, None)?,
            });
        }
        let mut names: Vec<String> = Vec::new();
        for &(line, class, weapon, raw) in &raw_rows {
            if weapon == "*" {
                continue;
            }
            let base = rows
                .iter()
                .find(|row| row.class == class && row.weapon.is_none())
                .map(|row| row.sequence);
            let id = match names.iter().position(|name| name == weapon) {
                Some(id) => id,
                None => {
                    names.push(weapon.to_string());
                    names.len() - 1
                }
            };
            rows.push(Row {
                class,
                weapon: u8::try_from(id).ok(),
                sequence: Self::read_row(line, raw, base.as_ref())?,
            });
        }
        Ok(Self { rows, names })
    }

    /// One row's sequence: every key from the text, or, with a `base`, the
    /// keys the text names over the base's.
    fn read_row(line: usize, raw: &str, base: Option<&Sequence>) -> Result<Sequence, ImpactError> {
        let mut sequence = base.copied().unwrap_or(Sequence::QUIET);
        for (index, &(key, range)) in KEYS.iter().enumerate() {
            let text = raw
                .split_whitespace()
                .find_map(|part| part.strip_prefix(key)?.strip_prefix('='));
            match (text, base) {
                (Some(text), _) => {
                    let number = text
                        .parse::<u32>()
                        .ok()
                        .filter(|number| (range.0..=range.1).contains(number))
                        .ok_or_else(|| ImpactError::Bad {
                            line,
                            key,
                            value: text.to_string(),
                            range,
                        })?;
                    sequence.set(index, number);
                }
                (None, Some(_)) => {}
                (None, None) => return Err(ImpactError::Missing { line, key }),
            }
        }
        Ok(sequence)
    }

    /// The shipped table ([`IMPACT_ROWS`]), parsed once. If the embedded text
    /// ever failed to parse (a test pins that it does not) the built-in copy
    /// of the old numbers stands in, so a blow always has a sequence.
    #[must_use]
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<ImpactTable> = OnceLock::new();
        SHIPPED.get_or_init(|| Self::parse(IMPACT_ROWS).unwrap_or_else(|_| Self::builtin()))
    }

    /// The numbers the feel flow and the look carried before this table: the
    /// flow's shipped rows (a hit shakes and stops, a parry shakes less, a
    /// block nudges, a knockdown shakes hard and slows time) and the look's
    /// trauma. No kick, no weapon rows.
    #[must_use]
    pub fn builtin() -> Self {
        let base = Sequence {
            stop_rate_permille: 100,
            ..Sequence::QUIET
        };
        let row = |class, sequence| Row {
            class,
            weapon: None,
            sequence,
        };
        let hit = Sequence {
            shake_ticks: 12,
            shake_power_permille: 1000,
            trauma_permille: 400,
            ..base
        };
        Self {
            rows: vec![
                row(ImpactClass::Light, hit),
                row(ImpactClass::Heavy, hit),
                row(
                    ImpactClass::Parry,
                    Sequence {
                        shake_ticks: 8,
                        shake_power_permille: 700,
                        ..base
                    },
                ),
                row(
                    ImpactClass::Block,
                    Sequence {
                        stop_rate_permille: 1000,
                        bump_milli: 1500,
                        ..base
                    },
                ),
                row(
                    ImpactClass::Knockdown,
                    Sequence {
                        stop_rate_permille: 1000,
                        shake_ticks: 30,
                        shake_power_permille: 2000,
                        trauma_permille: 700,
                        slowmo_ticks: 45,
                        slowmo_permille: 300,
                        ..base
                    },
                ),
            ],
            names: Vec::new(),
        }
    }

    /// The table's handle for a weapon id of `weapons.rows`, or `None` when it
    /// has no override rows (the class defaults play). A cheap scan over a
    /// handful of names; the handle is for [`Self::sequence_of`].
    #[must_use]
    pub fn weapon_id(&self, weapon: &str) -> Option<u8> {
        self.names
            .iter()
            .position(|name| name == weapon)
            .and_then(|id| u8::try_from(id).ok())
    }

    /// The sequence of `class` for a weapon handle: the weapon's own row (its
    /// changed keys over the class default's), else the class default.
    #[must_use]
    pub fn sequence_of(&self, class: ImpactClass, weapon: Option<u8>) -> Sequence {
        let pick = |id: Option<u8>| {
            self.rows
                .iter()
                .find(|row| row.class == class && row.weapon == id)
                .map(|row| row.sequence)
        };
        pick(weapon)
            .or_else(|| pick(None))
            .unwrap_or_else(|| Self::fallback(class))
    }

    /// The sequence of `class` for `weapon` (a weapon id of `weapons.rows`);
    /// `None`, or a weapon with no row, is the class default.
    #[must_use]
    pub fn sequence(&self, class: ImpactClass, weapon: Option<&str>) -> Sequence {
        self.sequence_of(class, weapon.and_then(|name| self.weapon_id(name)))
    }

    /// A parsed table always holds every default; this is the unreachable
    /// answer for a hand-built one, the quiet built-in light hit.
    fn fallback(class: ImpactClass) -> Sequence {
        let builtin = Self::builtin();
        builtin
            .rows
            .iter()
            .find(|row| row.class == class)
            .or_else(|| builtin.rows.first())
            .map_or(Sequence::QUIET, |row| row.sequence)
    }

    /// Weapon ids the table has override rows for, in first-seen order.
    #[must_use]
    pub fn weapons(&self) -> &[String] {
        &self.names
    }
}

/// One stop in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stop {
    frames_left: u32,
    rate_permille: u32,
}

/// Hit-stop as a stack of independent stops, not one timer a second blow
/// resets. Overlapping stops last as long as the longest and run at the
/// lowest rate any live one asks for; when each ends the clock returns to the
/// rate under it, whatever slow motion was already running (Wallop Lite).
/// Fixed size, no heap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StopStack {
    stops: [Option<Stop>; STACK_DEPTH],
}

impl StopStack {
    /// An empty stack: the clock runs at the rate under it.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stops: [None; STACK_DEPTH],
        }
    }

    /// Push a stop of `frames` at `rate_permille`. Zero frames or a rate of a
    /// whole real second stops nothing. A stop already in flight that lasts at
    /// least as long and runs at least as low absorbs it; a full stack drops
    /// its weakest entry (shortest left) for a stronger newcomer.
    pub fn push(&mut self, frames: u32, rate_permille: u32) {
        if frames == 0 || rate_permille >= 1000 {
            return;
        }
        let new = Stop {
            frames_left: frames.min(STOP_CAP_FRAMES),
            rate_permille,
        };
        if self
            .stops
            .iter()
            .flatten()
            .any(|s| s.frames_left >= new.frames_left && s.rate_permille <= new.rate_permille)
        {
            return;
        }
        // Entries the newcomer covers (no longer, no lower) are redundant.
        for slot in &mut self.stops {
            if slot.is_some_and(|s| {
                s.frames_left <= new.frames_left && s.rate_permille >= new.rate_permille
            }) {
                *slot = None;
            }
        }
        if let Some(free) = self.stops.iter_mut().find(|s| s.is_none()) {
            *free = Some(new);
            return;
        }
        if let Some(weakest) = self
            .stops
            .iter_mut()
            .filter(|s| s.is_some())
            .min_by_key(|s| s.map_or(u32::MAX, |s| s.frames_left))
        {
            if weakest.is_some_and(|s| s.frames_left < new.frames_left) {
                *weakest = Some(new);
            }
        }
    }

    /// The rate the stack holds the clock at this frame, permille: the lowest
    /// live rate, or 1000 with none. Does not advance.
    #[must_use]
    pub fn rate_permille(&self) -> u32 {
        self.stops
            .iter()
            .flatten()
            .map(|s| s.rate_permille)
            .min()
            .unwrap_or(1000)
    }

    /// Frames until the stack is quiet: the longest live stop.
    #[must_use]
    pub fn frames_left(&self) -> u32 {
        self.stops
            .iter()
            .flatten()
            .map(|s| s.frames_left)
            .max()
            .unwrap_or(0)
    }

    /// Age every stop one frame and drop the finished ones.
    pub fn advance(&mut self) {
        for slot in &mut self.stops {
            if let Some(stop) = slot {
                stop.frames_left = stop.frames_left.saturating_sub(1);
                if stop.frames_left == 0 {
                    *slot = None;
                }
            }
        }
    }

    /// True while any stop is live.
    #[must_use]
    pub fn active(&self) -> bool {
        self.stops.iter().any(Option::is_some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped weapons table's ids and masses, for the weight ordering.
    fn weapons_by_mass() -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = engine_arena::heft::WEAPONS_ROWS
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#') && line.contains("mass_g="))
            .filter_map(|line| {
                let id = line.split_whitespace().next()?.to_string();
                let mass = line
                    .split_whitespace()
                    .find_map(|part| part.strip_prefix("mass_g="))?
                    .parse()
                    .ok()?;
                Some((id, mass))
            })
            .collect();
        out.sort_by_key(|(_, mass)| *mass);
        out
    }

    #[test]
    fn the_shipped_rows_parse_and_every_class_has_its_default() {
        let table = ImpactTable::parse(IMPACT_ROWS).expect("the shipped rows");
        for class in CLASSES {
            let seq = table.sequence(class, None);
            assert!(seq.stop_rate_permille <= 1000, "{class:?}");
        }
        assert_eq!(
            ImpactTable::shipped(),
            &table,
            "the window reads the shipped rows, not the fallback"
        );
    }

    #[test]
    fn the_defaults_keep_the_numbers_the_flow_and_the_look_carried() {
        // The table reproduces the feel flow's shipped rows and the look's
        // trauma for every class's default: a row edit moves them, nothing else.
        let table = ImpactTable::shipped();
        let old = ImpactTable::builtin();
        for class in [
            ImpactClass::Light,
            ImpactClass::Parry,
            ImpactClass::Block,
            ImpactClass::Knockdown,
        ] {
            let (new, was) = (table.sequence(class, None), old.sequence(class, None));
            assert_eq!(new.stops(), was.stops(), "{class:?} stops");
            assert_eq!(
                (new.shake_ticks, new.shake_power_permille, new.bump_milli),
                (was.shake_ticks, was.shake_power_permille, was.bump_milli),
                "{class:?} shake and bump"
            );
            assert_eq!(
                (new.slowmo_ticks, new.slowmo_permille),
                (was.slowmo_ticks, was.slowmo_permille),
                "{class:?} slow motion"
            );
            // The look added trauma for a hit and a knockdown only.
            if matches!(class, ImpactClass::Light | ImpactClass::Knockdown) {
                assert_eq!(new.trauma_permille, was.trauma_permille, "{class:?} trauma");
            }
        }
    }

    #[test]
    fn a_heavier_weapon_hits_harder_on_every_channel() {
        // Like the weapon rows (startup, recovery, stagger never fall with
        // mass), the hit moment never softens as the weapon gets heavier.
        let table = ImpactTable::shipped();
        let by_mass = weapons_by_mass();
        assert!(by_mass.len() >= 4, "the shipped weapons: {by_mass:?}");
        let mut last: Option<Sequence> = None;
        for (id, _) in &by_mass {
            let seq = table.sequence(ImpactClass::Light, Some(id));
            if let Some(prev) = last {
                assert!(
                    seq.shake_power_permille >= prev.shake_power_permille,
                    "{id} shake"
                );
                assert!(seq.trauma_permille >= prev.trauma_permille, "{id} trauma");
                assert!(seq.kick_mm >= prev.kick_mm, "{id} kick");
            }
            last = Some(seq);
        }
        // The gladius has no override: it hits like the class default.
        assert_eq!(
            table.sequence(ImpactClass::Light, Some("gladius")),
            table.sequence(ImpactClass::Light, None)
        );
        let (dagger, club) = (
            table.sequence(ImpactClass::Light, Some("pugio")),
            table.sequence(ImpactClass::Light, Some("clava")),
        );
        assert!(
            club.kick_mm > dagger.kick_mm
                && club.shake_power_permille > dagger.shake_power_permille
        );
        // The look reads trauma from the class default: a weapon row inherits it.
        assert_eq!(club.trauma_permille, dagger.trauma_permille);
    }

    #[test]
    fn heavy_blows_and_knockdowns_outweigh_light_ones() {
        let table = ImpactTable::shipped();
        let light = table.sequence(ImpactClass::Light, None);
        let heavy = table.sequence(ImpactClass::Heavy, None);
        let down = table.sequence(ImpactClass::Knockdown, None);
        assert!(heavy.stop_rate_permille <= light.stop_rate_permille);
        assert!(heavy.kick_mm > light.kick_mm && heavy.trauma_permille > light.trauma_permille);
        assert!(down.kick_mm > heavy.kick_mm && down.slowmo_ticks > 0);
    }

    #[test]
    fn bad_rows_are_refused_with_the_line_and_the_key() {
        assert_eq!(
            ImpactTable::parse("light weapon=*"),
            Err(ImpactError::Header)
        );
        let bad_class = "table impact version 1\nsword weapon=*";
        assert!(matches!(
            ImpactTable::parse(bad_class),
            Err(ImpactError::Class { line: 2, .. })
        ));
        let row = IMPACT_ROWS
            .lines()
            .find(|line| line.starts_with("light     weapon=*"))
            .expect("the light default");
        // A class default names every key; a weapon row may name fewer.
        let lacking = format!("table impact version 1\n{}", row.replace("kick_mm=20", ""));
        assert_eq!(
            ImpactTable::parse(&lacking),
            Err(ImpactError::Missing {
                line: 2,
                key: "kick_mm"
            })
        );
        let wild = format!(
            "table impact version 1\n{}",
            row.replace("kick_mm=20", "kick_mm=9999")
        );
        assert!(matches!(
            ImpactTable::parse(&wild),
            Err(ImpactError::Bad { key: "kick_mm", .. })
        ));
        // One class alone is not a table: every class needs its default.
        let only_light = format!("table impact version 1\n{row}");
        assert_eq!(
            ImpactTable::parse(&only_light),
            Err(ImpactError::NoDefault("heavy"))
        );
        // A weapon row's bad key is refused too, with its own line.
        let bad_weapon = format!("{IMPACT_ROWS}\nlight weapon=x kick_mm=nine");
        assert!(matches!(
            ImpactTable::parse(&bad_weapon),
            Err(ImpactError::Bad { key: "kick_mm", .. })
        ));
    }

    #[test]
    fn a_weapon_row_changes_only_the_keys_it_names() {
        let table = ImpactTable::shipped();
        assert!(
            table.weapon_id("gladius").is_none(),
            "the kit weapon plays the defaults"
        );
        let id = table.weapon_id("clava").expect("a club row");
        let (club, plain) = (
            table.sequence_of(ImpactClass::Light, Some(id)),
            table.sequence_of(ImpactClass::Light, None),
        );
        assert_ne!(club.kick_mm, plain.kick_mm);
        assert_eq!(
            club.stop_rate_permille, plain.stop_rate_permille,
            "inherited"
        );
        assert_eq!(club.trauma_permille, plain.trauma_permille, "inherited");
        // A weapon with a light row only still plays the defaults for the other classes.
        assert_eq!(
            table.sequence_of(ImpactClass::Knockdown, Some(id)),
            table.sequence_of(ImpactClass::Knockdown, None)
        );
        let names: Vec<&str> = table.weapons().iter().map(String::as_str).collect();
        assert_eq!(names, ["pugio", "spatha", "clava", "pilum"]);
    }

    #[test]
    fn the_class_of_a_hit_follows_the_sims_hold() {
        assert_eq!(ImpactClass::of_hit(0), ImpactClass::Light);
        assert_eq!(ImpactClass::of_hit(3), ImpactClass::Light);
        assert_eq!(ImpactClass::of_hit(4), ImpactClass::Light);
        assert_eq!(ImpactClass::of_hit(6), ImpactClass::Heavy);
        assert_eq!(ImpactClass::of_hit(8), ImpactClass::Heavy);
        let seq = ImpactTable::shipped().sequence(ImpactClass::Light, None);
        assert_eq!(seq.stop_for(None), DEFAULT_STOP_FRAMES);
        assert_eq!(seq.stop_for(Some(3)), 3);
        let fixed = Sequence {
            stop_frames: 5,
            ..seq
        };
        assert_eq!(fixed.stop_for(Some(3)), 5, "an authored count wins");
    }

    fn run(stack: &mut StopStack, frames: u32) -> Vec<u32> {
        (0..frames)
            .map(|_| {
                let rate = stack.rate_permille();
                stack.advance();
                rate
            })
            .collect()
    }

    #[test]
    fn a_lone_stop_holds_its_frames_then_the_clock_runs_free() {
        let mut stack = StopStack::new();
        stack.push(3, 100);
        assert_eq!(run(&mut stack, 5), [100, 100, 100, 1000, 1000]);
        assert!(!stack.active());
    }

    #[test]
    fn overlapping_stops_last_the_longest_and_run_the_lowest() {
        // A parry's 8 at a tenth, then a light blow's 3 at a fifth lands inside
        // it: the clock never speeds up for the weaker one, and a stronger
        // second stop deepens the rate without ending the first early.
        let mut stack = StopStack::new();
        stack.push(8, 100);
        let first = run(&mut stack, 2);
        stack.push(3, 200);
        let rest = run(&mut stack, 8);
        assert_eq!(first, [100, 100]);
        assert_eq!(rest, [100, 100, 100, 100, 100, 100, 1000, 1000]);
        // A later, lower, shorter stop dips the rate and the longer one's rate
        // returns after it.
        let mut stack = StopStack::new();
        stack.push(6, 300);
        run(&mut stack, 1);
        stack.push(2, 50);
        assert_eq!(run(&mut stack, 6), [50, 50, 300, 300, 300, 1000]);
    }

    #[test]
    fn a_second_blow_extends_the_freeze_instead_of_restarting_it() {
        // The old single timer restarted at the full count on every blow.
        let mut stack = StopStack::new();
        stack.push(4, 100);
        run(&mut stack, 3);
        stack.push(4, 100);
        // One frame was left; the new blow runs four: five frames in all would
        // be a restart-plus-leftover, four is the longest of the two.
        assert_eq!(stack.frames_left(), 4);
        assert_eq!(run(&mut stack, 5), [100, 100, 100, 100, 1000]);
    }

    #[test]
    fn the_stack_is_bounded_and_ignores_stops_that_stop_nothing() {
        let mut stack = StopStack::new();
        stack.push(0, 100);
        stack.push(5, 1000);
        assert!(
            !stack.active(),
            "zero frames and a full-speed rate stop nothing"
        );
        // Four stops that cover nobody and are covered by nobody fill it.
        for (frames, rate) in [(10, 100), (8, 90), (6, 80), (4, 70)] {
            stack.push(frames, rate);
        }
        assert_eq!(stack.stops.iter().flatten().count(), STACK_DEPTH);
        assert_eq!(stack.rate_permille(), 70);
        // A fifth that fits the chain takes the weakest entry's place.
        stack.push(7, 85);
        assert_eq!(stack.stops.iter().flatten().count(), STACK_DEPTH);
        assert_eq!(stack.rate_permille(), 80, "the 4-frame entry made room");
        assert_eq!(stack.frames_left(), 10, "the longest survives a full stack");
        // One shorter than every entry is dropped.
        stack.push(1, 75);
        assert_eq!(stack.rate_permille(), 80);
        stack.push(500, 100);
        assert_eq!(
            stack.frames_left(),
            STOP_CAP_FRAMES,
            "a stray row is capped"
        );
    }
}
