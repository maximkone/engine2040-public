//! `codex` pack: the Rome codex at the door.
//!
//! Two verbs over the local codex. `codex.search { query, topic?, limit? }`
//! reads the passage index and answers with the passages that match, each
//! with its ref, title, source and the numbers it states. `codex.cite { file,
//! key, ref }` writes a ref into a Rome table entry as one undoable change:
//! the bytes the table held before the cite go on the undo stack, so the same
//! verb with `undo: true` puts them back byte for byte.
//!
//! Scope: both verbs work under one project root (the repo the door was
//! built in, unless the caller names another). The index sits at
//! `<root>/content/local/codex/index.jsonl` and the tables under
//! `<root>/content/rome/`. A table path is relative to the root and may not
//! leave it, so a cite touches a table inside the root and nothing else.
//!
//! Reads never download: the pack reads what the fetch and index tools left
//! on disk. A missing index, a missing table, a key the table does not hold
//! and a ref the index does not know all refuse typed ([`CodexError`]), each
//! message naming what was missing and one recovery.
//!
//! Units: chars for the index and the tables, counts for passages and
//! numbers. Determinism: a passage is ranked by the query words it holds and
//! then by its place in the index, so the same index always answers the same
//! way.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "codex";
/// Verb: search the passage index.
pub const TOOL_CODEX_SEARCH: &str = "codex.search";
/// Verb: cite a passage in a Rome table entry (undoable).
pub const TOOL_CODEX_CITE: &str = "codex.cite";

/// The topics the passage index tags, in the order the index lists them.
pub const TOPICS: [&str; 9] = [
    "march", "camp", "tool", "bridge", "road", "siege", "craft", "place", "year",
];

/// Passages a search answers when the caller names no limit.
pub const SEARCH_DEFAULT_LIMIT: u32 = 8;
/// Most passages a search may answer.
pub const SEARCH_MAX_LIMIT: u32 = 32;
/// Most cites one session keeps undoable per table file.
pub const MAX_UNDO_RECORDS: usize = 32;
/// The passage index, under the project root.
pub const INDEX_RELATIVE_PATH: &str = "content/local/codex/index.jsonl";
/// The Rome tables, under the project root.
pub const ROME_RELATIVE_PATH: &str = "content/rome";

/// Why a codex verb refused. Every variant names what was missing and one
/// recovery, so a weak model retries without a second round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexError {
    /// The passage index file is not there.
    MissingIndex {
        /// The index path that was looked for.
        path: String,
    },
    /// The passage index file cannot be read.
    UnreadableIndex {
        /// The index path.
        path: String,
        /// Why the read failed.
        reason: String,
    },
    /// A line of the index is not a passage.
    BadIndexLine {
        /// The index path.
        path: String,
        /// The line number, counted from one.
        line: usize,
        /// Why the line did not parse.
        reason: String,
    },
    /// The Rome table file is not there.
    MissingTable {
        /// The table path that was looked for.
        path: String,
    },
    /// The Rome table file cannot be read.
    UnreadableTable {
        /// The table path.
        path: String,
        /// Why the read failed.
        reason: String,
    },
    /// The table holds no entry with that key.
    MissingKey {
        /// The table path.
        path: String,
        /// The key the caller named.
        key: String,
    },
    /// The entry holds a `ref` that is not a quoted string.
    BadRefLine {
        /// The table path.
        path: String,
        /// The key the caller named.
        key: String,
    },
    /// The index knows no passage with that ref.
    UnknownRef {
        /// The ref the caller named.
        reference: String,
    },
    /// The topic is not one of the nine the index tags.
    BadTopic {
        /// The topic the caller named.
        topic: String,
    },
    /// The limit is outside the legal range.
    BadLimit {
        /// The limit the caller named.
        limit: u32,
    },
    /// The query holds no word to look for.
    EmptyQuery,
    /// The table path is not a relative path inside the project root.
    BadPath {
        /// The path the caller named.
        path: String,
    },
    /// No cite of that table is left to undo.
    NothingToUndo {
        /// The table path.
        path: String,
    },
    /// The table cannot be written.
    WriteFailed {
        /// The table path.
        path: String,
        /// Why the write failed.
        reason: String,
    },
}

