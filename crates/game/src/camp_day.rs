//! camp_day: the camp day is a works table.
//!
//! Consumer: the legionary mode hands the camp day one command per player action and
//! one [`CampDay::tick`] per fixed tick; the window draws the props the camp day shows
//! when [`CampDay::frame`] hands them over. Data owner: `content/rome/works.toml` holds
//! the eight works and their man-tick costs; this module owns the state that spends
//! them. The ditch's shape (width and depth by threat) stays in `crates/rome/src/camp.rs`
//! over `content/rome/camp.toml`; this module only prices the digging.
//! Units: man-ticks, one man working one fixed tick ([`MAN_TICKS_PER_HOUR`] is an hour
//! of one man); men are the tent group's eight at most ([`MAX_CREW`]).
//! Determinism: integer math only, no clock, no RNG and no float; the same table plus
//! the same calls give the same state twice, and [`CampDay::hash`] proves it.
//! Errors: [`CampDayError`] at every boundary; a bad table or a bad command fails
//! loudly, never silently.
//! Budget: a cold path (a camp day opens on it, and one tick is O(works) integer work
//! with no allocation); the table is parsed once.
//!
//! The table is the strict Rome table subset: a section that carries a number carries a
//! `ref` naming its source, and a section without one is refused. The small reader
//! lives here for the reason [`crate::march`] gives: `engine-game` does not depend on
//! `engine-rome`, which owns `crates/rome/src/table.rs`.
//!
//! The finished prop is handed over, never drawn in the tick that finishes it: the work
//! row lands in `pending` on its last man-tick, and [`CampDay::frame`] moves it to the
//! presented props, so the consumer sees it on the next frame.

use core::fmt;

use engine_core::text::is_identifier;
use engine_core::{Fnv64, TICK_HZ_U32};

/// The eight works a camp day holds: the row count of `content/rome/works.toml`.
pub const MAX_WORKS: usize = 8;

/// The most men one work takes: the eight-man tent group.
pub const MAX_CREW: u16 = 8;

/// Man-ticks in one hour of one man: 3600 s at the engine's fixed tick rate.
pub const MAN_TICKS_PER_HOUR: u32 = 3600 * TICK_HZ_U32;

/// Why a works table or a camp-day command was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CampDayError {
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
    /// A section holds the same key twice.
    DuplicateKey {
        /// The 1-based line of the second key.
        line: usize,
        /// The section key.
        entry: String,
        /// The repeated key.
        key: String,
    },
    /// A section is neither `[units]` nor `[work.<id>]`.
    UnknownSection {
        /// The 1-based line of the header.
        line: usize,
        /// The section key as written.
        key: String,
    },
    /// A section carried a number and no `ref` naming its source.
    MissingRef {
        /// The 1-based line of the section header.
        line: usize,
        /// The section key.
        entry: String,
    },
    /// A section holds a key the camp day does not read.
    UnknownKey {
        /// The section key.
        entry: String,
        /// The key it does not read.
        key: String,
    },
    /// A section holds no value for a key the camp day must read.
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
        /// The value as written.
        text: String,
    },
    /// A value does not fit the unit it is read as.
    OutOfRange {
        /// The 1-based line that refused.
        line: usize,
        /// The section key.
        entry: String,
    },
    /// The table does not hold [`MAX_WORKS`] works.
    WorkCount {
        /// The works the table holds.
        found: usize,
    },
    /// A command names a work the table does not hold.
    UnknownWork {
        /// The work it names.
        id: String,
    },
    /// A command starts a work with no men on it.
    NoMen,
    /// A command puts more than [`MAX_CREW`] men on one work.
    TooManyMen {
        /// The men it puts on the work.
        men: u16,
    },
    /// A command restarts a work that is already running or finished.
    AlreadyStarted {
        /// The work it names.
        id: String,
    },
}

