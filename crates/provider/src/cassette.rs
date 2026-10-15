//! cassette: content-addressed record and replay of provider answers.
//! A live run records each answer under a key of (adapter label, model,
//! request digest, schema version); an eval or CI run replays them with no
//! network and no key. [`CassetteAdapter`] wraps any [`ProviderAdapter`], so
//! every backend is cassette-backed the same way.
//!
//! The digest is taken over the whole request image (task kind, system
//! prompt, user prompt, sampling pins, tools), never over the request id or
//! deadline, so a re-run with fresh ids still hits. The image itself is
//! stored in the file and compared exactly on load, so a 64-bit digest
//! collision cannot serve a wrong answer. A request that no entry matches is
//! refused with a typed [`MissReason`] naming what changed (system prompt,
//! user prompt, sampling, tools) or that the entry is stale (other schema
//! version, other file format): a stale entry is never reused silently.
//!
//! Only answers the model gave are recorded: a complete proposal, or output
//! that failed the strict grammar. Timeouts, quota, privacy and other
//! environment refusals are not, so a flaky night never freezes into a
//! fixture. Replayed answers are rebuilt through the strict proposal parser
//! under the asking request's id, so a hand-edited cassette cannot smuggle a
//! malformed proposal past the grammar.
//!
//! Units: digests are FNV-1a 64 (`engine_core`), non-adversarial; files are
//! pretty-printed JSON, bounded by [`MAX_CASSETTE_BYTES`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use engine_core::Fnv64;

use super::core::AdapterOutcome;
use super::ladder::Rung;
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::{
    validate_request, AiError, AiEvent, AiRequest, AiResult, EventKind, FinishReason, Proposal,
    ProposalAction, RequestId, TaskKind, Usage,
};
use super::vendor::{jparse, parse_proposal_value, prompt_text, JVal};

/// Format version of one cassette file. A file at another version misses as
/// stale instead of being parsed on a guess.
pub const CASSETTE_FORMAT: u16 = 1;
/// Largest cassette file read, in bytes.
pub const MAX_CASSETTE_BYTES: u64 = 256 * 1024;
/// Most events a [`CassetteAdapter`] keeps; the oldest drop first.
pub const MAX_LOG_EVENTS: usize = 256;
/// Longest adapter label.
const MAX_LABEL_LEN: usize = 32;
/// Longest readable model slug in a file name.
const MAX_SLUG_LEN: usize = 40;
/// Most files one miss explanation reads.
const MAX_SCAN_FILES: usize = 4096;

/// What a [`CassetteAdapter`] does with each call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CassetteMode {
    /// Pass through; touch no file.
    Off,
    /// Call the live adapter and write what the model answered.
    Record,
    /// Serve only from the store; never call the inner adapter.
    Replay,
}

impl CassetteMode {
    /// Parse `off`, `record` or `replay`. The caller reads its own env var or
    /// flag; this stays pure.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(CassetteMode::Off),
            "record" => Some(CassetteMode::Record),
            "replay" => Some(CassetteMode::Replay),
            _ => None,
        }
    }
}

/// Which parts of a request differ between a recorded and an asked image.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ChangedParts {
    /// The system prompt differs.
    pub system: bool,
    /// The user prompt (or the task kind) differs.
    pub user: bool,
    /// Temperature, seed or output-token bound differs.
    pub sampling: bool,
    /// The tool set differs.
    pub tools: bool,
    /// The starting rung of the structured-output ladder differs.
    pub structured: bool,
}

impl ChangedParts {
    fn count(self) -> u8 {
        u8::from(self.system)
            + u8::from(self.user)
            + u8::from(self.sampling)
            + u8::from(self.tools)
            + u8::from(self.structured)
    }
}

/// Why a lookup found nothing it may serve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MissReason {
    /// Nothing recorded for this model and request.
    NotRecorded,
    /// An entry for the same request id exists, but the request changed.
    Changed(ChangedParts),
    /// The same request was recorded under another schema version.
    SchemaVersion {
        /// Version the entry was recorded under.
        recorded: u16,
    },
    /// The entry was written by another cassette file format.
    Format {
        /// Format version found in the file.
        recorded: u16,
    },
}

impl std::fmt::Display for MissReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MissReason::NotRecorded => f.write_str("no cassette entry for this request"),
            MissReason::Changed(parts) => {
                let names = [
                    (parts.system, "system prompt"),
                    (parts.user, "user prompt"),
                    (parts.sampling, "sampling"),
                    (parts.tools, "tools"),
                    (parts.structured, "structured output"),
                ];
                let changed: Vec<&str> = names
                    .iter()
                    .filter(|(on, _)| *on)
                    .map(|(_, name)| *name)
                    .collect();
                write!(f, "request changed since recording: {}", changed.join(", "))
            }
            MissReason::SchemaVersion { recorded } => {
                write!(f, "entry recorded under schema version {recorded}")
            }
            MissReason::Format { recorded } => {
                write!(f, "entry written by cassette format {recorded}")
            }
        }
    }
}

/// Typed cassette failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CassetteError {
    /// No entry may serve the request; the reason says why.
    Miss(MissReason),
    /// The adapter label is empty, too long or outside `[a-z0-9-]`.
    BadLabel,
    /// The store directory or a file could not be read or written.
    Io(io::ErrorKind),
    /// A file is not a well-formed entry for its own key.
    Corrupt,
}

impl std::fmt::Display for CassetteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CassetteError::Miss(reason) => write!(f, "cassette miss: {reason}"),
            CassetteError::BadLabel => f.write_str("cassette label must be [a-z0-9-], 1..=32"),
            CassetteError::Io(kind) => write!(f, "cassette i/o failed: {kind}"),
            CassetteError::Corrupt => f.write_str("cassette entry is corrupt"),
        }
    }
}

impl std::error::Error for CassetteError {}

/// Everything the model was asked, in full: the digest preimage and the
/// exact-compare copy stored beside every answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RequestImage {
    kind: TaskKind,
    system: String,
    user: String,
    temperature_milli: u16,
    seed: Option<u64>,
    max_output_tokens: u32,
    tools: String,
    structured: String,
}

impl RequestImage {
    /// The image of `request` as an adapter holding `pins` would send it.
    /// An adapter with no pins (the mock) sends no system prompt, sampling
    /// or tools, and its image says so.
    #[must_use]
    pub fn of(request: &AiRequest, pins: Option<&RequestPins>) -> Self {
        let (system, temperature_milli, seed, tools, structured) = match pins {
            Some(pins) => {
                let mut tools_text = String::from("[");
                for (index, tool) in pins.tools().iter().enumerate() {
                    if index > 0 {
                        tools_text.push(',');
                    }
                    tools_text.push_str(&format!(
                        "{{\"name\":{:?},\"description\":{:?},\"parameters\":{}}}",
                        tool.name(),
                        tool.description(),
                        tool.parameters()
                    ));
                }
                tools_text.push(']');
                (
                    pins.system_for(request.kind).to_owned(),
                    pins.temperature_milli(),
                    pins.seed(),
                    tools_text,
                    pins.ladder().start().label().to_owned(),
                )
            }
            None => (
                String::new(),
                0,
                None,
                String::from("[]"),
                Rung::Text.label().to_owned(),
            ),
        };
        Self {
            kind: request.kind,
            system,
            user: prompt_text(request),
            temperature_milli,
            seed,
            max_output_tokens: request.budget.max_output_tokens,
            tools,
            structured,
        }
    }

