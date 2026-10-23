//! AX26 transcript: one scripted client conversation with the MCP door, as
//! JSON-RPC lines over an in-memory pipe, so the wire shape of the first
//! minute of a session is pinned in the lib suite (the stdio suite in
//! `tests/it/mcp_stdio.rs` spawns the binary; this one needs no process).
//!
//! The script: `initialize`, `tools/list`, a good call (`scene.read`), a call
//! that fails inside the door (`tx.undo` with nothing to undo) and a call to
//! a tool the door does not have. Data owner: [`McpServer`]. Determinism: the
//! document starts empty at revision 0, so every reply below is exact.
//!
//! BG1 adds the push tests: a 2026-07-28 client opens inline with
//! `server/discover`, subscribes through `subscriptions/listen` and hears
//! the tool list and the live scene move on its stream; a legacy client
//! subscribes to the scene with `resources/subscribe`.

use super::mcp::{default_tool_names, McpServer};
use engine_editor::capability::Capability;
use engine_scene::ids::ObjectId;
use rmcp::serde_json::{json, Value};
use rmcp::ServiceExt;
use std::time::Duration;
use tokio::io::{
    duplex, split, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf,
    WriteHalf,
};

/// Bounded wait per reply: a silent server fails the test, never hangs it.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// The client end of the pipe: newline-delimited JSON-RPC 2.0.
struct Wire {
    write: WriteHalf<DuplexStream>,
    read: Lines<BufReader<ReadHalf<DuplexStream>>>,
    next_id: u64,
    log: Vec<String>,
    /// True after a `server/discover` opener: a 2026-07-28 session carries
    /// its protocol version and client capabilities in every request `_meta`.
    modern: bool,
}

/// The `_meta` every request of a 2026-07-28 session carries (SEP-2575).
fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "bg1-push", "version": "0"},
    })
}

impl Wire {
    async fn send(&mut self, frame: Value) {
        let line = format!("{frame}\n");
        self.log.push(format!("> {frame}"));
        self.write
            .write_all(line.as_bytes())
            .await
            .expect("request writes");
    }

    /// Attach the modern session metadata when the wire opened that way.
    fn with_meta(&self, mut params: Value) -> Value {
        if self.modern && params.is_object() {
            params["_meta"] = modern_meta();
        }
        params
    }

    /// One request; returns the whole reply object (`result` or `error`).
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let params = self.with_meta(params);
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let next = tokio::time::timeout(REPLY_TIMEOUT, self.read.next_line()).await;
            let line = match next {
                Ok(Ok(Some(line))) => line,
                other => panic!(
                    "no reply to {method} ({other:?}); transcript:\n{}",
                    self.dump()
                ),
            };
            self.log.push(format!("< {line}"));
            let reply: Value = rmcp::serde_json::from_str(&line).expect("one JSON object a line");
            if reply["id"].as_u64() == Some(id) {
                return reply;
            }
        }
    }

    /// One frame, notification or reply, with the same bounded wait.
    async fn read_frame(&mut self) -> Value {
        let next = tokio::time::timeout(REPLY_TIMEOUT, self.read.next_line()).await;
        let line = match next {
            Ok(Ok(Some(line))) => line,
            other => panic!("no frame ({other:?}); transcript:\n{}", self.dump()),
        };
        self.log.push(format!("< {line}"));
        rmcp::serde_json::from_str(&line).expect("one JSON object a line")
    }

    /// One request whose pushed notes matter: the reply plus every
    /// notification the server wrote before it.
    async fn request_notes(&mut self, method: &str, params: Value) -> (Value, Vec<Value>) {
        let params = self.with_meta(params);
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        let mut notes = Vec::new();
        loop {
            let frame = self.read_frame().await;
            if frame["id"].as_u64() == Some(id) {
                return (frame, notes);
            }
            notes.push(frame);
        }
    }

    fn dump(&self) -> String {
        self.log.join("\n")
    }
}

/// Open one door over a fresh pipe and return its client end.
fn open(server: McpServer) -> (Wire, tokio::task::JoinHandle<()>) {
    let (client_io, server_io) = duplex(1 << 16);
    let serving = tokio::spawn(async move {
        let running = Box::pin(server.serve(server_io))
            .await
            .expect("server starts");
        running.waiting().await.expect("server ends cleanly");
    });
    let (read, write) = split(client_io);
    let wire = Wire {
        write,
        read: BufReader::new(read).lines(),
        next_id: 1,
        log: Vec::new(),
        modern: false,
    };
    (wire, serving)
}