impl CodexError {
    /// The machine code of the refusal.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingIndex { .. } => "missing_index",
            Self::UnreadableIndex { .. } => "unreadable_index",
            Self::BadIndexLine { .. } => "bad_index_line",
            Self::MissingTable { .. } => "missing_table",
            Self::UnreadableTable { .. } => "unreadable_table",
            Self::MissingKey { .. } => "missing_key",
            Self::BadRefLine { .. } => "bad_ref_line",
            Self::UnknownRef { .. } => "unknown_ref",
            Self::BadTopic { .. } => "bad_topic",
            Self::BadLimit { .. } => "bad_limit",
            Self::EmptyQuery => "empty_query",
            Self::BadPath { .. } => "bad_path",
            Self::NothingToUndo { .. } => "nothing_to_undo",
            Self::WriteFailed { .. } => "write_failed",
        }
    }
}

impl fmt::Display for CodexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIndex { path } => write!(
                f,
                "the passage index {path} is missing; re-run the index tool to write it"
            ),
            Self::UnreadableIndex { path, reason } => write!(
                f,
                "the passage index {path} cannot be read ({reason}); re-run after the file is readable"
            ),
            Self::BadIndexLine {
                path,
                line,
                reason,
            } => write!(
                f,
                "line {line} of the passage index {path} is not a passage ({reason}); \
                 re-run the index tool to rebuild it"
            ),
            Self::MissingTable { path } => write!(
                f,
                "the Rome table {path} is missing; re-run with a table the project root holds"
            ),
            Self::UnreadableTable { path, reason } => write!(
                f,
                "the Rome table {path} cannot be read ({reason}); re-run after the file is readable"
            ),
            Self::MissingKey { path, key } => write!(
                f,
                "the Rome table {path} holds no entry `{key}`; re-run with a key the table lists"
            ),
            Self::BadRefLine { path, key } => write!(
                f,
                "the ref of `{key}` in {path} is not a quoted string; re-run after the table is fixed"
            ),
            Self::UnknownRef { reference } => write!(
                f,
                "the passage index knows no ref `{reference}`; re-run codex.search for the ref to cite"
            ),
            Self::BadTopic { topic } => write!(
                f,
                "`{topic}` is not a codex topic; legal topics: [{}]; re-run with one of those",
                TOPICS.join(", ")
            ),
            Self::BadLimit { limit } => write!(
                f,
                "limit {limit} is outside 1..={SEARCH_MAX_LIMIT}; re-run with a limit inside that range"
            ),
            Self::EmptyQuery => {
                write!(f, "the query holds no word; re-run with the words to look for")
            }
            Self::BadPath { path } => write!(
                f,
                "`{path}` is not a relative .toml path inside the project root; \
                 re-run with a path the root holds"
            ),
            Self::NothingToUndo { path } => write!(
                f,
                "no cite of {path} is left to undo; re-run codex.cite to make one"
            ),
            Self::WriteFailed { path, reason } => write!(
                f,
                "the Rome table {path} cannot be written ({reason}); re-run after the folder is writable"
            ),
        }
    }
}

impl std::error::Error for CodexError {}

impl From<CodexError> for McpToolError {
    /// The refusal keeps its machine code and its recovery message.
    fn from(error: CodexError) -> Self {
        McpToolError::new(error.code(), error.to_string())
    }
}

/// Args for `codex.search`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchArgs {
    /// The words to look for; the passage ref, title and text are searched.
    pub query: String,
    /// Topic filter: march, camp, tool, bridge, road, siege, craft, place, year.
    #[serde(default)]
    pub topic: Option<String>,
    /// Most passages to answer (1..=32, default 8).
    #[serde(default)]
    pub limit: Option<u32>,
    /// The project root that holds the codex (default: the repo the door was
    /// built in).
    #[serde(default)]
    pub root: Option<String>,
}

/// One number a passage states, with its unit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NumberOut {
    /// The number as the passage states it.
    pub value: f64,
    /// The unit it carries (ft, mi, men, year, ...).
    pub unit: String,
    /// The words as they stand in the passage.
    pub text: String,
}

/// One passage the index holds.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PassageOut {
    /// The passage ref (`vegetius.3.8`).
    pub reference: String,
    /// The work the passage belongs to.
    pub work: String,
    /// The source id the passage came from.
    pub source: String,
    /// The title of the work.
    pub title: String,
    /// The author of the work.
    pub author: String,
    /// The topics the passage holds.
    pub topics: Vec<String>,
    /// The numbers with units the passage states.
    pub numbers: Vec<NumberOut>,
    /// The passage text.
    pub text: String,
    /// The characters in the passage text.
    pub chars: u64,
    /// How many query words the passage holds.
    pub hits: u32,
}

/// Result of `codex.search`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchOutput {
    /// The query that ran.
    pub query: String,
    /// The topic filter, when one was named.
    pub topic: Option<String>,
    /// The index file that was read.
    pub index: String,
    /// The passages that match, best first.
    pub passages: Vec<PassageOut>,
    /// How many passages matched before the limit cut the tail.
    pub matched: u64,
    /// True when the limit cut the tail.
    pub truncated: bool,
}

