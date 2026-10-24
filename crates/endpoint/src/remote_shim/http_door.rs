//! Opt-in streamable HTTP door on 127.0.0.1: the same server, a real socket.
//!
//! The stdio door stays the default and byte-identical: nothing here binds
//! until a host asks for it. [`HttpDoor::start_on`] takes a loopback address
//! (a non-loopback one is refused), mints a per-run bearer token that every
//! request must carry, bounds the body at [`MAX_BODY_BYTES`] and bounds the
//! read at [`READ_TIMEOUT`], so a stalled client holds a handler thread only
//! for that long. Routes: `GET /tools` lists the door's tool names, and
//! `POST /tx/batch` applies one BRP-shaped batch through the in-process shim
//! (`execute_via_shim`), which keeps the preview/apply/undo gate, the
//! revision check and the idempotency binding. One request per connection,
//! `Connection: close`, JSON in and JSON out.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmcp::serde_json::{self, Value};

use super::loopback::{run_token, token_matches};
use super::BrpRequest;
use crate::mcp::{tool_names, McpServer};

/// Largest request body accepted: one BRP batch, well under a level edit.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// Largest request head accepted (request line plus headers).
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// How long a quiet client may hold a handler thread.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Pause between non-blocking accept attempts while waiting for a client.
const ACCEPT_PAUSE: Duration = Duration::from_millis(10);

/// A running HTTP door. Dropping it stops the accept loop and joins it.
pub struct HttpDoor {
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HttpDoor {
    /// Bind the loopback listener on an ephemeral port.
    pub fn start(server: Arc<McpServer>) -> std::io::Result<Self> {
        Self::start_on(server, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
    }

    /// Bind `addr` on loopback only; any other address is refused before the
    /// bind, so a non-loopback door is unrepresentable.
    pub fn start_on(server: Arc<McpServer>, addr: SocketAddr) -> std::io::Result<Self> {
        if !addr.ip().is_loopback() {
            return Err(std::io::Error::other(
                "http door refused a non-loopback bind",
            ));
        }
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let token = run_token();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = thread::Builder::new()
            .name("e2040-http-door".to_owned())
            .spawn({
                let token = token.clone();
                let stop = Arc::clone(&stop);
                move || accept_loop(&listener, &server, &token, &stop)
            })?;
        Ok(Self {
            addr,
            token,
            stop,
            worker: Some(worker),
        })
    }

    /// The bound loopback address, including the chosen port.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The bearer token every request must carry, so two runs never share one.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The one line a host prints so a client can find port and token.
    pub fn announce_line(&self) -> String {
        format!(
            "e2040 http door on http://{} (bearer token {})",
            self.addr, self.token
        )
    }
}

impl Drop for HttpDoor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Accept one connection at a time, each on its own thread.
fn accept_loop(listener: &TcpListener, server: &Arc<McpServer>, token: &str, stop: &AtomicBool) {
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let server = Arc::clone(server);
                let token = token.to_owned();
                workers.push(thread::spawn(move || {
                    let _ = serve(&server, stream, &token);
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_PAUSE);
            }
            Err(_) => break,
        }
        workers.retain(|worker| !worker.is_finished());
    }
    for worker in workers {
        let _ = worker.join();
    }
}

/// One request head: method, path, headers (lowercased names).
struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Serve one connection: read one request, answer once, close.
fn serve(server: &McpServer, mut stream: TcpStream, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let (head, body) = match read_request(&mut stream) {
        Ok(parts) => parts,
        Err(reply) => return write_reply(&mut stream, reply),
    };
    let reply = route(server, token, &head, &body);
    write_reply(&mut stream, reply)
}

/// One answer: status line code, then the JSON body.
struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    fn json(status: u16, body: String) -> Self {
        Self { status, body }
    }

    fn refused(status: u16, code: &str, message: &str) -> Self {
        let body = serde_json::json!({"ok": false, "code": code, "message": message});
        Self::json(status, body.to_string())
    }
}