/// Open a 2026-07-28 session the inline way: no `initialize` handshake, just
/// `server/discover` with the protocol version in `_meta`. Returns the result.
async fn discover(wire: &mut Wire) -> Value {
    wire.modern = true;
    let found = wire.request("server/discover", json!({})).await;
    assert!(
        found.get("error").is_none(),
        "server/discover:\n{}",
        wire.dump()
    );
    found["result"].clone()
}

/// Initialize at `version` and answer with the result.
async fn handshake(wire: &mut Wire, version: &str) -> Value {
    let init = wire
        .request(
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {"name": "bg1-push", "version": "0"},
            }),
        )
        .await;
    assert!(init.get("error").is_none(), "initialize:\n{}", wire.dump());
    wire.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    init
}

/// Preview then apply one insert at `revision`, returning the notes the apply
/// pushed and the revision it landed at.
async fn insert_one(wire: &mut Wire, revision: u64, key: u64) -> (Vec<Value>, u64) {
    // Each key sits two meters further out, so a second insert never trips
    // the door's staged-overlap check on the first.
    let x = 1.0 + key as f64 * 2.0;
    let op = json!({
        "op": "insert",
        "id": ObjectId::new().to_string(),
        "parent": ObjectId::NIL.to_string(),
        "local": {"pos": [x, 0.0, 0.0], "rot": [0.0, 0.0, 0.0, 1.0], "scale": [1.0, 1.0, 1.0]},
    });
    let (preview, notes) = wire
        .request_notes(
            "tools/call",
            json!({
                "name": "tx.preview",
                "arguments": {"expected_revision": revision, "idempotency_key": key, "ops": [op.clone()]},
            }),
        )
        .await;
    assert!(preview.get("error").is_none(), "preview:\n{}", wire.dump());
    assert!(notes.is_empty(), "a preview moves nothing: {notes:?}");
    let approval = preview["result"]["structuredContent"]["approval"].clone();
    assert!(approval.is_object(), "preview mints an approval: {preview}");
    let (apply, notes) = wire
        .request_notes(
            "tools/call",
            json!({
                "name": "tx.apply",
                "arguments": {
                    "expected_revision": revision,
                    "idempotency_key": key,
                    "ops": [op],
                    "approval": approval,
                },
            }),
        )
        .await;
    assert!(apply.get("error").is_none(), "apply:\n{}", wire.dump());
    let landed = apply["result"]["structuredContent"]["revision"]
        .as_u64()
        .unwrap_or_else(|| panic!("apply names its revision: {apply}"));
    assert_eq!(landed, revision + 1, "the apply landed one revision on");
    (notes, landed)
}

/// The notes of one method in a frame list.
fn notes_of<'a>(notes: &'a [Value], method: &str) -> Vec<&'a Value> {
    notes
        .iter()
        .filter(|note| note["method"] == method)
        .collect()
}

