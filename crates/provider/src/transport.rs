//! transport: minimal std-only HTTP/1.1 POST client plus the chat codec the
//! provider adapters share. The client speaks to configured endpoints
//! only (loopback for local, session-authorized hosts for remote), never
//! follows redirects, and never carries secrets in its errors: failures name
//! a kind and a status, never headers or bodies.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::pins::RequestPins;
use super::protocol::{AiError, AiRequest, Proposal, Usage};

/// An expired socket deadline: Windows reports `TimedOut`, Unix `WouldBlock`.
fn timed_out(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

/// Default path for OpenAI-compatible chat completions (llama.cpp server).
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
/// Largest response body accepted; bounds memory on hostile input.
const MAX_BODY_BYTES: usize = 65_536;
/// Largest raw response (headers plus body) accepted before giving up.
const MAX_RAW_BYTES: usize = 73_728;
const MAX_URL_LEN: usize = 512;

/// Parsed `http://host[:port]/path` endpoint, plus `https://` when the crate
/// `tls` feature is on. Without `tls` this slice owns no TLS, so `https://`
/// URLs are refused and a production remote over TLS needs an operator-run
/// terminator in front of the configured host.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Endpoint {
    host: String,
    port: u16,
    path: String,
    tls: bool,
}

impl Endpoint {
    /// Endpoint host as configured (never resolved eagerly).
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Endpoint port (`80` when the URL carries none).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Request path (defaults to [`CHAT_COMPLETIONS_PATH`]).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// True for `https://` endpoints: the client wraps the TCP stream in a
    /// rustls session. Always false without the crate `tls` feature.
    pub fn tls(&self) -> bool {
        self.tls
    }
}

/// Why an endpoint string was rejected. Carries no secrets by construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointError {
    /// Only `http://` URLs are accepted; TLS lives outside this slice.
    Scheme,
    /// Host missing or over 253 bytes.
    Host,
    /// Port missing, non-numeric or out of range.
    Port,
    /// URL or path over its bound.
    TooLong,
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            EndpointError::Scheme => scheme_error(),
            EndpointError::Host => "endpoint host is missing or too long",
            EndpointError::Port => "endpoint port is invalid",
            EndpointError::TooLong => "endpoint URL or path is too long",
        };
        f.write_str(text)
    }
}

/// Scheme refusal text: names the accepted schemes, so an `https://` URL on
/// a no-`tls` build tells the operator which feature unlocks it.
fn scheme_error() -> &'static str {
    #[cfg(feature = "tls")]
    {
        "endpoint must be an http:// or https:// URL"
    }
    #[cfg(not(feature = "tls"))]
    {
        "endpoint must be an http:// URL (https:// needs the provider `tls` feature)"
    }
}

/// Parse `http://host[:port][/path]`, plus `https://` when the crate `tls`
/// feature is on (default port 443 there, 80 for plain HTTP). Anything else
/// is refused without a socket attempt.
pub fn parse_endpoint(url: &str) -> Result<Endpoint, EndpointError> {
    if url.len() > MAX_URL_LEN {
        return Err(EndpointError::TooLong);
    }
    let (rest, tls) = match url.strip_prefix("http://") {
        Some(rest) => (rest, false),
        None => match url.strip_prefix("https://") {
            // Without `tls` there is no handshake to offer: refuse before
            // any socket attempt and name the missing feature.
            #[cfg(not(feature = "tls"))]
            Some(_) => return Err(EndpointError::Scheme),
            #[cfg(feature = "tls")]
            Some(rest) => (rest, true),
            None => return Err(EndpointError::Scheme),
        },
    };
    let default_port = if tls { 443 } else { 80 };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, CHAT_COMPLETIONS_PATH),
    };
    if authority.is_empty() || authority.len() > 253 {
        return Err(EndpointError::Host);
    }
    let (host, port) = split_authority(authority, default_port)?;
    if host.is_empty() || host.len() > 253 {
        return Err(EndpointError::Host);
    }
    if path.len() > 256 || !path.starts_with('/') {
        return Err(EndpointError::TooLong);
    }
    Ok(Endpoint {
        host: host.to_owned(),
        port,
        path: path.to_owned(),
        tls,
    })
}

