//! stub: loopback HTTP stub servers for adapter tests. Real TCP roundtrips
//! against 127.0.0.1 with recorded hits, request paths, request bodies and
//! auth-header presence, kept separate from mock receipts. Tests only.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Loopback stub: serves canned responses, records what arrived.
pub struct StubServer {
    port: u16,
    hits: Arc<Mutex<u64>>,
    auth_hits: Arc<Mutex<u64>>,
    paths: Arc<Mutex<Vec<String>>>,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl StubServer {
    /// 200 completion with `body`, no delay.
    pub fn spawn_completion(body: String) -> Self {
        Self::spawn(200, body, 0, String::new())
    }

    /// 200 completion with `body` after `delay_ms` (timeout tests).
    pub fn spawn_delayed(body: String, delay_ms: u64) -> Self {
        Self::spawn(200, body, delay_ms, String::new())
    }

    /// Arbitrary status with `body`, no delay (auth/quota/malformed tests).
    pub fn spawn_raw(status: u16, body: String) -> Self {
        Self::spawn(status, body, 0, String::new())
    }

    /// 301 with a `Location` header (redirect-refusal tests).
    pub fn spawn_redirect(location: &str) -> Self {
        Self::spawn(301, String::new(), 0, format!("Location: {location}\r\n"))
    }

    /// The i-th request gets `script[i]` (status, body); past the end the
    /// last entry repeats. For tests that need a different answer per rung.
    pub fn spawn_script(script: Vec<(u16, String)>) -> Self {
        Self::spawn_with(script, 0, String::new())
    }

    /// [`StubServer::spawn_script`] with `headers` (each line ending in CR LF)
    /// on every response, e.g. `Retry-After: 1`.
    pub fn spawn_script_headers(script: Vec<(u16, String)>, headers: &str) -> Self {
        Self::spawn_with(script, 0, headers.to_owned())
    }

    /// Full endpoint URL for this stub.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.port)
    }

    /// Completed request count.
    pub fn hits(&self) -> u64 {
        self.hits.lock().map(|guard| *guard).unwrap_or(0)
    }

    /// Requests carrying an `Authorization` header.
    pub fn auth_hits(&self) -> u64 {
        self.auth_hits.lock().map(|guard| *guard).unwrap_or(0)
    }

