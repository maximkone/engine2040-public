// Attribution: list-and-measure pattern adapted from
// modelcontextprotocol/rust-sdk (Apache-2.0, verified): enumerate the
// `ToolRouter` list, measure each entry, gate the sum. Fresh exhaustion
// vectors against our own `ToolBudgetLedger` API; no donor code copied.

//! MCP tool-budget exhaustion vectors: the ledger refuses bloat, the floor holds.
//!
//! Consumer: the MCP door author sizing the discovery surface (names plus
//! input/output schemas stay under budget). Data owner is the endpoint
//! `McpServer` ledger; this file owns only the vectors. Units are chars.
//! Determinism: pure ledger math over fixed constants, no clock or IO.
//! Errors are typed tool codes, asserted by name.

#[cfg(feature = "mcp")]
use engine_editor::capability::Capability;
#[cfg(feature = "mcp")]
use engine_endpoint::mcp::{
    check_tool_budget, McpServer, TOOL_BUDGET_PROBE_BLOAT_CHARS, TOOL_LIST_BUDGET_CHARS,
    TOOL_LIST_MIN_HEADROOM_CHARS,
};

/// Empty-doc session: the ledger measures the default advertised list.
#[cfg(feature = "mcp")]
fn test_server() -> McpServer {
    McpServer::new(7, Capability::unrestricted(7))
}

#[cfg(feature = "mcp")]
#[test]
fn ledger_rows_sum_to_total() {
    let server = test_server();
    let ledger = server.tool_budget_ledger();
    assert!(
        !ledger.rows.is_empty(),
        "ledger names every advertised tool"
    );
    for row in &ledger.rows {
        assert!(!row.name.is_empty(), "every row names its tool");
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

#[cfg(feature = "mcp")]
#[test]
fn live_list_holds_headroom_floor() {
    assert_eq!(
        TOOL_LIST_MIN_HEADROOM_CHARS, 2_048,
        "floor pins at 2048 chars"
    );
    let server = test_server();
    let ledger = server
        .tool_budget_gate_with_bloat(0)
        .expect("live list passes the ledger gate unbloated");
    assert!(
        ledger.headroom_chars >= TOOL_LIST_MIN_HEADROOM_CHARS,
        "headroom {} chars under minimum {TOOL_LIST_MIN_HEADROOM_CHARS}",
        ledger.headroom_chars
    );
}

#[cfg(feature = "mcp")]
#[test]
fn seeded_500_probe_refuses_bloated() {
    assert_eq!(
        TOOL_BUDGET_PROBE_BLOAT_CHARS, 500,
        "probe pins at +500 chars"
    );
    // One char under the ceiling: the +500 probe overflows by one.
    let seeded = TOOL_LIST_BUDGET_CHARS - (TOOL_BUDGET_PROBE_BLOAT_CHARS - 1);
    let refused = check_tool_budget(seeded, TOOL_BUDGET_PROBE_BLOAT_CHARS)
        .expect_err("seeded +500 bloat must fail the gate");
    assert_eq!(refused.code, "tool_list_bloated");
    assert!(
        refused
            .message
            .contains(TOOL_LIST_BUDGET_CHARS.to_string().as_str()),
        "refusal names the budget: {}",
        refused.message
    );
}

#[cfg(feature = "mcp")]
#[test]
fn headroom_floor_exact_passes_one_under_refuses() {
    // Exactly the minimum headroom passes and reports it.
    let floor = TOOL_LIST_BUDGET_CHARS - TOOL_LIST_MIN_HEADROOM_CHARS;
    let headroom = check_tool_budget(floor, 0).expect("exact floor passes");
    assert_eq!(headroom, TOOL_LIST_MIN_HEADROOM_CHARS);
    // One char under the floor refuses with the raise rule attached.
    let tight = TOOL_LIST_BUDGET_CHARS - (TOOL_LIST_MIN_HEADROOM_CHARS - 1);
    let refused = check_tool_budget(tight, 0).expect_err("sub-floor headroom must fail");
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
}

#[cfg(feature = "mcp")]
#[test]
fn live_gate_with_overflow_bloat_refuses() {
    let server = test_server();
    let total = server.tool_list_budget_chars();
    assert!(
        total < TOOL_LIST_BUDGET_CHARS,
        "live list fits before the probe: {total} of {TOOL_LIST_BUDGET_CHARS}"
    );
    // Just enough seeded bloat to overflow the ceiling by one char.
    let extra = TOOL_LIST_BUDGET_CHARS - total + 1;
    let refused = server
        .tool_budget_gate_with_bloat(extra)
        .expect_err("overflow bloat must fail the live gate");
    assert_eq!(refused.code, "tool_list_bloated");
}

#[cfg(not(feature = "mcp"))]
macro_rules! skipped_tests {
    ($($name:ident),*) => { $(#[test] fn $name() { println!("SKIPPED: mcp feature off"); })* };
}
#[cfg(not(feature = "mcp"))]
skipped_tests!(
    ledger_rows_sum_to_total,
    live_list_holds_headroom_floor,
    seeded_500_probe_refuses_bloated,
    headroom_floor_exact_passes_one_under_refuses,
    live_gate_with_overflow_bloat_refuses
);
