//! wounds: a cut outlives the fight.
//!
//! Consumer: the legionary mode reads a [`Wounds`] to carry a man's injuries out of a
//! fight and through the camps between fights; the fight reports the [`Cut`] events the
//! combat resolver already raises, and a non-severing hit reports its part directly.
//! Data owner: `content/rome/wound.toml` prices four body parts at three severities,
//! one heal time each in camp days; this module maps a joint to its part, owns the
//! state that survives the fight, and heals one step per camp day.
//!
//! A cut at a part's root joint severs the part ([`JointId::Neck`] the head,
//! [`JointId::Waist`] and [`JointId::Chest`] the torso, a shoulder an arm, a hip a
//! leg) and the part never heals. A cut below the root (elbow, wrist, knee, ankle)
//! leaves the part attached and heals. [`JointId::Pelvis`] never takes a wound.
//!
//! Units: fixed ticks for the fight clock, camp days for healing, Q16 for the energy
//! and protection a [`Cut`] carries (`1.0 == 65536`). Determinism: integer math only,
//! no clock, no RNG and no float; wounds stay sorted by slot and part, so the same
//! cuts in any order give the same state and [`Wounds::hash`] proves it.
//! Never hashed by the arena tick: like [`crate::roster`], this is v2 campaign state,
//! and the golden `d57b5c8ae3096614` does not move. Errors: [`WoundError`] at every
//! boundary; a bad table, a bad cut or a bad payload fails loudly, never silently.
//! Budget: a cold path (a fight report or a camp opens it); one event is O(wounds)
//! over at most [`MAX_WOUNDS`], one camp day the same, and the shipped table is
//! parsed once.

use core::fmt;

use engine_combat::anatomy::JointId;
use engine_combat::cut::Cut;
use engine_core::text::is_identifier;
use engine_core::Fnv64;

/// Body parts the wound table prices.
pub const PART_COUNT: usize = 4;

/// Severities the wound table prices.
pub const SEVERITY_COUNT: usize = 3;

/// Slots a wound list tracks: the arena's participant bound (32).
pub const MAX_SLOTS: usize = 32;

/// One wound per slot and part at most.
pub const MAX_WOUNDS: usize = MAX_SLOTS * PART_COUNT;

/// Longest heal time a row may name, in camp days.
pub const MAX_HEAL_DAYS: u8 = 30;

/// Margin in Q16 (`2.0`) over protection that turns a light cut serious.
pub const SERIOUS_MARGIN_Q16: i64 = 131_072;

/// Version tag of the save payload [`Wounds::encode`] writes.
pub const WOUNDS_PAYLOAD_VERSION: u8 = 1;

/// First four bytes of the save payload.
pub const WOUNDS_PAYLOAD_MAGIC: [u8; 4] = *b"WND1";

/// Why a wound table, a cut or a save payload was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WoundError {
    /// A line is neither a section header nor a `key = value` pair.
    Syntax {
        /// The 1-based line that refused.
        line: usize,
        /// The line as written, trimmed.
        text: String,
    },
    /// A section appears twice.
    DuplicateSection {
        /// The 1-based line of the second header.
        line: usize,
        /// The repeated section key.
        key: String,
    },
    /// A section is not `[wound.<part>]`.
    UnknownSection {
        /// The 1-based line of the header.
        line: usize,
        /// The section key as written.
        key: String,
    },
    /// A section holds the same key twice.
    DuplicateKey {
        /// The 1-based line of the second key.
        line: usize,
        /// The section key.
        entry: String,
        /// The repeated key.
        key: String,
    },
    /// A section carried a number and no `ref` naming its source.
    MissingRef {
        /// The 1-based line of the section header.
        line: usize,
        /// The section key.
        entry: String,
    },
    /// A section holds a key the wound table does not read.
    UnknownKey {
        /// The section key.
        entry: String,
        /// The key it does not read.
        key: String,
    },
    /// A section holds no value for a key the wound table must read.
    MissingValue {
        /// The section key.
        entry: String,
        /// The key it lacks.
        key: String,
    },
    /// A value is not a whole number.
    NotAnInteger {
        /// The 1-based line that refused.
        line: usize,
        /// The line as written.
        text: String,
    },
    /// A heal time is outside `1..=MAX_HEAL_DAYS`.
    OutOfRange {
        /// The 1-based line that refused.
        line: usize,
        /// The section key.
        entry: String,
    },
    /// The table does not price every part.
    PartCount {
        /// The parts the table prices.
        found: usize,
    },
    /// A cut names a joint no body has.
    BadJoint {
        /// The joint wire value it names.
        joint: u8,
    },
    /// A cut names the pelvis, which never takes a wound.
    PelvisCut,
    /// A wound names a slot past the arena's bound.
    SlotOutOfRange {
        /// The slot it names.
        slot: u8,
    },
    /// A payload holds more wounds than a full grid.
    WoundCount {
        /// The wounds it holds.
        found: usize,
    },
    /// A payload does not start with [`WOUNDS_PAYLOAD_MAGIC`].
    BadMagic,
    /// A payload names a version this reader does not know.
    BadVersion {
        /// The version it names.
        version: u8,
    },
    /// A payload's length does not match its count.
    BadLength {
        /// The bytes found.
        found: usize,
        /// The bytes the header asks for.
        expected: usize,
    },
    /// A payload names a part no body has.
    BadPart {
        /// The part wire value it names.
        value: u8,
    },
    /// A payload names a severity no cut has.
    BadSeverity {
        /// The severity wire value it names.
        value: u8,
    },
    /// A payload's severed flag is neither zero nor one.
    BadSevered {
        /// The flag value it names.
        value: u8,
    },
    /// A payload holds the same slot and part twice, or a severed light wound.
    InconsistentWound {
        /// The slot it names.
        slot: u8,
        /// The part it names.
        part: Part,
    },
}

