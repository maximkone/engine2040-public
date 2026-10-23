//! Unit tests of the MCP door. Moved out of `mcp.rs`, which sits at the
//! 3000-line cap; `use super::*` still reaches the same items from here.

use super::*;
use engine_editor::capability::OpMask;
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};

fn pose(x: f32) -> PoseDto {
    PoseDto {
        pos: [x, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

fn nil_id() -> String {
    ObjectId::NIL.to_string()
}

fn insert_dto(id: &ObjectId, parent: &str, x: f32) -> OpDto {
    OpDto::Insert {
        id: id.to_string(),
        parent: parent.to_owned(),
        local: pose(x),
    }
}

fn test_server() -> McpServer {
    McpServer::new(7, Capability::unrestricted(7))
}

/// Observable session surface for no-write asserts.
struct Snapshot {
    doc: Document,
    past: usize,
    future: usize,
    budget: u32,
    spent: usize,
}

fn snapshot(server: &McpServer) -> Snapshot {
    let state = server.state.lock().expect("test lock holds");
    Snapshot {
        doc: state.doc.clone(),
        past: state.journal.past_len(),
        future: state.journal.future_len(),
        budget: state.cap.remaining(),
        spent: state.ledger.len(),
    }
}

impl Snapshot {
    fn assert_same_doc(&self, server: &McpServer) {
        let after = snapshot(server);
        assert_eq!(self.doc, after.doc, "document changed");
        assert_eq!(self.past, after.past, "journal past moved");
        assert_eq!(self.future, after.future, "journal future moved");
        assert_eq!(self.doc.revision, after.doc.revision, "revision moved");
    }

    fn assert_same_all(&self, server: &McpServer) {
        self.assert_same_doc(server);
        let after = snapshot(server);
        assert_eq!(self.budget, after.budget, "capability budget moved");
        assert_eq!(self.spent, after.spent, "approval ledger moved");
    }
}

fn preview(
    server: &McpServer,
    rev: u64,
    key: u64,
    ops: Vec<OpDto>,
) -> Result<TxPreviewOutput, McpToolError> {
    TxPreviewTool::invoke(
        server,
        TxPreviewArgs {
            expected_revision: rev,
            idempotency_key: key,
            ops,
        },
    )
}

fn apply(
    server: &McpServer,
    rev: u64,
    key: u64,
    ops: Vec<OpDto>,
    approval: ApprovalDto,
) -> Result<TxApplyOutput, McpToolError> {
    TxApplyTool::invoke(
        server,
        TxApplyArgs {
            expected_revision: rev,
            idempotency_key: key,
            ops,
            approval,
            delta: None,
        },
    )
}

fn read(server: &McpServer, limit: Option<u32>, cursor: Option<u32>) -> SceneReadOutput {
    SceneReadTool::invoke(server, SceneReadArgs { limit, cursor }).expect("valid page reads")
}

#[test]
fn prompts_publish_the_shipped_skills() {
    let server = test_server();
    let info = server.get_info();
    assert!(
        info.capabilities.prompts.is_some(),
        "the handshake advertises prompts when skills ship"
    );
    let prompts = server.prompt_list();
    assert!(prompts.len() >= 30, "got {} prompts", prompts.len());
    let door = prompts
        .iter()
        .find(|prompt| prompt.name == "ai-door")
        .expect("ai-door is published");
    assert!(door
        .description
        .as_deref()
        .is_some_and(|text| !text.is_empty()));
    let body = server.prompt_text("ai-door").expect("ai-door body");
    assert!(body.contains("# AI door"), "the body is the skill text");
    assert!(server.prompt_text("not-a-skill").is_none());
}

#[test]
fn scripted_insert_preview_apply_undo_roundtrip() {
    let server = test_server();
    // Empty document at revision zero.
    let empty = read(&server, None, None);
    assert_eq!(empty.revision, 0);
    assert_eq!(empty.total, 0);
    assert!(!empty.truncated);

    let id = ObjectId::new();
    let ops = vec![insert_dto(&id, &nil_id(), 1.0)];

    // Preview diffs with no side effects: document, journal, budget and
    // ledger are all identical afterwards.
    let before = snapshot(&server);
    let drafted = preview(&server, 0, 1, ops.clone()).expect("preview inserts");
    assert_eq!(drafted.effects.len(), 1);
    assert!(drafted.effects[0].contains(&id.to_string()));
    assert_eq!(drafted.resulting_revision, 1);
    assert_eq!(drafted.approval.revision, 0);
    assert_eq!(drafted.approval.expiry_tick, 0);
    assert_eq!(drafted.approval.action_hash, drafted.action_hash);
    before.assert_same_all(&server);

    // Apply lands exactly one revision with the previewed effects.
    let landed =
        apply(&server, 0, 1, ops.clone(), drafted.approval.clone()).expect("apply inserts");
    assert_eq!(landed.revision, 1);
    assert_eq!(landed.effects, drafted.effects);
    assert_eq!(landed.key, 1);
    assert_eq!(landed.undo_token, 1);

    let page = read(&server, None, None);
    assert_eq!(page.revision, 1);
    assert_eq!(page.total, 1);
    assert_eq!(page.objects[0].id, id.to_string());
    assert_eq!(page.objects[0].parent, None);
    assert_eq!(page.objects[0].local, pose(1.0));

    // Same key plus same payload replays the recorded receipt; the change list
    // rebuilds the landed pair from the journal, so the live revision stays 1.
    let replayed =
        apply(&server, 0, 1, ops.clone(), drafted.approval.clone()).expect("idempotent replay");
    assert_eq!(replayed.revision, landed.revision);
    assert_eq!(replayed.effects, landed.effects);
    assert_eq!(replayed.key, landed.key);
    assert_eq!(replayed.undo_token, landed.undo_token);
    assert_eq!(replayed.spatial_delta, landed.spatial_delta);
    assert_eq!(
        replayed.changes, landed.changes,
        "a replay rebuilds the original pair from the journal, got {replayed:?}"
    );
    assert_eq!(read(&server, None, None).revision, 1);

    // Undo reverts the hierarchy as a new revision, keeping a token link.
    let undone = TxUndoTool::invoke(
        &server,
        TxUndoArgs {
            undo_token: Some(landed.undo_token),
        },
    )
    .expect("undo reverts");
    assert_eq!(undone.revision, 2);
    assert_eq!(undone.undone_key, 1);
    let back = read(&server, None, None);
    assert_eq!(back.revision, 2);
    assert_eq!(back.total, 0);
    assert_eq!(back.objects, empty.objects);
}

#[test]
fn tx_apply_receipt_reports_exactly_one_change() {
    let server = test_server();
    let id = ObjectId::new();
    let ops = vec![insert_dto(&id, &nil_id(), 1.0)];
    let drafted = preview(&server, 0, 71, ops.clone()).expect("preview inserts");
    let landed = apply(&server, 0, 71, ops, drafted.approval).expect("apply inserts");
    assert_eq!(landed.revision, 1);
    // One insert is exactly one change: no phantom row, counts exact.
    let changes = &landed.changes;
    assert_eq!(changes.revision_before, 0);
    assert_eq!(changes.revision_after, 1);
    assert_eq!(
        (changes.added, changes.removed, changes.moved),
        (1, 0, 0),
        "one insert adds exactly one object"
    );
    assert_eq!(changes.rows, vec![format!("added {id}")]);
    assert!(!changes.truncated, "one change is under the cap");
}

#[test]
fn tx_apply_receipt_counts_a_move_and_a_remove() {
    let server = test_server();
    let a = ObjectId::new();
    let b = ObjectId::new();
    let seed = vec![
        insert_dto(&a, &nil_id(), 1.0),
        insert_dto(&b, &nil_id(), 5.0),
    ];
    let drafted = preview(&server, 0, 72, seed.clone()).expect("seed previews");
    let seeded = apply(&server, 0, 72, seed, drafted.approval).expect("seed lands");
    assert_eq!((seeded.changes.added, seeded.changes.removed), (2, 0));

    // A move of `a` by itself: exactly one moved, no add or remove.
    let move_ops = vec![OpDto::SetLocal {
        id: a.to_string(),
        local: pose(2.0),
    }];
    let drafted = preview(&server, 1, 73, move_ops.clone()).expect("move previews");
    let moved = apply(&server, 1, 73, move_ops, drafted.approval).expect("move lands");
    assert_eq!(moved.changes.revision_before, 1);
    assert_eq!(moved.changes.revision_after, 2);
    assert_eq!(
        (
            moved.changes.added,
            moved.changes.removed,
            moved.changes.moved
        ),
        (0, 0, 1),
        "one move is exactly one moved object"
    );
    assert_eq!(moved.changes.rows, vec![format!("moved {a}")]);

    // A remove of `b`: exactly one removed.
    let remove_ops = vec![OpDto::Remove { id: b.to_string() }];
    let drafted = preview(&server, 2, 74, remove_ops.clone()).expect("remove previews");
    let removed = apply(&server, 2, 74, remove_ops, drafted.approval).expect("remove lands");
    assert_eq!(
        (
            removed.changes.added,
            removed.changes.removed,
            removed.changes.moved
        ),
        (0, 1, 0),
        "one remove is exactly one removed object"
    );
    assert_eq!(removed.changes.rows, vec![format!("removed {b}")]);
}

#[test]
fn set_material_preview_apply_roundtrip_with_typed_bad_name() {
    let server = test_server();
    let node = ObjectId::new();
    let seed_ops = vec![insert_dto(&node, &nil_id(), 0.0)];
    let seeded = preview(&server, 0, 60, seed_ops.clone()).expect("seed preview");
    apply(&server, 0, 60, seed_ops, seeded.approval).expect("seed apply");

    // Bind: preview diffs with no side effects, apply lands the binding.
    let bind = vec![OpDto::SetMaterial {
        id: node.to_string(),
        material: Some("hero-mat".to_owned()),
    }];
    let before = snapshot(&server);
    let drafted = preview(&server, 1, 61, bind.clone()).expect("bind previews");
    assert_eq!(drafted.effects.len(), 1);
    assert!(
        drafted.effects[0].contains("hero-mat"),
        "effect names the bound asset: {}",
        drafted.effects[0]
    );
    before.assert_same_all(&server);
    let landed = apply(&server, 1, 61, bind.clone(), drafted.approval.clone()).expect("bind lands");
    assert_eq!(landed.revision, 2);
    assert_eq!(landed.effects, drafted.effects);

    // Clear: `None` unbinds through the same preview/apply gate.
    let clear = vec![OpDto::SetMaterial {
        id: node.to_string(),
        material: None,
    }];
    let drafted = preview(&server, 2, 62, clear.clone()).expect("clear previews");
    let landed = apply(&server, 2, 62, clear, drafted.approval).expect("clear lands");
    assert_eq!(landed.revision, 3);

    // Bad name: typed refusal names the legal charset, writes nothing.
    let before = snapshot(&server);
    let bad = vec![OpDto::SetMaterial {
        id: node.to_string(),
        material: Some("hero mat!".to_owned()),
    }];
    let refused = preview(&server, 3, 63, bad).expect_err("bad name must fail");
    assert_eq!(refused.code, "bad_material");
    assert!(
        refused.message.contains("0-9 A-Z a-z - _ ."),
        "names the legal charset: {}",
        refused.message
    );
    before.assert_same_all(&server);
}

#[test]
fn denied_stale_duplicate_calls_write_nothing() {
    let server = test_server();
    let seed = ObjectId::new();
    let seed_ops = vec![insert_dto(&seed, &nil_id(), 0.0)];
    let seeded = preview(&server, 0, 10, seed_ops.clone()).expect("seed preview");
    apply(&server, 0, 10, seed_ops, seeded.approval).expect("seed apply");
    assert_eq!(read(&server, None, None).revision, 1);

    // Forged approval: hash from another batch.
    let other = ObjectId::new();
    let other_ops = vec![insert_dto(&other, &nil_id(), 5.0)];
    let other_preview = preview(&server, 1, 11, other_ops.clone()).expect("other preview");
    let before = snapshot(&server);
    let mut forged = other_preview.approval.clone();
    forged.action_hash ^= 0xdead_beef;
    let refused = apply(&server, 1, 11, other_ops.clone(), forged);
    assert_eq!(
        refused.expect_err("forged approval must fail").code,
        "forged_approval"
    );
    before.assert_same_all(&server);

    // Stale revision: batch prepared before the seed landed.
    let stale_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 2.0)];
    let stale_preview = preview(&server, 0, 12, stale_ops);
    assert_eq!(
        stale_preview.expect_err("stale preview must fail").code,
        "stale_revision"
    );
    before.assert_same_all(&server);

    // Expired approval: a short-lived grant presented after its expiry.
    // (Preview mints expiry == current revision; a host may narrow it.
    // CAP1 checks staleness before expiry, so the batch stays current.)
    let filler_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 4.0)];
    let filler = preview(&server, 1, 14, filler_ops.clone()).expect("filler preview");
    apply(&server, 1, 14, filler_ops, filler.approval).expect("filler lands");
    assert_eq!(read(&server, None, None).revision, 2);
    let expiring_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 3.0)];
    let expiring = preview(&server, 2, 13, expiring_ops.clone()).expect("expiring preview");
    let mut short_lived = expiring.approval.clone();
    short_lived.expiry_tick = 1;
    let before = snapshot(&server);
    let expired = apply(&server, 2, 13, expiring_ops, short_lived);
    assert_eq!(
        expired.expect_err("expired approval must fail").code,
        "expired_approval"
    );
    before.assert_same_all(&server);

    // Duplicate key with a different payload: refused as a conflict.
    // (The ledger spend on this path is CAP1 design; document and both
    // journal stacks stay put.)
    let clash_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 6.0)];
    let clash = preview(&server, 2, 15, clash_ops.clone()).expect("clash preview");
    apply(&server, 2, 15, clash_ops.clone(), clash.approval.clone()).expect("clash lands");
    let different = vec![insert_dto(&ObjectId::new(), &nil_id(), 7.0)];
    let different_preview = preview(&server, 3, 15, different.clone()).expect("conflict preview");
    let before = snapshot(&server);
    let conflict = apply(&server, 3, 15, different, different_preview.approval);
    assert_eq!(
        conflict.expect_err("key conflict must fail").code,
        "duplicate_key"
    );
    before.assert_same_doc(&server);
    // The recorded key still replays its original receipt.
    let replayed = apply(&server, 2, 15, clash_ops, clash.approval).expect("replay");
    assert_eq!(replayed.revision, 3);

    // Reused approval after undo: the retry is refused, never resurrected.
    let cycle_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 8.0)];
    let cycle = preview(&server, 3, 16, cycle_ops.clone()).expect("cycle preview");
    apply(&server, 3, 16, cycle_ops.clone(), cycle.approval.clone()).expect("cycle lands");
    TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("cycle undone");
    let before = snapshot(&server);
    let resurrected = apply(&server, 3, 16, cycle_ops, cycle.approval);
    assert_eq!(
        resurrected.expect_err("spent approval must fail").code,
        "reused_approval"
    );
    before.assert_same_all(&server);

    // Undone key with a fresh approval: refused, never resurrected.
    // (Same CAP1 ledger-spend note as the duplicate-key path.)
    let current = read(&server, None, None).revision;
    let rehash_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 9.0)];
    let rehash =
        preview(&server, current, 16, rehash_ops.clone()).expect("fresh preview for an undone key");
    let before = snapshot(&server);
    let undone = apply(&server, current, 16, rehash_ops, rehash.approval);
    assert_eq!(undone.expect_err("undone key must fail").code, "undone_key");
    before.assert_same_doc(&server);

    // Out-of-scope object: the gate fires on apply (preview is a dry
    // run), writing nothing.
    let scoped = McpServer::new(
        9,
        Capability::new(9, OpMask::ALL, vec![seed.to_string()], 1),
    );
    let guided_ops = vec![insert_dto(&seed, &nil_id(), 0.0)];
    assert!(
        preview(&scoped, 0, 20, guided_ops).is_ok(),
        "in-scope insert previews"
    );
    let before = snapshot(&scoped);
    let far = ObjectId::new();
    let far_ops = vec![insert_dto(&far, &nil_id(), 0.0)];
    let far_approval = preview(&scoped, 0, 21, far_ops.clone())
        .expect("preview is scope-free")
        .approval;
    let denied = apply(&scoped, 0, 21, far_ops, far_approval);
    let denied_error = denied.expect_err("out-of-scope apply must fail");
    assert_eq!(denied_error.code, "out_of_scope");
    assert!(denied_error.message.contains(&far.to_string()));
    before.assert_same_all(&scoped);

    // Exhausted budget: same no-write guarantee.
    let broke = McpServer::new(11, Capability::new(11, OpMask::ALL, vec![], 0));
    let tight_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 0.0)];
    let tight_approval = preview(&broke, 0, 22, tight_ops.clone())
        .expect("preview is budget-free")
        .approval;
    let before = snapshot(&broke);
    let broke_apply = apply(&broke, 0, 22, tight_ops, tight_approval);
    let broke_error = broke_apply.expect_err("over-budget apply must fail");
    assert_eq!(broke_error.code, "over_budget");
    assert!(broke_error.message.contains('0'));
    before.assert_same_all(&broke);
}

