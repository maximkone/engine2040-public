//! march: the road march of the legion.
//!
//! Consumer: the legionary mode opens a chapter on the road; this module
//! owns the column on the road line, the pace of the step, the stamina the
//! pack load costs and the time skip to the next set piece. Data owner: the
//! rules file `content/rome/march.toml` holds the numbers and the objective
//! line; this module owns the state that walks them.
//! Units: millimetres along the road, fixed ticks at [`TICKS_PER_HOUR`]'s
//! rate, stamina in permille (`0..=1000`). Determinism: integer math only,
//! no clock, no RNG and no float; the same rules plus the same calls give the
//! same state twice. Errors: [`MarchError`] at every boundary; a bad rules
//! file or a bad request fails loudly, never silently.
//! Budget: a cold path (a chapter opens on it, not every tick): one step is
//! O(men) integer work with no allocation, and the rules are parsed once.
//!
//! The rules file is the strict Rome table subset: a section that carries a
//! number carries a `ref` naming its source, and a section without one is
//! refused. The reader lives here because `engine-game` does not depend on
//! `engine-rome` (the gap: `crates/game/Cargo.toml` would need that edge, and
//! `crates/rome/src/table.rs` owns the reader).

use core::fmt;

use engine_core::{Fnv64, TICK_HZ_U32};

/// Fixed ticks in one hour at the engine's tick rate: the march clock counts
/// them, so five summer hours are five of these.
pub const TICKS_PER_HOUR: u32 = 3600 * TICK_HZ_U32;

/// Men in one marching column: a tent group of eight (`docs/plan/rome.md`
/// section 0, decision 5).
pub const COLUMN_MEN: usize = 8;

/// Largest column the march builds.
pub const MAX_MEN: usize = 64;

/// Millimetres in one kilometre: the unit the stamina rate counts.
const MM_PER_KM: u64 = 1_000_000;

/// The pace of the step: the regular step or the full step.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Pace {
    /// The regular step: 20 Roman miles in five summer hours (Vegetius 1.9).
    Regular,
    /// The full step: 24 Roman miles in the same five hours (Vegetius 1.9).
    Full,
}

impl Pace {
    /// The pace byte: `0` regular, `1` full. Pure.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Regular => 0,
            Self::Full => 1,
        }
    }
}

/// Why a rules file or a march request was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MarchError {
    /// A line is not a section header, a `key = value` pair, a comment or
    /// blank.
    Syntax {
        /// 1-based line number.
        line: usize,
        /// The line as written.
        text: String,
    },
    /// A value is neither a whole number nor a quoted string.
    NotAnInteger {
        /// 1-based line number.
        line: usize,
        /// The line as written.
        text: String,
    },
    /// A section appears twice.
    DuplicateSection {
        /// 1-based line number.
        line: usize,
        /// The section key.
        key: String,
    },
    /// A key appears twice inside one section.
    DuplicateKey {
        /// 1-based line number.
        line: usize,
        /// The section key.
        entry: String,
        /// The repeated key.
        key: String,
    },
    /// A section carries a number but no `ref` naming its source.
    MissingRef {
        /// 1-based line number of the section header.
        line: usize,
        /// The section key.
        entry: String,
    },
    /// A required section is missing.
    MissingEntry {
        /// The section key.
        entry: String,
    },
    /// A required key is missing from a section that exists.
    MissingKey {
        /// The section key.
        entry: String,
        /// The missing key.
        key: String,
    },
    /// A section holds a key the march does not read.
    UnknownKey {
        /// The section key.
        entry: String,
        /// The unknown key.
        key: String,
    },
    /// A value that must be above zero is zero.
    Zero {
        /// The section key.
        entry: String,
        /// The zero value's key.
        key: String,
    },
    /// A number does not fit the type its key holds.
    Overflow {
        /// The section key.
        entry: String,
        /// The overflowing key.
        key: String,
    },
    /// The column has no men, or more than [`MAX_MEN`].
    BadColumn {
        /// The men the caller asked for.
        men: usize,
    },
    /// The road holds no set piece ahead of the column.
    NoSetPiece,
}

impl fmt::Display for MarchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, text } => {
                write!(f, "line {line}: not a `key = value` line: {text}")
            }
            Self::NotAnInteger { line, text } => {
                write!(
                    f,
                    "line {line}: not a whole number or a quoted string: {text}"
                )
            }
            Self::DuplicateSection { line, key } => {
                write!(f, "line {line}: section `[{key}]` appears twice")
            }
            Self::DuplicateKey { line, entry, key } => {
                write!(f, "line {line}: `[{entry}]` holds `{key}` twice")
            }
            Self::MissingRef { line, entry } => write!(
                f,
                "line {line}: `[{entry}]` has numbers but no `ref` naming its source"
            ),
            Self::MissingEntry { entry } => write!(f, "the rules file has no `[{entry}]`"),
            Self::MissingKey { entry, key } => {
                write!(f, "`[{entry}]` has no `{key}`")
            }
            Self::UnknownKey { entry, key } => {
                write!(f, "`[{entry}]` has an unknown key `{key}`")
            }
            Self::Zero { entry, key } => {
                write!(f, "`[{entry}]` {key} must be above zero")
            }
            Self::Overflow { entry, key } => {
                write!(f, "`[{entry}]` {key} does not fit its number")
            }
            Self::BadColumn { men } => {
                write!(f, "a column of {men} men is not between 1 and {MAX_MEN}")
            }
            Self::NoSetPiece => write!(f, "the road holds no set piece ahead"),
        }
    }
}

