//! Refusal mapping for the MCP door: command and capability refusals become
//! tool errors with the legal values or the culprit named. Re-exported from
//! `mcp` so the door and its sibling tools keep their existing paths.

use engine_editor::capability::{CapError, Capability, OpMask};
use engine_editor::commands::{self, Document};

use super::McpToolError;

/// Map a command refusal to a tool error with the legal values attached.
/// Crate-visible: the scene tools map staging failures the same way.
pub(crate) fn batch_error(error: &commands::BatchError) -> McpToolError {
    match error {
        commands::BatchError::StaleRevision { expected, got } => McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {expected}, batch expects {got}; \
                 re-read scene.read, re-preview, then apply"
            ),
        ),
        commands::BatchError::DuplicateKey { key } => McpToolError::new(
            "duplicate_key",
            format!(
                "idempotency key {key} was already used with a different payload; \
                 submit a new batch with a fresh key"
            ),
        ),
        commands::BatchError::UndoneKey { key } => McpToolError::new(
            "undone_key",
            format!(
                "idempotency key {key} belongs to an undone batch; \
                 submit a new batch with a fresh key to reapply"
            ),
        ),
        _ => McpToolError::new("invalid_batch", error.to_string()),
    }
}

/// Map a capability refusal to a tool error. `granted` is the op-class list
/// snapshotted before the gated apply, so denials name what is allowed. `doc`
/// is the document the refused apply left unchanged: spatial refusals name
/// their culprit objects from it. Crate-visible: `scene.place` applies through
/// the same gate.
pub(crate) fn cap_error(error: &CapError, granted: &[&str], doc: &Document) -> McpToolError {
    match error {
        CapError::ForgedApproval => McpToolError::new(
            "forged_approval",
            "approval does not match this session and batch; \
             re-run tx.preview for a fresh approval",
        ),
        CapError::StaleRevision { expected, got } => McpToolError::new(
            "stale_revision",
            format!(
                "document is at revision {expected}, approval expects {got}; \
                 re-read scene.read, re-preview, then apply"
            ),
        ),
        CapError::Expired { expiry, current } => McpToolError::new(
            "expired_approval",
            format!(
                "approval expired at revision {expiry}, document is at {current}; \
                 re-run tx.preview for a fresh approval"
            ),
        ),
        CapError::ReusedApproval => McpToolError::new(
            "reused_approval",
            "approval was already spent; re-run tx.preview for a fresh approval",
        ),
        CapError::OpNotAllowed { op } => McpToolError::new(
            "op_not_allowed",
            format!(
                "capability does not grant `{op}` ops; granted: [{}]",
                granted.join(", ")
            ),
        ),
        CapError::OutOfScope { id } => McpToolError::new(
            "out_of_scope",
            format!("node {id} is outside this session's object scope"),
        ),
        CapError::OverBudget { remaining, needed } => McpToolError::new(
            "over_budget",
            format!("batch needs {needed} ops, {remaining} left in this session"),
        ),
        CapError::Spatial { check, culprits } => {
            crate::scene_tools::refusals::spatial_refusal(doc, check, culprits)
        }
        CapError::Command(inner) => batch_error(inner),
    }
}

/// Op classes granted by a capability, for denial messages.
/// Crate-visible: `scene.place` snapshots the same list before its apply.
pub(crate) fn granted_ops(cap: &Capability) -> Vec<&'static str> {
    let mut granted = Vec::with_capacity(4);
    if cap.ops.contains(OpMask::INSERT) {
        granted.push("insert");
    }
    if cap.ops.contains(OpMask::MOVE) {
        granted.push("move");
    }
    if cap.ops.contains(OpMask::EDIT) {
        granted.push("edit");
    }
    if cap.ops.contains(OpMask::DELETE) {
        granted.push("delete");
    }
    granted
}