#[test]
fn capability_gate_fronts_apply_ahead_of_approval_checks() {
    // Out-of-scope batch plus a stale approval: the taxonomy gate fires
    // first, so the door names the scope, not the approval.
    let seed = ObjectId::new();
    let scoped = McpServer::new(
        9,
        Capability::new(9, OpMask::ALL, vec![seed.to_string()], 8),
    );
    let far = ObjectId::new();
    let far_ops = vec![insert_dto(&far, &nil_id(), 0.0)];
    let stale_approval = ApprovalDto {
        revision: 99,
        action_hash: 1,
        expiry_tick: 99,
    };
    let before = snapshot(&scoped);
    let refused = apply(&scoped, 0, 60, far_ops, stale_approval)
        .expect_err("out-of-scope apply must fail on scope first");
    assert_eq!(refused.code, "out_of_scope");
    assert!(refused.message.contains(&far.to_string()));
    before.assert_same_all(&scoped);

    // Over-budget batch plus a forged approval: budget wins the same way.
    let broke = McpServer::new(11, Capability::new(11, OpMask::ALL, vec![], 0));
    let tight_ops = vec![insert_dto(&ObjectId::new(), &nil_id(), 0.0)];
    let forged = ApprovalDto {
        revision: 0,
        action_hash: 0xBEEF,
        expiry_tick: 0,
    };
    let before = snapshot(&broke);
    let refused = apply(&broke, 0, 61, tight_ops, forged)
        .expect_err("over-budget apply must fail on budget first");
    assert_eq!(refused.code, "over_budget");
    before.assert_same_all(&broke);
}