    /// FNV-1a 64 over the image, with a length before each text so two
    /// fields cannot trade bytes.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        let texts = [
            kind_name(self.kind),
            &self.system,
            &self.user,
            &self.tools,
            &self.structured,
        ];
        for text in texts {
            hasher.write_u64(u64::try_from(text.len()).unwrap_or(u64::MAX));
            hasher.write_str(text);
        }
        hasher.write_u16(self.temperature_milli);
        match self.seed {
            Some(seed) => {
                hasher.write_u8(1);
                hasher.write_u64(seed);
            }
            None => hasher.write_u8(0),
        }
        hasher.write_u32(self.max_output_tokens);
        hasher.finish()
    }

    fn diff(&self, other: &Self) -> ChangedParts {
        ChangedParts {
            system: self.system != other.system,
            user: self.user != other.user || self.kind != other.kind,
            sampling: (self.temperature_milli, self.seed, self.max_output_tokens)
                != (other.temperature_milli, other.seed, other.max_output_tokens),
            tools: self.tools != other.tools,
            structured: self.structured != other.structured,
        }
    }

    fn to_jval(&self) -> JVal {
        JVal::Obj(vec![
            (
                String::from("kind"),
                JVal::Str(String::from(kind_name(self.kind))),
            ),
            (String::from("system"), JVal::Str(self.system.clone())),
            (String::from("user"), JVal::Str(self.user.clone())),
            (
                String::from("temperature_milli"),
                JVal::Num(self.temperature_milli.to_string()),
            ),
            (
                String::from("seed"),
                match self.seed {
                    None => JVal::Null,
                    Some(seed) => JVal::Num(seed.to_string()),
                },
            ),
            (
                String::from("max_output_tokens"),
                JVal::Num(self.max_output_tokens.to_string()),
            ),
            (String::from("tools"), JVal::Str(self.tools.clone())),
            (
                String::from("structured"),
                JVal::Str(self.structured.clone()),
            ),
        ])
    }

    fn from_jval(value: &JVal) -> Option<Self> {
        Some(Self {
            kind: kind_from_name(field_str(value, "kind")?)?,
            system: field_str(value, "system")?.to_owned(),
            user: field_str(value, "user")?.to_owned(),
            temperature_milli: u16::try_from(field_u64(value, "temperature_milli")?).ok()?,
            seed: match value.get("seed")? {
                JVal::Null => None,
                other => Some(other.as_u64()?),
            },
            max_output_tokens: u32::try_from(field_u64(value, "max_output_tokens")?).ok()?,
            tools: field_str(value, "tools")?.to_owned(),
            structured: field_str(value, "structured")?.to_owned(),
        })
    }
}

fn kind_name(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Inspect => "inspect",
        TaskKind::Propose => "propose",
        TaskKind::Diagnose => "diagnose",
        TaskKind::Repair => "repair",
        TaskKind::Announce => "announce",
        TaskKind::Taunt => "taunt",
    }
}

fn kind_from_name(name: &str) -> Option<TaskKind> {
    Some(match name {
        "inspect" => TaskKind::Inspect,
        "propose" => TaskKind::Propose,
        "diagnose" => TaskKind::Diagnose,
        "repair" => TaskKind::Repair,
        "announce" => TaskKind::Announce,
        "taunt" => TaskKind::Taunt,
        _ => return None,
    })
}

fn field_str<'a>(value: &'a JVal, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn field_u64(value: &JVal, key: &str) -> Option<u64> {
    value.get(key)?.as_u64()
}

fn jemit(value: &JVal, out: &mut String) {
    match value {
        JVal::Null => out.push_str("null"),
        JVal::Bool(true) => out.push_str("true"),
        JVal::Bool(false) => out.push_str("false"),
        JVal::Num(raw) => out.push_str(raw),
        JVal::Str(text) => out.push_str(&format!("{text:?}")),
        JVal::Arr(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                jemit(item, out);
            }
            out.push(']');
        }
        JVal::Obj(pairs) => {
            out.push('{');
            for (index, (key, item)) in pairs.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&format!("{key:?}: "));
                jemit(item, out);
            }
            out.push('}');
        }
    }
}

fn jcompact(value: &JVal) -> String {
    let mut out = String::new();
    jemit(value, &mut out);
    out.push('\n');
    out
}

/// True for a label of `[a-z0-9-]`, 1 to [`MAX_LABEL_LEN`] bytes.
fn label_ok(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL_LEN
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The key one entry lives under: (adapter label, model, schema version,
/// request digest).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CassetteKey {
    label: String,
    model: String,
    schema_version: u16,
    digest: u64,
}

impl CassetteKey {
    /// Key for `request` as `label` serves it. The label names the adapter
    /// (`openai`, `local`, ...); the model and schema version come from the
    /// request, the digest from its image.
    pub fn new(
        label: &str,
        request: &AiRequest,
        image: &RequestImage,
    ) -> Result<Self, CassetteError> {
        if !label_ok(label) {
            return Err(CassetteError::BadLabel);
        }
        Ok(Self {
            label: label.to_owned(),
            model: request.model.model.clone(),
            schema_version: request.schema_version,
            digest: image.digest(),
        })
    }

    /// The request digest this key addresses.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// File-name prefix shared by every entry of this label and model: the
    /// label, a readable model slug and a hash of the exact model string, so
    /// two model names that slug alike never share a file.
    fn prefix(&self) -> String {
        let slug: String = self
            .model
            .chars()
            .take(MAX_SLUG_LEN)
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut hasher = Fnv64::new();
        hasher.write_str(&self.model);
        format!(
            "{}__{slug}-{:08x}",
            self.label,
            hasher.finish() & 0xffff_ffff
        )
    }

    fn file_name(&self) -> String {
        format!(
            "{}__v{}__{:016x}.json",
            self.prefix(),
            self.schema_version,
            self.digest
        )
    }
}

/// What the model answered, in the two shapes worth replaying.
#[expect(private_interfaces, reason = "replay body is the vendor scanner value")]
#[derive(Clone, PartialEq, Debug)]
pub enum Answer {
    /// A complete proposal body (`actions` and `diagnostics`) plus usage.
    Complete {
        /// The proposal as the wire grammar spells it.
        proposal: JVal,
        /// Token usage the server reported; unknown stays `None`.
        usage: Usage,
    },
    /// Output that failed the strict grammar.
    Malformed,
}

impl Answer {
    /// The recordable answer inside `outcome`, or `None` for anything the
    /// environment decided (timeouts, quota, privacy, duplicates).
    #[must_use]
    pub fn from_outcome(outcome: &AdapterOutcome) -> Option<Self> {
        if outcome.duplicate || outcome.discarded {
            return None;
        }
        match &outcome.result {
            Ok(result) if result.reason == FinishReason::Complete => {
                let proposal = result.proposal.as_ref()?;
                Some(Answer::Complete {
                    proposal: proposal_body(proposal)?,
                    usage: result.usage,
                })
            }
            Err(AiError::MalformedOutput) => Some(Answer::Malformed),
            _ => None,
        }
    }

    /// Rebuild the outcome for `request`. A complete proposal goes back
    /// through the strict parser under the asking request's id; a body that
    /// no longer parses is [`CassetteError::Corrupt`].
    pub fn into_outcome(self, request: &AiRequest) -> Result<AdapterOutcome, CassetteError> {
        match self {
            Answer::Malformed => Ok(failed_outcome(request, AiError::MalformedOutput)),
            Answer::Complete { proposal, usage } => {
                let proposal = parse_proposal_value(&proposal, request, "")
                    .map_err(|_| CassetteError::Corrupt)?;
                let result = AiResult {
                    request: request.id,
                    reason: FinishReason::Complete,
                    proposal: Some(proposal),
                    usage,
                };
                let event = AiEvent {
                    request: request.id,
                    seq: 0,
                    kind: EventKind::Completed(result.clone()),
                };
                Ok(AdapterOutcome {
                    result: Ok(result),
                    events: vec![event],
                    dispatched_remote: false,
                    duplicate: false,
                    mutated: true,
                    discarded: false,
                })
            }
        }
    }

    fn to_jval(&self) -> JVal {
        match self {
            Answer::Malformed => JVal::Obj(vec![(
                String::from("err"),
                JVal::Str(String::from("malformed_output")),
            )]),
            Answer::Complete { proposal, usage } => JVal::Obj(vec![(
                String::from("ok"),
                JVal::Obj(vec![
                    (String::from("proposal"), proposal.clone()),
                    (
                        String::from("usage"),
                        JVal::Obj(vec![
                            (
                                String::from("input_tokens"),
                                match usage.input_tokens {
                                    None => JVal::Null,
                                    Some(n) => JVal::Num(n.to_string()),
                                },
                            ),
                            (
                                String::from("output_tokens"),
                                match usage.output_tokens {
                                    None => JVal::Null,
                                    Some(n) => JVal::Num(n.to_string()),
                                },
                            ),
                        ]),
                    ),
                ]),
            )]),
        }
    }

    fn from_jval(value: &JVal) -> Option<Self> {
        let JVal::Obj(pairs) = value else {
            return None;
        };
        if let Some(err) = value.get("err") {
            let known = err.as_str() == Some("malformed_output") && pairs.len() == 1;
            return known.then_some(Answer::Malformed);
        }
        let ok = value.get("ok")?;
        let usage = ok.get("usage")?;
        let count = |key: &str| match usage.get(key)? {
            JVal::Null => Some(None),
            other => other.as_u64().map(Some),
        };
        Some(Answer::Complete {
            proposal: ok.get("proposal")?.clone(),
            usage: Usage {
                input_tokens: count("input_tokens")?,
                output_tokens: count("output_tokens")?,
            },
        })
    }
}

/// The proposal as the wire grammar spells it, or `None` when it carries
/// state the grammar cannot (object versions).
fn proposal_body(proposal: &Proposal) -> Option<JVal> {
    if !proposal.object_versions.is_empty() {
        return None;
    }
    let mut actions: Vec<JVal> = Vec::new();
    for action in &proposal.actions {
        actions.push(match action {
            ProposalAction::Noop => JVal::Obj(vec![(String::from("noop"), JVal::Bool(true))]),
            ProposalAction::SetPosition { object, xyz_cm } => JVal::Obj(vec![(
                String::from("set_position"),
                JVal::Obj(vec![
                    (String::from("object"), JVal::Num(object.to_string())),
                    (
                        String::from("xyz_cm"),
                        JVal::Arr(vec![
                            JVal::Num(xyz_cm[0].to_string()),
                            JVal::Num(xyz_cm[1].to_string()),
                            JVal::Num(xyz_cm[2].to_string()),
                        ]),
                    ),
                ]),
            )]),
            ProposalAction::SetLabel { object, label } => JVal::Obj(vec![(
                String::from("set_label"),
                JVal::Obj(vec![
                    (String::from("object"), JVal::Num(object.to_string())),
                    (String::from("label"), JVal::Str(label.clone())),
                ]),
            )]),
        });
    }
    let diagnostics = proposal
        .diagnostics
        .iter()
        .map(|line| JVal::Str(line.clone()))
        .collect();
    Some(JVal::Obj(vec![
        (String::from("actions"), JVal::Arr(actions)),
        (String::from("diagnostics"), JVal::Arr(diagnostics)),
    ]))
}

/// A failed outcome in the contract's envelope: one terminal event, nothing
/// mutated, nothing dispatched.
fn failed_outcome(request: &AiRequest, error: AiError) -> AdapterOutcome {
    AdapterOutcome {
        result: Err(error),
        events: vec![AiEvent {
            request: request.id,
            seq: 0,
            kind: EventKind::Failed(error),
        }],
        dispatched_remote: false,
        duplicate: false,
        mutated: false,
        discarded: false,
    }
}

/// One parsed cassette file.
struct Entry {
    label: String,
    model: String,
    schema_version: u16,
    digest: u64,
    request_id: u64,
    image: RequestImage,
    answer: Answer,
}

impl Entry {
    fn to_jval(&self) -> JVal {
        JVal::Obj(vec![
            (
                String::from("format"),
                JVal::Num(CASSETTE_FORMAT.to_string()),
            ),
            (String::from("label"), JVal::Str(self.label.clone())),
            (String::from("model"), JVal::Str(self.model.clone())),
            (
                String::from("schema_version"),
                JVal::Num(self.schema_version.to_string()),
            ),
            (
                String::from("digest"),
                JVal::Str(format!("{:016x}", self.digest)),
            ),
            (
                String::from("request_id"),
                JVal::Num(self.request_id.to_string()),
            ),
            (String::from("request"), self.image.to_jval()),
            (String::from("answer"), self.answer.to_jval()),
        ])
    }

    fn from_jval(value: &JVal) -> Option<Self> {
        Some(Self {
            label: field_str(value, "label")?.to_owned(),
            model: field_str(value, "model")?.to_owned(),
            schema_version: u16::try_from(field_u64(value, "schema_version")?).ok()?,
            digest: u64::from_str_radix(field_str(value, "digest")?, 16).ok()?,
            request_id: field_u64(value, "request_id")?,
            image: RequestImage::from_jval(value.get("request")?)?,
            answer: Answer::from_jval(value.get("answer")?)?,
        })
    }

    /// Does this entry sit under `key`'s own address?
    fn matches(&self, key: &CassetteKey) -> bool {
        self.label == key.label
            && self.model == key.model
            && self.schema_version == key.schema_version
            && self.digest == key.digest
            && self.image.digest() == self.digest
    }
}

/// The file format version in `value`, if it has one.
fn format_of(value: &JVal) -> Option<u16> {
    u16::try_from(value.get("format")?.as_u64()?).ok()
}

/// A directory of cassette files.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CassetteStore {
    dir: PathBuf,
}

