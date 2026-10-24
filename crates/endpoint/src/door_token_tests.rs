//! EA6 accept proofs: the door token refuses what the host policy does not
//! grant, a wrong-version policy fails closed at startup, and a default-policy
//! session still lands insert, move and set batches.

use std::path::PathBuf;

use engine_editor::capability::{self, ApprovalLedger, CapError, OpMask};
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use super::*;
use crate::explain::ExplainCaps;
use crate::startup::{build_server, StartupConfig, StartupError};
use crate::tool_capabilities::{
    message_carries_recovery, parse_scope_policy, refuse_write, ScopePolicyError,
};

/// Fresh scratch directory per test (parallel-safe: unique names).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("e2040-ea6-{n}-{name}", n = std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir creates");
    dir
}

fn insert_op(id: ObjectId, x: f32) -> Op {
    Op::Insert {
        id,
        parent: ObjectId::NIL,
        local: Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
    }
}

fn batch(ops: Vec<Op>, base_revision: u64, key: u64) -> Batch {
    Batch {
        ops,
        base_revision,
        idempotency_key: key,
        provenance: None,
    }
}

/// One node at revision 1, so a delete and a move both have a target.
fn seeded() -> (Document, Journal, ObjectId) {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let id = ObjectId::new();
    let seed = batch(vec![insert_op(id, 0.0)], 0, 11);
    commands::apply(&mut doc, &mut journal, &seed).expect("seed applies");
    (doc, journal, id)
}

fn insert_only() -> ScopePolicy {
    parse_scope_policy("scope-policy-version: 1\nscope-policy-ops: insert\n")
        .expect("insert-only policy parses")
}

#[test]
fn door_token_insert_only_policy_refuses_a_delete_batch() {
    let policy = insert_only();
    assert_eq!(policy.ops(), OpMask::INSERT);
    let mut token = token_from_policy(&policy, 1);
    let (mut doc, mut journal, id) = seeded();
    let before_doc = doc.clone();
    let before_journal = journal.past_len();
    let delete = batch(vec![Op::Remove { id }], doc.revision, 21);

    // The door's own write gate names the missing class with the capability
    // code and a recovery, before any approval is spent.
    let refusal = refuse_write(&doc, &delete, &token, &ExplainCaps::default_caps())
        .expect("an insert-only token cannot delete");
    assert_eq!(refusal.code, "op_not_allowed");
    assert!(refusal.message.contains("delete"), "{refusal}");
    assert!(message_carries_recovery(&refusal.message), "{refusal}");

    // The gate underneath refuses too, and the journal never moves.
    let approval = capability::approve(1, &delete, doc.revision, 99);
    let mut ledger = ApprovalLedger::new();
    let error = capability::apply_gated(
        &mut doc,
        &mut journal,
        &delete,
        &mut token,
        &approval,
        &mut ledger,
    )
    .expect_err("the token refuses the delete");
    assert_eq!(
        error,
        CapError::OpNotAllowed { op: "delete" },
        "the gate names the delete class"
    );
    assert_eq!(
        journal.past_len(),
        before_journal,
        "the journal is unchanged"
    );
    assert_eq!(doc, before_doc, "the document is unchanged");
}

#[test]
fn door_token_wrong_version_or_bad_ops_fails_closed_at_start() {
    let dir = scratch("policy-version");
    let stale = dir.join("stale.policy");
    std::fs::write(&stale, "scope-policy-version: 999\n").expect("stale policy writes");
    let error = build_server(&StartupConfig {
        project_dir: dir.clone(),
        web: None,
        policy: Some(stale.clone()),
    })
    .err()
    .expect("a wrong-version policy must not open the door");
    assert!(
        matches!(error, StartupError::BadPolicy { .. }),
        "stale policy is a typed start failure: {error:?}"
    );
    assert_ne!(error.exit_code(), 0, "a refused door exits nonzero");
    assert!(error.to_string().contains("999"), "{error}");

    // An unknown op class is the same fail-closed shape, never a silent grant.
    let bad_ops = dir.join("bad-ops.policy");
    std::fs::write(
        &bad_ops,
        "scope-policy-version: 1\nscope-policy-ops: insert,shell\n",
    )
    .expect("bad ops policy writes");
    let error = build_server(&StartupConfig {
        project_dir: dir.clone(),
        web: None,
        policy: Some(bad_ops),
    })
    .err()
    .expect("an unknown op class must not open the door");
    assert!(matches!(error, StartupError::BadPolicy { .. }), "{error:?}");

    // A missing policy file is a refusal too, not an empty grant.
    let error = build_server(&StartupConfig {
        project_dir: dir.clone(),
        web: None,
        policy: Some(dir.join("missing.policy")),
    })
    .err()
    .expect("a missing policy must not open the door");
    assert!(matches!(error, StartupError::BadPolicy { .. }), "{error:?}");
    let _ = std::fs::remove_dir_all(&dir);

    // The parser itself names every legal class plus one recovery.
    let parse_error = parse_scope_policy("scope-policy-version: 1\nscope-policy-ops: nope\n")
        .expect_err("unknown class refuses");
    assert_eq!(parse_error, ScopePolicyError::BadOps);
    let text = parse_error.to_string();
    for word in ["insert", "move", "set", "delete", "re-run"] {
        assert!(text.contains(word), "policy error misses `{word}`: {text}");
    }
}

