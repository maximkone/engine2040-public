//! BG14: the lesson rows `tools/memory/memory.mjs` keeps, read and appended
//! behind the door.
//!
//! Owner: `engine-endpoint`; consumers: the `e2040://memory` resource
//! ([`crate::resources`]) and the `memory.record` verb
//! ([`crate::packs::memory`]). The data lives in one JSONL file, the store the
//! tool already writes: `tools/memory/memory.jsonl`. The tool and this module
//! apply the same rules to it, so a row has one shape, one id and one order in
//! both readers, and the door adds no second store.
//!
//! A row is one **decision**, one **failed fix** or one **tuned value**, and it
//! carries the tape hash (or golden) it was taken under, the repo-relative
//! source path it is about and a UTC stamp. The hash is the point: a lesson
//! that cannot be re-checked under a named hash is refused here exactly as the
//! tool refuses it.
//!
//! Units: hex digits for the hash, chars for the text, rows for the read bound,
//! bytes for a store line. Determinism: an id is derived from the row's own
//! canonical fields, never from a clock or an address, so one row always
//! carries one id; rows order by UTC stamp then id, and a read of the same file
//! answers the same bytes. `memory.record` takes its stamp from the caller
//! because the door keeps no wall clock; the CLI defaults to the machine clock
//! when `--utc` is omitted, which is the one place the two writers differ.
//!
//! Errors: [`StoreError`], one named variant per refusal, every message naming
//! the legal shape.

use std::path::{Path, PathBuf};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json;

/// The three kinds a lesson row may carry, in the order the tool lists them.
pub const KINDS: [&str; 3] = ["decision", "failed-fix", "tuned-value"];
/// Hex digits a tape hash (or golden) carries; `0x` may prefix it on input.
pub const HASH_HEX_LEN: usize = 16;
/// Hex digits one row id carries: the head of SHA-256 over the canonical join.
pub const ID_HEX_LEN: usize = 12;
/// Chars in the UTC stamp a row carries: `YYYY-MM-DDTHH:MM:SSZ`.
pub const STAMP_CHARS: usize = 20;
/// Most rows one read takes from a store. A store past this is a bug in the
/// writer, and a door read never walks an unbounded file.
pub const MAX_STORE_ROWS: usize = 4096;
/// Bytes one store read takes. A lesson file past this is not a memory lane;
/// it is a log somebody pointed the door at.
pub const MAX_STORE_CHARS: usize = 256 * 1024;
/// The store's path from the repo root, the file `tools/memory/memory.mjs`
/// writes by default.
pub const DEFAULT_STORE_REL: &str = "tools/memory/memory.jsonl";

/// One lesson row, in the field order the tool writes: `kind`, `hash`, `path`,
/// `utc`, `text`, then `evidence` when the row carries one, then its `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct Record {
    /// One of [`KINDS`].
    pub kind: String,
    /// The tape hash (or golden) the lesson was taken under, lowercase, 16 hex.
    pub hash: String,
    /// The source path the lesson is about, repo-relative, `/`-separated.
    pub path: String,
    /// The UTC stamp the row was taken at.
    pub utc: String,
    /// The lesson itself.
    pub text: String,
    /// The run id or commit that proves it, when the writer named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// Derived id: the head of SHA-256 over the canonical field join.
    pub id: String,
}

