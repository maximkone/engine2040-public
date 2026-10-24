//! MCP31 unit tests: the ask's typed shape, its licence rule and its answers.

use super::*;
use crate::mcp::{advice_for_code, Advice};
use rmcp::serde_json::json;

fn answers(value: Value) -> InputResponses {
    rmcp::serde_json::from_value(value).expect("answers decode")
}

#[test]
fn input_required_a_licence_ask_is_one_typed_request_with_a_state() {
    let error = licence_ask("VAST-AI/TripoSG");
    assert_eq!(error.code, CODE);
    assert_eq!(CODE, "input_required", "the code the door publishes");
    assert_eq!(
        advice_for_code(CODE),
        Advice::FixArgument,
        "the ask needs an argument, so it carries that advice"
    );
    let data = error.data().expect("the ask carries its wire result");
    assert_eq!(data["resultType"], "input_required");
    assert_eq!(
        data["requestState"], REQUEST_STATE,
        "the retry echoes the request state"
    );
    let requests = data["inputRequests"]
        .as_object()
        .expect("the ask carries one request map");
    assert_eq!(requests.len(), 1, "one request, never a list of guesses");
    assert!(
        requests.contains_key(LICENCE_REQUEST_ID),
        "the request is keyed by its id: {requests:?}"
    );
    assert!(
        error.message.contains("SPDX") && error.message.contains("inputResponses"),
        "the message names the fact and the retry: {}",
        error.message
    );
    let response = into_response(&error).expect("the ask turns into its typed result");
    let CallToolResponse::InputRequired(result) = response else {
        panic!("an ask is an InputRequired response");
    };
    assert!(result.result_type.is_input_required());
    assert_eq!(
        result.input_requests.as_ref().map(InputRequests::len),
        Some(1)
    );
    assert_eq!(result.request_state.as_deref(), Some(REQUEST_STATE));
}

#[test]
fn input_required_into_response_leaves_every_other_refusal_alone() {
    for code in [
        "bad_args",
        "license_refused",
        "unknown_tool",
        "stale_revision",
    ] {
        let error = McpToolError::new(code, "refused");
        assert!(
            into_response(&error).is_none(),
            "`{code}` stays a refusal, not an ask"
        );
    }
    // The code alone is not enough: an ask without its wire result cannot
    // become a typed result, so the door must keep the refusal.
    assert!(into_response(&McpToolError::new(CODE, "ask")).is_none());
}

#[test]
fn input_required_licence_is_unknown_reads_an_empty_or_shapeless_answer_only() {
    for unknown in [
        "",
        "   ",
        "unknown",
        "Unknown",
        "n/a",
        "none",
        "see the card!",
    ] {
        assert!(licence_is_unknown(unknown), "`{unknown}` names no licence");
    }
    for decided in [
        "MIT",
        "Apache-2.0",
        "CC-BY-NC-4.0",
        "MIT OR Apache-2.0",
        "MIT AND CC0-1.0",
    ] {
        assert!(
            !licence_is_unknown(decided),
            "`{decided}` is decided, so it never asks"
        );
    }
}

#[test]
fn input_required_answered_licence_reads_only_an_accepted_answer() {
    let accept = answers(json!({
        "licence": {"action": "accept", "content": {"licence": "MIT"}}
    }));
    assert_eq!(answered_licence(&accept).as_deref(), Some("MIT"));
    let decline = answers(json!({"licence": {"action": "decline"}}));
    assert_eq!(answered_licence(&decline), None);
    let empty = answers(json!({
        "licence": {"action": "accept", "content": {"licence": "  "}}
    }));
    assert_eq!(answered_licence(&empty), None);
    let other = answers(json!({
        "something_else": {"action": "accept", "content": {"licence": "MIT"}}
    }));
    assert_eq!(answered_licence(&other), None);
}

#[test]
fn input_required_the_same_ask_is_the_same_bytes_twice() {
    let first = licence_ask("VAST-AI/TripoSG");
    let second = licence_ask("VAST-AI/TripoSG");
    assert_eq!(
        first.wire_payload().to_string(),
        second.wire_payload().to_string(),
        "the same ask serializes byte for byte"
    );
}

#[test]
fn input_required_the_ask_result_is_refused_when_it_would_be_empty() {
    assert!(ask_result(&[]).is_none(), "an ask carries a request");
}
