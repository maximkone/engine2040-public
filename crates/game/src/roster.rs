//! roster: the contubernium has names.
//!
//! Consumer: the legionary mode's camp and roster view on rules v2 reads a
//! [`Roster`] to draw the seven men beside the player (slot 0). Data owner:
//! `content/rome/contubernium.toml` names the seven men and their traits;
//! this module owns the state each man carries (fights, kills, wounds, rank)
//! and his diary, one entry per fight.
//!
//! The shape is OpenXcom's `Soldier` and `SoldierDiary` (read 2026-10-23, GPL
//! game code: ideas only, no line copied): a soldier carries a name, a rank
//! and mission, kill and wound counts, and a diary keeps one record per
//! mission. Rank words are the career ranks of [`crate::ladder`], and glory
//! follows its published numbers, so a man who fights and kills earns the
//! rank the ladder names.
//!
//! Units: counts of fights, kills and wounds, glory points and fixed ticks.
//! Determinism: integer math only, no clock, no RNG, no float, and the men
//! stay in content order, so the same fight reports fold to the same state
//! twice. Never hashed: nothing here enters the arena tick or the proof
//! scene; the roster is a v2 camp path only. Errors: [`RosterError`] at every
//! boundary, and an unknown man id refuses by name. Budget: a cold path (a
//! camp opens on it); a fold is O(men) plus one line of diary text, and the
//! shipped file is parsed once.

use core::fmt;

use engine_core::text::is_identifier;
use engine_core::Fnv64;

use crate::ladder::{Ladder, GLORY_PER_WIN, GLORY_RANKS};

/// Men in the contubernium: the seven comrades beside the player in slot 0
/// (`docs/plan/rome.md` section 0, decision 5).
pub const MEN: usize = 7;

/// Largest roster the reader builds; the shipped file holds [`MEN`].
pub const MAX_MEN: usize = 32;

/// Payload version [`Roster::to_text`] writes and [`Roster::parse`] reads.
pub const ROSTER_VERSION: u8 = 1;

/// Why a roster file, a state payload or a fight report was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RosterError {
    /// A line is neither a section header nor a `key = "text"` pair.
    Syntax {
        /// 1-based line number.
        line: usize,
        /// The line as written.
        text: String,
    },
    /// A value that must be a quoted string is not one.
    NotQuoted {
        /// 1-based line number.
        line: usize,
        /// The key whose value refused.
        key: String,
    },
    /// A state value is not a whole number that fits its field.
    NotAnInteger {
        /// 1-based line number.
        line: usize,
        /// The line as written.
        text: String,
    },
    /// A state number does not fit the field it fills.
    Overflow {
        /// 1-based line number.
        line: usize,
        /// The key whose number overflowed.
        key: String,
    },
    /// A section key is neither `[man.<id>]` nor a repeated man.
    DuplicateMan {
        /// 1-based line number.
        line: usize,
        /// The repeated man id.
        id: String,
    },
    /// A section appears twice.
    DuplicateSection {
        /// 1-based line number.
        line: usize,
        /// The repeated section key.
        key: String,
    },
    /// A man section holds the same key twice.
    DuplicateKey {
        /// The man id.
        id: String,
        /// The repeated key.
        key: String,
    },
    /// A man section has no such key.
    MissingKey {
        /// The man id.
        id: String,
        /// The missing key.
        key: String,
    },
    /// A man section holds a key the roster does not read.
    UnknownKey {
        /// The man id.
        id: String,
        /// The unknown key.
        key: String,
    },
    /// A trait is empty or written twice for one man.
    BadTrait {
        /// The man id.
        id: String,
        /// The trait as written.
        trait_name: String,
    },
    /// The rules file names no man at all.
    NoMen,
    /// The rules file names more than [`MAX_MEN`] men.
    TooManyMen {
        /// The men the file names.
        men: usize,
    },
    /// A state payload omits a man the contubernium holds.
    MissingMan {
        /// The man id that is absent.
        id: String,
    },
    /// A fight report or a state row names a man the roster does not hold.
    UnknownMan {
        /// The man id as written.
        id: String,
    },
    /// A state row carries a line count that is not the man's fight count.
    DiaryMismatch {
        /// The man id.
        id: String,
        /// The fights the counters give.
        fights: u32,
        /// The diary lines the payload carries.
        lines: usize,
    },
    /// A state payload is written by a newer roster version.
    FutureVersion {
        /// The version as written.
        version: u8,
    },
}

