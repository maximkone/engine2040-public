//! dialogue: scripted talk compiled offline to validated baked tables.
//!
//! Consumer: game ludus voices step baked lines and barks with no model in
//! reach; writers and AI draft the text offline and the bake refuses broken
//! refs and dead branches with source line numbers. Data owner: this module
//! owns [`CompiledDialogue`] tables and [`DialogueBake`] lines; the
//! [`ProvenanceLog`] owns the per-line records (consumed, never mirrored).
//! Units: ticks are fixed steps, texts are `1..=MAX_TEXT` chars. Determinism:
//! source order plus FNV-1a only, no floats, no hash order, no clock. Errors:
//! [`DialogueError`] (source line, `0` means baked bytes or runtime input)
//! and [`RunError`]. Budget: at most [`MAX_NODES`] nodes, [`MAX_FLAGS`]
//! flags, [`MAX_OPTIONS_PER_BRANCH`] options per branch; bake is one-shot and
//! a play walks at most `nodes + 1` steps.

use super::cursor::{ByteCursor, ShortRead};
use super::mind::{ActionSource, MAX_TEXT};
use super::provenance::{PendingRecord, ProvenanceError, ProvenanceLog, ProvenanceRecord, Writer};
use engine_core::{hash_bytes, Fnv64};
use std::collections::BTreeMap;

/// Baked-table layout version, little-endian `u32` first. Unknown versions
/// refuse loudly so a future layout never parses as this one.
pub const DIALOGUE_BYTES_VERSION: u32 = 1;
/// Most nodes (lines, branches and barks together) one script holds.
pub const MAX_NODES: usize = 512;
/// Most condition flags one script declares.
pub const MAX_FLAGS: usize = 64;
/// Most options one branch offers.
pub const MAX_OPTIONS_PER_BRANCH: usize = 8;
/// Longest script, node, speaker or flag name, in characters.
pub const MAX_ID: usize = 64;
/// Most baked lines one table holds: every node plus every option.
pub const MAX_BAKED_LINES: usize = MAX_NODES * (1 + MAX_OPTIONS_PER_BRANCH);
/// Writer id for a script's records: FNV-1a of the script name, so every
/// baked line traces to the script that spoke it.
fn script_writer(name: &str) -> u64 {
    hash_bytes(name.as_bytes())
}

/// Digest chain step: `after = FNV(before || seq || detail)`, so the record
/// order is checkable from the seed alone.
fn chain_after(before: u64, seq: u64, detail: &str) -> u64 {
    let mut hash = Fnv64::new();
    hash.write_u64(before);
    hash.write_u64(seq);
    hash.write_str(detail);
    hash.finish()
}

fn is_ident(text: &str) -> bool {
    if text.is_empty() || text.chars().count() > MAX_ID {
        return false;
    }
    let mut chars = text.chars();
    let first = chars.next().expect("non-empty ident has a first char");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Where one flow step goes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Next {
    /// The talk ends here.
    End,
    /// Continue at the named line or branch node.
    Node(String),
}

/// One branch option: a shown choice with an optional flag condition.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DialogueOption {
    /// Where picking this option continues.
    pub target: Next,
    /// Flag that must be set for this option to be offered (`None` = always).
    pub cond: Option<String>,
    /// Shown choice text (`1..=MAX_TEXT` chars).
    pub text: String,
}

/// How one node flows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Flow {
    /// One spoken line, then the jump.
    Line {
        /// Where the talk continues.
        next: Next,
    },
    /// One prompt with a validated option table.
    Branch {
        /// Options in source order; the first satisfied one plays.
        options: Vec<DialogueOption>,
    },
    /// One ambient line looked up by id; never in the flow.
    Bark,
}

/// One validated script node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    /// Node id, unique per script (`end` is reserved).
    pub id: String,
    /// Voice that speaks this node.
    pub speaker: String,
    /// Spoken text: line, branch prompt or bark (`1..=MAX_TEXT` chars).
    pub text: String,
    /// How the talk flows through this node.
    pub flow: Flow,
}

/// Validated dialogue tables in source order. Reachability from the entry
/// holds by construction; barks are ambient roots looked up by id.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompiledDialogue {
    name: String,
    entry: usize,
    nodes: Vec<Node>,
    index: BTreeMap<String, usize>,
    flags: Vec<String>,
}

impl CompiledDialogue {
    /// Script name from the `script` line.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Entry node id from the `start` line.
    pub fn entry_id(&self) -> &str {
        &self.nodes[self.entry].id
    }

    /// Nodes in source order.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Declared condition flags in source order.
    pub fn flags(&self) -> &[String] {
        &self.flags
    }

    /// Node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Look up a node by id; unknown ids read `None`.
    pub fn find(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|index| &self.nodes[*index])
    }

    /// Compile then bake in one call: one record per baked line stamped at
    /// `bake_tick` under `request`, chained from `digest_seed`.
    pub fn bake(
        &self,
        bake_tick: u64,
        request: u64,
        digest_seed: u64,
    ) -> Result<DialogueBake, ProvenanceError> {
        DialogueBake::bake(self, bake_tick, request, digest_seed)
    }
}

/// Why a script, its bytes or a runtime input was refused. `line` is the
/// 1-based source line; `0` means baked bytes or runtime input, never a
/// source row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DialogueError {
    /// 1-based source line, or `0` for baked bytes and runtime inputs.
    pub line: usize,
    /// What failed.
    pub kind: ErrorKind,
}

/// Typed dialogue failures.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    /// No `script` line in the source.
    MissingScript,
    /// Second `script` line.
    DuplicateScript,
    /// No `start` line in the source.
    MissingStart,
    /// Second `start` line.
    DuplicateStart,
    /// First token names no directive; the word is carried.
    UnknownDirective(String),
    /// A directive has the wrong token shape.
    BadDirective,
    /// A name breaks the ident rule; the word is carried.
    BadName(String),
    /// `end` used as a node id.
    ReservedId,
    /// A node id repeats; the first definition line is carried.
    DuplicateNode {
        /// Source line of the first definition.
        other: usize,
    },
    /// A flag repeats; the first declaration line is carried.
    DuplicateFlag {
        /// Source line of the first declaration.
        other: usize,
    },
    /// A `line`, `branch`, `option` or `bark` line has no `:` text part.
    MissingColon,
    /// A `line` or `option` header has no well-formed `->` jump.
    BadArrow,
    /// Spoken text empty or whitespace-only.
    EmptyText,
    /// Spoken text longer than [`MAX_TEXT`] characters.
    TextTooLong,
    /// A jump names no node; the target is carried.
    UnknownTarget(String),
    /// A jump names a bark, which has no flow; the target is carried.
    TargetIsBark(String),
    /// An option names no node; the branch is carried.
    UnknownBranch(String),
    /// An option names a line or bark, not a branch; the node is carried.
    NotABranch(String),
    /// An option condition names no declared flag; the flag is carried.
    UnknownFlag(String),
    /// A branch carries no options.
    BranchNoOptions,
    /// A branch carries more than [`MAX_OPTIONS_PER_BRANCH`] options.
    TooManyOptions,
    /// The script holds more than [`MAX_NODES`] nodes.
    TooManyNodes,
    /// The script declares more than [`MAX_FLAGS`] flags.
    TooManyFlags,
    /// The `start` line names no node; the target is carried.
    StartUnknown(String),
    /// The `start` line names a bark, which has no flow.
    StartIsBark,
    /// A line or branch node is unreachable from `start`; barks are exempt.
    /// The node id is carried.
    DeadNode(String),
    /// Baked bytes truncated, mistyped, mis-ordered or trailing.
    Corrupt,
    /// Baked bytes carry another layout version.
    UnknownVersion,
}