impl fmt::Display for WoundError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, text } => write!(out, "line {line}: not a table line `{text}`"),
            Self::DuplicateSection { line, key } => {
                write!(out, "line {line}: section `[{key}]` appears twice")
            }
            Self::UnknownSection { line, key } => {
                write!(out, "line {line}: `[{key}]` is not `[wound.<part>]`")
            }
            Self::DuplicateKey { line, entry, key } => {
                write!(out, "line {line}: `[{entry}]` holds `{key}` twice")
            }
            Self::MissingRef { line, entry } => {
                write!(
                    out,
                    "line {line}: `[{entry}]` carries a number and no `ref`"
                )
            }
            Self::UnknownKey { entry, key } => {
                write!(
                    out,
                    "`[{entry}]` holds `{key}`, which the wound table does not read"
                )
            }
            Self::MissingValue { entry, key } => write!(out, "`[{entry}]` gives no `{key}`"),
            Self::NotAnInteger { line, text } => {
                write!(out, "line {line}: `{text}` is not a whole number")
            }
            Self::OutOfRange { line, entry } => {
                write!(
                    out,
                    "line {line}: `[{entry}]` heals outside 1..={MAX_HEAL_DAYS} days"
                )
            }
            Self::PartCount { found } => {
                write!(out, "the table prices {found} parts, not {PART_COUNT}")
            }
            Self::BadJoint { joint } => write!(out, "joint {joint} is no body joint"),
            Self::PelvisCut => write!(out, "the pelvis never takes a wound"),
            Self::SlotOutOfRange { slot } => {
                write!(out, "slot {slot} is past the arena's {MAX_SLOTS}")
            }
            Self::WoundCount { found } => {
                write!(
                    out,
                    "the payload holds {found} wounds, at most {MAX_WOUNDS}"
                )
            }
            Self::BadMagic => write!(out, "the payload does not start with WND1"),
            Self::BadVersion { version } => write!(out, "payload version {version} is unknown"),
            Self::BadLength { found, expected } => {
                write!(out, "the payload holds {found} bytes, not {expected}")
            }
            Self::BadPart { value } => write!(out, "part {value} is no body part"),
            Self::BadSeverity { value } => write!(out, "severity {value} is no wound severity"),
            Self::BadSevered { value } => write!(out, "severed flag {value} is not 0 or 1"),
            Self::InconsistentWound { slot, part } => {
                write!(
                    out,
                    "slot {slot} part {} is repeated or severed while not critical",
                    part.key()
                )
            }
        }
    }
}

impl std::error::Error for WoundError {}

/// One body part the wound table prices.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Part {
    /// The head, cut through the neck.
    Head,
    /// Either arm, cut through a shoulder, elbow or wrist.
    Arm,
    /// The torso, cut through the waist or chest.
    Torso,
    /// Either leg, cut through a hip, knee or ankle.
    Leg,
}

impl Part {
    /// Every part, in table order.
    pub const ALL: [Self; PART_COUNT] = [Self::Head, Self::Arm, Self::Torso, Self::Leg];

    /// Wire and table value, `0..PART_COUNT`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Head => 0,
            Self::Arm => 1,
            Self::Torso => 2,
            Self::Leg => 3,
        }
    }

    /// Decode a wire value; unknown values are `None`, never a panic.
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Head),
            1 => Some(Self::Arm),
            2 => Some(Self::Torso),
            3 => Some(Self::Leg),
            _ => None,
        }
    }

    /// The name a `[wound.<part>]` section writes.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Arm => "arm",
            Self::Torso => "torso",
            Self::Leg => "leg",
        }
    }

    /// The part a section name names, or `None`.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.key() == key)
    }
}