#[test]
fn door_token_scope_policy_flag_parses_both_forms() {
    use crate::startup::parse_args;
    let words = [
        "e2040-mcp",
        "--scope-policy",
        "p.policy",
        "--project-dir",
        "proj",
    ];
    let argv: Vec<String> = words.iter().map(ToString::to_string).collect();
    let config = parse_args(&argv).expect("flag form parses");
    assert_eq!(config.policy, Some(PathBuf::from("p.policy")));
    assert_eq!(config.project_dir, PathBuf::from("proj"));

    let words = ["e2040-mcp", "--scope-policy=p.policy"];
    let argv: Vec<String> = words.iter().map(ToString::to_string).collect();
    let config = parse_args(&argv).expect("equals form parses");
    assert_eq!(config.policy, Some(PathBuf::from("p.policy")));

    // No flag keeps the shipped policy: every op class, the old door behavior.
    let argv: Vec<String> = ["e2040-mcp"].iter().map(ToString::to_string).collect();
    assert_eq!(parse_args(&argv).expect("bare parses").policy, None);

    for words in [
        vec!["e2040-mcp", "--scope-policy"],
        vec!["e2040-mcp", "--scope-policy="],
        vec!["e2040-mcp", "--scope-policy", "a", "--scope-policy", "b"],
    ] {
        let argv: Vec<String> = words.iter().map(ToString::to_string).collect();
        let error = parse_args(&argv).expect_err("misuse refuses");
        assert!(matches!(error, StartupError::Usage { .. }), "{error:?}");
    }
}

#[test]
fn door_token_default_policy_session_lands_insert_move_and_set() {
    let policy = ScopePolicy::default_file();
    assert_eq!(
        policy.ops(),
        OpMask::ALL,
        "the shipped file grants every class"
    );
    let mut token = token_from_policy(&policy, 1);
    assert_eq!(token.remaining(), DOOR_EDIT_BUDGET);
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let mut ledger = ApprovalLedger::new();
    let caps = ExplainCaps::default_caps();
    let parent = ObjectId::new();
    let child = ObjectId::new();

    // insert, insert, move (reparent), set (set-local): four landed batches.
    let steps: Vec<Vec<Op>> = vec![
        vec![insert_op(parent, 0.0)],
        vec![insert_op(child, 2.0)],
        vec![Op::Reparent {
            id: child,
            new_parent: parent,
        }],
        vec![Op::SetLocal {
            id: child,
            local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        }],
    ];
    for (index, ops) in steps.into_iter().enumerate() {
        let key = 31 + u64::try_from(index).expect("small index");
        let batch = batch(ops, doc.revision, key);
        assert!(
            refuse_write(&doc, &batch, &token, &caps).is_none(),
            "the default policy admits batch {index}"
        );
        let approval = capability::approve(1, &batch, doc.revision, 99);
        let landed = capability::apply_gated(
            &mut doc,
            &mut journal,
            &batch,
            &mut token,
            &approval,
            &mut ledger,
        )
        .expect("the default token lands the batch");
        assert_eq!(
            landed.revision,
            u64::try_from(index).expect("small index") + 1
        );
    }
    assert_eq!(
        journal.past_len(),
        4,
        "every default-policy batch journaled"
    );
    assert_eq!(doc.revision, 4);
    assert!(
        doc.hierarchy.parent(child) == Some(parent),
        "the move landed"
    );
}