impl fmt::Display for RosterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, text } => {
                write!(f, "line {line}: not a `key = \"text\"` line: {text}")
            }
            Self::NotQuoted { line, key } => {
                write!(f, "line {line}: `{key}` is not a quoted string")
            }
            Self::NotAnInteger { line, text } => {
                write!(f, "line {line}: not a whole number: {text}")
            }
            Self::Overflow { line, key } => {
                write!(f, "line {line}: `{key}` does not fit its number")
            }
            Self::DuplicateMan { line, id } => {
                write!(f, "line {line}: the roster writes the man `{id}` twice")
            }
            Self::DuplicateSection { line, key } => {
                write!(f, "line {line}: section `[{key}]` appears twice")
            }
            Self::DuplicateKey { id, key } => {
                write!(f, "the man `{id}` writes `{key}` twice")
            }
            Self::MissingKey { id, key } => write!(f, "the man `{id}` has no `{key}`"),
            Self::UnknownKey { id, key } => {
                write!(f, "the man `{id}` has an unknown key `{key}`")
            }
            Self::BadTrait { id, trait_name } => {
                write!(f, "the man `{id}` has a bad trait `{trait_name}`")
            }
            Self::NoMen => write!(f, "the contubernium names no man"),
            Self::TooManyMen { men } => {
                write!(f, "the contubernium names {men} men, over {MAX_MEN}")
            }
            Self::MissingMan { id } => write!(f, "the roster has no row for the man `{id}`"),
            Self::UnknownMan { id } => write!(f, "the roster holds no man `{id}`"),
            Self::DiaryMismatch { id, fights, lines } => write!(
                f,
                "the man `{id}` has {fights} fights but {lines} diary lines"
            ),
            Self::FutureVersion { version } => {
                write!(
                    f,
                    "the roster payload version {version} is newer than {ROSTER_VERSION}"
                )
            }
        }
    }
}

impl std::error::Error for RosterError {}

/// One named man of the contubernium: the content seat, read-only.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Man {
    id: String,
    name: String,
    traits: Vec<String>,
}

impl Man {
    /// The state key; never shown to the player.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name the roster view shows.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The traits, in the order the file writes them.
    #[must_use]
    pub fn traits(&self) -> &[String] {
        &self.traits
    }
}

/// The seven named men of the contubernium, in slot order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Contubernium {
    men: Vec<Man>,
}

impl Contubernium {
    /// The rules file shipped with the game.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/contubernium.toml");

    /// Read the shipped rules file.
    ///
    /// # Errors
    ///
    /// Returns the first [`RosterError`] the file earns: a malformed line, a
    /// repeated section or key, an unquoted name, a bad trait, or a count
    /// outside `1..=MAX_MEN`.
    pub fn shipped() -> Result<Self, RosterError> {
        Self::parse(Self::SHIPPED)
    }

    /// Read a rules file.
    ///
    /// # Errors
    ///
    /// Returns the first [`RosterError`] the text earns, as
    /// [`Contubernium::shipped`] does for the shipped file.
    pub fn parse(text: &str) -> Result<Self, RosterError> {
        let drafts = read_drafts(text)?;
        let mut men = Vec::with_capacity(drafts.len());
        for draft in drafts {
            men.push(draft.into_man()?);
        }
        if men.is_empty() {
            return Err(RosterError::NoMen);
        }
        if men.len() > MAX_MEN {
            return Err(RosterError::TooManyMen { men: men.len() });
        }
        Ok(Self { men })
    }

    /// The men the file names, in slot order.
    #[must_use]
    pub fn men(&self) -> &[Man] {
        &self.men
    }

