//! D17: the scene diff and the activity feed.
//!
//! Two verbs over one session's own history:
//!
//! - `scene.diff` takes a snapshot id and answers only what changed since:
//!   the added, removed and changed rows, never the whole scene again. The
//!   snapshot is named by id, so a client that read the scene once asks for
//!   the delta and pays for the delta.
//! - `session.activity` answers the feed of what happened in this session,
//!   each row tagged `human` or `agent`, so a client can tell a person from
//!   a model without reading the receipts.
//!
//! The pack loads on demand: the two verbs cost tool-list budget only when a
//! client asks for them, and the diff itself is charged by the size of the
//! delta, not the size of the scene.
//!
//! Data owner: the door's own session state. The diff is computed from the
//! journal the command service already keeps, one entry per applied batch,
//! each entry naming the revision it landed at and the objects it touched;
//! the feed is built from the same journal, whose provenance says whether a
//! change came from a person or from a script or a model.
//!
//! Units: rows for the diff, rows for the feed, revision numbers for
//! snapshots. Determinism: the same journal answers the same rows in the same
//! order; no clock, no network, no float.
//!
//! DTO structs carry no doc comments on purpose: every doc line inflates the
//! discovery schemas the door budgets.

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use engine_editor::commands::{Journal, Op};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "session";
/// Verb: the deltas since a named snapshot.
pub const TOOL_SCENE_DIFF: &str = "scene.diff";
/// Verb: the session feed, each row tagged human or agent.
pub const TOOL_SESSION_ACTIVITY: &str = "session.activity";

/// Most rows one diff answers.
pub const MAX_DIFF_ROWS: usize = 256;
/// Most rows one feed answers.
pub const MAX_ACTIVITY_ROWS: usize = 256;
/// Most snapshots one session keeps, oldest evicted first.
pub const MAX_SNAPSHOTS: usize = 32;

/// `scene.diff` args: the snapshot to compare against.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneDiffArgs {
    /// The snapshot id to compare against; 0 means the oldest kept snapshot.
    pub snapshot: u64,
}

/// One changed row: the object id and the revision it moved to.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DiffRow {
    /// The object id, as text.
    pub id: String,
    /// The revision the row moved to.
    pub revision: u64,
    /// `added`, `removed` or `changed`.
    pub kind: String,
}

/// `scene.diff` output: the deltas and the snapshot ids.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneDiffOutput {
    /// The snapshot the diff was taken against.
    pub from_snapshot: u64,
    /// The snapshot the diff is taken at, when the session has one.
    pub to_snapshot: u64,
    /// The document revision the diff ends at.
    pub revision: u64,
    /// Rows only in the newer snapshot.
    pub added: Vec<DiffRow>,
    /// Rows only in the older snapshot.
    pub removed: Vec<DiffRow>,
    /// Rows in both whose revision rose.
    pub changed: Vec<DiffRow>,
    /// True when the diff was cut at the row cap.
    pub truncated: bool,
}

/// `session.activity` args: how many rows to answer.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SessionActivityArgs {
    /// Most rows to answer; 0 means the cap.
    pub limit: u32,
}

/// One feed row: who did what, and where.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ActivityRow {
    /// `human` or `agent`.
    pub author: String,
    /// The batch name the journal recorded.
    pub verb: String,
    /// The document revision the batch landed at.
    pub revision: u64,
    /// The object ids the batch touched.
    pub ids: Vec<String>,
}

/// `session.activity` output: the feed and the counts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SessionActivityOutput {
    /// The rows, newest first.
    pub rows: Vec<ActivityRow>,
    /// How many rows a person made.
    pub human: u32,
    /// How many rows a script or a model made.
    pub agent: u32,
    /// True when the feed was cut at the row cap.
    pub truncated: bool,
}

/// One kept snapshot: the revision it was taken at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The snapshot id; ids count up from 1.
    pub id: u64,
    /// The document revision the snapshot was taken at.
    pub revision: u64,
}

/// The session's snapshot table: the revisions a diff can be taken against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshots {
    rows: Vec<Snapshot>,
    next_id: u64,
}