fn split_authority(authority: &str, default_port: u16) -> Result<(&str, u16), EndpointError> {
    // Bracketed IPv6 (`[::1]` or `[::1]:port`).
    if let Some(inner) = authority.strip_prefix('[') {
        let end = inner.find(']').ok_or(EndpointError::Host)?;
        let host = &authority[..end + 1];
        let tail = &inner[end + 1..];
        if tail.is_empty() {
            return Ok((host, default_port));
        }
        let digits = tail.strip_prefix(':').ok_or(EndpointError::Port)?;
        return digits
            .parse::<u16>()
            .map(|port| (host, port))
            .map_err(|_| EndpointError::Port);
    }
    match authority.rsplit_once(':') {
        // A bare `::1` has no brackets and no port: treat multi-colon hosts
        // as literals rather than host/port splits.
        Some((host, digits)) if !host.contains(':') => {
            if host.is_empty() {
                return Err(EndpointError::Host);
            }
            let port = digits.parse::<u16>().map_err(|_| EndpointError::Port)?;
            Ok((host, port))
        }
        _ => Ok((authority, default_port)),
    }
}

/// True for loopback hosts only: `localhost`, the `127.0.0.0/8` range and
/// `::1` (bracketed or bare). The local adapter refuses anything else.
pub fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]"
        || host.starts_with("127.")
}

/// Transport failure. Variants carry a kind and at most a status code: never
/// auth headers, request bodies or response text, so formatting one cannot
/// leak a secret.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransportError {
    /// TCP connect failed (closed port, unknown host, refused route).
    Connect,
    /// Connect, read or write exceeded the request budget.
    Timeout,
    /// Write to the socket failed.
    Write,
    /// Read from the socket failed mid-response.
    Read,
    /// Response is not parseable HTTP/1.1.
    Protocol,
    /// Response or body claims an unacceptable encoding.
    BadEncoding,
    /// Body over [`MAX_BODY_BYTES`].
    TooLarge,
    /// 3xx status: redirects and endpoint changes are refused per policy and
    /// never followed.
    Redirect,
    /// Non-200, non-redirect status; the code travels in `Status`.
    Status(u16),
    /// TLS setup or handshake failed (crate `tls` feature only). Like every
    /// variant this names a kind only: never the certificate, the hostname
    /// proof or alert bytes, so formatting one cannot leak a secret.
    Tls,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Connect => f.write_str("endpoint unreachable"),
            TransportError::Timeout => f.write_str("request budget exhausted"),
            TransportError::Write => f.write_str("request write failed"),
            TransportError::Read => f.write_str("response read failed"),
            TransportError::Protocol => f.write_str("response is not HTTP/1.1"),
            TransportError::BadEncoding => f.write_str("response encoding rejected"),
            TransportError::TooLarge => f.write_str("response over its bound"),
            TransportError::Redirect => f.write_str("redirect refused by policy"),
            TransportError::Status(code) => write!(f, "server status {code}"),
            TransportError::Tls => f.write_str("TLS handshake failed"),
        }
    }
}

/// Completed HTTP exchange: status plus the body text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TransportResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body text.
    pub body: String,
}

/// Socket under one request: plain TCP by default, a rustls session over the
/// same TCP stream for `https://` endpoints (crate `tls` feature only). One
/// type keeps the request/response code shared: timeouts are set on the TCP
/// socket before the wrap, so the handshake and the body share the request
/// budget and `TimedOut`/`WouldBlock` still mean budget exhaustion.
enum TransportStream {
    /// Plain `http://` socket.
    Plain(TcpStream),
    /// `https://` socket: the rustls handshake runs lazily on first I/O.
    /// Boxed: the session is a kilobyte next to an 8-byte socket, and one
    /// heap alloc per request is noise off the sim tick.
    #[cfg(feature = "tls")]
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for TransportStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            TransportStream::Plain(stream) => stream.read(buf),
            #[cfg(feature = "tls")]
            TransportStream::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for TransportStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            TransportStream::Plain(stream) => stream.write(buf),
            #[cfg(feature = "tls")]
            TransportStream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            TransportStream::Plain(stream) => stream.flush(),
            #[cfg(feature = "tls")]
            TransportStream::Tls(stream) => stream.flush(),
        }
    }
}

