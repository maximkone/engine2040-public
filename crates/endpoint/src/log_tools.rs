//! OB3 log tools: `log.digest` and `log.query` over observe events.
//!
//! Consumer: external MCP clients plus the A9 proof. Data owner for the
//! events is each calling subsystem (it pushes its own [`Event`](engine_observe::Event)s);
//! this module owns only the session ring, the stable digest view and the
//! cursor-paged query view. Storage is [`engine_observe::RingBuffer`],
//! digests are [`engine_observe::digest`]; nothing here reimplements them.
//! Units: event counts in events, sizes in bytes, budgets in bytes.
//! Determinism scope: retained order is insertion order (absolute sequence
//! numbers ascend), and the digest sorts by (count desc, code asc), so any
//! input permutation gives the same digest string. Errors are typed
//! [`LogError`]: a bad cursor names the live window so the client refreshes,
//! never an empty page that looks like an empty log.

use std::borrow::Cow;

use engine_observe::{digest, Event, EventCode, RingBuffer};
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::mcp::{McpServer, McpToolError, ServerState};

/// Tool: stable digest of the observe ring.
pub const TOOL_LOG_DIGEST: &str = "log.digest";
/// Tool: cursor-paged observe events.
pub const TOOL_LOG_QUERY: &str = "log.query";

/// Default `log.digest` byte budget.
pub const LOG_DIGEST_DEFAULT_BUDGET: u32 = 4096;
/// Hard `log.digest` byte budget.
pub const LOG_DIGEST_MAX_BUDGET: u32 = 16_384;
/// Default `log.query` page size.
pub const LOG_QUERY_DEFAULT_LIMIT: u32 = 64;
/// Hard `log.query` page cap.
pub const LOG_QUERY_MAX_ROWS: u32 = 128;
/// Bounded session ring: at most this many events are retained; older ones
/// evict and count as overflow, never as silence.
pub const LOG_RING_CAP: usize = 256;

// ---------------------------------------------------------------------------
// Errors: typed, with the legal values attached. A stale or past-end cursor
// tells the client the live window so it refreshes; it never answers an
// empty page for a non-empty window.
// ---------------------------------------------------------------------------

/// Typed failure for the log tools. Maps one to one onto [`McpToolError`]
/// codes so the wire stays uniform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    /// Cursor is outside the live window `[oldest, newest]`.
    BadCursor {
        cursor: u64,
        oldest: u64,
        newest: u64,
    },
    /// Page size is outside `1..=LOG_QUERY_MAX_ROWS`.
    BadLimit { limit: u32 },
    /// Digest budget is outside `1..=LOG_DIGEST_MAX_BUDGET`.
    BadBudget { budget: u32 },
    /// Raw code is not in the observe registry.
    UnknownCode { code: u32 },
}

impl LogError {
    /// Machine code for the wire error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadCursor { .. } => "bad_cursor",
            Self::BadLimit { .. } => "bad_limit",
            Self::BadBudget { .. } => "bad_budget",
            Self::UnknownCode { .. } => "unknown_code",
        }
    }
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadCursor {
                cursor,
                oldest,
                newest,
            } => write!(
                f,
                "bad_cursor: cursor {cursor} is outside the live window \
                 {oldest}..={newest}; refresh from the newest digest, then query again"
            ),
            Self::BadLimit { limit } => write!(
                f,
                "bad_limit: limit {limit} is outside 1..={LOG_QUERY_MAX_ROWS} \
                 (default {LOG_QUERY_DEFAULT_LIMIT})"
            ),
            Self::BadBudget { budget } => write!(
                f,
                "bad_budget: budget_bytes {budget} is outside 1..={LOG_DIGEST_MAX_BUDGET} \
                 (default {LOG_DIGEST_DEFAULT_BUDGET})"
            ),
            Self::UnknownCode { code } => write!(
                f,
                "unknown_code: code {code} is not a registered observe event code"
            ),
        }
    }
}

impl std::error::Error for LogError {}

impl From<LogError> for McpToolError {
    fn from(error: LogError) -> Self {
        McpToolError::new(error.code(), error.to_string())
    }
}

// ---------------------------------------------------------------------------
// Session store: bounded ring plus the absolute sequence clock. Retained
// events carry seqs `[pushed - len, pushed)`; `pushed` is the next cursor.
// ---------------------------------------------------------------------------