#[test]
fn spatial_rejection_writes_nothing() {
    let server = test_server();
    // Two grounded roots: x=0 and x=10, unit cubes, no overlap.
    let a = ObjectId::new();
    let b = ObjectId::new();
    for (key, id, x) in [(30u64, a, 0.0f32), (31u64, b, 10.0f32)] {
        let ops = vec![insert_dto(&id, &nil_id(), x)];
        let rev = read(&server, None, None).revision;
        let drafted = preview(&server, rev, key, ops.clone()).expect("seed preview");
        apply(&server, rev, key, ops, drafted.approval).expect("seed apply");
    }
    assert_eq!(read(&server, None, None).revision, 2);

    // Drag `b` onto `a`: the staged overlap check refuses, writing nothing.
    let before = snapshot(&server);
    let clash = vec![OpDto::SetLocal {
        id: b.to_string(),
        local: pose(0.0),
    }];
    let drafted = preview(&server, 2, 32, clash.clone()).expect("overlap previews");
    let refused = apply(&server, 2, 32, clash, drafted.approval);
    let error = refused.expect_err("overlap must fail");
    assert_eq!(error.code, "spatial_rejected");
    assert!(error.message.contains("overlap"));
    // The refusal names both objects of the pair in the `name [uuid]` form.
    for culprit in [a, b] {
        let label = format!("[{culprit}]");
        assert!(
            error.message.contains(&label),
            "refusal must name {culprit}: {}",
            error.message
        );
    }
    before.assert_same_all(&server);

    // A clean grounded nudge nearby still applies through the same gate.
    let nudge = vec![OpDto::SetLocal {
        id: b.to_string(),
        local: pose(5.0),
    }];
    let drafted = preview(&server, 2, 33, nudge.clone()).expect("nudge previews");
    apply(&server, 2, 33, nudge, drafted.approval).expect("nudge lands");
    assert_eq!(read(&server, None, None).revision, 3);
}