impl std::error::Error for MarchError {}

/// One value written in the rules file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Value<'a> {
    /// A non-negative whole number as written.
    Int(u64),
    /// A quoted string without its quotes.
    Text(&'a str),
}

/// One section of the rules file: its key path, the line it starts on and its
/// values in file order.
#[derive(Clone, Debug)]
struct Section<'a> {
    key: &'a str,
    line: usize,
    values: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Section<'a> {
    /// The whole number written under `key`, when the section holds one.
    fn int(&self, key: &str) -> Option<u64> {
        for (name, value) in &self.values {
            if *name != key {
                continue;
            }
            if let Value::Int(number) = value {
                return Some(*number);
            }
        }
        None
    }

    /// The string written under `key`, when the section holds one.
    fn text(&self, key: &str) -> Option<&'a str> {
        for (name, value) in &self.values {
            if *name != key {
                continue;
            }
            if let Value::Text(text) = value {
                return Some(*text);
            }
        }
        None
    }

    /// The `ref` of this section: where its numbers come from.
    fn source(&self) -> Option<&'a str> {
        self.text("ref")
    }

    /// The whole number under `key`, or a refusal naming the key.
    fn require_int(&self, key: &str) -> Result<u64, MarchError> {
        self.int(key).ok_or_else(|| MarchError::MissingKey {
            entry: self.key.to_string(),
            key: key.to_string(),
        })
    }

    /// The string under `key`, or a refusal naming the key.
    fn require_text(&self, key: &str) -> Result<&'a str, MarchError> {
        self.text(key).ok_or_else(|| MarchError::MissingKey {
            entry: self.key.to_string(),
            key: key.to_string(),
        })
    }

    /// Refuse a key the march does not read; `ref` is always allowed.
    fn only(&self, allowed: &[&str]) -> Result<(), MarchError> {
        for (key, _) in &self.values {
            if *key == "ref" || allowed.contains(key) {
                continue;
            }
            return Err(MarchError::UnknownKey {
                entry: self.key.to_string(),
                key: (*key).to_string(),
            });
        }
        Ok(())
    }
}

/// One value of a rules file, read as a whole number.
fn whole(text: &str, line: usize, whole_line: &str) -> Result<u64, MarchError> {
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(MarchError::NotAnInteger {
            line,
            text: whole_line.to_string(),
        });
    }
    text.parse().map_err(|_| MarchError::NotAnInteger {
        line,
        text: whole_line.to_string(),
    })
}

/// A lowercase identifier: a letter or `_`, then letters, digits and `_`.
fn is_identifier(text: &str) -> bool {
    let mut letters = text.chars();
    let Some(head) = letters.next() else {
        return false;
    };
    if !(head.is_ascii_lowercase() || head == '_') {
        return false;
    }
    letters.all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit() || letter == '_')
}

/// Read the rules file into its sections, in file order.
///
/// A file is a list of `[a.b]` sections holding `key = 12` or `key = "text"`
/// lines; `#` starts a comment and blank lines are ignored. Nothing else is
/// accepted: no floats, no arrays, no inline tables, no repeated key or
/// section, and a refused file names the line that refused it.
fn parse_sections<'a>(text: &'a str) -> Result<Vec<Section<'a>>, MarchError> {
    let mut sections: Vec<Section<'a>> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if let Some(key) = section_header(body, line)? {
            if sections.iter().any(|section| section.key == key) {
                return Err(MarchError::DuplicateSection {
                    line,
                    key: key.to_string(),
                });
            }
            sections.push(Section {
                key,
                line,
                values: Vec::new(),
            });
            continue;
        }
        let Some(section) = sections.last_mut() else {
            return Err(MarchError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        let Some((key, value)) = body.split_once('=') else {
            return Err(MarchError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        let key = key.trim();
        if !is_identifier(key) {
            return Err(MarchError::Syntax {
                line,
                text: body.to_string(),
            });
        }
        if section.values.iter().any(|(name, _)| *name == key) {
            return Err(MarchError::DuplicateKey {
                line,
                entry: section.key.to_string(),
                key: key.to_string(),
            });
        }
        let value = parse_value(value.trim(), line, body)?;
        section.values.push((key, value));
    }
    check_refs(&sections)?;
    Ok(sections)
}

/// The section key of a `[a.b]` header, or `None` when the line is not one.
fn section_header(body: &str, line: usize) -> Result<Option<&str>, MarchError> {
    let Some(rest) = body.strip_prefix('[') else {
        return Ok(None);
    };
    let Some(inner) = rest.strip_suffix(']') else {
        return Err(MarchError::Syntax {
            line,
            text: body.to_string(),
        });
    };
    let valid =
        !inner.is_empty() && !inner.contains(['[', ']']) && inner.split('.').all(is_identifier);
    if !valid {
        return Err(MarchError::Syntax {
            line,
            text: body.to_string(),
        });
    }
    Ok(Some(inner))
}

/// One `key = value` line: the key and its typed value.
fn parse_value<'a>(
    text: &'a str,
    line: usize,
    whole_line: &'a str,
) -> Result<Value<'a>, MarchError> {
    if let Some(rest) = text.strip_prefix('"') {
        let Some(inner) = rest.strip_suffix('"') else {
            return Err(MarchError::Syntax {
                line,
                text: whole_line.to_string(),
            });
        };
        if inner.contains('"') || inner.contains('\\') {
            return Err(MarchError::Syntax {
                line,
                text: whole_line.to_string(),
            });
        }
        return Ok(Value::Text(inner));
    }
    Ok(Value::Int(whole(text, line, whole_line)?))
}

