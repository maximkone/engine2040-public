//! Engine Insight: always-fresh engine index for agents.
//!
//! Owner: API1, consumer `e2040-mcp` (`api.search`, `api.describe`,
//! `code.callers`, `asset.refs`). Units: tokens are chars/4 estimates.
//! Determinism: same file inputs give the same index and ranking; only
//! rebuild receipts carry wall time. Errors are [`InsightError`].

pub mod eval;
pub mod graph;
pub mod index;
#[cfg(feature = "mcp")]
pub mod mcp_tools;
pub mod search;

pub use index::{
    ApiItem, FileInput, Index, InsightError, ItemKind, RebuildReceipt, RefreshReceipt,
};
pub use search::SearchHit;