impl core::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ErrorKind::MissingScript => write!(f, "missing script line"),
            ErrorKind::DuplicateScript => write!(f, "duplicate script line"),
            ErrorKind::MissingStart => write!(f, "missing start line"),
            ErrorKind::DuplicateStart => write!(f, "duplicate start line"),
            ErrorKind::UnknownDirective(word) => write!(f, "unknown directive {word}"),
            ErrorKind::BadDirective => write!(f, "malformed directive"),
            ErrorKind::BadName(word) => write!(f, "bad name {word}"),
            ErrorKind::ReservedId => write!(f, "end is reserved"),
            ErrorKind::DuplicateNode { other } => {
                write!(f, "duplicate node, first at line {other}")
            }
            ErrorKind::DuplicateFlag { other } => {
                write!(f, "duplicate flag, first at line {other}")
            }
            ErrorKind::MissingColon => write!(f, "missing : text part"),
            ErrorKind::BadArrow => write!(f, "malformed -> jump"),
            ErrorKind::EmptyText => write!(f, "empty text"),
            ErrorKind::TextTooLong => write!(f, "text over cap"),
            ErrorKind::UnknownTarget(target) => write!(f, "unknown target {target}"),
            ErrorKind::TargetIsBark(target) => write!(f, "target {target} is a bark"),
            ErrorKind::UnknownBranch(branch) => write!(f, "unknown branch {branch}"),
            ErrorKind::NotABranch(node) => write!(f, "{node} is not a branch"),
            ErrorKind::UnknownFlag(flag) => write!(f, "unknown flag {flag}"),
            ErrorKind::BranchNoOptions => write!(f, "branch has no options"),
            ErrorKind::TooManyOptions => write!(f, "branch over option cap"),
            ErrorKind::TooManyNodes => write!(f, "script over node cap"),
            ErrorKind::TooManyFlags => write!(f, "script over flag cap"),
            ErrorKind::StartUnknown(target) => write!(f, "unknown start {target}"),
            ErrorKind::StartIsBark => write!(f, "start names a bark"),
            ErrorKind::DeadNode(id) => write!(f, "dead node {id}"),
            ErrorKind::Corrupt => write!(f, "corrupt bytes"),
            ErrorKind::UnknownVersion => write!(f, "unknown version"),
        }
    }
}

impl core::fmt::Display for DialogueError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "dialogue:{}: {}", self.line, self.kind)
    }
}

impl std::error::Error for DialogueError {}

fn fail(line: usize, kind: ErrorKind) -> DialogueError {
    DialogueError { line, kind }
}

fn check_text(text: &str, line: usize) -> Result<(), DialogueError> {
    if text.trim().is_empty() {
        return Err(fail(line, ErrorKind::EmptyText));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(fail(line, ErrorKind::TextTooLong));
    }
    Ok(())
}

fn check_ident(word: &str, line: usize) -> Result<(), DialogueError> {
    if !is_ident(word) {
        return Err(fail(line, ErrorKind::BadName(word.to_owned())));
    }
    Ok(())
}

struct RawNode {
    id: String,
    speaker: String,
    text: String,
    flow: RawFlow,
    line: usize,
}

enum RawFlow {
    Line { target: String },
    Branch,
    Bark,
}

struct RawOption {
    branch: String,
    target: String,
    cond: Option<String>,
    text: String,
    line: usize,
}

/// Parse output: script, start, flag, node and option rows in source order.
type ParsedScript = (
    Vec<(String, usize)>,
    Vec<(String, usize)>,
    Vec<(String, usize)>,
    Vec<RawNode>,
    Vec<RawOption>,
);

fn keyword_of(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(cut) => (&line[..cut], line[cut..].trim()),
        None => (line, ""),
    }
}

fn split_text(rest: &str) -> Option<(&str, &str)> {
    let cut = rest.find(':')?;
    Some((rest[..cut].trim(), rest[cut + 1..].trim()))
}

/// Parse one source into raw rows; structural validation runs after, so
/// forward references compile.
fn parse(source: &str) -> Result<ParsedScript, DialogueError> {
    let mut scripts: Vec<(String, usize)> = Vec::new();
    let mut starts: Vec<(String, usize)> = Vec::new();
    let mut flags: Vec<(String, usize)> = Vec::new();
    let mut nodes: Vec<RawNode> = Vec::new();
    let mut options: Vec<RawOption> = Vec::new();
    for (row, raw) in source.lines().enumerate() {
        let lineno = row + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, rest) = keyword_of(line);
        match keyword {
            "script" => {
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return Err(fail(lineno, ErrorKind::BadDirective));
                }
                check_ident(rest, lineno)?;
                scripts.push((rest.to_owned(), lineno));
            }
            "start" => {
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return Err(fail(lineno, ErrorKind::BadDirective));
                }
                check_ident(rest, lineno)?;
                starts.push((rest.to_owned(), lineno));
            }
            "flag" => {
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return Err(fail(lineno, ErrorKind::BadDirective));
                }
                check_ident(rest, lineno)?;
                flags.push((rest.to_owned(), lineno));
            }
            "line" => {
                let (head, text) =
                    split_text(rest).ok_or_else(|| fail(lineno, ErrorKind::MissingColon))?;
                let parts: Vec<&str> = head.split_whitespace().collect();
                if parts.len() != 4 || parts[2] != "->" {
                    return Err(fail(lineno, ErrorKind::BadArrow));
                }
                check_ident(parts[0], lineno)?;
                if parts[0] == "end" {
                    return Err(fail(lineno, ErrorKind::ReservedId));
                }
                check_ident(parts[1], lineno)?;
                if parts[3] != "end" {
                    check_ident(parts[3], lineno)?;
                }
                check_text(text, lineno)?;
                nodes.push(RawNode {
                    id: parts[0].to_owned(),
                    speaker: parts[1].to_owned(),
                    text: text.to_owned(),
                    flow: RawFlow::Line {
                        target: parts[3].to_owned(),
                    },
                    line: lineno,
                });
            }
            "branch" => {
                let (head, text) =
                    split_text(rest).ok_or_else(|| fail(lineno, ErrorKind::MissingColon))?;
                let parts: Vec<&str> = head.split_whitespace().collect();
                if parts.len() != 2 {
                    return Err(fail(lineno, ErrorKind::BadDirective));
                }
                check_ident(parts[0], lineno)?;
                if parts[0] == "end" {
                    return Err(fail(lineno, ErrorKind::ReservedId));
                }
                check_ident(parts[1], lineno)?;
                check_text(text, lineno)?;
                nodes.push(RawNode {
                    id: parts[0].to_owned(),
                    speaker: parts[1].to_owned(),
                    text: text.to_owned(),
                    flow: RawFlow::Branch,
                    line: lineno,
                });
            }
            "option" => {
                let (head, text) =
                    split_text(rest).ok_or_else(|| fail(lineno, ErrorKind::MissingColon))?;
                let parts: Vec<&str> = head.split_whitespace().collect();
                let (branch, target, cond) = match parts.as_slice() {
                    [branch, "->", target] => (*branch, *target, None),
                    [branch, "->", target, "if", flag] => (*branch, *target, Some(*flag)),
                    _ => return Err(fail(lineno, ErrorKind::BadArrow)),
                };
                check_ident(branch, lineno)?;
                if target != "end" {
                    check_ident(target, lineno)?;
                }
                if let Some(flag) = cond {
                    check_ident(flag, lineno)?;
                }
                check_text(text, lineno)?;
                options.push(RawOption {
                    branch: branch.to_owned(),
                    target: target.to_owned(),
                    cond: cond.map(str::to_owned),
                    text: text.to_owned(),
                    line: lineno,
                });
            }
            "bark" => {
                let (head, text) =
                    split_text(rest).ok_or_else(|| fail(lineno, ErrorKind::MissingColon))?;
                let parts: Vec<&str> = head.split_whitespace().collect();
                if parts.len() != 2 {
                    return Err(fail(lineno, ErrorKind::BadDirective));
                }
                check_ident(parts[0], lineno)?;
                if parts[0] == "end" {
                    return Err(fail(lineno, ErrorKind::ReservedId));
                }
                check_ident(parts[1], lineno)?;
                check_text(text, lineno)?;
                nodes.push(RawNode {
                    id: parts[0].to_owned(),
                    speaker: parts[1].to_owned(),
                    text: text.to_owned(),
                    flow: RawFlow::Bark,
                    line: lineno,
                });
            }
            other => return Err(fail(lineno, ErrorKind::UnknownDirective(other.to_owned()))),
        }
    }
    Ok((scripts, starts, flags, nodes, options))
}