    /// The men the file names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.men.len()
    }

    /// True when the file named no man; a parsed file never is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.men.is_empty()
    }

    /// The man under `id`, when the file names him.
    #[must_use]
    pub fn man(&self, id: &str) -> Option<&Man> {
        self.men.iter().find(|man| man.id == id)
    }

    /// The names the roster view shows, in slot order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.men.iter().map(Man::name).collect()
    }

    /// A digest of the file: the same file read twice gives the same digest.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"roster.men");
        for man in &self.men {
            hasher.write_str(man.id.as_str());
            hasher.write_str(man.name.as_str());
            hasher.write_len(man.traits.len());
            for trait_name in &man.traits {
                hasher.write_str(trait_name.as_str());
            }
        }
        hasher.finish()
    }
}

/// One man section being read: its id and the two values it must give.
#[derive(Clone, Debug)]
struct Draft {
    id: String,
    name: Option<String>,
    traits: Option<String>,
}

impl Draft {
    /// The man this section names: both values present, the traits split and
    /// checked.
    fn into_man(self) -> Result<Man, RosterError> {
        let name = self.name.ok_or_else(|| RosterError::MissingKey {
            id: self.id.clone(),
            key: "name".to_string(),
        })?;
        let traits = self.traits.ok_or_else(|| RosterError::MissingKey {
            id: self.id.clone(),
            key: "traits".to_string(),
        })?;
        let mut list: Vec<String> = Vec::new();
        for trait_name in traits.split(',') {
            let trait_name = trait_name.trim();
            let repeated = list.iter().any(|held| held == trait_name);
            if trait_name.is_empty() || repeated || !trait_is_plain(trait_name) {
                return Err(RosterError::BadTrait {
                    id: self.id,
                    trait_name: trait_name.to_string(),
                });
            }
            list.push(trait_name.to_string());
        }
        Ok(Man {
            id: self.id,
            name,
            traits: list,
        })
    }
}

/// Read the man sections of a rules file, in slot order.
fn read_drafts(text: &str) -> Result<Vec<Draft>, RosterError> {
    let mut drafts: Vec<Draft> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if let Some(key) = body
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let id = key
                .strip_prefix("man.")
                .filter(|id| is_identifier(id))
                .ok_or_else(|| syntax(line, body))?;
            if drafts.iter().any(|draft| draft.id == id) {
                return Err(RosterError::DuplicateMan {
                    line,
                    id: id.to_string(),
                });
            }
            drafts.push(Draft {
                id: id.to_string(),
                name: None,
                traits: None,
            });
            continue;
        }
        let Some(draft) = drafts.last_mut() else {
            return Err(syntax(line, body));
        };
        let Some((key, value)) = body.split_once('=') else {
            return Err(syntax(line, body));
        };
        let key = key.trim();
        if key != "name" && key != "traits" {
            return Err(RosterError::UnknownKey {
                id: draft.id.clone(),
                key: key.to_string(),
            });
        }
        let value = quoted(value.trim(), line, key)?.to_string();
        let slot = if key == "name" {
            &mut draft.name
        } else {
            &mut draft.traits
        };
        if slot.replace(value).is_some() {
            return Err(RosterError::DuplicateKey {
                id: draft.id.clone(),
                key: key.to_string(),
            });
        }
    }
    Ok(drafts)
}

/// A refused rules line, carrying the line as written.
fn syntax(line: usize, body: &str) -> RosterError {
    RosterError::Syntax {
        line,
        text: body.to_string(),
    }
}

/// A trait is one or two lowercase words, nothing else.
fn trait_is_plain(trait_name: &str) -> bool {
    trait_name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte == b' ')
        && !trait_name.starts_with(' ')
        && !trait_name.ends_with(' ')
}

/// One quoted string value without its quotes; an empty or escaped one refuses.
fn quoted<'a>(text: &'a str, line: usize, key: &str) -> Result<&'a str, RosterError> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .filter(|inner| !inner.is_empty() && !inner.contains(['"', '\\']));
    inner.ok_or_else(|| RosterError::NotQuoted {
        line,
        key: key.to_string(),
    })
}

/// One man's career state: his counters, his glory rank and his diary.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ManState {
    id: String,
    wins: u32,
    losses: u32,
    kills: u32,
    wounds: u32,
    glory: u32,
    diary: Vec<DiaryEntry>,
}

impl ManState {
    /// The state key; the content seat maps it to a name and traits.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The fights the man has fought, won or lost.
    #[must_use]
    pub fn fights(&self) -> u32 {
        self.wins.saturating_add(self.losses)
    }

