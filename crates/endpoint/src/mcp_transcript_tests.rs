//! AX26 transcript: one scripted client conversation with the MCP door, as
//! JSON-RPC lines over an in-memory pipe, so the wire shape of the first
//! minute of a session is pinned in the lib suite (the stdio suite in
//! `tests/it/mcp_stdio.rs` spawns the binary; this one needs no process).
//!
//! The script: `initialize`, `tools/list`, a good call (`scene.read`), a call
//! that fails inside the door (`tx.undo` with nothing to undo) and a call to
//! a tool the door does not have. Data owner: [`McpServer`]. Determinism: the
//! document starts empty at revision 0, so every reply below is exact.

use super::mcp::{default_tool_names, McpServer};
use engine_editor::capability::Capability;
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

    /// One request; returns the whole reply object (`result` or `error`).
    async fn request(&mut self, method: &str, params: Value) -> Value {
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

    fn dump(&self) -> String {
        self.log.join("\n")
    }
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
        let (client_io, server_io) = duplex(1 << 16);
        let server = McpServer::new(7, Capability::unrestricted(7));
        let serving = tokio::spawn(async move {
            let running = Box::pin(server.serve(server_io))
                .await
                .expect("server starts");
            running.waiting().await.expect("server ends cleanly");
        });
        let (read, write) = split(client_io);
        let mut wire = Wire {
            write,
            read: BufReader::new(read).lines(),
            next_id: 1,
            log: Vec::new(),
        };

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