/// Bounded per-session observe log. Sequence numbers are absolute and
/// monotonic, so a cursor stays meaningful across pages; evicted seqs fail
/// loudly via [`LogError::BadCursor`] instead of reading back wrong rows.
#[derive(Debug, Clone)]
pub struct LogStore {
    ring: RingBuffer,
    pushed: u64,
}

impl LogStore {
    /// Empty store retaining at most `cap` events.
    pub fn new(cap: usize) -> Self {
        Self {
            ring: RingBuffer::new(cap),
            pushed: 0,
        }
    }

    /// Events currently retained.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when no events are retained.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Events evicted by the bound. Monotonic; always surfaced, never silent.
    pub fn overflow_count(&self) -> u64 {
        self.ring.overflow_count()
    }

    /// Total events ever pushed; also the cursor one past the newest event.
    pub fn next_seq(&self) -> u64 {
        self.pushed
    }

    /// Sequence of the oldest retained event (`pushed` when empty).
    pub fn oldest_seq(&self) -> u64 {
        self.pushed.saturating_sub(self.ring.len() as u64)
    }

    /// Push one event, evicting the oldest when full.
    pub fn push(&mut self, event: Event) {
        self.ring.push(event);
        self.pushed = self.pushed.saturating_add(1);
    }

    /// Oldest-first snapshot of retained events.
    pub fn snapshot(&self) -> Vec<Event> {
        self.ring.iter().cloned().collect()
    }
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new(LOG_RING_CAP)
    }
}

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the eleven-tool discovery budget is a gate.
// ---------------------------------------------------------------------------

/// Digest request: optional byte budget for the stable ring digest.
/// Units are bytes of budget. Errors are typed `bad_budget` outside
/// `1..=16384`. Budget defaults to 4096 bytes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LogDigestArgs {
    pub budget_bytes: Option<u32>,
}

/// Digest answer: stable per-code counts bounded by the byte budget.
/// Units are events, bytes and budget bytes. No errors once read; a tight
/// budget sets `truncated`. Budget is at most 16384 bytes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LogDigestOutput {
    pub digest: String,
    pub events: u64,
    pub bytes: u64,
    pub budget: u64,
    pub overflow: u64,
    pub truncated: bool,
}

/// Query request: absolute cursor, page size and optional code filter.
/// Units are sequence numbers and event counts. Errors are typed
/// `bad_cursor`, `bad_limit` and `unknown_code`. Budget is 128 rows a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LogQueryArgs {
    pub cursor: Option<u64>,
    pub limit: Option<u32>,
    pub code: Option<u32>,
}

/// One retained event: absolute sequence, code, tick and message.
/// Units are sequence numbers, ticks and message chars. No errors: rows are
/// read-only snapshots. Budget rides on the page limit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LogEventDto {
    pub seq: u64,
    pub code: u32,
    pub tick: u64,
    pub message: String,
}

/// Query answer: one cursor page in stable sequence order.
/// Units are sequence numbers, counts and bytes. No errors once paged;
/// stale cursors fail at the request. Budget is 128 rows a page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LogQueryOutput {
    pub cursor: u64,
    pub next_cursor: u64,
    pub total: u64,
    pub returned: u64,
    pub truncated: bool,
    pub overflow: u64,
    pub events: Vec<LogEventDto>,
}

impl ServerState {
    /// Record one observe event in the session ring.
    pub fn push_log(&mut self, event: Event) {
        self.logs.push(event);
    }

    /// Stable digest of the retained ring, bounded by `budget_bytes`.
    pub fn log_digest(&self, args: &LogDigestArgs) -> Result<LogDigestOutput, McpToolError> {
        let budget = args.budget_bytes.unwrap_or(LOG_DIGEST_DEFAULT_BUDGET);
        if budget == 0 || budget > LOG_DIGEST_MAX_BUDGET {
            return Err(LogError::BadBudget { budget }.into());
        }
        let events = self.logs.snapshot();
        let kept = digest(&events, budget as usize);
        let full = digest(&events, usize::MAX);
        Ok(LogDigestOutput {
            bytes: kept.len() as u64,
            truncated: kept != full,
            digest: kept,
            events: events.len() as u64,
            budget: u64::from(budget),
            overflow: self.logs.overflow_count(),
        })
    }

