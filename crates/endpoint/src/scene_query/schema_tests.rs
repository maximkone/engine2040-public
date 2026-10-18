//! SPR3 tests: `scene.query` teaches itself through its schema, its refusals
//! and `api.describe`, all from the one `KIND_CARDS` table.

use super::*;
use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
use engine_editor::capability::Capability;

fn server() -> McpServer {
    McpServer::new(7, Capability::unrestricted(7))
}

fn schema() -> rmcp::serde_json::Value {
    rmcp::serde_json::to_value(rmcp::schemars::schema_for!(SceneQueryArgs)).expect("schema")
}

/// Every field has a doc, `kind` is an enum of the nine words, and the kind
/// doc lists the fields each kind needs, straight from the card table.
#[test]
fn query_schema_docs_cover_every_field_and_kind() {
    let value = schema();
    let props = value["properties"].as_object().expect("properties");
    assert_eq!(props.len(), 19);
    for (name, prop) in props {
        let doc = prop["description"].as_str().unwrap_or("");
        assert!(!doc.is_empty(), "field `{name}` has no doc");
    }
    let words: Vec<&str> = props["kind"]["enum"]
        .as_array()
        .expect("kind is an enum")
        .iter()
        .filter_map(|word| word.as_str())
        .collect();
    let want: Vec<&str> = KIND_CARDS.iter().map(|card| card.kind.name()).collect();
    assert_eq!(words, want);
    let kind_doc = props["kind"]["description"].as_str().expect("kind doc");
    for card in &KIND_CARDS {
        let line = format!("{} {}", card.kind.name(), card.needs);
        assert!(kind_doc.contains(&line), "kind doc misses `{line}`");
    }
}

/// A request without a field its kind needs refuses `missing_arg`, naming
/// the kind, the field and a working call; kinds that need nothing answer.
#[test]
fn query_schema_docs_missing_arg_names_a_working_call() {
    let server = server();
    for card in &KIND_CARDS {
        let args = SceneQueryArgs {
            kind: card.kind.name().to_owned(),
            ..Default::default()
        };
        let answer = SceneQueryTool::invoke(&server, args);
        let field = card.needs.split(", ").next().expect("needs");
        match card.needs {
            "-" => assert_ne!(
                answer.err().map(|refused| refused.code),
                Some("missing_arg"),
                "{} needs nothing",
                card.kind.name()
            ),
            _ => {
                let refused = answer.expect_err("missing field refuses");
                assert_eq!(refused.code, "missing_arg", "{}", card.kind.name());
                assert!(
                    refused.message.contains(card.kind.name()),
                    "{}",
                    refused.message
                );
                assert!(refused.message.contains(field), "{}", refused.message);
                assert!(
                    refused.message.contains(card.example),
                    "{}",
                    refused.message
                );
            }
        }
    }
    let visible = SceneQueryTool::invoke(
        &server,
        SceneQueryArgs {
            kind: "visible".to_owned(),
            ..Default::default()
        },
    )
    .expect_err("visible without eye_m refuses");
    assert!(visible.message.contains("`eye_m`"), "{}", visible.message);
    assert!(visible.message.contains(r#""eye_m":[0.0,1.6,0.0]"#));
}

/// `api.describe query:<kind>` answers with the kind's needs and its
/// example, and every example parses as a request of that kind.
#[test]
fn query_schema_docs_api_describe_gives_one_example_per_kind() {
    let server = server();
    for card in &KIND_CARDS {
        let path = format!("query:{}", card.kind.name());
        let described = ApiDescribeTool::invoke(&server, ApiDescribeArgs { path: path.clone() })
            .unwrap_or_else(|error| panic!("{path} describes: {error}"));
        assert!(
            described.docs.contains(card.example),
            "{path}: {}",
            described.docs
        );
        assert!(
            described.docs.contains(card.needs),
            "{path}: {}",
            described.docs
        );
        let parsed: SceneQueryArgs =
            rmcp::serde_json::from_str(card.example).expect("example is a valid request");
        assert_eq!(parsed.kind, card.kind.name());
    }
}
