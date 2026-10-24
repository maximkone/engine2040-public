//! AD26 tests: the advice table over every produced code, and the multi-field
//! reply for a bad argument list. Named `refusal_advice_*` so the row's proof
//! filter runs them.

use super::*;
use crate::mcp::mcp_refusal::{classify, next_for};
use crate::mcp::{build_batch, refusal_outcome, tool_names, Kind, McpToolError, OpDto, PoseDto};
use engine_scene::ids::ObjectId;
use rmcp::serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A uuid the door accepts as an object id.
const VALID_ID: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

fn pose() -> PoseDto {
    PoseDto {
        pos: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

fn crate_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Code literals passed to the two refusal constructors anywhere in the crate:
/// the set the row says must have a table row.
fn produced_codes() -> BTreeSet<String> {
    let mut codes = BTreeSet::new();
    let mut stack = vec![crate_src()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for marker in ["McpToolError::new(", "McpToolError::wrap("] {
                for chunk in text.split(marker).skip(1) {
                    let Some(rest) = chunk.trim_start().strip_prefix('"') else {
                        continue;
                    };
                    let Some(end) = rest.find('"') else {
                        continue;
                    };
                    let code = &rest[..end];
                    if !code.is_empty()
                        && code.chars().all(|lane| {
                            lane.is_ascii_lowercase() || lane.is_ascii_digit() || lane == '_'
                        })
                    {
                        codes.insert(code.to_owned());
                    }
                }
            }
        }
    }
    codes
}

#[test]
fn refusal_advice_every_produced_code_has_a_table_row() {
    let codes = produced_codes();
    assert!(
        codes.len() >= 150,
        "the scan found only {} codes, so it is not reading the tree",
        codes.len()
    );
    let missing: Vec<&str> = codes
        .iter()
        .map(String::as_str)
        .filter(|code| advice_for_code(code) == Advice::Unknown)
        .collect();
    assert!(
        missing.is_empty(),
        "these codes carry no advice, so a model cannot branch on them: {missing:?}"
    );
}

#[test]
fn refusal_advice_closed_list_is_short_and_wire_names_are_unique() {
    assert!(
        Advice::ALL.len() <= 12,
        "the advice list stays a short fixed set"
    );
    let names: BTreeSet<&str> = Advice::ALL.iter().map(|advice| advice.as_str()).collect();
    assert_eq!(
        names.len(),
        Advice::ALL.len(),
        "every advice has its own word"
    );
    for advice in Advice::ALL {
        let name = advice.as_str();
        assert!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|lane| lane.is_ascii_lowercase() || lane == '_'),
            "`{name}` is a snake_case wire word"
        );
    }
}

#[test]
fn refusal_advice_retry_flag_and_advice_agree() {
    for code in produced_codes() {
        let (kind, retryable) = classify(&code);
        let advice = advice_for_code(&code);
        if retryable {
            assert_ne!(
                advice,
                Advice::Reread,
                "`{code}` is retryable, so it never only says re-read"
            );
        } else {
            assert_ne!(
                advice,
                Advice::RetryLater,
                "`{code}` is not retryable, so it never says retry"
            );
        }
        assert_ne!(kind, Kind::Unknown, "`{code}` keeps its kind");
    }
}

#[test]
fn refusal_advice_pinned_codes_map_to_their_advice() {
    let pinned = [
        ("stale_revision", Advice::Repreview),
        ("expired_approval", Advice::Repreview),
        ("forged_approval", Advice::Repreview),
        ("duplicate_key", Advice::Repreview),
        ("undone_key", Advice::Reapply),
        ("unknown_tool", Advice::SearchVerb),
        ("bad_args", Advice::FixArgument),
        ("input_required", Advice::FixArgument),
        ("over_budget", Advice::NarrowCall),
        ("op_not_allowed", Advice::NarrowCall),
        ("lock_poisoned", Advice::Escalate),
        ("feature_off", Advice::Escalate),
        ("apply_in_flight", Advice::RetryLater),
        ("io_failed", Advice::RetryLater),
    ];
    for (code, advice) in pinned {
        assert_eq!(advice_for_code(code), advice, "`{code}` advice");
    }
    let produced = produced_codes();
    for (code, _) in OVERRIDES {
        assert!(
            produced.contains(*code),
            "override `{code}` names no code the crate produces"
        );
    }
}

#[test]
fn refusal_advice_payload_carries_the_advice_code() {
    let error = McpToolError::new("stale_revision", "document moved under the batch");
    let payload = error.wire_payload();
    assert_eq!(payload["advice"], "repreview");
    assert_eq!(payload["code"], "stale_revision");
    assert_eq!(payload["kind"], "state");
    assert_eq!(payload["retryable"], true);
    let next = payload["next"]["verb"]
        .as_str()
        .expect("stale revision names one next call");
    assert!(tool_names().contains(&next), "next call `{next}` is live");
    // The advice word is one the closed list publishes.
    let published: BTreeSet<&str> = Advice::ALL.iter().map(|advice| advice.as_str()).collect();
    assert!(published.contains(payload["advice"].as_str().unwrap()));
}