    /// One cursor page over the retained ring in stable (seq-ascending)
    /// order. `cursor` is an absolute sequence; `next_cursor` continues the
    /// walk. Stale or past-end cursors fail with the live window attached.
    pub fn log_query(&self, args: &LogQueryArgs) -> Result<LogQueryOutput, McpToolError> {
        let limit = args.limit.unwrap_or(LOG_QUERY_DEFAULT_LIMIT);
        if limit == 0 || limit > LOG_QUERY_MAX_ROWS {
            return Err(LogError::BadLimit { limit }.into());
        }
        let filter = match args.code {
            None => None,
            Some(raw) => {
                Some(EventCode::from_raw(raw).map_err(|_| LogError::UnknownCode { code: raw })?)
            }
        };
        let oldest = self.logs.oldest_seq();
        let newest = self.logs.next_seq();
        let cursor = args.cursor.unwrap_or(oldest);
        if cursor < oldest || cursor > newest {
            return Err(LogError::BadCursor {
                cursor,
                oldest,
                newest,
            }
            .into());
        }
        // Retained events with absolute seqs, oldest first; the optional
        // code filter applies before paging so pages stay stable.
        let mut matching: Vec<(u64, &Event)> = Vec::new();
        for (index, event) in self.logs.ring_iter().enumerate() {
            if filter.is_some_and(|code| event.code != code) {
                continue;
            }
            matching.push((oldest + index as u64, event));
        }
        let total = matching.len() as u64;
        let start = matching
            .iter()
            .position(|(seq, _)| *seq >= cursor)
            .unwrap_or(matching.len());
        let end = start.saturating_add(limit as usize).min(matching.len());
        let mut rows = Vec::with_capacity(end - start);
        for (seq, event) in &matching[start..end] {
            rows.push(LogEventDto {
                seq: *seq,
                code: event.code.raw(),
                tick: event.tick,
                message: event.message.clone(),
            });
        }
        let next_cursor = rows.last().map_or(cursor, |row| row.seq + 1);
        Ok(LogQueryOutput {
            cursor,
            next_cursor,
            total,
            returned: rows.len() as u64,
            truncated: end < matching.len(),
            overflow: self.logs.overflow_count(),
            events: rows,
        })
    }
}

impl LogStore {
    /// Oldest-first iterator over retained events (delegate of the ring).
    fn ring_iter(&self) -> impl DoubleEndedIterator<Item = &Event> + '_ {
        self.ring.iter()
    }
}

// ---------------------------------------------------------------------------
// rmcp tools: one SyncTool per contract tool, sync so tests need no executor.
// ---------------------------------------------------------------------------

/// `log.digest` tool: stable ring digest, read-only.
pub struct LogDigestTool;

impl ToolBase for LogDigestTool {
    type Parameter = LogDigestArgs;
    type Output = LogDigestOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_LOG_DIGEST.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Stable digest of the observe ring: per-code counts, bounded by budget. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for LogDigestTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_LOG_DIGEST, |state| state.log_digest(&param))
    }
}

/// `log.query` tool: cursor-paged events, read-only.
pub struct LogQueryTool;

