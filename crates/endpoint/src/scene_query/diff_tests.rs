//! AX23 tests: a landed write and a revision diff say what changed — the
//! touched object ids, the relation edges gained or lost, and the digest pair.
//!
//! The scene is a table with crates resting on it: supports/on_top_of are the
//! only edges, and lifting a crate off removes exactly those two.

use super::*;
use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_scene::transform::Transform;
use rmcp::handler::server::router::tool::SyncTool;
use std::collections::BTreeSet;

use crate::mcp::{
    OpDto, PoseDto, TxApplyArgs, TxApplyOutput, TxApplyTool, TxPreviewArgs, TxPreviewTool,
};

const CRATE_SCALE: [f32; 3] = [0.5, 0.5, 0.5];
const TABLE_SCALE: [f32; 3] = [4.0, 0.2, 4.0];

fn pose(x: f32, y: f32, z: f32) -> PoseDto {
    PoseDto {
        pos: [x, y, z],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: CRATE_SCALE,
    }
}

fn transform(pos: [f32; 3], scale: [f32; 3]) -> Transform {
    Transform {
        position: pos,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale,
    }
}

fn insert(id: ObjectId, pos: [f32; 3], scale: [f32; 3]) -> Op {
    Op::Insert {
        id,
        parent: ObjectId::NIL,
        local: transform(pos, scale),
    }
}

/// Table and two crates, on top: one seed batch, so revision 1 is the base the
/// measured batches start from.
fn seeded() -> (McpServer, u64, ObjectId, ObjectId, ObjectId) {
    let table = ObjectId::new();
    let left = ObjectId::new();
    let right = ObjectId::new();
    let ops = vec![
        insert(table, [0.0, 0.0, 0.0], TABLE_SCALE),
        insert(left, [-0.9, 0.35, 0.0], CRATE_SCALE),
        insert(right, [0.9, 0.35, 0.0], CRATE_SCALE),
    ];
    let mut doc = Document::new();
    let mut journal = Journal::default();
    let batch = Batch {
        ops,
        base_revision: 0,
        idempotency_key: 1,
        provenance: None,
    };
    let applied = commands::apply(&mut doc, &mut journal, &batch).expect("seed applies");
    let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
    (server, applied.revision, table, left, right)
}

/// Preview then apply one batch through the real door.
fn apply_ops(server: &McpServer, revision: u64, key: u64, ops: Vec<OpDto>) -> TxApplyOutput {
    let drafted = TxPreviewTool::invoke(
        server,
        TxPreviewArgs {
            expected_revision: revision,
            idempotency_key: key,
            ops: ops.clone(),
        },
    )
    .expect("preview validates");
    TxApplyTool::invoke(
        server,
        TxApplyArgs {
            expected_revision: revision,
            idempotency_key: key,
            ops,
            approval: drafted.approval,
            delta: Some(false),
        },
    )
    .expect("apply lands")
}

fn lift(
    server: &McpServer,
    id: ObjectId,
    x: f32,
    y: f32,
    revision: u64,
    key: u64,
) -> TxApplyOutput {
    let ops = vec![OpDto::SetLocal {
        id: id.to_string(),
        local: pose(x, y, 0.0),
    }];
    apply_ops(server, revision, key, ops)
}

fn diff(server: &McpServer, a: u64, b: u64) -> Result<SceneQueryOutput, McpToolError> {
    SceneQueryTool::invoke(
        server,
        SceneQueryArgs {
            kind: "diff".to_owned(),
            revision_a: Some(a),
            revision_b: Some(b),
            ..Default::default()
        },
    )
}

fn change_of(server: &McpServer, a: u64, b: u64) -> crate::mcp::ChangeListDto {
    diff(server, a, b)
        .expect("diff answers")
        .change
        .expect("diff carries the change list")
}