impl Record {
    /// The row as one store line: canonical JSON, no spaces, no trailing
    /// newline. Byte-identical to what `memory.mjs record` appends.
    #[must_use]
    pub fn line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Why a row, a line or a store refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The kind is not one of [`KINDS`].
    BadKind {
        /// The kind that was given.
        kind: String,
    },
    /// The hash is missing or is not 16 hex digits.
    BadHash {
        /// The hash that was given.
        hash: String,
    },
    /// The path is empty, absolute or escapes the repo.
    BadPath {
        /// The path that was given.
        path: String,
    },
    /// The text is empty.
    EmptyText,
    /// The stamp is not `YYYY-MM-DDTHH:MM:SSZ`, or a field is out of range.
    BadStamp {
        /// The stamp that was given.
        utc: String,
    },
    /// One store line is not a row: bad JSON, a missing field or a refusal.
    Line {
        /// The 1-based line number.
        line: usize,
        /// What the line failed.
        reason: String,
    },
    /// The store holds more rows than [`MAX_STORE_ROWS`].
    TooRows {
        /// How many rows were read.
        rows: usize,
    },
    /// The store file is bigger than [`MAX_STORE_CHARS`].
    TooBig {
        /// The path that was read.
        path: String,
        /// How many bytes the file holds.
        bytes: u64,
    },
    /// The file could not be read or written.
    Io {
        /// The path that failed.
        path: String,
        /// What the OS said.
        reason: String,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadKind { kind } => write!(
                f,
                "kind `{kind}` is not a lesson kind; legal kinds are {}",
                KINDS.join(", ")
            ),
            Self::BadHash { hash } => write!(
                f,
                "hash `{hash}` is not a tape hash; recovery: pass the 16 hex digits \
                 of the tape digest or the golden it was learned under"
            ),
            Self::BadPath { path } => write!(
                f,
                "path `{path}` is not a repo-relative source path; recovery: pass a \
                 `/`-separated path under the repo root, such as crates/anim/src/motion_bake.rs"
            ),
            Self::EmptyText => write!(f, "the row has no text; a lesson names what it learned"),
            Self::BadStamp { utc } => write!(
                f,
                "utc `{utc}` is not a stamp; recovery: pass YYYY-MM-DDTHH:MM:SSZ in UTC"
            ),
            Self::Line { line, reason } => {
                write!(f, "store line {line} is not a row: {reason}")
            }
            Self::TooRows { rows } => write!(
                f,
                "the store holds {rows} rows, over the {MAX_STORE_ROWS} read bound"
            ),
            Self::TooBig { path, bytes } => write!(
                f,
                "store `{path}` holds {bytes} bytes, over the {MAX_STORE_CHARS} read bound"
            ),
            Self::Io { path, reason } => {
                write!(f, "store `{path}` could not be read or written: {reason}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// True for the three lesson kinds.
#[must_use]
pub fn is_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// The tape hash a row carries: 16 hex digits, `0x` optional, lowercase out —
/// the same normalization `memory.mjs` applies, so `0xD57B…` and `d57b…` name
/// one row.
///
/// # Errors
/// [`StoreError::BadHash`] when the value is not exactly 16 hex digits.
pub fn normalize_hash(hash: &str) -> Result<String, StoreError> {
    let trimmed = hash.trim();
    let body = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let lowered = body.to_ascii_lowercase();
    if lowered.len() != HASH_HEX_LEN || !lowered.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return Err(StoreError::BadHash {
            hash: trimmed.to_owned(),
        });
    }
    Ok(lowered)
}

/// The source path a row carries, `/`-separated and inside the repo.
///
/// # Errors
/// [`StoreError::BadPath`] for an empty, absolute, drive-qualified or
/// parent-escaping path.
pub fn normalize_path(path: &str) -> Result<String, StoreError> {
    let trimmed = path.trim().replace('\\', "/");
    let bad = trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.starts_with("../")
        || trimmed == ".."
        || trimmed.chars().nth(1) == Some(':');
    if bad {
        return Err(StoreError::BadPath {
            path: path.to_owned(),
        });
    }
    Ok(trimmed)
}

/// True for `YYYY-MM-DDTHH:MM:SSZ` with every field in range.
///
/// The shape is what the store orders by, so it is checked by hand: four
/// digits, a dash, two, a dash, two, `T`, then three pairs of digits split by
/// colons and closed by `Z`.
#[must_use]
pub fn is_stamp(utc: &str) -> bool {
    let bytes = utc.as_bytes();
    if bytes.len() != STAMP_CHARS {
        return false;
    }
    if !(bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z')
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        if !bytes[range.clone()].iter().all(u8::is_ascii_digit) {
            return None;
        }
        utc[range].parse::<u32>().ok()
    };
    matches!(number(0..4), Some(year) if year > 0)
        && matches!(number(5..7), Some(1..=12))
        && matches!(number(8..10), Some(1..=31))
        && matches!(number(11..13), Some(0..=23))
        && matches!(number(14..16), Some(0..=59))
        && matches!(number(17..19), Some(0..=59))
}

/// The id of one row: the head of SHA-256 over the canonical field join, the
/// same join `memory.mjs` hashes, so one row has one name in both readers.
#[must_use]
pub fn row_id(kind: &str, hash: &str, path: &str, utc: &str, text: &str, evidence: &str) -> String {
    let canonical = format!("{kind}\0{hash}\0{path}\0{utc}\0{text}\0{evidence}");
    let digest = engine_game::camera_feed::showcase_sha256(canonical.as_bytes());
    let mut hex = crate::play::hex_bytes(&digest);
    hex.truncate(ID_HEX_LEN);
    hex
}

/// Validate one draft row and return it with its derived id.
///
/// Every field is normalized the way the tool normalizes it: the hash loses an
/// `0x` and lower-cases, the path loses backslashes, the text and an evidence
/// tag lose their outer spaces.
///
/// # Errors
/// [`StoreError`] for the first field that fails, naming the legal shape.
pub fn build(
    kind: &str,
    hash: &str,
    path: &str,
    utc: &str,
    text: &str,
    evidence: Option<&str>,
) -> Result<Record, StoreError> {
    if !is_kind(kind) {
        return Err(StoreError::BadKind {
            kind: kind.to_owned(),
        });
    }
    let hash = normalize_hash(hash)?;
    let path = normalize_path(path)?;
    if text.trim().is_empty() {
        return Err(StoreError::EmptyText);
    }
    if !is_stamp(utc) {
        return Err(StoreError::BadStamp {
            utc: utc.to_owned(),
        });
    }
    let evidence = evidence
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned);
    let text = text.trim().to_owned();
    let id = row_id(
        kind,
        &hash,
        &path,
        utc,
        &text,
        evidence.as_deref().unwrap_or(""),
    );
    Ok(Record {
        kind: kind.to_owned(),
        hash,
        path,
        utc: utc.to_owned(),
        text,
        evidence,
        id,
    })
}

/// Every row in JSONL text, in store order, each re-validated so a hand-edited
/// line answers with the id its fields derive.
///
/// # Errors
/// [`StoreError::Line`] for the first line that is not a row, and
/// [`StoreError::TooRows`] past [`MAX_STORE_ROWS`].
pub fn parse(text: &str, where_: &str) -> Result<Vec<Record>, StoreError> {
    let mut rows: Vec<Record> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(trimmed).map_err(|error| StoreError::Line {
                line: index + 1,
                reason: format!("{where_}: not JSON ({error})"),
            })?;
        let field = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let evidence = value.get("evidence").and_then(serde_json::Value::as_str);
        let record = build(
            &field("kind"),
            &field("hash"),
            &field("path"),
            &field("utc"),
            &field("text"),
            evidence,
        )
        .map_err(|reason| StoreError::Line {
            line: index + 1,
            reason: reason.to_string(),
        })?;
        rows.push(record);
        if rows.len() > MAX_STORE_ROWS {
            return Err(StoreError::TooRows { rows: rows.len() });
        }
    }
    Ok(rows)
}

/// Rows in the store's own order: UTC stamp, then id. Never a clock.
#[must_use]
pub fn stamp_order(rows: &[Record]) -> Vec<Record> {
    let mut sorted = rows.to_vec();
    sorted.sort_by(|a, b| a.utc.cmp(&b.utc).then_with(|| a.id.cmp(&b.id)));
    sorted
}

/// The three filters a `?` query carries: `kind`, `path` (a substring) and
/// `hash`. An absent field filters nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    /// The kind a row must equal.
    pub kind: Option<String>,
    /// The substring a row's path must contain.
    pub path: Option<String>,
    /// The hash a row must equal.
    pub hash: Option<String>,
}