/// Validate raw rows into tables. Shared by [`compile`] and baked-bytes
/// loading, so stored tables are as valid as fresh ones.
fn validate(
    scripts: &[(String, usize)],
    starts: &[(String, usize)],
    flags: &[(String, usize)],
    raws: &[RawNode],
    raws_options: &[RawOption],
) -> Result<CompiledDialogue, DialogueError> {
    if scripts.is_empty() {
        return Err(fail(0, ErrorKind::MissingScript));
    }
    if scripts.len() > 1 {
        return Err(fail(scripts[1].1, ErrorKind::DuplicateScript));
    }
    if starts.is_empty() {
        return Err(fail(0, ErrorKind::MissingStart));
    }
    if starts.len() > 1 {
        return Err(fail(starts[1].1, ErrorKind::DuplicateStart));
    }
    if raws.len() > MAX_NODES {
        let line = raws.get(MAX_NODES).map_or(0, |node| node.line);
        return Err(fail(line, ErrorKind::TooManyNodes));
    }
    if flags.len() > MAX_FLAGS {
        let line = flags.get(MAX_FLAGS).map_or(0, |flag| flag.1);
        return Err(fail(line, ErrorKind::TooManyFlags));
    }
    let mut seen_nodes: BTreeMap<&str, usize> = BTreeMap::new();
    for raw in raws {
        if let Some(other) = seen_nodes.insert(raw.id.as_str(), raw.line) {
            return Err(fail(raw.line, ErrorKind::DuplicateNode { other }));
        }
    }
    let mut seen_flags: BTreeMap<&str, usize> = BTreeMap::new();
    for flag in flags {
        if let Some(other) = seen_flags.insert(flag.0.as_str(), flag.1) {
            return Err(fail(flag.1, ErrorKind::DuplicateFlag { other }));
        }
    }
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (position, raw) in raws.iter().enumerate() {
        index.insert(raw.id.clone(), position);
    }
    let is_bark = |id: &str| -> bool {
        index
            .get(id)
            .is_some_and(|position| matches!(raws[*position].flow, RawFlow::Bark))
    };
    let resolve = |target: &str, line: usize| -> Result<(), DialogueError> {
        if target == "end" {
            return Ok(());
        }
        if !index.contains_key(target) {
            return Err(fail(line, ErrorKind::UnknownTarget(target.to_owned())));
        }
        if is_bark(target) {
            return Err(fail(line, ErrorKind::TargetIsBark(target.to_owned())));
        }
        Ok(())
    };
    // The entry exists and carries flow.
    let entry = match index.get(starts[0].0.as_str()) {
        None => {
            return Err(fail(
                starts[0].1,
                ErrorKind::StartUnknown(starts[0].0.clone()),
            ));
        }
        Some(position) => *position,
    };
    if matches!(raws[entry].flow, RawFlow::Bark) {
        return Err(fail(starts[0].1, ErrorKind::StartIsBark));
    }
    for raw in raws {
        if let RawFlow::Line { target } = &raw.flow {
            resolve(target, raw.line)?;
        }
    }
    let mut per_branch: BTreeMap<&str, usize> = BTreeMap::new();
    for option in raws_options {
        let position = match index.get(option.branch.as_str()) {
            None => {
                return Err(fail(
                    option.line,
                    ErrorKind::UnknownBranch(option.branch.clone()),
                ));
            }
            Some(position) => *position,
        };
        if !matches!(raws[position].flow, RawFlow::Branch) {
            return Err(fail(
                option.line,
                ErrorKind::NotABranch(option.branch.clone()),
            ));
        }
        resolve(&option.target, option.line)?;
        if let Some(flag) = &option.cond {
            if !seen_flags.contains_key(flag.as_str()) {
                return Err(fail(option.line, ErrorKind::UnknownFlag(flag.clone())));
            }
        }
        let count = per_branch.entry(option.branch.as_str()).or_insert(0);
        *count += 1;
        if *count > MAX_OPTIONS_PER_BRANCH {
            return Err(fail(option.line, ErrorKind::TooManyOptions));
        }
    }
    for raw in raws {
        if matches!(raw.flow, RawFlow::Branch) && !per_branch.contains_key(raw.id.as_str()) {
            return Err(fail(raw.line, ErrorKind::BranchNoOptions));
        }
    }
    // Reachability from the entry over line jumps and option jumps; barks
    // are ambient roots and stay exempt.
    let mut reachable = vec![false; raws.len()];
    let mut stack = vec![entry];
    reachable[entry] = true;
    while let Some(position) = stack.pop() {
        let mut visit = |id: &str| {
            if id == "end" {
                return;
            }
            if let Some(next) = index.get(id) {
                if !reachable[*next] {
                    reachable[*next] = true;
                    stack.push(*next);
                }
            }
        };
        match &raws[position].flow {
            RawFlow::Line { target } => visit(target),
            RawFlow::Branch => {
                for option in raws_options
                    .iter()
                    .filter(|o| o.branch == raws[position].id)
                {
                    visit(&option.target);
                }
            }
            RawFlow::Bark => {}
        }
    }
    for (position, raw) in raws.iter().enumerate() {
        if !reachable[position] && !matches!(raw.flow, RawFlow::Bark) {
            return Err(fail(raw.line, ErrorKind::DeadNode(raw.id.clone())));
        }
    }
    // Build tables in source order.
    let mut nodes: Vec<Node> = Vec::with_capacity(raws.len());
    for raw in raws {
        let flow = match &raw.flow {
            RawFlow::Line { target } => Flow::Line {
                next: if target == "end" {
                    Next::End
                } else {
                    Next::Node(target.clone())
                },
            },
            RawFlow::Branch => Flow::Branch {
                options: raws_options
                    .iter()
                    .filter(|o| o.branch == raw.id)
                    .map(|o| DialogueOption {
                        target: if o.target == "end" {
                            Next::End
                        } else {
                            Next::Node(o.target.clone())
                        },
                        cond: o.cond.clone(),
                        text: o.text.clone(),
                    })
                    .collect(),
            },
            RawFlow::Bark => Flow::Bark,
        };
        nodes.push(Node {
            id: raw.id.clone(),
            speaker: raw.speaker.clone(),
            text: raw.text.clone(),
            flow,
        });
    }
    let mut table = BTreeMap::new();
    for (position, node) in nodes.iter().enumerate() {
        table.insert(node.id.clone(), position);
    }
    Ok(CompiledDialogue {
        name: scripts[0].0.clone(),
        entry,
        nodes,
        index: table,
        flags: flags.iter().map(|flag| flag.0.clone()).collect(),
    })
}

/// Compile dialogue source to validated tables. Writers and AI draft this
/// text offline; the game only ever steps baked tables, never a model.
///
/// ```text
/// script greeting
/// start hello
/// flag cheered
/// line hello trainer -> pick: Welcome to the yard.
/// branch pick crowd: How do they answer?
/// option pick -> cheer if cheered: They roar for you!
/// option pick -> quiet: Silence falls.
/// line cheer crowd -> end: The crowd roars!
/// line quiet crowd -> end: Only the wind answers.
/// bark forge blacksmith: Steel sings today.
/// ```
pub fn compile(source: &str) -> Result<CompiledDialogue, DialogueError> {
    let (scripts, starts, flags, nodes, options) = parse(source)?;
    validate(&scripts, &starts, &flags, &nodes, &options)
}

/// What one baked line is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum BakedKind {
    /// A `line` node body.
    Line,
    /// A `branch` prompt.
    Prompt,
    /// One branch option choice.
    Choice,
    /// A `bark` body.
    Bark,
}

