//! memory: typed facts behind the session's own capability token.
//!
//! Owner: `engine-endpoint`; consumer: an agent that wants to write a fact,
//! recall what its session knows, and share one fact with other sessions. The
//! store and its gate are `engine_provider::memory`; this pack is the door on
//! top of it and adds no second rule.
//!
//! A write presents the session's capability token: the token folded from the
//! capability's own fields ([`memory_key`]) plus the `memory.write` grant. A
//! capability narrower than the door's full token carries no grant, so a
//! restricted session is refused with the store's own `memory_not_granted`.
//! Every refusal carries the store's code unchanged (`memory_no_key`,
//! `memory_wrong_key`, `memory_not_granted`, `memory_value_too_long`,
//! `memory_bad_confidence`, `memory_no_fact`, `memory_not_yours`).
//!
//! Recall scope and order are the store's: a session's own facts plus the ones
//! another session shared, in `(type, tick, id)` order, never a clock.
//!
//! `memory.record` is the fourth verb and a different store: the lesson rows
//! `tools/memory/memory.mjs` keeps, one JSONL file the whole repo shares (BG14).
//! Its rules are the tool's rules, held in [`crate::memory_store`]: one of three
//! kinds, a source path under the repo, a caller-supplied UTC stamp and — the
//! point of the row — the 16 hex digits of the tape hash the lesson was learned
//! under. A row without one is refused with `record_bad_hash`, and the file is
//! left exactly as it was. The same store reads back as the `e2040://memory`
//! resource, filtered by `kind`, `path` and `hash`, in stamp order, one canonical
//! JSON line per row, so the door and the tool answer the same bytes.
//!
//! Units: confidence in thousandths, ticks as write counts, bytes for the
//! value bound. Determinism: the same writes in the same order always produce
//! the same store; the table lives until the door restarts.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use engine_editor::capability::{Capability, OpMask};
use engine_provider::memory::{
    Draft, FactType, MemoryError, MemoryKey, MemoryStore, Source, MEMORY_WRITE,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::memory_store::{self, StoreError};
use crate::resources::MEMORY_URI;
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "memory";
/// Verb: write one typed fact under the session's capability token.
pub const TOOL_MEMORY_WRITE: &str = "memory.write";
/// Verb: recall the session's own facts plus every shared fact.
pub const TOOL_MEMORY_RECALL: &str = "memory.recall";
/// Verb: mark one of the session's own facts shared.
pub const TOOL_MEMORY_SHARE: &str = "memory.share";
/// Verb: append one lesson row to the repo's memory store.
pub const TOOL_MEMORY_RECORD: &str = "memory.record";

/// The process-wide store plus the sessions already registered in it.
fn table() -> &'static Mutex<(MemoryStore, BTreeSet<u64>)> {
    static TABLE: OnceLock<Mutex<(MemoryStore, BTreeSet<u64>)>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new((MemoryStore::new(), BTreeSet::new())))
}

fn lock_table() -> Result<MutexGuard<'static, (MemoryStore, BTreeSet<u64>)>, McpToolError> {
    table().lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "the memory table is poisoned; restart the door",
        )
    })
}

/// The session's capability as a store key: the token folds the capability's
/// own fields, so the same token always presents the same bytes and another
/// session's token never matches. Only the door's full token (every op class)
/// carries `memory.write`; a narrower token is refused by the store.
fn memory_key(cap: &Capability) -> MemoryKey {
    let mut fold = engine_core::Fnv64::new();
    fold.write_u64(cap.session);
    fold.write_u32(u32::from(cap.edit_budget.min(u32::MAX)));
    fold.write_u8(cap.ops.0);
    let grants = if cap.ops.contains(OpMask::ALL) {
        MEMORY_WRITE
    } else {
        0
    };
    MemoryKey::new(cap.session, fold.finish(), grants)
}