#[test]
fn tool_schema_drift_matches_command_shapes() {
    // Every tx input schema advertises the seven command ops.
    for tool in [TxPreviewTool::input_schema(), TxApplyTool::input_schema()] {
        let schema = tool.expect("tx tools carry input schemas");
        let text = rmcp::serde_json::to_string(&*schema).expect("schema serializes");
        let value: rmcp::serde_json::Value =
            rmcp::serde_json::from_str(&text).expect("schema is JSON");
        // Exactly seven discriminator branches: one per Op variant.
        assert_eq!(collect_branches(&value), 7, "schema drift: {text}");
        // Exactly the seven op tags, matching Op's snake_case variants.
        let mut tags = collect_op_tags(&value);
        tags.sort();
        tags.dedup();
        assert_eq!(
            tags,
            [
                "insert",
                "remove",
                "remove_component",
                "reparent",
                "set_component",
                "set_local",
                "set_material"
            ]
        );
    }

    // Every advertised op converts to a real command and validates alone.
    let server = test_server();
    let target = ObjectId::new();
    let seed_ops = vec![insert_dto(&target, &nil_id(), 0.0)];
    let seed = preview(&server, 0, 40, seed_ops.clone()).expect("seed preview");
    apply(&server, 0, 40, seed_ops, seed.approval).expect("seed apply");
    let fresh = ObjectId::new();
    let all_ops = [
        insert_dto(&fresh, &nil_id(), 1.0),
        OpDto::SetLocal {
            id: target.to_string(),
            local: pose(2.0),
        },
        OpDto::Reparent {
            id: target.to_string(),
            new_parent: nil_id(),
        },
        OpDto::Remove {
            id: target.to_string(),
        },
        OpDto::SetMaterial {
            id: target.to_string(),
            material: Some("hero-mat".to_owned()),
        },
        OpDto::SetComponent {
            id: target.to_string(),
            component: "mesh".to_owned(),
            value: rmcp::serde_json::json!({"asset": "meshes/wall"}),
            version: None,
        },
    ];
    for (index, dto) in all_ops.iter().enumerate() {
        let op = op_from_dto(index, dto).expect("advertised op converts");
        let discriminant = match op {
            Op::Insert { .. } => "insert",
            Op::Remove { .. } => "remove",
            Op::SetLocal { .. } => "set_local",
            Op::Reparent { .. } => "reparent",
            Op::SetMaterial { .. } => "set_material",
            Op::SetComponent { .. } => "set_component",
            Op::RemoveComponent { .. } => "remove_component",
            Op::SetBond { .. } => "set_bond",
            // The door's DTOs cover the node ops; the material-table and bond
            // ops are not on the wire yet.
            Op::PutMaterial { .. } => "put_material",
            Op::SetMaterialParam { .. } => "set_material_param",
            Op::RemoveLastMaterial => "remove_last_material",
        };
        let tag = match dto {
            OpDto::Insert { .. } => "insert",
            OpDto::Remove { .. } => "remove",
            OpDto::SetLocal { .. } => "set_local",
            OpDto::Reparent { .. } => "reparent",
            OpDto::SetMaterial { .. } => "set_material",
            OpDto::SetComponent { .. } => "set_component",
            OpDto::RemoveComponent { .. } => "remove_component",
        };
        assert_eq!(discriminant, tag, "DTO tag must match the Op variant");
    }
    // Each op alone must validate (all target live nodes here).
    for (key, dto) in all_ops.iter().enumerate() {
        let rev = read(&server, None, None).revision;
        preview(&server, rev, 50 + key as u64, vec![dto.clone()])
            .expect("advertised op shape must validate");
    }
    // The removal tag converts too (validation needs a set component).
    let removal = OpDto::RemoveComponent {
        id: target.to_string(),
        component: "mesh".to_owned(),
    };
    assert!(matches!(
        op_from_dto(0, &removal).expect("converts"),
        Op::RemoveComponent { .. }
    ));

    // Unknown op tag on the wire lists the allowed values, never an
    // empty pass.
    let wire = rmcp::serde_json::json!({
        "op": "teleport",
        "id": ObjectId::new().to_string(),
    });
    let error = rmcp::serde_json::from_value::<OpDto>(wire).expect_err("unknown op");
    let report = error.to_string();
    for allowed in [
        "insert",
        "remove",
        "set_local",
        "reparent",
        "set_material",
        "set_component",
        "remove_component",
    ] {
        assert!(
            report.contains(allowed),
            "unknown op must list `{allowed}`: {report}"
        );
    }
}

/// Count every `oneOf`/`anyOf` array length found in a schema value.
fn collect_branches(value: &rmcp::serde_json::Value) -> usize {
    use rmcp::serde_json::Value;
    let mut total = 0;
    match value {
        Value::Array(items) => {
            for item in items {
                total += collect_branches(item);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key == "oneOf" || key == "anyOf" {
                    if let Value::Array(branches) = item {
                        total += branches.len();
                    }
                } else {
                    total += collect_branches(item);
                }
            }
        }
        _ => {}
    }
    total
}

/// Collect every op tag under a `const`/`enum` key in a schema value.
fn collect_op_tags(value: &rmcp::serde_json::Value) -> Vec<String> {
    use rmcp::serde_json::Value;
    const KNOWN: &[&str] = &[
        "insert",
        "remove",
        "set_local",
        "reparent",
        "set_material",
        "set_component",
        "remove_component",
    ];
    let mut out = Vec::new();
    match value {
        Value::Array(items) => {
            for item in items {
                out.extend(collect_op_tags(item));
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                match key.as_str() {
                    "const" => {
                        if let Value::String(text) = item {
                            if KNOWN.contains(&text.as_str()) {
                                out.push(text.clone());
                            }
                        }
                    }
                    "enum" => {
                        if let Value::Array(options) = item {
                            for option in options {
                                if let Value::String(text) = option {
                                    if KNOWN.contains(&text.as_str()) {
                                        out.push(text.clone());
                                    }
                                }
                            }
                        }
                    }
                    _ => out.extend(collect_op_tags(item)),
                }
            }
        }
        _ => {}
    }
    out
}