impl CassetteStore {
    /// A store rooted at `dir`. The directory is created on the first write.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The repository's cassette directory, `fixtures/cassettes`, resolved
    /// from this crate's manifest so it works from any lane checkout.
    #[must_use]
    pub fn default_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("cassettes")
    }

    /// The directory this store reads and writes.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where `key`'s entry lives.
    #[must_use]
    pub fn path_for(&self, key: &CassetteKey) -> PathBuf {
        self.dir.join(key.file_name())
    }

    /// Write the answer for `key`, replacing any earlier entry. The write
    /// goes to a temp file first, so a crash never leaves half an entry.
    pub fn save(
        &self,
        key: &CassetteKey,
        request_id: RequestId,
        image: &RequestImage,
        answer: &Answer,
    ) -> Result<PathBuf, CassetteError> {
        let entry = Entry {
            label: key.label.clone(),
            model: key.model.clone(),
            schema_version: key.schema_version,
            digest: key.digest,
            request_id: request_id.0,
            image: image.clone(),
            answer: answer.clone(),
        };
        let text = jcompact(&entry.to_jval());
        fs::create_dir_all(&self.dir).map_err(|e| CassetteError::Io(e.kind()))?;
        let path = self.path_for(key);
        let temp = path.with_extension("json.tmp");
        let written = fs::write(&temp, text).and_then(|()| fs::rename(&temp, &path));
        if let Err(error) = written {
            let _ = fs::remove_file(&temp);
            return Err(CassetteError::Io(error.kind()));
        }
        Ok(path)
    }

    /// The answer recorded for exactly this request, or the typed reason
    /// nothing may serve it.
    pub fn load(
        &self,
        key: &CassetteKey,
        request_id: RequestId,
        image: &RequestImage,
    ) -> Result<Answer, CassetteError> {
        let path = self.path_for(key);
        let value = match read_json(&path) {
            Ok(value) => value,
            Err(CassetteError::Io(io::ErrorKind::NotFound)) => {
                return Err(CassetteError::Miss(self.explain(key, request_id, image)));
            }
            Err(error) => return Err(error),
        };
        let format = format_of(&value).ok_or(CassetteError::Corrupt)?;
        if format != CASSETTE_FORMAT {
            return Err(CassetteError::Miss(MissReason::Format { recorded: format }));
        }
        let entry = Entry::from_jval(&value).ok_or(CassetteError::Corrupt)?;
        if !entry.matches(key) {
            return Err(CassetteError::Corrupt);
        }
        if entry.image != *image {
            // A digest collision, or an entry edited by hand: refuse with the
            // parts that differ rather than serve it.
            let parts = entry.image.diff(image);
            return Err(if parts.count() == 0 {
                CassetteError::Corrupt
            } else {
                CassetteError::Miss(MissReason::Changed(parts))
            });
        }
        Ok(entry.answer)
    }

    /// Why nothing matched `key`: scan this label and model's entries for a
    /// stale or near match. Bounded; unreadable files are skipped.
    fn explain(
        &self,
        key: &CassetteKey,
        request_id: RequestId,
        image: &RequestImage,
    ) -> MissReason {
        let prefix = format!("{}__v", key.prefix());
        let Ok(listing) = fs::read_dir(&self.dir) else {
            return MissReason::NotRecorded;
        };
        let mut names: Vec<String> = listing
            .filter_map(|item| item.ok()?.file_name().into_string().ok())
            .filter(|name| name.starts_with(&prefix) && name.ends_with(".json"))
            .collect();
        names.sort();
        names.truncate(MAX_SCAN_FILES);
        let mut stale_schema = None;
        let mut stale_format = None;
        let mut closest: Option<ChangedParts> = None;
        for name in names {
            let Ok(value) = read_json(&self.dir.join(&name)) else {
                continue;
            };
            let format = format_of(&value);
            if format != Some(CASSETTE_FORMAT) {
                stale_format = stale_format.or(format);
                continue;
            }
            let Some(entry) = Entry::from_jval(&value) else {
                continue;
            };
            if entry.digest == key.digest && entry.schema_version != key.schema_version {
                stale_schema = stale_schema.or(Some(entry.schema_version));
            } else if entry.schema_version == key.schema_version && entry.request_id == request_id.0
            {
                let parts = entry.image.diff(image);
                if closest.is_none_or(|best| parts.count() < best.count()) {
                    closest = Some(parts);
                }
            }
        }
        if let Some(recorded) = stale_schema {
            MissReason::SchemaVersion { recorded }
        } else if let Some(parts) = closest {
            MissReason::Changed(parts)
        } else if let Some(recorded) = stale_format {
            MissReason::Format { recorded }
        } else {
            MissReason::NotRecorded
        }
    }
}