/// Args for `codex.cite`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CiteArgs {
    /// The Rome table file, relative to the project root
    /// (`content/rome/camp.toml`).
    pub file: String,
    /// The entry key in the table (`ditch.one_night`).
    pub key: String,
    /// The codex ref to attach (`vegetius.3.8`).
    #[serde(rename = "ref")]
    pub reference: String,
    /// True undoes the newest cite of this table instead of citing.
    #[serde(default)]
    pub undo: Option<bool>,
    /// The project root that holds the codex (default: the repo the door was
    /// built in).
    #[serde(default)]
    pub root: Option<String>,
}

/// Result of `codex.cite`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CiteOutput {
    /// The table file the cite touched.
    pub file: String,
    /// The entry key.
    pub key: String,
    /// The ref the entry holds now.
    pub reference: String,
    /// True when the cite was undone.
    pub undone: bool,
    /// True when the file bytes moved.
    pub changed: bool,
    /// The bytes of the table after the change.
    pub bytes: u64,
    /// The cites of this table that are still undoable.
    pub undoable: u32,
}

/// One number of a passage line in the index.
#[derive(Debug, Clone, Deserialize)]
#[serde(crate = "rmcp::serde")]
struct IndexNumber {
    value: f64,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    text: String,
}

/// One passage line of the index.
#[derive(Debug, Clone, Deserialize)]
#[serde(crate = "rmcp::serde")]
struct IndexPassage {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    work: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    numbers: Vec<IndexNumber>,
    #[serde(default)]
    text: String,
}

/// The project root the codex lives under: the one the caller names, or the
/// repo the door was built in.
pub(super) fn project_root(root: Option<&str>) -> PathBuf {
    match root {
        Some(name) if !name.trim().is_empty() => PathBuf::from(name),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
    }
}

/// The passage index under `root`. A missing file refuses typed, never an
/// empty answer.
fn read_index(root: &Path) -> Result<Vec<IndexPassage>, CodexError> {
    let path = root.join(INDEX_RELATIVE_PATH);
    let text = std::fs::read_to_string(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => CodexError::MissingIndex {
            path: path.display().to_string(),
        },
        _ => CodexError::UnreadableIndex {
            path: path.display().to_string(),
            reason: error.to_string(),
        },
    })?;
    let mut passages = Vec::new();
    for (at, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let passage: IndexPassage =
            rmcp::serde_json::from_str(line).map_err(|error| CodexError::BadIndexLine {
                path: path.display().to_string(),
                line: at + 1,
                reason: error.to_string(),
            })?;
        passages.push(passage);
    }
    Ok(passages)
}

/// The words of a query, lowercased and without empties.
fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

/// How many query words a passage holds: the ref, the title and the text are
/// searched, lowercased.
fn hit_count(passage: &IndexPassage, terms: &[String]) -> u32 {
    let haystack =
        format!("{} {} {}", passage.reference, passage.title, passage.text).to_lowercase();
    let mut hits = 0;
    for term in terms {
        if haystack.contains(term.as_str()) {
            hits += 1;
        }
    }
    hits
}

/// The topic a caller named, checked against the nine the index tags.
fn checked_topic(topic: &Option<String>) -> Result<Option<String>, CodexError> {
    let Some(name) = topic else {
        return Ok(None);
    };
    let trimmed = name.trim().to_lowercase();
    if TOPICS.contains(&trimmed.as_str()) {
        Ok(Some(trimmed))
    } else {
        Err(CodexError::BadTopic {
            topic: name.clone(),
        })
    }
}