impl fmt::Display for CampDayError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, text } => write!(out, "line {line}: not a table line `{text}`"),
            Self::DuplicateSection { line, key } => {
                write!(out, "line {line}: section `[{key}]` appears twice")
            }
            Self::DuplicateKey { line, entry, key } => {
                write!(out, "line {line}: `[{entry}]` holds `{key}` twice")
            }
            Self::UnknownSection { line, key } => {
                write!(
                    out,
                    "line {line}: `[{key}]` is not `[units]` or `[work.<id>]`"
                )
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
                    "`[{entry}]` holds `{key}`, which the camp day does not read"
                )
            }
            Self::MissingValue { entry, key } => {
                write!(out, "`[{entry}]` gives no `{key}`")
            }
            Self::NotAnInteger { line, text } => {
                write!(out, "line {line}: `{text}` is not a whole number")
            }
            Self::OutOfRange { line, entry } => {
                write!(out, "line {line}: `[{entry}]` holds a number out of range")
            }
            Self::WorkCount { found } => {
                write!(out, "the table holds {found} works, not {MAX_WORKS}")
            }
            Self::UnknownWork { id } => write!(out, "no work `{id}` in the table"),
            Self::NoMen => write!(out, "a work needs at least one man"),
            Self::TooManyMen { men } => {
                write!(out, "{men} men on one work, at most {MAX_CREW}")
            }
            Self::AlreadyStarted { id } => write!(out, "work `{id}` is already started"),
        }
    }
}

impl std::error::Error for CampDayError {}

/// One work of the camp day: the prop it raises and what it costs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Work {
    /// The work id as written, for example `ditch`.
    pub id: String,
    /// The prop row it yields when it completes, for example `camp_ditch`.
    pub prop: String,
    /// Cost in man-ticks: one man working one tick spends one.
    pub man_ticks: u32,
}

/// The works table: the rows of `content/rome/works.toml` in file order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorksTable {
    works: Vec<Work>,
    ticks_per_man_hour: u32,
}

impl WorksTable {
    /// The works table shipped with the game.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/works.toml");

    /// Read the shipped works table.
    ///
    /// # Errors
    ///
    /// Same as [`WorksTable::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, CampDayError> {
        Self::parse(Self::SHIPPED)
    }

    /// Read one works table.
    ///
    /// # Errors
    ///
    /// Returns the first [`CampDayError`] the text earns: a malformed line, a repeated
    /// key or section, a section that is not `[units]` or `[work.<id>]`, a number with
    /// no `ref`, a missing or unknown key, a non-integer value, or a work count that is
    /// not [`MAX_WORKS`].
    pub fn parse(text: &str) -> Result<Self, CampDayError> {
        let rows = read_rows(text)?;
        let ticks_per_man_hour = read_units(&rows)?;
        let mut works = Vec::with_capacity(MAX_WORKS);
        for row in &rows {
            let Some(id) = row.key.strip_prefix("work.") else {
                continue;
            };
            row.only(&["man_hours", "prop"])?;
            let hours = row.u32("man_hours")?;
            let man_ticks =
                hours
                    .checked_mul(ticks_per_man_hour)
                    .ok_or_else(|| CampDayError::OutOfRange {
                        line: row.line,
                        entry: row.key.to_string(),
                    })?;
            works.push(Work {
                id: id.to_string(),
                prop: row.text_or_err("prop")?.to_string(),
                man_ticks,
            });
        }
        if works.len() != MAX_WORKS {
            return Err(CampDayError::WorkCount { found: works.len() });
        }
        Ok(Self {
            works,
            ticks_per_man_hour,
        })
    }

    /// Every work, in file order.
    #[must_use]
    pub fn works(&self) -> &[Work] {
        &self.works
    }

    /// The work an id names, if the table holds it.
    #[must_use]
    pub fn work(&self, id: &str) -> Option<&Work> {
        self.works.iter().find(|work| work.id == id)
    }

    /// The table's own man-ticks to the hour: every cost turns through it.
    #[must_use]
    pub const fn ticks_per_man_hour(&self) -> u32 {
        self.ticks_per_man_hour
    }

    /// A digest of the loaded costs: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_str("engine-game/camp_day/works/1");
        hasher.write_u32(self.ticks_per_man_hour);
        hasher.write_len32(self.works.len());
        for work in &self.works {
            hasher.write_str(&work.id);
            hasher.write_str(&work.prop);
            hasher.write_u32(work.man_ticks);
        }
        hasher.finish()
    }
}

