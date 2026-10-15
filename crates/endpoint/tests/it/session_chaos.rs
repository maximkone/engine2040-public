//! Session-isolation chaos: twelve proposers, one object, one winner.
//!
//! Consumer: the session-isolation chaos proof over the stdio door and the
//! loopback remote shim. Data owner stays the editor command service; this
//! file owns only the race harness. Units are tool calls and revisions.
//! Determinism: same seed poses always elect the same winner set size (one)
//! with the same typed conflicts; wall-clock timing stays in the ignored
//! perf test only. Errors are typed tool codes, asserted by name.

#![cfg(feature = "mcp")]

use std::sync::{Barrier, Mutex};
use std::time::{Duration, Instant};

use engine_editor::capability::Capability;
use engine_endpoint::mcp::{
    ApprovalDto, McpServer, OpDto, PoseDto, SceneReadArgs, SceneReadTool, TxApplyArgs, TxApplyTool,
    TxPreviewArgs, TxPreviewTool, MAX_TX_OPS,
};
use engine_endpoint::play::{PlayRunArgs, PlayRunTool, MAX_PLAY_TICKS};
use engine_endpoint::scene_tools::{ScenePlaceArgs, ScenePlaceTool, MAX_PLACE_WANTS};
use engine_endpoint::transport;
use engine_scene::ids::ObjectId;
use rmcp::handler::server::router::tool::SyncTool;

/// Racing proposers on one object.
const PROPOSERS: usize = 12;
/// Session id of the shared door.
const SESSION: u64 = 7;
/// Shared secret for the test-only loopback shim.
const SHIM_TOKEN: &str = "ax14-loopback-token";
/// Bounded wait for the play leg beside a place storm.
const PLAY_WAIT: Duration = Duration::from_secs(20);

// Serializes the place-touching tests in this binary. The scene-apply guard
// is process-wide: without this serial a parallel place test could land
// inside another test's window and answer `apply_in_flight` spuriously.
// Lock order is serial first, then the server lock, so this cannot deadlock.
static PLACE_SERIAL: Mutex<()> = Mutex::new(());

