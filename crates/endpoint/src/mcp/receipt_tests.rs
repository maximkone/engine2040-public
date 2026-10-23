//! Receipt content digests (AD2): one landed write and its idempotent replay
//! carry the same before, staged and after digests.
//!
//! Kept out of `mcp/tests.rs` so a shared tree can prove this row without
//! staging another writer's hunks in that file.

use super::*;
use rmcp::handler::server::router::tool::SyncTool;

fn pose(x: f32) -> PoseDto {
    PoseDto {
        pos: [x, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

fn insert_dto(id: &ObjectId, parent: &str, x: f32) -> OpDto {
    OpDto::Insert {
        id: id.to_string(),
        parent: parent.to_owned(),
        local: pose(x),
    }
}

/// True for 64 lowercase hex chars: one 32-byte digest.
fn is_digest_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[test]
fn receipt_digests_survive_a_replay() {
    let server = McpServer::new(7, Capability::unrestricted(7));
    let id = ObjectId::new();
    let ops = vec![insert_dto(&id, &ObjectId::NIL.to_string(), 1.0)];
    let drafted = TxPreviewTool::invoke(
        &server,
        TxPreviewArgs {
            expected_revision: 0,
            idempotency_key: 91,
            ops: ops.clone(),
        },
    )
    .expect("preview inserts");

    let first = TxApplyTool::invoke(
        &server,
        TxApplyArgs {
            expected_revision: 0,
            idempotency_key: 91,
            ops: ops.clone(),
            approval: drafted.approval.clone(),
            delta: None,
            origin: None,
        },
    )
    .expect("apply inserts");
    for digest in [
        &first.before_digest,
        &first.staged_digest,
        &first.after_digest,
    ] {
        assert!(is_digest_hex(digest), "not a 64-hex digest: {digest}");
    }
    assert_ne!(
        first.before_digest, first.after_digest,
        "an insert changes the document"
    );
    assert_eq!(
        first.staged_digest, first.after_digest,
        "in-memory landing must equal the staged content"
    );

    let replay = TxApplyTool::invoke(
        &server,
        TxApplyArgs {
            expected_revision: 0,
            idempotency_key: 91,
            ops,
            approval: drafted.approval,
            delta: None,
            origin: None,
        },
    )
    .expect("same key and payload replay");
    assert_eq!(replay.revision, first.revision);
    assert_eq!(replay.before_digest, first.before_digest);
    assert_eq!(replay.staged_digest, first.staged_digest);
    assert_eq!(replay.after_digest, first.after_digest);
}
