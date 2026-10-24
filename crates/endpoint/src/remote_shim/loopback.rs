//! Opt-in loopback inspector: a read-only TCP door on 127.0.0.1.
//!
//! The in-process shim needs no socket and stays the default. A human or a
//! second tool can watch a live door over TCP only when the host asks for it:
//! [`LoopbackInspector::start`] binds `127.0.0.1:0` (an ephemeral port, never a
//! caller-supplied address, so a non-loopback bind is unrepresentable), prints
//! the port once, and mints a per-run bearer token that every request must
//! carry. Only pack verbs whose advertised annotations say read-only run; every
//! other name is refused by name, so no write, apply or undo can cross the
//! socket. One JSON object per line, at most [`MAX_FRAME_BYTES`] bytes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmcp::serde::Serialize;
use rmcp::serde_json::{self, Value};

use crate::mcp::{tool_names, McpServer};
use crate::packs::PackRegistry;

/// Largest request line accepted: one JSON object plus its newline.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// How long a quiet connection may hold a handler thread.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Pause between non-blocking accept attempts while waiting for a client.
const ACCEPT_PAUSE: Duration = Duration::from_millis(10);

/// A running inspector. Dropping it stops the accept loop and joins it.
pub struct LoopbackInspector {
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LoopbackInspector {
    /// Bind the loopback listener; the host prints [`Self::announce_line`] once.
    ///
    /// Opt-in: nothing binds until this call. There is no address parameter on
    /// purpose, so the listener can only ever bind `127.0.0.1` on an ephemeral
    /// port; `addr().ip().is_loopback()` is true by construction and checked
    /// again after the bind.
    pub fn start(server: Arc<McpServer>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        if !addr.ip().is_loopback() {
            return Err(std::io::Error::other(
                "inspector refused a non-loopback bind",
            ));
        }
        let token = run_token();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = thread::Builder::new()
            .name("e2040-inspector".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                let token = token.clone();
                move || accept_loop(&listener, &server, &token, &stop)
            })?;
        Ok(Self {
            addr,
            token,
            stop,
            worker: Some(worker),
        })
    }

    /// Address the listener bound: always loopback, port allocated by the OS.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The bearer token every request must carry, so two runs never share one.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The one line the host prints when the listener starts: address plus
    /// token. The library never prints, so the announcement happens exactly
    /// once, where the host decides to show it.
    #[must_use]
    pub fn announce_line(&self) -> String {
        format!(
            "e2040 inspector on tcp://{} (read-only, bearer token {})",
            self.addr, self.token
        )
    }
}

impl Drop for LoopbackInspector {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Wait for clients; one handler thread per connection, so two reads overlap.
fn accept_loop(listener: &TcpListener, server: &Arc<McpServer>, token: &str, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() {
                    // Unreachable behind a loopback bind; fail closed anyway.
                    drop(stream);
                    continue;
                }
                let server = Arc::clone(server);
                let token = token.to_owned();
                let spawned = thread::Builder::new()
                    .name("e2040-inspector-client".to_owned())
                    .spawn(move || {
                        let _ = serve(&server, stream, &token);
                    });
                if spawned.is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_PAUSE);
            }
            Err(_) => break,
        }
    }
}

/// Answer every line on one connection until it closes or fails the gate.
fn serve(server: &McpServer, stream: TcpStream, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    loop {
        match read_frame(&mut reader)? {
            Frame::Eof => return Ok(()),
            Frame::TooLarge => {
                let reply = Reply::refused("", "frame_too_large", &frame_limit_message());
                send(&mut writer, &reply)?;
                return Ok(());
            }
            Frame::Line(line) => {
                let (reply, close) = answer(server, &line, token);
                send(&mut writer, &reply)?;
                if close {
                    return Ok(());
                }
            }
        }
    }
}

/// One framed request line.
enum Frame {
    /// One line without its newline.
    Line(Vec<u8>),
    /// The line outgrew [`MAX_FRAME_BYTES`].
    TooLarge,
    /// The client closed its half.
    Eof,
}

/// Read one line, refusing a line longer than the frame cap.
fn read_frame(reader: &mut BufReader<TcpStream>) -> std::io::Result<Frame> {
    let limit = u64::try_from(MAX_FRAME_BYTES + 1).unwrap_or(u64::MAX);
    let mut line = Vec::new();
    let read = reader.by_ref().take(limit).read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(Frame::Eof);
    }
    if line.len() > MAX_FRAME_BYTES {
        return Ok(Frame::TooLarge);
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(Frame::Line(line))
}

/// One request: the run token, a verb name, and that verb's own JSON args.
#[derive(Debug, rmcp::serde::Deserialize)]
#[serde(crate = "rmcp::serde")]
struct Request {
    #[serde(default)]
    token: String,
    #[serde(default)]
    verb: String,
    #[serde(default)]
    args: Option<Value>,
}