#[test]
fn protocol_version_negotiation_answers_supported() {
    let server = test_server();
    assert_eq!(
        server.get_info().protocol_version,
        ProtocolVersion::V_2026_07_28
    );
    assert_eq!(
        server.supported_protocol_versions().as_ref(),
        ProtocolVersion::KNOWN_VERSIONS
    );

    // Every known version echoes: exact matches never answer -32022.
    for candidate in ProtocolVersion::KNOWN_VERSIONS {
        let agreed = negotiate_version(candidate).expect("exact match passes");
        assert_eq!(&agreed, candidate, "exact match echoes");
        let live = server
            .initialize_result(candidate)
            .expect("handshake answers exact match");
        assert_eq!(&live.protocol_version, candidate);
        assert!(live.capabilities.tools.is_some());
        assert!(server.supported_protocol_versions().contains(candidate));
    }

    // The EB-2026-09-26-79 client: 2025-06-18 handshakes and lists tools.
    let legacy = server
        .initialize_result(&ProtocolVersion::V_2025_06_18)
        .expect("2025-06-18 handshakes");
    assert_eq!(legacy.protocol_version, ProtocolVersion::V_2025_06_18);
    assert!(legacy.capabilities.tools.is_some());
    assert!(
        !server.tools().is_empty(),
        "legacy handshake still lists tools"
    );

    // Unknown versions fall back to a supported version, never -32022.
    let unknown: ProtocolVersion =
        rmcp::serde_json::from_str(r#""1999-01-01""#).expect("unknown version parses");
    let fallback = negotiate_version(&unknown).expect("mismatch answers");
    assert!(
        server.supported_protocol_versions().contains(&fallback),
        "fallback is supported: {fallback}"
    );
    let live = server
        .initialize_result(&unknown)
        .expect("handshake answers mismatch");
    assert!(
        server
            .supported_protocol_versions()
            .contains(&live.protocol_version),
        "handshake fallback is supported"
    );
}

/// The 21 tools of the old door, every one still reachable.
const OLD_DOOR: [&str; 21] = [
    "api.describe",
    "api.search",
    "asset.refs",
    "capture",
    "code.callers",
    "level.build",
    "log.digest",
    "log.query",
    "play.run",
    "play.step",
    "rpc.discover",
    "scene.brief",
    "scene.place",
    "scene.query",
    "scene.read",
    "scene.spatial",
    "scene.watch",
    "sim.plan_goals",
    "tx.apply",
    "tx.preview",
    "tx.undo",
];

/// Discovery chars of a tool list.
fn chars_of(tools: &[rmcp::model::Tool]) -> usize {
    tools.iter().map(|tool| tool_cost_row(tool).chars).sum()
}

fn assert_core_tools(server: &McpServer) -> Vec<String> {
    let mut names: Vec<String> = server
        .tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    let mut expected: Vec<String> = default_tool_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    expected.sort();
    assert_eq!(
        names, expected,
        "fresh session lists the core plus the scene pack"
    );
    for core in CORE_TOOL_NAMES {
        assert!(
            names.contains(&core.to_owned()),
            "core tool `{core}` is listed"
        );
    }
    names
}

fn assert_default_packs(names: &[String]) {
    assert!(
        names.contains(&"scene.brief".to_owned()),
        "scene pack is loaded by default"
    );
    assert!(
        !names.contains(&"log.digest".to_owned()),
        "view pack waits for api.load"
    );
}

fn assert_back_compat(server: &McpServer) -> Vec<String> {
    // Every old tool is reachable: a contract name, a router or pack
    // entry, and advertised once its pack loads.
    let reachable = tool_names();
    let mut all: Vec<String> = server
        .all_tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    all.sort();
    for old in OLD_DOOR {
        assert!(
            reachable.contains(&old),
            "old tool `{old}` stays a contract name"
        );
        assert!(
            all.contains(&old.to_owned()),
            "old tool `{old}` is reachable"
        );
    }
    let mut contract: Vec<&str> = reachable.clone();
    contract.sort_unstable();
    contract.dedup();
    assert_eq!(contract.len(), reachable.len(), "contract names are unique");
    assert_eq!(
        all,
        contract
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
        "all_tools covers exactly the contract names"
    );

    // No arbitrary execute/shell/file-write surface, by name or by hint.
    // `play.run` is allowlisted by exact name: it runs a bounded headless
    // arena match through the job registry, never arbitrary code.
    // `code.callers` is allowlisted by exact name: it is a read-only
    // reverse lookup over the Insight callers graph, never code execution.
    // `test.run` is allowlisted by exact name: it runs the read-only
    // harvest checks (scene, preview, budget, manifest, log), never arbitrary code.
    // `codex.search` and `codex.cite` are allowlisted by exact name: they
    // read the codex passage index and write one ref line in a Rome table
    // entry, never arbitrary code.
    const ALLOWED_BY_NAME: [&str; 5] = [
        "play.run",
        "code.callers",
        "test.run",
        "codex.search",
        "codex.cite",
    ];
    for tool in server.all_tools() {
        let name = tool.name.to_lowercase();
        if !ALLOWED_BY_NAME.contains(&name.as_str()) {
            for forbidden in ["exec", "shell", "file", "run", "code", "eval", "terminal"] {
                assert!(
                    !name.contains(forbidden),
                    "tool `{name}` looks like arbitrary execution"
                );
            }
        }
        assert!(
            !tool.input_schema.is_empty(),
            "tool `{name}` must advertise its input schema"
        );
        assert!(
            tool.output_schema.is_some(),
            "tool `{name}` must advertise its output schema"
        );
    }

    all
}

fn assert_budget_grows(server: &McpServer, all: &[String]) {
    // Token budget: the advertised list is shorter than the 21-tool door
    // (43514 chars budgeted, 40514 measured on 2026-10-01) and shorter
    // than the whole reachable set, with the ledger headroom intact.
    let budget = server.tool_list_budget_chars();
    let everything = chars_of(&server.all_tools());
    let core_only = chars_of(&server.core_tools());
    eprintln!(
        "door v2: default list {budget} chars, core {core_only}, all {} tools {everything} \
             chars, budget {TOOL_LIST_BUDGET_CHARS}",
        all.len()
    );
    assert!(budget > 0, "budget counts something");
    assert!(
        budget < TOOL_LIST_BUDGET_CHARS,
        "default list costs {budget} chars, the old door budgeted {TOOL_LIST_BUDGET_CHARS}"
    );
    // 2026-10-18: the SPR6 `blocked_by` and SPX3 `spatial_delta` schemas (and the two
    // `delta` flags) took the default list to 47.4k before trimming: bound 45_500 -> 47_500.
    // 2026-10-18 (backlog wave 2): five query kinds, role triage and verdict rows: 47_500 -> 48_500.
    // 2026-10-18 (endpoint wave): `scene.pick` took the default list to 50.9k: bound 48_500 -> 51_500.
    // 2026-10-18 (AX24): the `tx.apply` change list (AX24 `ChangeListDto`, revisions,
    // counts and rows) took the default list to 52.3k: bound 51_500 -> 53_000.
    // 2026-10-19 (PF24): the opt-in `timing` receipt (a flag on both play args plus
    // the label/p50/p99/samples object on both play outputs) took the default list to
    // 53.9k: bound 53_000 -> 54_500.
    // 2026-10-20 (GS46): `world.query` (nested data/filter plus the flat aliases) took
    // the default list to 55.8k: bound 54_500 -> 56_500.
    // 2026-10-21 (D19, the AI-native MCP lane): the verifier block on the three build
    // outputs (`level.build`, `game.build`, `sim.build_encounter`) grew the default list
    // past this bound. Measured on 2026-10-21: HEAD (37c33077) already stood at 56,737
    // chars, over the 56,500 bound (another lane's growth), and the nested block cost 290
    // chars per build tool (871 for the three). The wire shape is now one flat list of
    // lines (the bucket first, then one line per check), about 60 chars per build tool.
    // 2026-10-21 (API6): the merged verb and schema growth since GS46 measures 56_737
    // ledger chars on the default list (59_386 serialized, 22 tools), so bound
    // 56_500 -> 58_000. The API6 guardrail caps what a session advertises at the
    // ledger ceiling, so this bound still guards a doubling contract.
    // Both lanes grew the list on the same day from the same measurement, so the door
    // takes the looser of the two measured bounds.
    // 2026-10-23 (AD7): `scene.spatial` names default-sized nodes (`default_sized`,
    // the labelled fallback box a bare node gets), taking the default list to 58,512:
    // bound 58_500 -> 58_700.
    // 2026-10-23 (later window): three landed schemas grew the same list — AD2's
    // apply receipt digests (`a08254413`, `38dec9bb7`), SP66's dark/unknown node
    // rows (`c2a2c6b4c`) and view.compare's amplified diff image (`912a44723`) —
    // to 60,036, measured at HEAD 5b29bac02: bound 58_700 -> 60_200.
    // 2026-10-23 (AX23, `14fd8625c`): the inlined ChangeListDto schema (2,388
    // chars) plus its two args and the diff kind word took the default list to
    // 64,060: bound 60_200 -> 64_400.
    // 2026-10-23 (door budget): `scene.query` advertises the diff shape once
    // (`tx.apply` owns the inlined `ChangeListDto`), taking the default list to
    // 61,742: bound 64_400 -> 62_400.
    assert!(
        budget < 62_400,
        "default list {budget} stays near the measured 2026-10-23 door (61_742)"
    );
    assert!(budget < everything, "the default list hides something");
    assert!(core_only < budget, "the scene pack adds to the core");
    let ledger = server
        .tool_budget_gate_with_bloat(0)
        .expect("live tool list passes the ledger gate");
    assert_eq!(ledger.total_chars, budget, "ledger sums the live list");
    assert!(
        ledger.headroom_chars >= TOOL_LIST_MIN_HEADROOM_CHARS,
        "headroom {} chars under minimum {TOOL_LIST_MIN_HEADROOM_CHARS}",
        ledger.headroom_chars
    );

    // Loading a pack advertises its verbs and grows the list; the
    // ledger follows the live list.
    server
        .lock()
        .expect("test lock holds")
        .load_pack("view", false)
        .expect("view loads");
    let mut listed: Vec<String> = server
        .tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    listed.sort();
    assert!(listed.contains(&"log.digest".to_owned()));
    assert!(listed.contains(&"rpc.discover".to_owned()));
    let grown = server.tool_list_budget_chars();
    eprintln!(
            "door v2: view-loaded list {grown} chars, headroom {} chars, floor {TOOL_LIST_MIN_HEADROOM_CHARS}",
            TOOL_LIST_BUDGET_CHARS.saturating_sub(grown)
        );
    assert!(
        grown > budget,
        "loading view grows the list: {grown} vs {budget}"
    );
    assert_eq!(server.tool_budget_ledger().total_chars, grown);
    // The loaded list is a real advertised list too: the same ledger gate
    // must pass it, floor included, or `api.load` has no room to hand a
    // pack to a client (bug:endpoint-api-load-budget).
    let loaded_gate = server
        .tool_budget_gate_with_bloat(0)
        .expect("the loaded list passes the ledger gate with the floor intact");
    assert_eq!(loaded_gate.total_chars, grown);
    assert!(
        loaded_gate.headroom_chars >= TOOL_LIST_MIN_HEADROOM_CHARS,
        "loaded list leaves {} chars, floor is {TOOL_LIST_MIN_HEADROOM_CHARS}",
        loaded_gate.headroom_chars
    );
}

#[test]
fn door_v2_lists_core_plus_default_packs_and_keeps_every_old_tool_reachable() {
    let server = test_server();
    let names = assert_core_tools(&server);
    assert_default_packs(&names);
    let all = assert_back_compat(&server);
    assert_budget_grows(&server, &all);
}

/// D16: a progress note is a notification on the call, not a tool.
///
/// It carries the client's own token and the progress value, it never
/// becomes a second answer, and it costs no tool-list budget: the
/// advertised list holds no progress tool, and no description sells the
/// notification surface.
#[test]
fn progress_note_costs_no_tool_budget_and_carries_the_client_token() {
    // The long calls get a note; the short ones do not.
    assert!(is_long_call(TOOL_BUILD_LEVEL));
    assert!(is_long_call(TOOL_GAME_BUILD));
    assert!(is_long_call(TOOL_SIM_BUILD_ENCOUNTER));
    assert!(is_long_call(TOOL_PLAY_RUN));
    assert!(!is_long_call(TOOL_SCENE_READ));
    assert!(!is_long_call(TOOL_API_CALL));

    // The payload carries the client's own token and the progress value.
    let token = rmcp::model::ProgressToken(rmcp::model::NumberOrString::Number(7));
    let param = progress_note_params(&token, 0.5, "level.build: started");
    assert_eq!(param.progress_token, token);
    assert_eq!(param.progress, 0.5);
    assert_eq!(param.message.as_deref(), Some("level.build: started"));
    assert!(param.total.is_none(), "the total is unknown, not zero");

    // The same token, progress and message make the same payload: no wall
    // clock, no session id, nothing that changes between two sends.
    assert_eq!(
        param,
        progress_note_params(&token, 0.5, "level.build: started"),
        "the note is pure"
    );

    // No tool carries the progress surface: the list pays nothing for it.
    let server = test_server();
    for tool in server.tools() {
        let name = tool.name.to_lowercase();
        assert!(
            !name.contains("progress"),
            "tool `{name}` adds a progress surface"
        );
        let description = tool
            .description
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        // The check names the notification wording, not the plain word:
        // `play.run` already said "poll reads progress" about the job's
        // own progress (it predates D16), and that word is not a
        // notification surface.
        assert!(
            !description.contains("progress notification")
                && !description.contains("progress token"),
            "tool `{name}` advertises a progress surface"
        );
    }
    println!("progress_note: progress notifications sent, 0 tool-budget chars PASS");
}

#[test]
fn component_ops_set_undo_and_refuse_typed_through_the_door() {
    let server = test_server();
    let node = ObjectId::new();
    let seed = vec![insert_dto(&node, &nil_id(), 0.0)];
    let drafted = preview(&server, 0, 70, seed.clone()).expect("seed previews");
    apply(&server, 0, 70, seed, drafted.approval).expect("seed lands");

    // All eight kinds in one batch, authored as plain JSON.
    let kinds = [
        ("mesh", rmcp::serde_json::json!({"asset": "meshes/wall"})),
        (
            "collider",
            rmcp::serde_json::json!({"shape": "box", "half": [1.0, 2.0, 0.2]}),
        ),
        ("material", rmcp::serde_json::json!({"asset": "marble"})),
        (
            "light",
            rmcp::serde_json::json!({"kind": "point", "color": [1, 1, 1], "intensity": 5, "range": 10}),
        ),
        (
            "spawner",
            rmcp::serde_json::json!({"archetype": "grunt", "count": 2, "period_ticks": 300}),
        ),
        (
            "destructible",
            rmcp::serde_json::json!({"material": "stone", "health": 100, "chunks": 32}),
        ),
        (
            "anatomy",
            rmcp::serde_json::json!({"template": "humanoid", "limbs": 4}),
        ),
        (
            "rig",
            rmcp::serde_json::json!({"skeleton": "hero", "clips": "hero_clips"}),
        ),
    ];
    let ops: Vec<OpDto> = kinds
        .iter()
        .map(|(kind, value)| OpDto::SetComponent {
            id: node.to_string(),
            component: (*kind).to_owned(),
            value: value.clone(),
            version: None,
        })
        .collect();
    let drafted = preview(&server, 1, 71, ops.clone()).expect("components preview");
    assert_eq!(drafted.effects.len(), 8);
    assert!(
        drafted.effects[1].contains("collider"),
        "{:?}",
        drafted.effects
    );
    let landed = apply(&server, 1, 71, ops, drafted.approval).expect("components land");
    assert_eq!(landed.revision, 2);
    {
        let state = server.lock().expect("lock");
        let slot = state.doc.components.get(&node.to_string()).expect("slot");
        assert_eq!(slot.len(), 8);
    }

    // Bad kind, bad payload, bad range and unknown version refuse typed.
    let refuse = |component: &str, value: rmcp::serde_json::Value, version: Option<u32>| {
        preview(
            &server,
            2,
            80,
            vec![OpDto::SetComponent {
                id: node.to_string(),
                component: component.to_owned(),
                value,
                version,
            }],
        )
        .expect_err("refused")
    };
    let bad_kind = refuse("teleporter", rmcp::serde_json::json!({}), None);
    assert_eq!(bad_kind.code, "bad_component");
    assert!(
        bad_kind.message.contains("mesh, collider"),
        "{}",
        bad_kind.message
    );
    let bad_field = refuse(
        "mesh",
        rmcp::serde_json::json!({"asset": "a", "extra": 1}),
        None,
    );
    assert_eq!(bad_field.code, "bad_component");
    let bad_range = refuse(
        "spawner",
        rmcp::serde_json::json!({"archetype": "a", "count": 1, "period_ticks": 0}),
        None,
    );
    assert!(
        bad_range.message.contains("period_ticks"),
        "{}",
        bad_range.message
    );
    let future = refuse("mesh", rmcp::serde_json::json!({"asset": "a"}), Some(9));
    assert!(future.message.contains("v9"), "{}", future.message);

    // Undo restores the pre-batch document (no components).
    TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
    assert!(server.lock().expect("lock").doc.components.is_empty());
}

#[test]
fn approvals_bind_the_engine_session_not_the_protocol_session() {
    // The wire approval carries no session at all: `tx.apply` stamps the
    // server's own engine session (fixed at construction), so nothing a
    // protocol handshake says can move the binding. Two engine sessions
    // keep two ledgers: a spend on one never consumes the other's.
    let seven = McpServer::new(7, Capability::unrestricted(7));
    let nine = McpServer::new(9, Capability::unrestricted(9));
    assert_eq!(seven.lock().expect("lock").session(), 7);
    assert_eq!(nine.lock().expect("lock").session(), 9);
    let id = ObjectId::new();
    let ops = vec![insert_dto(&id, &nil_id(), 1.0)];
    let minted = preview(&seven, 0, 500, ops.clone()).expect("preview mints");
    // The protocol handshake, at any version, never moves the engine
    // session or the minted binding.
    for candidate in ProtocolVersion::KNOWN_VERSIONS {
        seven
            .initialize_result(candidate)
            .expect("handshake answers");
    }
    assert_eq!(seven.lock().expect("lock").session(), 7);
    let landed =
        apply(&seven, 0, 500, ops.clone(), minted.approval.clone()).expect("own session lands");
    assert_eq!(landed.revision, 1);
    // Single use per (engine session, batch hash): replay refuses.
    let replay = apply(&seven, 0, 501, ops.clone(), minted.approval.clone())
        .expect_err("a spent approval refuses");
    assert!(
        !replay.code.is_empty(),
        "the replay refuses with a typed code"
    );
    // The other engine session spends from its own ledger.
    let own = preview(&nine, 0, 500, ops.clone()).expect("nine mints its own");
    let landed = apply(&nine, 0, 500, ops, own.approval).expect("nine lands");
    assert_eq!(landed.revision, 1);
    // The stdio door binds one fixed engine session per process.
    assert_eq!(super::super::startup::MCP_SESSION, 1);
}

#[test]
fn tool_cost_table_sums_to_total_and_posts_rows() {
    let server = test_server();
    let ledger = server.tool_budget_ledger();
    // Posted: one ledger line per tool, so budget growth names its owner.
    for row in &ledger.rows {
        eprintln!("tool cost: {:<16} {:>6} chars", row.name, row.chars);
    }
    eprintln!(
        "tool budget: total {} chars of {}, headroom {}",
        ledger.total_chars, ledger.budget_chars, ledger.headroom_chars
    );
    assert_eq!(ledger.rows.len(), default_tool_names().len());
    let mut names: Vec<&str> = ledger.rows.iter().map(|row| row.name.as_str()).collect();
    names.sort_unstable();
    let mut expected = default_tool_names();
    expected.sort_unstable();
    assert_eq!(
        names.as_slice(),
        expected.as_slice(),
        "cost table covers exactly the advertised tools"
    );
    for row in &ledger.rows {
        assert!(row.chars > 0, "tool `{}` must cost something", row.name);
    }
    let sum: usize = ledger.rows.iter().map(|row| row.chars).sum();
    assert_eq!(sum, ledger.total_chars, "rows sum to the ledger total");
    assert_eq!(
        ledger.total_chars,
        server.tool_list_budget_chars(),
        "ledger total matches the advertised total"
    );
    assert_eq!(ledger.budget_chars, TOOL_LIST_BUDGET_CHARS);
    assert_eq!(
        ledger.headroom_chars,
        TOOL_LIST_BUDGET_CHARS.saturating_sub(ledger.total_chars)
    );
}

#[test]
fn seeded_bloat_of_500_chars_fails_the_gate() {
    // Seed a total one char under the ceiling: the 500-char probe overflows.
    let seeded = TOOL_LIST_BUDGET_CHARS - (TOOL_BUDGET_PROBE_BLOAT_CHARS - 1);
    let refused = check_tool_budget(seeded, TOOL_BUDGET_PROBE_BLOAT_CHARS)
        .expect_err("seeded 500-char bloat must fail the gate");
    assert_eq!(refused.code, "tool_list_bloated");
    assert!(
        refused
            .message
            .contains(TOOL_LIST_BUDGET_CHARS.to_string().as_str()),
        "refusal names the budget: {}",
        refused.message
    );
    // The live list still passes unbloated, ledger in hand.
    let server = test_server();
    let ledger = server
        .tool_budget_gate_with_bloat(0)
        .expect("live list passes unbloated");
    assert_eq!(ledger.total_chars, server.tool_list_budget_chars());
}

#[test]
fn headroom_below_2k_forces_raise_or_refusal() {
    // One char under the minimum headroom: refused, with the raise rule.
    let tight = TOOL_LIST_BUDGET_CHARS - (TOOL_LIST_MIN_HEADROOM_CHARS - 1);
    let refused = check_tool_budget(tight, 0).expect_err("sub-minimum headroom must fail");
    assert_eq!(refused.code, "budget_headroom_exhausted");
    assert!(
        refused
            .message
            .contains(TOOL_LIST_MIN_HEADROOM_CHARS.to_string().as_str()),
        "refusal names the minimum: {}",
        refused.message
    );
    assert!(
        refused.message.contains("raise"),
        "refusal forces raise-with-reason or shrink: {}",
        refused.message
    );
    // Exactly the minimum headroom passes and reports it.
    let floor = TOOL_LIST_BUDGET_CHARS - TOOL_LIST_MIN_HEADROOM_CHARS;
    let headroom = check_tool_budget(floor, 0).expect("exact minimum headroom passes");
    assert_eq!(headroom, TOOL_LIST_MIN_HEADROOM_CHARS);
}

#[test]
fn scene_read_paging_and_bounds() {
    let server = test_server();
    let mut ids = Vec::new();
    for key in 60u64..63 {
        let id = ObjectId::new();
        let rev = read(&server, None, None).revision;
        let ops = vec![insert_dto(&id, &nil_id(), key as f32)];
        let drafted = preview(&server, rev, key, ops.clone()).expect("seed preview");
        apply(&server, rev, key, ops, drafted.approval).expect("seed apply");
        ids.push(id.to_string());
    }
    ids.sort();

    // Pages walk the id-sorted rows.
    let first = read(&server, Some(2), Some(0));
    assert_eq!(first.total, 3);
    assert_eq!(first.returned, 2);
    assert!(first.truncated);
    let second = read(&server, Some(2), Some(2));
    assert_eq!(second.returned, 1);
    assert!(!second.truncated);
    let rows: Vec<String> = first
        .objects
        .iter()
        .map(|row| row.id.clone())
        .chain(second.objects.iter().map(|row| row.id.clone()))
        .collect();
    assert_eq!(rows, ids);

    // Out-of-range paging fails with the legal values attached.
    let over_limit = SceneReadTool::invoke(
        &server,
        SceneReadArgs {
            limit: Some(999),
            cursor: None,
        },
    )
    .expect_err("limit over the cap must fail");
    assert_eq!(over_limit.code, "bad_limit");
    assert!(over_limit.message.contains("128"));

    let zero_limit = SceneReadTool::invoke(
        &server,
        SceneReadArgs {
            limit: Some(0),
            cursor: None,
        },
    )
    .expect_err("zero limit must fail");
    assert_eq!(zero_limit.code, "bad_limit");

    let past_end = SceneReadTool::invoke(
        &server,
        SceneReadArgs {
            limit: None,
            cursor: Some(99),
        },
    )
    .expect_err("cursor past the end must fail");
    assert_eq!(past_end.code, "bad_cursor");
    assert!(past_end.message.contains('3'));
}

#[test]
fn empty_and_oversize_batches_are_rejected() {
    let server = test_server();
    let before = snapshot(&server);

    let empty = preview(&server, 0, 70, vec![]).expect_err("empty batch must fail");
    assert_eq!(empty.code, "empty_batch");
    assert!(empty.message.contains("256"));

    let id = ObjectId::new();
    let huge = vec![insert_dto(&id, &nil_id(), 0.0); MAX_TX_OPS + 1];
    let oversize = preview(&server, 0, 71, huge).expect_err("oversize batch must fail");
    assert_eq!(oversize.code, "too_many_ops");
    assert!(oversize.message.contains("256"));
    before.assert_same_all(&server);
}

#[test]
fn undo_token_mismatch_and_empty_undo_fail() {
    let server = test_server();
    let empty_undo = TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None })
        .expect_err("empty journal must fail");
    assert_eq!(empty_undo.code, "nothing_to_undo");

    let id = ObjectId::new();
    let rev = read(&server, None, None).revision;
    let ops = vec![insert_dto(&id, &nil_id(), 0.0)];
    let drafted = preview(&server, rev, 80, ops.clone()).expect("preview");
    apply(&server, rev, 80, ops, drafted.approval).expect("apply");

    let before = snapshot(&server);
    let mismatch = TxUndoTool::invoke(
        &server,
        TxUndoArgs {
            undo_token: Some(81),
        },
    )
    .expect_err("wrong token must fail");
    assert_eq!(mismatch.code, "undo_mismatch");
    assert!(mismatch.message.contains("80"));
    before.assert_same_all(&server);
}

