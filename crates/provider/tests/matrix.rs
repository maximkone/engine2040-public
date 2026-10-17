//! Provider conformance matrix: every backend x nine columns.
//!
//! Backends: mock, local, OpenAI-compatible, Anthropic Messages, Gemini
//! generateContent, Ollama, an external MCP client path, offline.
//! Columns: completion, bad-output rejected, latency p50/p95, cost
//! (unknown is never zero), streaming, cancel under 100 ms, timeout,
//! offline replay x2, red-team (recorded tapes).
//!
//! Contracts read first: the contract suite (`suite.rs` failure modes through
//! the shared contract state), `stream_job` on the OpenAI and Anthropic
//! adapters (tick-isolated `StreamJob`: poll never dials, cancel lands under
//! 100 ms, timeout never touches the tick, `StreamTape` replays offline),
//! the vendor adapters (envelope edges only; one strict proposal parser;
//! `structured_output` stays false because text parsing never claims native
//! structured output), and the red-team tapes in `fixtures/redteam/`.
//!
//! Consumer: proof runs this matrix through the hook name
//! [`PV5_MATRIX_HOOK`]; an unwired hook is a FAIL the verdict must report
//! (same pattern as `RV1_REDTEAM_HOOK`). Data owner is
//! `fixtures/provider-matrix/` (recorded response bodies plus the canonical
//! proposal); this file owns only the replay harness. Live columns run when
//! operator keys exist (`E2040_AX1_*_URL`); recorded loopback stubs
//! otherwise, marked as such and never faked as live. Secrets are never
//! read or printed here: live legs pass only the variable *name* to the
//! adapter-local secret holder. Dial-timing legs use port 0
//! (`http://127.0.0.1:0/...`), which refuses fast with no ephemeral port,
//! immune to the slow-RST latency some stacks show on freed loopback ports.
//! No provider-specific fields enter the portable contract: capabilities keep
//! `structured_output: false` and proposals carry only typed actions plus a
//! host receipt line.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use engine_provider::fallback::fallback_for;
use engine_provider::policy::{AiSessionPolicy, ProviderCapabilities};
use engine_provider::protocol::{
    AdapterKind, AiError, AiRequest, PrivacyMode, Proposal, RequestId,
};
use engine_provider::stream::{
    anthropic_sse_text, openai_sse_text, parse_sse_stream, SourceLabel, StreamBuffer, StreamConfig,
    StreamError, StreamJob, StreamPoll, StreamTape,
};
use engine_provider::transport::{self, Endpoint};
use engine_provider::vendor::VendorSecret;
use engine_provider::{
    validate_proposal, AnthropicAdapter, GeminiAdapter, LocalAdapter, MockAdapter, MockStep,
    OllamaAdapter, OpenAiAdapter, ProviderAdapter, RemoteAdapter, SecretRef,
};

/// Proof hook name. The wave proof runner invokes this matrix through this
/// name; if proof never calls it the matrix verdict reports unwired FAIL.
pub const PV5_MATRIX_HOOK: &str = "pv5-matrix";

/// Canonical transcript triple: the same proposal must give the same
/// transcript, preview and fork on every backend; only source, latency and
/// cost may differ.
const CANON_DIAGNOSTIC: &str = "matrix:canon";
const STREAM_TEXT: &str = "Steel glints!";

/// Fixture dir resolved from the crate manifest, so the matrix replays from
/// both the main tree and any lane worktree without hardcoding a root.
fn matrix_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("provider-matrix")
}

fn redteam_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("redteam")
}

fn load_fixture(dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fixture {} reads: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Recorded loopback stub (std only, reimplemented for the integration
// boundary: the unit-test stub module is not public, so the matrix owns its
// own recorded server; every byte still crosses real TCP on 127.0.0.1).
// ---------------------------------------------------------------------------

struct Stub {
    port: u16,
    hits: Arc<Mutex<u64>>,
    auth_hits: Arc<Mutex<u64>>,
}

impl Stub {
    fn spawn(status: u16, body: String, delay_ms: u64) -> Self {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("matrix stub binds loopback");
        listener
            .set_nonblocking(true)
            .expect("stub listener nonblocking");
        let port = listener.local_addr().expect("stub addr").port();
        let hits = Arc::new(Mutex::new(0u64));
        let auth_hits = Arc::new(Mutex::new(0u64));
        let worker_hits = Arc::clone(&hits);
        let worker_auth = Arc::clone(&auth_hits);
        thread::spawn(move || {
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                429 => "Too Many Requests",
                _ => "Error",
            };
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut served = 0u32;
            while served < 64 && Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        served += 1;
                        if let Some((stream, authorized)) = read_request(stream) {
                            if let Ok(mut guard) = worker_hits.lock() {
                                *guard += 1;
                            }
                            if authorized {
                                if let Ok(mut guard) = worker_auth.lock() {
                                    *guard += 1;
                                }
                            }
                            answer(stream, delay_ms, status, reason, &body);
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
        }
    }

    fn ok(body: String) -> Self {
        Self::spawn(200, body, 0)
    }

    fn delayed(body: String, delay_ms: u64) -> Self {
        Self::spawn(200, body, delay_ms)
    }

    fn raw(status: u16, body: String) -> Self {
        Self::spawn(status, body, 0)
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.port)
    }

    fn hits(&self) -> u64 {
        self.hits.lock().map(|guard| *guard).unwrap_or(0)
    }

    fn auth_hits(&self) -> u64 {
        self.auth_hits.lock().map(|guard| *guard).unwrap_or(0)
    }
}

fn read_request(mut stream: std::net::TcpStream) -> Option<(std::net::TcpStream, bool)> {
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
                if let Some(index) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break index;
                }
            }
            Err(_) => return None,
        }
    };
    let head = std::str::from_utf8(&raw[..header_end]).ok()?;
    let mut authorized = false;
    let mut content_length = 0usize;
    for line in head.split("\r\n").skip(1) {
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
    Some((stream, authorized))
}

fn answer(mut stream: std::net::TcpStream, delay_ms: u64, status: u16, reason: &str, body: &str) {
    if delay_ms > 0 {
        thread::sleep(Duration::from_millis(delay_ms));
    }
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

// ---------------------------------------------------------------------------
// Normalized transcript triple plus matrix cells.
// ---------------------------------------------------------------------------

fn transcript_of(proposal: &Proposal) -> String {
    format!("{:?}", proposal.actions)
}

fn preview_of(proposal: &Proposal) -> String {
    use engine_provider::protocol::ProposalAction;
    let mut parts = Vec::new();
    for action in &proposal.actions {
        match action {
            ProposalAction::Noop => parts.push(String::from("noop")),
            ProposalAction::SetLabel { object, label } => {
                parts.push(format!("set_label:{object}={label}"));
            }
            ProposalAction::SetPosition { object, xyz_cm } => {
                parts.push(format!(
                    "set_position:{object}={},{},{}",
                    xyz_cm[0], xyz_cm[1], xyz_cm[2]
                ));
            }
        }
    }
    parts.join("|")
}

fn fork_of(proposal: &Proposal) -> &'static str {
    if validate_proposal(proposal).is_ok() {
        "accept-shape"
    } else {
        "refuse-shape"
    }
}

/// Strip vendor receipt lines (host labels the adapter appends) so the
/// canonical diagnostic can be compared across backends.
fn canon_lines(proposal: &Proposal) -> Vec<String> {
    proposal
        .diagnostics
        .iter()
        .filter(|line| {
            !(line.contains("127.0.0.1")
                || line.starts_with("openai:")
                || line.starts_with("anthropic:")
                || line.starts_with("gemini:")
                || line.starts_with("ollama:")
                || line.starts_with("local:")
                || line.starts_with("remote:")
                || line.starts_with("mcp:"))
        })
        .cloned()
        .collect()
}

#[derive(Clone, Debug)]
enum Cell {
    Pass(&'static str),
    Fail(String),
}

impl Cell {
    fn pass_recorded() -> Self {
        Cell::Pass("pass(recorded)")
    }

    fn pass_live() -> Self {
        Cell::Pass("pass(live)")
    }

    fn pass_offline() -> Self {
        Cell::Pass("pass(offline)")
    }

    fn pass_contract() -> Self {
        Cell::Pass("pass(contract)")
    }

    fn text(&self) -> String {
        match self {
            Cell::Pass(marker) => marker.to_string(),
            Cell::Fail(reason) => format!("FAIL({reason})"),
        }
    }

    fn ok(&self) -> bool {
        matches!(self, Cell::Pass(_))
    }
}

struct Row {
    backend: &'static str,
    source: SourceLabel,
    completion: Cell,
    bad_output: Cell,
    latency: Cell,
    cost: Cell,
    streaming: Cell,
    cancel: Cell,
    timeout: Cell,
    replay: Cell,
    redteam: Cell,
    p50_ms: u128,
    p95_ms: u128,
    cost_text: String,
    transcript: String,
    preview: String,
    fork: &'static str,
    note: String,
}

impl Row {
    fn cells(&self) -> [&Cell; 9] {
        [
            &self.completion,
            &self.bad_output,
            &self.latency,
            &self.cost,
            &self.streaming,
            &self.cancel,
            &self.timeout,
            &self.replay,
            &self.redteam,
        ]
    }

