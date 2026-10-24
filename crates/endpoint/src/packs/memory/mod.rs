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
//! Units: confidence in thousandths, ticks as write counts, bytes for the
//! value bound. Determinism: the same writes in the same order always produce
//! the same store; the table lives until the door restarts.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, OnceLock};

use engine_editor::capability::{Capability, OpMask};
use engine_provider::memory::{
    Draft, FactType, MEMORY_WRITE, MemoryError, MemoryKey, MemoryStore, Source,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "memory";
/// Verb: write one typed fact under the session's capability token.
pub const TOOL_MEMORY_WRITE: &str = "memory.write";
/// Verb: recall the session's own facts plus every shared fact.
pub const TOOL_MEMORY_RECALL: &str = "memory.recall";
/// Verb: mark one of the session's own facts shared.
pub const TOOL_MEMORY_SHARE: &str = "memory.share";

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
        MemoryError::Malformed { .. } | MemoryError::SequenceExhausted => {
            "next: restart the door"
        }
    };
    McpToolError::new(error.code(), format!("{error}; {next}")).with_data(
        rmcp::serde_json::json!({ "code": error.code() }),
    )
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
    Ok(MemoryWriteOutput {
        fact: dto,
        stored,
    })
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

/// The shipped memory pack: three verbs, write and share commit, recall
/// observes.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "memory: typed facts behind the session's capability token. memory.write keeps one \
         fact, memory.recall answers the session's own facts plus the shared ones in store \
         order, memory.share marks one of the session's own facts shared",
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
        McpServer::new(session, Capability::new(session, OpMask::INSERT, Vec::new(), 8))
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
            MemoryShareArgs { id: decision.fact.id },
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
        assert!(after
            .facts
            .iter()
            .all(|fact| fact.value != "short matches"), "{:?}", after.facts);
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
        let not_yours = MemoryShareTool::invoke(
            &foreign,
            MemoryShareArgs { id: mine.fact.id },
        )
        .expect_err("another session's fact");
        assert_eq!(not_yours.code, "memory_not_yours");
        let no_fact = MemoryShareTool::invoke(&writer, MemoryShareArgs { id: 9_999_999 })
            .expect_err("no such fact");
        assert_eq!(no_fact.code, "memory_no_fact");
        let rows = MemoryRecallTool::invoke(&foreign, MemoryRecallArgs {}).expect("recalls");
        assert!(rows.facts.iter().all(|fact| fact.value != "mine only"));
    }

    #[test]
    fn pack_registers_three_verbs_with_their_cards() {
        let pack = pack().expect("registers");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![TOOL_MEMORY_WRITE, TOOL_MEMORY_RECALL, TOOL_MEMORY_SHARE]
        );
        assert!(!pack.default_loaded());
        for (name, card) in [
            (TOOL_MEMORY_WRITE, CapabilityCard::Commit),
            (TOOL_MEMORY_RECALL, CapabilityCard::Observe),
            (TOOL_MEMORY_SHARE, CapabilityCard::Commit),
        ] {
            let verb = pack
                .verbs()
                .iter()
                .find(|verb| verb.name() == name)
                .expect("registered");
            assert_eq!(verb.card(), card, "{name}");
        }
    }
}
