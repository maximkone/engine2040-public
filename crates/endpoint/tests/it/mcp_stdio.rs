//! MCP5 scripted external client: list, preview, apply and undo over stdio.
//!
//! Consumer: external MCP clients (Claude Code, OpenCode, Cursor) and the EV1
//! eval harness. Spawns the `e2040-mcp` binary as a child and speaks
//! newline-delimited JSON-RPC on its stdin/stdout, exactly like an outside
//! client would. Raw frames keep the client side dependency-free: no rmcp
//! client feature is needed (the packet forbids new dependencies).

#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use rmcp::serde_json::{self, Value};

/// Bounded wait per server reply: a silent server fails the test, never hangs it.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);
/// What a fresh door lists: the thirteen core tools plus the eight default
/// `scene`-pack verbs (21 names), in contract order. Every other verb is callable by name and listed
/// once its pack loads (`api.load`). The last three are the defined TOOL_
/// consts that inbox row EB-2026-10-08-S178 (E37) registered: `game.build`,
/// `sim.build_encounter`, `test.run`.
const EXPECTED_TOOLS: [&str; 21] = [
    "scene.read",
    "tx.preview",
    "tx.apply",
    "tx.undo",
    "play.run",
    "capture",
    "api.search",
    "api.describe",
    "api.call",
    "api.load",
    "scene.brief",
    "scene.spatial",
    "scene.place",
    "level.build",
    "scene.query",
    "scene.watch",
    "scene.next",
    "scene.pick",
    "game.build",
    "sim.build_encounter",
    "test.run",
];
/// Verbs of the `view` pack: listed only after `api.load`. 2026-10-19: SP62's
/// `view.observe` and `view.laws` joined the pack, so the list is six now.
const VIEW_PACK: [&str; 6] = [
    "log.digest",
    "log.query",
    "rpc.discover",
    "look.judge",
    "view.observe",
    "view.laws",
];
/// Fixed node id for the scripted edit (a fresh server serves an empty doc).
const NODE_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
/// Nil id: "no parent" on parent fields.
const NIL_ID: &str = "00000000-0000-0000-0000-000000000000";
/// Per-request client context for the 2026-07-28 inline lifecycle: the
/// `initialize` handshake is pre-2026-07-28 only, so every request carries
/// this `_meta` (protocol version plus client capabilities) in its params.
const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{"name":"e2040-mcp5-test","version":"0.0.0"}}"#;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_e2040-mcp")
}

/// Fresh scratch project per test (parallel-safe: unique names).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "e2040-mcp5-stdio-{n}-{name}",
        n = std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch project creates");
    dir
}

/// One outside client: a child binary plus its piped stdio.
struct StdioClient {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    next_id: u64,
}

impl StdioClient {
    /// Spawn the binary on a project folder with piped stdio.
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
        let reply = read_line(&self.stdout);
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
        assert!(
            reply.get("error").is_none(),
            "server answers ok, got: {reply}"
        );
        reply["result"].clone()
    }

    /// One tool call: `tools/call` with a name plus raw arguments JSON.
    /// Every call carries the 2026-07-28 client `_meta` in its params.
    fn call(&mut self, tool: &str, arguments: &str) -> Value {
        let params = format!("{{{META},\"name\":\"{tool}\",\"arguments\":{arguments}}}");
        let result = self.send("tools/call", &params);
        assert_ne!(
            result.get("isError"),
            Some(&Value::Bool(true)),
            "tool `{tool}` succeeds, got: {result}"
        );
        result
    }

    /// Close stdin and wait for the server to exit on EOF. True on a clean exit.
    fn close_and_wait(mut self) -> bool {
        drop(self.stdin);
        for _ in 0..200 {
            match self.child.try_wait().expect("exit status polls") {
                Some(status) => return status.success(),
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        let _ = self.child.kill();
        false
    }
}

/// Read one stdout line on a helper thread: bounded, never hangs the suite.
fn read_line(stdout: &Arc<Mutex<BufReader<ChildStdout>>>) -> Value {
    let (tx, rx) = mpsc::channel();
    let stdout = Arc::clone(stdout);
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = stdout
            .lock()
            .expect("reader lock holds")
            .read_line(&mut line);
        let _ = tx.send(read.map(|_| line));
    });
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(line)) => {
            serde_json::from_str(&line).expect("server replies with one JSON object per line")
        }
        Ok(Err(error)) => panic!("server stdout read failed: {error}"),
        Err(_) => panic!("timed out waiting for the server reply"),
    }
}