/// One baked line: spoken text plus its provenance sequence. The matching
/// [`ProvenanceRecord`] lives in the bake's [`ProvenanceLog`] at the same
/// sequence.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BakedLine {
    /// Node this line was baked from.
    pub node: String,
    /// What this line is.
    pub kind: BakedKind,
    /// Voice that speaks this line.
    pub speaker: String,
    /// Spoken text (`1..=MAX_TEXT` chars).
    pub text: String,
    /// Provenance sequence of this line's record.
    pub seq: u64,
}

/// One played step: spoken text plus its provenance sequence for the receipt.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Spoken {
    /// Node this step played.
    pub node: String,
    /// Voice that spoke.
    pub speaker: String,
    /// Spoken text.
    pub text: String,
    /// Provenance sequence of this line's record.
    pub seq: u64,
}

/// Why a model-free play was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RunError {
    /// No bark carries this id; the id is carried.
    UnknownBark(String),
    /// No option of this branch holds under the flags; the branch is carried.
    NoOption {
        /// Branch with no satisfied option.
        branch: String,
    },
    /// The walk outgrew the table: a cycle. Steps taken are carried.
    Loop {
        /// Steps walked before refusal.
        steps: usize,
    },
    /// A runtime flag breaks the ident rule; the word is carried.
    BadFlag(String),
    /// More than [`MAX_FLAGS`] runtime flags were passed.
    TooManyFlags,
    /// A mutated table points nowhere; the node is carried.
    BrokenTable(String),
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RunError::UnknownBark(id) => write!(f, "dialogue: unknown bark {id}"),
            RunError::NoOption { branch } => {
                write!(f, "dialogue: branch {branch} has no satisfied option")
            }
            RunError::Loop { steps } => write!(f, "dialogue: loop after {steps} steps"),
            RunError::BadFlag(word) => write!(f, "dialogue: bad flag {word}"),
            RunError::TooManyFlags => write!(f, "dialogue: too many flags"),
            RunError::BrokenTable(node) => write!(f, "dialogue: broken table at {node}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Validated tables baked to deterministic bytes with one provenance record per
/// baked line. The game steps this with [`DialogueBake::play`] and
/// [`DialogueBake::speak_bark`]: no model, no provider, only tables.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DialogueBake {
    compiled: CompiledDialogue,
    lines: Vec<BakedLine>,
    solo_seq: BTreeMap<String, u64>,
    prompt_seq: BTreeMap<String, u64>,
    choice_seq: BTreeMap<(String, usize), u64>,
    log: ProvenanceLog,
    bake_tick: u64,
    request: u64,
    writer_id: u64,
    digest_seed: u64,
}

impl DialogueBake {
    fn bake(
        compiled: &CompiledDialogue,
        bake_tick: u64,
        request: u64,
        digest_seed: u64,
    ) -> Result<Self, ProvenanceError> {
        let writer_id = script_writer(&compiled.name);
        let mut log = ProvenanceLog::new();
        let mut lines: Vec<BakedLine> = Vec::new();
        let mut solo_seq: BTreeMap<String, u64> = BTreeMap::new();
        let mut prompt_seq: BTreeMap<String, u64> = BTreeMap::new();
        let mut choice_seq: BTreeMap<(String, usize), u64> = BTreeMap::new();
        let mut before = digest_seed;
        let push = |log: &mut ProvenanceLog,
                    lines: &mut Vec<BakedLine>,
                    before: &mut u64,
                    node: &str,
                    kind: BakedKind,
                    speaker: &str,
                    text: &str|
         -> Result<u64, ProvenanceError> {
            let seq = lines.len() as u64;
            let after = chain_after(*before, seq, text);
            let recorded = log.record(PendingRecord::external(
                bake_tick,
                ActionSource::Scripted,
                request,
                writer_id,
                *before,
                after,
                text,
            ))?;
            debug_assert_eq!(recorded, seq, "bake records in line order");
            lines.push(BakedLine {
                node: node.to_owned(),
                kind,
                speaker: speaker.to_owned(),
                text: text.to_owned(),
                seq,
            });
            *before = after;
            Ok(seq)
        };
        for node in &compiled.nodes {
            match &node.flow {
                Flow::Line { .. } => {
                    let seq = push(
                        &mut log,
                        &mut lines,
                        &mut before,
                        &node.id,
                        BakedKind::Line,
                        &node.speaker,
                        &node.text,
                    )?;
                    solo_seq.insert(node.id.clone(), seq);
                }
                Flow::Branch { options } => {
                    let seq = push(
                        &mut log,
                        &mut lines,
                        &mut before,
                        &node.id,
                        BakedKind::Prompt,
                        &node.speaker,
                        &node.text,
                    )?;
                    prompt_seq.insert(node.id.clone(), seq);
                    for (position, option) in options.iter().enumerate() {
                        let seq = push(
                            &mut log,
                            &mut lines,
                            &mut before,
                            &node.id,
                            BakedKind::Choice,
                            &node.speaker,
                            &option.text,
                        )?;
                        choice_seq.insert((node.id.clone(), position), seq);
                    }
                }
                Flow::Bark => {
                    let seq = push(
                        &mut log,
                        &mut lines,
                        &mut before,
                        &node.id,
                        BakedKind::Bark,
                        &node.speaker,
                        &node.text,
                    )?;
                    solo_seq.insert(node.id.clone(), seq);
                }
            }
        }
        Ok(Self {
            compiled: compiled.clone(),
            lines,
            solo_seq,
            prompt_seq,
            choice_seq,
            log,
            bake_tick,
            request,
            writer_id,
            digest_seed,
        })
    }

    /// Validated tables this bake was built from.
    pub fn compiled(&self) -> &CompiledDialogue {
        &self.compiled
    }

    /// Every baked line in bake order; `line.seq` indexes the log.
    pub fn baked_lines(&self) -> &[BakedLine] {
        &self.lines
    }

    /// Baked line count: lines plus prompts plus options plus barks.
    pub fn baked_count(&self) -> usize {
        self.lines.len()
    }

    /// The provenance log: exactly one [`ProvenanceRecord`] per baked line, in bake
    /// order, so `log.records()[line.seq]` is that line's record.
    pub fn provenance(&self) -> &ProvenanceLog {
        &self.log
    }

    /// Fixed tick every record of this bake was observed at.
    pub fn bake_tick(&self) -> u64 {
        self.bake_tick
    }

    /// Ask or envelope id every record of this bake carries.
    pub fn request(&self) -> u64 {
        self.request
    }

    /// Record for one baked line; `None` when the sequence is out of range.
    pub fn record_for_seq(&self, seq: u64) -> Option<&ProvenanceRecord> {
        usize::try_from(seq)
            .ok()
            .and_then(|index| self.log.records().get(index))
    }