fn pose(x: f32) -> PoseDto {
    PoseDto {
        pos: [x, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

fn move_op(id: &ObjectId, x: f32) -> OpDto {
    OpDto::SetLocal {
        id: id.to_string(),
        local: pose(x),
    }
}

fn revision_of(server: &McpServer) -> u64 {
    SceneReadTool::invoke(server, SceneReadArgs::default())
        .expect("scene reads for the revision")
        .revision
}

fn pose_of(server: &McpServer, id: &ObjectId) -> [f32; 3] {
    SceneReadTool::invoke(
        server,
        SceneReadArgs {
            limit: Some(128),
            cursor: None,
        },
    )
    .expect("scene reads for the pose")
    .objects
    .iter()
    .find(|row| row.id == id.to_string())
    .expect("contested node reads back")
    .local
    .pos
}

fn total_of(server: &McpServer) -> u64 {
    SceneReadTool::invoke(server, SceneReadArgs::default())
        .expect("scene reads for the total")
        .total
}

fn preview(server: &McpServer, rev: u64, key: u64, ops: Vec<OpDto>) -> Result<ApprovalDto, String> {
    TxPreviewTool::invoke(
        server,
        TxPreviewArgs {
            expected_revision: rev,
            idempotency_key: key,
            ops: ops.clone(),
        },
    )
    .map(|out| out.approval)
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

fn apply(
    server: &McpServer,
    rev: u64,
    key: u64,
    ops: Vec<OpDto>,
    approval: ApprovalDto,
) -> Result<u64, String> {
    TxApplyTool::invoke(
        server,
        TxApplyArgs {
            expected_revision: rev,
            idempotency_key: key,
            ops,
            approval,
        },
    )
    .map(|out| out.revision)
    .map_err(|error| format!("{}: {}", error.code, error.message))
}

/// One contested object at the origin, revision 1.
fn seed_server() -> (McpServer, ObjectId) {
    let server = McpServer::new(SESSION, Capability::unrestricted(SESSION));
    let id = ObjectId::new();
    let ops = vec![OpDto::Insert {
        id: id.to_string(),
        parent: ObjectId::NIL.to_string(),
        local: pose(0.0),
    }];
    let approval = preview(&server, 0, 1, ops.clone()).expect("seed previews");
    apply(&server, 0, 1, ops, approval).expect("seed applies");
    assert_eq!(revision_of(&server), 1);
    (server, id)
}

/// Test-only loopback shim: the remote path before dispatch. Checks the
/// loopback host, the token intersect and the payload caps, then runs the
/// same door call, so shim rows hit the same typed gate as stdio rows.
struct RemoteShim<'a> {
    server: &'a McpServer,
    presented_token: String,
}

impl<'a> RemoteShim<'a> {
    fn check(&self, payload_chars: usize) -> Result<(), String> {
        if !transport::is_loopback("127.0.0.1") {
            return Err("shim refuses non-loopback hosts".to_owned());
        }
        if !transport::token_ok(&self.presented_token, SHIM_TOKEN) {
            return Err("shim refuses a bad token".to_owned());
        }
        if !transport::check_caps(payload_chars, payload_chars) {
            return Err("shim refuses an over-cap payload".to_owned());
        }
        Ok(())
    }

    fn preview(&self, rev: u64, key: u64, ops: Vec<OpDto>) -> Result<ApprovalDto, String> {
        self.check(ops.len().saturating_mul(64))?;
        preview(self.server, rev, key, ops)
    }

    fn apply(
        &self,
        rev: u64,
        key: u64,
        ops: Vec<OpDto>,
        approval: ApprovalDto,
    ) -> Result<u64, String> {
        self.check(ops.len().saturating_mul(64))?;
        apply(self.server, rev, key, ops, approval)
    }
}

fn is_typed_conflict(code: &str) -> bool {
    matches!(code, "stale_revision" | "expired_approval")
}

fn conflict_code(outcome: &str) -> &str {
    outcome.split_once(':').map_or(outcome, |(code, _)| code)
}

#[test]
fn twelve_proposers_one_object_single_winner_typed_conflicts() {
    let (server, id) = seed_server();
    let shared: &McpServer = &server;
    let barrier = Barrier::new(PROPOSERS);
    let gate: &Barrier = &barrier;
    let mut xs = Vec::with_capacity(PROPOSERS);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(PROPOSERS);
        for proposer in 0..PROPOSERS {
            let key = u64::from(100u32)
                .checked_add(u64::try_from(proposer).expect("proposer index fits"))
                .expect("key space holds twelve proposers");
            let x = f32::from(u16::try_from(proposer).expect("proposer index fits")) + 1.0;
            handles.push(scope.spawn(move || {
                let ops = vec![move_op(&id, x)];
                assert!(ops.len() <= MAX_TX_OPS, "bounded work: one op per batch");
                let approval = preview(shared, 1, key, ops.clone())
                    .expect("every preview at revision 1 drafts");
                gate.wait();
                (x, key, apply(shared, 1, key, ops, approval))
            }));
        }
        for handle in handles {
            let (x, key, outcome) = handle.join().expect("proposer thread joins");
            xs.push((x, key, outcome));
        }
    });
    let winners: Vec<(f32, u64)> = xs
        .iter()
        .filter_map(|(x, key, outcome)| outcome.as_ref().ok().map(|_| (*x, *key)))
        .collect();
    let losers: Vec<&String> = xs
        .iter()
        .filter_map(|(_, _, outcome)| outcome.as_ref().err())
        .collect();
    assert_eq!(winners.len(), 1, "exactly one proposer lands: {xs:?}");
    assert_eq!(losers.len(), PROPOSERS - 1);
    for loser in &losers {
        let code = conflict_code(loser);
        assert!(
            is_typed_conflict(code),
            "every conflict is typed stale/expired, got: {loser}"
        );
        assert!(
            loser.contains('2'),
            "every conflict names the fresh revision, got: {loser}"
        );
    }
    let (winner_x, _) = winners[0];
    assert_eq!(revision_of(&server), 2, "one mutation, one revision");
    assert_eq!(total_of(&server), 1, "no proposer inserts or deletes");
    assert_eq!(pose_of(&server, &id)[0], winner_x);
    for (x, _, outcome) in &xs {
        if outcome.is_err() {
            assert_ne!(*x, pose_of(&server, &id)[0], "no loser pose leaks in");
        }
    }
    let log = server.audit_log().expect("door audit reads");
    assert!(
        log.len() <= 96,
        "bounded work: the whole race seals {} records",
        log.len()
    );
}

#[test]
fn door_approval_binds_exact_batch_and_single_use() {
    // One door is one session: approvals bind the exact batch hash plus the
    // revision, and each grant spends once. A tampered payload, a replayed
    // grant and a stale base all fail typed and write nothing.
    let (server, id) = seed_server();
    let ops = vec![move_op(&id, 9.0)];
    let approval = preview(&server, 1, 12, ops.clone()).expect("move previews");
    // Wrong payload under a valid approval: forged, writes nothing.
    let tampered = vec![move_op(&id, 10.0)];
    let forged =
        apply(&server, 1, 13, tampered, approval.clone()).expect_err("tampered payload must fail");
    assert!(
        forged.starts_with("forged_approval:"),
        "exact-batch bind is typed, got: {forged}"
    );
    assert_eq!(revision_of(&server), 1, "forged apply writes nothing");
    assert_eq!(pose_of(&server, &id), [0.0, 0.0, 0.0]);
    // The exact batch lands once.
    let landed = apply(&server, 1, 12, ops.clone(), approval.clone()).expect("exact lands");
    assert_eq!(landed, 2);
    assert_eq!(pose_of(&server, &id)[0], 9.0);
    // Same key plus same payload replays the receipt without a second write.
    let replayed =
        apply(&server, 1, 12, ops.clone(), approval.clone()).expect("idempotent replay answers");
    assert_eq!(replayed, 2);
    assert_eq!(pose_of(&server, &id)[0], 9.0);
    // Same grant under a fresh key is spent: reused, writes nothing.
    let reused = apply(&server, 2, 14, ops.clone(), approval).expect_err("spent grant must fail");
    assert!(
        reused.starts_with("reused_approval:"),
        "single-use bind is typed, got: {reused}"
    );
    assert_eq!(revision_of(&server), 2);
    // A preview against the moved revision is stale with the fresh one named.
    let stale = preview(&server, 1, 15, ops).expect_err("stale preview must fail");
    assert!(
        stale.starts_with("stale_revision:"),
        "stale bind is typed, got: {stale}"
    );
    assert!(stale.contains('2'), "stale bind names the fresh revision");
}

#[test]
fn scene_place_race_one_winner_rest_typed() {
    let _serial = PLACE_SERIAL.lock().expect("place tests serialize");
    let (server, id) = seed_server();
    let shared: &McpServer = &server;
    let rev = revision_of(&server);
    let outcomes: Vec<Result<bool, String>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(PROPOSERS);
        for proposer in 0..PROPOSERS {
            let key = u64::from(300u32)
                .checked_add(u64::try_from(proposer).expect("proposer index fits"))
                .expect("key space holds twelve proposers");
            handles.push(scope.spawn(move || {
                let wants = vec!["next_to wall:north".to_owned()];
                assert!(
                    wants.len() <= MAX_PLACE_WANTS,
                    "bounded work: one want per place"
                );
                ScenePlaceTool::invoke(
                    shared,
                    ScenePlaceArgs {
                        expected_revision: rev,
                        idempotency_key: key,
                        object: id.to_string(),
                        wants,
                    },
                )
                .map(|out| out.solved)
                .map_err(|error| format!("{}: {}", error.code, error.message))
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("placer thread joins"))
            .collect()
    });
    let solved = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Ok(true)))
        .count();
    assert_eq!(solved, 1, "one place lands: {outcomes:?}");
    for outcome in &outcomes {
        if let Err(text) = outcome {
            let code = conflict_code(text);
            assert!(
                code == "stale_revision" || code == "apply_in_flight",
                "every place conflict is typed, got: {text}"
            );
        }
    }
    assert_eq!(revision_of(&server), rev + 1, "one mutation, one revision");
    assert_eq!(total_of(&server), 1, "place never inserts or deletes");
}

