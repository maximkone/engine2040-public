//! AD8 door parity: the same 20 mirror tasks through the playtest door and
//! through the spawned `e2040-mcp` stdio binary must give equal outcome
//! digests, else the benchmark measured a stand-in.
//!
//! Own test target: the Accept names it as `--test door_parity`, and it spawns
//! the stdio binary as a child, exactly like an outside MCP client.
//!
//! Consumer: the AD8 row and every later door change: one library read of the
//! mirror (playtest `run_mirror`) and one door call (`verify.proof`) must
//! digest the same rows. Units: task counts, 16-char digests. Determinism: the
//! mirror is recorded replay, so the same build answers the same digests on
//! both paths, twice.
//!
//! Run: `cargo test -p engine-endpoint --features mcp,verify --test door_parity`.

#![cfg(all(feature = "mcp", feature = "verify"))]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use engine_endpoint::packs::verify::{mirror_task_digests, VerifyProofOutput};
use engine_playtest::mirror::run_mirror;
use engine_playtest::seeded_bug::{bisect_seeded, seeded_bugfile, SEEDED_TICK};
use rmcp::serde_json::{self, Value};

/// Mirror tasks the parity claim covers (the first 20 in report order).
const PARITY_TASKS: usize = 20;

/// Bounded wait per server reply: a silent server fails the test, never hangs
/// it. The proof verb runs a 400-tick seeded match plus the 30-task mirror.
const REPLY_TIMEOUT: Duration = Duration::from_secs(240);

/// Client context for the 2026-07-28 inline lifecycle: every request carries
/// the protocol version and client capabilities in its `_meta`.
const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"e2040-ad8-parity","version":"0.0.0"}}"#;

/// The stdio binary cargo just built for this feature set.
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_e2040-mcp")
}

/// Fresh scratch project (parallel-safe: one unique name per process).
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("e2040-ad8-parity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch project creates");
    dir
}

/// One outside client: a child binary plus its piped stdin and stdout.
struct Door {
    child: Child,
    stdin: ChildStdin,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    next_id: u64,
}

impl Door {
    /// Spawn the stdio door on one project folder.
    fn spawn(project_dir: &Path) -> Self {
        let mut child = Command::new(bin())
            .arg("--project-dir")
            .arg(project_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("e2040-mcp spawns");
        let stdin = child.stdin.take().expect("child stdin pipes");
        let stdout = child.stdout.take().expect("child stdout pipes");
        Self {
            child,
            stdin,
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            next_id: 1,
        }
    }

    /// One JSON-RPC request: write a line, read the reply line, return `result`.
    fn send(&mut self, method: &str, params: &str) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}"
        )
        .expect("request writes");
        self.stdin.flush().expect("request flushes");
        let reply = read_reply(&self.stdout);
        assert_eq!(reply["id"].as_u64(), Some(id), "id echoes: {reply}");
        assert!(reply.get("error").is_none(), "request succeeds: {reply}");
        reply["result"].clone()
    }

    /// One tool call with raw arguments JSON; the payload is returned parsed.
    fn call(&mut self, tool: &str, arguments: &str) -> Value {
        let params = format!("{{{META},\"name\":\"{tool}\",\"arguments\":{arguments}}}");
        structured(&self.send("tools/call", &params))
    }
}

impl Drop for Door {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Read one stdout line on a helper thread: bounded, never hangs the suite.
fn read_reply(stdout: &Arc<Mutex<BufReader<ChildStdout>>>) -> Value {
    let (sender, receiver) = mpsc::channel();
    let reader = Arc::clone(stdout);
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = reader
            .lock()
            .expect("reader lock holds")
            .read_line(&mut line);
        let _ = sender.send(read.map(|_| line));
    });
    match receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(line)) => {
            serde_json::from_str(&line).expect("the server replies with one JSON object per line")
        }
        Ok(Err(error)) => panic!("server stdout read failed: {error}"),
        Err(_) => panic!("timed out waiting for the server reply"),
    }
}

/// Tool result payload: `structuredContent` when sent, else the first text block.
fn structured(result: &Value) -> Value {
    if let Some(payload) = result.get("structuredContent") {
        return payload.clone();
    }
    let text = result["content"][0]["text"]
        .as_str()
        .expect("tool answers JSON text");
    serde_json::from_str(text).expect("tool text parses as JSON")
}

/// One door-side proof read: spawn, discover, then ask twice with the same args.
fn door_proof(dir: &Path) -> (VerifyProofOutput, VerifyProofOutput) {
    let mut door = Door::spawn(dir);
    let discovered = door.send("server/discover", &format!("{{{META}}}"));
    assert!(
        discovered["capabilities"]["tools"].is_object(),
        "the door advertises tools: {discovered}"
    );
    let args = format!("{{\"tasks\":{PARITY_TASKS}}}");
    let first: VerifyProofOutput =
        serde_json::from_value(door.call("verify.proof", &args)).expect("proof output parses");
    let second: VerifyProofOutput =
        serde_json::from_value(door.call("verify.proof", &args)).expect("second proof parses");
    (first, second)
}

#[test]
fn the_same_mirror_tasks_digest_equal_through_the_playtest_door_and_stdio() {
    // Library path: the playtest mirror, digested by the one shared owner.
    let expected = mirror_task_digests(&run_mirror(), PARITY_TASKS);
    assert_eq!(
        expected.len(),
        PARITY_TASKS * 3,
        "20 tasks on three backends"
    );

    // The A7 seeded tape bisects to its seeded tick in-process first.
    let bug = seeded_bugfile().expect("the seeded run files a bug");
    let seeded_tick = bisect_seeded(&bug).expect("the filed tape bisects");
    assert_eq!(
        seeded_tick, SEEDED_TICK,
        "the control names the seeded tick"
    );

    // Door path: the spawned stdio binary runs the same bodies behind its own gate.
    let dir = scratch();
    let (first, second) = door_proof(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        first.seeded_tick, seeded_tick,
        "the door bisects the same seeded tape to the same tick"
    );
    assert_eq!(first.seeded_expected_tick, SEEDED_TICK);
    assert!(first.seeded_clean, "the control side replays clean");
    assert_eq!(first.tasks, u32::try_from(PARITY_TASKS).unwrap_or(0));
    assert_eq!(
        first.mirror, expected,
        "every task/backend digest is equal on both paths"
    );
    assert_eq!(first, second, "the same door call answers twice");
}