/// How hard a cut landed: the table's three rows.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// A cut under the joint's protection that still won its roll.
    Light,
    /// A cut up to [`SERIOUS_MARGIN_Q16`] over protection.
    Serious,
    /// A cut past that margin, or any cut that severs the part.
    Critical,
}

impl Severity {
    /// Every severity, in table order.
    pub const ALL: [Self; SEVERITY_COUNT] = [Self::Light, Self::Serious, Self::Critical];

    /// Wire and table value, `0..SEVERITY_COUNT`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Light => 0,
            Self::Serious => 1,
            Self::Critical => 2,
        }
    }

    /// Decode a wire value; unknown values are `None`, never a panic.
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Light),
            1 => Some(Self::Serious),
            2 => Some(Self::Critical),
            _ => None,
        }
    }

    /// The key a `[wound.<part>]` section writes.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Serious => "serious",
            Self::Critical => "critical",
        }
    }
}

/// The severity of an attached cut: its margin over the joint's protection.
fn severity_of(energy_q16: u32, protection_q16: u32) -> Severity {
    let margin = i64::from(energy_q16) - i64::from(protection_q16);
    if margin < 0 {
        Severity::Light
    } else if margin < SERIOUS_MARGIN_Q16 {
        Severity::Serious
    } else {
        Severity::Critical
    }
}

/// The part a joint wounds and whether the cut severs it.
fn joint_body(joint: JointId) -> Result<(Part, bool), WoundError> {
    match joint {
        JointId::Pelvis => Err(WoundError::PelvisCut),
        JointId::Neck => Ok((Part::Head, true)),
        JointId::Waist | JointId::Chest => Ok((Part::Torso, true)),
        JointId::ShoulderL | JointId::ShoulderR => Ok((Part::Arm, true)),
        JointId::ElbowL | JointId::ElbowR | JointId::WristL | JointId::WristR => {
            Ok((Part::Arm, false))
        }
        JointId::HipL | JointId::HipR => Ok((Part::Leg, true)),
        JointId::KneeL | JointId::KneeR | JointId::AnkleL | JointId::AnkleR => {
            Ok((Part::Leg, false))
        }
    }
}

/// The wound table: the rows of `content/rome/wound.toml`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WoundTable {
    days: [[u8; SEVERITY_COUNT]; PART_COUNT],
}

impl WoundTable {
    /// The wound table shipped with the game.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/wound.toml");

    /// Read the shipped wound table.
    ///
    /// # Errors
    ///
    /// Same as [`WoundTable::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, WoundError> {
        Self::parse(Self::SHIPPED)
    }

    /// Read one wound table.
    ///
    /// # Errors
    ///
    /// Returns the first [`WoundError`] the text earns: a malformed line, a repeated
    /// key or section, a section that is not `[wound.<part>]`, a number with no
    /// `ref`, a missing or unknown key, a non-integer or out-of-range day, or a part
    /// count that is not [`PART_COUNT`].
    pub fn parse(text: &str) -> Result<Self, WoundError> {
        let rows = read_rows(text)?;
        let mut days = [[0u8; SEVERITY_COUNT]; PART_COUNT];
        let mut seen = [false; PART_COUNT];
        for row in &rows {
            let Some(name) = row.key.strip_prefix("wound.") else {
                return Err(WoundError::UnknownSection {
                    line: row.line,
                    key: row.key.to_string(),
                });
            };
            let Some(part) = Part::from_key(name) else {
                return Err(WoundError::UnknownSection {
                    line: row.line,
                    key: row.key.to_string(),
                });
            };
            seen[usize::from(part.as_u8())] = true;
            row.only(&["light", "serious", "critical"])?;
            for severity in Severity::ALL {
                let value = row.u32(severity.key())?;
                let day = u8::try_from(value).map_err(|_| WoundError::OutOfRange {
                    line: row.line,
                    entry: row.key.to_string(),
                })?;
                if day == 0 || day > MAX_HEAL_DAYS {
                    return Err(WoundError::OutOfRange {
                        line: row.line,
                        entry: row.key.to_string(),
                    });
                }
                days[usize::from(part.as_u8())][usize::from(severity.as_u8())] = day;
            }
        }
        let found = seen.iter().filter(|seen| **seen).count();
        if found != PART_COUNT {
            return Err(WoundError::PartCount { found });
        }
        Ok(Self { days })
    }

    /// Camp days a wound of this part and severity takes to heal.
    #[must_use]
    pub fn days(&self, part: Part, severity: Severity) -> u32 {
        u32::from(self.days[usize::from(part.as_u8())][usize::from(severity.as_u8())])
    }

    /// A digest of the loaded days: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_str("engine-game/wounds/table/1");
        for part in Part::ALL {
            for severity in Severity::ALL {
                hasher
                    .write_u8(self.days[usize::from(part.as_u8())][usize::from(severity.as_u8())]);
            }
        }
        hasher.finish()
    }
}