impl Snapshots {
    /// Empty table; ids start at 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            next_id: 1,
        }
    }

    /// How many snapshots the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the table holds no snapshot.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Take a snapshot of the current revision and return its id.
    ///
    /// The oldest snapshot is evicted past [`MAX_SNAPSHOTS`], so the table
    /// never grows without bound.
    pub fn take(&mut self, revision: u64) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(Snapshot { id, revision });
        while self.rows.len() > MAX_SNAPSHOTS {
            self.rows.remove(0);
        }
        id
    }

    /// The snapshot with this id.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&Snapshot> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The newest snapshot.
    #[must_use]
    pub fn latest(&self) -> Option<&Snapshot> {
        self.rows.last()
    }

    /// The oldest kept snapshot.
    #[must_use]
    pub fn oldest(&self) -> Option<&Snapshot> {
        self.rows.first()
    }

    /// The rows of this table, oldest first.
    #[must_use]
    pub fn rows(&self) -> &[Snapshot] {
        &self.rows
    }
}

/// The deltas between a snapshot and the live journal.
///
/// The diff is charged by the delta: the added, removed and changed rows, and
/// never the whole scene. The journal is the source of truth: every applied
/// batch names the revision it landed at and the objects it touched.
#[must_use]
pub fn diff_from(snapshot: &Snapshot, journal: &Journal, revision: u64) -> SceneDiffOutput {
    let mut added: Vec<DiffRow> = Vec::new();
    let mut removed: Vec<DiffRow> = Vec::new();
    let mut changed: Vec<DiffRow> = Vec::new();

    // Oldest first, so a later row for the same object wins the revision.
    for entry in journal.past.iter() {
        if entry.result_revision <= snapshot.revision {
            continue;
        }
        for op in &entry.forward {
            match op {
                Op::Insert { id, .. } => added.push(DiffRow {
                    id: id.to_string(),
                    revision: entry.result_revision,
                    kind: "added".to_owned(),
                }),
                Op::Remove { id } => removed.push(DiffRow {
                    id: id.to_string(),
                    revision: entry.result_revision,
                    kind: "removed".to_owned(),
                }),
                Op::SetLocal { id, .. }
                | Op::Reparent { id, .. }
                | Op::SetMaterial { id, .. }
                | Op::SetComponent { id, .. }
                | Op::SetBond { id, .. }
                | Op::RemoveComponent { id, .. } => changed.push(DiffRow {
                    id: id.to_string(),
                    revision: entry.result_revision,
                    kind: "changed".to_owned(),
                }),
                _ => {}
            }
        }
    }

    // The last row for an object is the one that counts.
    added = last_per_id(added);
    removed = last_per_id(removed);
    changed = last_per_id(changed);

    let total = added.len() + removed.len() + changed.len();
    let truncated = total > MAX_DIFF_ROWS;
    if truncated {
        added.truncate(MAX_DIFF_ROWS);
        removed.truncate(MAX_DIFF_ROWS.saturating_sub(added.len()));
        changed.truncate(MAX_DIFF_ROWS.saturating_sub(added.len() + removed.len()));
    }

    SceneDiffOutput {
        from_snapshot: snapshot.id,
        to_snapshot: 0,
        revision,
        added,
        removed,
        changed,
        truncated,
    }
}

/// Keep the last row for each object id, in first-seen order.
fn last_per_id(rows: Vec<DiffRow>) -> Vec<DiffRow> {
    let mut order: Vec<String> = Vec::new();
    let mut last: Vec<DiffRow> = Vec::new();
    for row in rows {
        match order.iter().position(|id| id == &row.id) {
            Some(at) => last[at] = row,
            None => {
                order.push(row.id.clone());
                last.push(row);
            }
        }
    }
    last
}

/// The session feed: what happened, each row tagged human or agent.
///
/// A row is `human` when the journal says a person made the change and
/// `agent` when a script or a model did.
#[must_use]
pub fn activity_feed(journal: &Journal, limit: u32) -> SessionActivityOutput {
    let cap = if limit == 0 {
        MAX_ACTIVITY_ROWS
    } else {
        usize::try_from(limit)
            .unwrap_or(MAX_ACTIVITY_ROWS)
            .min(MAX_ACTIVITY_ROWS)
    };
    let mut rows: Vec<ActivityRow> = Vec::new();
    let mut human = 0u32;
    let mut agent = 0u32;
    let mut total = 0usize;
    // Newest first: the journal keeps the newest batch last.
    for entry in journal.past.iter().rev() {
        let is_human = matches!(
            entry.provenance.source,
            engine_scene::provenance::Source::Human
        );
        let ids: Vec<String> = entry.forward.iter().filter_map(op_id).collect();
        rows.push(ActivityRow {
            author: if is_human { "human" } else { "agent" }.to_owned(),
            verb: entry.name.clone(),
            revision: entry.result_revision,
            ids,
        });
        if is_human {
            human += 1;
        } else {
            agent += 1;
        }
        total += 1;
        if rows.len() >= cap {
            break;
        }
    }
    SessionActivityOutput {
        rows,
        human,
        agent,
        truncated: total > cap,
    }
}