/// Refuse every section that carries a number and no `ref` naming its source.
fn check_refs(sections: &[Section<'_>]) -> Result<(), MarchError> {
    for section in sections {
        let numbered = section
            .values
            .iter()
            .any(|(_, value)| matches!(value, Value::Int(_)));
        let sourced = section.source().is_some_and(|text| !text.trim().is_empty());
        if numbered && !sourced {
            return Err(MarchError::MissingRef {
                line: section.line,
                entry: section.key.to_string(),
            });
        }
    }
    Ok(())
}

/// One set piece on the road: where it stands, when it begins and the
/// objective line the mode shows while the column marches toward it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SetPiece {
    /// The name of the piece, for example `camp`.
    pub name: String,
    /// Distance up the road from the start of the march, in millimetres.
    pub distance_mm: u64,
    /// The tick the piece begins at, counted from the start of the march.
    pub start_tick: u32,
    /// The objective line the mode shows while the column marches here.
    pub objective: String,
}

/// The march rules: the numbers of `content/rome/march.toml`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MarchRules {
    mm_per_mile: u64,
    regular_mm_per_hour: u64,
    full_mm_per_hour: u64,
    pack_load_lb: u32,
    files: u32,
    frontage_mm: i64,
    rank_gap_mm: i64,
    stamina_max: u16,
    drain_per_lb_per_km: u32,
    full_step_extra_per_km: u32,
    set_pieces: Vec<SetPiece>,
    objective: String,
}

impl MarchRules {
    /// The rules file shipped with the game.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/march.toml");

    /// Read the shipped rules file.
    ///
    /// # Errors
    ///
    /// Returns the first [`MarchError`] the file earns: a malformed line, a
    /// repeated key or section, a number without a `ref`, or a missing number.
    pub fn shipped() -> Result<Self, MarchError> {
        Self::parse(Self::SHIPPED)
    }

    /// Read a rules file.
    ///
    /// # Errors
    ///
    /// Returns the first [`MarchError`] the text earns, as [`MarchRules::shipped`]
    /// does for the shipped file.
    pub fn parse(text: &str) -> Result<Self, MarchError> {
        let sections = parse_sections(text)?;
        let (mm_per_ft, mm_per_mile) = read_units(&sections)?;
        let (regular_mm_per_hour, full_mm_per_hour) = read_paces(&sections, mm_per_mile)?;
        let pack_load_lb = read_pack(&sections)?;
        let (files, frontage_mm, rank_gap_mm) = read_column(&sections, mm_per_ft)?;
        let (stamina_max, drain_per_lb_per_km, full_step_extra_per_km) = read_stamina(&sections)?;
        let objective = read_objective(&sections)?;
        let set_pieces = read_set_pieces(&sections, mm_per_mile, &objective)?;

        Ok(Self {
            mm_per_mile,
            regular_mm_per_hour,
            full_mm_per_hour,
            pack_load_lb,
            files,
            frontage_mm,
            rank_gap_mm,
            stamina_max,
            drain_per_lb_per_km,
            full_step_extra_per_km,
            set_pieces,
            objective,
        })
    }

    /// Millimetres one Roman mile holds.
    #[must_use]
    pub const fn mm_per_mile(&self) -> u64 {
        self.mm_per_mile
    }

    /// The millimetres the column walks in one hour at `pace`.
    ///
    /// The regular step walks 20 Roman miles in five summer hours, the full
    /// step 24 in the same five (Vegetius 1.9).
    #[must_use]
    pub const fn mm_per_hour(&self, pace: Pace) -> u64 {
        match pace {
            Pace::Regular => self.regular_mm_per_hour,
            Pace::Full => self.full_mm_per_hour,
        }
    }

    /// The pack a man carries, in pounds.
    #[must_use]
    pub const fn pack_load_lb(&self) -> u32 {
        self.pack_load_lb
    }

    /// The files the column marches abreast.
    #[must_use]
    pub const fn files(&self) -> u32 {
        self.files
    }

    /// The frontage of one man, in millimetres.
    #[must_use]
    pub const fn frontage_mm(&self) -> i64 {
        self.frontage_mm
    }

    /// The gap between two ranks, in millimetres.
    #[must_use]
    pub const fn rank_gap_mm(&self) -> i64 {
        self.rank_gap_mm
    }

    /// The stamina a fresh man holds, in permille.
    #[must_use]
    pub const fn stamina_max(&self) -> u16 {
        self.stamina_max
    }

    /// The stamina one kilometre costs a man with `load_lb` on his back at
    /// `pace`, in permille.
    ///
    /// The pack load is the cost: every pound of it costs the leg a permille
    /// per kilometre. The full step costs more than the regular one, because
    /// the harder pace tires the man faster.
    #[must_use]
    pub fn drain_per_km(&self, pace: Pace, load_lb: u32) -> u32 {
        let pack = load_lb.saturating_mul(self.drain_per_lb_per_km);
        match pace {
            Pace::Regular => pack,
            Pace::Full => pack.saturating_add(self.full_step_extra_per_km),
        }
    }