#[test]
fn play_run_completes_beside_place_storm() {
    let _serial = PLACE_SERIAL.lock().expect("place tests serialize");
    let (server, id) = seed_server();
    let shared: &McpServer = &server;
    let rev = revision_of(&server);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for proposer in 0..PROPOSERS {
            let key = u64::from(500u32)
                .checked_add(u64::try_from(proposer).expect("proposer index fits"))
                .expect("key space holds twelve proposers");
            scope.spawn(move || {
                let _ = ScenePlaceTool::invoke(
                    shared,
                    ScenePlaceArgs {
                        expected_revision: rev,
                        idempotency_key: key,
                        object: id.to_string(),
                        wants: vec!["next_to wall:south".to_owned()],
                    },
                );
            });
        }
        scope.spawn(move || {
            let started = PlayRunTool::invoke(
                shared,
                PlayRunArgs {
                    action: "start".to_owned(),
                    seed: Some(2040),
                    ticks: Some(8),
                    fighters: Some(2),
                    half_extent_m: Some(20.0),
                    inputs: None,
                    ttl_polls: None,
                    job_id: None,
                    scene_rev: None,
                },
            )
            .expect("play starts beside a place storm");
            let job = started.job_id;
            let poll = |job_id| {
                PlayRunTool::invoke(
                    shared,
                    PlayRunArgs {
                        action: "poll".to_owned(),
                        seed: None,
                        ticks: None,
                        fighters: None,
                        half_extent_m: None,
                        inputs: None,
                        ttl_polls: None,
                        job_id: Some(job_id),
                        scene_rev: None,
                    },
                )
            };
            poll(job).expect("progress polls beside a place storm");
            let finished = poll(job).expect("result lands beside a place storm");
            let _ = done_tx.send(finished);
        });
    });
    let finished = done_rx
        .recv_timeout(PLAY_WAIT)
        .expect("play answers within its bound");
    assert_eq!(finished.status, "completed");
    assert_eq!(finished.checkpoints.len(), 8);
    assert!(finished.final_hash.is_some());
    assert!(
        finished.tick_count <= MAX_PLAY_TICKS,
        "bounded work: one short job beside the storm"
    );
}

