//! BRP-shaped loopback shim over the verb table (in-process, no socket).
//! PORT(bevy@4e6bd14 crates/bevy_remote/src/http.rs RemoteHttpPlugin + lib.rs BrpRequest/BrpBatch) -> transport-plus-allowlist shape only, idea only. Loopback stdio/in-process so headless stays clean.
//! Consumer: `McpServer::brp_batch` via `execute_via_shim` (one gesture id per batch, seal at end) plus dry-run readers via `preview_via_shim`. Owner stays editor commands; this owns only `{method, params}` plus batch shapes. Units meters via `OpDto`. Determinism input-ordered, no clock. Errors one `RpcError` shape via default deny.
use super::discover::{
    batch_execute_grouped, preview_batch, BatchExecuteOutput, PreviewItem, PreviewResult,
};
use super::mcp::{McpServer, OpDto};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
/// Loopback address the shim mirrors (never bound, no listener).
pub const BRP_DEFAULT_ADDR: &str = "127.0.0.1";
/// Loopback port the shim mirrors (never bound, no listener).
pub const BRP_DEFAULT_PORT: u16 = 15702;
/// One JSON-RPC request driving the same verb table as the local door.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BrpRequest {
    /// Method name, allowlisted by `map_method` (default deny).
    pub method: String,
    /// Must equal the document revision, else this item errors.
    pub expected_revision: u64,
    /// Retry identity for this item.
    pub idempotency_key: u64,
    /// Edits in order.
    pub ops: Vec<OpDto>,
}
/// Caller: `preview_via_shim` plus `execute_via_shim` (both map each request through here).
pub fn to_preview(req: &BrpRequest) -> PreviewItem {
    PreviewItem {
        method: req.method.clone(),
        expected_revision: req.expected_revision,
        idempotency_key: req.idempotency_key,
        ops: req.ops.clone(),
    }
}
/// Caller: dry-run readers (see `discover` tests); `McpServer::brp_batch` runs the execute lane only.
pub fn preview_via_shim(server: &McpServer, batch: &[BrpRequest]) -> Vec<PreviewResult> {
    let items: Vec<PreviewItem> = batch.iter().map(to_preview).collect();
    preview_batch(server, &items)
}
/// Caller: `McpServer::brp_batch`.
pub fn execute_via_shim(
    server: &McpServer,
    batch: &[BrpRequest],
    gesture_key: u64,
) -> BatchExecuteOutput {
    let items: Vec<PreviewItem> = batch.iter().map(to_preview).collect();
    batch_execute_grouped(server, &items, gesture_key)
}