    fn check_flags<'a>(flags: &[&'a str]) -> Result<Vec<&'a str>, RunError> {
        if flags.len() > MAX_FLAGS {
            return Err(RunError::TooManyFlags);
        }
        for flag in flags {
            if !is_ident(flag) {
                return Err(RunError::BadFlag((*flag).to_owned()));
            }
        }
        Ok(flags.to_vec())
    }

    /// Play from the entry with no model: lines speak in order, branches take
    /// the first option in source order whose condition holds under `flags`.
    /// A branch with no satisfied option refuses; a walk that outgrows the
    /// table refuses as a cycle.
    pub fn play(&self, flags: &[&str]) -> Result<Vec<Spoken>, RunError> {
        let flags = Self::check_flags(flags)?;
        let mut out: Vec<Spoken> = Vec::new();
        let mut current = self.compiled.entry;
        loop {
            if out.len() > self.compiled.nodes.len() {
                return Err(RunError::Loop { steps: out.len() });
            }
            let node = &self.compiled.nodes[current];
            match &node.flow {
                Flow::Line { next } => {
                    let seq = self
                        .solo_seq
                        .get(&node.id)
                        .copied()
                        .ok_or_else(|| RunError::BrokenTable(node.id.clone()))?;
                    out.push(Spoken {
                        node: node.id.clone(),
                        speaker: node.speaker.clone(),
                        text: node.text.clone(),
                        seq,
                    });
                    match next {
                        Next::End => return Ok(out),
                        Next::Node(id) => {
                            current = self
                                .compiled
                                .index
                                .get(id)
                                .copied()
                                .ok_or_else(|| RunError::BrokenTable(node.id.clone()))?;
                        }
                    }
                }
                Flow::Branch { options } => {
                    let picked = options.iter().enumerate().find(|(_, option)| {
                        option
                            .cond
                            .as_ref()
                            .is_none_or(|flag| flags.contains(&flag.as_str()))
                    });
                    let (position, option) = match picked {
                        None => {
                            return Err(RunError::NoOption {
                                branch: node.id.clone(),
                            });
                        }
                        Some(found) => found,
                    };
                    let prompt = self
                        .prompt_seq
                        .get(&node.id)
                        .copied()
                        .ok_or_else(|| RunError::BrokenTable(node.id.clone()))?;
                    out.push(Spoken {
                        node: node.id.clone(),
                        speaker: node.speaker.clone(),
                        text: node.text.clone(),
                        seq: prompt,
                    });
                    let seq = self
                        .choice_seq
                        .get(&(node.id.clone(), position))
                        .copied()
                        .ok_or_else(|| RunError::BrokenTable(node.id.clone()))?;
                    out.push(Spoken {
                        node: node.id.clone(),
                        speaker: node.speaker.clone(),
                        text: option.text.clone(),
                        seq,
                    });
                    match &option.target {
                        Next::End => return Ok(out),
                        Next::Node(id) => {
                            current = self
                                .compiled
                                .index
                                .get(id)
                                .copied()
                                .ok_or_else(|| RunError::BrokenTable(node.id.clone()))?;
                        }
                    }
                }
                Flow::Bark => return Err(RunError::BrokenTable(node.id.clone())),
            }
        }
    }

    /// Speak one ambient bark by id with no model. Unknown ids refuse.
    pub fn speak_bark(&self, id: &str) -> Result<Spoken, RunError> {
        let node = self
            .compiled
            .find(id)
            .filter(|node| matches!(node.flow, Flow::Bark))
            .ok_or_else(|| RunError::UnknownBark(id.to_owned()))?;
        let seq = self
            .solo_seq
            .get(&node.id)
            .copied()
            .ok_or_else(|| RunError::BrokenTable(node.id.clone()))?;
        Ok(Spoken {
            node: node.id.clone(),
            speaker: node.speaker.clone(),
            text: node.text.clone(),
            seq,
        })
    }

    /// Deterministic bytes: header, tables, baked lines, then the provenance log
    /// bytes. The same script baked with the same stamp always gives the
    /// same bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&DIALOGUE_BYTES_VERSION.to_le_bytes());
        out.extend_from_slice(&self.bake_tick.to_le_bytes());
        out.extend_from_slice(&self.request.to_le_bytes());
        out.extend_from_slice(&self.writer_id.to_le_bytes());
        out.extend_from_slice(&self.digest_seed.to_le_bytes());
        write_str(&mut out, &self.compiled.name);
        out.extend_from_slice(&(self.compiled.flags.len() as u64).to_le_bytes());
        for flag in &self.compiled.flags {
            write_str(&mut out, flag);
        }
        out.extend_from_slice(&(self.compiled.nodes.len() as u64).to_le_bytes());
        for node in &self.compiled.nodes {
            match &node.flow {
                Flow::Line { next } => {
                    out.push(0);
                    write_str(&mut out, &node.id);
                    write_str(&mut out, &node.speaker);
                    write_str(&mut out, &node.text);
                    write_next(&mut out, next);
                }
                Flow::Branch { options } => {
                    out.push(1);
                    write_str(&mut out, &node.id);
                    write_str(&mut out, &node.speaker);
                    write_str(&mut out, &node.text);
                    out.extend_from_slice(&(options.len() as u64).to_le_bytes());
                    for option in options {
                        write_next(&mut out, &option.target);
                        match &option.cond {
                            None => out.push(0),
                            Some(flag) => {
                                out.push(1);
                                write_str(&mut out, flag);
                            }
                        }
                        write_str(&mut out, &option.text);
                    }
                }
                Flow::Bark => {
                    out.push(2);
                    write_str(&mut out, &node.id);
                    write_str(&mut out, &node.speaker);
                    write_str(&mut out, &node.text);
                }
            }
        }
        out.extend_from_slice(&(self.lines.len() as u64).to_le_bytes());
        for line in &self.lines {
            write_str(&mut out, &line.node);
            out.push(match line.kind {
                BakedKind::Line => 0,
                BakedKind::Prompt => 1,
                BakedKind::Choice => 2,
                BakedKind::Bark => 3,
            });
            write_str(&mut out, &line.speaker);
            write_str(&mut out, &line.text);
            out.extend_from_slice(&line.seq.to_le_bytes());
        }
        out.extend_from_slice(&self.log.bytes());
        out
    }

    /// Rebuild a bake from [`DialogueBake::bytes`]: revalidates the tables,
    /// the baked lines and every provenance record. Corrupt or future bytes refuse
    /// loudly and build nothing.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DialogueError> {
        let mut cursor = ByteCursor::new(bytes);
        if cursor.take_u32()? != DIALOGUE_BYTES_VERSION {
            return Err(fail(0, ErrorKind::UnknownVersion));
        }
        let bake_tick = cursor.take_u64()?;
        let request = cursor.take_u64()?;
        let writer_id = cursor.take_u64()?;
        let digest_seed = cursor.take_u64()?;
        let name = take_name(&mut cursor)?;
        if writer_id != script_writer(&name) {
            return Err(fail(0, ErrorKind::Corrupt));
        }
        let flag_count = cursor.take_u64()?;
        if flag_count > MAX_FLAGS as u64 {
            return Err(fail(0, ErrorKind::Corrupt));
        }
        let mut flags: Vec<(String, usize)> = Vec::new();
        for _ in 0..flag_count {
            let flag = take_name(&mut cursor)?;
            if !is_ident(&flag) {
                return Err(fail(0, ErrorKind::Corrupt));
            }
            flags.push((flag, 0));
        }
        let node_count = cursor.take_u64()?;
        if node_count > MAX_NODES as u64 {
            return Err(fail(0, ErrorKind::Corrupt));
        }
        // The entry id travels out of band: the header carries no separate
        // start line, so the first line-or-branch node in table order is the
        // entry the bake was built from.
        let mut raws: Vec<RawNode> = Vec::new();
        let mut raws_options: Vec<RawOption> = Vec::new();
        let mut entry_id: Option<String> = None;
        for _ in 0..node_count {
            let kind = cursor.take_u8()?;
            match kind {
                0 => {
                    let id = take_name(&mut cursor)?;
                    let speaker = take_name(&mut cursor)?;
                    let text = take_text(&mut cursor)?;
                    let target = read_next(&mut cursor)?;
                    if entry_id.is_none() {
                        entry_id = Some(id.clone());
                    }
                    raws.push(RawNode {
                        id,
                        speaker,
                        text,
                        flow: RawFlow::Line { target },
                        line: 0,
                    });
                }
                1 => {
                    let id = take_name(&mut cursor)?;
                    let speaker = take_name(&mut cursor)?;
                    let text = take_text(&mut cursor)?;
                    let option_count = cursor.take_u64()?;
                    if option_count > MAX_OPTIONS_PER_BRANCH as u64 {
                        return Err(fail(0, ErrorKind::Corrupt));
                    }
                    if entry_id.is_none() {
                        entry_id = Some(id.clone());
                    }
                    for _ in 0..option_count {
                        let target = read_next(&mut cursor)?;
                        let cond = match cursor.take_u8()? {
                            0 => None,
                            1 => Some(take_name(&mut cursor)?),
                            _ => return Err(fail(0, ErrorKind::Corrupt)),
                        };
                        let text = take_text(&mut cursor)?;
                        raws_options.push(RawOption {
                            branch: id.clone(),
                            target,
                            cond,
                            text,
                            line: 0,
                        });
                    }
                    raws.push(RawNode {
                        id,
                        speaker,
                        text,
                        flow: RawFlow::Branch,
                        line: 0,
                    });
                }
                2 => {
                    let id = take_name(&mut cursor)?;
                    let speaker = take_name(&mut cursor)?;
                    let text = take_text(&mut cursor)?;
                    raws.push(RawNode {
                        id,
                        speaker,
                        text,
                        flow: RawFlow::Bark,
                        line: 0,
                    });
                }
                _ => return Err(fail(0, ErrorKind::Corrupt)),
            }
        }
        let entry_id = entry_id.ok_or_else(|| fail(0, ErrorKind::Corrupt))?;
        let scripts = vec![(name.clone(), 0)];
        let starts = vec![(entry_id, 0)];
        let compiled = validate(&scripts, &starts, &flags, &raws, &raws_options)?;
        let baked_count = cursor.take_u64()?;
        if baked_count > MAX_BAKED_LINES as u64 {
            return Err(fail(0, ErrorKind::Corrupt));
        }
        let mut lines: Vec<BakedLine> = Vec::new();
        for position in 0..baked_count {
            let node = take_name(&mut cursor)?;
            let kind = match cursor.take_u8()? {
                0 => BakedKind::Line,
                1 => BakedKind::Prompt,
                2 => BakedKind::Choice,
                3 => BakedKind::Bark,
                _ => return Err(fail(0, ErrorKind::Corrupt)),
            };
            let speaker = take_name(&mut cursor)?;
            let text = take_text(&mut cursor)?;
            let seq = cursor.take_u64()?;
            if seq != position {
                return Err(fail(0, ErrorKind::Corrupt));
            }
            lines.push(BakedLine {
                node,
                speaker,
                text,
                kind,
                seq,
            });
        }
        // Baked lines must be exactly what the tables bake to.
        let rebuilt = compiled
            .bake(bake_tick, request, digest_seed)
            .map_err(|_| fail(0, ErrorKind::Corrupt))?;
        if rebuilt.lines != lines {
            return Err(fail(0, ErrorKind::Corrupt));
        }
        let rest = cursor.rest();
        let log = ProvenanceLog::from_bytes(rest).map_err(|_| fail(0, ErrorKind::Corrupt))?;
        if log.len() != lines.len() {
            return Err(fail(0, ErrorKind::Corrupt));
        }
        let mut before = digest_seed;
        for line in &lines {
            let record = log
                .records()
                .get(line.seq as usize)
                .ok_or_else(|| fail(0, ErrorKind::Corrupt))?;
            if record.detail != line.text
                || record.source != ActionSource::Scripted
                || record.observed_tick != bake_tick
                || record.request != request
                || record.writer != Writer::External(writer_id)
                || record.seq != line.seq
            {
                return Err(fail(0, ErrorKind::Corrupt));
            }
            let after = chain_after(before, line.seq, &line.text);
            if record.digest_before != before || record.digest_after != after {
                return Err(fail(0, ErrorKind::Corrupt));
            }
            before = after;
        }
        Ok(rebuilt)
    }

    /// Deterministic FNV-1a checkpoint over [`DialogueBake::bytes`]. One
    /// script plus one stamp always yields one checkpoint.
    pub fn checkpoint(&self) -> u64 {
        hash_bytes(&self.bytes())
    }
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u64).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn write_next(out: &mut Vec<u8>, next: &Next) {
    match next {
        Next::End => out.push(0),
        Next::Node(id) => {
            out.push(1);
            write_str(out, id);
        }
    }
}