/// Read and parse one bounded JSON file.
fn read_json(path: &Path) -> Result<JVal, CassetteError> {
    let io_err = |e: io::Error| CassetteError::Io(e.kind());
    let meta = fs::metadata(path).map_err(io_err)?;
    if meta.len() > MAX_CASSETTE_BYTES {
        return Err(CassetteError::Corrupt);
    }
    let text = fs::read_to_string(path).map_err(io_err)?;
    jparse(&text).map_err(|_| CassetteError::Corrupt)
}

/// One thing a [`CassetteAdapter`] did, for the eval log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CassetteEvent {
    /// An answer was written.
    Recorded {
        /// Request that produced it.
        request: RequestId,
        /// Request digest it lives under.
        digest: u64,
    },
    /// An answer was served from the store.
    Replayed {
        /// Request served.
        request: RequestId,
        /// Request digest that hit.
        digest: u64,
    },
    /// Replay refused: nothing may serve this request.
    Missed {
        /// Request refused.
        request: RequestId,
        /// What the store knows about it.
        error: CassetteError,
    },
    /// Record mode left an outcome unwritten: the environment decided it,
    /// not the model, or the write failed.
    Skipped {
        /// Request whose outcome was left out.
        request: RequestId,
    },
}

/// Wraps any adapter so its answers are recorded, replayed or ignored.
#[derive(Debug)]
pub struct CassetteAdapter<A> {
    inner: A,
    store: CassetteStore,
    mode: CassetteMode,
    label: String,
    served: BTreeMap<u64, AdapterOutcome>,
    log: Vec<CassetteEvent>,
}

impl<A: ProviderAdapter> CassetteAdapter<A> {
    /// Wrap `inner`. `label` names the adapter in file names (`openai`,
    /// `local`, `mock`); it is `[a-z0-9-]`, at most 32 bytes.
    pub fn new(
        inner: A,
        store: CassetteStore,
        mode: CassetteMode,
        label: &str,
    ) -> Result<Self, CassetteError> {
        // Validate the label once, up front, so a bad one fails at wiring time.
        if !label_ok(label) {
            return Err(CassetteError::BadLabel);
        }
        Ok(Self {
            inner,
            store,
            mode,
            label: label.to_owned(),
            served: BTreeMap::new(),
            log: Vec::new(),
        })
    }

    /// The wrapped adapter.
    #[must_use]
    pub fn inner(&self) -> &A {
        &self.inner
    }

    /// The wrapped adapter, mutably.
    pub fn inner_mut(&mut self) -> &mut A {
        &mut self.inner
    }

    /// Unwrap.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.inner
    }

    /// Current mode.
    #[must_use]
    pub fn mode(&self) -> CassetteMode {
        self.mode
    }

    /// What this adapter did, oldest first, at most [`MAX_LOG_EVENTS`].
    #[must_use]
    pub fn log(&self) -> &[CassetteEvent] {
        &self.log
    }

    /// The typed reason of the latest replay refusal, if any.
    #[must_use]
    pub fn last_miss(&self) -> Option<CassetteError> {
        self.log.iter().rev().find_map(|event| match event {
            CassetteEvent::Missed { error, .. } => Some(*error),
            _ => None,
        })
    }

    fn note(&mut self, event: CassetteEvent) {
        if self.log.len() >= MAX_LOG_EVENTS {
            self.log.remove(0);
        }
        self.log.push(event);
    }

    fn key_and_image(
        &self,
        request: &AiRequest,
    ) -> Result<(CassetteKey, RequestImage), CassetteError> {
        let image = RequestImage::of(request, self.inner.request_pins());
        let key = CassetteKey::new(&self.label, request, &image)?;
        Ok((key, image))
    }

    fn record(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
        outcome: &AdapterOutcome,
    ) {
        // A request the contract refuses never reached a model: its
        // `MalformedOutput` is the caller's, not the model's.
        let answer = match validate_request(request, capabilities, policy) {
            Ok(()) => Answer::from_outcome(outcome),
            Err(_) => None,
        };
        let saved = answer.and_then(|answer| {
            let (key, image) = self.key_and_image(request).ok()?;
            self.store
                .save(&key, request.id, &image, &answer)
                .ok()
                .map(|_| key.digest())
        });
        self.note(match saved {
            Some(digest) => CassetteEvent::Recorded {
                request: request.id,
                digest,
            },
            None => CassetteEvent::Skipped {
                request: request.id,
            },
        });
    }

    fn replay(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        if let Some(first) = self.served.get(&request.id.0) {
            let mut again = first.clone();
            again.duplicate = true;
            again.mutated = false;
            again.events = Vec::new();
            // The double check covers served answers only: a stored refusal
            // logs `Missed`, never `Replayed`, so it replays as-is.
            if first.mutated {
                if let Ok((_, image)) = self.key_and_image(request) {
                    debug_assert!(check_double_fidelity(
                        first, &again, None, None, &image, &self.log
                    )
                    .is_ok());
                }
            }
            return again;
        }
        let outcome = match validate_request(request, capabilities, policy) {
            Err(error) => failed_outcome(request, error),
            Ok(()) => self.serve(request),
        };
        self.served.insert(request.id.0, outcome.clone());
        outcome
    }

    /// Serve `request` from the store or refuse it, and log which.
    fn serve(&mut self, request: &AiRequest) -> AdapterOutcome {
        let found = self.key_and_image(request).and_then(|(key, image)| {
            let answer = self.store.load(&key, request.id, &image)?;
            Ok((key.digest(), answer.into_outcome(request)?))
        });
        match found {
            Ok((digest, outcome)) => {
                self.note(CassetteEvent::Replayed {
                    request: request.id,
                    digest,
                });
                outcome
            }
            Err(error) => {
                self.note(CassetteEvent::Missed {
                    request: request.id,
                    error,
                });
                // Replay cannot serve this request: refused, and not
                // retryable (a retry would miss the same way).
                failed_outcome(request, AiError::UnsupportedCapability)
            }
        }
    }
}

impl<A: ProviderAdapter> ProviderAdapter for CassetteAdapter<A> {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        match self.mode {
            CassetteMode::Off => self.inner.call(request, policy, capabilities),
            CassetteMode::Record => {
                let outcome = self.inner.call(request, policy, capabilities);
                self.record(request, policy, capabilities, &outcome);
                outcome
            }
            CassetteMode::Replay => self.replay(request, policy, capabilities),
        }
    }

    fn request_pins(&self) -> Option<&RequestPins> {
        self.inner.request_pins()
    }
}

/// Replay double check: a replayed answer matches live, or names its miss.
/// A hit (`None`, `None`) needs the same result plus a [`CassetteEvent::Replayed`]
/// entry for the asked image's digest in `log`; a refusal (both `Some`) needs
/// the logged miss to equal the expected one and a failed replay result. Any
/// mix of the two, or a mismatch, is [`CassetteError::Corrupt`].
pub fn check_double_fidelity(
    live: &AdapterOutcome,
    replayed: &AdapterOutcome,
    expected_miss: Option<MissReason>,
    actual_miss: Option<MissReason>,
    image: &RequestImage,
    log: &[CassetteEvent],
) -> Result<(), CassetteError> {
    let digest = image.digest();
    match (expected_miss, actual_miss) {
        (None, None) => {
            if live.result != replayed.result {
                return Err(CassetteError::Corrupt);
            }
            let hit = log.iter().any(
                |e| matches!(e, CassetteEvent::Replayed { digest: seen, .. } if *seen == digest),
            );
            if !hit {
                return Err(CassetteError::Corrupt);
            }
            Ok(())
        }
        (Some(want_miss), Some(got_miss)) => {
            if want_miss != got_miss || replayed.result.is_ok() {
                return Err(CassetteError::Corrupt);
            }
            Ok(())
        }
        _ => Err(CassetteError::Corrupt),
    }
}