    /// Request paths in arrival order.
    pub fn paths(&self) -> Vec<String> {
        self.paths
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Request bodies, as received, in arrival order.
    pub fn bodies(&self) -> Vec<String> {
        self.bodies
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn spawn(status: u16, body: String, delay_ms: u64, extra_headers: String) -> Self {
        Self::spawn_with(vec![(status, body)], delay_ms, extra_headers)
    }

    fn spawn_with(script: Vec<(u16, String)>, delay_ms: u64, extra_headers: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("stub binds loopback");
        listener
            .set_nonblocking(true)
            .expect("stub listener nonblocking");
        let port = listener.local_addr().expect("stub addr").port();
        let hits = Arc::new(Mutex::new(0u64));
        let auth_hits = Arc::new(Mutex::new(0u64));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let worker_bodies = Arc::clone(&bodies);
        let worker_hits = Arc::clone(&hits);
        let worker_auth = Arc::clone(&auth_hits);
        let worker_paths = Arc::clone(&paths);
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut served = 0u32;
            let mut answered = 0usize;
            while served < 32 && Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        served += 1;
                        // Accepted streams inherit the listener's nonblocking
                        // flag: put this one back to blocking so the two
                        // client writes (head, then body) are read whole.
                        // A nonblocking stream returns WouldBlock when the
                        // body has not arrived yet, which truncated bodies
                        // to "" under parallel load and panicked the
                        // body-capture assert.
                        let _ = stream.set_nonblocking(false);
                        // Read first, record second, answer last: the client
                        // unblocks on the response bytes, so counters must be
                        // incremented before they are written.
                        if let Some((stream, path, authorized, sent)) = read_request(stream) {
                            if let Ok(mut guard) = worker_hits.lock() {
                                *guard += 1;
                            }
                            if authorized {
                                if let Ok(mut guard) = worker_auth.lock() {
                                    *guard += 1;
                                }
                            }
                            if let Ok(mut guard) = worker_paths.lock() {
                                guard.push(path);
                            }
                            if let Ok(mut guard) = worker_bodies.lock() {
                                guard.push(sent);
                            }
                            let pick = answered.min(script.len().saturating_sub(1));
                            answered += 1;
                            if let Some((status, body)) = script.get(pick) {
                                let reason = reason_for(*status);
                                answer(stream, delay_ms, *status, reason, body, &extra_headers);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            hits,
            auth_hits,
            paths,
            bodies,
        }
    }
}

/// The reason phrase for the statuses the stubs serve.
fn reason_for(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        400 => "Bad Request",
        401 => "Unauthorized",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        _ => "Error",
    }
}

/// Read one request and hand back the open stream plus the request path,
/// whether an `Authorization` header arrived and the request body. The caller
/// records first and answers after, so counters settle before the client
/// unblocks.
fn read_request(
    mut stream: std::net::TcpStream,
) -> Option<(std::net::TcpStream, String, bool, String)> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        if raw.len() > 16_384 {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(count) => {
                raw.extend_from_slice(&chunk[..count]);
                if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index;
                }
            }
            Err(_) => return None,
        }
    };
    let head = std::str::from_utf8(&raw[..header_end]).ok()?;
    let mut lines = head.split("\r\n");
    let path = lines
        .next()?
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_owned();
    let mut authorized = false;
    let mut content_length = 0usize;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("authorization:") {
            authorized = true;
        }
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse::<usize>().unwrap_or(0);
        }
    }
    let mut pending = raw.split_off(header_end + 4);
    while pending.len() < content_length {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => pending.extend_from_slice(&chunk[..count]),
            Err(_) => break,
        }
    }
    let sent = String::from_utf8_lossy(&pending).into_owned();
    Some((stream, path, authorized, sent))
}

/// Sleep `delay_ms` (timeout tests observe a slow server), then write the
/// canned response and close.
fn answer(
    mut stream: std::net::TcpStream,
    delay_ms: u64,
    status: u16,
    reason: &str,
    body: &str,
    extra_headers: &str,
) {
    if delay_ms > 0 {
        thread::sleep(Duration::from_millis(delay_ms));
    }
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Bind-and-drop a loopback port so dials deterministically fail. Tests use
/// this to prove refusals happen with no (or no further) socket attempts.
pub fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

/// Retry a loopback exchange while it reports a transport setup failure.
///
/// Under full-workspace parallel load the loopback socket setup itself can
/// fail (ephemeral-port pressure, refused dial) before any byte moves, so an
/// exchange can come back `Unavailable` without ever testing the mapping
/// under proof. Retrying only while the outcome says setup-never-completed,
/// bounded, preserves the proof: a genuine mapping regression settles the
/// same wrong error on every attempt and still fails the caller's assertion
/// below. Test-only; product code paths are untouched.
#[cfg(test)]
pub fn settle<T>(mut attempt: impl FnMut(u32) -> T, complete: impl Fn(&T) -> bool) -> T {
    const TRIES: u32 = 8;
    let mut out = attempt(0);
    for n in 1..TRIES {
        if complete(&out) {
            break;
        }
        out = attempt(n);
    }
    out
}
/// Build an OpenAI-compatible chat-completions body whose message content is
/// `inner_json` (itself a strict proposal object). `usage` is attached only
/// when given, so suites can prove unknown stays unknown.
pub fn chat_ok(inner_json: &str, usage: Option<(u64, u64)>) -> String {
    let mut envelope = serde_json::json!({
        "id": "stub-1",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": inner_json},
            "finish_reason": "stop",
        }],
    });
    if let Some((input, output)) = usage {
        envelope["usage"] =
            serde_json::json!({"prompt_tokens": input, "completion_tokens": output});
    }
    envelope.to_string()
}