/// One store refusal as the door's typed error, code unchanged, with the one
/// recovery step that clears it.
fn refusal(error: MemoryError) -> McpToolError {
    let next = match error {
        MemoryError::NoKey | MemoryError::WrongKey | MemoryError::NotGranted => {
            "next: write from the session that holds the token; a narrower token never writes"
        }
        MemoryError::ValueTooLong { .. } => "next: shorten the value",
        MemoryError::BadConfidence { .. } => "next: confidence is 0..=1000",
        MemoryError::NoFact { .. } => "next: recall first and share an id you saw",
        MemoryError::NotYours { .. } => "next: only the session that wrote a fact shares it",
        MemoryError::Malformed { .. } | MemoryError::SequenceExhausted => "next: restart the door",
    };
    McpToolError::new(error.code(), format!("{error}; {next}"))
        .with_data(rmcp::serde_json::json!({ "code": error.code() }))
}

/// What `memory.write` offers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryWriteArgs {
    /// Fact type: `fact`, `preference` or `decision`.
    pub kind: String,
    /// The value itself, at most 512 bytes.
    pub value: String,
    /// Confidence in `0..=1000`; default 1000.
    pub confidence: Option<u16>,
    /// Turn inside the session that stated the fact; default 0.
    pub turn: Option<u32>,
    /// True when every session may recall it; default false.
    pub shared: Option<bool>,
}

/// One stored fact as the door reports it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryFactDto {
    /// Stable id, equal to the write tick.
    pub id: u64,
    /// Declared type: `fact`, `preference` or `decision`.
    pub kind: String,
    /// The value itself.
    pub value: String,
    /// Confidence in `0..=1000`.
    pub confidence: u16,
    /// Session that wrote it.
    pub session: u64,
    /// Turn inside that session.
    pub turn: u32,
    /// Write tick, starting at 1.
    pub tick: u64,
    /// True when every session may recall it.
    pub shared: bool,
    /// True when this session wrote it.
    pub own: bool,
}

/// What `memory.write` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryWriteOutput {
    /// The written fact.
    pub fact: MemoryFactDto,
    /// Facts the store retains after this write.
    pub stored: u32,
}

/// `memory.recall` takes no argument: the session is the scope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryRecallArgs {}

/// What `memory.recall` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryRecallOutput {
    /// Recalled facts in the store's order: type, then tick, then id.
    pub facts: Vec<MemoryFactDto>,
    /// How many of them this session wrote.
    pub own: u32,
    /// How many of them another session shared.
    pub shared: u32,
}

/// What `memory.share` offers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryShareArgs {
    /// Id of one of the session's own facts.
    pub id: u64,
}

/// What `memory.share` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryShareOutput {
    /// The fact, now shared.
    pub fact: MemoryFactDto,
}

/// What `memory.record` offers: one lesson row, in the fields the memory tool
/// takes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryRecordArgs {
    /// Lesson kind: `decision`, `failed-fix` or `tuned-value`.
    pub kind: String,
    /// The 16 hex digits of the tape hash (or golden) the lesson was learned
    /// under; `0x` may prefix it. A row without one is refused.
    pub hash: String,
    /// The source path the lesson is about, repo-relative.
    pub path: String,
    /// The lesson itself.
    pub text: String,
    /// When the lesson was taken: `YYYY-MM-DDTHH:MM:SSZ` in UTC. The door keeps
    /// no clock, so the caller states it.
    pub utc: String,
    /// The run id or commit that proves it.
    pub evidence: Option<String>,
    /// An alternative JSONL store to append to; default
    /// `tools/memory/memory.jsonl` under the repo root.
    pub store: Option<String>,
}

/// What `memory.record` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MemoryRecordOutput {
    /// The row as the store holds it, with its derived id.
    pub row: memory_store::Record,
    /// Rows the store holds after this write.
    pub rows: u32,
    /// The resource read that shows the row.
    pub uri: String,
}

/// One fact as the door reports it for `viewer`.
fn fact_dto(fact: &engine_provider::memory::Fact, viewer: u64) -> MemoryFactDto {
    MemoryFactDto {
        id: fact.id,
        kind: fact.kind.name().to_owned(),
        value: fact.value.clone(),
        confidence: fact.confidence,
        session: fact.source.session,
        turn: fact.source.turn,
        tick: fact.tick,
        shared: fact.shared,
        own: fact.source.session == viewer,
    }
}