/// Client config for `https://` endpoints: explicit ring provider (already in
/// the graph, no C++ toolchain) plus the platform-native roots, so operator
/// trust — enterprise proxies included — decides what verifies. Built per
/// request: the native store read is millisecond-scale and never on the sim
/// tick, so there is no shared cache to poison across `SSL_CERT_FILE`
/// overrides in tests.
#[cfg(feature = "tls")]
fn tls_config() -> Result<std::sync::Arc<rustls::ClientConfig>, TransportError> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let native = rustls_native_certs::load_native_certs();
    if native.certs.is_empty() {
        return Err(TransportError::Tls);
    }
    let mut roots = rustls::RootCertStore::empty();
    for cert in native.certs {
        roots.add(cert).map_err(|_| TransportError::Tls)?;
    }
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::ALL_VERSIONS)
        .map_err(|_| TransportError::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(std::sync::Arc::new(config))
}

/// Server name for the handshake: IP literals (brackets stripped) verify as
/// IPs, everything else as DNS. A failure is [`TransportError::Tls`]: the
/// endpoint parsed, only the handshake name is unusable.
#[cfg(feature = "tls")]
fn server_name(host: &str) -> Result<rustls::pki_types::ServerName<'static>, TransportError> {
    let bare = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(address) = bare.parse::<std::net::IpAddr>() {
        return Ok(rustls::pki_types::ServerName::IpAddress(address.into()));
    }
    rustls::pki_types::ServerName::try_from(bare)
        .map(|name| name.to_owned())
        .map_err(|_| TransportError::Tls)
}

/// Wrap the connected socket: plain TCP for `http://`, a rustls session for
/// `https://`. Timeouts are already on the socket, so the lazy handshake
/// shares the request budget.
fn wrap_stream(stream: TcpStream, endpoint: &Endpoint) -> Result<TransportStream, TransportError> {
    if !endpoint.tls {
        return Ok(TransportStream::Plain(stream));
    }
    #[cfg(feature = "tls")]
    {
        let name = server_name(&endpoint.host)?;
        let connection =
            rustls::ClientConnection::new(tls_config()?, name).map_err(|_| TransportError::Tls)?;
        Ok(TransportStream::Tls(Box::new(rustls::StreamOwned::new(
            connection, stream,
        ))))
    }
    #[cfg(not(feature = "tls"))]
    {
        // Unreachable: `tls` endpoints cannot parse without the feature.
        Ok(TransportStream::Plain(stream))
    }
}

/// POST `body` as JSON to the endpoint and return the status plus body. The
/// optional bearer token travels in the `Authorization` header only; it is
/// never stored in an error or a log. No redirect is ever followed.
pub fn post_json(
    endpoint: &Endpoint,
    body: &str,
    bearer: Option<&str>,
    timeout_ms: u64,
) -> Result<TransportResponse, TransportError> {
    let timeout = Duration::from_millis(timeout_ms.clamp(1, 30_000));
    let address = format!("{}:{}", endpoint.host, endpoint.port)
        .to_socket_addrs()
        .map_err(|_| TransportError::Connect)?
        .next()
        .ok_or(TransportError::Connect)?;
    let stream = TcpStream::connect_timeout(&address, timeout).map_err(|error| {
        if timed_out(&error) {
            TransportError::Timeout
        } else {
            TransportError::Connect
        }
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| TransportError::Connect)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| TransportError::Connect)?;
    let mut stream = wrap_stream(stream, endpoint)?;
    // The header block briefly holds the bearer token in memory; it is
    // written once and never copied into errors, events or diagnostics.
    let mut head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        endpoint.path,
        endpoint.host,
        body.len()
    );
    if let Some(token) = bearer {
        head.push_str("Authorization: Bearer ");
        head.push_str(token);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let write_error = |error: std::io::Error| {
        if timed_out(&error) {
            TransportError::Timeout
        } else {
            TransportError::Write
        }
    };
    stream.write_all(head.as_bytes()).map_err(write_error)?;
    stream.write_all(body.as_bytes()).map_err(write_error)?;
    let (status, text) = read_response(&mut stream, timeout)?;
    Ok(TransportResponse { status, body: text })
}