/// One command of a camp day, so a script replays the same state twice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step<'a> {
    /// Put `men` on the work `work` and let it run.
    Start {
        /// The work id.
        work: &'a str,
        /// The men on it, `1..=MAX_CREW`.
        men: u16,
    },
    /// Run this many fixed ticks.
    Tick(u32),
    /// Hand the tick's finished props to the presented frame.
    Frame,
}

/// The camp day: which works run, the men on them and the props it shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CampDay {
    table: WorksTable,
    left: Vec<u32>,
    crew: Vec<u16>,
    started: Vec<bool>,
    done: Vec<bool>,
    pending: Vec<usize>,
    props: Vec<String>,
    ticks: u64,
    frames: u32,
}

impl CampDay {
    /// A camp day over `table` with no work started.
    #[must_use]
    pub fn new(table: WorksTable) -> Self {
        let count = table.works.len();
        Self {
            table,
            left: vec![0; count],
            crew: vec![0; count],
            started: vec![false; count],
            done: vec![false; count],
            pending: Vec::new(),
            props: Vec::new(),
            ticks: 0,
            frames: 0,
        }
    }

    /// Put `men` on the work `id`; its man-ticks start ticking down on the next tick.
    ///
    /// # Errors
    ///
    /// Refuses an id the table does not hold, a crew of zero or over [`MAX_CREW`], and a
    /// work that is already started or finished.
    pub fn start(&mut self, id: &str, men: u16) -> Result<(), CampDayError> {
        let Some(index) = self.index(id) else {
            return Err(CampDayError::UnknownWork { id: id.to_string() });
        };
        if men == 0 {
            return Err(CampDayError::NoMen);
        }
        if men > MAX_CREW {
            return Err(CampDayError::TooManyMen { men });
        }
        if self.started[index] {
            return Err(CampDayError::AlreadyStarted { id: id.to_string() });
        }
        self.started[index] = true;
        self.crew[index] = men;
        self.left[index] = self.table.works[index].man_ticks;
        Ok(())
    }

    /// One fixed tick: every running work spends its crew's man-ticks.
    pub fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
        for index in 0..self.left.len() {
            if !self.started[index] || self.done[index] {
                continue;
            }
            let spend = u32::from(self.crew[index]).min(self.left[index]);
            self.left[index] -= spend;
            if self.left[index] == 0 {
                self.done[index] = true;
                self.pending.push(index);
            }
        }
    }

    /// One presented frame: the props finished since the last frame enter the show.
    ///
    /// Returns the props that appeared this frame, in completion order.
    pub fn frame(&mut self) -> Vec<String> {
        self.frames = self.frames.saturating_add(1);
        let mut shown = Vec::with_capacity(self.pending.len());
        for index in self.pending.drain(..) {
            let prop = self.table.works[index].prop.clone();
            self.props.push(prop.clone());
            shown.push(prop);
        }
        shown
    }

    /// The man-ticks still owed on `id`, or `None` for an unknown id.
    #[must_use]
    pub fn man_ticks_left(&self, id: &str) -> Option<u32> {
        self.index(id).map(|index| self.left[index])
    }

    /// Whether `id` has spent its last man-tick and waits for the frame.
    #[must_use]
    pub fn is_done(&self, id: &str) -> bool {
        self.index(id).is_some_and(|index| self.done[index])
    }

    /// The props finished since the last frame: they appear on that frame.
    #[must_use]
    pub fn props(&self) -> &[String] {
        &self.props
    }

    /// Fixed ticks run so far.
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Presented frames so far.
    #[must_use]
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// A digest of the whole state: the same commands twice give the same number.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_str("engine-game/camp_day/state/1");
        hasher.write_u64(self.table.hash());
        hasher.write_u64(self.ticks);
        hasher.write_u32(self.frames);
        for index in 0..self.left.len() {
            hasher.write_u32(self.left[index]);
            hasher.write_u16(self.crew[index]);
            hasher.write_bool(self.started[index]);
            hasher.write_bool(self.done[index]);
        }
        hasher.write_len32(self.pending.len());
        for index in &self.pending {
            hasher.write_u32(u32::try_from(*index).unwrap_or(u32::MAX));
        }
        hasher.write_len32(self.props.len());
        for prop in &self.props {
            hasher.write_str(prop);
        }
        hasher.finish()
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.table.works.iter().position(|work| work.id == id)
    }
}