#[test]
fn refusal_advice_stale_revision_result_flags_is_error_and_retry() {
    let response = refusal_outcome(McpToolError::new(
        "stale_revision",
        "document is at revision 7, batch expects 6",
    ))
    .expect("a door refusal is a tool result");
    let rmcp::model::CallToolResponse::Complete(result) = response else {
        panic!("a refusal completes the call");
    };
    assert_eq!(result.is_error, Some(true), "a refusal flags isError");
    let payload = result
        .structured_content
        .as_ref()
        .expect("the refusal carries structured content");
    assert_eq!(payload["code"], "stale_revision");
    assert_eq!(payload["advice"], "repreview");
    assert_eq!(payload["retryable"], true);
    assert_eq!(payload["next"]["verb"], "scene.read");
}

#[test]
fn refusal_advice_unknown_tool_stays_an_rpc_error() {
    let envelope = refusal_outcome(McpToolError::new("unknown_tool", "no tool named `nope`"))
        .expect_err("an unknown name is a protocol error");
    assert_eq!(envelope.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

#[test]
fn refusal_advice_three_bad_fields_return_three_pointers() {
    let mut bad_pose = pose();
    bad_pose.pos[0] = f32::NAN;
    let ops = [
        OpDto::Insert {
            id: "not-a-uuid".to_owned(),
            parent: ObjectId::NIL.to_string(),
            local: pose(),
        },
        OpDto::Remove {
            id: "also-not-a-uuid".to_owned(),
        },
        OpDto::SetLocal {
            id: VALID_ID.to_owned(),
            local: bad_pose,
        },
    ];
    let error = build_batch(0, 7, &ops, None).expect_err("three bad fields refuse the batch");
    assert_eq!(error.code, "bad_args");
    assert_eq!(error.advice(), Advice::FixArgument);
    let data = error.data().expect("the refusal carries the field list");
    let fields = data["fields"].as_array().expect("fields is a list");
    assert_eq!(fields.len(), 3, "one row per bad field: {fields:?}");
    let pointers: Vec<&str> = fields
        .iter()
        .map(|row| row["pointer"].as_str().expect("a pointer per field"))
        .collect();
    assert_eq!(
        pointers,
        vec!["/ops/0/id", "/ops/1/id", "/ops/2/local/pos"],
        "each pointer names the field that failed"
    );
    assert_eq!(fields[0]["reason"], "not_an_id");
    assert_eq!(fields[2]["reason"], "not_finite");
    for row in fields {
        let hint = row["hint"].as_str().expect("a hint per field");
        assert!(!hint.is_empty(), "every field says how to fix it");
    }
}

#[test]
fn refusal_advice_one_bad_field_keeps_its_precise_code() {
    let ops = [OpDto::Remove {
        id: "not-a-uuid".to_owned(),
    }];
    let error = build_batch(0, 7, &ops, None).expect_err("a bad id refuses the batch");
    assert_eq!(error.code, "bad_id", "one culprit keeps its precise code");
}

#[test]
fn refusal_advice_secret_shaped_hint_is_redacted() {
    // Not a credential: a fake prefix the redactor must catch, never echoed.
    let echoed = "sk-example-not-a-credential";
    let ops = [
        OpDto::SetMaterial {
            id: echoed.to_owned(),
            material: Some("ok".to_owned()),
        },
        OpDto::Remove {
            id: "not-a-uuid".to_owned(),
        },
    ];
    let error = build_batch(0, 7, &ops, None).expect_err("two bad fields refuse the batch");
    let wire = error.wire_payload().to_string();
    assert!(
        !wire.contains(echoed),
        "a secret-shaped value never echoes back: {wire}"
    );
    assert!(
        wire.contains("<redacted>"),
        "the hint says the value was redacted"
    );
    assert_eq!(redact(echoed), "<redacted>");
    assert_eq!(
        redact(VALID_ID),
        VALID_ID,
        "an object id is not a secret, so it still echoes"
    );
    assert_eq!(redact("not-a-uuid"), "not-a-uuid");
}

#[test]
fn refusal_advice_same_bad_args_give_equal_bytes_twice() {
    let build = || {
        let ops = [
            OpDto::Remove {
                id: "not-a-uuid".to_owned(),
            },
            OpDto::SetComponent {
                id: VALID_ID.to_owned(),
                component: "not_a_component".to_owned(),
                value: json!({}),
                version: None,
            },
        ];
        build_batch(3, 11, &ops, None)
            .expect_err("bad fields refuse the batch")
            .wire_payload()
            .to_string()
    };
    assert_eq!(build(), build(), "the same bad args are the same bytes");
}

#[test]
fn refusal_advice_every_next_call_is_a_live_verb() {
    let codes = produced_codes();
    for code in &codes {
        if let Some(next) = next_for(code) {
            assert!(
                tool_names().contains(&next.verb),
                "`{code}` names next call `{}`, which the door does not serve",
                next.verb
            );
        }
    }
}