/// One wound a man carries out of a fight.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Wound {
    slot: u8,
    part: Part,
    severity: Severity,
    severed: bool,
    days_healed: u32,
}

impl Wound {
    /// The slot of the man who carries it.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }

    /// The body part it sits on.
    #[must_use]
    pub const fn part(&self) -> Part {
        self.part
    }

    /// How hard the cut that set it landed.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Whether the part came off; a severed part never heals.
    #[must_use]
    pub const fn severed(&self) -> bool {
        self.severed
    }

    /// Camp days healed so far, `0..` the table's days for the row.
    #[must_use]
    pub const fn days_healed(&self) -> u32 {
        self.days_healed
    }
}

/// The wounds of a fight, outliving it until the camp heals them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Wounds {
    table: WoundTable,
    rows: Vec<Wound>,
    ticks: u64,
    days: u32,
}

impl Wounds {
    /// An empty wound list over the shipped table.
    ///
    /// # Errors
    ///
    /// Same as [`WoundTable::shipped`].
    pub fn shipped() -> Result<Self, WoundError> {
        Ok(Self::new(WoundTable::shipped()?))
    }

    /// An empty wound list over `table`.
    #[must_use]
    pub fn new(table: WoundTable) -> Self {
        Self {
            table,
            rows: Vec::new(),
            ticks: 0,
            days: 0,
        }
    }

    /// Set the wound a [`Cut`] event leaves: its joint picks the part, and a cut at
    /// the part's root severs it and never heals.
    ///
    /// # Errors
    ///
    /// Refuses the pelvis, a joint wire value no body has, and a victim slot past
    /// [`MAX_SLOTS`].
    pub fn apply_cut(&mut self, cut: &Cut) -> Result<(), WoundError> {
        let joint = JointId::from_u8(cut.joint).ok_or(WoundError::BadJoint { joint: cut.joint })?;
        let (part, severed) = joint_body(joint)?;
        let severity = if severed {
            Severity::Critical
        } else {
            severity_of(cut.energy_q16, cut.protection_q16)
        };
        self.put(cut.victim, part, severity, severed)
    }

    /// Set an attached wound of `severity` on `part`, for a hit that did not sever.
    ///
    /// # Errors
    ///
    /// Refuses a slot past [`MAX_SLOTS`].
    pub fn set_wound(
        &mut self,
        slot: u8,
        part: Part,
        severity: Severity,
    ) -> Result<(), WoundError> {
        self.put(slot, part, severity, false)
    }

    /// One fixed tick of the fight clock: a wound never heals without a camp day.
    pub fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// One camp day: every attached wound heals one step, and one whose table days
    /// are spent leaves the list. A severed part stays.
    pub fn camp_day(&mut self) {
        self.days = self.days.saturating_add(1);
        let table = self.table;
        let mut index = 0;
        while index < self.rows.len() {
            let wound = &mut self.rows[index];
            if wound.severed {
                index += 1;
                continue;
            }
            if wound.days_healed + 1 >= table.days(wound.part, wound.severity) {
                self.rows.remove(index);
            } else {
                wound.days_healed += 1;
                index += 1;
            }
        }
    }

    /// Every wound, sorted by slot and part.
    #[must_use]
    pub fn wounds(&self) -> &[Wound] {
        &self.rows
    }

    /// The wound on `slot`'s `part`, when it carries one.
    #[must_use]
    pub fn wound(&self, slot: u8, part: Part) -> Option<&Wound> {
        self.index(slot, part).map(|index| &self.rows[index])
    }

    /// The wounds carried.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no wound was taken.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The table the list heals through.
    #[must_use]
    pub const fn table(&self) -> &WoundTable {
        &self.table
    }

    /// Fixed ticks survived with these wounds.
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Camp days served with these wounds.
    #[must_use]
    pub const fn days(&self) -> u32 {
        self.days
    }