/// The passages that match, best first: the ones holding the most query
/// words lead, and the place in the index settles a tie.
fn rank(index: &[IndexPassage], terms: &[String], topic: Option<&str>) -> Vec<(u32, usize)> {
    let mut hits: Vec<(u32, usize)> = Vec::new();
    for (at, passage) in index.iter().enumerate() {
        if let Some(topic) = topic {
            if !passage.topics.iter().any(|held| held == topic) {
                continue;
            }
        }
        let count = hit_count(passage, terms);
        if count == 0 {
            continue;
        }
        hits.push((count, at));
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits
}

/// One passage as the search answers it.
fn passage_out(passage: &IndexPassage, hits: u32) -> PassageOut {
    let numbers = passage
        .numbers
        .iter()
        .map(|number| NumberOut {
            value: number.value,
            unit: number.unit.clone(),
            text: number.text.clone(),
        })
        .collect();
    PassageOut {
        reference: passage.reference.clone(),
        work: passage.work.clone(),
        source: passage.source.clone(),
        title: passage.title.clone(),
        author: passage.author.clone(),
        topics: passage.topics.clone(),
        numbers,
        text: passage.text.clone(),
        chars: u64::try_from(passage.text.len()).unwrap_or(u64::MAX),
        hits,
    }
}

/// The table file a cite may write: a relative path inside the project root
/// that names a `.toml` file. A path that climbs out of the root refuses.
fn table_path(root: &Path, file: &str) -> Result<PathBuf, CodexError> {
    let relative = Path::new(file);
    let climbs = relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)));
    let is_toml = relative.extension().and_then(|ext| ext.to_str()) == Some("toml");
    if file.trim().is_empty() || relative.is_absolute() || climbs || !is_toml {
        return Err(CodexError::BadPath {
            path: file.to_owned(),
        });
    }
    Ok(root.join(relative))
}

/// The table text split into lines that keep their line ends, so an edit
/// touches only the bytes it must.
fn split_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// True when a line opens a table.
fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// The entry a key names: the header line and the lines under it.
fn entry_block(lines: &[&str], key: &str) -> Option<(usize, usize)> {
    let header = format!("[{key}]");
    let at = lines.iter().position(|line| line.trim() == header)?;
    let end = lines[at + 1..]
        .iter()
        .position(|line| is_header(line))
        .map(|offset| at + 1 + offset)
        .unwrap_or(lines.len());
    Some((at, end))
}

/// True when a line sets the `ref` key.
fn is_ref_line(line: &str) -> bool {
    let trimmed = line.trim();
    match trimmed.strip_prefix("ref") {
        Some(rest) => rest.trim_start().starts_with('='),
        None => false,
    }
}

/// The quoted value of a `ref` line.
fn ref_value(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let end = line.rfind('"')?;
    if end <= start {
        return None;
    }
    Some(line[start + 1..end].to_owned())
}

/// A quoted TOML string: the quotes plus the escaped value.
fn quoted(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The line end a table uses: the first one it holds, else a newline.
fn line_end(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The value an entry holds after the cite: the old refs plus the new one.
fn cited_value(current: &str, reference: &str) -> String {
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return reference.to_owned();
    }
    let known = trimmed.split(';').any(|part| part.trim() == reference);
    if known {
        return trimmed.to_owned();
    }
    format!("{trimmed}; {reference}")
}

/// The table text with the ref attached to the entry: the new text and
/// whether the bytes moved. The entry must exist and its `ref` must be a
/// quoted string.
fn cited_text(
    path: &str,
    text: &str,
    key: &str,
    reference: &str,
) -> Result<(String, bool), CodexError> {
    let lines = split_lines(text);
    let Some((start, end)) = entry_block(&lines, key) else {
        return Err(CodexError::MissingKey {
            path: path.to_owned(),
            key: key.to_owned(),
        });
    };
    let eol = line_end(text);
    let existing = (start + 1..end).find(|at| is_ref_line(lines[*at]));
    let mut out: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
    match existing {
        Some(at) => {
            let current = ref_value(lines[at]).ok_or_else(|| CodexError::BadRefLine {
                path: path.to_owned(),
                key: key.to_owned(),
            })?;
            let value = cited_value(&current, reference);
            if value == current.trim() {
                return Ok((text.to_owned(), false));
            }
            let ending = if lines[at].ends_with('\n') { eol } else { "" };
            out[at] = format!("ref = {}{ending}", quoted(&value));
            Ok((out.concat(), true))
        }
        None => {
            let value = cited_value("", reference);
            let at = (start + 1..end)
                .rev()
                .find(|at| !lines[*at].trim().is_empty())
                .map(|at| at + 1)
                .unwrap_or(start + 1);
            if at > 0 && !out[at - 1].ends_with('\n') {
                out[at - 1].push_str(eol);
            }
            out.insert(at, format!("ref = {}{eol}", quoted(&value)));
            Ok((out.concat(), true))
        }
    }
}

/// Write a table file: a temp file in the same folder, then a rename over
/// the old one, so a half-written table never lands.
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), CodexError> {
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "table".to_owned());
    let temp = folder.join(format!("{name}.codex-tmp"));
    let failed = |reason: String| CodexError::WriteFailed {
        path: path.display().to_string(),
        reason,
    };
    std::fs::write(&temp, bytes).map_err(|error| failed(error.to_string()))?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(failed(error.to_string()));
    }
    Ok(())
}