impl Filter {
    /// True when nothing is asked for, so every row passes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kind.is_none() && self.path.is_none() && self.hash.is_none()
    }

    /// The rows this filter keeps, in the order they were given.
    #[must_use]
    pub fn select(&self, rows: &[Record]) -> Vec<Record> {
        rows.iter()
            .filter(|row| self.kind.as_deref().is_none_or(|kind| row.kind == kind))
            .filter(|row| {
                self.path
                    .as_deref()
                    .is_none_or(|needle| row.path.contains(needle))
            })
            .filter(|row| self.hash.as_deref().is_none_or(|hash| row.hash == hash))
            .cloned()
            .collect()
    }

    /// The query as one line, in contract order, or the word for all rows.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "all rows".to_owned();
        }
        let mut parts: Vec<String> = Vec::new();
        for (key, value) in [
            ("kind", self.kind.as_deref()),
            ("path", self.path.as_deref()),
            ("hash", self.hash.as_deref()),
        ] {
            if let Some(value) = value {
                parts.push(format!("{key}={value}"));
            }
        }
        parts.join("&")
    }
}

/// The store path a door read or write goes to: [`DEFAULT_STORE_REL`] under the
/// repo root this crate was built in.
#[must_use]
pub fn store_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(DEFAULT_STORE_REL)
}