impl ToolBase for LogQueryTool {
    type Parameter = LogQueryArgs;
    type Output = LogQueryOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_LOG_QUERY.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Cursor-paged observe events in stable order with overflow counters. Read-only.".into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for LogQueryTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_LOG_QUERY, |state| state.log_query(&param))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn push(server: &McpServer, code: EventCode, tick: u64, message: &str) {
        server
            .lock()
            .expect("test lock holds")
            .push_log(Event::new(code, tick, message));
    }

    fn seed_mixed(server: &McpServer) {
        push(server, EventCode::PROVIDER_TIMEOUT, 3, "slow");
        push(server, EventCode::SIM_TICK, 1, "tick");
        push(server, EventCode::PROVIDER_TIMEOUT, 7, "slow again");
        push(server, EventCode::HEAL_APPLIED, 5, "replanned");
    }

    fn digest(server: &McpServer, budget: Option<u32>) -> LogDigestOutput {
        LogDigestTool::invoke(
            server,
            LogDigestArgs {
                budget_bytes: budget,
            },
        )
        .expect("digest reads")
    }

    fn query(server: &McpServer, cursor: Option<u64>, limit: Option<u32>) -> LogQueryOutput {
        LogQueryTool::invoke(
            server,
            LogQueryArgs {
                cursor,
                limit,
                code: None,
            },
        )
        .expect("query reads")
    }

    #[test]
    fn digest_empty_ring_is_empty_and_bounded() {
        let server = server();
        let out = digest(&server, None);
        assert_eq!(out.events, 0);
        assert_eq!(out.overflow, 0);
        assert_eq!(out.digest, String::new());
        assert_eq!(out.bytes, 0);
        assert!(!out.truncated);
        assert_eq!(out.budget, u64::from(LOG_DIGEST_DEFAULT_BUDGET));
    }

    #[test]
    fn digest_stable_and_bounded() {
        let server = server();
        seed_mixed(&server);
        let first = digest(&server, None);
        let second = digest(&server, None);
        assert_eq!(first.digest, second.digest, "same ring twice, same digest");
        assert_eq!(first.events, 4);
        assert_eq!(first.overflow, 0);
        assert_eq!(first.bytes, first.digest.len() as u64);
        assert!(!first.truncated);
        assert!(first.digest.contains("provider.timeout x2 first=3 last=7"));
        // Every budget bounds the bytes; a tight budget truncates loudly.
        for budget in [1u32, 10, 32, 64] {
            let out = digest(&server, Some(budget));
            assert!(
                out.bytes <= u64::from(budget),
                "digest {} bytes over budget {budget}",
                out.bytes
            );
            assert_eq!(out.budget, u64::from(budget));
        }
        let tight = digest(&server, Some(10));
        assert!(tight.truncated, "10 bytes cannot hold four rows");
        // A fresh ring with the same events in another order digests the same.
        let other = McpServer::new(7, Capability::unrestricted(7));
        push(&other, EventCode::HEAL_APPLIED, 5, "replanned");
        push(&other, EventCode::PROVIDER_TIMEOUT, 7, "slow again");
        push(&other, EventCode::SIM_TICK, 1, "tick");
        push(&other, EventCode::PROVIDER_TIMEOUT, 3, "slow");
        assert_eq!(digest(&other, None).digest, first.digest);
    }

    #[test]
    fn digest_rejects_bad_budgets() {
        let server = server();
        for budget in [Some(0), Some(LOG_DIGEST_MAX_BUDGET + 1)] {
            let refused = LogDigestTool::invoke(
                &server,
                LogDigestArgs {
                    budget_bytes: budget,
                },
            )
            .expect_err("bad budget must fail");
            assert_eq!(refused.code, "bad_budget");
            assert!(refused.message.contains("16384"));
        }
    }

    #[test]
    fn query_pages_walk_with_next_cursor_in_stable_order() {
        let server = server();
        for tick in 0..10u64 {
            push(&server, EventCode::SIM_TICK, tick, "tick");
        }
        // Walk the whole log two pages at a time through next_cursor.
        let mut cursor = None;
        let mut seqs = Vec::new();
        let mut pages = 0;
        loop {
            let page = query(&server, cursor, Some(4));
            assert_eq!(page.overflow, 0);
            if pages == 0 {
                assert_eq!(page.cursor, 0);
                assert_eq!(page.total, 10);
            }
            for row in &page.events {
                assert_eq!(row.code, EventCode::SIM_TICK.raw());
                seqs.push(row.seq);
            }
            pages += 1;
            if !page.truncated {
                assert_eq!(
                    seqs.len() as u64,
                    page.total,
                    "last page completes the walk"
                );
                break;
            }
            cursor = Some(page.next_cursor);
        }
        assert_eq!(seqs, (0..10u64).collect::<Vec<_>>(), "stable seq order");
        assert_eq!(pages, 3, "10 rows in pages of 4");
        // The cursor one past the end reads an empty, non-truncated page.
        let tail = query(&server, Some(10), Some(4));
        assert_eq!(tail.returned, 0);
        assert!(!tail.truncated);
        assert_eq!(tail.next_cursor, 10);
    }

    #[test]
    fn query_overflow_visible_and_stale_cursor_refreshes() {
        let server = server();
        for tick in 0..(LOG_RING_CAP as u64 + 4) {
            push(&server, EventCode::SIM_TICK, tick, "tick");
        }
        let full = digest(&server, None);
        assert_eq!(full.events, LOG_RING_CAP as u64);
        assert_eq!(full.overflow, 4);
        assert!(full.digest.contains("sim.tick"));
        // The four evicted seqs fail loudly with the live window attached.
        for stale in [0u64, 3] {
            let refused = LogQueryTool::invoke(
                &server,
                LogQueryArgs {
                    cursor: Some(stale),
                    limit: None,
                    code: None,
                },
            )
            .expect_err("evicted cursor must fail");
            assert_eq!(refused.code, "bad_cursor");
            assert!(
                refused.message.contains('4'),
                "names the oldest live seq: {}",
                refused.message
            );
        }
        // The live window still pages from the oldest retained seq.
        let page = query(&server, Some(4), Some(LOG_QUERY_MAX_ROWS));
        assert_eq!(page.overflow, 4);
        assert_eq!(page.total, LOG_RING_CAP as u64);
        assert_eq!(page.events[0].seq, 4);
        assert_eq!(page.events[0].tick, 4);
        // A cursor past the end fails too, never an empty page.
        let past_end = LOG_RING_CAP as u64 + 5;
        let refused = LogQueryTool::invoke(
            &server,
            LogQueryArgs {
                cursor: Some(past_end),
                limit: None,
                code: None,
            },
        )
        .expect_err("past-end cursor must fail");
        assert_eq!(refused.code, "bad_cursor");
        assert!(refused.message.contains(&past_end.to_string()));
    }

    #[test]
    fn query_bad_limits_and_unknown_codes_fail() {
        let server = server();
        seed_mixed(&server);
        for limit in [Some(0), Some(LOG_QUERY_MAX_ROWS + 1)] {
            let refused = LogQueryTool::invoke(
                &server,
                LogQueryArgs {
                    cursor: None,
                    limit,
                    code: None,
                },
            )
            .expect_err("bad limit must fail");
            assert_eq!(refused.code, "bad_limit");
            assert!(refused.message.contains("128"));
        }
        let refused = LogQueryTool::invoke(
            &server,
            LogQueryArgs {
                cursor: None,
                limit: None,
                code: Some(9999),
            },
        )
        .expect_err("unknown code must fail");
        assert_eq!(refused.code, "unknown_code");
    }

    #[test]
    fn query_code_filter_pages_matching_events_only() {
        let server = server();
        seed_mixed(&server);
        let page = LogQueryTool::invoke(
            &server,
            LogQueryArgs {
                cursor: None,
                limit: None,
                code: Some(EventCode::PROVIDER_TIMEOUT.raw()),
            },
        )
        .expect("filtered query reads");
        assert_eq!(page.total, 2);
        assert_eq!(page.returned, 2);
        assert!(!page.truncated);
        assert!(page
            .events
            .iter()
            .all(|row| row.code == EventCode::PROVIDER_TIMEOUT.raw()));
        assert_eq!(page.events[0].seq, 0);
        assert_eq!(page.events[1].seq, 2);
        assert_eq!(page.next_cursor, 3);
        // Filtering to an absent code reads an empty, honest page.
        let empty = LogQueryTool::invoke(
            &server,
            LogQueryArgs {
                cursor: None,
                limit: None,
                code: Some(EventCode::NET_CLIENT_DESYNC.raw()),
            },
        )
        .expect("absent code reads empty");
        assert_eq!(empty.total, 0);
        assert_eq!(empty.returned, 0);
        assert!(!empty.truncated);
    }

    #[test]
    fn external_client_pages_log_and_checks_stable_digest() {
        // A9 shape: one client pages the whole log, another (or the same one
        // later) takes the digest; both agree, and a re-digest is identical.
        let server = server();
        seed_mixed(&server);
        let before = digest(&server, None);
        // Page the log like an outside client: default cursor, small pages.
        let mut cursor = None;
        let mut rows = Vec::new();
        loop {
            let page = query(&server, cursor, Some(2));
            rows.extend(page.events);
            if !page.truncated {
                break;
            }
            cursor = Some(page.next_cursor);
        }
        assert_eq!(rows.len(), 4);
        let seqs: Vec<u64> = rows.iter().map(|row| row.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3]);
        assert_eq!(rows[0].message, "slow");
        assert_eq!(rows[3].message, "replanned");
        let after = digest(&server, None);
        assert_eq!(
            before.digest, after.digest,
            "paging never disturbs the digest"
        );
        assert_eq!(after.events, rows.len() as u64);
    }

    #[test]
    fn log_error_codes_and_messages_name_the_fix() {
        let over = LogError::BadCursor {
            cursor: 99,
            oldest: 4,
            newest: 10,
        };
        assert_eq!(over.code(), "bad_cursor");
        assert!(over.to_string().contains("4..=10"));
        assert_eq!(LogError::BadLimit { limit: 0 }.code(), "bad_limit");
        assert_eq!(LogError::BadBudget { budget: 0 }.code(), "bad_budget");
        assert_eq!(LogError::UnknownCode { code: 7 }.code(), "unknown_code");
        let wire = McpToolError::from(LogError::UnknownCode { code: 9999 });
        assert_eq!(wire.code, "unknown_code");
        let _: &dyn std::error::Error = &over;
    }
}
