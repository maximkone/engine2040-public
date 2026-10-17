//! MCP JSON-RPC conformance vectors: 12/12 code agreement.
//!
//! Consumer: MCP clients over the `e2040-mcp` door plus the review panel that
//! pins the wire contract. Each vector starts from one [`McpToolError`],
//! converts to the wire [`RpcError`], crosses the transport [`ErrorData`]
//! envelope and decodes back with the same machine `code` at every hop.
//! Door refusals always ride `Err(ErrorData)` with JSON-RPC code
//! `INVALID_PARAMS` (-32602); forged and stale refusals never downgrade to an
//! `Ok` tool result, and foreign envelopes never decode as door refusals.
//!
//! Attribution: the error-mapping pattern (protocol error as `Err(ErrorData)`
//! with a JSON-RPC code, tool finding as `Ok(CallToolResult)` with `is_error`)
//! follows modelcontextprotocol/rust-sdk (Apache-2.0, license verified). Every
//! vector below is fresh, written against our `RpcError` API in
//! `crates/endpoint/src/discover.rs`; no donor code is copied.

#![cfg(feature = "mcp")]

use engine_endpoint::discover::RpcError;
use engine_endpoint::mcp::McpToolError;
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};

/// Twelve door codes with a message each: the two transport-guard codes
/// (`forged_approval`, `stale_revision`) first, then ten representative
/// refusals spanning ids, poses, batches, approvals, jobs and dispatch.
const VECTORS: [(&str, &str); 12] = [
    (
        "forged_approval",
        "approval is forged; preview again at the live revision for a fresh approval",
    ),
    (
        "stale_revision",
        "expected revision 99 is stale; re-read scene.read for the live revision",
    ),
    (
        "denied_op",
        "method `exec` is denied; mapped methods: [list, query, get, set, spawn, watch]",
    ),
    (
        "unknown_tool",
        "unknown tool `shell.exec`; the door serves scene.read, tx.preview, tx.apply, tx.undo",
    ),
    (
        "bad_id",
        "ops[0].id is not an object id (expected UUID text; the nil id means no parent)",
    ),
    (
        "bad_pose",
        "ops[0].local holds a non-finite lane; poses must be finite (meters, quaternion, scale)",
    ),
    (
        "empty_batch",
        "batch holds no ops; preview needs 1..=256 edits",
    ),
    (
        "expired_approval",
        "approval expired at revision 4; preview again at the live revision",
    ),
    (
        "reused_approval",
        "approval already spent its idempotency key; preview again for a fresh approval",
    ),
    (
        "duplicate_key",
        "idempotency key 4242 already landed a batch; reuse answers the first receipt",
    ),
    (
        "stale_tick",
        "tick 90 is stale; the duel already runs past it",
    ),
    (
        "unknown_subscription",
        "subscription 7 is gone; subscribe again through scene.watch",
    ),
];

/// One hop of the door wire: tool shape converts, the envelope carries
/// `INVALID_PARAMS`, and the decode agrees on the machine code.
fn agree_on_code(code: &'static str, message: &'static str) {
    let tool =
        McpToolError::new(code, message).with_data(rmcp::serde_json::json!({"vector": code}));
    assert_eq!(tool.code(), code, "tool keeps its code");
    let wire = RpcError::from(tool);
    assert_eq!(wire.code(), code, "wire keeps the code");
    assert_eq!(
        wire.data(),
        Some(&rmcp::serde_json::json!({"vector": code})),
        "wire keeps the culprit"
    );
    let envelope = ErrorData::from(wire.clone());
    assert_eq!(
        envelope.code,
        ErrorCode::INVALID_PARAMS,
        "door errors ride invalid_params"
    );
    assert_eq!(envelope.code.0, -32602, "invalid_params is JSON-RPC -32602");
    assert!(
        envelope.message.contains(code),
        "envelope names the code: {}",
        envelope.message
    );
    let text = rmcp::serde_json::to_string(&wire).expect("error serializes");
    let back: RpcError = rmcp::serde_json::from_str(&text).expect("error round-trips");
    assert_eq!(back, wire, "code `{code}` survives JSON");
    let decoded = RpcError::from_wire(&envelope).expect("door envelopes decode back");
    assert_eq!(decoded, wire, "code `{code}` survives the envelope");
    assert_eq!(decoded.code(), code, "decode agrees on the code");
}

#[test]
fn jsonrpc_conformance_12_vectors_agree_on_code() {
    let mut agreed = 0usize;
    for (code, message) in VECTORS {
        // The table holds machine codes only: non-empty, lowercase, no spaces.
        assert!(
            !code.is_empty()
                && code
                    .chars()
                    .all(|lane| lane.is_ascii_lowercase() || lane == '_' || lane.is_ascii_digit()),
            "vector code `{code}` stays a machine code"
        );
        agree_on_code(code, message);
        agreed += 1;
    }
    assert_eq!(agreed, 12, "all 12 vectors agree on code");
}

#[test]
fn invalid_params_is_error_and_forged_stale_stay_transport_err() {
    // Tool findings that the caller must see carry `is_error`.
    let seen = CallToolResult::structured_error(rmcp::serde_json::json!({
        "code": "forged_approval",
        "message": "approval is forged",
    }));
    assert_eq!(seen.is_error, Some(true), "error results flag is_error");
    // Door refusals never ride `Ok`: forged and stale stay transport `Err`
    // with the INVALID_PARAMS code and a decodable machine code.
    for code in ["forged_approval", "stale_revision"] {
        let tool = McpToolError::new(code, format!("{code} refuses the probe; re-read first"));
        let refusal = ErrorData::from(RpcError::from(tool));
        assert_eq!(
            refusal.code,
            ErrorCode::INVALID_PARAMS,
            "code `{code}` rides invalid_params"
        );
        let decoded = RpcError::from_wire(&refusal).expect("door envelopes decode back");
        assert_eq!(decoded.code(), code, "code `{code}` agrees after the hop");
    }
}

#[test]
fn foreign_envelopes_never_decode_as_door_refusals() {
    let bare = ErrorData::invalid_params("gone", None);
    assert!(
        RpcError::from_wire(&bare).is_none(),
        "a bare envelope is foreign, never a door refusal"
    );
    let numeric = ErrorData::invalid_params("gone", Some(rmcp::serde_json::json!({"code": 7})));
    assert!(
        RpcError::from_wire(&numeric).is_none(),
        "a numeric code is foreign, never a door refusal"
    );
    let nulled = ErrorData::invalid_params(
        "gone",
        Some(rmcp::serde_json::json!({"code": "stale_revision", "message": "x", "data": null})),
    );
    let decoded = RpcError::from_wire(&nulled).expect("null data still decodes");
    assert_eq!(decoded.code(), "stale_revision");
    assert_eq!(decoded.data(), None, "null data decodes to no culprit");
}