/// Every row in the store file; a missing store is an empty memory, not a
/// refusal.
///
/// # Errors
/// [`StoreError`] for an unreadable file, a file past [`MAX_STORE_CHARS`] or a
/// line that is not a row.
pub fn read_rows(store: &Path) -> Result<Vec<Record>, StoreError> {
    if !store.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(store).map_err(|error| StoreError::Io {
        path: store.display().to_string(),
        reason: error.to_string(),
    })?;
    if text.len() > MAX_STORE_CHARS {
        return Err(StoreError::TooBig {
            path: store.display().to_string(),
            bytes: u64::try_from(text.len()).unwrap_or(u64::MAX),
        });
    }
    parse(
        &text,
        store
            .file_name()
            .map_or("store", |name| name.to_str().unwrap_or("store")),
    )
}

/// The store's rows in stamp order.
///
/// # Errors
/// As [`read_rows`].
pub fn read_ordered(store: &Path) -> Result<Vec<Record>, StoreError> {
    Ok(stamp_order(&read_rows(store)?))
}

/// Append one validated row to the store: one JSON line, one newline, never a
/// rewrite of the rows already there.
///
/// A row whose derived id already sits in the store is refused, so one lesson
/// recorded twice stays one line.
///
/// # Errors
/// [`StoreError::Io`] for an unwritable path, [`StoreError::Line`] when the
/// store holds a line that is not a row.
pub fn append(store: &Path, row: &Record) -> Result<(), StoreError> {
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent).map_err(|error| StoreError::Io {
            path: parent.display().to_string(),
            reason: error.to_string(),
        })?;
    }
    for held in read_rows(store)? {
        if held.id == row.id {
            return Ok(());
        }
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store)
        .map_err(|error| StoreError::Io {
            path: store.display().to_string(),
            reason: error.to_string(),
        })
        .and_then(|mut file| {
            use std::io::Write as _;
            writeln!(file, "{}", row.line()).map_err(|error| StoreError::Io {
                path: store.display().to_string(),
                reason: error.to_string(),
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden_row(kind: &str, path: &str, utc: &str, text: &str) -> Record {
        build(kind, "d57b5c8ae3096614", path, utc, text, Some("294738a77"))
            .expect("a well-shaped row builds")
    }

    #[test]
    fn a_row_normalizes_and_derives_the_tools_id() {
        // The store's own first row, fed through the door's rules: the id the
        // door derives must equal the id `memory.mjs` wrote.
        let line = std::fs::read_to_string(store_path())
            .expect("the shipped store reads")
            .lines()
            .next()
            .expect("the shipped store is not empty")
            .to_owned();
        let value: serde_json::Value = serde_json::from_str(&line).expect("the row is JSON");
        let door = build(
            value["kind"].as_str().expect("kind"),
            value["hash"].as_str().expect("hash"),
            value["path"].as_str().expect("path"),
            value["utc"].as_str().expect("utc"),
            value["text"].as_str().expect("text"),
            value.get("evidence").and_then(serde_json::Value::as_str),
        )
        .expect("the shipped row is a row");
        assert_eq!(
            door.id,
            value["id"].as_str().expect("the row carries an id"),
            "one row, one id in both readers"
        );
        assert_eq!(
            door.line().trim_end_matches(['\n']),
            line.trim_end_matches(['\n'])
        );

        // The same fields with an `0x` and an upper-case hash are the same row.
        let loud = build(
            "decision",
            "0xD57B5C8AE3096614",
            "crates/game/src/bin/arena.rs",
            "2026-10-23T09:40:00Z",
            "x",
            None,
        )
        .expect("the tool accepts an 0x upper-case hash");
        assert_eq!(loud.hash, "d57b5c8ae3096614");

        // A backslash path is the same row as its `/` form, and the text is
        // trimmed: what the tool stores is what the door reads.
        let a = golden_row(
            "decision",
            "crates\\anim\\src\\x.rs",
            "2026-10-23T09:40:00Z",
            " padded ",
        );
        let b = golden_row(
            "decision",
            "crates/anim/src/x.rs",
            "2026-10-23T09:40:00Z",
            "padded",
        );
        assert_eq!(a.path, "crates/anim/src/x.rs");
        assert_eq!(a.id, b.id, "the id is over normalized fields");
    }

    #[test]
    fn a_row_without_a_tape_hash_is_refused() {
        for hash in [
            "",
            "   ",
            "d57b5c8ae309661",
            "d57b5c8ae30966144",
            "zzzzzzzzzzzzzzzz",
        ] {
            let error = build(
                "decision",
                hash,
                "crates/anim/src/x.rs",
                "2026-10-23T09:40:00Z",
                "a lesson",
                None,
            )
            .expect_err("a bad hash refuses");
            assert!(
                matches!(error, StoreError::BadHash { .. }),
                "{hash:?} is not a tape hash: {error}"
            );
            assert!(error.to_string().contains("16 hex"), "{error}");
        }
    }

    #[test]
    fn the_other_fields_refuse_by_name() {
        let kinds = [
            build(
                "rumour",
                "d57b5c8ae3096614",
                "a.rs",
                "2026-10-23T09:40:00Z",
                "x",
                None,
            ),
            build(
                "decision",
                "d57b5c8ae3096614",
                "/abs/a.rs",
                "2026-10-23T09:40:00Z",
                "x",
                None,
            ),
            build(
                "decision",
                "d57b5c8ae3096614",
                "../a.rs",
                "2026-10-23T09:40:00Z",
                "x",
                None,
            ),
            build(
                "decision",
                "d57b5c8ae3096614",
                "",
                "2026-10-23T09:40:00Z",
                "x",
                None,
            ),
            build(
                "decision",
                "d57b5c8ae3096614",
                "a.rs",
                "2026-10-23T09:40:00",
                "x",
                None,
            ),
            build(
                "decision",
                "d57b5c8ae3096614",
                "a.rs",
                "2026-13-23T09:40:00Z",
                "x",
                None,
            ),
            build(
                "decision",
                "d57b5c8ae3096614",
                "a.rs",
                "2026-10-23T09:40:00Z",
                "   ",
                None,
            ),
        ];
        let codes: Vec<String> = kinds
            .iter()
            .map(|outcome| match outcome {
                Err(error) => error.to_string(),
                Ok(row) => format!("accepted {}", row.id),
            })
            .collect();
        for (index, message) in codes.iter().enumerate() {
            assert!(
                message.starts_with("kind `")
                    || message.starts_with("path `")
                    || message.starts_with("utc `")
                    || message.starts_with("the row has no text"),
                "case {index} refused by name: {message}"
            );
        }
        assert!(
            is_stamp("2026-02-29T00:00:00Z"),
            "a stamp shape is a shape, not a calendar"
        );
    }

    #[test]
    fn store_text_parses_in_line_order_and_orders_by_stamp() {
        let text = concat!(
            "{\"kind\":\"tuned-value\",\"hash\":\"d57b5c8ae3096614\",\"path\":\"b.rs\",\
             \"utc\":\"2026-10-24T00:00:00Z\",\"text\":\"later\",\"id\":\"aaaaaaaaaaaa\"}\n",
            "\n",
            "{\"kind\":\"decision\",\"hash\":\"d57b5c8ae3096614\",\"path\":\"a.rs\",\
             \"utc\":\"2026-10-23T00:00:00Z\",\"text\":\"earlier\"}\n",
        );
        let rows = parse(text, "fixture.jsonl").expect("both lines are rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "later", "parse keeps store order");
        let ordered = stamp_order(&rows);
        assert_eq!(ordered[0].text, "earlier", "stamp order is the read order");
        assert!(
            parse("{not json}\n", "fixture.jsonl").is_err(),
            "a bad line refuses by number"
        );
        let bad = parse(
            "{\"kind\":\"decision\",\"hash\":\"nope\",\"path\":\"a.rs\",\
             \"utc\":\"2026-10-23T00:00:00Z\",\"text\":\"x\"}\n",
            "fixture.jsonl",
        )
        .expect_err("a row with no tape hash refuses the store");
        assert!(
            matches!(bad, StoreError::Line { line: 1, .. }),
            "the refusal names the line: {bad}"
        );
    }

    #[test]
    fn filters_answer_the_rows_the_tool_would_answer() {
        let rows = [
            golden_row(
                "decision",
                "crates/anim/src/a.rs",
                "2026-10-23T09:00:00Z",
                "one",
            ),
            golden_row(
                "failed-fix",
                "crates/anim/src/b.rs",
                "2026-10-23T10:00:00Z",
                "two",
            ),
            golden_row(
                "tuned-value",
                "crates/game/src/c.rs",
                "2026-10-23T11:00:00Z",
                "three",
            ),
        ];
        let all = Filter::default();
        assert!(all.is_empty());
        assert_eq!(all.select(&rows).len(), 3);
        assert_eq!(all.describe(), "all rows");
        let anim = Filter {
            path: Some("crates/anim".to_owned()),
            ..Filter::default()
        };
        assert_eq!(anim.select(&rows).len(), 2);
        let kind = Filter {
            kind: Some("failed-fix".to_owned()),
            ..Filter::default()
        };
        assert_eq!(kind.select(&rows).len(), 1);
        let other = Filter {
            hash: Some("0000000000000000".to_owned()),
            ..Filter::default()
        };
        assert!(other.select(&rows).is_empty());
        let both = Filter {
            kind: Some("decision".to_owned()),
            path: Some("crates/anim".to_owned()),
            hash: Some("d57b5c8ae3096614".to_owned()),
        };
        assert_eq!(both.select(&rows).len(), 1);
        assert_eq!(
            both.describe(),
            "kind=decision&path=crates/anim&hash=d57b5c8ae3096614"
        );
    }

    #[test]
    fn append_writes_one_line_and_a_second_write_of_a_row_adds_none() {
        let dir = std::env::temp_dir().join(format!(
            "e2040-memory-store-{}-{}",
            std::process::id(),
            line!()
        ));
        let store = dir.join("lessons.jsonl");
        let row = golden_row(
            "decision",
            "crates/anim/src/x.rs",
            "2026-10-23T09:40:00Z",
            "padded",
        );
        assert!(read_rows(&store)
            .expect("a missing store is empty")
            .is_empty());
        append(&store, &row).expect("the row appends");
        append(&store, &row).expect("the same row appends once");
        append(
            &store,
            &golden_row(
                "failed-fix",
                "crates/anim/src/y.rs",
                "2026-10-23T10:40:00Z",
                "next",
            ),
        )
        .expect("a second row appends");
        let text = std::fs::read_to_string(&store).expect("the store reads");
        assert_eq!(text.lines().count(), 2, "{text}");
        assert_eq!(
            parse(&text, "lessons.jsonl")
                .expect("both lines read")
                .len(),
            2
        );
        assert!(text.contains(&row.line()), "the line is the canonical JSON");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_store_past_the_byte_bound_refuses_before_a_line_parses() {
        let dir = std::env::temp_dir().join(format!(
            "e2040-memory-store-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("the temp dir");
        let store = dir.join("big.jsonl");
        let line = golden_row(
            "decision",
            "crates/anim/src/x.rs",
            "2026-10-23T09:40:00Z",
            "padded",
        )
        .line();
        let one = format!("{line}\n");
        // Enough lines to cross the byte bound while staying under the row
        // bound, so the size guard is what refuses here.
        let rows = MAX_STORE_CHARS / one.len() + 1;
        assert!(rows < MAX_STORE_ROWS, "the row bound stays open");
        std::fs::write(&store, one.repeat(rows)).expect("the store writes");
        let refused = read_rows(&store).expect_err("a store past the bound refuses");
        assert!(matches!(refused, StoreError::TooBig { .. }), "{refused}");
        assert!(
            refused.to_string().contains("read bound"),
            "the message names the bound: {refused}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