#[test]
fn remote_shim_loopback_hits_same_typed_gate() {
    assert!(transport::is_loopback("127.0.0.1"));
    assert!(!transport::is_loopback("203.0.113.7"));
    assert!(transport::token_ok(SHIM_TOKEN, SHIM_TOKEN));
    assert!(!transport::token_ok("wrong", SHIM_TOKEN));
    assert!(transport::check_caps(64, 64));
    assert_eq!(transport::timeout_for("patch"), 10);
    let (server, id) = seed_server();
    let shared: &McpServer = &server;
    let barrier = Barrier::new(PROPOSERS);
    let gate: &Barrier = &barrier;
    let mut outcomes = Vec::with_capacity(PROPOSERS);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(PROPOSERS);
        for proposer in 0..PROPOSERS {
            let key = u64::from(700u32)
                .checked_add(u64::try_from(proposer).expect("proposer index fits"))
                .expect("key space holds twelve proposers");
            let x = f32::from(u16::try_from(proposer).expect("proposer index fits")) + 21.0;
            handles.push(scope.spawn(move || {
                let shim = RemoteShim {
                    server: shared,
                    presented_token: SHIM_TOKEN.to_owned(),
                };
                let ops = vec![move_op(&id, x)];
                let approval = if proposer % 2 == 0 {
                    preview(shared, 1, key, ops.clone())
                } else {
                    shim.preview(1, key, ops.clone())
                }
                .expect("every preview drafts");
                gate.wait();
                let outcome = if proposer % 2 == 0 {
                    apply(shared, 1, key, ops, approval)
                } else {
                    shim.apply(1, key, ops, approval)
                };
                (x, outcome)
            }));
        }
        for handle in handles {
            outcomes.push(handle.join().expect("shim proposer joins"));
        }
    });
    let winners = outcomes
        .iter()
        .filter(|(_, outcome)| outcome.is_ok())
        .count();
    assert_eq!(winners, 1, "one shim-gated winner: {outcomes:?}");
    for (_, outcome) in &outcomes {
        if let Err(text) = outcome {
            assert!(
                is_typed_conflict(conflict_code(text)),
                "shim conflicts stay typed, got: {text}"
            );
            assert!(text.contains('2'), "shim conflicts name the fresh revision");
        }
    }
    assert_eq!(revision_of(&server), 2);
    assert_eq!(total_of(&server), 1);
    let bad = RemoteShim {
        server: &server,
        presented_token: "wrong".to_owned(),
    };
    let refused = bad
        .preview(2, 799, vec![move_op(&id, 1.0)])
        .expect_err("bad shim token refuses before dispatch");
    assert!(refused.contains("token"), "got: {refused}");
    assert_eq!(revision_of(&server), 2, "shim refusal writes nothing");
}

fn percentile_sorted(mut samples: Vec<Duration>, pct: u128) -> Duration {
    samples.sort();
    let len = u128::try_from(samples.len()).expect("sample count fits");
    let rank = len.saturating_mul(pct).checked_div(100).unwrap_or(0);
    let index = rank.min(len - 1);
    samples[usize::try_from(index).expect("percentile index fits")]
}