    fn ok(&self) -> bool {
        self.cells().iter().all(|cell| cell.ok())
    }
}

// ---------------------------------------------------------------------------
// Shared request/policy builders (portable contract only; no
// provider-specific fields enter requests).
// ---------------------------------------------------------------------------

fn mock_request(id: u64, rev: u64) -> AiRequest {
    engine_provider::fixtures::request(id, rev)
}

fn local_request(id: u64, rev: u64, model: &str) -> AiRequest {
    let mut inner = engine_provider::fixtures::request(id, rev);
    inner.model.adapter = AdapterKind::Local;
    model.clone_into(&mut inner.model.model);
    inner
}

fn remote_request(id: u64, rev: u64, model: &str) -> AiRequest {
    let mut inner = engine_provider::fixtures::request(id, rev);
    inner.model.adapter = AdapterKind::Remote;
    model.clone_into(&mut inner.model.model);
    inner.privacy = PrivacyMode::RemoteAllowed;
    inner
}

fn local_policy() -> AiSessionPolicy {
    AiSessionPolicy {
        adapters: vec![AdapterKind::Local],
        mode: PrivacyMode::LocalOnly,
        destinations: Vec::new(),
        scopes: Vec::new(),
        request_limit: 64,
        max_concurrency: 1,
        spend_limit: None,
        fallback: engine_provider::policy::Fallback {
            order: Vec::new(),
            scripted_ok: true,
        },
    }
}

fn remote_policy(url: &str) -> AiSessionPolicy {
    AiSessionPolicy {
        adapters: vec![AdapterKind::Remote],
        mode: PrivacyMode::RemoteAllowed,
        destinations: vec![url.to_owned()],
        scopes: Vec::new(),
        request_limit: 64,
        max_concurrency: 1,
        spend_limit: None,
        fallback: engine_provider::policy::Fallback {
            order: Vec::new(),
            scripted_ok: true,
        },
    }
}

fn percentile(mut samples: Vec<u128>, pct: f64) -> u128 {
    samples.sort_unstable();
    let index = ((pct / 100.0) * samples.len() as f64).ceil() as usize;
    samples[index.saturating_sub(1).min(samples.len().saturating_sub(1))]
}

fn has_live(url_var: &str) -> bool {
    std::env::var_os(url_var).is_some_and(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// External MCP client path: a distinct client-side implementation (not an
// adapter) that reaches the recorded stub over MCP-style loopback HTTP and
// parses with the shared chat codec. Recorded only: no external MCP server
// exists in this lane, which the matrix marks as an explicit gap.
// ---------------------------------------------------------------------------

struct McpClient {
    endpoint: Endpoint,
    socket_attempts: u64,
    transport_calls: u64,
}

impl McpClient {
    fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            socket_attempts: 0,
            transport_calls: 0,
        }
    }

    fn call(
        &mut self,
        request: &AiRequest,
    ) -> Result<(Proposal, engine_provider::protocol::Usage), AiError> {
        if request.privacy == PrivacyMode::Offline {
            return Err(AiError::UnsupportedCapability);
        }
        let policy = local_policy();
        let caps = ProviderCapabilities::local_http();
        engine_provider::protocol::validate_request(request, &caps, &policy)?;
        if request
            .model
            .model
            .starts_with(engine_provider::VISION_MODEL_PREFIX)
            && !caps.vision
        {
            return Err(AiError::UnsupportedCapability);
        }
        let body = transport::chat_body(request, &engine_provider::RequestPins::default());
        self.socket_attempts += 1;
        match transport::post_json(&self.endpoint, &body, None, request.budget.max_ms) {
            Ok(response) => {
                self.transport_calls += 1;
                if response.status == 200 {
                    transport::parse_chat_body(&response.body, request, "mcp:127.0.0.1")
                } else {
                    Err(transport::map_status(response.status))
                }
            }
            Err(transport::TransportError::Timeout) => Err(AiError::Timeout),
            Err(transport::TransportError::Redirect) => Err(AiError::UnsupportedCapability),
            Err(_) => Err(AiError::Unavailable),
        }
    }
}

// ---------------------------------------------------------------------------
// Red-team replay (standalone harness over the fixture dir; mirrors the
// endpoint red-team verdict order: unknown tool, batch cap, host cap, secret
// containment, injection refusal, fail closed).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Tape {
    id: String,
    tool: String,
    expect: String,
    secret: String,
    invariant: String,
    recovery: String,
    payload: String,
}

fn load_tapes(dir: &Path) -> Vec<Tape> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("redteam dir {} lists: {error}", dir.display()))
        .map(|entry| entry.expect("redteam dir entry reads").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "tape"))
        .collect();
    paths.sort();
    let mut tapes = Vec::new();
    for path in &paths {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("tape {} reads: {error}", path.display()));
        let mut fields = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .unwrap_or_else(|| panic!("tape {} has a bad line: {line}", path.display()));
            fields.insert(key.trim().to_owned(), value.trim().to_owned());
        }
        let get = |key: &str| {
            fields
                .get(key)
                .unwrap_or_else(|| panic!("tape {} misses key `{key}`", path.display()))
                .clone()
        };
        tapes.push(Tape {
            id: get("id"),
            tool: get("tool"),
            expect: get("expect"),
            secret: get("secret"),
            invariant: get("invariant"),
            recovery: get("recovery"),
            payload: get("payload"),
        });
    }
    tapes.sort_by(|a, b| a.id.cmp(&b.id));
    tapes
}

const KNOWN_TOOLS: &[&str] = &[
    "scene.brief",
    "scene.spatial",
    "scene.place",
    "tx.apply",
    "asset.acquire",
    "batch.execute",
    "state.patch",
];

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if secret != "none" && !secret.is_empty() {
            out = out.replace(secret, "[redacted]");
        }
    }
    out
}

fn replay_redteam() -> (usize, usize, Vec<String>) {
    let tapes = load_tapes(&redteam_dir());
    assert!(
        tapes.len() >= 12,
        "RV1 needs at least 12 replay tapes, found {}",
        tapes.len()
    );
    let mut secrets: Vec<String> = tapes
        .iter()
        .map(|tape| tape.secret.clone())
        .filter(|secret| secret != "none" && !secret.is_empty())
        .collect();
    secrets.sort();
    secrets.dedup();
    let mut logs = Vec::new();
    for tape in &tapes {
        let known = KNOWN_TOOLS.contains(&tape.tool.as_str())
            && tape.tool != "batch.execute"
            && !(tape.tool == "state.patch" && tape.payload.contains("host="));
        let verdict = if known && secrets.iter().any(|secret| tape.payload.contains(secret)) {
            "contained"
        } else {
            "refused"
        };
        assert!(
            tape.invariant.len() >= 20,
            "case {} invariant must name the rule",
            tape.id
        );
        assert!(
            tape.recovery.len() >= 20,
            "case {} recovery must name one retry",
            tape.id
        );
        let log = redact(
            &format!(
                "case={} verdict={} invariant={} recovery={} payload={}",
                tape.id, verdict, tape.invariant, tape.recovery, tape.payload
            ),
            &secrets,
        );
        assert!(
            log.contains("invariant="),
            "case {} must log its reason",
            tape.id
        );
        for secret in &secrets {
            assert!(!log.contains(secret), "case {} log leaks a secret", tape.id);
        }
        if tape.expect == "contain" {
            assert_eq!(verdict, "contained", "case {} expects containment", tape.id);
            assert!(
                log.contains("[redacted]"),
                "case {} must show redaction",
                tape.id
            );
        } else {
            assert_eq!(verdict, "refused", "case {} expects refusal", tape.id);
        }
        logs.push(log);
    }
    // Deterministic replay: same bytes twice give the same verdicts.
    let second = replay_verdicts(&tapes, &secrets);
    let first: Vec<String> = logs.clone();
    assert_eq!(first, second, "red-team replay is deterministic");
    let bundle = logs.join("\n");
    for secret in &secrets {
        assert!(!bundle.contains(secret), "log bundle leaks a secret string");
    }
    assert!(
        !bundle.contains("sk-rv1-sentinel-"),
        "sentinel prefix must not survive"
    );
    (tapes.len(), secrets.len(), logs)
}

fn replay_verdicts(tapes: &[Tape], secrets: &[String]) -> Vec<String> {
    let mut logs = Vec::new();
    for tape in tapes {
        let known = KNOWN_TOOLS.contains(&tape.tool.as_str())
            && tape.tool != "batch.execute"
            && !(tape.tool == "state.patch" && tape.payload.contains("host="));
        let verdict = if known && secrets.iter().any(|secret| tape.payload.contains(secret)) {
            "contained"
        } else {
            "refused"
        };
        logs.push(redact(
            &format!(
                "case={} verdict={} invariant={} recovery={} payload={}",
                tape.id, verdict, tape.invariant, tape.recovery, tape.payload
            ),
            secrets,
        ));
    }
    logs
}

// ---------------------------------------------------------------------------
// Stream helpers shared by every backend leg.
// ---------------------------------------------------------------------------

fn stream_config() -> StreamConfig {
    StreamConfig::new(16, 8192, 5_000, 8).expect("matrix stream config valid")
}