#[test]
fn service_rehydrate_replays_recorded_result_and_undoes() {
    let server = test_server();
    let id = ObjectId::new();
    let ops = vec![insert_dto(&id, &nil_id(), 9.0)];
    let drafted = preview(&server, 0, 6201, ops.clone()).expect("preview");
    let first =
        apply(&server, 0, 6201, ops.clone(), drafted.approval.clone()).expect("apply lands");
    assert_eq!(first.revision, 1);

    // Restart: the sealed pair lands in a fresh service via rehydrate.
    let (doc, journal) = {
        let state = server.lock().expect("test lock holds");
        (state.doc.clone(), state.journal.clone())
    };
    let fresh = test_server();
    fresh
        .lock()
        .expect("test lock holds")
        .rehydrate(doc, journal);

    // Same key plus same payload replays the recorded result without a
    // new revision; undo still works after rehydrate.
    let replayed = apply(&fresh, 0, 6201, ops, drafted.approval).expect("stored key replays");
    assert_eq!(replayed.revision, first.revision);
    assert_eq!(read(&fresh, None, None).revision, 1);
    let undone =
        TxUndoTool::invoke(&fresh, TxUndoArgs { undo_token: None }).expect("rehydrated undo works");
    assert_eq!(undone.revision, 2);
    assert_eq!(read(&fresh, None, None).revision, 2);
}

#[test]
fn wrap_re_codes_new_family_with_data_preserved() {
    // The post-snapshot family (`bad_word`, `bad_wall`, `bad_receipt`,
    // `bad_brief`) layers a new code over a wire refusal without dropping
    // the culprit: `wrap` keeps message and data, and the result still
    // round-trips through the canonical `{code, message, data}` shape.
    for code in ["bad_word", "bad_wall", "bad_receipt", "bad_brief"] {
        let culprit = rmcp::serde_json::json!({"item": code});
        let wire = crate::discover::RpcError::new(code, format!("{code} probe"))
            .with_data(culprit.clone());
        let layered = McpToolError::wrap("subscribe_failed", wire);
        assert_eq!(layered.code(), "subscribe_failed");
        assert!(layered.message.contains(code));
        assert_eq!(layered.data(), Some(&culprit));
        let back = crate::discover::RpcError::from(layered);
        assert_eq!(back.code(), "subscribe_failed");
        assert_eq!(back.data, Some(culprit));
    }
}