/// Register the session's key once; a later narrower token is then a wrong key
/// by the store's own rule rather than a silent re-grant.
fn key_for(table: &mut (MemoryStore, BTreeSet<u64>), cap: &Capability) -> MemoryKey {
    let key = memory_key(cap);
    let (store, registered) = table;
    if registered.insert(key.session()) {
        store.grant(key);
    }
    key
}

/// `memory.write` handler: one typed fact under the session's token.
fn memory_write(
    service: &McpServer,
    args: &MemoryWriteArgs,
) -> Result<MemoryWriteOutput, McpToolError> {
    service.dispatch(TOOL_MEMORY_WRITE, |state| memory_write_in(state, args))
}

/// `memory.write` body: the store call and the receipt.
fn memory_write_in(
    state: &mut crate::mcp::ServerState,
    args: &MemoryWriteArgs,
) -> Result<MemoryWriteOutput, McpToolError> {
    let kind = FactType::from_name(&args.kind).ok_or_else(|| {
        McpToolError::new(
            "bad_kind",
            format!(
                "`{}` is not a fact type; next: write kind `fact`, `preference` or `decision`",
                args.kind
            ),
        )
    })?;
    let viewer = state.session();
    let draft = Draft {
        kind,
        value: args.value.clone(),
        confidence: args
            .confidence
            .unwrap_or(engine_provider::memory::MAX_CONFIDENCE),
        source: Source {
            session: viewer,
            turn: args.turn.unwrap_or(0),
        },
        shared: args.shared.unwrap_or(false),
    };
    let mut guard = lock_table()?;
    let key = key_for(&mut guard, &state.cap);
    let (store, _) = &mut *guard;
    let fact = store.write(Some(&key), draft).map_err(refusal)?;
    let dto = fact_dto(fact, viewer);
    let stored = u32::try_from(store.len()).unwrap_or(u32::MAX);
    Ok(MemoryWriteOutput { fact: dto, stored })
}

/// `memory.recall` handler: the session's own facts plus every shared fact.
fn memory_recall(
    service: &McpServer,
    _args: &MemoryRecallArgs,
) -> Result<MemoryRecallOutput, McpToolError> {
    service.dispatch(TOOL_MEMORY_RECALL, memory_recall_in)
}

/// `memory.recall` body: the store's scoped read.
fn memory_recall_in(
    state: &mut crate::mcp::ServerState,
) -> Result<MemoryRecallOutput, McpToolError> {
    let viewer = state.session();
    let guard = lock_table()?;
    let (store, _) = &*guard;
    let facts: Vec<MemoryFactDto> = store
        .recall(viewer)
        .into_iter()
        .map(|fact| fact_dto(fact, viewer))
        .collect();
    let own = facts.iter().filter(|fact| fact.own).count();
    Ok(MemoryRecallOutput {
        own: u32::try_from(own).unwrap_or(u32::MAX),
        shared: u32::try_from(facts.len().saturating_sub(own)).unwrap_or(u32::MAX),
        facts,
    })
}

/// `memory.share` handler: mark one of the session's own facts shared.
fn memory_share(
    service: &McpServer,
    args: &MemoryShareArgs,
) -> Result<MemoryShareOutput, McpToolError> {
    service.dispatch(TOOL_MEMORY_SHARE, |state| memory_share_in(state, args))
}

/// `memory.share` body: the store's share gate, unchanged.
fn memory_share_in(
    state: &mut crate::mcp::ServerState,
    args: &MemoryShareArgs,
) -> Result<MemoryShareOutput, McpToolError> {
    let viewer = state.session();
    let mut guard = lock_table()?;
    let (store, _) = &mut *guard;
    let fact = store.share(viewer, args.id).map_err(refusal)?;
    Ok(MemoryShareOutput {
        fact: fact_dto(fact, viewer),
    })
}