/// A batch that lifts one crate names that crate and the table it left, and
/// the two edges it lost (supports, on_top_of); the digest pair matches the
/// receipt.
#[test]
fn scene_diff_moved_crate_lists_its_id_and_edges() {
    let (server, base, table, left, _) = seeded();
    let applied = lift(&server, left, -0.9, 5.0, base, 10);
    let changes = &applied.changes;
    assert_eq!(changes.moved, 1, "one crate moved: {changes:?}");
    assert_eq!(changes.added, 0);
    assert_eq!(changes.removed, 0);
    assert!(
        changes.ids.contains(&left.to_string()),
        "the crate moved: {changes:?}"
    );
    assert!(
        changes.ids.contains(&table.to_string()),
        "the table lost the crate: {changes:?}"
    );
    assert_eq!(changes.ids_total, 2, "crate plus table: {changes:?}");
    assert_eq!(
        changes.edges_removed_total, 2,
        "supports and on_top_of go: {changes:?}"
    );
    assert_eq!(changes.edges_added_total, 0);
    assert!(
        changes
            .edges_removed
            .iter()
            .all(|row| row.starts_with("- relation ")),
        "{changes:?}"
    );
    assert!(
        changes
            .edges_removed
            .iter()
            .any(|row| row.contains("supports") && row.contains(&left.to_string())),
        "the supporting edge names the crate: {changes:?}"
    );
    assert!(
        changes
            .edges_removed
            .iter()
            .any(|row| row.contains("on_top_of") && row.contains(&table.to_string())),
        "the resting edge names the table: {changes:?}"
    );
    assert_eq!(changes.digest_before, applied.before_digest);
    assert_eq!(changes.digest_after, applied.after_digest);
    // The same pair asked as a diff answers the same list.
    assert_eq!(change_of(&server, base, applied.revision), *changes);
}

/// A landed batch that moves no pose and changes no edge lists nothing, even
/// though the document digest moves.
#[test]
fn scene_diff_no_op_batch_lists_none() {
    let (server, base, _, left, _) = seeded();
    let ops = vec![OpDto::SetComponent {
        id: left.to_string(),
        component: "mesh".to_owned(),
        value: rmcp::serde_json::json!({"asset": "meshes/wall"}),
        version: None,
    }];
    let applied = apply_ops(&server, base, 11, ops);
    let changes = &applied.changes;
    assert_eq!(changes.added, 0, "{changes:?}");
    assert_eq!(changes.removed, 0, "{changes:?}");
    assert_eq!(changes.moved, 0, "a mesh is not a pose: {changes:?}");
    assert!(changes.rows.is_empty(), "{changes:?}");
    assert!(changes.ids.is_empty(), "{changes:?}");
    assert_eq!(changes.ids_total, 0);
    assert!(changes.edges_added.is_empty(), "{changes:?}");
    assert!(changes.edges_removed.is_empty(), "{changes:?}");
    assert_eq!(changes.edges_added_total, 0);
    assert_eq!(changes.edges_removed_total, 0);
    // The write landed: the content digest moved, so this is not a refusal.
    assert_ne!(changes.digest_before, changes.digest_after);
    assert_eq!(changes.digest_after, applied.after_digest);
    assert_eq!(change_of(&server, base, applied.revision), *changes);
}

/// The diff over a range equals the lists of the batches in it: two crates,
/// two batches, then the union; equal revisions list nothing.
#[test]
fn scene_diff_between_two_revisions_equals_the_batches_between_them() {
    let (server, base, _, left, right) = seeded();
    let first = lift(&server, left, -0.9, 5.0, base, 20);
    let second = lift(&server, right, 0.9, 5.0, first.revision, 21);
    assert_eq!(
        change_of(&server, base, first.revision),
        first.changes,
        "the first batch alone"
    );
    assert_eq!(
        change_of(&server, first.revision, second.revision),
        second.changes,
        "the second batch alone"
    );
    let both = change_of(&server, base, second.revision);
    assert_eq!(both.added, 0, "{both:?}");
    assert_eq!(both.removed, 0, "{both:?}");
    assert_eq!(both.moved, 2, "both crates moved: {both:?}");
    let ids: BTreeSet<String> = first
        .changes
        .ids
        .iter()
        .chain(&second.changes.ids)
        .cloned()
        .collect();
    assert_eq!(both.ids, ids.into_iter().collect::<Vec<_>>());
    assert_eq!(both.ids_total, 3, "two crates plus the table: {both:?}");
    let mut edges: Vec<String> = first
        .changes
        .edges_removed
        .iter()
        .chain(&second.changes.edges_removed)
        .cloned()
        .collect();
    edges.sort();
    let mut both_edges = both.edges_removed.clone();
    both_edges.sort();
    assert_eq!(both_edges, edges, "every edge of both batches");
    assert_eq!(both.edges_added_total, 0);
    assert_eq!(both.digest_before, first.changes.digest_before);
    assert_eq!(both.digest_after, second.changes.digest_after);
    // Equal revisions are a range with no change, never an error.
    let none = change_of(&server, first.revision, first.revision);
    assert!(none.ids.is_empty(), "{none:?}");
    assert!(none.rows.is_empty(), "{none:?}");
    assert!(none.edges_added.is_empty() && none.edges_removed.is_empty());
    assert_eq!(none.digest_before, none.digest_after);
}