/// One answered request, or one refusal that names its code and verb.
#[derive(Debug, Serialize)]
#[serde(crate = "rmcp::serde")]
struct Reply {
    ok: bool,
    verb: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl Reply {
    /// A served read: the verb's own JSON value, plus its revision when the
    /// answer carries one.
    fn served(verb: &str, value: Value) -> Self {
        let revision = value.get("revision").and_then(Value::as_u64);
        Self {
            ok: true,
            verb: verb.to_owned(),
            revision,
            result: Some(value),
            code: None,
            message: None,
        }
    }

    /// A refusal: never a value, always a code a model can branch on.
    fn refused(verb: &str, code: &'static str, message: &str) -> Self {
        Self {
            ok: false,
            verb: verb.to_owned(),
            revision: None,
            result: None,
            code: Some(code),
            message: Some(message.to_owned()),
        }
    }

    /// A verb that ran and refused on its own terms (bad args, bad state).
    fn failed(verb: &str, error: &crate::mcp::McpToolError) -> Self {
        Self {
            ok: false,
            verb: verb.to_owned(),
            revision: None,
            result: None,
            code: Some(error.code),
            message: Some(error.message.clone()),
        }
    }
}

/// Gate one line: token, read-only allowlist, then the verb itself.
fn answer(server: &McpServer, line: &[u8], token: &str) -> (Reply, bool) {
    let Ok(request) = serde_json::from_slice::<Request>(line) else {
        return (
            Reply::refused(
                "",
                "bad_frame",
                "each line is one JSON object: {\"token\",\"verb\",\"args\"}",
            ),
            false,
        );
    };
    if !token_matches(token, &request.token) {
        return (
            Reply::refused(
                &request.verb,
                "bad_token",
                "this listener needs the per-run token it printed when it started",
            ),
            true,
        );
    }
    let registry = PackRegistry::shipped();
    let Some(spec) = registry.verb(&request.verb) else {
        if tool_names().contains(&request.verb.as_str()) {
            return (
                Reply::refused(
                    &request.verb,
                    "write_refused",
                    &format!(
                        "`{}` is a door tool outside the read-only pack verbs this listener serves",
                        request.verb
                    ),
                ),
                false,
            );
        }
        return (
            Reply::refused(
                &request.verb,
                "unknown_tool",
                &format!("no pack serves `{}`", request.verb),
            ),
            false,
        );
    };
    if !spec.readonly() {
        return (
            Reply::refused(
                &request.verb,
                "write_refused",
                &format!(
                    "`{}` writes; this listener serves read-only verbs",
                    request.verb
                ),
            ),
            false,
        );
    }
    let args = request
        .args
        .unwrap_or_else(|| Value::Object(rmcp::model::JsonObject::new()));
    match registry.invoke(server, &request.verb, args) {
        Ok(value) => (Reply::served(&request.verb, value), false),
        Err(error) => (Reply::failed(&request.verb, &error), false),
    }
}

/// Write one reply line and flush it.
fn send(writer: &mut TcpStream, reply: &Reply) -> std::io::Result<()> {
    let mut text = serde_json::to_string(reply).map_err(std::io::Error::other)?;
    text.push('\n');
    writer.write_all(text.as_bytes())?;
    writer.flush()
}

/// Message naming the frame cap.
fn frame_limit_message() -> String {
    format!("one JSON object per line, at most {MAX_FRAME_BYTES} bytes")
}

/// A fresh per-run token: two OS-seeded SipHash keys, printed as 32 hex chars.
///
/// `RandomState` is seeded by the standard library from the OS, so no
/// dependency and no invented cipher: the token is a bearer capability that
/// only this run's printed line reveals.
pub(super) fn run_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    fn draw(seed: u64) -> u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(seed);
        hasher.finish()
    }
    format!(
        "{:016x}{:016x}",
        draw(0xE204_0010_0000_0001),
        draw(0xE204_0010_0000_0002)
    )
}