/// One store refusal as the door's typed error: the rule that failed, named.
///
/// Each arm spells its code at the constructor: the door keeps one refusal
/// classification and it reads the literals the crate produces, so a code that
/// only ever reaches the wire through a variable is a code no model can
/// branch on.
fn store_refusal(error: StoreError) -> McpToolError {
    match error {
        StoreError::BadKind { .. } => McpToolError::new(
            "record_bad_kind",
            format!("{error}; next: kind is `decision`, `failed-fix` or `tuned-value`"),
        )
        .with_data(rmcp::serde_json::json!({ "code": "record_bad_kind" })),
        StoreError::BadHash { .. } => McpToolError::new(
            "record_bad_hash",
            format!(
                "{error}; next: pass the 16 hex digits of the tape digest or the golden the \
                 lesson was learned on"
            ),
        )
        .with_data(rmcp::serde_json::json!({ "code": "record_bad_hash" })),
        StoreError::BadPath { .. } => McpToolError::new(
            "record_bad_path",
            format!("{error}; next: path is repo-relative, such as crates/anim/src/motion_bake.rs"),
        )
        .with_data(rmcp::serde_json::json!({ "code": "record_bad_path" })),
        StoreError::EmptyText => McpToolError::new(
            "record_empty_text",
            format!("{error}; next: say what the lesson learned"),
        )
        .with_data(rmcp::serde_json::json!({ "code": "record_empty_text" })),
        StoreError::BadStamp { .. } => McpToolError::new(
            "record_bad_stamp",
            format!("{error}; next: utc is YYYY-MM-DDTHH:MM:SSZ in UTC"),
        )
        .with_data(rmcp::serde_json::json!({ "code": "record_bad_stamp" })),
        StoreError::Line { .. } | StoreError::TooRows { .. } | StoreError::TooBig { .. } => {
            McpToolError::new(
                "record_store_line",
                format!(
                    "{error}; next: repair the store with `node tools/memory/memory.mjs query` \
                     and retry"
                ),
            )
            .with_data(rmcp::serde_json::json!({ "code": "record_store_line" }))
        }
        StoreError::Io { .. } => McpToolError::new(
            "record_store_io",
            format!(
                "{error}; next: point `store` at a writable JSONL path, or leave it empty for \
                 the repo store"
            ),
        )
        .with_data(rmcp::serde_json::json!({ "code": "record_store_io" })),
    }
}

/// The store a record write goes to: the caller's, or the repo's own.
fn record_store(store: Option<&String>) -> PathBuf {
    store
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(memory_store::store_path)
}

/// `memory.record` handler: append one lesson row to the repo's memory store.
fn memory_record(
    service: &McpServer,
    args: &MemoryRecordArgs,
) -> Result<MemoryRecordOutput, McpToolError> {
    service.dispatch(TOOL_MEMORY_RECORD, |state| memory_record_in(state, args))
}

/// `memory.record` body: the tool's own rules, then one appended line.
///
/// A narrower token than the door's full token never writes here either, the
/// same rule the typed-fact store applies.
fn memory_record_in(
    state: &mut crate::mcp::ServerState,
    args: &MemoryRecordArgs,
) -> Result<MemoryRecordOutput, McpToolError> {
    if !state.cap.ops.contains(OpMask::ALL) {
        return Err(McpToolError::new(
            "memory_not_granted",
            "this session's token does not carry the memory grant; next: write from the \
             session that holds the full token, as `memory.write` requires",
        )
        .with_data(rmcp::serde_json::json!({ "code": "memory_not_granted" })));
    }
    let row = memory_store::build(
        &args.kind,
        &args.hash,
        &args.path,
        &args.utc,
        &args.text,
        args.evidence.as_deref(),
    )
    .map_err(store_refusal)?;
    let store = record_store(args.store.as_ref());
    memory_store::append(&store, &row).map_err(store_refusal)?;
    let rows = memory_store::read_rows(&store).map_err(store_refusal)?;
    let uri = format!("{MEMORY_URI}?path={}", row.path);
    Ok(MemoryRecordOutput {
        rows: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        row,
        uri,
    })
}

crate::e2_tool!(
    /// `memory.write`: one typed fact under the session's token.
    MemoryWriteTool,
    name = TOOL_MEMORY_WRITE,
    description = "Write one typed fact (fact, preference or decision) under the session's capability token: a bounded value, an integer confidence in 0..=1000, the turn that stated it and whether other sessions may recall it. A refusal keeps the store byte-identical and carries the store's own code (memory_not_granted, memory_value_too_long, memory_bad_confidence). Commits one fact.",
    readonly = false,
    args = MemoryWriteArgs,
    output = MemoryWriteOutput,
    invoke = memory_write
);