/// Suite record-once/replay-twice proof: the byte evidence one EV1 pass checks.
/// Consumer: the EV1 suite replay test in `crates/playtest/src/eval.rs`.
/// Units: byte distances and socket-attempt counts as integers.
/// Determinism: byte and integer compares only; no wall clock, no hash input.
/// Errors: [`CassetteError`] for wiring failures (bad label, store I/O);
/// a live record that refuses stays `Ok` with `recorded_ok` false.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecordReplayProof {
    /// True when the live record answered with a complete proposal.
    pub recorded_ok: bool,
    /// Debug text of the recorded result; replays must match it byte for byte.
    pub recorded_result: String,
    /// Debug text of each offline replay, in order.
    pub replay_results: [String; 2],
    /// Byte distance from the record to each replay; both must be 0.
    pub result_distance: [usize; 2],
    /// Byte distance between the two replays; must be 0.
    pub replay_distance: usize,
    /// Socket attempts each dead-endpoint adapter made; both must be 0.
    pub sockets: [u64; 2],
    /// Miss reason each replay surfaced; hits stay `None`.
    pub misses: [Option<MissReason>; 2],
}

/// Byte distance between two byte strings: mismatched bytes plus the length
/// gap. Zero exactly when the inputs are byte-identical.
fn byte_distance(first: &[u8], second: &[u8]) -> usize {
    let common = first.len().min(second.len());
    let mut diff = 0;
    let mut index = 0;
    while index < common {
        if first[index] != second[index] {
            diff += 1;
        }
        index += 1;
    }
    diff + first.len().max(second.len()) - common
}