#[test]
fn door_announces_list_change() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds");
    runtime.block_on(async {
        let (mut wire, serving) = open(McpServer::new(7, Capability::unrestricted(7)));

        // 1. A 2026-07-28 client opens inline with `server/discover` and
        //    reads both push flags there.
        let discovered = discover(&mut wire).await;
        assert_eq!(
            discovered["capabilities"]["tools"]["listChanged"],
            json!(true),
            "the door announces tool-list changes: {discovered}"
        );
        assert_eq!(
            discovered["capabilities"]["resources"]["subscribe"],
            json!(true),
            "the door announces resource subscriptions: {discovered}"
        );
        assert!(
            discovered["supportedVersions"]
                .to_string()
                .contains("2026-07-28"),
            "the door serves this session's version: {discovered}"
        );

        // 2. Subscribe to the tool list and the live scene on one stream.
        let listen_id = wire.next_id;
        wire.next_id += 1;
        wire.send(json!({
            "jsonrpc": "2.0",
            "id": listen_id,
            "method": "subscriptions/listen",
            "params": {
                "_meta": modern_meta(),
                "notifications": {
                    "toolsListChanged": true,
                    "resourceSubscriptions": ["e2040://scene"],
                },
            },
        }))
        .await;
        let ack = wire.read_frame().await;
        assert_eq!(
            ack["method"],
            "notifications/subscriptions/acknowledged",
            "the listen stream is acknowledged: {}",
            wire.dump()
        );
        assert_eq!(
            ack["params"]["notifications"]["toolsListChanged"],
            json!(true),
            "the stream keeps the tool-list filter: {ack}"
        );
        assert_eq!(
            ack["params"]["notifications"]["resourceSubscriptions"],
            json!(["e2040://scene"]),
            "the stream keeps the scene filter: {ack}"
        );

        // 3. api.load moves the tool list: exactly one note on the stream.
        let (loaded, notes) = wire
            .request_notes(
                "tools/call",
                json!({"name": "api.load", "arguments": {"pack": "web"}}),
            )
            .await;
        assert!(loaded.get("error").is_none(), "api.load:\n{}", wire.dump());
        let list_notes = notes_of(&notes, "notifications/tools/list_changed");
        assert_eq!(list_notes.len(), 1, "one note per load: {notes:?}");
        assert!(
            notes_of(&notes, "notifications/resources/updated").is_empty(),
            "a load moves no scene: {notes:?}"
        );

        // 4. A landed write moves the scene view: exactly one note.
        let (notes, landed) = insert_one(&mut wire, 0, 1).await;
        let updates = notes_of(&notes, "notifications/resources/updated");
        assert_eq!(updates.len(), 1, "one scene note per write: {notes:?}");
        assert_eq!(
            updates[0]["params"]["uri"],
            json!("e2040://scene"),
            "the note names the live scene: {updates:?}"
        );
        assert!(
            notes_of(&notes, "notifications/tools/list_changed").is_empty(),
            "a write moves no tool list: {notes:?}"
        );

        // 5. The resource carries the moved revision; reading it is silent.
        let (read, notes) = wire
            .request_notes("resources/read", json!({"uri": "e2040://scene"}))
            .await;
        assert!(
            read.get("error").is_none(),
            "resources/read:\n{}",
            wire.dump()
        );
        assert!(notes.is_empty(), "a read announces nothing: {notes:?}");
        let text = read["result"]["contents"][0]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(
            text.contains(&format!("revision: {landed}")),
            "the view carries the landed revision: {text}"
        );
        assert!(
            text.contains("objects: 1"),
            "the view counts the insert: {text}"
        );
        assert!(
            text.contains("digest: "),
            "the view carries a digest: {text}"
        );

        // 6. A 2026-07-28 session that opened no stream hears nothing: the
        //    modern protocol delivers changes on streams, not on the session.
        let (mut bare, bare_serving) = open(McpServer::new(8, Capability::unrestricted(8)));
        discover(&mut bare).await;
        let (loaded, notes) = bare
            .request_notes(
                "tools/call",
                json!({"name": "api.load", "arguments": {"pack": "web"}}),
            )
            .await;
        assert!(loaded.get("error").is_none(), "api.load:\n{}", bare.dump());
        assert!(
            notes.is_empty(),
            "a modern session with no stream hears no list move: {notes:?}"
        );
        let (notes, _) = insert_one(&mut bare, 0, 1).await;
        assert!(
            notes.is_empty(),
            "a modern session with no stream gets no scene note: {notes:?}"
        );

        drop(wire);
        drop(bare);
        tokio::time::timeout(REPLY_TIMEOUT, serving)
            .await
            .expect("server stops when the pipe closes")
            .expect("server task joins");
        tokio::time::timeout(REPLY_TIMEOUT, bare_serving)
            .await
            .expect("bare server stops when the pipe closes")
            .expect("bare server task joins");
    });
}