/// A revision the journal no longer holds answers `unknown_revision`, never an
/// empty list; the newest pair still answers.
#[test]
fn scene_diff_evicted_revision_answers_unknown_revision() {
    let table = ObjectId::new();
    let crate_id = ObjectId::new();
    let mut doc = Document::new();
    let mut journal = Journal::default();
    let seed = Batch {
        ops: vec![
            insert(table, [0.0, 0.0, 0.0], TABLE_SCALE),
            insert(crate_id, [0.0, 0.35, 0.0], CRATE_SCALE),
        ],
        base_revision: 0,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &seed).expect("seed applies");
    // One more batch than the journal keeps evicts the seed's revision.
    for step in 0..101u64 {
        let x = f32::from(u16::try_from(step).unwrap_or(0));
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: crate_id,
                local: transform([x, 0.35, 0.0], CRATE_SCALE),
            }],
            base_revision: doc.revision,
            idempotency_key: 100 + step,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &batch).expect("move lands");
    }
    let current = doc.revision;
    assert_eq!(current, 102, "seed plus 101 moves");
    let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
    let refused = diff(&server, 0, current).expect_err("evicted revision refuses");
    assert_eq!(refused.code, "unknown_revision", "{}", refused.message);
    assert!(
        refused.message.contains("not held"),
        "the refusal says why: {}",
        refused.message
    );
    // The newest pair is still held and names the crate that moved.
    let newest = diff(&server, current - 1, current).expect("the newest pair answers");
    let newest = newest.change.expect("diff carries the change list");
    assert!(newest.ids.contains(&crate_id.to_string()), "{newest:?}");
    assert_eq!(newest.moved, 1, "{newest:?}");
}

/// The same question twice gives equal bytes: the list, the digest and the
/// measured size all repeat (only the read guard's counter moves).
#[test]
fn scene_diff_equal_bytes_twice() {
    let (server, base, _, left, _) = seeded();
    let applied = lift(&server, left, -0.9, 5.0, base, 30);
    let first = diff(&server, base, applied.revision).expect("diff answers");
    let mut second = diff(&server, base, applied.revision).expect("diff answers again");
    assert_eq!(
        rmcp::serde_json::to_string(&first.change).expect("change serializes"),
        rmcp::serde_json::to_string(&second.change).expect("change serializes"),
        "equal bytes twice"
    );
    second.repeat_count = first.repeat_count;
    assert_eq!(first, second, "the whole answer repeats");
}

/// A replayed `tx.apply` names the same changes as the first call: the pair is
/// rebuilt from the journal instead of the unmoved live document.
#[test]
fn scene_diff_replayed_apply_lists_the_same_changes() {
    let (server, base, _, left, _) = seeded();
    let ops = vec![OpDto::SetLocal {
        id: left.to_string(),
        local: pose(0.0, 5.0, 0.0),
    }];
    let drafted = TxPreviewTool::invoke(
        &server,
        TxPreviewArgs {
            expected_revision: base,
            idempotency_key: 40,
            ops: ops.clone(),
        },
    )
    .expect("preview validates");
    let first = TxApplyTool::invoke(
        &server,
        TxApplyArgs {
            expected_revision: base,
            idempotency_key: 40,
            ops: ops.clone(),
            approval: drafted.approval.clone(),
            delta: Some(false),
        },
    )
    .expect("apply lands");
    let replayed = TxApplyTool::invoke(
        &server,
        TxApplyArgs {
            expected_revision: base,
            idempotency_key: 40,
            ops,
            approval: drafted.approval,
            delta: Some(false),
        },
    )
    .expect("replay answers");
    assert_eq!(replayed.revision, first.revision, "a replay lands nothing");
    assert_eq!(replayed.changes, first.changes, "the same list");
    assert_eq!(
        rmcp::serde_json::to_string(&replayed.changes).expect("changes serialize"),
        rmcp::serde_json::to_string(&first.changes).expect("changes serialize"),
        "equal bytes twice"
    );
}