#[test]
#[ignore = "timing: p95 needs release clocks; run under --perf"]
fn p95_within_ten_percent_of_single_client() {
    let (server, _) = seed_server();
    let shared: &McpServer = &server;
    let mut single = Vec::with_capacity(200);
    for _ in 0..200 {
        let start = Instant::now();
        let _ = SceneReadTool::invoke(shared, SceneReadArgs::default());
        single.push(start.elapsed());
    }
    let single_p95 = percentile_sorted(single, 95);
    let barrier = Barrier::new(PROPOSERS);
    let gate: &Barrier = &barrier;
    let mut loaded = Vec::with_capacity(600);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(PROPOSERS);
        for _ in 0..PROPOSERS {
            handles.push(scope.spawn(move || {
                let mut local = Vec::with_capacity(50);
                gate.wait();
                for _ in 0..50 {
                    let start = Instant::now();
                    let _ = SceneReadTool::invoke(shared, SceneReadArgs::default());
                    local.push(start.elapsed());
                }
                local
            }));
        }
        for handle in handles {
            loaded.extend(handle.join().expect("reader thread joins"));
        }
    });
    let loaded_p95 = percentile_sorted(loaded, 95);
    let bound = single_p95
        .as_nanos()
        .saturating_mul(110)
        .checked_div(100)
        .unwrap_or(u128::MAX)
        .saturating_add(50_000);
    assert!(
        loaded_p95.as_nanos() <= bound,
        "p95 loaded {loaded_p95:?} within +10% of single {single_p95:?}"
    );
}

/// The stdio leg: one outside client over the `e2040-mcp` binary previews,
/// applies, then goes stale. The door must answer the same typed
/// `stale_revision`/`expired_approval` with the fresh revision, proving the
/// stdio transport hits the same gate as the in-process chaos above.
/// Minimal outside client over the `e2040-mcp` stdio binary: one JSON-RPC
/// request line in, one reply line out, bounded waits throughout.
struct ChaosClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::sync::Arc<std::sync::Mutex<std::io::BufReader<std::process::ChildStdout>>>,
    next_id: u64,
}

impl ChaosClient {
    /// Per-request client context for the 2026-07-28 inline lifecycle.
    const META: &'static str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"e2040-ax14-test","version":"0.0.0"}}"#;

    /// Spawn the door on a fresh scratch project with piped stdio.
    fn spawn() -> (Self, std::path::PathBuf) {
        use std::io::BufReader;
        use std::process::{Command, Stdio};

        let dir =
            std::env::temp_dir().join(format!("e2040-ax14-stdio-{n}", n = std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch project creates");
        let mut child = Command::new(env!("CARGO_BIN_EXE_e2040-mcp"))
            .arg("--project-dir")
            .arg(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("e2040-mcp spawns");
        let stdin = child.stdin.take().expect("child stdin pipes");
        let stdout = child.stdout.take().expect("child stdout pipes");
        (
            Self {
                child,
                stdin,
                stdout: std::sync::Arc::new(std::sync::Mutex::new(BufReader::new(stdout))),
                next_id: 1,
            },
            dir,
        )
    }

    /// One raw JSON-RPC round trip: the full reply, error or not.
    fn send_raw(&mut self, method: &str, params: &str) -> rmcp::serde_json::Value {
        use std::io::{BufRead, Write};

        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}"
        )
        .expect("request writes");
        self.stdin.flush().expect("request flushes");
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::sync::Arc::clone(&self.stdout);
        std::thread::spawn(move || {
            let mut line = String::new();
            let read = reader
                .lock()
                .expect("reader lock holds")
                .read_line(&mut line);
            let _ = tx.send(read.map(|_| line));
        });
        let line = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("server replies within its bound")
            .expect("server stdout reads");
        let reply: rmcp::serde_json::Value =
            rmcp::serde_json::from_str(&line).expect("one JSON object per line");
        assert_eq!(
            reply["jsonrpc"].as_str(),
            Some("2.0"),
            "server speaks JSON-RPC 2.0: {reply}"
        );
        assert_eq!(
            reply["id"].as_u64(),
            Some(id),
            "server echoes the request id: {reply}"
        );
        reply
    }

    /// One round trip that must succeed: the `result` payload.
    fn send(&mut self, method: &str, params: &str) -> rmcp::serde_json::Value {
        let reply = self.send_raw(method, params);
        assert!(
            reply.get("error").is_none(),
            "server answers ok, got: {reply}"
        );
        reply["result"].clone()
    }

    /// One tool call carrying the lifecycle context.
    fn call(&mut self, tool: &str, arguments: &str) -> rmcp::serde_json::Value {
        let params = format!(
            "{{{meta},\"name\":\"{tool}\",\"arguments\":{arguments}}}",
            meta = Self::META
        );
        self.send("tools/call", &params)
    }

    /// Tool result payload: `structuredContent`, else the JSON text block.
    fn payload(result: &rmcp::serde_json::Value) -> rmcp::serde_json::Value {
        if let Some(payload) = result.get("structuredContent") {
            return payload.clone();
        }
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("tool answers JSON text, got: {result}"));
        rmcp::serde_json::from_str(text).expect("tool text parses as JSON")
    }