crate::e2_tool!(
    /// `memory.recall`: the session's own facts plus shared ones.
    MemoryRecallTool,
    name = TOOL_MEMORY_RECALL,
    description = "Recall what this session knows: its own facts plus every fact another session marked shared, in the store's order (type, then tick, then id), each marked own or shared. Nothing outside that scope is returned. Read-only.",
    readonly = true,
    args = MemoryRecallArgs,
    output = MemoryRecallOutput,
    invoke = memory_recall
);

crate::e2_tool!(
    /// `memory.share`: mark one of the session's own facts shared.
    MemoryShareTool,
    name = TOOL_MEMORY_SHARE,
    description = "Mark one of the session's own facts shared so every session recalls it. Another session's fact refuses with memory_not_yours and an unknown id with memory_no_fact; both leave the store unchanged. Commits one flag.",
    readonly = false,
    args = MemoryShareArgs,
    output = MemoryShareOutput,
    invoke = memory_share
);

crate::e2_tool!(
    /// `memory.record`: append one lesson row to the repo's memory store.
    MemoryRecordTool,
    name = TOOL_MEMORY_RECORD,
    description = "Record one lesson the build left behind, in the shape `tools/memory/memory.mjs` keeps: a kind (decision, failed-fix or tuned-value), a repo-relative source path, a UTC stamp, the lesson text and the 16 hex digits of the tape hash or golden it was learned under. A row without that hash refuses with record_bad_hash and the store keeps every byte it held. The row reads back on the e2040://memory resource, in stamp order. Commits one line.",
    readonly = false,
    args = MemoryRecordArgs,
    output = MemoryRecordOutput,
    invoke = memory_record
);