#[cfg(test)]
mod stub_caller_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    fn post_once(url: &str, body: &str, auth: bool) -> String {
        let addr = url
            .strip_prefix("http://")
            .expect("stub url is http")
            .split('/')
            .next()
            .expect("stub url has host")
            .to_owned();
        let mut stream = TcpStream::connect(addr).expect("stub accepts loopback");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .expect("write timeout");
        let auth_line = if auth {
            "Authorization: Bearer t\r\n"
        } else {
            ""
        };
        let request = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\n{auth_line}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .expect("write stub request");
        let mut out = Vec::new();
        stream.read_to_end(&mut out).expect("read stub response");
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn completion_serves_body_and_records_caller_view() {
        let body = "{\"ok\":true}".to_owned();
        let stub = StubServer::spawn_completion(body.clone());
        assert_eq!(stub.hits(), 0, "no request before the caller dials");
        assert!(stub.url().starts_with("http://127.0.0.1:"));
        assert!(stub.url().ends_with("/v1/chat/completions"));
        let sent = "{\"prompt\":\"hi\"}";
        let response = post_once(&stub.url(), sent, false);
        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "caller sees 200 OK: {response}"
        );
        assert!(
            response.ends_with(&body),
            "caller sees the canned body: {response}"
        );
        assert_eq!(stub.hits(), 1, "one dial counts once");
        assert_eq!(stub.auth_hits(), 0, "no auth header means no auth hit");
        assert_eq!(stub.paths(), vec!["/v1/chat/completions".to_owned()]);
        assert_eq!(stub.bodies(), vec![sent.to_owned()]);
    }

    #[test]
    fn raw_status_counts_auth_hit() {
        let stub = StubServer::spawn_raw(401, "{}".to_owned());
        let response = post_once(&stub.url(), "{}", true);
        assert!(
            response.starts_with("HTTP/1.1 401 Unauthorized"),
            "caller sees the raw status: {response}"
        );
        assert_eq!(stub.hits(), 1);
        assert_eq!(stub.auth_hits(), 1, "auth header counts once");
    }

    #[test]
    fn script_serves_each_rung_then_repeats_last() {
        let stub =
            StubServer::spawn_script(vec![(200, "first".to_owned()), (429, "second".to_owned())]);
        let first = post_once(&stub.url(), "a", false);
        let second = post_once(&stub.url(), "b", false);
        let third = post_once(&stub.url(), "c", false);
        assert!(first.ends_with("first"), "{first}");
        assert!(second.ends_with("second"), "{second}");
        assert!(
            third.ends_with("second"),
            "past the end the last entry repeats: {third}"
        );
        assert_eq!(stub.hits(), 3);
        assert_eq!(
            stub.bodies(),
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
    }

    #[test]
    fn chat_ok_envelope_carries_inner_and_usage() {
        let inner = "{\"actions\":[]}";
        let with_usage = chat_ok(inner, Some((3, 2)));
        let value: serde_json::Value = serde_json::from_str(&with_usage).expect("envelope is json");
        assert_eq!(value["choices"][0]["message"]["content"], inner);
        assert_eq!(value["usage"]["prompt_tokens"], 3);
        assert_eq!(value["usage"]["completion_tokens"], 2);
        let without_usage = chat_ok(inner, None);
        let plain: serde_json::Value =
            serde_json::from_str(&without_usage).expect("envelope is json");
        assert_eq!(plain["choices"][0]["message"]["content"], inner);
        assert!(plain.get("usage").is_none(), "no usage stays missing");
    }

    #[test]
    fn closed_port_refuses_loopback_dial() {
        let port = closed_port();
        assert!(port > 0, "closed port is a real port number");
        let addr: std::net::SocketAddr =
            format!("127.0.0.1:{port}").parse().expect("loopback addr");
        let dial = TcpStream::connect_timeout(&addr, Duration::from_millis(500));
        assert!(dial.is_err(), "dropped listener refuses the dial");
    }
}