    /// The fights the man won.
    #[must_use]
    pub fn wins(&self) -> u32 {
        self.wins
    }

    /// The fights the man lost.
    #[must_use]
    pub fn losses(&self) -> u32 {
        self.losses
    }

    /// The enemies the man has killed.
    #[must_use]
    pub fn kills(&self) -> u32 {
        self.kills
    }

    /// The wounds the man has taken.
    #[must_use]
    pub fn wounds(&self) -> u32 {
        self.wounds
    }

    /// The glory the man has earned; the rank floor reads it.
    #[must_use]
    pub fn glory(&self) -> u32 {
        self.glory
    }

    /// The career rank index, from the [`crate::ladder`] glory floors.
    #[must_use]
    pub fn rank(&self) -> u8 {
        rank_of_glory(self.glory)
    }

    /// The career rank word, from the [`crate::ladder`] rank table.
    #[must_use]
    pub fn rank_name(&self) -> &'static str {
        Ladder::rank_name(self.rank())
    }

    /// The diary, one entry per fight, in fight order.
    #[must_use]
    pub fn diary(&self) -> &[DiaryEntry] {
        &self.diary
    }
}

/// One diary entry: the facts of one fight, and its line as text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DiaryEntry {
    fight: u32,
    kills: u16,
    wounds: u16,
    won: bool,
    tick: u32,
}

impl DiaryEntry {
    /// The fight number on this man's roll, counting from one.
    #[must_use]
    pub fn fight(&self) -> u32 {
        self.fight
    }

    /// The kills the man made in this fight.
    #[must_use]
    pub fn kills(&self) -> u16 {
        self.kills
    }

    /// The wounds the man took in this fight.
    #[must_use]
    pub fn wounds(&self) -> u16 {
        self.wounds
    }

    /// Whether the man's side won this fight.
    #[must_use]
    pub fn won(&self) -> bool {
        self.won
    }

    /// The fixed tick the fight was folded at.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// The one diary line the roster view shows for this fight.
    #[must_use]
    pub fn line(&self) -> String {
        let outcome = if self.won { "won" } else { "lost" };
        format!(
            "Fight {} at tick {}: {outcome}, kills {}, wounds {}.",
            self.fight, self.tick, self.kills, self.wounds
        )
    }
}

/// One fight's result for one man: who fought, what he did and when.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FightReport<'a> {
    /// The man who fought, by state key.
    pub id: &'a str,
    /// The enemies he killed in the fight.
    pub kills: u16,
    /// The wounds he took in the fight.
    pub wounds: u16,
    /// Whether his side won.
    pub won: bool,
    /// The fixed tick the fight ended on.
    pub tick: u32,
}

/// The roster state: the seven named men with their counters and diaries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Roster {
    group: Contubernium,
    men: Vec<ManState>,
}

impl Roster {
    /// A fresh roster: every man of `group` named, no fights yet.
    #[must_use]
    pub fn new(group: &Contubernium) -> Self {
        let men = group
            .men()
            .iter()
            .map(|man| ManState {
                id: man.id.clone(),
                wins: 0,
                losses: 0,
                kills: 0,
                wounds: 0,
                glory: 0,
                diary: Vec::new(),
            })
            .collect();
        Self {
            group: group.clone(),
            men,
        }
    }

    /// The contubernium the roster names.
    #[must_use]
    pub fn group(&self) -> &Contubernium {
        &self.group
    }