/// The shipped memory pack: four verbs. Three of them keep typed facts in the
/// session store, `memory.record` appends one lesson row to the repo's store;
/// recall and the resource are the observers.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "memory: typed facts behind the session's capability token, and the lessons a \
         build leaves behind. memory.write keeps one fact, memory.recall answers the \
         session's own facts plus the shared ones in store order, memory.share marks one \
         of the session's own facts shared, memory.record appends one lesson row (kind, \
         path, stamp, text and the tape hash it was learned under) to the store the \
         e2040://memory resource reads",
        false,
    );
    pack.add(VerbSpec::of::<MemoryWriteTool>(
        TOOL_MEMORY_WRITE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<MemoryRecallTool>(
        TOOL_MEMORY_RECALL,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<MemoryShareTool>(
        TOOL_MEMORY_SHARE,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<MemoryRecordTool>(
        TOOL_MEMORY_RECORD,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    fn narrow(session: u64) -> McpServer {
        McpServer::new(
            session,
            Capability::new(session, OpMask::INSERT, Vec::new(), 8),
        )
    }

    fn write(server: &McpServer, kind: &str, value: &str, shared: bool) -> MemoryWriteOutput {
        MemoryWriteTool::invoke(
            server,
            MemoryWriteArgs {
                kind: kind.to_owned(),
                value: value.to_owned(),
                confidence: Some(900),
                turn: Some(2),
                shared: Some(shared),
            },
        )
        .expect("a full token writes")
    }

    /// F2P: the pack is new. The accept: two sessions, one shared fact, and the
    /// second session recalls it.
    #[test]
    fn two_sessions_share_one_fact_and_the_second_recalls_it() {
        let writer = server(31);
        let reader = server(32);
        let private = write(&writer, "preference", "short matches", false);
        let decision = write(&writer, "decision", "arena seeds at 2040", false);
        let own = write(&reader, "fact", "the reader's own note", false);
        assert_eq!(own.fact.session, 32);

        // Before the share the reader sees only its own fact.
        let before = MemoryRecallTool::invoke(&reader, MemoryRecallArgs {}).expect("recalls");
        assert_eq!(before.facts.len(), 1, "{:?}", before.facts);
        assert_eq!(before.own, 1);
        assert_eq!(before.facts[0].value, "the reader's own note");

        // The writer shares its decision; the reader now recalls it too.
        let shared = MemoryShareTool::invoke(
            &writer,
            MemoryShareArgs {
                id: decision.fact.id,
            },
        )
        .expect("the writer shares its own fact");
        assert!(shared.fact.shared);
        assert_eq!(shared.fact.value, "arena seeds at 2040");

        let after = MemoryRecallTool::invoke(&reader, MemoryRecallArgs {}).expect("recalls");
        assert_eq!(after.facts.len(), 2, "{:?}", after.facts);
        assert_eq!(after.own, 1);
        assert_eq!(after.shared, 1);
        // Store order is type, then tick, then id: `fact` before `decision`.
        assert_eq!(after.facts[0].value, "the reader's own note");
        assert_eq!(after.facts[1].value, "arena seeds at 2040");
        assert_eq!(after.facts[1].session, 31, "the source stays the writer");
        assert!(!after.facts[1].own, "a shared fact is not the reader's own");

        // The private fact never leaks.
        assert!(
            after.facts.iter().all(|fact| fact.value != "short matches"),
            "{:?}",
            after.facts
        );
        assert!(private.fact.id < decision.fact.id, "ids rise with writes");

        // The writer recalls its own two facts; the reader's private note stays
        // out of its scope.
        let writer_rows = MemoryRecallTool::invoke(&writer, MemoryRecallArgs {}).expect("recalls");
        assert_eq!(writer_rows.facts.len(), 2, "{:?}", writer_rows.facts);
        assert_eq!(writer_rows.own, 2);
        assert_eq!(writer_rows.shared, 0);
    }

    #[test]
    fn refusals_carry_the_store_codes_and_change_nothing() {
        // A token narrower than the door's full token does not carry
        // memory.write: the store refuses by name.
        let narrow = narrow(33);
        let refused = MemoryWriteTool::invoke(
            &narrow,
            MemoryWriteArgs {
                kind: "fact".to_owned(),
                value: "half a grant".to_owned(),
                confidence: None,
                turn: None,
                shared: None,
            },
        )
        .expect_err("a narrow token cannot write");
        assert_eq!(refused.code, "memory_not_granted");
        let rows = MemoryRecallTool::invoke(&narrow, MemoryRecallArgs {}).expect("recalls");
        assert!(rows.facts.iter().all(|fact| fact.value != "half a grant"));

        let writer = server(34);
        let long = "x".repeat(engine_provider::memory::MAX_VALUE_LEN + 1);
        let too_long = MemoryWriteTool::invoke(
            &writer,
            MemoryWriteArgs {
                kind: "fact".to_owned(),
                value: long,
                confidence: None,
                turn: None,
                shared: None,
            },
        )
        .expect_err("over the value bound");
        assert_eq!(too_long.code, "memory_value_too_long");

        let over = MemoryWriteTool::invoke(
            &writer,
            MemoryWriteArgs {
                kind: "fact".to_owned(),
                value: "ok".to_owned(),
                confidence: Some(1001),
                turn: None,
                shared: None,
            },
        )
        .expect_err("over the confidence bound");
        assert_eq!(over.code, "memory_bad_confidence");

        let unknown_kind = MemoryWriteTool::invoke(
            &writer,
            MemoryWriteArgs {
                kind: "rumour".to_owned(),
                value: "ok".to_owned(),
                confidence: None,
                turn: None,
                shared: None,
            },
        )
        .expect_err("an unknown kind");
        assert_eq!(unknown_kind.code, "bad_kind");

        // Sharing another session's fact, or an id no fact carries.
        let mine = write(&writer, "fact", "mine only", false);
        let foreign = server(35);
        let not_yours = MemoryShareTool::invoke(&foreign, MemoryShareArgs { id: mine.fact.id })
            .expect_err("another session's fact");
        assert_eq!(not_yours.code, "memory_not_yours");
        let no_fact = MemoryShareTool::invoke(&writer, MemoryShareArgs { id: 9_999_999 })
            .expect_err("no such fact");
        assert_eq!(no_fact.code, "memory_no_fact");
        let rows = MemoryRecallTool::invoke(&foreign, MemoryRecallArgs {}).expect("recalls");
        assert!(rows.facts.iter().all(|fact| fact.value != "mine only"));
    }

    #[test]
    fn pack_registers_four_verbs_with_their_cards() {
        let pack = pack().expect("registers");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_MEMORY_WRITE,
                TOOL_MEMORY_RECALL,
                TOOL_MEMORY_SHARE,
                TOOL_MEMORY_RECORD
            ]
        );
        assert!(!pack.default_loaded());
        for (name, card) in [
            (TOOL_MEMORY_WRITE, CapabilityCard::Commit),
            (TOOL_MEMORY_RECALL, CapabilityCard::Observe),
            (TOOL_MEMORY_SHARE, CapabilityCard::Commit),
            (TOOL_MEMORY_RECORD, CapabilityCard::Commit),
        ] {
            let verb = pack
                .verbs()
                .iter()
                .find(|verb| verb.name() == name)
                .expect("registered");
            assert_eq!(verb.card(), card, "{name}");
        }
    }

    /// A JSONL store under the system temp dir, one per call site.
    fn lesson_store(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "e2040-memory-record-{tag}-{}-{n}.jsonl",
            std::process::id()
        ))
    }

    fn lesson_args(
        store: &PathBuf,
        kind: &str,
        hash: &str,
        path: &str,
        utc: &str,
    ) -> MemoryRecordArgs {
        MemoryRecordArgs {
            kind: kind.to_owned(),
            hash: hash.to_owned(),
            path: path.to_owned(),
            text: "the lesson text".to_owned(),
            utc: utc.to_owned(),
            evidence: Some("49f3615e4".to_owned()),
            store: Some(store.display().to_string()),
        }
    }

    #[test]
    fn memory_record_appends_the_tools_own_row_and_the_resource_reads_it_back() {
        let store = lesson_store("write");
        let writer = server(41);
        // Written late first: the read still answers in stamp order.
        let late = MemoryRecordTool::invoke(
            &writer,
            lesson_args(
                &store,
                "decision",
                "d57b5c8ae3096614",
                "crates/anim/src/b.rs",
                "2026-10-24T09:00:00Z",
            ),
        )
        .expect("a full token records");
        let early = MemoryRecordTool::invoke(
            &writer,
            lesson_args(
                &store,
                "failed-fix",
                "0xD57B5C8AE3096614",
                "crates\\anim\\src\\a.rs",
                "2026-10-23T09:00:00Z",
            ),
        )
        .expect("the second row records");
        assert_eq!(late.rows, 1, "{late:?}");
        assert_eq!(early.rows, 2, "{early:?}");
        assert_eq!(
            early.row.hash, "d57b5c8ae3096614",
            "0x upper-case normalizes"
        );
        assert_eq!(
            early.row.path, "crates/anim/src/a.rs",
            "the path is `/`-separated"
        );
        assert_eq!(
            early.row.id,
            memory_store::row_id(
                "failed-fix",
                "d57b5c8ae3096614",
                "crates/anim/src/a.rs",
                "2026-10-23T09:00:00Z",
                "the lesson text",
                "49f3615e4",
            ),
            "one row, one id in both readers"
        );
        assert_eq!(early.uri, "e2040://memory?path=crates/anim/src/a.rs");

        // The file holds exactly the two canonical lines the tool would write.
        let text = std::fs::read_to_string(&store).expect("the store reads");
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            vec![late.row.line().as_str(), early.row.line().as_str()],
            "{text}"
        );

        // The resource body for the same store: stamp order, both rows.
        let rows = memory_store::read_ordered(&store).expect("the store parses");
        let body = crate::resources::memory_body(&rows, &memory_store::Filter::default());
        let lines: Vec<&str> = body.lines().filter(|line| line.starts_with('{')).collect();
        assert_eq!(
            lines,
            vec![early.row.line().as_str(), late.row.line().as_str()]
        );
        assert!(body.contains("rows: 2 of 2 | filter: all rows"), "{body}");

        // Recording the same lesson again keeps one line: the id is the key.
        let again = MemoryRecordTool::invoke(
            &writer,
            lesson_args(
                &store,
                "failed-fix",
                "d57b5c8ae3096614",
                "crates/anim/src/a.rs",
                "2026-10-23T09:00:00Z",
            ),
        )
        .expect("the same lesson records again");
        assert_eq!(again.rows, 2, "one row, one line");
        assert_eq!(again.row.id, early.row.id);
        let _ = std::fs::remove_file(&store);
    }

    #[test]
    fn a_lesson_row_without_a_tape_hash_never_reaches_the_store() {
        let store = lesson_store("refuse");
        let writer = server(42);

        // F2P: the row the book asks for. A lesson with no hash is not memory.
        for (field, hash, code) in [
            ("empty hash", "", "record_bad_hash"),
            ("short hash", "d57b5c8ae309661", "record_bad_hash"),
            ("long hash", "d57b5c8ae30966144", "record_bad_hash"),
            ("not hex", "nope", "record_bad_hash"),
        ] {
            let refused = MemoryRecordTool::invoke(
                &writer,
                lesson_args(
                    &store,
                    "decision",
                    hash,
                    "crates/anim/src/a.rs",
                    "2026-10-24T09:00:00Z",
                ),
            )
            .expect_err("the door refuses the row");
            assert_eq!(refused.code, code, "{field}");
            assert!(
                refused.message.contains("16 hex"),
                "the recovery names the hash shape: {}",
                refused.message
            );
        }

        // The other fields refuse by name, and the store never appears.
        let bad_kind = MemoryRecordTool::invoke(
            &writer,
            MemoryRecordArgs {
                kind: "rumour".to_owned(),
                ..lesson_args(
                    &store,
                    "decision",
                    "d57b5c8ae3096614",
                    "crates/anim/src/a.rs",
                    "2026-10-24T09:00:00Z",
                )
            },
        )
        .expect_err("an unknown kind");
        assert_eq!(bad_kind.code, "record_bad_kind");
        for path in ["/abs/a.rs", "../a.rs", "C:/a.rs", "   "] {
            let refused = MemoryRecordTool::invoke(
                &writer,
                MemoryRecordArgs {
                    path: path.to_owned(),
                    ..lesson_args(
                        &store,
                        "decision",
                        "d57b5c8ae3096614",
                        "crates/anim/src/a.rs",
                        "2026-10-24T09:00:00Z",
                    )
                },
            )
            .expect_err("the path is outside the repo");
            assert_eq!(refused.code, "record_bad_path", "{path}");
        }
        let bad_stamp = MemoryRecordTool::invoke(
            &writer,
            lesson_args(&store, "decision", "d57b5c8ae3096614", "a.rs", "yesterday"),
        )
        .expect_err("a stamp is a shape");
        assert_eq!(bad_stamp.code, "record_bad_stamp");
        let no_text = MemoryRecordTool::invoke(
            &writer,
            MemoryRecordArgs {
                text: "   ".to_owned(),
                ..lesson_args(
                    &store,
                    "decision",
                    "d57b5c8ae3096614",
                    "a.rs",
                    "2026-10-24T09:00:00Z",
                )
            },
        )
        .expect_err("an empty lesson is no lesson");
        assert_eq!(no_text.code, "record_empty_text");
        assert!(
            memory_store::read_rows(&store)
                .expect("no store is an empty memory")
                .is_empty(),
            "every refusal left the store untouched"
        );

        // A narrower token never records, the same rule as memory.write.
        let narrow = narrow(43);
        let refused = MemoryRecordTool::invoke(
            &narrow,
            lesson_args(
                &store,
                "decision",
                "d57b5c8ae3096614",
                "a.rs",
                "2026-10-24T09:00:00Z",
            ),
        )
        .expect_err("a narrow token cannot record");
        assert_eq!(refused.code, "memory_not_granted");
        assert!(!store.exists(), "the refusal created no store");
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 13,
    name: PACK,
    default_loaded: false,
    tools: &[
        "memory.write",
        "memory.recall",
        "memory.share",
        "memory.record",
    ],
    build: pack,
};