/// Read the head plus exactly `content-length` body bytes, under both caps.
fn read_request(stream: &mut TcpStream) -> Result<(Head, Vec<u8>), Reply> {
    let mut head_bytes: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    let split = loop {
        if let Some(at) = find_head_end(&head_bytes) {
            break at;
        }
        if head_bytes.len() > MAX_HEAD_BYTES {
            return Err(Reply::refused(
                431,
                "head_too_large",
                "request head exceeds the door's cap",
            ));
        }
        let read = stream
            .read(&mut chunk)
            .map_err(|_| Reply::refused(408, "read_timeout", "the client stalled mid-request"))?;
        if read == 0 {
            return Err(Reply::refused(
                400,
                "short_request",
                "the connection closed before a full head arrived",
            ));
        }
        head_bytes.extend_from_slice(&chunk[..read]);
    };
    let text = String::from_utf8_lossy(&head_bytes[..split]).to_string();
    let mut rest = head_bytes[split..].to_vec();
    let head = parse_head(&text)?;
    let length = match head.header("content-length") {
        Some(value) => value
            .trim()
            .parse::<usize>()
            .map_err(|_| Reply::refused(400, "bad_length", "content-length is not a byte count"))?,
        None => 0,
    };
    if length > MAX_BODY_BYTES {
        return Err(Reply::refused(
            413,
            "body_too_large",
            "the body exceeds the door's cap",
        ));
    }
    while rest.len() < length {
        let read = stream
            .read(&mut chunk)
            .map_err(|_| Reply::refused(408, "read_timeout", "the body stalled mid-read"))?;
        if read == 0 {
            return Err(Reply::refused(
                400,
                "short_body",
                "the connection closed before the body arrived",
            ));
        }
        rest.extend_from_slice(&chunk[..read]);
    }
    rest.truncate(length);
    Ok((head, rest))
}

/// Offset just past the blank line that ends the head.
fn find_head_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| at + 4)
}

/// Parse the request line plus headers; header names are lowercased.
fn parse_head(text: &str) -> Result<Head, Reply> {
    let mut lines = text.split("\r\n");
    let request = lines.next().unwrap_or_default();
    let mut words = request.split_whitespace();
    let method = words.next().unwrap_or_default().to_owned();
    let path = words.next().unwrap_or_default().to_owned();
    let version = words.next().unwrap_or_default();
    if method.is_empty() || path.is_empty() || !version.starts_with("HTTP/") {
        return Err(Reply::refused(
            400,
            "bad_request_line",
            "expected `METHOD PATH HTTP/1.x`",
        ));
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Reply::refused(400, "bad_header", "a header has no colon"));
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    Ok(Head {
        method,
        path,
        headers,
    })
}

/// The bearer value from `Authorization: Bearer <token>`.
fn bearer(head: &Head) -> Option<&str> {
    head.header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
}

/// Answer one request: token first, then the route.
fn route(server: &McpServer, token: &str, head: &Head, body: &[u8]) -> Reply {
    match bearer(head) {
        Some(given) if token_matches(token, given) => {}
        _ => {
            return Reply::refused(
                401,
                "bad_token",
                "the request carried no valid bearer token for this run",
            )
        }
    }
    match (head.method.as_str(), head.path.as_str()) {
        ("GET", "/tools") => {
            let body = serde_json::json!({"ok": true, "tools": tool_names()});
            Reply::json(200, body.to_string())
        }
        ("POST", "/tx/batch") => match batch(server, body) {
            Ok(value) => Reply::json(200, value.to_string()),
            Err(reply) => reply,
        },
        ("GET", _) | ("POST", _) => {
            Reply::refused(404, "not_found", "known routes: GET /tools, POST /tx/batch")
        }
        _ => Reply::refused(
            405,
            "method_not_allowed",
            "known methods: GET, POST on the loopback door",
        ),
    }
}

/// One batch body: `{"gesture": u64, "batch": [BrpRequest, ..]}`.
#[derive(rmcp::serde::Deserialize)]
#[serde(crate = "rmcp::serde")]
struct BatchBody {
    #[serde(default)]
    gesture: u64,
    #[serde(default)]
    batch: Vec<BrpRequest>,
}