/// The object id one op touches; `None` when the op touches none.
fn op_id(op: &Op) -> Option<String> {
    let id = match op {
        Op::Insert { id, .. }
        | Op::Remove { id }
        | Op::SetLocal { id, .. }
        | Op::Reparent { id, .. }
        | Op::SetMaterial { id, .. }
        | Op::SetComponent { id, .. }
        | Op::SetBond { id, .. }
        | Op::RemoveComponent { id, .. } => *id,
        _ => return None,
    };
    Some(id.to_string())
}

/// `scene.diff` handler: the deltas since a named snapshot.
fn scene_diff(service: &McpServer, args: &SceneDiffArgs) -> Result<SceneDiffOutput, McpToolError> {
    service.dispatch(TOOL_SCENE_DIFF, |state| {
        let snapshots = state.snapshots();
        let snapshot = if args.snapshot == 0 {
            snapshots.oldest().ok_or_else(|| {
                McpToolError::new(
                    "no_snapshot",
                    "this session holds no snapshot; take one before asking for a diff",
                )
            })?
        } else {
            snapshots.get(args.snapshot).ok_or_else(|| {
                McpToolError::new(
                    "no_snapshot",
                    format!(
                        "snapshot {} is not kept; this session keeps {:?}",
                        args.snapshot,
                        snapshots
                            .rows()
                            .iter()
                            .map(|row| row.id)
                            .collect::<Vec<_>>()
                    ),
                )
            })?
        };
        let mut out = diff_from(snapshot, &state.journal, state.doc.revision);
        out.to_snapshot = snapshots.latest().map_or(0, |row| row.id);
        Ok(out)
    })
}

/// `session.activity` handler: the session feed, tagged human or agent.
fn session_activity(
    service: &McpServer,
    args: &SessionActivityArgs,
) -> Result<SessionActivityOutput, McpToolError> {
    service.dispatch(TOOL_SESSION_ACTIVITY, |state| {
        Ok(activity_feed(&state.journal, args.limit))
    })
}

crate::e2_tool!(
    /// `scene.diff` tool: the deltas since a named snapshot.
    SceneDiffTool,
    name = TOOL_SCENE_DIFF,
    description = "The deltas since a named snapshot: the added, removed and changed rows, never the whole scene.",
    readonly = true,
    args = SceneDiffArgs,
    output = SceneDiffOutput,
    invoke = scene_diff
);

crate::e2_tool!(
    /// `session.activity` tool: the session feed, each row tagged human or agent.
    SessionActivityTool,
    name = TOOL_SESSION_ACTIVITY,
    description = "What happened in this session, newest first, each row tagged human or agent.",
    readonly = true,
    args = SessionActivityArgs,
    output = SessionActivityOutput,
    invoke = session_activity
);