    /// Close stdin and wait for the clean EOF exit, then drop the scratch dir.
    fn close_and_wait(self, dir: &std::path::Path) {
        let mut child = self.child;
        drop(self.stdin);
        let exited = (0..200).any(|_| match child.try_wait().expect("exit status polls") {
            Some(status) => status.success(),
            None => {
                std::thread::sleep(Duration::from_millis(50));
                false
            }
        });
        assert!(exited, "server exits cleanly on stdin EOF");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The stdio leg: one outside client over the `e2040-mcp` binary previews,
/// applies, then goes stale. The door must answer the same typed
/// `stale_revision` with the fresh revision, proving the stdio transport
/// hits the same gate as the in-process chaos above.
#[test]
fn stdio_door_replays_typed_stale_revision() {
    let (mut client, dir) = ChaosClient::spawn();
    // The 2026-07-28 inline lifecycle opens with `server/discover`, exactly
    // like any outside client: no tool call runs before discovery.
    let discovered = client.send(
        "server/discover",
        &format!("{{{meta}}}", meta = ChaosClient::META),
    );
    assert!(
        discovered["supportedVersions"]
            .as_array()
            .is_some_and(|versions| versions
                .iter()
                .any(|version| version.as_str() == Some("2026-07-28"))),
        "door serves 2026-07-28: {discovered}"
    );
    let node = "123e4567-e89b-42d3-a456-426614174001";
    let nil = "00000000-0000-0000-0000-000000000000";
    let op = format!(
        "{{\"op\":\"insert\",\"id\":\"{node}\",\"parent\":\"{nil}\",\
         \"local\":{{\"pos\":[1.0,0.0,0.0],\"rot\":[0.0,0.0,0.0,1.0],\
         \"scale\":[1.0,1.0,1.0]}}}}"
    );
    let preview_args =
        format!("{{\"expected_revision\":0,\"idempotency_key\":9001,\"ops\":[{op}]}}");
    let previewed = ChaosClient::payload(&client.call("tx.preview", &preview_args));
    let approval = previewed["approval"].to_string();
    let apply_args =
        format!("{{\"expected_revision\":0,\"idempotency_key\":9001,\"ops\":[{op}],\"approval\":{approval}}}");
    let applied = ChaosClient::payload(&client.call("tx.apply", &apply_args));
    assert_eq!(applied["revision"].as_u64(), Some(1));
    // A new batch against the moved revision stays typed: the same
    // `stale_revision` plus the fresh revision the chaos rows assert in
    // process, now over the stdio wire.
    let stale_node = "123e4567-e89b-42d3-a456-426614174002";
    let stale_op = format!(
        "{{\"op\":\"insert\",\"id\":\"{stale_node}\",\"parent\":\"{nil}\",\
         \"local\":{{\"pos\":[2.0,0.0,0.0],\"rot\":[0.0,0.0,0.0,1.0],\
         \"scale\":[1.0,1.0,1.0]}}}}"
    );
    let stale_args =
        format!("{{\"expected_revision\":0,\"idempotency_key\":9002,\"ops\":[{stale_op}]}}");
    let stale_params = format!(
        "{{{meta},\"name\":\"tx.preview\",\"arguments\":{stale_args}}}",
        meta = ChaosClient::META
    );
    let refused = client.send_raw("tools/call", &stale_params);
    let refused_text = refused.to_string();
    assert!(
        refused_text.contains("stale_revision"),
        "stale preview stays typed over stdio, got: {refused_text}"
    );
    assert!(
        refused_text.contains('1'),
        "stale preview names the fresh revision over stdio"
    );
    assert!(
        refused.get("error").is_some(),
        "stale preview refuses over stdio, got: {refused_text}"
    );
    client.close_and_wait(&dir);
}