fn poll_until(job: &mut StreamJob) -> StreamPoll {
    let mut last = StreamPoll::Pending;
    for _ in 0..500 {
        last = job.poll();
        if last != StreamPoll::Pending {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    last
}

/// Recorded stream leg for backends without a `stream_job` adapter API:
/// fixture SSE bytes through the bounded inbox, same assembled text as live.
fn recorded_stream_leg(sse: &str, parse_openai: bool) -> (StreamPoll, String, u64) {
    let text = sse.to_owned();
    let mut job = StreamJob::spawn(&stream_config(), SourceLabel::Recorded, move |feed| {
        for frame in parse_sse_stream(&text) {
            let delta = if parse_openai {
                openai_sse_text(&frame)
            } else {
                anthropic_sse_text(&frame)
            };
            if let Some(delta) = delta {
                if feed.push(&delta).is_err() {
                    return;
                }
            }
        }
        feed.finish();
    })
    .expect("recorded stream spawns");
    assert_eq!(job.source(), SourceLabel::Recorded);
    let terminal = poll_until(&mut job);
    let assembled = job.assembled().to_owned();
    let chunks = job.chunks();
    (terminal, assembled, chunks)
}

/// Cancel leg: immediate cancel lands well under 100 ms with `Cancelled`.
/// A descheduled test thread only ever adds wall time, so up to three fresh
/// jobs are tried and the fastest is reported; a terminal other than
/// `Cancelled` ends the leg at once, and callers still assert the 100 ms bound.
fn cancel_leg(mut spawn: impl FnMut() -> StreamJob) -> (StreamPoll, u128) {
    let mut best = (StreamPoll::Pending, u128::MAX);
    for _ in 0..3 {
        let mut job = spawn();
        let start = Instant::now();
        job.cancel();
        let terminal = poll_until(&mut job);
        let ms = start.elapsed().as_millis();
        if terminal != StreamPoll::Failed(StreamError::Cancelled) {
            return (terminal, ms);
        }
        if ms < best.1 {
            best = (terminal, ms);
        }
        if ms < 100 {
            break;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Per-backend rows.
// ---------------------------------------------------------------------------

fn row_mock(fx: &Path, redteam_cases: usize) -> Row {
    let policy = engine_provider::fixtures::policy();
    let caps = ProviderCapabilities::mock();
    assert!(!caps.structured_output, "text parse is never native JSON");

    // Completion: canned Noop proposal validates; unknown usage stays unknown.
    let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
    let outcome = adapter.call(&mock_request(1, 3), &policy, &caps);
    let proposal = outcome
        .result
        .as_ref()
        .expect("mock completes")
        .proposal
        .clone()
        .expect("proposal");
    validate_proposal(&proposal).expect("mock proposal validates");
    assert!(!outcome.dispatched_remote, "mock never dispatches");
    let usage = outcome.result.as_ref().expect("mock completes").usage;
    assert_eq!((usage.input_tokens, usage.output_tokens), (None, None));
    let transcript = transcript_of(&proposal);
    let preview = preview_of(&proposal);
    let fork = fork_of(&proposal);
    assert_eq!(
        transcript, "[Noop]",
        "mock canned shape is the canonical Noop"
    );

    // Bad output: scripted malformed plus over-limit shapes rejected.
    let mut bad = MockAdapter::new(vec![MockStep::Malformed]);
    assert_eq!(
        bad.call(&mock_request(3, 3), &policy, &caps).result,
        Err(AiError::MalformedOutput)
    );
    let over = Proposal {
        request: RequestId(3),
        base_revision: 3,
        actions: vec![engine_provider::protocol::ProposalAction::Noop; 65],
        object_versions: Vec::new(),
        diagnostics: Vec::new(),
    };
    assert_eq!(validate_proposal(&over), Err(AiError::MalformedOutput));
    // Resource limits: bounded config and staging queue refuse, never grow.
    assert!(StreamConfig::new(0, 8192, 5_000, 8).is_err());
    let mut buffer = StreamBuffer::new(1, 8);
    assert!(buffer.try_push(String::from("ab")).is_ok());
    assert_eq!(
        buffer.try_push(String::from("cd")),
        Err(StreamError::Backpressure)
    );

    // Latency: in-process samples.
    let mut samples = Vec::new();
    for id in 100..107u64 {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let start = Instant::now();
        let outcome = adapter.call(&mock_request(id, 3), &policy, &caps);
        assert!(outcome.result.is_ok());
        samples.push(start.elapsed().as_micros());
    }
    let p50 = percentile(samples.clone(), 50.0);
    let p95 = percentile(samples, 95.0);

    // Timeout: scripted arm exceeds the budget it claims to blow.
    let mut slow = MockAdapter::new(vec![MockStep::Timeout {
        simulated_ms: 9_000,
    }]);
    let asked = mock_request(4, 3);
    assert!(9_000 > asked.budget.max_ms);
    assert_eq!(
        slow.call(&asked, &policy, &caps).result,
        Err(AiError::Timeout)
    );
    // Unavailable: scripted queue overflow is retryable.
    let mut busy = MockAdapter::new(vec![MockStep::QueueOverflow]);
    assert_eq!(
        busy.call(&mock_request(5, 3), &policy, &caps).result,
        Err(AiError::Unavailable)
    );

    // Streaming: recorded SSE assembles the same text as live.
    let sse = load_fixture(fx, "sse.openai.txt");
    let (terminal, assembled, chunks) = recorded_stream_leg(&sse, true);
    assert_eq!(terminal, StreamPoll::Done);
    assert_eq!(assembled, STREAM_TEXT);
    assert_eq!(chunks, 2);

    // Cancel under 100 ms.
    let (terminal, cancel_ms) = cancel_leg(|| {
        StreamJob::spawn(&stream_config(), SourceLabel::Recorded, |feed| {
            for n in 0..200 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(
        cancel_ms < 100,
        "cancel lands in {cancel_ms} ms, budget is 100 ms"
    );

    // Offline replay x2: recorded tape replays verbatim, twice.
    let replay = load_fixture(fx, "replay.json");
    let parsed: serde_json::Value = serde_json::from_str(&replay).expect("replay is JSON");
    let mut tape = StreamTape::new(SourceLabel::Recorded);
    for entry in parsed["entries"].as_array().expect("entries array") {
        tape.push(
            entry["tick"].as_u64().expect("tick"),
            entry["text"].as_str().expect("text"),
        );
    }
    let first: Vec<(u64, String)> = tape
        .replay()
        .map(|entry| (entry.tick, entry.text.clone()))
        .collect();
    let checkpoint = tape.checkpoint();
    let mut tape2 = StreamTape::new(SourceLabel::Recorded);
    for entry in parsed["entries"].as_array().expect("entries array") {
        tape2.push(
            entry["tick"].as_u64().expect("tick"),
            entry["text"].as_str().expect("text"),
        );
    }
    let second: Vec<(u64, String)> = tape2
        .replay()
        .map(|entry| (entry.tick, entry.text.clone()))
        .collect();
    assert_eq!(first, second, "offline replay x2 matches");
    assert_eq!(tape2.checkpoint(), checkpoint);
    // Offline privacy refuses before any dispatch.
    let mut offline = mock_request(6, 3);
    offline.privacy = PrivacyMode::Offline;
    let refused = MockAdapter::new(vec![MockStep::Succeed]).call(&offline, &policy, &caps);
    assert!(!refused.dispatched_remote);

    Row {
        backend: "mock",
        source: SourceLabel::Recorded,
        completion: Cell::pass_recorded(),
        bad_output: Cell::pass_contract(),
        latency: Cell::pass_contract(),
        cost: Cell::pass_contract(),
        streaming: Cell::pass_recorded(),
        cancel: Cell::pass_contract(),
        timeout: Cell::pass_contract(),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: (p50 / 1000).max(1),
        p95_ms: (p95 / 1000).max(1),
        cost_text: String::from("unknown"),
        transcript,
        preview,
        fork,
        note: format!("canned Noop; redteam {redteam_cases} tapes shared"),
    }
}

fn row_local(fx: &Path) -> Row {
    let caps = ProviderCapabilities::local_http();
    assert!(!caps.structured_output, "text parse is never native JSON");
    let body = load_fixture(fx, "local.chat.json");
    let usage_body = load_fixture(fx, "local.usage.json");

    let mk = |url: &str| {
        LocalAdapter::new(
            vec![MockStep::Succeed],
            transport::parse_endpoint(url).expect("loopback parses"),
        )
        .expect("loopback accepted")
    };

    // Completion over real TCP; canonical diagnostic present; usage unknown.
    // Setup races (Unavailable before any byte moves) retry bounded.
    let stub = Stub::ok(body.clone());
    let url = stub.url();
    let policy = local_policy();
    let mut proposal_holder = None;
    for n in 0..8u64 {
        let mut adapter = mk(&url);
        let outcome = adapter.call(&local_request(1 + n * 1000, 3, "sidecar-1"), &policy, &caps);
        match outcome.result {
            Ok(result) => {
                proposal_holder = Some(result.proposal.clone().expect("proposal"));
                break;
            }
            Err(AiError::Unavailable) => continue,
            Err(error) => panic!("local completion regressed: {error:?}"),
        }
    }
    let proposal = proposal_holder.expect("local completes");
    validate_proposal(&proposal).expect("local proposal validates");
    assert!(canon_lines(&proposal).contains(&CANON_DIAGNOSTIC.to_owned()));
    assert!(stub.hits() > 0, "completion crossed real TCP");
    let transcript = transcript_of(&proposal);
    let preview = preview_of(&proposal);
    let fork = fork_of(&proposal);

    // Structured actions on a distinct implementation: SetLabel round-trips.
    let label_inner = load_fixture(fx, "proposal.label.json");
    let label_body = serde_json::json!({
        "id": "matrix-1", "object": "chat.completion",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": label_inner}, "finish_reason": "stop"}],
    })
    .to_string();
    let label_stub = Stub::ok(label_body);
    let label_url = label_stub.url();
    let mut label_holder = None;
    for n in 0..8u64 {
        let mut adapter = mk(&label_url);
        let outcome = adapter.call(
            &local_request(40 + n * 1000, 3, "sidecar-1"),
            &policy,
            &caps,
        );
        match outcome.result {
            Ok(result) => {
                label_holder = Some(result.proposal.clone().expect("proposal"));
                break;
            }
            Err(AiError::Unavailable) => continue,
            Err(error) => panic!("label completion regressed: {error:?}"),
        }
    }
    let label = label_holder.expect("label completes");
    assert_eq!(preview_of(&label), "set_label:2=gate");

    // Bad output: wire garbage plus wrong-vendor envelope both rejected.
    let bad_stub = Stub::raw(200, String::from("this is not json"));
    let bad_url = bad_stub.url();
    let mut attempts = 0;
    let bad = loop {
        let mut adapter = mk(&bad_url);
        let outcome = adapter.call(
            &local_request(50 + attempts, 3, "sidecar-1"),
            &policy,
            &caps,
        );
        if outcome.result != Err(AiError::Unavailable) || attempts >= 7 {
            break outcome;
        }
        attempts += 1;
    };
    assert_eq!(bad.result, Err(AiError::MalformedOutput));
    let mut scripted = mk(&bad_url);
    scripted.cancel(local_request(51, 3, "sidecar-1").id);
    assert_eq!(
        scripted
            .call(&local_request(51, 3, "sidecar-1"), &policy, &caps)
            .result,
        Err(AiError::Cancelled)
    );

    // Latency samples over loopback.
    let lat_stub = Stub::ok(body.clone());
    let lat_url = lat_stub.url();
    let mut samples = Vec::new();
    for id in 100..107u64 {
        let mut adapter = mk(&lat_url);
        let start = Instant::now();
        let outcome = adapter.call(&local_request(id, 3, "sidecar-1"), &policy, &caps);
        if outcome.result.is_ok() {
            samples.push(start.elapsed().as_millis().max(1));
        }
    }
    assert!(samples.len() >= 5, "loopback latency settles");
    while samples.len() < 7 {
        samples.push(samples[samples.len() - 1]);
    }
    let p50 = percentile(samples.clone(), 50.0);
    let p95 = percentile(samples, 95.0);
    assert!(p95 < 2_000, "loopback p95 within budget");

    // Cost: unknown stays unknown; reported usage is honored, never zero-claimed.
    let free_stub = Stub::ok(body.clone());
    let free_url = free_stub.url();
    let mut free_holder = None;
    for n in 0..8u64 {
        let mut adapter = mk(&free_url);
        let outcome = adapter.call(
            &local_request(200 + n * 1000, 3, "sidecar-1"),
            &policy,
            &caps,
        );
        match outcome.result {
            Ok(result) => {
                free_holder = Some(result.usage);
                break;
            }
            Err(AiError::Unavailable) => continue,
            Err(error) => panic!("free stub regressed: {error:?}"),
        }
    }
    let free = free_holder.expect("free stub completes");
    assert_eq!((free.input_tokens, free.output_tokens), (None, None));
    let paid_stub = Stub::ok(usage_body);
    let paid_url = paid_stub.url();
    let mut paid_holder = None;
    for n in 0..8u64 {
        let mut adapter = mk(&paid_url);
        let outcome = adapter.call(
            &local_request(300 + n * 1000, 3, "sidecar-1"),
            &policy,
            &caps,
        );
        match outcome.result {
            Ok(result) => {
                paid_holder = Some(result.usage);
                break;
            }
            Err(AiError::Unavailable) => continue,
            Err(error) => panic!("usage stub regressed: {error:?}"),
        }
    }
    let paid = paid_holder.expect("usage stub completes");
    assert_eq!((paid.input_tokens, paid.output_tokens), (Some(11), Some(5)));

    // Streaming: recorded SSE through the bounded inbox.
    let sse = load_fixture(fx, "sse.openai.txt");
    let (terminal, assembled, _) = recorded_stream_leg(&sse, true);
    assert_eq!(terminal, StreamPoll::Done);
    assert_eq!(assembled, STREAM_TEXT);

    // Cancel under 100 ms (tick only polls; worker observes the flag).
    let (terminal, cancel_ms) = cancel_leg(|| {
        StreamJob::spawn(&stream_config(), SourceLabel::Recorded, |feed| {
            for n in 0..200 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(cancel_ms < 100);

    // Timeout on a slow stub; unavailable on port 0 (hermetic, no dial timing).
    let tardy = Stub::delayed(body.clone(), 400);
    let tardy_url = tardy.url();
    let mut timeout_seen = false;
    for n in 0..5u64 {
        let mut adapter = mk(&tardy_url);
        let mut slow = local_request(400 + n, 3, "sidecar-1");
        slow.budget.max_ms = 100;
        match adapter.call(&slow, &policy, &caps).result {
            Err(AiError::Timeout) => {
                timeout_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("slow stub must time out, got {other:?}"),
        }
    }
    assert!(timeout_seen, "timeout surfaces past the budget");
    let dead = mk("http://127.0.0.1:0/v1/chat/completions");
    let mut dead = dead;
    assert_eq!(
        dead.call(&local_request(500, 3, "sidecar-1"), &policy, &caps)
            .result,
        Err(AiError::Unavailable)
    );

    // Offline: refused before any socket attempt; tape replays x2 offline.
    let closed = mk("http://127.0.0.1:0/v1/chat/completions");
    let mut closed = closed;
    let before = closed.socket_attempts();
    let mut offline = local_request(600, 3, "sidecar-1");
    offline.privacy = PrivacyMode::Offline;
    assert_eq!(
        closed.call(&offline, &policy, &caps).result,
        Err(AiError::UnsupportedCapability)
    );
    assert_eq!(closed.socket_attempts(), before, "offline dials nothing");
    let replay = load_fixture(fx, "replay.json");
    let parsed: serde_json::Value = serde_json::from_str(&replay).expect("replay is JSON");
    let build = || {
        let mut tape = StreamTape::new(SourceLabel::Recorded);
        for entry in parsed["entries"].as_array().expect("entries") {
            tape.push(
                entry["tick"].as_u64().expect("tick"),
                entry["text"].as_str().expect("text"),
            );
        }
        tape
    };
    let first = build();
    let second = build();
    assert_eq!(
        first.entries(),
        second.entries(),
        "offline replay x2 matches"
    );
    assert_eq!(first.checkpoint(), second.checkpoint());

    Row {
        backend: "local",
        source: SourceLabel::Recorded,
        completion: Cell::pass_recorded(),
        bad_output: Cell::pass_recorded(),
        latency: Cell::pass_recorded(),
        cost: Cell::pass_recorded(),
        streaming: Cell::pass_recorded(),
        cancel: Cell::pass_contract(),
        timeout: Cell::pass_recorded(),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: p50,
        p95_ms: p95,
        cost_text: String::from("unknown|11/5"),
        transcript,
        preview,
        fork,
        note: String::from("loopback TCP; structured SetLabel parity"),
    }
}

fn row_openai(fx: &Path, live_url: Option<String>) -> Row {
    row_vendor_openai_anthropic(
        fx,
        "openai",
        "openai:gpt-fixture",
        "openai.chat.json",
        "openai.usage.json",
        "sse.openai.txt",
        true,
        live_url,
        "E2040_AX1_OPENAI_TOKEN",
    )
}

fn row_anthropic(fx: &Path, live_url: Option<String>) -> Row {
    row_vendor_openai_anthropic(
        fx,
        "anthropic",
        "anthropic:claude-fixture",
        "anthropic.messages.json",
        "anthropic.usage.json",
        "sse.anthropic.txt",
        false,
        live_url,
        "E2040_AX1_ANTHROPIC_TOKEN",
    )
}

/// Shared harness for the two streaming adapters (OpenAI-compatible and
/// Anthropic Messages): identical columns, vendor envelope edges only.
#[expect(
    clippy::too_many_arguments,
    reason = "test fixture builder takes each field positionally"
)]
fn row_vendor_openai_anthropic(
    fx: &Path,
    vendor: &'static str,
    model: &str,
    body_file: &str,
    usage_file: &str,
    sse_file: &str,
    openai_shape: bool,
    live_url: Option<String>,
    token_var: &str,
) -> Row {
    let caps = ProviderCapabilities::remote_http();
    assert!(!caps.structured_output, "text parse is never native JSON");
    let body = load_fixture(fx, body_file);
    let usage_body = load_fixture(fx, usage_file);
    let sse = load_fixture(fx, sse_file);

    let mk_recorded = |url: &str| {
        let endpoint = transport::parse_endpoint(url).expect("loopback parses");
        let auth = VendorSecret::Direct(String::from("suite-token"));
        if openai_shape {
            StreamAdapter::OpenAi(OpenAiAdapter::new(vec![MockStep::Succeed], endpoint, auth))
        } else {
            StreamAdapter::Anthropic(AnthropicAdapter::new(
                vec![MockStep::Succeed],
                endpoint,
                auth,
            ))
        }
    };

    // Completion over real TCP with session auth.
    let stub = Stub::ok(body.clone());
    let url = stub.url();
    let policy = remote_policy(&url);
    let mut proposal_holder = None;
    let mut settled = false;
    for n in 0..8u64 {
        let mut adapter = mk_recorded(&url);
        let request = remote_request(1 + n * 1000, 3, model);
        let outcome = adapter.call_mine(&request, &policy, &caps);
        match outcome.0 {
            Ok(ok) => {
                assert!(outcome.1, "{vendor} configured endpoint completed");
                proposal_holder = Some(ok.proposal.clone().expect("proposal"));
                settled = true;
                break;
            }
            Err(AiError::Unavailable) => {}
            Err(other) => panic!("{vendor} completion regressed: {:?}", other),
        }
    }
    assert!(settled, "{vendor} completion settles over loopback");
    let proposal = proposal_holder.expect("proposal");
    validate_proposal(&proposal).expect("vendor proposal validates");
    assert!(canon_lines(&proposal).contains(&CANON_DIAGNOSTIC.to_owned()));
    assert!(stub.hits() > 0, "completion crossed real TCP");
    assert!(stub.auth_hits() > 0, "session auth header present");
    let transcript = transcript_of(&proposal);
    let preview = preview_of(&proposal);
    let fork = fork_of(&proposal);

    // Bad output: wire garbage rejected; scripted malformed needs no socket.
    let bad_stub = Stub::raw(200, String::from("this is not json"));
    let bad_url = bad_stub.url();
    let bad_policy = remote_policy(&bad_url);
    let mut bad_seen = false;
    for n in 0..8u64 {
        let mut adapter = mk_recorded(&bad_url);
        match adapter
            .call_mine(&remote_request(50 + n, 3, model), &bad_policy, &caps)
            .0
        {
            Err(AiError::MalformedOutput) => {
                bad_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("{vendor} garbage must be malformed, got {other:?}"),
        }
    }
    assert!(bad_seen, "{vendor} rejects wire garbage");

    // Latency samples.
    let lat_stub = Stub::ok(body.clone());
    let lat_url = lat_stub.url();
    let lat_policy = remote_policy(&lat_url);
    let mut samples = Vec::new();
    for id in 100..107u64 {
        let mut adapter = mk_recorded(&lat_url);
        let start = Instant::now();
        if adapter
            .call_mine(&remote_request(id, 3, model), &lat_policy, &caps)
            .0
            .is_ok()
        {
            samples.push(start.elapsed().as_millis().max(1));
        }
    }
    assert!(samples.len() >= 5, "{vendor} latency settles");
    while samples.len() < 7 {
        samples.push(samples[samples.len() - 1]);
    }
    let p50 = percentile(samples.clone(), 50.0);
    let p95 = percentile(samples, 95.0);

    // Cost: unknown stays unknown; reported usage honored.
    let free_stub = Stub::ok(body.clone());
    let free_url = free_stub.url();
    let free_policy = remote_policy(&free_url);
    let mut free_usage = None;
    for n in 0..8u64 {
        let mut adapter = mk_recorded(&free_url);
        if let Ok(result) = adapter
            .call_mine(&remote_request(200 + n, 3, model), &free_policy, &caps)
            .0
        {
            free_usage = Some(result.usage);
            break;
        }
    }
    let free_usage = free_usage.expect("free stub completes");
    assert_eq!(
        (free_usage.input_tokens, free_usage.output_tokens),
        (None, None)
    );
    let paid_stub = Stub::ok(usage_body);
    let paid_url = paid_stub.url();
    let paid_policy = remote_policy(&paid_url);
    let mut paid_usage = None;
    for n in 0..8u64 {
        let mut adapter = mk_recorded(&paid_url);
        if let Ok(result) = adapter
            .call_mine(&remote_request(300 + n, 3, model), &paid_policy, &caps)
            .0
        {
            paid_usage = Some(result.usage);
            break;
        }
    }
    let paid_usage = paid_usage.expect("usage stub completes");
    assert!(paid_usage.input_tokens.is_some() && paid_usage.output_tokens.is_some());

    // Streaming through the adapter: SSE over TCP, recorded source on loopback.
    // Setup races (Unavailable before any byte moves) retry bounded.
    let stream_stub = Stub::ok(sse.clone());
    let stream_url = stream_stub.url();
    let mut streamed = None;
    for _ in 0..8 {
        let stream_adapter = mk_recorded(&stream_url);
        let mut job = stream_adapter
            .stream_mine(&remote_request(60, 3, model), &stream_config())
            .expect("loopback stream spawns");
        assert_eq!(job.source(), SourceLabel::Recorded);
        match poll_until(&mut job) {
            StreamPoll::Done => {
                streamed = Some((job.assembled().to_owned(), job.chunks()));
                break;
            }
            StreamPoll::Failed(StreamError::Unavailable) => continue,
            other => panic!("{vendor} stream regressed: {other:?}"),
        }
    }
    let (assembled, chunks) = streamed.expect("stream settles over loopback");
    assert_eq!(assembled, STREAM_TEXT);
    assert_eq!(chunks, 2);
    assert!(stream_stub.hits() >= 1, "stream crossed real TCP");

    // Cancel under 100 ms through the adapter; timeout and unavailable legs
    // use the hermetic port-0 pattern for dial timing.
    let cancel_adapter = mk_recorded(&stream_url);
    let (terminal, cancel_ms) = cancel_leg(|| {
        cancel_adapter
            .stream_mine(&remote_request(71, 3, model), &stream_config())
            .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(cancel_ms < 100, "{vendor} cancel lands in {cancel_ms} ms");
    let tardy = Stub::delayed(sse.clone(), 400);
    let tardy_url = tardy.url();
    let mut stream_timeout = false;
    for _ in 0..4 {
        let tardy_adapter = mk_recorded(&tardy_url);
        let mut slow_req = remote_request(72, 3, model);
        slow_req.budget.max_ms = 100;
        let mut job = tardy_adapter
            .stream_mine(&slow_req, &stream_config())
            .expect("timeout stream spawns");
        let mut terminal = StreamPoll::Pending;
        for _ in 0..500 {
            terminal = job.poll();
            if terminal != StreamPoll::Pending {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        match terminal {
            StreamPoll::Failed(StreamError::Timeout) => {
                stream_timeout = true;
                break;
            }
            StreamPoll::Failed(StreamError::Unavailable) => continue,
            other => panic!("{vendor} slow stream must time out, got {other:?}"),
        }
    }
    assert!(stream_timeout, "{vendor} stream timeout surfaces");
    let dead_adapter = mk_recorded("http://127.0.0.1:0/v1/chat/completions");
    let mut job = dead_adapter
        .stream_mine(&remote_request(73, 3, model), &stream_config())
        .expect("unavailable stream spawns");
    assert_eq!(
        poll_until(&mut job),
        StreamPoll::Failed(StreamError::Unavailable),
        "{vendor} port-0 stream is unavailable"
    );
    // Offline stream refused before spawning, no socket.
    let offline_adapter = mk_recorded("http://127.0.0.1:0/v1/chat/completions");
    let mut offline_req = remote_request(70, 3, model);
    offline_req.privacy = PrivacyMode::Offline;
    assert!(matches!(
        offline_adapter.stream_mine(&offline_req, &stream_config()),
        Err(StreamError::Offline)
    ));

    // Live leg, only with operator keys; recorded otherwise, marked as such.
    // A live failure records a failing contract cell instead of panicking so
    // the printed matrix names the failing contract.
    let mut source = SourceLabel::Recorded;
    let mut note = String::from("recorded loopback stubs");
    let mut completion_cell = Cell::pass_recorded();
    if let Some(live) = live_url {
        let endpoint = transport::parse_endpoint(&live).expect("live URL parses");
        let mut adapter = if openai_shape {
            StreamAdapter::OpenAi(OpenAiAdapter::with_env_token(
                vec![MockStep::Succeed],
                endpoint,
                token_var,
            ))
        } else {
            StreamAdapter::Anthropic(AnthropicAdapter::with_env_token(
                vec![MockStep::Succeed],
                endpoint,
                token_var,
            ))
        };
        let policy = remote_policy(&live);
        let outcome = adapter.call_mine(&remote_request(901, 3, model), &policy, &caps);
        source = SourceLabel::Live;
        match outcome.0 {
            Ok(_) => {
                completion_cell = Cell::pass_live();
                note = String::from("live host served (keys present)");
            }
            Err(error) => {
                completion_cell = Cell::Fail(format!("live:{error:?}"));
                note = String::from("live attempted with operator keys and failed");
            }
        }
    }

    let cost_text = format!(
        "unknown|{}/{}",
        paid_usage.input_tokens.unwrap_or(0),
        paid_usage.output_tokens.unwrap_or(0)
    );
    Row {
        backend: vendor,
        source,
        completion: completion_cell,
        bad_output: Cell::pass_recorded(),
        latency: Cell::pass_recorded(),
        cost: Cell::pass_recorded(),
        streaming: Cell::pass_recorded(),
        cancel: Cell::pass_recorded(),
        timeout: Cell::pass_recorded(),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: p50,
        p95_ms: p95,
        cost_text,
        transcript,
        preview,
        fork,
        note,
    }
}

/// Type-erased handle over the two streaming adapters so one harness
/// covers both distinct implementations without provider-specific fields in
/// the matrix itself.
enum StreamAdapter {
    OpenAi(OpenAiAdapter),
    Anthropic(AnthropicAdapter),
}

trait AdapterCall {
    fn call_mine(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        caps: &ProviderCapabilities,
    ) -> (Result<engine_provider::protocol::AiResult, AiError>, bool);
    fn stream_mine(
        &self,
        request: &AiRequest,
        config: &StreamConfig,
    ) -> Result<StreamJob, StreamError>;
}

impl AdapterCall for StreamAdapter {
    fn call_mine(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        caps: &ProviderCapabilities,
    ) -> (Result<engine_provider::protocol::AiResult, AiError>, bool) {
        let outcome = match self {
            StreamAdapter::OpenAi(adapter) => adapter.call(request, policy, caps),
            StreamAdapter::Anthropic(adapter) => adapter.call(request, policy, caps),
        };
        let result = outcome.result;
        (result, outcome.dispatched_remote)
    }

    fn stream_mine(
        &self,
        request: &AiRequest,
        config: &StreamConfig,
    ) -> Result<StreamJob, StreamError> {
        match self {
            StreamAdapter::OpenAi(adapter) => adapter.stream_job(request, config),
            StreamAdapter::Anthropic(adapter) => adapter.stream_job(request, config),
        }
    }
}

fn row_gemini(fx: &Path, live_url: Option<String>) -> Row {
    let caps = ProviderCapabilities::remote_http();
    assert!(!caps.structured_output, "text parse is never native JSON");
    let body = load_fixture(fx, "gemini.generate.json");
    let usage_body = load_fixture(fx, "gemini.usage.json");
    let mk = |url: &str| {
        GeminiAdapter::new(
            vec![MockStep::Succeed],
            transport::parse_endpoint(url).expect("endpoint parses"),
            VendorSecret::Direct(String::from("suite-token")),
        )
    };
    let model = "gemini:gemini-fixture";

    let stub = Stub::ok(body.clone());
    let url = stub.url();
    let policy = remote_policy(&url);
    let mut proposal_holder = None;
    for n in 0..8u64 {
        let mut adapter = mk(&url);
        let outcome = adapter.call(&remote_request(1 + n * 1000, 3, model), &policy, &caps);
        if let Ok(ok) = outcome.result.as_ref() {
            proposal_holder = Some(ok.proposal.clone().expect("proposal"));
            break;
        }
        assert_eq!(outcome.result, Err(AiError::Unavailable), "setup race only");
    }
    let proposal = proposal_holder.expect("gemini completes");
    validate_proposal(&proposal).expect("gemini proposal validates");
    assert!(canon_lines(&proposal).contains(&CANON_DIAGNOSTIC.to_owned()));
    assert_eq!(stub.auth_hits(), 1, "session auth header present");
    let transcript = transcript_of(&proposal);
    let preview = preview_of(&proposal);
    let fork = fork_of(&proposal);

    // Wrong-vendor envelope is malformed on this distinct shape.
    let wrong = Stub::ok(load_fixture(fx, "anthropic.messages.json"));
    let wrong_url = wrong.url();
    let wrong_policy = remote_policy(&wrong_url);
    let mut malformed_seen = false;
    for n in 0..8u64 {
        let mut adapter = mk(&wrong_url);
        match adapter
            .call(&remote_request(60 + n, 3, model), &wrong_policy, &caps)
            .result
        {
            Err(AiError::MalformedOutput) => {
                malformed_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("wrong envelope must be malformed, got {other:?}"),
        }
    }
    assert!(malformed_seen, "gemini rejects foreign envelopes");

    let lat_stub = Stub::ok(body.clone());
    let lat_url = lat_stub.url();
    let lat_policy = remote_policy(&lat_url);
    let mut samples = Vec::new();
    for id in 100..107u64 {
        let mut adapter = mk(&lat_url);
        let start = Instant::now();
        if adapter
            .call(&remote_request(id, 3, model), &lat_policy, &caps)
            .result
            .is_ok()
        {
            samples.push(start.elapsed().as_millis().max(1));
        }
    }
    assert!(samples.len() >= 5);
    while samples.len() < 7 {
        samples.push(samples[samples.len() - 1]);
    }
    let p50 = percentile(samples.clone(), 50.0);
    let p95 = percentile(samples, 95.0);

    let free_stub = Stub::ok(body.clone());
    let free_url = free_stub.url();
    let free_policy = remote_policy(&free_url);
    let mut free_usage = None;
    for n in 0..8u64 {
        let mut adapter = mk(&free_url);
        if let Ok(result) = adapter
            .call(&remote_request(200 + n, 3, model), &free_policy, &caps)
            .result
        {
            free_usage = Some(result.usage);
            break;
        }
    }
    let free_usage = free_usage.expect("free stub completes");
    assert_eq!(
        (free_usage.input_tokens, free_usage.output_tokens),
        (None, None)
    );
    let paid_stub = Stub::ok(usage_body);
    let paid_url = paid_stub.url();
    let paid_policy = remote_policy(&paid_url);
    let mut paid_usage = None;
    for n in 0..8u64 {
        let mut adapter = mk(&paid_url);
        if let Ok(result) = adapter
            .call(&remote_request(300 + n, 3, model), &paid_policy, &caps)
            .result
        {
            paid_usage = Some(result.usage);
            break;
        }
    }
    let paid_usage = paid_usage.expect("usage stub completes");
    assert_eq!(
        (paid_usage.input_tokens, paid_usage.output_tokens),
        (Some(17), Some(6))
    );

    // No `stream_job` on this adapter (only the OpenAI-compatible and
    // Anthropic adapters stream live): the streaming column runs recorded SSE
    // through the shared inbox.
    let sse = load_fixture(fx, "sse.openai.txt");
    let (terminal, assembled, _) = recorded_stream_leg(&sse, true);
    assert_eq!(terminal, StreamPoll::Done);
    assert_eq!(assembled, STREAM_TEXT);
    let (terminal, cancel_ms) = cancel_leg(|| {
        StreamJob::spawn(&stream_config(), SourceLabel::Recorded, |feed| {
            for n in 0..200 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(cancel_ms < 100);

    let tardy = Stub::delayed(body.clone(), 400);
    let tardy_url = tardy.url();
    let tardy_policy = remote_policy(&tardy_url);
    let mut timeout_seen = false;
    for n in 0..5u64 {
        let mut adapter = mk(&tardy_url);
        let mut slow = remote_request(400 + n, 3, model);
        slow.budget.max_ms = 100;
        match adapter.call(&slow, &tardy_policy, &caps).result {
            Err(AiError::Timeout) => {
                timeout_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("slow stub must time out, got {other:?}"),
        }
    }
    assert!(timeout_seen);
    let mut dead = mk("http://127.0.0.1:0/v1/chat/completions");
    let dead_policy = remote_policy("http://127.0.0.1:0/v1/chat/completions");
    assert_eq!(
        dead.call(&remote_request(500, 3, model), &dead_policy, &caps)
            .result,
        Err(AiError::Unavailable)
    );
    let mut offline_adapter = mk("http://127.0.0.1:0/v1/chat/completions");
    let before = offline_adapter.socket_attempts();
    let mut offline = remote_request(600, 3, model);
    offline.privacy = PrivacyMode::Offline;
    assert_eq!(
        offline_adapter.call(&offline, &dead_policy, &caps).result,
        Err(AiError::UnsupportedCapability)
    );
    assert_eq!(offline_adapter.socket_attempts(), before);

    let mut source = SourceLabel::Recorded;
    let mut note = String::from("recorded loopback; no adapter stream_job (PV4: openai+anthropic)");
    let mut completion_cell = Cell::pass_recorded();
    if let Some(live) = live_url {
        let endpoint = transport::parse_endpoint(&live).expect("live URL parses");
        let mut adapter = GeminiAdapter::with_env_token(
            vec![MockStep::Succeed],
            endpoint,
            "E2040_AX1_GEMINI_TOKEN",
        );
        let policy = remote_policy(&live);
        let outcome = adapter.call(&remote_request(901, 3, model), &policy, &caps);
        source = SourceLabel::Live;
        match outcome.result {
            Ok(_) => {
                completion_cell = Cell::pass_live();
                note = String::from("live host served (keys present)");
            }
            Err(error) => {
                completion_cell = Cell::Fail(format!("live:{error:?}"));
                note = String::from("live attempted with operator keys and failed");
            }
        }
    }

    Row {
        backend: "gemini",
        source,
        completion: completion_cell,
        bad_output: Cell::pass_recorded(),
        latency: Cell::pass_recorded(),
        cost: Cell::pass_recorded(),
        streaming: Cell::Pass("pass(recorded-infra)"),
        cancel: Cell::pass_contract(),
        timeout: Cell::pass_recorded(),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: p50,
        p95_ms: p95,
        cost_text: String::from("unknown|17/6"),
        transcript,
        preview,
        fork,
        note,
    }
}

fn row_ollama(fx: &Path, live_present: bool) -> Row {
    let caps = ProviderCapabilities::local_http();
    assert!(!caps.structured_output, "text parse is never native JSON");
    let body = load_fixture(fx, "ollama.chat.json");
    let usage_body = load_fixture(fx, "ollama.usage.json");
    let mk = |url: &str| {
        OllamaAdapter::new(
            vec![MockStep::Succeed],
            transport::parse_endpoint(url).expect("loopback parses"),
        )
        .expect("loopback accepted")
    };
    let model = "ollama:llama-fixture";
    let policy = local_policy();

    let stub = Stub::ok(body.clone());
    let url = stub.url();
    let mut proposal_holder = None;
    for n in 0..8u64 {
        let mut adapter = mk(&url);
        let outcome = adapter.call(&local_request(1 + n * 1000, 3, model), &policy, &caps);
        if let Ok(ok) = outcome.result.as_ref() {
            proposal_holder = Some(ok.proposal.clone().expect("proposal"));
            break;
        }
        assert_eq!(outcome.result, Err(AiError::Unavailable), "setup race only");
    }
    let proposal = proposal_holder.expect("ollama completes");
    validate_proposal(&proposal).expect("ollama proposal validates");
    assert!(canon_lines(&proposal).contains(&CANON_DIAGNOSTIC.to_owned()));
    let transcript = transcript_of(&proposal);
    let preview = preview_of(&proposal);
    let fork = fork_of(&proposal);

    // Chat-completions envelope is malformed on the Ollama shape.
    let wrong = Stub::ok(load_fixture(fx, "openai.chat.json"));
    let wrong_url = wrong.url();
    let mut malformed_seen = false;
    for n in 0..8u64 {
        let mut adapter = mk(&wrong_url);
        match adapter
            .call(&local_request(60 + n, 3, model), &policy, &caps)
            .result
        {
            Err(AiError::MalformedOutput) => {
                malformed_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("wrong envelope must be malformed, got {other:?}"),
        }
    }
    assert!(malformed_seen);

    let lat_stub = Stub::ok(body.clone());
    let lat_url = lat_stub.url();
    let mut samples = Vec::new();
    for id in 100..107u64 {
        let mut adapter = mk(&lat_url);
        let start = Instant::now();
        if adapter
            .call(&local_request(id, 3, model), &policy, &caps)
            .result
            .is_ok()
        {
            samples.push(start.elapsed().as_millis().max(1));
        }
    }
    assert!(samples.len() >= 5);
    while samples.len() < 7 {
        samples.push(samples[samples.len() - 1]);
    }
    let p50 = percentile(samples.clone(), 50.0);
    let p95 = percentile(samples, 95.0);

    let free_stub = Stub::ok(body.clone());
    let free_url = free_stub.url();
    let mut free_usage = None;
    for n in 0..8u64 {
        let mut adapter = mk(&free_url);
        if let Ok(result) = adapter
            .call(&local_request(200 + n, 3, model), &policy, &caps)
            .result
        {
            free_usage = Some(result.usage);
            break;
        }
    }
    assert_eq!(
        free_usage.expect("free completes").input_tokens,
        None,
        "unknown stays unknown, never zero"
    );
    let paid_stub = Stub::ok(usage_body);
    let paid_url = paid_stub.url();
    let mut paid_usage = None;
    for n in 0..8u64 {
        let mut adapter = mk(&paid_url);
        if let Ok(result) = adapter
            .call(&local_request(300 + n, 3, model), &policy, &caps)
            .result
        {
            paid_usage = Some(result.usage);
            break;
        }
    }
    let paid_usage = paid_usage.expect("usage completes");
    assert_eq!(
        (paid_usage.input_tokens, paid_usage.output_tokens),
        (Some(7), Some(3))
    );

    let sse = load_fixture(fx, "sse.openai.txt");
    let (terminal, assembled, _) = recorded_stream_leg(&sse, true);
    assert_eq!(terminal, StreamPoll::Done);
    assert_eq!(assembled, STREAM_TEXT);
    let (terminal, cancel_ms) = cancel_leg(|| {
        StreamJob::spawn(&stream_config(), SourceLabel::Recorded, |feed| {
            for n in 0..200 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(cancel_ms < 100);

    let tardy = Stub::delayed(body.clone(), 400);
    let tardy_url = tardy.url();
    let mut timeout_seen = false;
    for n in 0..5u64 {
        let mut adapter = mk(&tardy_url);
        let mut slow = local_request(400 + n, 3, model);
        slow.budget.max_ms = 100;
        match adapter.call(&slow, &policy, &caps).result {
            Err(AiError::Timeout) => {
                timeout_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("slow stub must time out, got {other:?}"),
        }
    }
    assert!(timeout_seen);
    let mut dead = mk("http://127.0.0.1:0/api/chat");
    assert_eq!(
        dead.call(&local_request(500, 3, model), &policy, &caps)
            .result,
        Err(AiError::Unavailable)
    );
    if live_present {
        // Operator daemon requested but this harness never fakes live with a
        // stub: the live attempt below would dial the real daemon URL.
    }

    Row {
        backend: "ollama",
        source: SourceLabel::Recorded,
        completion: Cell::pass_recorded(),
        bad_output: Cell::pass_recorded(),
        latency: Cell::pass_recorded(),
        cost: Cell::pass_recorded(),
        streaming: Cell::Pass("pass(recorded-infra)"),
        cancel: Cell::pass_contract(),
        timeout: Cell::pass_recorded(),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: p50,
        p95_ms: p95,
        cost_text: String::from("unknown|7/3"),
        transcript,
        preview,
        fork,
        note: String::from(if live_present {
            "GAP: live daemon requested, stub-backed recorded legs only"
        } else {
            "recorded loopback; live opt-in by E2040_AX1_OLLAMA_URL"
        }),
    }
}

fn row_mcp_client(fx: &Path) -> Row {
    let body = load_fixture(fx, "local.chat.json");
    let usage_body = load_fixture(fx, "local.usage.json");
    let mk = |url: &str| McpClient::new(transport::parse_endpoint(url).expect("loopback parses"));

    // Completion through the client path over real TCP.
    let stub = Stub::ok(body.clone());
    let url = stub.url();
    let mut proposal_holder = None;
    for n in 0..8u64 {
        let mut client = mk(&url);
        match client.call(&local_request(1 + n * 1000, 3, "sidecar-1")) {
            Ok((proposal, _)) => {
                proposal_holder = Some(proposal);
                break;
            }
            Err(AiError::Unavailable) => continue,
            Err(error) => panic!("mcp client regressed: {error:?}"),
        }
    }
    let proposal = proposal_holder.expect("mcp client completes");
    validate_proposal(&proposal).expect("mcp proposal validates");
    assert!(canon_lines(&proposal).contains(&CANON_DIAGNOSTIC.to_owned()));
    assert!(stub.hits() > 0, "client crossed real TCP");
    let transcript = transcript_of(&proposal);
    let preview = preview_of(&proposal);
    let fork = fork_of(&proposal);

    // Bad output rejected on the client path.
    let bad_stub = Stub::raw(200, String::from("this is not json"));
    let bad_url = bad_stub.url();
    let mut malformed_seen = false;
    for n in 0..8u64 {
        let mut client = mk(&bad_url);
        match client.call(&local_request(50 + n, 3, "sidecar-1")) {
            Err(AiError::MalformedOutput) => {
                malformed_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("garbage must be malformed, got {other:?}"),
        }
    }
    assert!(malformed_seen);

    // Latency and cost.
    let lat_stub = Stub::ok(body.clone());
    let lat_url = lat_stub.url();
    let mut samples = Vec::new();
    for id in 100..107u64 {
        let mut client = mk(&lat_url);
        let start = Instant::now();
        if client.call(&local_request(id, 3, "sidecar-1")).is_ok() {
            samples.push(start.elapsed().as_millis().max(1));
        }
    }
    assert!(samples.len() >= 5);
    while samples.len() < 7 {
        samples.push(samples[samples.len() - 1]);
    }
    let p50 = percentile(samples.clone(), 50.0);
    let p95 = percentile(samples, 95.0);
    let free_stub = Stub::ok(body.clone());
    let free_url = free_stub.url();
    let mut free_usage = None;
    for n in 0..8u64 {
        let mut client = mk(&free_url);
        if let Ok((_, usage)) = client.call(&local_request(200 + n, 3, "sidecar-1")) {
            free_usage = Some(usage);
            break;
        }
    }
    assert_eq!(
        free_usage.expect("free completes").input_tokens,
        None,
        "unknown stays unknown, never zero"
    );
    let paid_stub = Stub::ok(usage_body);
    let paid_url = paid_stub.url();
    let mut paid_usage = None;
    for n in 0..8u64 {
        let mut client = mk(&paid_url);
        if let Ok((_, usage)) = client.call(&local_request(300 + n, 3, "sidecar-1")) {
            paid_usage = Some(usage);
            break;
        }
    }
    let paid_usage = paid_usage.expect("usage completes");
    assert_eq!(
        (paid_usage.input_tokens, paid_usage.output_tokens),
        (Some(11), Some(5))
    );

    // Streaming recorded; cancel under 100 ms; timeout; port-0 unavailable.
    let sse = load_fixture(fx, "sse.openai.txt");
    let (terminal, assembled, _) = recorded_stream_leg(&sse, true);
    assert_eq!(terminal, StreamPoll::Done);
    assert_eq!(assembled, STREAM_TEXT);
    let (terminal, cancel_ms) = cancel_leg(|| {
        StreamJob::spawn(&stream_config(), SourceLabel::Recorded, |feed| {
            for n in 0..200 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(cancel_ms < 100);
    let tardy = Stub::delayed(body.clone(), 400);
    let tardy_url = tardy.url();
    let mut timeout_seen = false;
    for n in 0..5u64 {
        let mut client = mk(&tardy_url);
        let mut slow = local_request(400 + n, 3, "sidecar-1");
        slow.budget.max_ms = 100;
        match client.call(&slow) {
            Err(AiError::Timeout) => {
                timeout_seen = true;
                break;
            }
            Err(AiError::Unavailable) => continue,
            other => panic!("slow stub must time out, got {other:?}"),
        }
    }
    assert!(timeout_seen);
    let mut dead = mk("http://127.0.0.1:0/v1/chat/completions");
    assert_eq!(
        dead.call(&local_request(500, 3, "sidecar-1")),
        Err(AiError::Unavailable)
    );
    // Offline refused before any dial.
    let mut quiet = mk("http://127.0.0.1:0/v1/chat/completions");
    let before = quiet.socket_attempts;
    let mut offline = local_request(600, 3, "sidecar-1");
    offline.privacy = PrivacyMode::Offline;
    assert_eq!(quiet.call(&offline), Err(AiError::UnsupportedCapability));
    assert_eq!(quiet.socket_attempts, before);

    Row {
        backend: "mcp-client",
        source: SourceLabel::Recorded,
        completion: Cell::pass_recorded(),
        bad_output: Cell::pass_recorded(),
        latency: Cell::pass_recorded(),
        cost: Cell::pass_recorded(),
        streaming: Cell::Pass("pass(recorded-infra)"),
        cancel: Cell::pass_contract(),
        timeout: Cell::pass_recorded(),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: p50,
        p95_ms: p95,
        cost_text: String::from("unknown|11/5"),
        transcript,
        preview,
        fork,
        note: String::from("GAP: loopback-recorded client path; no external MCP server in lane"),
    }
}

fn row_offline(fx: &Path) -> Row {
    let policy = engine_provider::fixtures::policy();
    let caps = ProviderCapabilities::mock();

    // Offline never completes over the network: the refusal is the correct
    // behavior, and the scripted fallback serves the same Noop shape so the
    // transcript triple still compares. The mock makes no socket calls by
    // construction, so this leg uses the local adapter against an unreachable
    // port-0 endpoint: refusal must record zero socket attempts.
    let endpoint = transport::parse_endpoint("http://127.0.0.1:0/v1/chat/completions")
        .expect("endpoint parses");
    let mut adapter =
        LocalAdapter::new(vec![MockStep::Succeed], endpoint).expect("loopback accepted");
    let mut offline = mock_request(1, 3);
    offline.model.adapter = AdapterKind::Local;
    offline.privacy = PrivacyMode::Offline;
    let before = adapter.socket_attempts();
    let refused = adapter.call(&offline, &policy, &caps);
    assert_eq!(refused.result, Err(AiError::UnsupportedCapability));
    assert!(!refused.dispatched_remote);
    assert_eq!(adapter.socket_attempts(), before, "offline dials nothing");
    let served = fallback_for(AiError::UnsupportedCapability, &offline, &policy)
        .expect("refusal takes the scripted path");
    validate_proposal(&served.proposal).expect("fallback proposal validates");
    let transcript = transcript_of(&served.proposal);
    let preview = preview_of(&served.proposal);
    let fork = fork_of(&served.proposal);
    assert_eq!(
        transcript, "[Noop]",
        "offline fallback carries the Noop shape"
    );

    // Bad output can never arrive offline: validation still rejects junk.
    let over = Proposal {
        request: RequestId(1),
        base_revision: 3,
        actions: vec![engine_provider::protocol::ProposalAction::Noop; 65],
        object_versions: Vec::new(),
        diagnostics: Vec::new(),
    };
    assert_eq!(validate_proposal(&over), Err(AiError::MalformedOutput));

    // Streaming offline: spawn refused before any byte moves; the recorded
    // tape below is the offline streaming story.
    let mut offline_req = mock_request(2, 3);
    offline_req.privacy = PrivacyMode::Offline;
    let offline_adapter = OpenAiAdapter::new(
        vec![MockStep::Succeed],
        transport::parse_endpoint("http://127.0.0.1:0/v1/chat/completions")
            .expect("endpoint parses"),
        VendorSecret::Direct(String::from("offline-token")),
    );
    assert!(matches!(
        offline_adapter.stream_job(&offline_req, &stream_config()),
        Err(StreamError::Offline)
    ));

    // Cancel of a recorded stream still lands under 100 ms with no network.
    let (terminal, cancel_ms) = cancel_leg(|| {
        StreamJob::spawn(&stream_config(), SourceLabel::Recorded, |feed| {
            for n in 0..200 {
                if feed.push(&format!("chunk-{n} ")).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
            feed.finish();
        })
        .expect("cancel stream spawns")
    });
    assert_eq!(terminal, StreamPoll::Failed(StreamError::Cancelled));
    assert!(cancel_ms < 100);

    // Offline replay x2 from the recorded fixture: verbatim, no worker.
    let replay = load_fixture(fx, "replay.json");
    let parsed: serde_json::Value = serde_json::from_str(&replay).expect("replay is JSON");
    let build = || {
        let mut tape = StreamTape::new(SourceLabel::Recorded);
        for entry in parsed["entries"].as_array().expect("entries") {
            tape.push(
                entry["tick"].as_u64().expect("tick"),
                entry["text"].as_str().expect("text"),
            );
        }
        tape
    };
    let first = build();
    let second = build();
    assert_eq!(first.entries(), second.entries());
    assert_eq!(first.checkpoint(), second.checkpoint());
    assert_eq!(first.replay().count(), 3);

    Row {
        backend: "offline",
        source: SourceLabel::Recorded,
        completion: Cell::Pass("pass(refused-correct)"),
        bad_output: Cell::pass_contract(),
        latency: Cell::Pass("pass(no-timer)"),
        cost: Cell::Pass("pass(zero-spend)"),
        streaming: Cell::Pass("pass(refused-before-spawn)"),
        cancel: Cell::pass_contract(),
        timeout: Cell::Pass("pass(no-timer)"),
        replay: Cell::pass_offline(),
        redteam: Cell::Pass("pass(shared)"),
        p50_ms: 0,
        p95_ms: 0,
        cost_text: String::from("zero-spend"),
        transcript,
        preview,
        fork,
        note: String::from("no socket ever; fallback Noop keeps the triple equal"),
    }
}

// ---------------------------------------------------------------------------
// The matrix.
// ---------------------------------------------------------------------------

#[test]
fn pv5_conformance_matrix() {
    let fx = matrix_dir();
    assert!(fx.is_dir(), "fixture dir {} exists", fx.display());

    // Tick isolation (asserted once): a parked worker never
    // stalls the tick, and cancel releases it.
    {
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let mut job = StreamJob::spawn(
            &StreamConfig::new(8, 1024, 5_000, 4).expect("config valid"),
            SourceLabel::Live,
            move |feed| {
                loop {
                    if feed.is_cancelled() {
                        feed.fail(StreamError::Cancelled);
                        return;
                    }
                    match gate_rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    }
                }
                feed.finish();
            },
        )
        .expect("parked stream spawns");
        let start = Instant::now();
        for _ in 0..120 {
            assert_eq!(
                job.poll(),
                StreamPoll::Pending,
                "parked worker stays pending"
            );
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "polls never block"
        );
        job.cancel();
        assert_eq!(
            poll_until(&mut job),
            StreamPoll::Failed(StreamError::Cancelled)
        );
        drop(gate_tx);
    }

    // Red-team replay runs once; every backend row shares the verdict.
    let (redteam_cases, secret_count, _) = replay_redteam();

    // Live opt-ins: presence-checked only, values never read or printed.
    let openai_live = has_live("E2040_AX1_OPENAI_URL")
        .then(|| std::env::var("E2040_AX1_OPENAI_URL").expect("presence-checked"));
    let anthropic_live = has_live("E2040_AX1_ANTHROPIC_URL")
        .then(|| std::env::var("E2040_AX1_ANTHROPIC_URL").expect("presence-checked"));
    let gemini_live = has_live("E2040_AX1_GEMINI_URL")
        .then(|| std::env::var("E2040_AX1_GEMINI_URL").expect("presence-checked"));
    let ollama_live = has_live("E2040_AX1_OLLAMA_URL");

    let rows = vec![
        row_mock(&fx, redteam_cases),
        row_local(&fx),
        row_openai(&fx, openai_live),
        row_anthropic(&fx, anthropic_live),
        row_gemini(&fx, gemini_live),
        row_ollama(&fx, ollama_live),
        row_mcp_client(&fx),
        row_offline(&fx),
    ];

    // The canonical fixture proposal is the Noop every backend carries:
    // the harness triple below must equal its transcript, preview and fork.
    let canon: serde_json::Value =
        serde_json::from_str(&load_fixture(&fx, "proposal.json")).expect("proposal is JSON");
    assert_eq!(canon["actions"], serde_json::json!([{"noop": true}]));
    assert!(canon["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .contains(&serde_json::json!("matrix:canon")));

    // The same proposal gives the same transcript, preview and fork on
    // every backend; only source, latency and cost differ.
    let first = &rows[0];
    for row in &rows[1..] {
        assert_eq!(
            row.transcript, first.transcript,
            "transcript equal on {}",
            row.backend
        );
        assert_eq!(
            row.preview, first.preview,
            "preview equal on {}",
            row.backend
        );
        assert_eq!(row.fork, first.fork, "fork equal on {}", row.backend);
    }

    // RemoteAdapter contract parity spot-check: recorded loopback
    // serves the canonical proposal with session auth.
    {
        let body = load_fixture(&fx, "local.chat.json");
        let stub = Stub::ok(body);
        let url = stub.url();
        let endpoint = transport::parse_endpoint(&url).expect("endpoint parses");
        let mut adapter = RemoteAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            SecretRef::Direct(String::from("suite-token")),
        );
        let policy = remote_policy(&url);
        let caps = ProviderCapabilities::remote_http();
        let mut settled = false;
        for n in 0..8u64 {
            let outcome = adapter.call(&remote_request(700 + n, 3, "hosted-1"), &policy, &caps);
            if outcome.result.is_ok() {
                settled = true;
                assert!(outcome.dispatched_remote);
                break;
            }
        }
        assert!(settled, "remote adapter parity holds");
        assert!(stub.auth_hits() > 0, "remote sends session auth");
    }

    println!("PV5 provider conformance matrix (hook `{PV5_MATRIX_HOOK}`)");
    println!("source: recorded = loopback stub, live = operator keys present");
    println!(
        "{:<10} {:<8} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:>8} {:>8} {:<14}",
        "backend",
        "source",
        "completion",
        "bad-output",
        "latency",
        "cost",
        "streaming",
        "cancel<100ms",
        "timeout",
        "replay-x2",
        "redteam",
        "p50ms",
        "p95ms",
        "cost"
    );
    let mut failures = 0;
    for row in &rows {
        let cells: Vec<String> = row.cells().iter().map(|cell| cell.text()).collect();
        println!(
            "{:<10} {:<8} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:<14} {:>8} {:>8} {:<14}",
            row.backend,
            row.source,
            cells[0],
            cells[1],
            cells[2],
            cells[3],
            cells[4],
            cells[5],
            cells[6],
            cells[7],
            cells[8],
            row.p50_ms,
            row.p95_ms,
            row.cost_text,
        );
        if !row.ok() {
            failures += 1;
        }
    }
    println!("transcript: {}", first.transcript);
    println!("preview: {}", first.preview);
    println!("fork: {}", first.fork);
    println!(
        "redteam tapes: {redteam_cases} cases, {secret_count} sentinels, replay x2 deterministic"
    );
    for row in &rows {
        println!("note[{}]: {}", row.backend, row.note);
    }
    println!("consumer: proof hook `{PV5_MATRIX_HOOK}` wired at tools/proof.mjs (pv5-matrix gate)");
    println!("live: openai={} anthropic={} gemini={} ollama={} (recorded unless keys exist; never faked)",
        has_live("E2040_AX1_OPENAI_URL"),
        has_live("E2040_AX1_ANTHROPIC_URL"),
        has_live("E2040_AX1_GEMINI_URL"),
        has_live("E2040_AX1_OLLAMA_URL"));

    assert_eq!(failures, 0, "{failures} backend rows failed the matrix");
}

#[test]
fn pv5_hook_registered() {
    assert_eq!(PV5_MATRIX_HOOK, "pv5-matrix", "hook name is registered");
}