/// Run a command script over `table`; the same script twice gives the same day.
///
/// # Errors
///
/// The first [`CampDayError`] a [`Step::Start`] earns.
pub fn play(table: WorksTable, script: &[Step<'_>]) -> Result<CampDay, CampDayError> {
    let mut day = CampDay::new(table);
    for step in script {
        match *step {
            Step::Start { work, men } => day.start(work, men)?,
            Step::Tick(count) => {
                for _ in 0..count {
                    day.tick();
                }
            }
            Step::Frame => {
                day.frame();
            }
        }
    }
    Ok(day)
}

/// One value written in a table row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Value<'a> {
    /// A whole number as written.
    Int(u64),
    /// A quoted string without its quotes.
    Text(&'a str),
}

/// One section of the works table: its key, its line and its values.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row<'a> {
    key: &'a str,
    line: usize,
    values: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Row<'a> {
    fn int(&self, key: &str) -> Option<u64> {
        self.values.iter().find_map(|(name, value)| match value {
            Value::Int(number) if *name == key => Some(*number),
            _ => None,
        })
    }

    fn text(&self, key: &str) -> Option<&'a str> {
        self.values.iter().find_map(|(name, value)| match value {
            Value::Text(text) if *name == key => Some(*text),
            _ => None,
        })
    }

    /// The whole number `key`, refused when absent or past `u32`.
    fn u32(&self, key: &str) -> Result<u32, CampDayError> {
        let number = self.int(key).ok_or_else(|| CampDayError::MissingValue {
            entry: self.key.to_string(),
            key: key.to_string(),
        })?;
        u32::try_from(number).map_err(|_| CampDayError::OutOfRange {
            line: self.line,
            entry: self.key.to_string(),
        })
    }

    /// The text `key`, refused when absent.
    fn text_or_err(&self, key: &str) -> Result<&'a str, CampDayError> {
        self.text(key).ok_or_else(|| CampDayError::MissingValue {
            entry: self.key.to_string(),
            key: key.to_string(),
        })
    }

    /// Refuse a key this row's owner does not read; `ref` is always allowed.
    fn only(&self, allowed: &[&str]) -> Result<(), CampDayError> {
        for (key, _) in &self.values {
            if *key == "ref" || allowed.contains(key) {
                continue;
            }
            return Err(CampDayError::UnknownKey {
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
fn read_rows(text: &str) -> Result<Vec<Row<'_>>, CampDayError> {
    let mut rows: Vec<Row<'_>> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if let Some(key) = header(body, line)? {
            if rows.iter().any(|row| row.key == key) {
                return Err(CampDayError::DuplicateSection {
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
            return Err(CampDayError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        let Some((key, value)) = body.split_once('=') else {
            return Err(CampDayError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        let key = key.trim();
        if !is_identifier(key) {
            return Err(CampDayError::Syntax {
                line,
                text: body.to_string(),
            });
        }
        if row.values.iter().any(|(name, _)| *name == key) {
            return Err(CampDayError::DuplicateKey {
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
            return Err(CampDayError::MissingRef {
                line: row.line,
                entry: row.key.to_string(),
            });
        }
    }
    Ok(rows)
}

/// The section key of a `[a.b]` header, or `None` when the line is not one.
fn header(body: &str, line: usize) -> Result<Option<&str>, CampDayError> {
    let Some(rest) = body.strip_prefix('[') else {
        return Ok(None);
    };
    let Some(inner) = rest.strip_suffix(']') else {
        return Err(CampDayError::Syntax {
            line,
            text: body.to_string(),
        });
    };
    let valid =
        !inner.is_empty() && !inner.contains(['[', ']']) && inner.split('.').all(is_identifier);
    if !valid {
        return Err(CampDayError::Syntax {
            line,
            text: body.to_string(),
        });
    }
    Ok(Some(inner))
}

/// One `key = value` line's value: a whole number or a quoted string.
fn value_of<'a>(text: &'a str, line: usize, body: &str) -> Result<Value<'a>, CampDayError> {
    if let Some(rest) = text.strip_prefix('"') {
        let Some(inner) = rest.strip_suffix('"') else {
            return Err(CampDayError::Syntax {
                line,
                text: body.to_string(),
            });
        };
        if inner.contains('"') || inner.contains('\\') {
            return Err(CampDayError::Syntax {
                line,
                text: body.to_string(),
            });
        }
        return Ok(Value::Text(inner));
    }
    whole(text, line, body).map(Value::Int)
}

/// A whole number: digits only, at most nineteen of them.
fn whole(text: &str, line: usize, body: &str) -> Result<u64, CampDayError> {
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CampDayError::NotAnInteger {
            line,
            text: body.to_string(),
        });
    }
    text.parse().map_err(|_| CampDayError::NotAnInteger {
        line,
        text: body.to_string(),
    })
}

/// The `[units]` row: man-ticks to the hour, and the only row with no work id.
fn read_units(rows: &[Row<'_>]) -> Result<u32, CampDayError> {
    let row =
        rows.iter()
            .find(|row| row.key == "units")
            .ok_or_else(|| CampDayError::MissingValue {
                entry: "units".to_string(),
                key: "ticks_per_man_hour".to_string(),
            })?;
    row.only(&["ticks_per_man_hour"])?;
    let ticks = row.u32("ticks_per_man_hour")?;
    if ticks == 0 {
        return Err(CampDayError::OutOfRange {
            line: row.line,
            entry: row.key.to_string(),
        });
    }
    for row in rows {
        if row.key != "units" && !row.key.starts_with("work.") {
            return Err(CampDayError::UnknownSection {
                line: row.line,
                key: row.key.to_string(),
            });
        }
    }
    Ok(ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> WorksTable {
        WorksTable::shipped().expect("the shipped works table reads")
    }

    #[test]
    fn shipped_table_prices_eight_works() {
        let table = table();
        assert_eq!(table.works().len(), MAX_WORKS);
        assert_eq!(table.ticks_per_man_hour(), MAN_TICKS_PER_HOUR);
        for work in table.works() {
            assert!(work.man_ticks > 0, "{} costs nothing", work.id);
            assert!(!work.prop.is_empty(), "{} raises no prop", work.id);
            assert!(work.man_ticks.is_multiple_of(MAN_TICKS_PER_HOUR));
        }
        assert_eq!(
            table.hash(),
            WorksTable::shipped().expect("reads again").hash()
        );
        assert!(table.work("ditch").is_some());
    }

    #[test]
    fn started_work_ticks_down_and_completes() {
        let table = table();
        let cost = table.work("tents").expect("tents").man_ticks;
        let mut day = CampDay::new(table);
        day.start("tents", MAX_CREW).expect("eight men pitch tents");
        assert_eq!(day.man_ticks_left("tents"), Some(cost));
        day.tick();
        assert_eq!(
            day.man_ticks_left("tents"),
            Some(cost - u32::from(MAX_CREW))
        );
        assert!(!day.is_done("tents"));
        let mut ticks = 1;
        while !day.is_done("tents") {
            day.tick();
            ticks += 1;
        }
        assert_eq!(ticks, cost / u32::from(MAX_CREW));
        assert_eq!(day.man_ticks_left("tents"), Some(0));
        assert_eq!(day.ticks(), u64::from(cost / u32::from(MAX_CREW)));
    }

    #[test]
    fn prop_appears_in_the_next_frame() {
        let table = table();
        let mut day = CampDay::new(table);
        day.start("tents", MAX_CREW).expect("eight men pitch tents");
        while !day.is_done("tents") {
            day.tick();
        }
        // The finishing tick hands the prop over; it is not shown yet.
        assert!(day.props().is_empty());
        assert_eq!(day.frames(), 0);
        let shown = day.frame();
        assert_eq!(shown, vec!["tent_row".to_string()]);
        assert_eq!(day.props(), ["tent_row".to_string()]);
        assert_eq!(day.frames(), 1);
        // The next frame repeats nothing: the prop was handed over once.
        assert!(day.frame().is_empty());
        assert_eq!(day.props().len(), 1);
    }

    #[test]
    fn more_hands_finish_sooner() {
        let table = table();
        let eight = play(
            table.clone(),
            &[
                Step::Start {
                    work: "well",
                    men: 8,
                },
                Step::Tick(108_000),
            ],
        )
        .expect("eight men dig the well");
        let four = play(
            table,
            &[
                Step::Start {
                    work: "well",
                    men: 4,
                },
                Step::Tick(108_000),
            ],
        )
        .expect("four men dig the well");
        assert!(eight.is_done("well"));
        assert!(!four.is_done("well"));
        assert_eq!(four.man_ticks_left("well"), Some(432_000));
    }

    #[test]
    fn replay_run_twice_is_equal() {
        let table = table();
        let script = [
            Step::Start {
                work: "tents",
                men: 8,
            },
            Step::Tick(30_000),
            Step::Frame,
            Step::Start {
                work: "gate",
                men: 4,
            },
            Step::Tick(120_000),
            Step::Frame,
            Step::Start {
                work: "ditch",
                men: 8,
            },
            Step::Tick(5_000),
        ];
        let first = play(table.clone(), &script).expect("the script runs");
        let second = play(table, &script).expect("the script runs again");
        assert_eq!(first, second);
        assert_eq!(first.hash(), second.hash());
        assert_eq!(
            first.props(),
            ["tent_row".to_string(), "camp_gate".to_string()]
        );
        assert_eq!(first.frames(), 2);
        assert!(!first.is_done("ditch"));
        assert!(first.man_ticks_left("ditch").expect("ditch") > 0);
    }

    #[test]
    fn bad_commands_are_refused() {
        let mut day = CampDay::new(table());
        assert_eq!(
            day.start("wall", 8),
            Err(CampDayError::UnknownWork {
                id: "wall".to_string()
            })
        );
        assert_eq!(day.start("gate", 0), Err(CampDayError::NoMen));
        assert_eq!(
            day.start("gate", MAX_CREW + 1),
            Err(CampDayError::TooManyMen { men: MAX_CREW + 1 })
        );
        day.start("gate", 2).expect("two men raise the gate");
        assert_eq!(
            day.start("gate", 2),
            Err(CampDayError::AlreadyStarted {
                id: "gate".to_string()
            })
        );
    }

    #[test]
    fn parser_refuses_bad_tables() {
        let unsourced = WorksTable::parse(
            "[units]\nticks_per_man_hour = 216000\n[work.ditch]\nman_hours = 8\nprop = \"d\"\n",
        );
        assert_eq!(
            unsourced,
            Err(CampDayError::MissingRef {
                line: 1,
                entry: "units".to_string()
            })
        );
        let short = WorksTable::parse("[units]\nticks_per_man_hour = 216000\nref = \"x\"\n");
        assert_eq!(short, Err(CampDayError::WorkCount { found: 0 }));
        let float = WorksTable::parse("[units]\nticks_per_man_hour = 1.5\nref = \"x\"\n");
        assert_eq!(
            float,
            Err(CampDayError::NotAnInteger {
                line: 2,
                text: "ticks_per_man_hour = 1.5".to_string()
            })
        );
    }
}
