//! `look.judge` through the door once a host registers a judge.
//!
//! Own process on purpose: the hook slot is process-wide and one-shot, and the
//! library tests rely on it staying empty. Run:
//! `cargo test -p engine-endpoint --features mcp --test look_judge_hook`.
//! Units: permille. Determinism: the hook is a pure function of its request.

#![cfg(feature = "mcp")]

use engine_editor::capability::Capability;
use engine_endpoint::mcp::{McpServer, McpToolError};
use engine_endpoint::packs::view::{
    register_look_judge, LookJudgeArgs, LookJudgeTool, LOOK_RUBRIC_IDS,
};
use engine_endpoint::packs::PackRegistry;
use rmcp::handler::server::router::tool::SyncTool;

fn server() -> McpServer {
    McpServer::new(7, Capability::unrestricted(7))
}

fn request(dir: &str) -> LookJudgeArgs {
    LookJudgeArgs {
        capture_dir: dir.to_owned(),
        preview_dir: None,
        q1_dir: None,
        // ARX05: the door sets the wording; a caller never names one.
        wording: None,
    }
}

/// A stand-in judge: the directory name picks the behaviour, so one
/// registration serves every case below.
fn host_judge(args: &LookJudgeArgs) -> Result<String, McpToolError> {
    match args.capture_dir.as_str() {
        "refuse" => Err(McpToolError::new(
            "no_frames",
            "capture set holds no frame; re-run",
        )),
        "float" => Ok(report(500).replace("\"score\":500,", "\"score\":500.5,")),
        dir => Ok(report(u64::try_from(dir.len()).unwrap_or(0) * 100)),
    }
}

fn report(score: u64) -> String {
    let items: Vec<String> = LOOK_RUBRIC_IDS
        .iter()
        .map(|id| format!("{{\"id\":\"{id}\",\"score\":{score},\"pass\":true}}"))
        .collect();
    format!(
        "{{\"schema\":1,\"digest\":\"0123456789abcdef\",\"pass\":true,\"score\":{score},\
         \"rubric\":[{}],\"frames\":[{{\"name\":\"frame_000000.png\"}}]}}",
        items.join(",")
    )
}

#[test]
fn registered_judge_answers_through_the_door_and_the_registry() {
    let door = server();
    // Before registration the door refuses, never a success shape.
    let early = LookJudgeTool::invoke(&door, request("captures")).expect_err("unwired refuses");
    assert_eq!(early.code, "unsupported_look_judge");

    register_look_judge(host_judge).expect("first registration lands");
    let second = register_look_judge(host_judge).expect_err("one shot");
    assert_eq!(second.code, "look_judge_registered");

    let out = LookJudgeTool::invoke(&door, request("captures")).expect("judge answers");
    assert_eq!((out.schema, out.pass, out.score), (1, true, 800));
    assert_eq!(out.digest, "0123456789abcdef");
    assert_eq!(out.judgement["rubric"].as_array().map(Vec::len), Some(9));

    // The registry path (`api.call`) lands on the same answer.
    let verb = PackRegistry::shipped()
        .verb("look.judge")
        .expect("listed verb");
    let again = verb
        .invoke(&door, rmcp::serde_json::json!({"capture_dir": "captures"}))
        .expect("registry call answers");
    assert_eq!(again["score"], 800);

    // The judge's own refusal and a contract break both come back typed.
    let refused = LookJudgeTool::invoke(&door, request("refuse")).expect_err("judge refuses");
    assert_eq!(refused.code, "no_frames");
    let broken = LookJudgeTool::invoke(&door, request("float")).expect_err("float refused");
    assert_eq!(broken.code, "bad_receipt");
    // Bad paths never reach the judge.
    let escape = LookJudgeTool::invoke(&door, request("../outside")).expect_err("escape refused");
    assert_eq!(escape.code, "bad_args");
}