    /// The men the roster holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.men.len()
    }

    /// True when the roster holds no man; a built roster never is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.men.is_empty()
    }

    /// The men the roster holds, in slot order.
    #[must_use]
    pub fn men(&self) -> &[ManState] {
        &self.men
    }

    /// The names the roster view shows, in slot order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.group.names()
    }

    /// The man under `id`, when the roster holds him.
    #[must_use]
    pub fn man(&self, id: &str) -> Option<&ManState> {
        self.men.iter().find(|man| man.id == id)
    }

    /// Fold one fight into the man who fought it: the counters, the glory
    /// behind his rank, and one diary line.
    ///
    /// # Errors
    ///
    /// Returns [`RosterError::UnknownMan`] naming `report.id` when the roster
    /// holds no such man; the state is left as it was.
    pub fn after_fight(&mut self, report: FightReport<'_>) -> Result<(), RosterError> {
        let man = self
            .men
            .iter_mut()
            .find(|man| man.id == report.id)
            .ok_or_else(|| RosterError::UnknownMan {
                id: report.id.to_string(),
            })?;
        if report.won {
            man.wins = man.wins.saturating_add(1);
        } else {
            man.losses = man.losses.saturating_add(1);
        }
        man.kills = man.kills.saturating_add(u32::from(report.kills));
        man.wounds = man.wounds.saturating_add(u32::from(report.wounds));
        let won_glory = if report.won { GLORY_PER_WIN } else { 0 };
        man.glory = man
            .glory
            .saturating_add(won_glory)
            .saturating_add(u32::from(report.kills).saturating_mul(GLORY_PER_WIN));
        man.diary.push(DiaryEntry {
            fight: man.fights(),
            kills: report.kills,
            wounds: report.wounds,
            won: report.won,
            tick: report.tick,
        });
        Ok(())
    }

    /// The state as text: one version line, then one row per man in slot
    /// order and one row per diary line after its man.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = format!("roster {ROSTER_VERSION}\n");
        for man in &self.men {
            out.push_str(&format!(
                "man {} wins={} losses={} kills={} wounds={} glory={}\n",
                man.id, man.wins, man.losses, man.kills, man.wounds, man.glory
            ));
            for entry in &man.diary {
                out.push_str(&format!(
                    "diary fight={} kills={} wounds={} won={} tick={}\n",
                    entry.fight,
                    entry.kills,
                    entry.wounds,
                    u8::from(entry.won),
                    entry.tick
                ));
            }
        }
        out
    }

    /// Read state text against the contubernium that names the men.
    ///
    /// # Errors
    ///
    /// Returns the first [`RosterError`] the text earns: a bad line or number,
    /// an unknown man, a repeated man, a missing man, or a diary whose line
    /// count is not the man's fight count.
    pub fn parse(text: &str, group: &Contubernium) -> Result<Self, RosterError> {
        let mut men: Vec<ManState> = Vec::new();
        let mut version: Option<u8> = None;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = raw.trim();
            if body.is_empty() {
                continue;
            }
            let (tag, rest) = body.split_once(' ').ok_or_else(|| syntax(line, body))?;
            match tag {
                "roster" => {
                    version = Some(read_version(
                        rest,
                        line,
                        body,
                        version.is_some() || !men.is_empty(),
                    )?);
                }
                "man" => {
                    if version.is_none() {
                        return Err(syntax(line, body));
                    }
                    men.push(read_state_row(rest, line, body, group, &men)?);
                }
                "diary" => read_diary_row(rest, line, body, &mut men)?,
                _ => return Err(syntax(line, body)),
            }
        }
        if version.is_none() {
            return Err(RosterError::Syntax {
                line: 1,
                text: String::new(),
            });
        }
        check_state(&men)?;
        let men = order_men(men, group)?;
        Ok(Self {
            group: group.clone(),
            men,
        })
    }
}

/// The version line: one `u8` no newer than [`ROSTER_VERSION`], first line only.
fn read_version(rest: &str, line: usize, body: &str, repeated: bool) -> Result<u8, RosterError> {
    if repeated {
        return Err(syntax(line, body));
    }
    let written = whole(rest, line, body)?;
    let found = u8::try_from(written).map_err(|_| RosterError::Overflow {
        line,
        key: "roster".to_string(),
    })?;
    if found > ROSTER_VERSION {
        return Err(RosterError::FutureVersion { version: found });
    }
    Ok(found)
}