    /// The set pieces on the road, in road order.
    #[must_use]
    pub fn set_pieces(&self) -> &[SetPiece] {
        &self.set_pieces
    }

    /// The objective line the mode shows: the line the rules file writes.
    #[must_use]
    pub fn objective(&self) -> &str {
        &self.objective
    }

    /// A digest of the rules: the same file read twice gives the same digest.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"march.rules");
        hasher.write_u64(self.mm_per_mile);
        hasher.write_u64(self.regular_mm_per_hour);
        hasher.write_u64(self.full_mm_per_hour);
        hasher.write_u32(self.pack_load_lb);
        hasher.write_u32(self.files);
        hasher.write_i64(self.frontage_mm);
        hasher.write_i64(self.rank_gap_mm);
        hasher.write_u16(self.stamina_max);
        hasher.write_u32(self.drain_per_lb_per_km);
        hasher.write_u32(self.full_step_extra_per_km);
        hasher.write_str(self.objective.as_str());
        for piece in &self.set_pieces {
            hasher.write_str(piece.name.as_str());
            hasher.write_u64(piece.distance_mm);
            hasher.write_u32(piece.start_tick);
        }
        hasher.finish()
    }
}

/// The millimetres of a Roman foot and of a Roman mile.
fn read_units(sections: &[Section<'_>]) -> Result<(u64, u64), MarchError> {
    let units = find_section(sections, "units")?;
    units.only(&["mm_per_ft", "mm_per_mile"])?;
    let mm_per_ft = units.require_int("mm_per_ft")?;
    let mm_per_mile = units.require_int("mm_per_mile")?;
    if mm_per_ft == 0 || mm_per_mile == 0 {
        return Err(MarchError::Zero {
            entry: "units".to_string(),
            key: "mm_per_ft".to_string(),
        });
    }
    Ok((mm_per_ft, mm_per_mile))
}

/// The millimetres one hour of the regular step and of the full step holds.
fn read_paces(sections: &[Section<'_>], mm_per_mile: u64) -> Result<(u64, u64), MarchError> {
    let regular = find_section(sections, "pace.regular")?;
    regular.only(&["miles", "hours"])?;
    let full = find_section(sections, "pace.full")?;
    full.only(&["miles", "hours"])?;
    Ok((
        pace_mm_per_hour(regular, mm_per_mile)?,
        pace_mm_per_hour(full, mm_per_mile)?,
    ))
}

/// The pack a man carries, in pounds.
fn read_pack(sections: &[Section<'_>]) -> Result<u32, MarchError> {
    let pack = find_section(sections, "pack")?;
    pack.only(&["load_lb"])?;
    let load = pack.require_int("load_lb")?;
    if load == 0 {
        return Err(MarchError::Zero {
            entry: "pack".to_string(),
            key: "load_lb".to_string(),
        });
    }
    u32::try_from(load).map_err(|_| MarchError::Overflow {
        entry: "pack".to_string(),
        key: "load_lb".to_string(),
    })
}

/// The files the column marches abreast, with the frontage and the rank gap
/// it holds in millimetres.
fn read_column(sections: &[Section<'_>], mm_per_ft: u64) -> Result<(u32, i64, i64), MarchError> {
    let column = find_section(sections, "column")?;
    column.only(&["files", "frontage_ft", "rank_gap_ft"])?;
    let files = column.require_int("files")?;
    let frontage_ft = column.require_int("frontage_ft")?;
    let rank_gap_ft = column.require_int("rank_gap_ft")?;
    if files == 0 || frontage_ft == 0 || rank_gap_ft == 0 {
        return Err(MarchError::Zero {
            entry: "column".to_string(),
            key: "files".to_string(),
        });
    }
    let files = u32::try_from(files).map_err(|_| MarchError::Overflow {
        entry: "column".to_string(),
        key: "files".to_string(),
    })?;
    let frontage_mm = feet_to_mm(frontage_ft, mm_per_ft, "frontage_ft")?;
    let rank_gap_mm = feet_to_mm(rank_gap_ft, mm_per_ft, "rank_gap_ft")?;
    Ok((files, frontage_mm, rank_gap_mm))
}

/// The stamina a fresh man holds, with the two rates that spend it.
fn read_stamina(sections: &[Section<'_>]) -> Result<(u16, u32, u32), MarchError> {
    let stamina = find_section(sections, "stamina")?;
    stamina.only(&[
        "max_permille",
        "drain_per_lb_per_km",
        "full_step_extra_per_km",
    ])?;
    let max_permille = stamina.require_int("max_permille")?;
    let drain = stamina.require_int("drain_per_lb_per_km")?;
    let full_extra = stamina.require_int("full_step_extra_per_km")?;
    if max_permille == 0 {
        return Err(MarchError::Zero {
            entry: "stamina".to_string(),
            key: "max_permille".to_string(),
        });
    }
    let stamina_max = u16::try_from(max_permille).map_err(|_| MarchError::Overflow {
        entry: "stamina".to_string(),
        key: "max_permille".to_string(),
    })?;
    let drain_per_lb_per_km = u32::try_from(drain).map_err(|_| MarchError::Overflow {
        entry: "stamina".to_string(),
        key: "drain_per_lb_per_km".to_string(),
    })?;
    let full_step_extra_per_km = u32::try_from(full_extra).map_err(|_| MarchError::Overflow {
        entry: "stamina".to_string(),
        key: "full_step_extra_per_km".to_string(),
    })?;
    Ok((stamina_max, drain_per_lb_per_km, full_step_extra_per_km))
}

/// The objective line the mode shows.
fn read_objective(sections: &[Section<'_>]) -> Result<String, MarchError> {
    let objective = find_section(sections, "objective")?;
    objective.only(&["line"])?;
    Ok(objective.require_text("line")?.to_string())
}

/// The set pieces on the road, in road order.
///
/// Every set piece carries the objective line of the march: the mode shows the
/// same line while the column marches toward any of them.
fn read_set_pieces(
    sections: &[Section<'_>],
    mm_per_mile: u64,
    objective: &str,
) -> Result<Vec<SetPiece>, MarchError> {
    let mut set_pieces = Vec::new();
    for section in sections {
        let Some(name) = section.key.strip_prefix("set_piece.") else {
            continue;
        };
        section.only(&["miles", "start_hour"])?;
        let miles = section.require_int("miles")?;
        let start_hour = section.require_int("start_hour")?;
        if miles == 0 {
            return Err(MarchError::Zero {
                entry: section.key.to_string(),
                key: "miles".to_string(),
            });
        }
        let distance_mm = miles
            .checked_mul(mm_per_mile)
            .ok_or_else(|| MarchError::Overflow {
                entry: section.key.to_string(),
                key: "miles".to_string(),
            })?;
        let start_tick = start_hour
            .checked_mul(u64::from(TICKS_PER_HOUR))
            .and_then(|ticks| u32::try_from(ticks).ok())
            .ok_or_else(|| MarchError::Overflow {
                entry: section.key.to_string(),
                key: "start_hour".to_string(),
            })?;
        set_pieces.push(SetPiece {
            name: name.to_string(),
            distance_mm,
            start_tick,
            objective: objective.to_string(),
        });
    }
    // The road runs from the near set piece to the far one.
    set_pieces.sort_unstable_by_key(|piece| piece.distance_mm);
    Ok(set_pieces)
}

/// The section of the rules file the march reads.
fn find_section<'a, 'b>(
    sections: &'b [Section<'a>],
    key: &str,
) -> Result<&'b Section<'a>, MarchError> {
    sections
        .iter()
        .find(|section| section.key == key)
        .ok_or_else(|| MarchError::MissingEntry {
            entry: key.to_string(),
        })
}

/// Roman feet in millimetres.
fn feet_to_mm(feet: u64, mm_per_ft: u64, key: &str) -> Result<i64, MarchError> {
    let mm = feet
        .checked_mul(mm_per_ft)
        .ok_or_else(|| MarchError::Overflow {
            entry: "column".to_string(),
            key: key.to_string(),
        })?;
    i64::try_from(mm).map_err(|_| MarchError::Overflow {
        entry: "column".to_string(),
        key: key.to_string(),
    })
}

/// The millimetres one hour of a pace's march holds.
fn pace_mm_per_hour(section: &Section<'_>, mm_per_mile: u64) -> Result<u64, MarchError> {
    let miles = section.require_int("miles")?;
    let hours = section.require_int("hours")?;
    if miles == 0 || hours == 0 {
        return Err(MarchError::Zero {
            entry: section.key.to_string(),
            key: "miles".to_string(),
        });
    }
    let mm = miles
        .checked_mul(mm_per_mile)
        .ok_or_else(|| MarchError::Overflow {
            entry: section.key.to_string(),
            key: "miles".to_string(),
        })?;
    mm.checked_div(hours).ok_or_else(|| MarchError::Overflow {
        entry: section.key.to_string(),
        key: "hours".to_string(),
    })
}

/// One man of the column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Marcher {
    /// His file across the road, `0..files`.
    pub file: u8,
    /// His rank back from the front of the column; `0` is the front rank.
    pub rank: u8,
    /// The pack he carries, in pounds.
    pub load_lb: u32,
    /// His stamina, in permille.
    pub stamina: u16,
}

/// Where one man of the column stands on the road.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    /// The slot of the column this man holds.
    pub index: u8,
    /// His file across the road.
    pub file: u8,
    /// His rank back from the front.
    pub rank: u8,
    /// His distance from the start of the road, in millimetres.
    pub along_mm: i64,
    /// His offset from the middle of the road, in millimetres; negative is
    /// the left side.
    pub lateral_mm: i64,
}

/// What one time skip did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Skip {
    /// The name of the set piece the column landed on.
    pub piece: String,
    /// The road the column covered, in millimetres.
    pub road_mm: u64,
    /// The tick the column stands at after the skip.
    pub tick: u32,
    /// The ticks the column waited for the set piece to begin.
    pub waited_ticks: u32,
}

/// The march: the column on the road and the clock that walks it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct March {
    rules: MarchRules,
    pace: Pace,
    /// The distance the front of the column has walked, in millimetres.
    distance_mm: u64,
    /// The clock: ticks since the march began.
    tick: u32,
    /// The millimetres the pace has not spent yet: a pace is a fraction of a
    /// millimetre per tick, and the rest waits for the next step.
    remainder: u64,
    /// The men of the column, in slot order.
    column: Vec<Marcher>,
}

impl March {
    /// Start a march: a column of `men` men on the road at the regular step.
    ///
    /// # Errors
    ///
    /// Returns [`MarchError::BadColumn`] when the column has no men or more
    /// than [`MAX_MEN`].
    pub fn new(rules: MarchRules, men: usize) -> Result<Self, MarchError> {
        if men == 0 || men > MAX_MEN {
            return Err(MarchError::BadColumn { men });
        }
        let files = rules.files();
        let mut column = Vec::with_capacity(men);
        for index in 0..men {
            let slot = u32::try_from(index).unwrap_or(u32::MAX);
            column.push(Marcher {
                file: u8::try_from(slot % files).unwrap_or(0),
                rank: u8::try_from(slot / files).unwrap_or(0),
                load_lb: rules.pack_load_lb(),
                stamina: rules.stamina_max(),
            });
        }
        Ok(Self {
            rules,
            pace: Pace::Regular,
            distance_mm: 0,
            tick: 0,
            remainder: 0,
            column,
        })
    }

    /// The rules the march walks.
    #[must_use]
    pub const fn rules(&self) -> &MarchRules {
        &self.rules
    }

    /// The pace the column walks at.
    #[must_use]
    pub const fn pace(&self) -> Pace {
        self.pace
    }

    /// Set the pace: the regular step or the full step.
    pub fn set_pace(&mut self, pace: Pace) {
        self.pace = pace;
    }

    /// The clock: ticks since the march began.
    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.tick
    }

    /// The distance the front of the column has walked, in millimetres.
    #[must_use]
    pub const fn distance_mm(&self) -> u64 {
        self.distance_mm
    }

    /// The men of the column, in slot order.
    #[must_use]
    pub fn men(&self) -> &[Marcher] {
        &self.column
    }

    /// The objective line the mode shows, from the rules file.
    #[must_use]
    pub fn objective(&self) -> &str {
        self.rules.objective()
    }

    /// Where a man of the column stands on the road.
    ///
    /// The man of rank `r` stands `r` rank gaps behind the front of the
    /// column, and his file holds its place across the road. The front of the
    /// column stands where the march has walked, so the men of the last rank
    /// stand behind the start of the road until the column moves off.
    #[must_use]
    pub fn position(&self, index: usize) -> Option<Slot> {
        let man = self.column.get(index)?;
        let along_mm = i64::try_from(self.distance_mm).unwrap_or(i64::MAX)
            - self.rules.rank_gap_mm() * i64::from(man.rank);
        Some(Slot {
            index: u8::try_from(index).unwrap_or(u8::MAX),
            file: man.file,
            rank: man.rank,
            along_mm,
            lateral_mm: self.lateral_mm(man.file),
        })
    }

    /// The offset of a file from the middle of the road, in millimetres.
    ///
    /// The files are centred on the road, so an even number of files stands
    /// half a frontage apart around the middle and an odd number puts one
    /// file on the middle. An odd frontage truncates toward the middle.
    fn lateral_mm(&self, file: u8) -> i64 {
        let files = i64::from(self.rules.files());
        let step = 2 * i64::from(file) + 1 - files;
        step * self.rules.frontage_mm() / 2
    }

    /// The ticks the column needs to walk `millimetres` at the current pace.
    #[must_use]
    pub fn ticks_for_mm(&self, millimetres: u64) -> u32 {
        let per_hour = self.rules.mm_per_hour(self.pace);
        if per_hour == 0 {
            return 0;
        }
        let ticks = millimetres
            .saturating_mul(u64::from(TICKS_PER_HOUR))
            .div_ceil(per_hour);
        u32::try_from(ticks).unwrap_or(u32::MAX)
    }

    /// Walk the column `ticks` ticks up the road.
    ///
    /// The pace gives the millimetres of one hour, and the fraction of a
    /// millimetre a tick cannot spend waits for the next one, so a long
    /// march loses no ground. Every man pays the stamina the walked road
    /// costs at the current pace.
    pub fn advance_ticks(&mut self, ticks: u32) {
        if ticks == 0 {
            return;
        }
        let per_hour = self.rules.mm_per_hour(self.pace);
        let moved = per_hour
            .saturating_mul(u64::from(ticks))
            .saturating_add(self.remainder);
        let walked = moved / u64::from(TICKS_PER_HOUR);
        self.remainder = moved % u64::from(TICKS_PER_HOUR);
        self.distance_mm = self.distance_mm.saturating_add(walked);
        self.tick = self.tick.saturating_add(ticks);
        self.drain(walked);
    }

    /// Take the stamina the walked road costs every man.
    fn drain(&mut self, millimetres: u64) {
        if millimetres == 0 {
            return;
        }
        let pace = self.pace;
        for man in &mut self.column {
            let per_km = self.rules.drain_per_km(pace, man.load_lb);
            let cost = u64::from(per_km).saturating_mul(millimetres) / MM_PER_KM;
            let cost = u16::try_from(cost).unwrap_or(u16::MAX);
            man.stamina = man.stamina.saturating_sub(cost);
        }
    }

    /// The set piece the column marches to next: the first one ahead of it.
    #[must_use]
    pub fn next_set_piece(&self) -> Option<&SetPiece> {
        self.rules
            .set_pieces
            .iter()
            .find(|piece| piece.distance_mm > self.distance_mm)
    }

    /// Skip the road to the next set piece.
    ///
    /// The column lands on the set piece's place and the clock on the tick it
    /// begins at; the men pay the stamina the covered road costs. The wait
    /// between the arrival and the start is the time the skip cuts.
    ///
    /// # Errors
    ///
    /// Returns [`MarchError::NoSetPiece`] when the road holds no set piece
    /// ahead of the column.
    pub fn skip_to_next_set_piece(&mut self) -> Result<Skip, MarchError> {
        let piece = self.next_set_piece().ok_or(MarchError::NoSetPiece)?.clone();
        let road_mm = piece.distance_mm.saturating_sub(self.distance_mm);
        let travel = self.ticks_for_mm(road_mm);
        let arrival = self.tick.saturating_add(travel);
        let tick = arrival.max(piece.start_tick);
        self.drain(road_mm);
        self.distance_mm = piece.distance_mm;
        self.tick = tick;
        self.remainder = 0;
        Ok(Skip {
            piece: piece.name,
            road_mm,
            tick,
            waited_ticks: tick.saturating_sub(arrival),
        })
    }

    /// A digest of the march: the pace, the clock, the road and every man's
    /// file, rank and stamina. Two marches that walked the same road give the
    /// same digest.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"march");
        hasher.write_u8(self.pace.code());
        hasher.write_u32(self.tick);
        hasher.write_u64(self.distance_mm);
        for man in &self.column {
            hasher.write_u8(man.file);
            hasher.write_u8(man.rank);
            hasher.write_u32(man.load_lb);
            hasher.write_u16(man.stamina);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot of every man of a march, in slot order.
    fn slots(march: &March) -> Vec<Slot> {
        (0..march.men().len())
            .map(|index| march.position(index).expect("a man of the column"))
            .collect()
    }

    #[test]
    fn the_column_holds_order_over_500_m_of_road() {
        let rules = MarchRules::shipped().expect("march rules");
        let mut march = March::new(rules, COLUMN_MEN).expect("a column of eight");
        let frontage = march.rules().frontage_mm();
        let rank_gap = march.rules().rank_gap_mm();
        let files = usize::try_from(march.rules().files()).expect("files");
        assert_eq!(frontage, 888, "3 ft of front per man");
        assert_eq!(rank_gap, 1776, "6 ft between the ranks");

        let start = slots(&march);
        // Walk five hundred metres of road. One tick of headroom, so the
        // column stands at the mark and not a stride short of it.
        let ticks = march.ticks_for_mm(500_000);
        march.advance_ticks(ticks + 1);
        assert!(
            march.distance_mm() >= 500_000,
            "the column walked {} mm",
            march.distance_mm()
        );
        let end = slots(&march);

        // The front of the column stands where the march walked.
        let head = end[0].along_mm;
        assert_eq!(head, i64::try_from(march.distance_mm()).expect("the road"));

        // Every man kept his file: the offset across the road never moved.
        for (before, after) in start.iter().zip(end.iter()) {
            assert_eq!(before.lateral_mm, after.lateral_mm, "a man left his file");
            assert_eq!(before.file, after.file);
            assert_eq!(before.rank, after.rank);
        }

        // Every man kept his rank: the gap to the front is the rank gap.
        for (index, after) in end.iter().enumerate() {
            let rank = i64::try_from(index / files).expect("the rank");
            assert_eq!(
                head - after.along_mm,
                rank * rank_gap,
                "rank {rank} kept its gap"
            );
        }

        // The files stand the frontage apart, centred on the road.
        for file in 0..files {
            let man = march.position(file).expect("a file");
            let step = 2 * i64::try_from(file).expect("the file") + 1
                - i64::try_from(files).expect("the files");
            assert_eq!(man.lateral_mm, step * frontage / 2, "file {file} sat right");
        }

        // Nobody passed anybody: the column is still in slot order.
        for pair in end.windows(2) {
            assert!(
                pair[0].along_mm >= pair[1].along_mm,
                "the column lost its order"
            );
        }
    }

    #[test]
    fn the_full_step_drains_stamina_faster_than_the_regular_step() {
        let rules = MarchRules::shipped().expect("march rules");
        let mut regular = March::new(rules.clone(), COLUMN_MEN).expect("a column of eight");
        let mut full = March::new(rules, COLUMN_MEN).expect("a column of eight");
        full.set_pace(Pace::Full);

        // The same hour on the same road.
        regular.advance_ticks(TICKS_PER_HOUR);
        full.advance_ticks(TICKS_PER_HOUR);

        // The full step walks further in the hour...
        assert!(
            full.distance_mm() > regular.distance_mm(),
            "the full step is faster"
        );
        // ...and costs the man more stamina for it.
        assert!(
            full.men()[0].stamina < regular.men()[0].stamina,
            "the full step tires the man faster: {} against {}",
            full.men()[0].stamina,
            regular.men()[0].stamina
        );
        // The full step costs more per kilometre too, so it tires the man
        // faster on the same road.
        let load = regular.rules().pack_load_lb();
        assert!(
            full.rules().drain_per_km(Pace::Full, load)
                > regular.rules().drain_per_km(Pace::Regular, load),
            "the full step costs more per kilometre"
        );
    }

    #[test]
    fn the_skip_lands_at_the_set_piece_with_the_same_state_twice() {
        let rules = MarchRules::shipped().expect("march rules");
        let mut first = March::new(rules.clone(), COLUMN_MEN).expect("a column of eight");
        let mut second = March::new(rules, COLUMN_MEN).expect("a column of eight");

        let one = first.skip_to_next_set_piece().expect("a set piece ahead");
        let two = second.skip_to_next_set_piece().expect("a set piece ahead");
        assert_eq!(one, two, "the skip reports the same thing twice");
        assert_eq!(first, second, "the same state twice");
        assert_eq!(first.digest(), second.digest(), "the same digest twice");

        // The column stands on the camp and the clock on its start.
        let camp = first.rules().set_pieces().first().expect("the camp");
        assert_eq!(camp.name, "camp");
        assert_eq!(first.distance_mm(), camp.distance_mm);
        assert_eq!(first.tick(), camp.start_tick);
        assert_eq!(one.piece, "camp");

        // The men paid for the road they walked.
        assert!(
            first.men()[0].stamina < first.rules().stamina_max(),
            "the road cost the men their stamina"
        );

        // The next skip lands on the piece behind it.
        let next = first.skip_to_next_set_piece().expect("a set piece ahead");
        assert_eq!(next.piece, "ambush");
    }

    #[test]
    fn the_objective_line_comes_from_the_rules_file() {
        let rules = MarchRules::shipped().expect("march rules");
        let march = March::new(rules, COLUMN_MEN).expect("a column of eight");
        assert_eq!(march.objective(), "March to the camp");
    }

    #[test]
    fn the_rules_read_the_sourced_numbers() {
        let rules = MarchRules::shipped().expect("march rules");
        assert_eq!(rules.pack_load_lb(), 60, "60 lb of kit");
        assert_eq!(
            rules.mm_per_hour(Pace::Regular),
            5_920_000,
            "20 miles in 5 hours"
        );
        assert_eq!(
            rules.mm_per_hour(Pace::Full),
            7_104_000,
            "24 miles in 5 hours"
        );
        assert_eq!(rules.mm_per_mile(), 1_480_000, "a Roman mile");
        assert_eq!(rules.stamina_max(), 1000);
        assert_eq!(rules.set_pieces().len(), 2, "the camp and the ambush");
    }

    #[test]
    fn the_regular_step_walks_twenty_miles_in_five_hours() {
        let rules = MarchRules::shipped().expect("march rules");
        let mut regular = March::new(rules.clone(), COLUMN_MEN).expect("a column of eight");
        let mut full = March::new(rules, COLUMN_MEN).expect("a column of eight");
        full.set_pace(Pace::Full);
        regular.advance_ticks(TICKS_PER_HOUR * 5);
        full.advance_ticks(TICKS_PER_HOUR * 5);
        assert_eq!(regular.distance_mm(), 20 * 1_480_000, "20 Roman miles");
        assert_eq!(full.distance_mm(), 24 * 1_480_000, "24 Roman miles");
    }

    #[test]
    fn two_loads_of_the_rules_hash_the_same() {
        let first = MarchRules::shipped().expect("march rules");
        let second = MarchRules::shipped().expect("march rules");
        assert_eq!(first, second);
        assert_eq!(first.digest(), second.digest());
    }

    #[test]
    fn a_number_without_a_source_is_refused() {
        let text = "[units]\nmm_per_ft = 296\n";
        let error = MarchRules::parse(text).expect_err("a number with no ref");
        assert!(
            matches!(error, MarchError::MissingRef { .. }),
            "wrong error: {error}"
        );
    }

    #[test]
    fn a_float_is_refused_at_its_line() {
        let text = "[units]\nmm_per_ft = 296.5\nref = \"rome.md section 5\"\n";
        let error = MarchRules::parse(text).expect_err("a float");
        assert!(
            matches!(error, MarchError::NotAnInteger { line: 2, .. }),
            "wrong error: {error}"
        );
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = MarchRules::SHIPPED.replace("rank_gap_ft = 6", "rank_gapp_ft = 6");
        let error = MarchRules::parse(&text).expect_err("a typo in a key");
        assert!(error.to_string().contains("rank_gapp_ft"), "{error}");
    }

    #[test]
    fn a_missing_section_is_refused_by_name() {
        let text = MarchRules::SHIPPED.replace("[pack]", "[packk]");
        let error = MarchRules::parse(&text).expect_err("a missing section");
        assert!(
            matches!(error, MarchError::MissingEntry { ref entry } if entry == "pack"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn an_empty_column_is_refused() {
        let rules = MarchRules::shipped().expect("march rules");
        assert!(matches!(
            March::new(rules.clone(), 0),
            Err(MarchError::BadColumn { men: 0 })
        ));
        assert!(matches!(
            March::new(rules, MAX_MEN + 1),
            Err(MarchError::BadColumn { .. })
        ));
    }

    #[test]
    fn a_road_without_a_set_piece_refuses_the_skip() {
        let text = MarchRules::SHIPPED
            .replace("[set_piece.camp]", "[setpiece.camp]")
            .replace("[set_piece.ambush]", "[setpiece.ambush]");
        let rules = MarchRules::parse(&text).expect("march rules");
        let mut march = March::new(rules, COLUMN_MEN).expect("a column of eight");
        assert!(matches!(
            march.skip_to_next_set_piece(),
            Err(MarchError::NoSetPiece)
        ));
    }
}