/// Compare the run token without an early exit on the first differing byte.
pub(super) fn token_matches(expected: &str, given: &str) -> bool {
    let difference = expected.bytes().zip(given.bytes()).fold(
        u8::from(expected.len() != given.len()),
        |acc, (want, got)| acc | (want ^ got),
    );
    difference == 0 && expected.len() == given.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_tools::SceneBriefTool;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use std::io::Read;

    fn server() -> Arc<McpServer> {
        Arc::new(McpServer::new(7, Capability::unrestricted(7)))
    }

    fn connect(inspector: &LoopbackInspector) -> TcpStream {
        let stream = TcpStream::connect(inspector.addr()).expect("connect to the inspector");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        stream
    }

    fn ask(stream: &mut TcpStream, token: &str, verb: &str, args: Value) -> Value {
        let request = serde_json::json!({"token": token, "verb": verb, "args": args});
        let mut text = serde_json::to_string(&request).expect("request json");
        text.push('\n');
        stream.write_all(text.as_bytes()).expect("write request");
        stream.flush().expect("flush request");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read reply");
        serde_json::from_str(&line).expect("reply is json")
    }

    /// F2P: two clients read the same scene revision at once, and a write is
    /// refused by name. There is no listener at all without `start`.
    #[test]
    fn loopback_serves_two_readers_at_once_and_refuses_a_write_by_name() {
        let server = server();
        let inspector = LoopbackInspector::start(Arc::clone(&server)).expect("inspector starts");
        assert!(inspector.addr().ip().is_loopback(), "loopback only");
        assert_eq!(inspector.addr().ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(inspector.addr().port(), 0, "the OS picked a real port");
        assert_eq!(inspector.token().len(), 32, "128 bits of run token");
        let token = inspector.token().to_owned();

        // Two live clients, both connected before either asks.
        let mut first = connect(&inspector);
        let mut second = connect(&inspector);
        let a = ask(&mut first, &token, "scene.brief", serde_json::json!({}));
        let b = ask(&mut second, &token, "scene.brief", serde_json::json!({}));
        assert_eq!(a["ok"], true, "{a}");
        assert_eq!(b["ok"], true, "{b}");
        let door = SceneBriefTool::invoke(&server, Default::default()).expect("door brief");
        assert_eq!(a["revision"], serde_json::json!(door.revision));
        assert_eq!(b["revision"], a["revision"], "{b}");

        // A write verb is refused by name, and the scene is untouched.
        let refused = ask(
            &mut first,
            &token,
            "scene.place",
            serde_json::json!({"expected_revision": 0, "idempotency_key": 1, "object": "x", "wants": []}),
        );
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "write_refused", "{refused}");
        assert!(refused["verb"] == "scene.place", "{refused}");
        assert!(
            refused["message"]
                .as_str()
                .expect("message")
                .contains("scene.place"),
            "the refusal names the verb: {refused}"
        );

        // A core write and an unknown name refuse differently, both by name.
        let core = ask(&mut first, &token, "tx.apply", serde_json::json!({}));
        assert_eq!(core["code"], "write_refused", "{core}");
        let unknown = ask(&mut first, &token, "no.such.verb", serde_json::json!({}));
        assert_eq!(unknown["code"], "unknown_tool", "{unknown}");

        // The door's own document never moved: a different brief budget is a
        // different read, so the repeat guard does not answer for us.
        let after = SceneBriefTool::invoke(
            &server,
            crate::scene_tools::SceneBriefArgs {
                budget_bytes: Some(4096),
                ..Default::default()
            },
        )
        .expect("door brief");
        assert_eq!(after.revision, door.revision, "no write crossed the socket");
    }

    /// Every request needs this run's token, and a wrong one closes the line.
    #[test]
    fn loopback_requires_the_run_token_on_every_request() {
        let server = server();
        let inspector = LoopbackInspector::start(Arc::clone(&server)).expect("inspector starts");
        let token = inspector.token().to_owned();
        let mut stream = connect(&inspector);
        let refused = ask(
            &mut stream,
            "00000000000000000000000000000000",
            "scene.brief",
            serde_json::json!({}),
        );
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "bad_token", "{refused}");
        assert!(
            refused.get("result").is_none(),
            "no value under a bad token"
        );
        let mut rest = Vec::new();
        let read = stream.read_to_end(&mut rest).expect("server closed");
        assert_eq!(read, 0, "the handler closes after a bad token");
        let mut fresh = connect(&inspector);
        let served = ask(&mut fresh, &token, "scene.brief", serde_json::json!({}));
        assert_eq!(served["ok"], true, "{served}");
    }

    /// A line over the frame cap is refused and the connection closes.
    #[test]
    fn loopback_refuses_a_frame_over_the_cap() {
        let server = server();
        let inspector = LoopbackInspector::start(Arc::clone(&server)).expect("inspector starts");
        let mut stream = connect(&inspector);
        let oversized = vec![b'x'; MAX_FRAME_BYTES + 8];
        stream.write_all(&oversized).expect("write oversized");
        stream.flush().expect("flush oversized");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read refusal");
        let reply: Value = serde_json::from_str(&line).expect("refusal json");
        assert_eq!(reply["code"], "frame_too_large", "{reply}");
    }

    /// Each run mints its own token and its own ephemeral port.
    #[test]
    fn loopback_port_and_token_are_per_run() {
        let first = LoopbackInspector::start(server()).expect("first inspector");
        let second = LoopbackInspector::start(server()).expect("second inspector");
        assert_ne!(first.token(), second.token(), "tokens are per run");
        assert_ne!(first.addr(), second.addr(), "ports are ephemeral");
        assert!(second.addr().ip().is_loopback());
        // One stable line, so the host prints the ephemeral port exactly once.
        let line = first.announce_line();
        assert_eq!(
            line,
            first.announce_line(),
            "the announcement never changes"
        );
        assert!(line.contains(&first.addr().to_string()), "{line}");
        assert!(line.contains(first.token()), "{line}");
        assert!(line.contains("read-only"), "{line}");
    }
}