/// One `man` row: the men must be named by the contubernium and appear once.
fn read_state_row(
    rest: &str,
    line: usize,
    body: &str,
    group: &Contubernium,
    men: &[ManState],
) -> Result<ManState, RosterError> {
    let (id, fields) = rest.split_once(' ').ok_or_else(|| syntax(line, body))?;
    group
        .man(id)
        .ok_or_else(|| RosterError::UnknownMan { id: id.to_string() })?;
    if men.iter().any(|man| man.id == id) {
        return Err(RosterError::DuplicateMan {
            line,
            id: id.to_string(),
        });
    }
    let pairs = pairs(fields, line, body)?;
    for (key, _) in &pairs {
        if !is_man_key(key) {
            return Err(RosterError::UnknownKey {
                id: id.to_string(),
                key: (*key).to_string(),
            });
        }
    }
    Ok(ManState {
        id: id.to_string(),
        wins: count(&pairs, "wins", line, body)?,
        losses: count(&pairs, "losses", line, body)?,
        kills: count(&pairs, "kills", line, body)?,
        wounds: count(&pairs, "wounds", line, body)?,
        glory: count(&pairs, "glory", line, body)?,
        diary: Vec::new(),
    })
}

/// One `diary` row: it belongs to the man row just read.
fn read_diary_row(
    rest: &str,
    line: usize,
    body: &str,
    men: &mut [ManState],
) -> Result<(), RosterError> {
    let Some(man) = men.last_mut() else {
        return Err(syntax(line, body));
    };
    let pairs = pairs(rest, line, body)?;
    for (key, _) in &pairs {
        if !is_diary_key(key) {
            return Err(RosterError::UnknownKey {
                id: man.id.clone(),
                key: (*key).to_string(),
            });
        }
    }
    let won = match whole(field(&pairs, "won", line, body)?, line, body)? {
        0 => false,
        1 => true,
        _ => return Err(syntax(line, body)),
    };
    man.diary.push(DiaryEntry {
        fight: count(&pairs, "fight", line, body)?,
        kills: diary_count(&pairs, "kills", line, body)?,
        wounds: diary_count(&pairs, "wounds", line, body)?,
        won,
        tick: count(&pairs, "tick", line, body)?,
    });
    Ok(())
}

/// Every man keeps exactly one diary line per fight.
fn check_state(men: &[ManState]) -> Result<(), RosterError> {
    for man in men {
        let lines = man.diary.len();
        let fights = usize::try_from(man.fights()).unwrap_or(usize::MAX);
        if fights != lines {
            return Err(RosterError::DiaryMismatch {
                id: man.id.clone(),
                fights: man.fights(),
                lines,
            });
        }
    }
    Ok(())
}

/// The state rows in content order, refusing a man the contubernium lacks.
fn order_men(mut men: Vec<ManState>, group: &Contubernium) -> Result<Vec<ManState>, RosterError> {
    let mut ordered = Vec::with_capacity(men.len());
    for named in group.men() {
        let index = men
            .iter()
            .position(|man| man.id == named.id)
            .ok_or_else(|| RosterError::MissingMan {
                id: named.id.clone(),
            })?;
        ordered.push(men.remove(index));
    }
    Ok(ordered)
}

/// The rank index of a glory score: the highest [`GLORY_RANKS`] floor reached.
fn rank_of_glory(glory: u32) -> u8 {
    let Some(index) = GLORY_RANKS.iter().rposition(|floor| glory >= *floor) else {
        return 0;
    };
    u8::try_from(index).unwrap_or(0)
}

/// The keys a state row for one man carries.
fn is_man_key(key: &str) -> bool {
    matches!(key, "wins" | "losses" | "kills" | "wounds" | "glory")
}

/// The keys one diary row carries.
fn is_diary_key(key: &str) -> bool {
    matches!(key, "fight" | "kills" | "wounds" | "won" | "tick")
}

/// One whole number, as a state row writes it.
fn whole(text: &str, line: usize, body: &str) -> Result<u64, RosterError> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    match text.parse::<u64>().ok().filter(|_| digits) {
        Some(value) => Ok(value),
        None => Err(RosterError::NotAnInteger {
            line,
            text: body.to_string(),
        }),
    }
}

/// The `key=value` pairs of one state row, in file order.
fn pairs<'a>(
    rest: &'a str,
    line: usize,
    body: &str,
) -> Result<Vec<(&'a str, &'a str)>, RosterError> {
    let mut out = Vec::new();
    for word in rest.split_whitespace() {
        let Some((key, value)) = word.split_once('=') else {
            return Err(RosterError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        out.push((key, value));
    }
    Ok(out)
}

/// The value under `key`, or a refusal naming the row.
fn field<'a>(
    pairs: &[(&'a str, &'a str)],
    key: &str,
    line: usize,
    body: &str,
) -> Result<&'a str, RosterError> {
    pairs
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| *value)
        .ok_or_else(|| RosterError::Syntax {
            line,
            text: body.to_string(),
        })
}