#[test]
fn legacy_subscription_hears_the_scene() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds");
    runtime.block_on(async {
        let (mut wire, serving) = open(McpServer::new(9, Capability::unrestricted(9)));
        handshake(&mut wire, "2025-06-18").await;

        // A legacy client subscribes by URI; a write answers one update.
        let subscribed = wire
            .request("resources/subscribe", json!({"uri": "e2040://scene"}))
            .await;
        assert!(
            subscribed.get("error").is_none(),
            "resources/subscribe:\n{}",
            wire.dump()
        );
        let (notes, landed) = insert_one(&mut wire, 0, 1).await;
        let updates = notes_of(&notes, "notifications/resources/updated");
        assert_eq!(updates.len(), 1, "one note per write: {notes:?}");
        assert_eq!(updates[0]["params"]["uri"], json!("e2040://scene"));

        // After unsubscribe the next write stays silent.
        let unsubscribed = wire
            .request("resources/unsubscribe", json!({"uri": "e2040://scene"}))
            .await;
        assert!(
            unsubscribed.get("error").is_none(),
            "resources/unsubscribe:\n{}",
            wire.dump()
        );
        let (notes, _) = insert_one(&mut wire, landed, 2).await;
        assert!(
            notes_of(&notes, "notifications/resources/updated").is_empty(),
            "an unsubscribed session gets no scene note: {notes:?}"
        );

        // An unknown URI refuses and names the one the door pushes.
        let refused = wire
            .request("resources/subscribe", json!({"uri": "e2040://nope"}))
            .await;
        assert!(
            refused.get("error").is_some(),
            "an unknown subscription refuses: {refused}"
        );

        drop(wire);
        tokio::time::timeout(REPLY_TIMEOUT, serving)
            .await
            .expect("server stops when the pipe closes")
            .expect("server task joins");
    });
}

/// Text of a tool reply: the structured payload when the server sends one,
/// else the first content block, else the JSON-RPC error.
fn payload(reply: &Value) -> String {
    let result = &reply["result"];
    if let Some(structured) = result.get("structuredContent") {
        return structured.to_string();
    }
    match result["content"][0]["text"].as_str() {
        Some(text) => text.to_owned(),
        None => reply["error"].to_string(),
    }
}

#[test]
fn transcript_initialize_list_call_error_and_unknown_tool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds");
    runtime.block_on(async {
        let (mut wire, serving) = open(McpServer::new(7, Capability::unrestricted(7)));

        // 1. initialize: the door names itself and offers tools.
        let init = wire
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "ax26-transcript", "version": "0"},
                }),
            )
            .await;
        assert!(init.get("error").is_none(), "initialize:\n{}", wire.dump());
        assert_eq!(
            init["result"]["serverInfo"]["name"].as_str(),
            Some("engine-endpoint"),
            "{}",
            wire.dump()
        );
        assert!(
            init["result"]["capabilities"]["tools"].is_object(),
            "{}",
            wire.dump()
        );
        wire.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;

        // 2. tools/list: the core plus the default packs, nothing else.
        let listed = wire.request("tools/list", json!({})).await;
        let mut names: Vec<String> = listed["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list:\n{}", wire.dump()))
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_owned())
            .collect();
        names.sort();
        let mut want: Vec<String> = default_tool_names()
            .iter()
            .map(ToString::to_string)
            .collect();
        want.sort();
        assert_eq!(names, want, "{}", wire.dump());

        // 3. A good call: the empty scene at revision 0.
        let read = wire
            .request("tools/call", json!({"name": "scene.read", "arguments": {}}))
            .await;
        assert!(read.get("error").is_none(), "scene.read:\n{}", wire.dump());
        assert_ne!(read["result"]["isError"], json!(true), "{}", wire.dump());
        assert!(payload(&read).contains("\"revision\":0"), "{}", wire.dump());

        // 4. An error from inside the door: nothing has been applied yet.
        let undo = wire
            .request("tools/call", json!({"name": "tx.undo", "arguments": {}}))
            .await;
        assert!(
            payload(&undo).contains("nothing_to_undo"),
            "tx.undo:\n{}",
            wire.dump()
        );

        // 5. A tool the door does not have: refused by name, no panic.
        let unknown = wire
            .request(
                "tools/call",
                json!({"name": "no.such.tool", "arguments": {}}),
            )
            .await;
        assert!(unknown.get("error").is_some(), "{}", wire.dump());
        assert!(
            unknown.to_string().contains("unknown_tool"),
            "unknown tool:\n{}",
            wire.dump()
        );

        // The conversation ends when the client hangs up.
        drop(wire);
        tokio::time::timeout(REPLY_TIMEOUT, serving)
            .await
            .expect("server stops when the pipe closes")
            .expect("server task joins");
    });
}