    /// A digest of the whole state: the same cuts twice give the same number.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_str("engine-game/wounds/state/1");
        hasher.write_u64(self.table.hash());
        hasher.write_u64(self.ticks);
        hasher.write_u32(self.days);
        hasher.write_len32(self.rows.len());
        for wound in &self.rows {
            hasher.write_u8(wound.slot);
            hasher.write_u8(wound.part.as_u8());
            hasher.write_u8(wound.severity.as_u8());
            hasher.write_bool(wound.severed);
            hasher.write_u32(wound.days_healed);
        }
        hasher.finish()
    }

    /// The save payload a load feeds back to [`Wounds::decode`].
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.rows.len() * ROW_LEN);
        out.extend_from_slice(&WOUNDS_PAYLOAD_MAGIC);
        out.push(WOUNDS_PAYLOAD_VERSION);
        out.extend_from_slice(&self.ticks.to_le_bytes());
        out.extend_from_slice(&self.days.to_le_bytes());
        let count = u16::try_from(self.rows.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&count.to_le_bytes());
        for wound in &self.rows {
            out.push(wound.slot);
            out.push(wound.part.as_u8());
            out.push(wound.severity.as_u8());
            out.push(u8::from(wound.severed));
            out.extend_from_slice(&wound.days_healed.to_le_bytes());
        }
        out
    }

    /// Read a save payload back over `table`.
    ///
    /// # Errors
    ///
    /// Refuses a payload with the wrong magic, an unknown version, a length that
    /// does not match its count, a slot, part, severity or flag outside its range, a
    /// repeated slot and part, or a severed wound that is not critical.
    pub fn decode(table: WoundTable, bytes: &[u8]) -> Result<Self, WoundError> {
        if bytes.len() < HEADER_LEN {
            return Err(WoundError::BadLength {
                found: bytes.len(),
                expected: HEADER_LEN,
            });
        }
        if bytes[..4] != WOUNDS_PAYLOAD_MAGIC {
            return Err(WoundError::BadMagic);
        }
        let version = bytes[4];
        if version != WOUNDS_PAYLOAD_VERSION {
            return Err(WoundError::BadVersion { version });
        }
        let short = |found: usize| WoundError::BadLength {
            found,
            expected: HEADER_LEN,
        };
        let ticks = u64_at(bytes, 5).ok_or_else(|| short(bytes.len()))?;
        let days = u32_at(bytes, 13).ok_or_else(|| short(bytes.len()))?;
        let count = usize::from(u16_at(bytes, 17).ok_or_else(|| short(bytes.len()))?);
        if count > MAX_WOUNDS {
            return Err(WoundError::WoundCount { found: count });
        }
        let expected = HEADER_LEN + count * ROW_LEN;
        if bytes.len() != expected {
            return Err(WoundError::BadLength {
                found: bytes.len(),
                expected,
            });
        }
        let mut rows = Vec::with_capacity(count);
        for index in 0..count {
            let at = HEADER_LEN + index * ROW_LEN;
            let slot = bytes[at];
            if usize::from(slot) >= MAX_SLOTS {
                return Err(WoundError::SlotOutOfRange { slot });
            }
            let part = Part::from_u8(bytes[at + 1]).ok_or(WoundError::BadPart {
                value: bytes[at + 1],
            })?;
            let severity = Severity::from_u8(bytes[at + 2]).ok_or(WoundError::BadSeverity {
                value: bytes[at + 2],
            })?;
            let severed = match bytes[at + 3] {
                0 => false,
                1 => true,
                value => return Err(WoundError::BadSevered { value }),
            };
            if severed && severity != Severity::Critical {
                return Err(WoundError::InconsistentWound { slot, part });
            }
            if rows
                .iter()
                .any(|wound: &Wound| wound.slot == slot && wound.part == part)
            {
                return Err(WoundError::InconsistentWound { slot, part });
            }
            rows.push(Wound {
                slot,
                part,
                severity,
                severed,
                days_healed: u32_at(bytes, at + 4).ok_or_else(|| short(bytes.len()))?,
            });
        }
        rows.sort_by_key(|wound| (wound.slot, wound.part.as_u8()));
        Ok(Self {
            table,
            rows,
            ticks,
            days,
        })
    }

    fn index(&self, slot: u8, part: Part) -> Option<usize> {
        self.rows
            .binary_search_by_key(&(slot, part.as_u8()), |wound| {
                (wound.slot, wound.part.as_u8())
            })
            .ok()
    }

    /// Record one wound: a repeat on the same slot and part keeps the worse
    /// severity and the severs, and starts the heal count over.
    fn put(
        &mut self,
        slot: u8,
        part: Part,
        severity: Severity,
        severed: bool,
    ) -> Result<(), WoundError> {
        if usize::from(slot) >= MAX_SLOTS {
            return Err(WoundError::SlotOutOfRange { slot });
        }
        let severity = if severed {
            Severity::Critical
        } else {
            severity
        };
        if let Some(index) = self.index(slot, part) {
            let wound = &mut self.rows[index];
            wound.severed |= severed;
            wound.severity = wound.severity.max(severity);
            wound.days_healed = 0;
            return Ok(());
        }
        let at = self
            .rows
            .partition_point(|wound| (wound.slot, wound.part.as_u8()) < (slot, part.as_u8()));
        self.rows.insert(
            at,
            Wound {
                slot,
                part,
                severity,
                severed,
                days_healed: 0,
            },
        );
        Ok(())
    }
}