/// One cite that can still be taken back: the table and the bytes it held
/// before the cite.
#[derive(Debug, Clone)]
struct CiteUndo {
    path: PathBuf,
    bytes: Vec<u8>,
}

/// The undo stacks of the sessions: one list per session id, each holding
/// the cites of one table that can still be taken back.
fn undo_stacks() -> &'static Mutex<BTreeMap<u64, Vec<CiteUndo>>> {
    static STACKS: OnceLock<Mutex<BTreeMap<u64, Vec<CiteUndo>>>> = OnceLock::new();
    STACKS.get_or_init(Mutex::default)
}

/// The undo stack table of the sessions, locked. A poisoned lock fails
/// closed.
fn session_stack(
) -> Result<std::sync::MutexGuard<'static, BTreeMap<u64, Vec<CiteUndo>>>, McpToolError> {
    undo_stacks().lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "the codex undo table is poisoned; restart the door",
        )
    })
}

/// The table bytes of a file, read whole.
fn read_table(path: &Path) -> Result<Vec<u8>, CodexError> {
    std::fs::read(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => CodexError::MissingTable {
            path: path.display().to_string(),
        },
        _ => CodexError::UnreadableTable {
            path: path.display().to_string(),
            reason: error.to_string(),
        },
    })
}

/// The text of a table, read whole.
fn read_table_text(path: &Path) -> Result<String, CodexError> {
    let bytes = read_table(path)?;
    String::from_utf8(bytes).map_err(|error| CodexError::UnreadableTable {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

/// `codex.search`: read the passage index and answer with the passages that
/// match the query.
fn search(server: &McpServer, args: &SearchArgs) -> Result<SearchOutput, McpToolError> {
    server.dispatch(TOOL_CODEX_SEARCH, |_state| {
        let root = project_root(args.root.as_deref());
        let topic = checked_topic(&args.topic)?;
        let limit = args.limit.unwrap_or(SEARCH_DEFAULT_LIMIT);
        if limit == 0 || limit > SEARCH_MAX_LIMIT {
            return Err(CodexError::BadLimit { limit }.into());
        }
        let terms = query_terms(&args.query);
        if terms.is_empty() {
            return Err(CodexError::EmptyQuery.into());
        }
        let index = read_index(&root)?;
        let ranked = rank(&index, &terms, topic.as_deref());
        let matched = u64::try_from(ranked.len()).unwrap_or(u64::MAX);
        let mut passages = Vec::new();
        let take = usize::try_from(limit).unwrap_or(usize::MAX);
        for (hits, at) in ranked.iter().take(take) {
            if let Some(passage) = index.get(*at) {
                passages.push(passage_out(passage, *hits));
            }
        }
        Ok(SearchOutput {
            query: args.query.clone(),
            topic,
            index: root.join(INDEX_RELATIVE_PATH).display().to_string(),
            matched,
            truncated: matched > u64::try_from(passages.len()).unwrap_or(u64::MAX),
            passages,
        })
    })
}

/// `codex.cite`: attach a ref to a Rome table entry as one undoable change,
/// or take the newest cite of that table back.
fn cite(server: &McpServer, args: &CiteArgs) -> Result<CiteOutput, McpToolError> {
    server.dispatch(TOOL_CODEX_CITE, |state| {
        let session = state.session();
        let root = project_root(args.root.as_deref());
        let path = table_path(&root, &args.file)?;
        let shown = path.display().to_string();
        if args.undo.unwrap_or(false) {
            return undo_cite(session, &path, &shown, args);
        }
        let index = read_index(&root)?;
        if !index
            .iter()
            .any(|passage| passage.reference == args.reference)
        {
            return Err(CodexError::UnknownRef {
                reference: args.reference.clone(),
            }
            .into());
        }
        let before = read_table(&path)?;
        let text =
            String::from_utf8(before.clone()).map_err(|error| CodexError::UnreadableTable {
                path: shown.clone(),
                reason: error.to_string(),
            })?;
        let (cited, changed) = cited_text(&shown, &text, &args.key, &args.reference)?;
        if changed {
            write_bytes(&path, cited.as_bytes())?;
            push_undo(session, path.clone(), before)?;
        }
        let undoable = undo_depth(session, &path)?;
        Ok(CiteOutput {
            file: args.file.clone(),
            key: args.key.clone(),
            reference: args.reference.clone(),
            undone: false,
            changed,
            bytes: u64::try_from(cited.len()).unwrap_or(u64::MAX),
            undoable,
        })
    })
}

/// Take the newest cite of one table back: the bytes it held before the
/// cite go back on disk, so the file is byte-equal to what it was.
fn undo_cite(
    session: u64,
    path: &Path,
    shown: &str,
    args: &CiteArgs,
) -> Result<CiteOutput, McpToolError> {
    let text = read_table_text(path)?;
    let lines = split_lines(&text);
    if entry_block(&lines, &args.key).is_none() {
        return Err(CodexError::MissingKey {
            path: shown.to_owned(),
            key: args.key.clone(),
        }
        .into());
    }
    let mut stacks = session_stack()?;
    let stack = stacks.entry(session).or_default();
    let at = stack.iter().rposition(|record| record.path == path);
    let Some(at) = at else {
        return Err(CodexError::NothingToUndo {
            path: shown.to_owned(),
        }
        .into());
    };
    let record = stack.remove(at);
    write_bytes(path, &record.bytes)?;
    let undoable = u32::try_from(stack.len()).unwrap_or(u32::MAX);
    Ok(CiteOutput {
        file: args.file.clone(),
        key: args.key.clone(),
        reference: args.reference.clone(),
        undone: true,
        changed: true,
        bytes: u64::try_from(record.bytes.len()).unwrap_or(u64::MAX),
        undoable,
    })
}

/// Keep the bytes a table held before a cite, so the cite can be taken back.
fn push_undo(session: u64, path: PathBuf, bytes: Vec<u8>) -> Result<(), McpToolError> {
    let mut stacks = session_stack()?;
    let stack = stacks.entry(session).or_default();
    stack.push(CiteUndo { path, bytes });
    while stack.len() > MAX_UNDO_RECORDS {
        stack.remove(0);
    }
    Ok(())
}

/// How many cites of one table are still undoable.
fn undo_depth(session: u64, path: &Path) -> Result<u32, McpToolError> {
    let mut stacks = session_stack()?;
    let stack = stacks.entry(session).or_default();
    let count = stack.iter().filter(|record| record.path == path).count();
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

crate::e2_tool!(
    /// `codex.search` tool: the passages that match a query, with their refs.
    SearchTool,
    name = TOOL_CODEX_SEARCH,
    description = "Search the Rome codex passage index: the passages that hold the query words, each with its ref, title, source and the numbers it states. Optional topic filter and limit. Reads the index only, downloads nothing.",
    readonly = true,
    args = SearchArgs,
    output = SearchOutput,
    invoke = search
);

crate::e2_tool!(
    /// `codex.cite` tool: attach a ref to a Rome table entry, undoable.
    CiteTool,
    name = TOOL_CODEX_CITE,
    description = "Attach a codex ref to a Rome table entry (file and key) as one undoable change; undo true puts the table bytes back. The ref must be one the passage index knows.",
    readonly = false,
    args = CiteArgs,
    output = CiteOutput,
    invoke = cite
);

/// The codex pack: the passage index and the Rome table cites, loaded on
/// demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "the Rome codex: search the passage index for the source behind a number, \
         and cite a passage in a Rome table entry; a cite is one undoable change",
        false,
    );
    pack.add(VerbSpec::of::<SearchTool>(
        TOOL_CODEX_SEARCH,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<CiteTool>(
        TOOL_CODEX_CITE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    /// A project root of this test: the fixture index and the fixture table
    /// live under it, so the pack never touches the repo tree.
    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("e2040-codex-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("content/local/codex")).expect("root creates");
        std::fs::create_dir_all(root.join("content/rome")).expect("root creates");
        root
    }

    /// The fixture index: one Vegetius passage that names the camp ditch, one
    /// Caesar passage that names a camp only and one Hyginus passage that
    /// names a ditch only.
    fn write_index(root: &Path) {
        let lines = [
            r#"{"ref":"vegetius.3.8","source":"gutenberg","file":"vegetius.txt","work":"vegetius","book":0,"chapter":3,"section":8,"title":"De Re Militari","author":"Vegetius","url":"https://example.invalid/vegetius.txt","licence":"public domain","topics":["camp"],"numbers":[{"value":5,"unit":"ft","text":"5 ft"},{"value":3,"unit":"ft","text":"3 ft"}],"chars":74,"text":"In every camp the ditch is dug first: five feet wide, three feet deep."}"#,
            r#"{"ref":"caesar.bg.1.2","source":"gutenberg","file":"caesar.txt","work":"caesar.bg","book":1,"chapter":2,"section":0,"title":"De Bello Gallico","author":"Julius Caesar","url":"","licence":"","topics":["march"],"numbers":[],"chars":55,"text":"The army marched on and reached the next camp before dawn."}"#,
            r#"{"ref":"hyginus.49","source":"gutenberg","file":"hyginus.txt","work":"hyginus","book":0,"chapter":49,"section":0,"title":"De Munitionibus Castrorum","author":"Hyginus","url":"","licence":"","topics":["tool"],"numbers":[],"chars":61,"text":"The ditch is measured with a rope before the men dig it."}"#,
        ];
        let text = lines.join("\n") + "\n";
        std::fs::write(root.join(INDEX_RELATIVE_PATH), text).expect("index writes");
    }

    /// The fixture table: one entry with a ref of its own.
    fn write_table(root: &Path) -> PathBuf {
        let path = root.join("content/rome/camp.toml");
        let text = "# Roman camp works.\n\n[ditch.one_night]\nwidth_ft = 5\ndepth_ft = 3\nref = \"rome.md section 5\"\n";
        std::fs::write(&path, text).expect("table writes");
        path
    }

    /// A door over one session.
    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    #[test]
    fn search_returns_the_vegetius_passage_first_for_a_camp_ditch_query() {
        let root = fixture_root("search");
        write_index(&root);
        let door = server(2040);
        let found = SearchTool::invoke(
            &door,
            SearchArgs {
                query: "camp ditch".to_owned(),
                topic: None,
                limit: None,
                root: Some(root.display().to_string()),
            },
        )
        .expect("search serves");
        assert_eq!(found.passages.len(), 3, "every passage with a word answers");
        assert_eq!(
            found.passages[0].reference, "vegetius.3.8",
            "the passage that names both words leads"
        );
        assert_eq!(found.passages[0].hits, 2);
        assert_eq!(found.passages[0].title, "De Re Militari");
        assert_eq!(found.passages[0].source, "gutenberg");
        assert_eq!(found.passages[0].numbers.len(), 2);
        assert_eq!(found.passages[0].numbers[0].unit, "ft");
        assert_eq!(found.passages[0].numbers[0].value, 5.0);
        assert!(found.passages[0].text.contains("ditch"));
        assert_eq!(found.matched, 3);
        assert!(!found.truncated);
        // The topic filter keeps the camp passage alone.
        let camp = SearchTool::invoke(
            &door,
            SearchArgs {
                query: "camp ditch".to_owned(),
                topic: Some("camp".to_owned()),
                limit: Some(1),
                root: Some(root.display().to_string()),
            },
        )
        .expect("search serves");
        assert_eq!(camp.passages.len(), 1);
        assert_eq!(camp.passages[0].reference, "vegetius.3.8");
        assert_eq!(camp.matched, 1);
        assert!(
            !camp.truncated,
            "one passage matched, the limit cut nothing"
        );
        // The limit cuts the tail and says so.
        let one = SearchTool::invoke(
            &door,
            SearchArgs {
                query: "camp ditch".to_owned(),
                topic: None,
                limit: Some(1),
                root: Some(root.display().to_string()),
            },
        )
        .expect("search serves");
        assert_eq!(one.passages.len(), 1);
        assert_eq!(one.matched, 3);
        assert!(one.truncated, "the limit cut the tail");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cite_then_its_undo_leaves_the_table_byte_equal() {
        let root = fixture_root("cite");
        write_index(&root);
        let path = write_table(&root);
        let before = std::fs::read(&path).expect("table reads");
        let door = server(2041);
        let cited = CiteTool::invoke(
            &door,
            CiteArgs {
                file: "content/rome/camp.toml".to_owned(),
                key: "ditch.one_night".to_owned(),
                reference: "vegetius.3.8".to_owned(),
                undo: None,
                root: Some(root.display().to_string()),
            },
        )
        .expect("cite serves");
        assert!(cited.changed);
        assert!(!cited.undone);
        assert_eq!(cited.undoable, 1);
        let after = std::fs::read(&path).expect("table reads");
        assert_ne!(before, after, "the cite moved the bytes");
        let text = String::from_utf8(after.clone()).expect("table is text");
        assert!(
            text.contains("ref = \"rome.md section 5; vegetius.3.8\""),
            "the ref is attached: {text}"
        );
        // The undo puts the bytes back exactly.
        let undone = CiteTool::invoke(
            &door,
            CiteArgs {
                file: "content/rome/camp.toml".to_owned(),
                key: "ditch.one_night".to_owned(),
                reference: "vegetius.3.8".to_owned(),
                undo: Some(true),
                root: Some(root.display().to_string()),
            },
        )
        .expect("undo serves");
        assert!(undone.undone);
        assert_eq!(undone.undoable, 0);
        let restored = std::fs::read(&path).expect("table reads");
        assert_eq!(restored, before, "the undo is byte-equal to the start");
        // Nothing left to undo: the refusal is typed.
        let empty = CiteTool::invoke(
            &door,
            CiteArgs {
                file: "content/rome/camp.toml".to_owned(),
                key: "ditch.one_night".to_owned(),
                reference: "vegetius.3.8".to_owned(),
                undo: Some(true),
                root: Some(root.display().to_string()),
            },
        )
        .expect_err("nothing is left to undo");
        assert_eq!(empty.code, "nothing_to_undo");
        assert!(empty.message.contains("re-run"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_index_is_a_typed_refusal() {
        let root = fixture_root("missing-index");
        let door = server(2042);
        let refused = SearchTool::invoke(
            &door,
            SearchArgs {
                query: "camp ditch".to_owned(),
                topic: None,
                limit: None,
                root: Some(root.display().to_string()),
            },
        )
        .expect_err("a missing index refuses");
        assert_eq!(refused.code, "missing_index");
        assert!(refused.message.contains("index.jsonl"));
        assert!(refused.message.contains("re-run"));
        // A cite without an index refuses the same way, and writes nothing.
        let path = write_table(&root);
        let before = std::fs::read(&path).expect("table reads");
        let cite_refused = CiteTool::invoke(
            &door,
            CiteArgs {
                file: "content/rome/camp.toml".to_owned(),
                key: "ditch.one_night".to_owned(),
                reference: "vegetius.3.8".to_owned(),
                undo: None,
                root: Some(root.display().to_string()),
            },
        )
        .expect_err("a missing index refuses the cite");
        assert_eq!(cite_refused.code, "missing_index");
        assert_eq!(
            std::fs::read(&path).expect("table reads"),
            before,
            "the refusal wrote nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_table_a_missing_key_and_an_unknown_ref_refuse_typed() {
        let root = fixture_root("refusals");
        write_index(&root);
        write_table(&root);
        let door = server(2043);
        let cite = |key: &str, reference: &str, file: &str| {
            CiteTool::invoke(
                &door,
                CiteArgs {
                    file: file.to_owned(),
                    key: key.to_owned(),
                    reference: reference.to_owned(),
                    undo: None,
                    root: Some(root.display().to_string()),
                },
            )
        };
        // A table the root does not hold.
        let missing_table = cite("ditch.one_night", "vegetius.3.8", "content/rome/nope.toml")
            .expect_err("a missing table refuses");
        assert_eq!(missing_table.code, "missing_table");
        assert!(missing_table.message.contains("re-run"));
        // A key the table does not hold.
        let missing_key = cite(
            "ditch.nine_nights",
            "vegetius.3.8",
            "content/rome/camp.toml",
        )
        .expect_err("a missing key refuses");
        assert_eq!(missing_key.code, "missing_key");
        assert!(missing_key.message.contains("ditch.nine_nights"));
        // A ref the index does not know.
        let unknown_ref = cite("ditch.one_night", "nobody.1.1", "content/rome/camp.toml")
            .expect_err("an unknown ref refuses");
        assert_eq!(unknown_ref.code, "unknown_ref");
        assert!(unknown_ref.message.contains("nobody.1.1"));
        // A path that climbs out of the root.
        let bad_path = cite("ditch.one_night", "vegetius.3.8", "../outside/camp.toml")
            .expect_err("a path out of the root refuses");
        assert_eq!(bad_path.code, "bad_path");
        // A topic the index does not tag.
        let bad_topic = SearchTool::invoke(
            &door,
            SearchArgs {
                query: "camp".to_owned(),
                topic: Some("rampart".to_owned()),
                limit: None,
                root: Some(root.display().to_string()),
            },
        )
        .expect_err("a bad topic refuses");
        assert_eq!(bad_topic.code, "bad_topic");
        assert!(bad_topic.message.contains("camp"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_pack_registers_its_verbs_with_annotations_and_schemas() {
        let pack = pack().expect("the codex pack registers");
        assert_eq!(pack.name(), "codex");
        let names = pack.verb_names();
        assert!(names.contains(&TOOL_CODEX_SEARCH));
        assert!(names.contains(&TOOL_CODEX_CITE));
        for verb in pack.verbs() {
            assert_ne!(verb.card(), CapabilityCard::Deny);
            let model = verb.model();
            assert!(model.annotations.is_some(), "{} is annotated", verb.name());
            assert!(
                model.output_schema.is_some(),
                "{} has an output",
                verb.name()
            );
        }
        assert!(super::super::PackRegistry::shipped().errors().is_empty());
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 20,
    name: PACK,
    default_loaded: false,
    tools: &["codex.search", "codex.cite"],
    build: pack,
};