fn read_next(cursor: &mut ByteCursor<'_>) -> Result<String, DialogueError> {
    match cursor.take_u8()? {
        0 => Ok("end".to_owned()),
        1 => take_name(cursor),
        _ => Err(fail(0, ErrorKind::Corrupt)),
    }
}

/// A short read of baked bytes is corruption.
impl From<ShortRead> for DialogueError {
    fn from(_: ShortRead) -> Self {
        fail(0, ErrorKind::Corrupt)
    }
}

/// One length-prefixed UTF-8 string no longer than `cap` bytes.
fn take_capped(cursor: &mut ByteCursor<'_>, cap: u64) -> Result<String, DialogueError> {
    let len = cursor.take_u64()?;
    if len > cap {
        return Err(fail(0, ErrorKind::Corrupt));
    }
    let raw = cursor.take(len as usize)?;
    let text = core::str::from_utf8(raw).map_err(|_| fail(0, ErrorKind::Corrupt))?;
    Ok(text.to_owned())
}

/// Ids are short; longer lengths are damage, refused before any alloc.
fn take_name(cursor: &mut ByteCursor<'_>) -> Result<String, DialogueError> {
    take_capped(cursor, 256)
}

/// Texts are bounded; longer lengths are damage, refused before any alloc.
fn take_text(cursor: &mut ByteCursor<'_>) -> Result<String, DialogueError> {
    take_capped(cursor, (MAX_TEXT as u64) * 4 + 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = "# trainer greets the yard\nscript greeting\n\nstart hello\nflag cheered\nline hello trainer -> pick: Welcome to the yard.\nbranch pick crowd: How do they answer?\noption pick -> cheer if cheered: They roar for you!\noption pick -> quiet: Silence falls.\nline cheer crowd -> end: The crowd roars!\nline quiet crowd -> end: Only the wind answers.\nbark forge blacksmith: Steel sings today.\n";

    fn bake_hello() -> DialogueBake {
        compile(HELLO)
            .expect("fixture compiles")
            .bake(90, 7, 1234)
            .expect("fixture bakes")
    }

    /// Twenty varied scripts: lines, branches, conditions, barks and
    /// multi-option tables across rotating voices.
    fn script_shapes() -> Vec<String> {
        let voices = ["trainer", "blacksmith", "crowd"];
        let mut scripts = Vec::with_capacity(20);
        for index in 0..20usize {
            let voice = voices[index % voices.len()];
            let other = voices[(index + 1) % voices.len()];
            let mut source = format!("# script {index}\nscript tale{index}\nstart open{index}\n");
            source.push_str(&format!("flag met{index}\nflag warned{index}\n"));
            source.push_str(&format!(
                "line open{index} {voice} -> ask{index}: The yard wakes for bout {index}.\n"
            ));
            source.push_str(&format!("branch ask{index} {other}: What stirs at dawn?\n"));
            source.push_str(&format!(
                "option ask{index} -> loud{index} if met{index}: Banners rise over the sand.\n"
            ));
            source.push_str(&format!(
                "option ask{index} -> soft{index} if warned{index}: Whispers trade along the rail.\n"
            ));
            source.push_str(&format!(
                "option ask{index} -> loud{index}: The horn sounds either way.\n"
            ));
            source.push_str(&format!(
                "line loud{index} {other} -> done{index}: The crowd roars for bout {index}!\n"
            ));
            source.push_str(&format!(
                "line soft{index} {voice} -> done{index}: Only the wind answers bout {index}.\n"
            ));
            source.push_str(&format!(
                "line done{index} {voice} -> end: Bout {index} is written.\n"
            ));
            source.push_str(&format!("bark side{index} {voice}: Steel sings at dawn.\n"));
            source.push_str(&format!(
                "bark bell{index} {other}: The bell keeps count.\n"
            ));
            scripts.push(source);
        }
        scripts
    }

    #[test]
    fn twenty_scripts_compile_x2_to_same_bytes() {
        let shapes = script_shapes();
        assert_eq!(shapes.len(), 20);
        for (position, source) in shapes.iter().enumerate() {
            let first = compile(source).expect("fixture compiles");
            let second = compile(source).expect("fixture compiles twice");
            assert_eq!(
                first, second,
                "script {position} compiles deterministically"
            );
            let baked_first = first.bake(40, position as u64, 999).expect("fixture bakes");
            let baked_second = second
                .bake(40, position as u64, 999)
                .expect("fixture bakes twice");
            assert_eq!(
                baked_first.bytes(),
                baked_second.bytes(),
                "script {position} bakes x2 to the same bytes"
            );
            assert_eq!(
                baked_first.checkpoint(),
                baked_second.checkpoint(),
                "script {position} checkpoints match"
            );
            // Every script carries all four shapes.
            assert!(
                baked_first.baked_count() >= 8,
                "script {position} bakes its lines"
            );
            assert!(
                baked_first.provenance().len() == baked_first.baked_count(),
                "script {position} records every line"
            );
            let loaded = DialogueBake::from_bytes(&baked_first.bytes()).expect("round-trips");
            assert_eq!(loaded, baked_first, "script {position} survives bytes");
        }
        println!(
            "dialogue-twenty scripts=20 bytes={} checkpoint={:016x}",
            bake_hello().bytes().len(),
            bake_hello().checkpoint(),
        );
    }

    #[test]
    fn comment_only_differences_bake_identically() {
        let plain = compile(HELLO).expect("fixture compiles");
        let noisy = format!("# top\n\n\n{HELLO}\n# tail\n");
        let rerun = compile(&noisy).expect("comments recompile");
        assert_eq!(plain, rerun, "comments never enter the tables");
        assert_eq!(
            plain.bake(1, 1, 1).expect("bakes").bytes(),
            rerun.bake(1, 1, 1).expect("bakes").bytes()
        );
    }

    #[test]
    fn broken_refs_refused_with_line_numbers() {
        // Unknown line jump at line 4.
        let err = compile("script bad\nstart a\nflag f\nline a trainer -> ghost: Hi.\n")
            .expect_err("unknown jump refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::UnknownTarget("ghost".to_owned()));
        // Unknown option target at line 6.
        let err = compile(
            "script bad\nstart a\nline a trainer -> b: Hi.\nbranch b crowd: Pick.\noption b -> ghost: Go.\nline c crowd -> end: Done.\n",
        )
        .expect_err("unknown option target refuses");
        assert_eq!(err.line, 5);
        assert_eq!(err.kind, ErrorKind::UnknownTarget("ghost".to_owned()));
        // Unknown flag at line 5.
        let err = compile(
            "script bad\nstart a\nline a trainer -> b: Hi.\nbranch b crowd: Pick.\noption b -> c if missing: Go.\nline c crowd -> end: Done.\n",
        )
        .expect_err("unknown flag refuses");
        assert_eq!(err.line, 5);
        assert_eq!(err.kind, ErrorKind::UnknownFlag("missing".to_owned()));
        // Jumps into a bark refuse at line 3.
        let err =
            compile("script bad\nstart a\nline a trainer -> hum: Hi.\nbark hum smith: Mending.\n")
                .expect_err("bark target refuses");
        assert_eq!(err.line, 3);
        assert_eq!(err.kind, ErrorKind::TargetIsBark("hum".to_owned()));
        // Options must name a branch: line at option row 5.
        let err = compile(
            "script bad\nstart a\nline a trainer -> b: Hi.\nline b crowd -> end: Done.\noption b -> c: Go.\nline c crowd -> end: Over.\n",
        )
        .expect_err("option on a line refuses");
        assert_eq!(err.line, 5);
        assert_eq!(err.kind, ErrorKind::NotABranch("b".to_owned()));
        // Options must name a node at all, row 4.
        let err =
            compile("script bad\nstart a\nline a trainer -> end: Hi.\noption ghost -> end: Go.\n")
                .expect_err("option on nothing refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::UnknownBranch("ghost".to_owned()));
        // Unknown start at line 2.
        let err = compile("script bad\nstart ghost\nline a trainer -> end: Hi.\n")
            .expect_err("unknown start refuses");
        assert_eq!(err.line, 2);
        assert_eq!(err.kind, ErrorKind::StartUnknown("ghost".to_owned()));
        // Start at a bark refuses at line 2.
        let err = compile("script bad\nstart hum\nbark hum smith: Mending.\n")
            .expect_err("bark start refuses");
        assert_eq!(err.line, 2);
        assert_eq!(err.kind, ErrorKind::StartIsBark);
        // Duplicate node names the repeat row and the first row.
        let err = compile(
            "script bad\nstart a\nline a trainer -> end: Hi.\nline a smith -> end: Again.\n",
        )
        .expect_err("duplicate node refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::DuplicateNode { other: 3 });
        // Duplicate flags name the repeat row and the first row.
        let err = compile("script bad\nstart a\nflag f\nflag f\nline a trainer -> end: Hi.\n")
            .expect_err("duplicate flag refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::DuplicateFlag { other: 3 });
        // Empty branches refuse at the branch row.
        let err = compile("script bad\nstart a\nline a trainer -> b: Hi.\nbranch b crowd: Pick.\n")
            .expect_err("empty branch refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::BranchNoOptions);
        // Missing start and script refuse without a row.
        assert_eq!(
            compile("line a trainer -> end: Hi.\n").expect_err("missing script refuses"),
            DialogueError {
                line: 0,
                kind: ErrorKind::MissingScript
            }
        );
        assert_eq!(
            compile("script bad\nline a trainer -> end: Hi.\n").expect_err("missing start refuses"),
            DialogueError {
                line: 0,
                kind: ErrorKind::MissingStart
            }
        );
        // Unknown directives name the row and the word.
        let err = compile("script bad\nstart a\nsing a trainer: Hi.\n")
            .expect_err("unknown directive refuses");
        assert_eq!(err.line, 3);
        assert_eq!(err.kind, ErrorKind::UnknownDirective("sing".to_owned()));
    }

    #[test]
    fn dead_branches_refused_with_line_numbers() {
        // A line nothing reaches refuses at its own row.
        let err = compile(
            "script bad\nstart a\nline a trainer -> end: Hi.\nline lost smith -> end: Unheard.\n",
        )
        .expect_err("dead line refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::DeadNode("lost".to_owned()));
        // A branch nothing reaches refuses at its own row.
        let err = compile(
            "script bad\nstart a\nline a trainer -> end: Hi.\nbranch cut crowd: Pick.\noption cut -> over: Go.\nline over crowd -> end: Done.\n",
        )
        .expect_err("dead branch refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::DeadNode("cut".to_owned()));
        // Barks are ambient roots: unreferenced barks stay legal.
        let baked = compile(
            "script fine\nstart a\nline a trainer -> end: Hi.\nbark hum smith: Mending.\nbark bell crowd: Ringing.\n",
        )
        .expect("unreferenced barks compile")
        .bake(3, 3, 3)
        .expect("barks bake");
        assert_eq!(baked.baked_count(), 3);
        assert_eq!(baked.provenance().len(), 3);
    }

    #[test]
    fn every_baked_line_carries_an_lw6_record() {
        let baked = bake_hello();
        // Line, prompt, two options, two closers, one bark: seven records.
        assert_eq!(baked.baked_count(), 7);
        assert_eq!(baked.provenance().len(), 7);
        assert_eq!(baked.bake_tick(), 90);
        assert_eq!(baked.request(), 7);
        let writer = Writer::External(script_writer("greeting"));
        let mut before = 1234u64;
        for line in baked.baked_lines() {
            let record = baked
                .record_for_seq(line.seq)
                .expect("every baked line resolves a record");
            assert_eq!(record.detail, line.text, "record labels its own line");
            assert_eq!(record.source, ActionSource::Scripted, "bakes are scripted");
            assert_eq!(record.observed_tick, 90);
            assert_eq!(record.request, 7);
            assert_eq!(record.writer, writer, "records trace to the script");
            assert_eq!(record.seq, line.seq, "sequences stay dense");
            let after = chain_after(before, line.seq, &line.text);
            assert_eq!(record.digest_before, before);
            assert_eq!(record.digest_after, after);
            before = after;
        }
        assert!(
            baked.record_for_seq(99).is_none(),
            "out-of-range reads None"
        );
        // Loaded bytes keep the same records, and tampered bytes refuse.
        let loaded = DialogueBake::from_bytes(&baked.bytes()).expect("round-trips");
        assert_eq!(loaded.provenance().records(), baked.provenance().records());
        let good = baked.bytes();
        for cut in [0usize, 5, 11, 40, 120] {
            let cut = cut.min(good.len().saturating_sub(1));
            assert!(
                DialogueBake::from_bytes(&good[..cut]).is_err(),
                "truncation at {cut} refuses"
            );
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            DialogueBake::from_bytes(&trailing).expect_err("trailing refuses"),
            DialogueError {
                line: 0,
                kind: ErrorKind::Corrupt
            }
        );
        let mut versioned = good.clone();
        versioned[0] = versioned[0].wrapping_add(1);
        if versioned[0..4] == good[0..4] {
            versioned[1] = versioned[1].wrapping_add(1);
        }
        assert_eq!(
            DialogueBake::from_bytes(&versioned).expect_err("version refuses"),
            DialogueError {
                line: 0,
                kind: ErrorKind::UnknownVersion
            }
        );
        // A record count that disagrees with the baked lines refuses.
        let mut short = good.clone();
        assert!(
            DialogueBake::from_bytes(&short[..short.len().saturating_sub(10)]).is_err(),
            "clipped log refuses"
        );
        let _ = short.pop();
        assert!(
            DialogueBake::from_bytes(&short).is_err(),
            "clipped log refuses"
        );
    }

    #[test]
    fn runtime_plays_with_no_model() {
        let baked = bake_hello();
        // Flag set: the conditional option wins over the fallback order.
        let played = baked.play(&["cheered"]).expect("flagged play runs");
        assert_eq!(played.len(), 4, "line plus prompt plus choice plus closer");
        assert_eq!(played[0].text, "Welcome to the yard.");
        assert_eq!(played[1].text, "How do they answer?");
        assert_eq!(played[2].text, "They roar for you!");
        assert_eq!(played[3].text, "The crowd roars!");
        // Flag off: the unconditional fallback plays instead.
        let played = baked.play(&[]).expect("unflagged play runs");
        assert_eq!(played[2].text, "Silence falls.");
        assert_eq!(played[3].text, "Only the wind answers.");
        // Every played step resolves its provenance record.
        for step in played.iter() {
            let record = baked.record_for_seq(step.seq).expect("steps carry records");
            assert_eq!(record.detail, step.text);
        }
        // Barks speak by id with no model.
        let bark = baked.speak_bark("forge").expect("bark speaks");
        assert_eq!(bark.text, "Steel sings today.");
        assert_eq!(bark.speaker, "blacksmith");
        assert_eq!(
            baked
                .record_for_seq(bark.seq)
                .expect("barks carry records")
                .detail,
            "Steel sings today."
        );
        assert_eq!(
            baked.speak_bark("ghost").expect_err("unknown bark refuses"),
            RunError::UnknownBark("ghost".to_owned())
        );
        // Runtime flags are validated like any external input.
        assert_eq!(
            baked.play(&["not a flag"]).expect_err("bad flag refuses"),
            RunError::BadFlag("not a flag".to_owned())
        );
        let many: Vec<&str> = (0..70).map(|_| "f").collect();
        assert_eq!(
            baked.play(&many).expect_err("flag flood refuses"),
            RunError::TooManyFlags
        );
    }

    #[test]
    fn stuck_branches_and_loops_refuse() {
        // Every option conditional, none set: the branch is stuck.
        let baked = compile(
            "script stuck\nstart a\nflag f\nline a trainer -> b: Hi.\nbranch b crowd: Pick.\noption b -> c if f: Go.\nline c crowd -> end: Done.\n",
        )
        .expect("stuck script compiles")
        .bake(1, 1, 1)
        .expect("stuck script bakes");
        assert_eq!(
            baked.play(&[]).expect_err("stuck branch refuses"),
            RunError::NoOption {
                branch: "b".to_owned()
            }
        );
        assert_eq!(baked.play(&["f"]).expect("set flag plays").len(), 4);
        // A two-node cycle never terminates: the walk refuses as a loop.
        let baked =
            compile("script loop\nstart a\nline a trainer -> b: One.\nline b crowd -> a: Two.\n")
                .expect("cyclic script compiles")
                .bake(1, 1, 1)
                .expect("cyclic script bakes");
        assert!(
            matches!(baked.play(&[]), Err(RunError::Loop { .. })),
            "cycles refuse instead of spinning"
        );
    }

    #[test]
    fn rejects_bad_text_names_and_caps() {
        // Empty and over-long texts refuse at their rows.
        let err = compile("script bad\nstart a\nline a trainer -> end:   \n")
            .expect_err("empty text refuses");
        assert_eq!(err.line, 3);
        assert_eq!(err.kind, ErrorKind::EmptyText);
        let long: String = "x".repeat(MAX_TEXT + 1);
        let err = compile(&format!(
            "script bad\nstart a\nline a trainer -> end: {long}\n"
        ))
        .expect_err("long text refuses");
        assert_eq!(err.line, 3);
        assert_eq!(err.kind, ErrorKind::TextTooLong);
        // Bad names refuse at their rows, and `end` stays reserved.
        let err = compile("script bad-name\nstart a\nline a trainer -> end: Hi.\n")
            .expect_err("bad script name refuses");
        assert_eq!(err.line, 1);
        assert!(matches!(err.kind, ErrorKind::BadName(_)));
        let err = compile("script bad\nstart a\nline end trainer -> end: Hi.\n")
            .expect_err("reserved id refuses");
        assert_eq!(err.line, 3);
        assert_eq!(err.kind, ErrorKind::ReservedId);
        let err =
            compile("script bad\nstart a\nline a trainer -> end: Hi.\nline a trainer -> end\n")
                .expect_err("missing colon refuses");
        assert_eq!(err.line, 4);
        assert_eq!(err.kind, ErrorKind::MissingColon);
        let err = compile("script bad\nstart a\nline a trainer end: Hi.\n")
            .expect_err("bad arrow refuses");
        assert_eq!(err.line, 3);
        assert_eq!(err.kind, ErrorKind::BadArrow);
        // Cap rows refuse at the offending row.
        let mut flood = "script flood\nstart n0\n".to_owned();
        for index in 0..=MAX_NODES {
            flood.push_str(&format!("line n{index} trainer -> end: Line {index}.\n"));
        }
        let err = compile(&flood).expect_err("node flood refuses");
        assert_eq!(err.kind, ErrorKind::TooManyNodes);
        let mut options =
            "script many\nstart a\nflag f\nline a trainer -> b: Hi.\nbranch b crowd: Pick.\n"
                .to_owned();
        for index in 0..=MAX_OPTIONS_PER_BRANCH {
            options.push_str(&format!("option b -> c{index} if f: Choice {index}.\n"));
        }
        for index in 0..=MAX_OPTIONS_PER_BRANCH {
            options.push_str(&format!("line c{index} crowd -> end: Done {index}.\n"));
        }
        let err = compile(&options).expect_err("option flood refuses");
        assert_eq!(err.kind, ErrorKind::TooManyOptions);
        let mut flagflood = "script flags\nstart a\n".to_owned();
        for index in 0..=MAX_FLAGS {
            flagflood.push_str(&format!("flag f{index}\n"));
        }
        flagflood.push_str("line a trainer -> end: Hi.\n");
        let err = compile(&flagflood).expect_err("flag flood refuses");
        assert_eq!(err.kind, ErrorKind::TooManyFlags);
    }

    #[test]
    fn error_display_names_line_and_reason() {
        let err = DialogueError {
            line: 12,
            kind: ErrorKind::DeadNode("lost".to_owned()),
        };
        assert_eq!(format!("{err}"), "dialogue:12: dead node lost");
        let err = DialogueError {
            line: 0,
            kind: ErrorKind::Corrupt,
        };
        assert_eq!(format!("{err}"), "dialogue:0: corrupt bytes");
    }
}