/// Tool result payload: `structuredContent` when the server sends it, else the
/// JSON text of the first content block.
fn structured(result: &Value) -> Value {
    if let Some(payload) = result.get("structuredContent") {
        return payload.clone();
    }
    let text = result["content"][0]["text"]
        .as_str()
        .expect("tool answers JSON text");
    serde_json::from_str(text).expect("tool text parses as JSON")
}

/// Tool names the door currently lists, sorted (the wire sorts; the contract
/// set is what matters).
fn listed_tools(client: &mut StdioClient) -> Vec<String> {
    let list = client.send("tools/list", &format!("{{{META}}}"));
    let mut names: Vec<String> = list["tools"]
        .as_array()
        .expect("tool list carries tools")
        .iter()
        .map(|tool| {
            tool["name"]
                .as_str()
                .expect("tool carries a name")
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

/// The scripted edit: one insert op under the nil root.
fn insert_op(id: &str) -> String {
    format!(
        "{{\"op\":\"insert\",\"id\":\"{id}\",\"parent\":\"{NIL_ID}\",\
         \"local\":{{\"pos\":[1.0,0.0,0.0],\"rot\":[0.0,0.0,0.0,1.0],\
         \"scale\":[1.0,1.0,1.0]}}}}"
    )
}

#[test]
fn scripted_client_lists_previews_applies_and_undoes_over_stdio() {
    let dir = scratch("roundtrip");
    let mut client = StdioClient::spawn(&dir);

    // Discovery first: the 2026-07-28 inline lifecycle opens with
    // `server/discover`, not `initialize`. The door must advertise exactly
    // the core tools plus the default pack.
    let discovered = client.send("server/discover", &format!("{{{META}}}"));
    let supported: Vec<&str> = discovered["supportedVersions"]
        .as_array()
        .expect("discover carries supported versions")
        .iter()
        .map(|version| version.as_str().expect("version is text"))
        .collect();
    assert!(
        supported.contains(&"2026-07-28"),
        "door serves 2026-07-28: {supported:?}"
    );
    assert!(
        discovered["capabilities"]["tools"].is_object(),
        "door carries the tools capability: {discovered}"
    );
    assert_eq!(
        discovered["_meta"]["io.modelcontextprotocol/serverInfo"]["name"].as_str(),
        Some("engine-endpoint")
    );
    assert!(
        discovered["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("tx.preview")),
        "door carries usage instructions: {discovered}"
    );

    // Discovery: exactly the core plus the scene pack, so no
    // execute/shell/file-write door. (The wire sorts names; the contract set
    // is what matters.)
    let names = listed_tools(&mut client);
    let mut expected: Vec<String> = EXPECTED_TOOLS.iter().map(ToString::to_string).collect();
    expected.sort();
    assert_eq!(names, expected);

    // Packs: a verb outside the list still answers by exact name, through
    // api.call, and gets listed once its pack loads.
    let digest = structured(&client.call("log.digest", "{}"));
    assert!(
        digest["events"].is_u64(),
        "unlisted verb answers by name: {digest}"
    );
    let called = structured(&client.call("api.call", r#"{"verb":"log.digest","args":{}}"#));
    assert_eq!(called["pack"].as_str(), Some("view"));
    assert_eq!(called["result"]["events"], digest["events"]);
    let loaded = structured(&client.call("api.load", r#"{"pack":"view"}"#));
    assert_eq!(loaded["loaded"].as_bool(), Some(true));
    let with_view = listed_tools(&mut client);
    for verb in VIEW_PACK {
        assert!(
            with_view.contains(&verb.to_owned()),
            "api.load lists `{verb}`"
        );
    }
    assert_eq!(with_view.len(), expected.len() + VIEW_PACK.len());
    let found = structured(&client.call("api.search", r#"{"query":"pack","limit":32}"#));
    let packs = found["hits"]
        .as_array()
        .expect("search answers hits")
        .iter()
        .filter(|hit| {
            hit["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("pack:"))
        })
        .count();
    assert!(packs >= 6, "api.search finds the packs: {found}");

    // Read: the fresh project is empty at revision zero.
    let read = structured(&client.call("scene.read", "{}"));
    assert_eq!(read["revision"].as_u64(), Some(0));
    assert_eq!(read["total"].as_u64(), Some(0));

    // MCP3 briefly: brief and spatial both read the staged scene.
    let brief = structured(&client.call("scene.brief", "{}"));
    assert_eq!(brief["revision"].as_u64(), Some(0));
    assert!(
        brief["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("digest ")),
        "brief carries digest text: {brief}"
    );
    let spatial = structured(&client.call("scene.spatial", "{}"));
    assert_eq!(spatial["verdict"].as_str(), Some("clean"));
    assert_eq!(spatial["total"].as_u64(), Some(0));

    // Preview: dry run diffs with no side effects, minting the approval.
    let op = insert_op(NODE_ID);
    let preview_args =
        format!("{{\"expected_revision\":0,\"idempotency_key\":4242,\"ops\":[{op}]}}");
    let preview = structured(&client.call("tx.preview", &preview_args));
    assert_eq!(preview["resulting_revision"].as_u64(), Some(1));
    assert_eq!(
        preview["approval"]["action_hash"], preview["action_hash"],
        "approval binds the exact batch"
    );
    assert_eq!(preview["effects"].as_array().map(Vec::len), Some(1));
    let approval = preview["approval"].to_string();

    // Apply: the previewed batch lands exactly one revision.
    let apply_args =
        format!("{{\"expected_revision\":0,\"idempotency_key\":4242,\"ops\":[{op}],\"approval\":{approval}}}");
    let applied = structured(&client.call("tx.apply", &apply_args));
    assert_eq!(applied["revision"].as_u64(), Some(1));
    assert_eq!(
        applied["key"].as_str(),
        Some("4242"),
        "ids go out as strings"
    );
    assert_eq!(applied["effects"], preview["effects"]);

    // The node reads back; MCP2 briefly: one tiny headless match runs clean.
    let read = structured(&client.call("scene.read", "{}"));
    assert_eq!(read["revision"].as_u64(), Some(1));
    assert_eq!(read["total"].as_u64(), Some(1));
    assert_eq!(read["objects"][0]["id"].as_str(), Some(NODE_ID));
    // SP11 briefly: scene.query answers over the door on the staged scene.
    let queried = structured(&client.call(
        "scene.query",
        &format!("{{\"kind\":\"near\",\"anchor\":\"{NODE_ID}\",\"radius_cm\":500}}"),
    ));
    assert_eq!(queried["kind"].as_str(), Some("near"));
    assert!(
        queried["digest"].is_u64(),
        "query carries its digest: {queried}"
    );
    let started = structured(&client.call(
        "play.run",
        r#"{"action":"start","seed":2040,"ticks":4,"fighters":2,"half_extent_m":20.0,"scene_rev":1}"#,
    ));
    assert_eq!(started["status"].as_str(), Some("running"));
    let job = started["job_id"].as_u64().expect("start returns a job id");
    let progress = structured(&client.call(
        "play.run",
        &format!("{{\"action\":\"poll\",\"job_id\":{job}}}"),
    ));
    assert_eq!(progress["status"].as_str(), Some("running"));
    let done = structured(&client.call(
        "play.run",
        &format!("{{\"action\":\"poll\",\"job_id\":{job}}}"),
    ));
    assert_eq!(done["status"].as_str(), Some("completed"));
    assert_eq!(done["checkpoints"].as_array().map(Vec::len), Some(4));

    // Undo: the edit reverts as a new revision, restoring the empty scene.
    let undone = structured(&client.call("tx.undo", "{\"undo_token\":4242}"));
    assert_eq!(undone["revision"].as_u64(), Some(2));
    assert_eq!(
        undone["undone_key"].as_str(),
        Some("4242"),
        "ids go out as strings"
    );
    let back = structured(&client.call("scene.read", "{}"));
    assert_eq!(back["revision"].as_u64(), Some(2));
    assert_eq!(back["total"].as_u64(), Some(0));

    assert!(client.close_and_wait(), "server exits cleanly on stdin EOF");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn physics_read_answers_equal_bytes_twice_over_stdio() {
    let dir = scratch("physics-read");
    let mut client = StdioClient::spawn(&dir);

    // The physics pack loads on demand: a fresh door lists no `physics.read`,
    // the verb answers by exact name anyway, and `api.load` lists it.
    let fresh = listed_tools(&mut client);
    assert!(
        !fresh.contains(&"physics.read".to_owned()),
        "the physics pack waits for api.load: {fresh:?}"
    );
    let loaded = structured(&client.call("api.load", r#"{"pack":"physics"}"#));
    assert_eq!(loaded["loaded"].as_bool(), Some(true));
    assert_eq!(loaded["verbs"][0].as_str(), Some("physics.read"));
    assert!(
        listed_tools(&mut client).contains(&"physics.read".to_owned()),
        "api.load lists the physics verb"
    );

    // One request, twice: the outside client reads the same bytes both times.
    let args = r#"{"stagger_line_q8":512,"blow_strength_q8":512}"#;
    let first = structured(&client.call("physics.read", args));
    let second = structured(&client.call("physics.read", args));
    assert_eq!(first, second, "the same call answers the same bytes twice");
    assert_eq!(first["verdict"].as_str(), Some("Staggered"));
    assert_eq!(first["recovery_ticks"].as_u64(), Some(30));
    assert_eq!(first["stumble_q8"].as_u64(), Some(640));
    assert_eq!(first["fall_q8"].as_u64(), Some(896));
    let matrix = first["matrix"].as_array().expect("five rows");
    assert_eq!(matrix.len(), 5, "one row per standard strength");
    assert_eq!(matrix[3]["verdict"].as_str(), Some("Staggered"));
    assert_eq!(matrix[4]["verdict"].as_str(), Some("Stumbling"));
    // The live facts the door cannot answer stay unwired, never zero.
    for fact in [
        "per_slot_balance",
        "bone_velocities",
        "bone_contacts",
        "joint_drift",
    ] {
        assert_eq!(
            first["live_bout"][fact].as_str(),
            Some("not_wired"),
            "`{fact}` answers not_wired"
        );
    }

    assert!(client.close_and_wait(), "server exits cleanly on stdin EOF");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_rejects_bad_dirs_and_flags_with_nonzero_exits() {
    let missing = std::env::temp_dir().join(format!("e2040-mcp5-nope-{n}", n = std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let out = Command::new(bin())
        .arg("--project-dir")
        .arg(&missing)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("missing-dir run exits");
    assert!(!out.status.success(), "a missing project dir must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bad project dir"),
        "typed bad-dir error, got: {stderr}"
    );

    let dir = scratch("file-is-dir");
    let file = dir.join("note.txt");
    std::fs::write(&file, b"not a directory").expect("note writes");
    let out = Command::new(bin())
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("file-dir run exits");
    assert!(!out.status.success(), "a file project dir must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bad project dir"),
        "typed bad-dir error, got: {stderr}"
    );

    let out = Command::new(bin())
        .arg("--frobnicate")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("bad-flag run exits");
    assert!(!out.status.success(), "an unknown flag must fail");
    assert_eq!(out.status.code(), Some(2), "usage errors exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("usage error"),
        "typed usage error, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