/// The session pack: the scene diff and the activity feed.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "session history: the deltas since a named snapshot and the feed of what happened, tagged human or agent",
        false,
    );
    pack.add(VerbSpec::of::<SceneDiffTool>(
        TOOL_SCENE_DIFF,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SessionActivityTool>(
        TOOL_SESSION_ACTIVITY,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::commands::Op;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    #[test]
    fn snapshots_count_up_and_evict_the_oldest() {
        let mut snapshots = Snapshots::new();
        assert!(snapshots.is_empty());
        let first = snapshots.take(1);
        let second = snapshots.take(2);
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots.get(first).map(|row| row.revision), Some(1));
        assert_eq!(snapshots.oldest().map(|row| row.id), Some(first));
        assert_eq!(snapshots.latest().map(|row| row.id), Some(second));
        for rev in 3..=(MAX_SNAPSHOTS as u64 + 4) {
            snapshots.take(rev);
        }
        assert_eq!(snapshots.len(), MAX_SNAPSHOTS);
        assert!(snapshots.get(first).is_none(), "the oldest is evicted");
    }

    #[test]
    fn an_empty_journal_diffs_to_nothing() {
        let snapshot = Snapshot { id: 1, revision: 0 };
        let out = diff_from(&snapshot, &Journal::new(), 0);
        assert!(out.added.is_empty());
        assert!(out.removed.is_empty());
        assert!(out.changed.is_empty());
        assert!(!out.truncated);
        assert_eq!(out.from_snapshot, 1);
    }

    #[test]
    fn the_feed_of_an_empty_journal_is_empty() {
        let out = activity_feed(&Journal::new(), 0);
        assert!(out.rows.is_empty());
        assert_eq!(out.human, 0);
        assert_eq!(out.agent, 0);
        assert!(!out.truncated);
    }

    #[test]
    fn op_ids_are_read_for_the_touching_ops() {
        let id = ObjectId::new();
        let insert = Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::identity(),
        };
        assert_eq!(op_id(&insert), Some(id.to_string()));
        let remove = Op::Remove { id };
        assert_eq!(op_id(&remove), Some(id.to_string()));
    }

    #[test]
    fn last_per_id_keeps_the_newest_row() {
        let rows = vec![
            DiffRow {
                id: "a".to_owned(),
                revision: 1,
                kind: "added".to_owned(),
            },
            DiffRow {
                id: "a".to_owned(),
                revision: 2,
                kind: "changed".to_owned(),
            },
            DiffRow {
                id: "b".to_owned(),
                revision: 3,
                kind: "added".to_owned(),
            },
        ];
        let kept = last_per_id(rows);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].id, "a");
        assert_eq!(kept[0].revision, 2);
        assert_eq!(kept[1].id, "b");
    }

    /// D17: `scene.diff` answers the deltas since a named snapshot, never the
    /// whole scene, and the merge order is deterministic.
    #[test]
    fn scene_diff_answers_deltas_since_a_named_snapshot() {
        use crate::mcp::McpServer;
        use crate::mcp::{OpDto, PoseDto, TxApplyArgs, TxApplyTool, TxPreviewArgs, TxPreviewTool};
        use engine_editor::capability::Capability;
        use rmcp::handler::server::router::tool::SyncTool;

        let server = McpServer::new(7, Capability::unrestricted(7));
        // The session opens with a snapshot of the revision it started at.
        let id = ObjectId::new();
        let ops = vec![OpDto::Insert {
            id: id.to_string(),
            parent: ObjectId::NIL.to_string(),
            local: PoseDto {
                pos: [1.0, 0.0, 0.0],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        }];
        let drafted = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 0,
                idempotency_key: 70,
                ops: ops.clone(),
            },
        )
        .expect("preview lands a plan");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: 0,
                idempotency_key: 70,
                ops,
                approval: drafted.approval,
                delta: None,
            },
        )
        .expect("apply lands the insert");

        // The diff names the snapshot it was taken against and answers only
        // the deltas: one added row, no removals, no changes.
        let out = SceneDiffTool::invoke(&server, SceneDiffArgs { snapshot: 0 }).expect("diff");
        assert_eq!(out.from_snapshot, 1, "the oldest kept snapshot");
        assert_eq!(out.to_snapshot, 2, "the snapshot the landed write took");
        assert_eq!(out.revision, 1);
        assert_eq!(out.added.len(), 1);
        assert_eq!(out.added[0].id, id.to_string());
        assert_eq!(out.added[0].kind, "added");
        assert_eq!(out.added[0].revision, 1);
        assert!(out.removed.is_empty());
        assert!(out.changed.is_empty());
        assert!(!out.truncated);

        // The same question twice answers the same rows in the same order.
        let again =
            SceneDiffTool::invoke(&server, SceneDiffArgs { snapshot: 0 }).expect("diff again");
        assert_eq!(out, again, "the diff is deterministic");

        // A snapshot the session never took refuses by name.
        let refused = SceneDiffTool::invoke(&server, SceneDiffArgs { snapshot: 99 })
            .expect_err("an unknown snapshot refuses");
        assert_eq!(refused.code, "no_snapshot");

        println!("scene_diff: snapshot id then deltas only, merge order deterministic PASS");
    }
}