/// Bytes of a payload header: magic, version, ticks, camp days and wound count.
const HEADER_LEN: usize = 4 + 1 + 8 + 4 + 2;

/// Bytes of one wound row: slot, part, severity, severed flag and days healed.
const ROW_LEN: usize = 1 + 1 + 1 + 1 + 4;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// One value written in a table row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Value<'a> {
    /// A whole number as written.
    Int(u64),
    /// A quoted string without its quotes.
    Text(&'a str),
}

/// One section of the wound table: its key, its line and its values.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row<'a> {
    key: &'a str,
    line: usize,
    values: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Row<'a> {
    fn int(&self, key: &str) -> Option<u64> {
        let (_, value) = self.values.iter().find(|(name, _)| *name == key)?;
        match value {
            Value::Int(number) => Some(*number),
            Value::Text(_) => None,
        }
    }

    fn text(&self, key: &str) -> Option<&'a str> {
        for (name, value) in &self.values {
            if *name != key {
                continue;
            }
            if let Value::Text(text) = value {
                return Some(text);
            }
        }
        None
    }

    /// The whole number `key`, refused when absent or past `u32`.
    fn u32(&self, key: &str) -> Result<u32, WoundError> {
        let number = self.int(key).ok_or_else(|| WoundError::MissingValue {
            entry: self.key.to_string(),
            key: key.to_string(),
        })?;
        u32::try_from(number).map_err(|_| WoundError::OutOfRange {
            line: self.line,
            entry: self.key.to_string(),
        })
    }

    /// Refuse a key this row's owner does not read; `ref` is always allowed.
    fn only(&self, allowed: &[&str]) -> Result<(), WoundError> {
        for (key, _) in &self.values {
            if *key == "ref" || allowed.contains(key) {
                continue;
            }
            return Err(WoundError::UnknownKey {
                entry: self.key.to_string(),
                key: (*key).to_string(),
            });
        }
        Ok(())
    }

    fn sourced(&self) -> bool {
        self.text("ref").is_some_and(|text| !text.trim().is_empty())
    }
}