fn read_response(
    stream: &mut TransportStream,
    timeout: Duration,
) -> Result<(u16, String), TransportError> {
    let _ = timeout;
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if raw.len() > MAX_RAW_BYTES {
            return Err(TransportError::TooLarge);
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(TransportError::Protocol),
            Ok(count) => {
                raw.extend_from_slice(&chunk[..count]);
                if let Some(index) = find_header_end(&raw) {
                    break index;
                }
            }
            Err(error) if timed_out(&error) => {
                return Err(TransportError::Timeout);
            }
            Err(_) => return Err(TransportError::Read),
        }
    };
    let head = std::str::from_utf8(&raw[..header_end]).map_err(|_| TransportError::BadEncoding)?;
    let mut lines = head.split("\r\n");
    let status = parse_status(lines.next().ok_or(TransportError::Protocol)?)?;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = raw.split_off(header_end + 4);
    match content_length {
        Some(length) => {
            if length > MAX_BODY_BYTES {
                return Err(TransportError::TooLarge);
            }
            while body.len() < length {
                match stream.read(&mut chunk) {
                    Ok(0) => return Err(TransportError::Protocol),
                    Ok(count) => body.extend_from_slice(&chunk[..count]),
                    Err(error) if timed_out(&error) => {
                        return Err(TransportError::Timeout);
                    }
                    Err(_) => return Err(TransportError::Read),
                }
                if body.len() > MAX_RAW_BYTES {
                    return Err(TransportError::TooLarge);
                }
            }
            body.truncate(length);
        }
        None => {
            // No length declared: read until close (stub and
            // `Connection: close` servers). Read timeouts bound this loop.
            loop {
                if body.len() > MAX_BODY_BYTES {
                    return Err(TransportError::TooLarge);
                }
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => body.extend_from_slice(&chunk[..count]),
                    Err(error) if timed_out(&error) => {
                        return Err(TransportError::Timeout);
                    }
                    Err(_) => return Err(TransportError::Read),
                }
            }
        }
    }
    if (300..400).contains(&status) {
        // Endpoint change without session authorization: refuse, never follow.
        return Err(TransportError::Redirect);
    }
    let text = String::from_utf8(body).map_err(|_| TransportError::BadEncoding)?;
    Ok((status, text))
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_status(line: &str) -> Result<u16, TransportError> {
    let mut parts = line.split_whitespace();
    let version = parts.next().ok_or(TransportError::Protocol)?;
    if !version.starts_with("HTTP/") {
        return Err(TransportError::Protocol);
    }
    let code = parts.next().ok_or(TransportError::Protocol)?;
    let status = code.parse::<u16>().map_err(|_| TransportError::Protocol)?;
    if !(100..=599).contains(&status) {
        return Err(TransportError::Protocol);
    }
    Ok(status)
}

/// Map a completed non-200 status to its contract error. Auth and quota
/// refusals are [`AiError::QuotaAuth`]; redirects are refused per policy as
/// [`AiError::UnsupportedCapability`]; anything else is a retryable
/// [`AiError::Unavailable`], never a guess.
pub fn map_status(status: u16) -> AiError {
    match status {
        300..=399 => AiError::UnsupportedCapability,
        401 | 403 | 429 => AiError::QuotaAuth,
        408 => AiError::Timeout,
        _ => AiError::Unavailable,
    }
}

/// Build the OpenAI-compatible chat request body for one engine request under
/// `pins`: system and user messages, token bound, temperature, seed and any
/// tools. The prompt carries the task kind, revision and context digests
/// only: context references are digests, never raw prompts or assets, and no
/// secret ever enters the body.
pub fn chat_body(request: &AiRequest, pins: &RequestPins) -> String {
    // Manual object framing over the pinned fields: same keys, same order,
    // same value bytes as the previous object serialization. Keys are static
    // ASCII names, so no key escaping is needed; values keep their own bytes.
    let fields = pins.chat_fields(request);
    let mut out = String::from("{");
    let mut first = true;
    for (key, value) in &fields {
        if !first {
            out.push(',');
        }
        first = false;
        out.push('"');
        out.push_str(key);
        out.push_str("\":");
        out.push_str(&value.to_string());
    }
    out.push('}');
    out
}

