//! `look.judge` through the door with the REAL engine-playtest judge hosted.
//!
//! Own process on purpose: the hook slot is process-wide and one-shot, and the
//! library tests rely on it staying empty. Run:
//! `cargo test -p engine-endpoint --features mcp --test look_judge_real`.
//! Units: permille. Determinism: the host judges whatever bytes the capture
//! directory holds, so a missing directory and a broken manifest refuse the
//! same way twice.

#![cfg(feature = "mcp")]

use engine_editor::capability::Capability;
use engine_endpoint::look_host::host_look_judge;
use engine_endpoint::mcp::McpServer;
use engine_endpoint::packs::view::{register_look_judge, LookJudgeArgs, LookJudgeTool};
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

#[test]
fn real_judge_answers_the_door_and_refuses_a_set_it_cannot_read() {
    let door = server();

    // One-shot: this is the only registration in the process, and the slot
    // refuses a second one.
    register_look_judge(host_look_judge).expect("the real host registers once");
    let second = register_look_judge(host_look_judge).expect_err("one shot");
    assert_eq!(second.code, "look_judge_registered");

    // A directory that is not there: the REAL judge ran and refused, so the
    // code is the bridge's, never the unwired `unsupported_look_judge`.
    let absent = format!("target/look-judge-real-absent-{}", std::process::id());
    let missing = LookJudgeTool::invoke(&door, request(&absent)).expect_err("no such captures");
    assert_eq!(missing.code, "look_capture");
    assert_ne!(missing.code, "unsupported_look_judge");
    assert!(!missing.message.is_empty());

    // A capture directory whose manifest is invalid JSON: the judge's own
    // manifest refusal crosses the door typed.
    let root = std::env::temp_dir().join(format!("e2040-look-judge-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let captures = root.join("captures");
    std::fs::create_dir_all(&captures).expect("temp capture dir");
    std::fs::write(captures.join("frame_000000.png"), b"not a png").expect("write frame");
    std::fs::write(captures.join("manifest.json"), "{{").expect("write manifest");

    let home = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&root).expect("enter the temp dir");
    let broken = LookJudgeTool::invoke(&door, request("captures")).expect_err("manifest refused");
    std::env::set_current_dir(&home).expect("leave the temp dir");

    assert_eq!(broken.code, "look_manifest");
    assert!(broken.message.contains("manifest"), "{}", broken.message);

    let _ = std::fs::remove_dir_all(&root);
}