/// Read the table into rows, in file order, refusing an unsourced number.
fn read_rows(text: &str) -> Result<Vec<Row<'_>>, WoundError> {
    let mut rows: Vec<Row<'_>> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if let Some(key) = header(body, line)? {
            if rows.iter().any(|row| row.key == key) {
                return Err(WoundError::DuplicateSection {
                    line,
                    key: key.to_string(),
                });
            }
            rows.push(Row {
                key,
                line,
                values: Vec::new(),
            });
            continue;
        }
        let Some(row) = rows.last_mut() else {
            return Err(WoundError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        let Some((key, value)) = body.split_once('=') else {
            return Err(WoundError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        let key = key.trim();
        if !is_identifier(key) {
            return Err(WoundError::Syntax {
                line,
                text: body.to_string(),
            });
        }
        if row.values.iter().any(|(name, _)| *name == key) {
            return Err(WoundError::DuplicateKey {
                line,
                entry: row.key.to_string(),
                key: key.to_string(),
            });
        }
        row.values.push((key, value_of(value.trim(), line, body)?));
    }
    for row in &rows {
        if row
            .values
            .iter()
            .any(|(key, value)| *key != "ref" && matches!(value, Value::Int(_)) && !row.sourced())
        {
            return Err(WoundError::MissingRef {
                line: row.line,
                entry: row.key.to_string(),
            });
        }
    }
    Ok(rows)
}

/// The section key of a `[a.b]` header, or `None` when the line is not one.
fn header(body: &str, line: usize) -> Result<Option<&str>, WoundError> {
    let Some(rest) = body.strip_prefix('[') else {
        return Ok(None);
    };
    let Some(inner) = rest.strip_suffix(']') else {
        return Err(WoundError::Syntax {
            line,
            text: body.to_string(),
        });
    };
    let valid =
        !inner.is_empty() && !inner.contains(['[', ']']) && inner.split('.').all(is_identifier);
    if !valid {
        return Err(WoundError::Syntax {
            line,
            text: body.to_string(),
        });
    }
    Ok(Some(inner))
}

/// One `key = value` line's value: a whole number or a quoted string.
fn value_of<'a>(text: &'a str, line: usize, body: &str) -> Result<Value<'a>, WoundError> {
    if let Some(rest) = text.strip_prefix('"') {
        let Some(inner) = rest.strip_suffix('"') else {
            return Err(WoundError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        if inner.contains('"') || inner.contains('\\') {
            return Err(WoundError::Syntax {
                line,
                text: body.to_string(),
            });
        }
        return Ok(Value::Text(inner));
    }
    whole(text, line, body).map(Value::Int)
}

/// A whole number: digits only, at most nineteen of them.
fn whole(text: &str, line: usize, body: &str) -> Result<u64, WoundError> {
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WoundError::NotAnInteger {
            line,
            text: body.to_string(),
        });
    }
    text.parse().map_err(|_| WoundError::NotAnInteger {
        line,
        text: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> WoundTable {
        WoundTable::shipped().expect("the shipped wound table reads")
    }

    fn wounds() -> Wounds {
        Wounds::shipped().expect("the shipped wound table reads")
    }

    fn cut(victim: u8, joint: JointId, energy_q16: u32, protection_q16: u32) -> Cut {
        Cut {
            swing: 1,
            attacker: 0,
            victim,
            joint: joint.as_u8(),
            flags: 0,
            energy_q16,
            protection_q16,
        }
    }

    const MINI: &str = "\
[wound.head]
light = 2
serious = 5
critical = 10
ref = \"test\"
[wound.arm]
light = 1
serious = 3
critical = 7
ref = \"test\"
[wound.torso]
light = 2
serious = 5
critical = 9
ref = \"test\"
[wound.leg]
light = 1
serious = 4
critical = 8
ref = \"test\"
";

    #[test]
    fn shipped_table_prices_four_parts() {
        assert_eq!(MAX_SLOTS, usize::from(engine_input::MAX_PARTICIPANTS));
        let table = table();
        assert_eq!(table.days(Part::Head, Severity::Light), 2);
        assert_eq!(table.days(Part::Head, Severity::Critical), 10);
        assert_eq!(table.days(Part::Torso, Severity::Critical), 9);
        for part in Part::ALL {
            let light = table.days(part, Severity::Light);
            let serious = table.days(part, Severity::Serious);
            let critical = table.days(part, Severity::Critical);
            assert!(light >= 1, "{} light heals in no day", part.key());
            assert!(
                light <= serious && serious <= critical,
                "{} gets cheaper when it gets worse",
                part.key()
            );
            assert!(critical <= u32::from(MAX_HEAL_DAYS));
        }
        assert_eq!(
            table.hash(),
            WoundTable::shipped().expect("reads again").hash()
        );
        assert!(WoundTable::parse(MINI).is_ok());
    }

    #[test]
    fn a_cut_sets_one_wound() {
        let mut wounds = wounds();
        wounds
            .apply_cut(&cut(3, JointId::ElbowR, 131_072, 65_536))
            .expect("the cut lands");
        let wound = wounds.wound(3, Part::Arm).expect("the arm carries it");
        assert_eq!(wound.slot(), 3);
        assert_eq!(wound.severity(), Severity::Serious);
        assert_eq!(wound.days_healed(), 0);
        assert!(!wound.severed());
        assert_eq!(wounds.len(), 1);
    }

    #[test]
    fn cuts_in_any_order_give_one_state() {
        let first = cut(1, JointId::ElbowL, 131_072, 65_536);
        let second = cut(2, JointId::KneeR, 65_535, 65_536);
        let mut forward = wounds();
        forward.apply_cut(&first).expect("the first cut lands");
        forward.apply_cut(&second).expect("the second cut lands");
        let mut backward = wounds();
        backward.apply_cut(&second).expect("the cut lands");
        backward.apply_cut(&first).expect("the cut lands");
        assert_eq!(forward.hash(), backward.hash());
        assert_eq!(forward.encode(), backward.encode());
    }

    #[test]
    fn a_cut_persists_across_the_tick_loop() {
        let mut wounds = wounds();
        wounds
            .apply_cut(&cut(3, JointId::ElbowR, 65_535, 65_536))
            .expect("the cut lands");
        for _ in 0..10_000 {
            wounds.tick();
        }
        assert_eq!(wounds.ticks(), 10_000);
        let wound = wounds
            .wound(3, Part::Arm)
            .expect("the wound outlives the fight");
        assert_eq!(wound.severity(), Severity::Light);
        assert_eq!(wound.days_healed(), 0);
        assert_eq!(wounds.len(), 1);
    }

    #[test]
    fn a_cut_persists_across_save_and_load() {
        let mut wounds = wounds();
        wounds
            .apply_cut(&cut(3, JointId::ShoulderR, 1, 0))
            .expect("the cut lands");
        wounds
            .apply_cut(&cut(5, JointId::AnkleL, 65_535, 65_536))
            .expect("the cut lands");
        for _ in 0..7 {
            wounds.tick();
        }
        let payload = wounds.encode();
        let back = Wounds::decode(*wounds.table(), &payload).expect("the payload reads");
        assert_eq!(back.hash(), wounds.hash());
        assert_eq!(back.encode(), payload);
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn one_camp_day_heals_one_step() {
        let mut wounds = wounds();
        wounds
            .set_wound(2, Part::Head, Severity::Light)
            .expect("the head takes it");
        wounds.camp_day();
        assert_eq!(
            wounds
                .wound(2, Part::Head)
                .expect("one step left")
                .days_healed(),
            1
        );
        wounds.camp_day();
        assert!(
            wounds.wound(2, Part::Head).is_none(),
            "healed after two days"
        );
        assert_eq!(wounds.days(), 2);
        wounds
            .set_wound(6, Part::Arm, Severity::Light)
            .expect("the arm takes it");
        wounds.camp_day();
        assert!(
            wounds.wound(6, Part::Arm).is_none(),
            "an arm light is one day"
        );
    }

    #[test]
    fn a_severed_part_never_heals() {
        let mut wounds = wounds();
        wounds
            .set_wound(4, Part::Arm, Severity::Light)
            .expect("the arm takes a light wound");
        wounds
            .apply_cut(&cut(4, JointId::ShoulderL, 10, 0))
            .expect("the shoulder cut severs");
        let wound = wounds.wound(4, Part::Arm).expect("the part is off");
        assert!(wound.severed());
        assert_eq!(wound.severity(), Severity::Critical);
        for _ in 0..100 {
            wounds.camp_day();
        }
        let wound = wounds.wound(4, Part::Arm).expect("a severed part stays");
        assert!(wound.severed());
        assert_eq!(wound.days_healed(), 0);
    }

    #[test]
    fn a_second_cut_keeps_the_worse_severity() {
        let mut wounds = wounds();
        wounds
            .set_wound(6, Part::Arm, Severity::Serious)
            .expect("the arm takes it");
        wounds.camp_day();
        assert_eq!(
            wounds
                .wound(6, Part::Arm)
                .expect("the arm heals")
                .days_healed(),
            1
        );
        wounds
            .set_wound(6, Part::Arm, Severity::Light)
            .expect("the lighter cut lands");
        let wound = wounds.wound(6, Part::Arm).expect("the wound remains");
        assert_eq!(wound.severity(), Severity::Serious);
        assert_eq!(wound.days_healed(), 0, "a new cut starts the count over");
        assert_eq!(wounds.len(), 1);
    }

    #[test]
    fn the_pelvis_and_unknown_joints_are_refused() {
        let mut wounds = wounds();
        assert_eq!(
            wounds.apply_cut(&cut(1, JointId::Pelvis, 1, 0)),
            Err(WoundError::PelvisCut)
        );
        assert_eq!(
            wounds.apply_cut(&Cut {
                joint: 16,
                ..cut(1, JointId::Neck, 1, 0)
            }),
            Err(WoundError::BadJoint { joint: 16 })
        );
        let past = u8::try_from(MAX_SLOTS).expect("the slot bound fits a byte");
        assert_eq!(
            wounds.set_wound(past, Part::Leg, Severity::Light),
            Err(WoundError::SlotOutOfRange { slot: past })
        );
        assert!(wounds.is_empty());
    }

    #[test]
    fn a_row_with_no_ref_is_refused() {
        let text = MINI.replacen("ref = \"test\"", "", 1);
        assert_eq!(
            WoundTable::parse(&text),
            Err(WoundError::MissingRef {
                line: 1,
                entry: "wound.head".to_string()
            })
        );
    }

    #[test]
    fn a_missing_part_is_refused() {
        let text = &MINI[..MINI.find("[wound.leg]").expect("the leg section")];
        assert_eq!(
            WoundTable::parse(text),
            Err(WoundError::PartCount { found: 3 })
        );
    }

    #[test]
    fn an_unknown_section_and_a_zero_day_are_refused() {
        let unknown = MINI.replace("[wound.leg]", "[wound.tail]");
        assert_eq!(
            WoundTable::parse(&unknown),
            Err(WoundError::UnknownSection {
                line: 16,
                key: "wound.tail".to_string()
            })
        );
        let zero = MINI.replace("light = 2", "light = 0");
        assert_eq!(
            WoundTable::parse(&zero),
            Err(WoundError::OutOfRange {
                line: 1,
                entry: "wound.head".to_string()
            })
        );
    }

    #[test]
    fn a_bad_payload_is_refused() {
        let table = table();
        let payload = wounds().encode();
        assert!(matches!(
            Wounds::decode(table, &payload[..HEADER_LEN - 1]),
            Err(WoundError::BadLength { .. })
        ));
        let mut bad = payload.clone();
        bad[0] = b'X';
        assert_eq!(Wounds::decode(table, &bad), Err(WoundError::BadMagic));
        let mut bad = payload.clone();
        bad[4] = 9;
        assert_eq!(
            Wounds::decode(table, &bad),
            Err(WoundError::BadVersion { version: 9 })
        );
        let mut bad = payload;
        bad.pop();
        assert!(matches!(
            Wounds::decode(table, &bad),
            Err(WoundError::BadLength { .. })
        ));
    }
}