/// Extract and strictly validate a proposal from a chat-completions body.
/// The outer envelope must carry `choices[0].message.content` as text, and
/// that text must itself be a strict proposal object: known action shapes
/// only, bounded labels and diagnostics, then [`validate_proposal`]. This is
/// the one strict parser every adapter shares, and it is text parsing, never
/// native structured output: callers must keep `structured_output: false` in
/// their capabilities. `receipt` (a loopback/host label from the caller) is
/// appended to the diagnostics so a roundtrip receipt names where it ran.
/// Usage is recorded only when the server reports it; otherwise it stays
/// unknown, never zero-claimed.
pub fn parse_chat_body(
    body: &str,
    request: &AiRequest,
    receipt: &str,
) -> Result<(Proposal, Usage), AiError> {
    super::vendor::parse_envelope(&super::vendor::CHAT_ENVELOPE, body, request, receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_parses_host_port_and_path() {
        let endpoint = parse_endpoint("http://127.0.0.1:8080/v1/chat/completions").unwrap();
        assert_eq!(endpoint.host(), "127.0.0.1");
        assert_eq!(endpoint.port(), 8080);
        assert_eq!(endpoint.path(), "/v1/chat/completions");
    }

    #[test]
    fn endpoint_defaults_port_and_path() {
        let endpoint = parse_endpoint("http://localhost").unwrap();
        assert_eq!(endpoint.port(), 80);
        assert_eq!(endpoint.path(), CHAT_COMPLETIONS_PATH);
    }

    /// Without `tls` there is no handshake to offer: `https://` is refused
    /// before any socket attempt, alongside the other malformed authorities.
    #[cfg(not(feature = "tls"))]
    #[test]
    fn endpoint_rejects_non_http_and_bad_authority() {
        assert_eq!(
            parse_endpoint("https://127.0.0.1:8080/v1/chat/completions"),
            Err(EndpointError::Scheme)
        );
        assert_eq!(parse_endpoint("http://"), Err(EndpointError::Host));
        assert_eq!(
            parse_endpoint("http://127.0.0.1:notaport/x"),
            Err(EndpointError::Port)
        );
        assert_eq!(
            parse_endpoint("http://127.0.0.1:99999/x"),
            Err(EndpointError::Port)
        );
    }

    /// With `tls` the same `https://` URL parses to a TLS endpoint instead:
    /// the scheme alone selects the handshake path, `http://` stays plain.
    #[cfg(feature = "tls")]
    #[test]
    fn endpoint_rejects_non_http_and_bad_authority() {
        assert_eq!(
            parse_endpoint("gopher://127.0.0.1:8080/x"),
            Err(EndpointError::Scheme)
        );
        assert_eq!(parse_endpoint("http://"), Err(EndpointError::Host));
        assert_eq!(
            parse_endpoint("http://127.0.0.1:notaport/x"),
            Err(EndpointError::Port)
        );
        assert_eq!(
            parse_endpoint("http://127.0.0.1:99999/x"),
            Err(EndpointError::Port)
        );
        let plain = parse_endpoint("http://127.0.0.1:8080/v1/chat/completions")
            .expect("parse_endpoint accepts valid loopback URLs");
        assert!(!plain.tls());
        let secure = parse_endpoint("https://127.0.0.1:8080/v1/chat/completions")
            .expect("parse_endpoint accepts valid loopback URLs");
        assert!(secure.tls());
        assert_eq!(secure.host(), "127.0.0.1");
        assert_eq!(secure.port(), 8080);
        assert_eq!(secure.path(), "/v1/chat/completions");
    }

    /// `https://` defaults to 443, `http://` to 80: an explicit port still
    /// wins on either scheme.
    #[cfg(feature = "tls")]
    #[test]
    fn https_defaults_port_and_selects_handshake() {
        let secure = parse_endpoint("https://models.internal/v1/chat/completions")
            .expect("parse_endpoint accepts valid https URLs");
        assert!(secure.tls());
        assert_eq!(secure.port(), 443);
        let plain = parse_endpoint("http://models.internal/v1/chat/completions")
            .expect("parse_endpoint accepts valid http URLs");
        assert!(!plain.tls());
        assert_eq!(plain.port(), 80);
        let custom = parse_endpoint("https://models.internal:8443/x")
            .expect("parse_endpoint accepts valid https URLs");
        assert!(custom.tls());
        assert_eq!(custom.port(), 8443);
    }

    /// Handshake names cover DNS plus IP literals (bracketed or bare), so
    /// loopback `https://` URLs name what they dial.
    #[cfg(feature = "tls")]
    #[test]
    fn server_name_covers_dns_and_ip_literals() {
        use rustls::pki_types::ServerName;
        assert!(matches!(
            server_name("models.internal"),
            Ok(ServerName::DnsName(_))
        ));
        assert!(matches!(
            server_name("127.0.0.1"),
            Ok(ServerName::IpAddress(_))
        ));
        assert!(matches!(server_name("[::1]"), Ok(ServerName::IpAddress(_))));
    }

    /// `https://` against a plain-HTTP loopback stub: no handshake can
    /// complete, and the failure stays a secret-free error — never a panic,
    /// never request bytes in the error text.
    #[cfg(feature = "tls")]
    #[test]
    fn https_stub_handshake_fails_secret_free() {
        let stub = crate::stub::StubServer::spawn_completion(String::from("{}"));
        let url = stub.url().replacen("http://", "https://", 1);
        let endpoint = parse_endpoint(&url).expect("https stub URL parses");
        assert!(endpoint.tls());
        let error =
            post_json(&endpoint, r#"{"model":"probe"}"#, Some("stub-token"), 2_000).unwrap_err();
        let text = format!("{error:?} {error}");
        assert!(!text.contains("stub-token"));
        assert!(!text.contains("probe"));
    }

    /// Full loopback TLS roundtrip (an `https://` client against a TLS stub
    /// with a test CA) needs a cert-minter crate the graph does not carry;
    /// until then the negative stub test above plus scheme selection prove
    /// the path. Kept visible so the gap stays explicit.
    #[cfg(feature = "tls")]
    #[test]
    #[ignore = "needs a loopback TLS stub with a test CA (no cert-minter in the graph)"]
    fn https_loopback_roundtrip() {
        // Would: mint a test CA, serve TLS on 127.0.0.1, post_json over
        // https, and expect 200 plus the stub body.
    }

    #[test]
    fn loopback_covers_localhost_and_ipv4_range() {
        assert!(is_loopback("localhost"));
        assert!(is_loopback("LOCALHOST"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.0.1.9"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("10.0.0.9"));
        assert!(!is_loopback("models.internal"));
        assert!(!is_loopback(""));
    }

    #[test]
    fn status_mapping_names_policy_refusals() {
        assert_eq!(map_status(301), AiError::UnsupportedCapability);
        assert_eq!(map_status(401), AiError::QuotaAuth);
        assert_eq!(map_status(403), AiError::QuotaAuth);
        assert_eq!(map_status(429), AiError::QuotaAuth);
        assert_eq!(map_status(408), AiError::Timeout);
        assert_eq!(map_status(500), AiError::Unavailable);
        assert_eq!(map_status(200), AiError::Unavailable);
    }

    #[test]
    fn chat_body_carries_digests_not_raw_context() {
        let request = crate::mock::fixtures::request(3, 7);
        let body = chat_body(&request, &RequestPins::default());
        assert!(
            body.contains("\"model\":\"mock-1\""),
            "model travels: {body}"
        );
        assert!(
            body.contains("revision=7"),
            "prompt carries revision: {body}"
        );
        assert!(body.contains("digest="), "prompt carries digests: {body}");
        assert!(
            body.contains("\"role\":\"system\"") && body.contains("\"role\":\"user\""),
            "chat roles travel: {body}"
        );
    }

    /// EG17 pinned requests: two builds of one prompt give equal bytes with
    /// temperature 0 and a system prompt, so a recorded answer replays.
    #[test]
    fn pinned_request_body_is_byte_stable() {
        let request = crate::mock::fixtures::request(3, 7);
        let pins = RequestPins::default();
        let first = chat_body(&request, &pins);
        let second = chat_body(&request, &pins);
        assert_eq!(first, second, "same prompt gives same bytes");
        assert!(
            first.contains("\"model\":\"mock-1\""),
            "model travels: {first}"
        );
        assert!(
            first.contains("\"max_tokens\""),
            "token bound travels: {first}"
        );
        assert_eq!(
            first.matches("\"role\":\"user\"").count(),
            1,
            "one user message travels: {first}"
        );
        assert!(
            first.contains("\"temperature\":0.0"),
            "greedy default travels: {first}"
        );
        assert!(
            first.contains("\"role\":\"system\""),
            "system role travels: {first}"
        );
        assert!(
            first.contains("\"seed\":2040"),
            "seed travels where offered: {first}"
        );
        assert!(first.contains("revision=7"), "revision travels: {first}");
        assert!(!first.contains("\"tools\""), "tools only when given");
        // Tool defs travel when given, still byte-stable.
        let tool = crate::pins::ToolDef::new(
            "move",
            "Move one object.",
            crate::ladder::action_schema("noop").expect("noop schema is an object"),
        )
        .expect("tool in bounds");
        let tooled = RequestPins::default()
            .with_tools(vec![tool])
            .expect("tools in bounds");
        let third = chat_body(&request, &tooled);
        assert_eq!(
            third,
            chat_body(&request, &tooled),
            "pinned tools stay byte-stable"
        );
        assert!(
            third.contains("\"name\":\"move\""),
            "tool def travels: {third}"
        );
    }

    /// Escape one inner proposal string as a JSON string value for the chat
    /// envelope used below. Mirrors the envelope escaping: quotes,
    /// backslashes and controls are escaped, nothing else is.
    fn wrap_chat(inner: &str) -> String {
        let mut escaped = String::with_capacity(inner.len() + 2);
        for c in inner.chars() {
            match c {
                '"' => escaped.push_str("\\\""),
                '\\' => escaped.push_str("\\\\"),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                '\t' => escaped.push_str("\\t"),
                c if u32::from(c) < 0x20 => {
                    escaped.push_str(&format!("\\u{:04x}", u32::from(c)));
                }
                c => escaped.push(c),
            }
        }
        format!("{{\"choices\":[{{\"message\":{{\"content\":\"{escaped}\"}}}}]}}")
    }

    #[test]
    fn strict_parse_rejects_unknown_actions() {
        let request = crate::mock::fixtures::request(3, 7);
        let inner = r#"{"actions": [{"teleport": {"object": 1}}], "diagnostics": []}"#;
        let body = wrap_chat(inner);
        assert_eq!(
            parse_chat_body(&body, &request, "local:127.0.0.1"),
            Err(AiError::MalformedOutput)
        );
    }

    #[test]
    fn strict_parse_rejects_non_object_content() {
        let request = crate::mock::fixtures::request(3, 7);
        let body = wrap_chat("[1, 2, 3]");
        assert_eq!(
            parse_chat_body(&body, &request, "local:127.0.0.1"),
            Err(AiError::MalformedOutput)
        );
    }

    #[test]
    fn transport_errors_carry_no_payload() {
        // Variants hold a kind plus at most a status: formatting one cannot
        // leak headers, bodies or tokens by construction.
        let text = format!(
            "{:?} {} {}",
            TransportError::Status(401),
            TransportError::Redirect,
            TransportError::Tls
        );
        assert!(text.contains("401"));
        assert!(!text.contains("Bearer"));
    }
    /// Every adapter shares one strict grammar: what the old lenient parse let
    /// through (a `noop: false`, a coordinate that wraps when narrowed to
    /// `i32`, unknown keys) is malformed output now.
    #[test]
    fn chat_body_parse_is_the_strict_grammar() {
        let request = crate::mock::fixtures::request(3, 7);
        let wrap = wrap_chat;
        for bad in [
            r#"{"actions": [{"noop": false}], "diagnostics": []}"#,
            r#"{"actions": [{"set_position": {"object": 1, "xyz_cm": [4294967296, 0, 0]}}], "diagnostics": []}"#,
            r#"{"actions": [{"noop": true, "extra": 1}], "diagnostics": []}"#,
            r#"{"actions": [], "diagnostics": [], "free_text": "hi"}"#,
        ] {
            assert_eq!(
                parse_chat_body(&wrap(bad), &request, "local:127.0.0.1"),
                Err(AiError::MalformedOutput),
                "{bad}"
            );
        }
        let good = r#"{"actions": [{"noop": true}], "diagnostics": ["ok"]}"#;
        let (proposal, _) =
            parse_chat_body(&wrap(good), &request, "local:127.0.0.1").expect("strict parse");
        assert!(proposal
            .diagnostics
            .contains(&String::from("local:127.0.0.1")));
    }
}