/// Apply one batch through the in-process shim; the door never edits state.
fn batch(server: &McpServer, body: &[u8]) -> Result<Value, Reply> {
    let body: BatchBody = serde_json::from_slice(body)
        .map_err(|_| Reply::refused(400, "bad_body", "expected {gesture, batch} JSON"))?;
    if body.batch.is_empty() {
        return Err(Reply::refused(
            400,
            "empty_batch",
            "a batch carries at least one item",
        ));
    }
    let out = super::execute_via_shim(server, &body.batch, body.gesture);
    Ok(serde_json::json!({
        "ok": out.applied == body.batch.len(),
        "applied": out.applied,
        "items": body.batch.len(),
        "revision": out.revision,
    }))
}

/// Write the answer as one HTTP/1.1 response and close.
fn write_reply(stream: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reply.status,
        reason(reply.status),
        reply.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(reply.body.as_bytes())?;
    stream.flush()
}

/// The canonical reason phrase for the handful of codes this door answers.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;

    /// Send one whole request and read the whole answer.
    fn request(
        addr: SocketAddr,
        token: Option<&str>,
        method: &str,
        path: &str,
        body: &str,
    ) -> String {
        let mut stream = TcpStream::connect(addr).expect("connect the door");
        let auth = token
            .map(|token| format!("authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nhost: {addr}\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).expect("write head");
        stream.write_all(body.as_bytes()).expect("write body");
        stream.flush().expect("flush");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("read reply");
        reply
    }

    fn door() -> (Arc<McpServer>, HttpDoor) {
        let server = Arc::new(McpServer::new(7, Capability::unrestricted(7)));
        let door = HttpDoor::start(Arc::clone(&server)).expect("door binds loopback");
        (server, door)
    }

    #[test]
    fn a_client_lists_tools_and_applies_one_scene_edit_over_http() {
        let (server, door) = door();
        assert!(door.addr().ip().is_loopback(), "loopback by construction");

        let listed = request(door.addr(), Some(door.token()), "GET", "/tools", "");
        assert!(listed.starts_with("HTTP/1.1 200"), "got: {listed}");
        assert!(listed.contains("scene.read"), "the door lists its tools");

        let refused = request(door.addr(), None, "GET", "/tools", "");
        assert!(refused.starts_with("HTTP/1.1 401"), "got: {refused}");

        let body = concat!(
            r#"{"gesture":11,"batch":[{"method":"spawn","expected_revision":0,"idempotency_key":1,"ops":["#,
            r#"{"op":"insert","id":"8f3a2b1c-0000-4000-8000-000000000001","#,
            r#""parent":"00000000-0000-0000-0000-000000000000","#,
            r#""local":{"pos":[0.0,0.0,0.0],"rot":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]}}]}]}"#
        );
        let applied = request(door.addr(), Some(door.token()), "POST", "/tx/batch", body);
        assert!(applied.starts_with("HTTP/1.1 200"), "got: {applied}");
        assert!(
            applied.contains("\"applied\":1"),
            "one edit landed: {applied}"
        );
        let revision = server.lock().expect("lock holds").doc.revision;
        assert_eq!(revision, 1, "the edit moved the document revision");
    }

    #[test]
    fn a_second_run_uses_a_different_token() {
        let (_a_server, first) = door();
        let (_b_server, second) = door();
        assert_ne!(first.token(), second.token(), "per-run tokens");
        assert_ne!(first.addr(), second.addr(), "ephemeral ports");
    }

    #[test]
    fn a_non_loopback_bind_is_refused() {
        let server = Arc::new(McpServer::new(7, Capability::unrestricted(7)));
        let outside = SocketAddr::from((Ipv4Addr::new(192, 0, 2, 1), 0));
        let refused = HttpDoor::start_on(server, outside);
        assert!(refused.is_err(), "only loopback may be served");
    }
}