/// One `u32` number of a state row: a counter, a fight number or a tick.
fn count(pairs: &[(&str, &str)], key: &str, line: usize, body: &str) -> Result<u32, RosterError> {
    let value = field(pairs, key, line, body)?;
    u32::try_from(whole(value, line, body)?).map_err(|_| RosterError::Overflow {
        line,
        key: key.to_string(),
    })
}

/// One `u16` field of a diary row.
fn diary_count(
    pairs: &[(&str, &str)],
    key: &str,
    line: usize,
    body: &str,
) -> Result<u16, RosterError> {
    let value = field(pairs, key, line, body)?;
    u16::try_from(whole(value, line, body)?).map_err(|_| RosterError::Overflow {
        line,
        key: key.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped contubernium: seven named men.
    fn shipped() -> Contubernium {
        Contubernium::shipped().expect("the shipped contubernium reads")
    }

    /// One fight report for one man.
    fn fight(id: &str, kills: u16, wounds: u16, won: bool, tick: u32) -> FightReport<'_> {
        FightReport {
            id,
            kills,
            wounds,
            won,
            tick,
        }
    }

    #[test]
    fn seven_named_men_stand_in_the_shipped_file() {
        let group = shipped();
        assert_eq!(group.len(), MEN);
        let names = group.names();
        assert_eq!(names.len(), MEN);
        for name in &names {
            assert!(!name.trim().is_empty(), "every slot carries a name");
        }
        for (index, name) in names.iter().enumerate() {
            assert!(
                !names[index + 1..].contains(name),
                "the name {name} is written once"
            );
        }
        for man in group.men() {
            assert_eq!(
                man.traits().len(),
                2,
                "the man {} holds two traits",
                man.id()
            );
            for trait_name in man.traits() {
                assert!(
                    trait_is_plain(trait_name),
                    "`{trait_name}` is a plain trait"
                );
            }
        }
        assert_eq!(
            shipped().digest(),
            group.digest(),
            "two loads hash the same"
        );
    }

    #[test]
    fn one_fight_names_the_man_and_writes_one_diary_line() {
        let group = shipped();
        let mut roster = Roster::new(&group);
        assert_eq!(roster.names().len(), MEN);
        roster
            .after_fight(fight("vitus", 2, 1, true, 600))
            .expect("the man fights");
        assert_eq!(roster.names().len(), MEN, "seven names after one fight");
        for (state, name) in roster.men().iter().zip(group.names()) {
            assert_eq!(group.man(state.id()).map(Man::name), Some(name));
        }
        let vitus = roster.man("vitus").expect("the man is in the roster");
        assert_eq!(vitus.fights(), 1);
        assert_eq!(vitus.kills(), 2);
        assert_eq!(vitus.wounds(), 1);
        assert_eq!(vitus.diary().len(), 1, "one diary line per fight");
        let line = vitus.diary()[0].line();
        assert!(
            line.contains("kills 2"),
            "the line carries the kills: {line}"
        );
        assert!(
            line.contains("wounds 1"),
            "the line carries the wounds: {line}"
        );
        assert!(line.contains("won"), "the line carries the outcome: {line}");
        for other in roster.men().iter().filter(|man| man.id() != "vitus") {
            assert_eq!(other.fights(), 0, "only the man who fought is charged");
            assert!(other.diary().is_empty());
        }
    }

    #[test]
    fn kills_earn_the_ladder_rank() {
        let mut roster = Roster::new(&shipped());
        let vitus = roster.man("vitus").expect("the man is in the roster");
        assert_eq!(vitus.rank(), 0);
        assert_eq!(vitus.rank_name(), "Tiro");
        roster
            .after_fight(fight("vitus", 3, 0, true, 600))
            .expect("the man fights");
        let vitus = roster.man("vitus").expect("the man is in the roster");
        assert_eq!(vitus.kills(), 3);
        assert_eq!(vitus.glory(), 3 * GLORY_PER_WIN + GLORY_PER_WIN);
        assert_eq!(vitus.rank(), 1);
        assert_eq!(vitus.rank_name(), Ladder::rank_name(1));
        assert_eq!(vitus.rank_name(), "Miles");
    }

    #[test]
    fn the_roster_round_trips() {
        let group = shipped();
        let mut roster = Roster::new(&group);
        roster
            .after_fight(fight("gaius", 1, 0, true, 120))
            .expect("the man fights");
        roster
            .after_fight(fight("gaius", 0, 2, false, 240))
            .expect("the man fights again");
        roster
            .after_fight(fight("marcus", 2, 1, true, 600))
            .expect("another man fights");
        let text = roster.to_text();
        let loaded = Roster::parse(&text, &group).expect("the state reads back");
        assert_eq!(loaded, roster, "serialise, load, equal");
        assert_eq!(loaded.to_text(), text, "the text is stable");
        let again = Roster::parse(&text, &group).expect("the state reads back twice");
        assert_eq!(again, roster);
    }

    #[test]
    fn the_same_fights_fold_twice_equal() {
        let group = shipped();
        let reports = [
            fight("titus", 2, 0, true, 60),
            fight("aulus", 0, 1, false, 120),
            fight("titus", 0, 3, false, 180),
        ];
        let mut first = Roster::new(&group);
        let mut second = Roster::new(&group);
        for report in reports {
            first.after_fight(report).expect("the fight folds");
        }
        for report in reports {
            second.after_fight(report).expect("the fight folds");
        }
        assert_eq!(first, second);
        assert_eq!(first.to_text(), second.to_text());
    }

    #[test]
    fn an_unknown_man_id_refuses_by_name() {
        let group = shipped();
        let mut roster = Roster::new(&group);
        let error = roster
            .after_fight(fight("nobody", 1, 0, true, 60))
            .expect_err("an unknown man refuses");
        assert_eq!(
            error,
            RosterError::UnknownMan {
                id: "nobody".to_string()
            }
        );
        assert!(
            error.to_string().contains("nobody"),
            "the refusal names the man: {error}"
        );
        assert_eq!(
            roster,
            Roster::new(&group),
            "a refused fight leaves the state as it was"
        );

        let state = format!(
            "roster {ROSTER_VERSION}\nman nobody wins=0 losses=0 kills=0 wounds=0 glory=0\n"
        );
        let error = Roster::parse(&state, &group).expect_err("an unknown man refuses");
        assert!(
            error.to_string().contains("nobody"),
            "the state load names the man too: {error}"
        );
    }

    #[test]
    fn a_state_row_without_its_diary_line_refuses() {
        let group = shipped();
        let text = format!(
            "roster {ROSTER_VERSION}\nman vitus wins=1 losses=0 kills=1 wounds=0 glory=200\n"
        );
        let error = Roster::parse(&text, &group).expect_err("one line per fight");
        assert!(
            matches!(
                error,
                RosterError::DiaryMismatch {
                    fights: 1,
                    lines: 0,
                    ..
                }
            ),
            "the counts and the diary must agree: {error}"
        );
    }

    #[test]
    fn a_bad_rules_file_refuses_at_its_line() {
        let doubled = "[man.vitus]\nname = \"Vitus\"\ntraits = \"steady\"\n\n\
                       [man.vitus]\nname = \"Vitus\"\ntraits = \"steady\"\n";
        assert_eq!(
            Contubernium::parse(doubled).expect_err("a man appears twice"),
            RosterError::DuplicateMan {
                line: 5,
                id: "vitus".to_string()
            }
        );
        let unquoted = "[man.vitus]\nname = Vitus\ntraits = \"steady\"\n";
        assert!(matches!(
            Contubernium::parse(unquoted).expect_err("a name is a quoted string"),
            RosterError::NotQuoted { line: 2, .. }
        ));
        let missing = "[man.vitus]\nname = \"Vitus\"\n";
        assert!(matches!(
            Contubernium::parse(missing).expect_err("the traits are required"),
            RosterError::MissingKey { key, .. } if key == "traits"
        ));
        assert_eq!(
            Contubernium::parse("# no man here\n").expect_err("a roster names men"),
            RosterError::NoMen
        );
    }
}