#[expect(
    clippy::too_many_arguments,
    reason = "proof driver mirrors the EV1 suite call shape"
)]
/// Record `request` once through a live adapter, then replay it twice offline.
///
/// The record pass wraps `build_live` in [`CassetteMode::Record`]; each replay
/// wraps `build_dead` in [`CassetteMode::Replay`] with the network off, so a
/// hit never dials. `sockets_of` reads the inner adapter's socket counter.
/// Byte distances use the result debug text, never the whole outcome (the
/// record dispatches remote, replays do not, by design).
pub fn record_once_replay_twice<A, Live, Dead, Sockets>(
    store: &CassetteStore,
    label: &str,
    request: &AiRequest,
    policy: &AiSessionPolicy,
    capabilities: &ProviderCapabilities,
    build_live: Live,
    build_dead: Dead,
    sockets_of: Sockets,
) -> Result<RecordReplayProof, CassetteError>
where
    A: ProviderAdapter,
    Live: Fn() -> A,
    Dead: Fn() -> A,
    Sockets: Fn(&A) -> u64,
{
    let mut recorder =
        CassetteAdapter::new(build_live(), store.clone(), CassetteMode::Record, label)?;
    let live = recorder.call(request, policy, capabilities);
    let recorded_ok = live.result.is_ok();
    let recorded_result = format!("{:?}", live.result);
    let mut replay_results = [String::new(), String::new()];
    let mut result_distance = [0, 0];
    let mut sockets = [0, 0];
    let mut misses = [None, None];
    let mut slot = 0;
    while slot < 2 {
        let mut replayer =
            CassetteAdapter::new(build_dead(), store.clone(), CassetteMode::Replay, label)?;
        let out = replayer.call(request, policy, capabilities);
        let text = format!("{:?}", out.result);
        result_distance[slot] = byte_distance(recorded_result.as_bytes(), text.as_bytes());
        sockets[slot] = sockets_of(replayer.inner());
        misses[slot] = match replayer.last_miss() {
            Some(CassetteError::Miss(reason)) => Some(reason),
            _ => None,
        };
        replay_results[slot] = text;
        slot += 1;
    }
    let replay_distance = byte_distance(replay_results[0].as_bytes(), replay_results[1].as_bytes());
    Ok(RecordReplayProof {
        recorded_ok,
        recorded_result,
        replay_results,
        result_distance,
        replay_distance,
        sockets,
        misses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{fixtures, MockAdapter};
    use crate::pins::ToolDef;
    use crate::protocol::{AdapterKind, PrivacyMode};
    use crate::stub::{chat_ok, closed_port, settle, StubServer};
    use crate::transport::{parse_endpoint, Endpoint};
    use crate::vendor::VendorSecret;
    use crate::{MockStep, OpenAiAdapter};

    const NOOP: &str = r#"{"actions": [{"noop": true}], "diagnostics": ["cassette:stub"]}"#;

    /// A fresh directory under the OS temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("e2040-cassette-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }

        fn store(&self) -> CassetteStore {
            CassetteStore::new(&self.0)
        }

        fn files(&self) -> Vec<PathBuf> {
            let Ok(listing) = fs::read_dir(&self.0) else {
                return Vec::new();
            };
            let mut files: Vec<PathBuf> = listing
                .filter_map(|item| item.ok().map(|i| i.path()))
                .collect();
            files.sort();
            files
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn end(url: &str) -> Endpoint {
        parse_endpoint(url).expect("stub url parses")
    }

    fn openai(url: &str) -> OpenAiAdapter {
        OpenAiAdapter::new(
            vec![MockStep::Succeed],
            end(url),
            VendorSecret::Direct(String::from("cassette-token")),
        )
    }

    fn remote_request(id: u64) -> AiRequest {
        let mut ask = fixtures::request(id, 3);
        ask.model.adapter = AdapterKind::Remote;
        ask.model.model = String::from("openai:gpt-fixture");
        ask.privacy = PrivacyMode::RemoteAllowed;
        ask
    }

    fn remote_policy(url: &str) -> AiSessionPolicy {
        AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![url.to_owned()],
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: crate::policy::Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        }
    }

    fn dead_url() -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", closed_port())
    }

    /// Record request `id` through the loopback `stub`: the live outcome and
    /// the wrapper that recorded it.
    fn record_live(
        dir: &TempDir,
        stub: &StubServer,
        id: u64,
    ) -> (AdapterOutcome, CassetteAdapter<OpenAiAdapter>) {
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        settle(
            |_| {
                let mut rec =
                    CassetteAdapter::new(openai(&url), dir.store(), CassetteMode::Record, "openai")
                        .expect("label");
                let out = rec.call(&remote_request(id), &remote_policy(&url), &caps);
                (out, rec)
            },
            |(out, _)| out.result != Err(AiError::Unavailable),
        )
    }

    /// A replay wrapper over `inner`, which must be pointed at nothing live.
    fn replayer(dir: &TempDir, inner: OpenAiAdapter) -> CassetteAdapter<OpenAiAdapter> {
        CassetteAdapter::new(inner, dir.store(), CassetteMode::Replay, "openai").expect("label")
    }

    fn ask_via(
        w: &mut CassetteAdapter<OpenAiAdapter>,
        request: &AiRequest,
        url: &str,
    ) -> AdapterOutcome {
        let caps = ProviderCapabilities::remote_http();
        w.call(request, &remote_policy(url), &caps)
    }

    /// Suite replay evidence: result text per request per replay plus the
    /// transport counters each dead replay observed.
    struct SuiteReplay {
        first: [String; 3],
        second: [String; 3],
        sockets: [u64; 2],
        transports: [u64; 2],
    }

    /// Suite driver: record requests 21..23 once through `stub`, then replay
    /// the whole suite twice against a dead endpoint. Returns the result
    /// debug text per request per replay (the suite `to_text` evidence) plus
    /// per-round socket and transport counts, which stay 0 on a hit.
    fn replay_suite_twice(dir: &TempDir, stub: &StubServer) -> SuiteReplay {
        let mut suite = [remote_request(21), remote_request(22), remote_request(23)];
        suite[1].base_revision = 4;
        suite[2].base_revision = 5;
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        for ask in &suite {
            let live = settle(
                |_| {
                    let mut rec = CassetteAdapter::new(
                        openai(&url),
                        dir.store(),
                        CassetteMode::Record,
                        "openai",
                    )
                    .expect("label");
                    rec.call(ask, &remote_policy(&url), &caps)
                },
                |out| out.result != Err(AiError::Unavailable),
            );
            assert!(live.result.is_ok(), "suite record answers id {}", ask.id.0);
        }
        assert_eq!(dir.files().len(), 3, "suite records three entries");
        let dead = dead_url();
        let hits = stub.hits();
        let mut first = [String::new(), String::new(), String::new()];
        let mut second = [String::new(), String::new(), String::new()];
        let mut sockets = [0, 0];
        let mut transports = [0, 0];
        let mut w = replayer(&dir, openai(&dead));
        for (index, ask) in suite.iter().enumerate() {
            let out = ask_via(&mut w, ask, &dead);
            first[index] = format!("{:?}", out.result);
        }
        sockets[0] = w.inner().socket_attempts();
        transports[0] = w.inner().transport_calls();
        let mut w = replayer(&dir, openai(&dead));
        for (index, ask) in suite.iter().enumerate() {
            let out = ask_via(&mut w, ask, &dead);
            second[index] = format!("{:?}", out.result);
        }
        sockets[1] = w.inner().socket_attempts();
        transports[1] = w.inner().transport_calls();
        assert_eq!(stub.hits(), hits, "replays never reach the stub");
        assert_eq!(dir.files().len(), 3, "replay writes no file");
        SuiteReplay {
            first,
            second,
            sockets,
            transports,
        }
    }

    #[test]
    fn record_then_replay_twice_is_byte_identical_with_no_transport() {
        let dir = TempDir::new("roundtrip");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, Some((11, 5))));
        let (live, recorder) = record_live(&dir, &stub, 1);
        assert!(live.result.is_ok(), "live exchange through the stub");
        assert!(live.dispatched_remote);
        assert!(matches!(recorder.log(), [CassetteEvent::Recorded { .. }]));
        let files = dir.files();
        assert_eq!(files.len(), 1, "one entry on disk");
        let before = fs::read(&files[0]).expect("entry reads");
        let hits = stub.hits();

        let dead = dead_url();
        let mut replays = Vec::new();
        for _ in 0..2 {
            let mut w = replayer(&dir, openai(&dead));
            let out = ask_via(&mut w, &remote_request(1), &dead);
            replays.push((w, out));
        }
        let (first, second) = (&replays[0].1, &replays[1].1);
        assert_eq!(
            format!("{first:?}"),
            format!("{second:?}"),
            "byte-identical x2"
        );
        assert_eq!(first.result, live.result, "the live answer, unchanged");
        assert!(!first.dispatched_remote, "replay dispatches nothing");
        assert!(first.mutated && !first.duplicate);
        for (w, _) in &replays {
            assert_eq!(w.inner().socket_attempts(), 0, "no socket attempt");
            assert_eq!(w.inner().transport_calls(), 0, "no exchange");
            assert!(matches!(w.log(), [CassetteEvent::Replayed { .. }]));
        }
        assert_eq!(stub.hits(), hits, "the stub saw nothing during replay");
        assert_eq!(fs::read(&files[0]).expect("entry reads"), before);
        assert_eq!(dir.files().len(), 1, "replay writes no file");
    }

    #[test]
    fn replay_hits_by_content_under_the_asking_request_id() {
        let dir = TempDir::new("by-content");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, Some((11, 5))));
        let (live, _) = record_live(&dir, &stub, 1);
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let out = ask_via(&mut w, &remote_request(900), &dead);
        assert!(
            matches!(
                out.events.as_slice(),
                [AiEvent {
                    request: RequestId(900),
                    ..
                }]
            ),
            "events carry the asking id"
        );
        let result = out.result.expect("hit under a fresh id");
        assert_eq!(result.request, RequestId(900));
        assert_eq!(result.usage.input_tokens, Some(11));
        assert_eq!(result.usage.output_tokens, Some(5));
        let proposal = result.proposal.expect("complete proposal");
        assert_eq!(proposal.request, RequestId(900));
        let recorded = live.result.expect("live ok").proposal.expect("proposal");
        assert_eq!(proposal.actions, recorded.actions);
        assert_eq!(proposal.diagnostics, recorded.diagnostics);
    }

    #[test]
    fn mock_answers_record_and_replay_without_consulting_the_inner_adapter() {
        let dir = TempDir::new("mock");
        let (policy, caps) = (fixtures::policy(), fixtures::caps());
        let mut rec = CassetteAdapter::new(
            MockAdapter::new(Vec::new()),
            dir.store(),
            CassetteMode::Record,
            "mock",
        )
        .expect("label");
        let live = rec.call(&fixtures::request(1, 3), &policy, &caps);
        assert!(live.result.is_ok());
        // An inner mock that would refuse every call: a hit never reaches it.
        let mut rep = CassetteAdapter::new(
            MockAdapter::new(vec![MockStep::QuotaAuth; 4]),
            dir.store(),
            CassetteMode::Replay,
            "mock",
        )
        .expect("label");
        let out = rep.call(&fixtures::request(9, 3), &policy, &caps);
        let proposal = out.result.expect("hit").proposal.expect("proposal");
        assert_eq!(proposal.actions, vec![ProposalAction::Noop]);
        assert!(proposal.diagnostics.contains(&String::from("mock:ok")));
        assert_eq!(rep.inner().script_remaining(), 4, "inner mock untouched");
    }

    #[test]
    fn a_changed_prompt_misses_and_says_why() {
        let dir = TempDir::new("changed");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let (live, _) = record_live(&dir, &stub, 7);
        assert!(live.result.is_ok());
        let dead = dead_url();
        let pinned = |pins: RequestPins| openai(&dead).with_pins(pins);
        let tool =
            ToolDef::new("move", "Move one.", crate::ladder::proposal_schema()).expect("tool");
        let only = |f: fn(&mut ChangedParts)| {
            let mut parts = ChangedParts::default();
            f(&mut parts);
            parts
        };
        let mut revised = remote_request(7);
        revised.base_revision = 4;
        let cases: Vec<(&str, AiRequest, OpenAiAdapter, ChangedParts, &str)> = vec![
            (
                "user prompt",
                revised,
                openai(&dead),
                only(|p| p.user = true),
                "user prompt",
            ),
            (
                "system prompt",
                remote_request(7),
                pinned(RequestPins::default().with_system("Other.").expect("pin")),
                only(|p| p.system = true),
                "system prompt",
            ),
            (
                "temperature",
                remote_request(7),
                pinned(
                    RequestPins::default()
                        .with_temperature_milli(500)
                        .expect("pin"),
                ),
                only(|p| p.sampling = true),
                "sampling",
            ),
            (
                "seed",
                remote_request(7),
                pinned(RequestPins::default().with_seed(Some(9))),
                only(|p| p.sampling = true),
                "sampling",
            ),
            (
                "ladder start",
                remote_request(7),
                pinned(RequestPins::default().with_ladder(crate::ladder::LadderPlan::native())),
                only(|p| p.structured = true),
                "structured output",
            ),
            (
                "tools",
                remote_request(7),
                pinned(RequestPins::default().with_tools(vec![tool]).expect("pin")),
                only(|p| p.tools = true),
                "tools",
            ),
        ];
        for (label, ask, inner, parts, word) in cases {
            let mut w = replayer(&dir, inner);
            let out = ask_via(&mut w, &ask, &dead);
            assert_eq!(out.result, Err(AiError::UnsupportedCapability), "{label}");
            assert!(!out.mutated && !out.dispatched_remote, "{label}");
            assert_eq!(w.inner().socket_attempts(), 0, "{label}: no transport");
            let want = CassetteError::Miss(MissReason::Changed(parts));
            assert_eq!(w.last_miss(), Some(want), "{label}");
            assert!(want.to_string().contains(word), "{label}: {want}");
        }
        assert!(
            !AiError::UnsupportedCapability.retryable(),
            "a miss is final"
        );
    }

    #[test]
    fn an_unseen_request_is_not_recorded_and_never_reaches_the_inner_adapter() {
        let dir = TempDir::new("unseen");
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let out = ask_via(&mut w, &remote_request(1), &dead);
        assert_eq!(out.result, Err(AiError::UnsupportedCapability));
        let not_recorded = Some(CassetteError::Miss(MissReason::NotRecorded));
        assert_eq!(w.last_miss(), not_recorded, "no store at all");
        assert_eq!(w.inner().socket_attempts(), 0);

        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let (live, _) = record_live(&dir, &stub, 1);
        assert!(live.result.is_ok());
        let mut other = remote_request(2);
        other.base_revision = 9;
        let mut w = replayer(&dir, openai(&dead));
        ask_via(&mut w, &other, &dead);
        assert_eq!(
            w.last_miss(),
            not_recorded,
            "another id and another prompt: nothing to compare against"
        );
        let mut other_model = remote_request(1);
        other_model.model.model = String::from("openai:gpt-other");
        ask_via(&mut w, &other_model, &dead);
        assert_eq!(
            w.last_miss(),
            not_recorded,
            "another model, its own entries"
        );
    }

    #[test]
    fn stale_schema_and_format_are_refused_with_their_reason() {
        let dir = TempDir::new("stale");
        let store = dir.store();
        let ask = remote_request(7);
        let image = RequestImage::of(&ask, None);
        let key = CassetteKey::new("openai", &ask, &image).expect("key");
        store
            .save(&key, ask.id, &image, &Answer::Malformed)
            .expect("save");
        assert_eq!(store.load(&key, ask.id, &image), Ok(Answer::Malformed));

        let mut newer = ask.clone();
        newer.schema_version = 2;
        let newer_key = CassetteKey::new("openai", &newer, &image).expect("key");
        assert_eq!(
            store.load(&newer_key, newer.id, &image),
            Err(CassetteError::Miss(MissReason::SchemaVersion {
                recorded: 1
            })),
            "same request under a newer schema is stale, not reused"
        );

        let path = store.path_for(&key);
        let text = fs::read_to_string(&path).expect("entry reads");
        fs::write(&path, text.replacen("\"format\": 1", "\"format\": 9", 1)).expect("edit");
        assert_eq!(
            store.load(&key, ask.id, &image),
            Err(CassetteError::Miss(MissReason::Format { recorded: 9 }))
        );
        assert_eq!(
            MissReason::Format { recorded: 9 }.to_string(),
            "entry written by cassette format 9"
        );
    }

    #[test]
    fn corrupt_and_tampered_entries_are_never_served() {
        let dir = TempDir::new("corrupt");
        let store = dir.store();
        let ask = remote_request(7);
        let image = RequestImage::of(&ask, None);
        let key = CassetteKey::new("openai", &ask, &image).expect("key");
        let path = store
            .save(&key, ask.id, &image, &Answer::Malformed)
            .expect("save");
        let good = fs::read_to_string(&path).expect("entry reads");

        fs::write(&path, "{ not json").expect("edit");
        assert_eq!(
            store.load(&key, ask.id, &image),
            Err(CassetteError::Corrupt)
        );

        let forged = good.replacen(&format!("{:016x}", key.digest()), "0000000000000001", 1);
        fs::write(&path, forged).expect("edit");
        assert_eq!(
            store.load(&key, ask.id, &image),
            Err(CassetteError::Corrupt),
            "an entry filed under another address is refused"
        );

        let too_big = usize::try_from(MAX_CASSETTE_BYTES).expect("fits") + 1;
        fs::write(&path, vec![b' '; too_big]).expect("edit");
        assert_eq!(
            store.load(&key, ask.id, &image),
            Err(CassetteError::Corrupt)
        );
    }

    #[test]
    fn a_hand_edited_proposal_that_breaks_the_grammar_is_refused_at_replay() {
        let dir = TempDir::new("evil");
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let ask = remote_request(7);
        // File the forged entry where the wrapper will look: keyed on its
        // adapter's own pins.
        let image = RequestImage::of(&ask, w.request_pins());
        let key = CassetteKey::new("openai", &ask, &image).expect("key");
        let evil = Answer::Complete {
            proposal: jparse(r#"{"actions": [{"teleport": {"object": 1}}], "diagnostics": []}"#)
                .expect("evil proposal parses"),
            usage: Usage::default(),
        };
        dir.store().save(&key, ask.id, &image, &evil).expect("save");
        let out = ask_via(&mut w, &ask, &dead);
        assert_eq!(out.result, Err(AiError::UnsupportedCapability));
        assert_eq!(w.last_miss(), Some(CassetteError::Corrupt));
        assert!(!out.mutated, "nothing applied");
    }

    #[test]
    fn model_garbage_is_recorded_and_replays_as_malformed() {
        let dir = TempDir::new("garbage");
        let stub = StubServer::spawn_raw(200, String::from("this is not json"));
        let (live, rec) = record_live(&dir, &stub, 1);
        assert_eq!(live.result, Err(AiError::MalformedOutput));
        assert!(matches!(rec.log(), [CassetteEvent::Recorded { .. }]));
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let out = ask_via(&mut w, &remote_request(1), &dead);
        assert_eq!(out.result, Err(AiError::MalformedOutput));
        assert!(!out.mutated);
        assert!(matches!(w.log(), [CassetteEvent::Replayed { .. }]));
        assert_eq!(w.inner().socket_attempts(), 0);
    }

    #[test]
    fn record_leaves_out_what_the_environment_decided() {
        let dir = TempDir::new("skips");
        let quota = StubServer::spawn_raw(429, String::from("{}"));
        let (out, rec) = record_live(&dir, &quota, 1);
        assert_eq!(out.result, Err(AiError::QuotaAuth));
        assert!(matches!(rec.log(), [CassetteEvent::Skipped { .. }]));

        // A request the contract refuses never reached a model: its
        // `MalformedOutput` belongs to the caller and must not be filed.
        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let url = stub.url();
        let mut rec =
            CassetteAdapter::new(openai(&url), dir.store(), CassetteMode::Record, "openai")
                .expect("label");
        let mut bad = remote_request(2);
        bad.schema_version = 2;
        let out = ask_via(&mut rec, &bad, &url);
        assert_eq!(out.result, Err(AiError::MalformedOutput));
        assert!(matches!(rec.log(), [CassetteEvent::Skipped { .. }]));
        assert!(
            dir.files().is_empty(),
            "nothing the model said, nothing filed"
        );
    }

    #[test]
    fn off_passes_through_and_writes_nothing() {
        let dir = TempDir::new("off");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let url = stub.url();
        let mut w = CassetteAdapter::new(openai(&url), dir.store(), CassetteMode::Off, "openai")
            .expect("label");
        let out = ask_via(&mut w, &remote_request(1), &url);
        assert!(out.result.is_ok(), "the live adapter answered");
        assert!(w.log().is_empty());
        assert!(dir.files().is_empty());
        assert!(w.request_pins().is_some(), "pins reach the recorder");
        assert_eq!(w.mode(), CassetteMode::Off);
    }

    #[test]
    fn replay_still_enforces_the_session_contract() {
        let dir = TempDir::new("contract");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let (live, _) = record_live(&dir, &stub, 1);
        assert!(live.result.is_ok());
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let mut policy = remote_policy(&dead);
        policy.mode = PrivacyMode::LocalOnly;
        let caps = ProviderCapabilities::remote_http();
        let out = w.call(&remote_request(1), &policy, &caps);
        assert_eq!(out.result, Err(AiError::UnsupportedCapability));
        assert!(w.log().is_empty(), "refused before any lookup");
        let mut stale = remote_request(2);
        stale.schema_version = 2;
        let out = w.call(&stale, &remote_policy(&dead), &caps);
        assert_eq!(out.result, Err(AiError::MalformedOutput));
    }

    #[test]
    fn replaying_one_id_twice_returns_the_first_answer_as_a_duplicate() {
        let dir = TempDir::new("dup");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let (live, _) = record_live(&dir, &stub, 1);
        assert!(live.result.is_ok());
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let first = ask_via(&mut w, &remote_request(1), &dead);
        let again = ask_via(&mut w, &remote_request(1), &dead);
        assert!(!first.duplicate && again.duplicate);
        assert!(!again.mutated && again.events.is_empty());
        assert_eq!(first.result, again.result);
        assert_eq!(w.log().len(), 1, "looked up once");
    }

    #[test]
    fn labels_and_model_names_cannot_leave_the_store() {
        let dir = TempDir::new("paths");
        let ask = remote_request(1);
        let image = RequestImage::of(&ask, None);
        let long = "a".repeat(MAX_LABEL_LEN + 1);
        for bad in ["", "Bad", "has/slash", "dots..", long.as_str()] {
            assert_eq!(
                CassetteKey::new(bad, &ask, &image).unwrap_err(),
                CassetteError::BadLabel,
                "{bad:?}"
            );
        }
        let mut sneaky = ask.clone();
        sneaky.model.model = String::from("../../evil:model\\x");
        let key = CassetteKey::new("openai", &sneaky, &image).expect("key");
        let path = dir.store().path_for(&key);
        assert_eq!(path.parent(), Some(dir.store().dir()));
        let name = path.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(
            !name.contains(['/', '\\', ':']) && !name.contains(".."),
            "{name}"
        );
        // Two model names that slug alike still get separate files.
        let mut twin = ask;
        twin.model.model = String::from("______evil_model_x");
        let twin_key = CassetteKey::new("openai", &twin, &image).expect("key");
        assert_ne!(dir.store().path_for(&twin_key), path);
    }

    #[test]
    fn modes_parse_by_name() {
        assert_eq!(CassetteMode::parse("record"), Some(CassetteMode::Record));
        assert_eq!(CassetteMode::parse("replay"), Some(CassetteMode::Replay));
        assert_eq!(CassetteMode::parse("off"), Some(CassetteMode::Off));
        assert_eq!(CassetteMode::parse("auto"), None);
    }

    #[test]
    fn opt24_double_live_and_replay_match() {
        let dir = TempDir::new("opt24-double-hit");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, Some((11, 5))));
        let (live, recorder) = record_live(&dir, &stub, 1);
        assert!(live.result.is_ok());
        let ask = remote_request(1);
        let image = RequestImage::of(&ask, recorder.inner().request_pins());
        let dead = dead_url();
        let mut log = recorder.log().to_vec();
        let mut w = replayer(&dir, openai(&dead));
        let out = ask_via(&mut w, &remote_request(1), &dead);
        log.extend_from_slice(w.log());
        assert_eq!(out.result, live.result);
        assert!(!out.dispatched_remote);
        assert_eq!(w.inner().socket_attempts(), 0);
        assert!(check_double_fidelity(&live, &out, None, None, &image, &log).is_ok());
    }

    #[test]
    fn opt24_double_changed_prompt_names_parts() {
        let dir = TempDir::new("opt24-double-changed");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, None));
        let (live, recorder) = record_live(&dir, &stub, 7);
        assert!(live.result.is_ok());
        let live_req = remote_request(7);
        let live_image = RequestImage::of(&live_req, recorder.inner().request_pins());
        let dead = dead_url();
        let mut changed = remote_request(7);
        changed.base_revision = 4;
        let mut w = replayer(&dir, openai(&dead));
        let out = ask_via(&mut w, &changed, &dead);
        assert_eq!(out.result, Err(AiError::UnsupportedCapability));
        let expected = ChangedParts {
            user: true,
            ..Default::default()
        };
        assert_eq!(
            w.last_miss(),
            Some(CassetteError::Miss(MissReason::Changed(expected)))
        );
        let mut combined = recorder.log().to_vec();
        combined.extend_from_slice(w.log());
        let actual = match w.last_miss() {
            Some(CassetteError::Miss(reason)) => Some(reason),
            _ => None,
        };
        assert!(check_double_fidelity(
            &live,
            &out,
            Some(MissReason::Changed(expected)),
            actual,
            &live_image,
            &combined
        )
        .is_ok());
        assert!(check_double_fidelity(&live, &out, None, None, &live_image, &combined).is_err());
    }

    #[test]
    fn record_once_replay_twice_is_byte_identical_without_dialing() {
        let dir = TempDir::new("opt24-proof");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, Some((11, 5))));
        let url = stub.url();
        let dead = dead_url();
        let ask = remote_request(1);
        let caps = ProviderCapabilities::remote_http();
        let proof = record_once_replay_twice(
            &dir.store(),
            "openai",
            &ask,
            &remote_policy(&url),
            &caps,
            || openai(&url),
            || openai(&dead),
            |adapter| adapter.socket_attempts(),
        )
        .expect("record answers live");
        assert!(proof.recorded_ok, "the live record must answer");
        assert_eq!(proof.result_distance, [0, 0], "replays match the record");
        assert_eq!(proof.replay_distance, 0, "the two replays match");
        assert_eq!(proof.sockets, [0, 0], "replay dials nothing");
        assert_eq!(proof.misses, [None, None], "replays hit, never miss");
    }

    #[test]
    fn replaying_a_miss_twice_replays_the_refusal_without_tripping_the_double_check() {
        let dir = TempDir::new("miss-twice");
        let dead = dead_url();
        let mut w = replayer(&dir, openai(&dead));
        let ask = remote_request(1);
        let first = ask_via(&mut w, &ask, &dead);
        assert_eq!(
            first.result,
            Err(AiError::UnsupportedCapability),
            "an unrecorded request is refused"
        );
        assert_eq!(w.inner().socket_attempts(), 0, "a miss dials nothing");
        let again = ask_via(&mut w, &ask, &dead);
        assert_eq!(
            again.result, first.result,
            "the stored refusal replays as-is"
        );
        assert!(again.duplicate, "the second serve is a duplicate");
        assert_eq!(
            w.last_miss(),
            Some(CassetteError::Miss(MissReason::NotRecorded)),
            "the refusal reason survives the duplicate"
        );
    }

    #[test]
    fn eval_suite_records_once_replays_twice_byte_identical() {
        let dir = TempDir::new("opt24-suite");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, Some((11, 5))));
        let proof = replay_suite_twice(&dir, &stub);
        assert_eq!(proof.first, proof.second, "two replays byte-identical");
        for index in 0..3 {
            assert_eq!(
                proof.first[index], proof.second[index],
                "suite request {index} replays"
            );
            assert!(!proof.first[index].is_empty(), "record answered");
        }
        assert_eq!(proof.sockets, [0, 0], "replay dials nothing");
        assert_eq!(proof.transports, [0, 0], "replay exchanges nothing");
        assert_eq!(dir.files().len(), 3, "replay writes no file");
    }

    #[test]
    fn eval_suite_changed_prompt_misses_with_reason() {
        let dir = TempDir::new("opt24-suite-miss");
        let stub = StubServer::spawn_completion(chat_ok(NOOP, Some((11, 5))));
        let proof = replay_suite_twice(&dir, &stub);
        assert_eq!(proof.first, proof.second, "suite records before the miss");
        let dead = dead_url();
        let mut altered = remote_request(22);
        altered.base_revision = 99;
        let mut w = replayer(&dir, openai(&dead));
        let out = ask_via(&mut w, &altered, &dead);
        assert_eq!(
            out.result,
            Err(AiError::UnsupportedCapability),
            "altered suite prompt misses"
        );
        let want = ChangedParts {
            user: true,
            ..Default::default()
        };
        let miss = CassetteError::Miss(MissReason::Changed(want));
        assert_eq!(w.last_miss(), Some(miss), "miss names the user prompt");
        assert!(
            miss.to_string().contains("user prompt"),
            "reason says why: {miss}"
        );
        assert_eq!(w.inner().socket_attempts(), 0, "a miss dials nothing");
        assert_eq!(w.inner().transport_calls(), 0, "a miss exchanges nothing");
        assert!(
            !out.mutated && !out.dispatched_remote,
            "nothing reused silently"
        );
    }

    #[test]
    fn cassette_slice1_changed_and_stale_refuse_with_reason() {
        let dir = TempDir::new("cassette-slice1");
        let store = dir.store();
        let ask = fixtures::request(7, 3);
        let image = RequestImage::of(&ask, None);
        let key = CassetteKey::new("mock", &ask, &image).expect("key");
        store
            .save(&key, ask.id, &image, &Answer::Malformed)
            .expect("save");
        let mut changed = ask.clone();
        changed.base_revision = 4;
        let changed_image = RequestImage::of(&changed, None);
        let changed_key = CassetteKey::new("mock", &changed, &changed_image).expect("key");
        let want_parts = ChangedParts {
            user: true,
            ..Default::default()
        };
        let want = CassetteError::Miss(MissReason::Changed(want_parts));
        assert_eq!(
            store.load(&changed_key, changed.id, &changed_image),
            Err(want),
            "changed prompt misses and says why"
        );
        let mut stale = ask.clone();
        stale.schema_version = 2;
        let stale_key = CassetteKey::new("mock", &stale, &image).expect("key");
        assert_eq!(
            store.load(&stale_key, stale.id, &image),
            Err(CassetteError::Miss(MissReason::SchemaVersion {
                recorded: 1
            })),
            "stale schema refuses, never reused"
        );
        let mut rep = CassetteAdapter::new(
            MockAdapter::new(vec![MockStep::QuotaAuth; 4]),
            store,
            CassetteMode::Replay,
            "mock",
        )
        .expect("label");
        let out = rep.call(&changed, &fixtures::policy(), &fixtures::caps());
        assert_eq!(out.result, Err(AiError::UnsupportedCapability));
        assert_eq!(rep.last_miss(), Some(want), "replay refuses with reason");
        assert_eq!(rep.inner().script_remaining(), 4, "inner never called");
        assert!(!out.mutated && !out.dispatched_remote);
    }
}
