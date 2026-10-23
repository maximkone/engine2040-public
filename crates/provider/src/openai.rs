//! openai: OpenAI-compatible chat-completions vendor adapter.
//! Request body and response envelope follow `/v1/chat/completions`
//! (`model`, `messages`, `max_tokens`; `choices[0].message.content` plus
//! `usage.prompt_tokens`/`completion_tokens`). The bearer token is
//! adapter-local ([`crate::vendor::VendorSecret`]), resolved at call time,
//! sent in the `Authorization` header only, and never appears in errors,
//! events, diagnostics or `Debug` output. Contract guards, jobs and scripted
//! failure arms run through the shared contract state so the contract suite
//! ports unchanged. Default tests use loopback stubs; live hosts are opt-in
//! with `E2040_AX1_OPENAI_URL` plus `E2040_AX1_OPENAI_TOKEN`.

use super::core::{AdapterOutcome, MockStep};
use super::ladder::RungSupport;
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::AiRequest;
use super::stream::{openai_sse_text, StreamConfig, StreamError, StreamJob};
use super::transport::{self, Endpoint};
use super::vendor::{http_adapter_api, Envelope, HttpAdapter, StreamWire, VendorSecret, Wire};

/// Production path for OpenAI-compatible chat completions.
pub const PRODUCTION_PATH: &str = "/v1/chat/completions";

/// Where a chat-completions response keeps its text and usage counters.
const ENVELOPE: Envelope = Envelope {
    text: "/choices/0/message/content",
    input_tokens: "/usage/prompt_tokens",
    output_tokens: "/usage/completion_tokens",
    native_tool: false,
};

/// Single-shot wire: the chat-completions body (digest header plus any
/// bounded scene brief, no secret), remote dispatch, strict proposal parse.
const WIRE: Wire = Wire {
    label: "openai",
    body: transport::chat_body,
    envelope: &ENVELOPE,
    remote: true,
    rungs: RungSupport::FULL,
};

/// Streaming wire: the stream body plus the chat-completions SSE delta
/// extractor.
const STREAM_WIRE: StreamWire = StreamWire {
    body: stream_body,
    text: openai_sse_text,
};

/// Build the streaming chat request body: the single-shot shape (pins
/// included) plus the portable `"stream": true` flag. No provider-specific
/// fields. Manual string building over the pin fields keeps the same keys,
/// escaping and order as the single-shot body: each field value renders
/// through its JSON display, joined in field order with the stream flag last.
fn stream_body(request: &AiRequest, pins: &RequestPins) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (key, value) in pins.chat_fields(request) {
        parts.push(format!("\"{key}\":{value}"));
    }
    parts.push(String::from("\"stream\":true"));
    format!("{{{}}}", parts.join(","))
}

/// OpenAI-compatible vendor adapter over the owned `std::net::TcpStream`
/// client, with the contract suite running unchanged.
#[derive(Clone, Debug)]
pub struct OpenAiAdapter {
    http: HttpAdapter,
    auth: VendorSecret,
}

impl OpenAiAdapter {
    /// Build an adapter for `endpoint` with an adapter-local credential.
    /// Session authorization is enforced per call, before any socket attempt.
    pub fn new(script: Vec<MockStep>, endpoint: Endpoint, auth: VendorSecret) -> Self {
        Self {
            http: HttpAdapter::new(script, endpoint),
            auth,
        }
    }

    /// Adapter-local credential (env var or held value, never printed).
    pub fn with_env_token(script: Vec<MockStep>, endpoint: Endpoint, env_var: &str) -> Self {
        Self::new(script, endpoint, VendorSecret::EnvVar(env_var.to_owned()))
    }

    http_adapter_api!();

    /// Completed HTTP exchanges with the configured endpoint.
    pub fn remote_calls(&self) -> u32 {
        self.http.remote_calls()
    }

    /// Spawn a tick-isolated SSE stream job against the configured endpoint
    /// (see [`HttpAdapter::stream_job`]): the streaming body is POSTed on the
    /// worker thread, so no network call happens on the tick.
    pub fn stream_job(
        &self,
        request: &AiRequest,
        config: &StreamConfig,
    ) -> Result<StreamJob, StreamError> {
        self.http
            .stream_job(&self.auth, STREAM_WIRE, request, config)
    }
}

impl ProviderAdapter for OpenAiAdapter {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        self.http
            .call(&WIRE, Some(&self.auth), request, policy, capabilities)
    }

    fn request_pins(&self) -> Option<&RequestPins> {
        Some(&self.http.pins)
    }
}

#[cfg(test)]
crate::vendor::impl_suite_adapter!(OpenAiAdapter);

#[cfg(test)]
mod tests {
    use super::super::policy::ProviderCapabilities;
    use super::super::protocol::{AdapterKind, AiError, PrivacyMode};
    use super::*;
    use crate::stub::{closed_port, StubServer};
    use crate::suite::{run_failure_suite, SuiteAdapter, SuiteExpect};

    pub(crate) fn policy_for(url: &str) -> AiSessionPolicy {
        AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![url.to_owned()],
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: crate::policy::Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        }
    }

    pub(crate) fn request(id: u64, base_revision: u64) -> AiRequest {
        let mut inner = crate::mock::fixtures::request(id, base_revision);
        inner.model.adapter = AdapterKind::Remote;
        inner.model.model = String::from("openai:gpt-fixture");
        inner.privacy = PrivacyMode::RemoteAllowed;
        inner
    }

    fn adapter_for(url: &str, script: Vec<MockStep>) -> OpenAiAdapter {
        let endpoint = transport::parse_endpoint(url).expect("test endpoint parses");
        OpenAiAdapter::new(
            script,
            endpoint,
            VendorSecret::Direct(String::from("suite-token")),
        )
    }

    #[test]
    fn pv1_failure_suite_passes_through_openai() {
        // Usage-free stub: unknown cost stays unknown, as in the mock suite.
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["suite:stub"]}"#,
            None,
        ));
        let url = stub.url();
        let policy = policy_for(&url);
        let caps = ProviderCapabilities::remote_http();
        assert!(!caps.structured_output, "text parse is never native JSON");
        run_failure_suite(
            &|script| Box::new(adapter_for(&url, script)) as Box<dyn SuiteAdapter>,
            &policy,
            &caps,
            &request,
            &|id| {
                let mut downgraded = request(id, 3);
                downgraded.privacy = PrivacyMode::LocalOnly;
                downgraded
            },
            &SuiteExpect {
                remote_dispatch_on_success: true,
                receipt_host: String::from("127.0.0.1"),
            },
        );
        assert!(
            stub.hits() > 0,
            "suite success arms crossed real TCP, not the mock"
        );
        assert!(stub.auth_hits() > 0, "openai sends session auth");
    }

    #[test]
    fn stub_roundtrip_receipt_and_usage() {
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["stub:ok"]}"#,
            Some((11, 5)),
        ));
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let (outcome, adapter) = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                let outcome = adapter.call(&request(1 + n as u64, 3), &policy_for(&url), &caps);
                (outcome, adapter)
            },
            |(o, _)| o.result.is_ok(),
        );
        assert!(outcome.result.is_ok(), "stub roundtrip succeeds");
        assert!(outcome.dispatched_remote, "configured endpoint completed");
        assert_eq!(adapter.remote_calls(), 1);
        assert_eq!(adapter.transport_calls(), 1);
        assert_eq!(stub.hits(), 1, "exactly one TCP roundtrip happened");
        assert_eq!(stub.auth_hits(), 1, "session auth header present");
        let result = outcome.result.as_ref().expect("success must complete");
        assert_eq!(result.usage.input_tokens, Some(11));
        assert_eq!(result.usage.output_tokens, Some(5));
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line == "openai:127.0.0.1"),
            "receipt labels the vendor plus host: {proposal:?}"
        );
    }

    #[test]
    fn scene_brief_enters_the_vendor_payload() {
        let brief = "scene brief: object 7 is a crate at (120, 0, 40) cm.\nobject 9 is the floor.";
        let mut ask = request(1, 3);
        ask.body = Some(String::from(brief));
        let body = transport::chat_body(&ask, &RequestPins::default());
        assert!(body.contains("[scene brief follows] body_bytes="), "{body}");
        assert!(body.contains("object 7 is a crate"), "{body}");
        assert!(body.contains("[end scene brief]"), "{body}");
        let sent: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
        let user = sent["messages"][1]["content"]
            .as_str()
            .expect("user prompt");
        assert!(user.contains(brief), "the brief stays one string: {user}");
    }

    #[test]
    fn scene_brief_reaches_the_socket_body() {
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["stub:brief"]}"#,
            None,
        ));
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let brief = "scene brief: object 7 is a crate at (120, 0, 40) cm.\nobject 9 is the floor.";
        let mut ask = request(1, 3);
        ask.body = Some(String::from(brief));
        let (outcome, _) = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                let mut attempt = ask.clone();
                attempt.id = crate::protocol::RequestId(40 + u64::from(n));
                let outcome = adapter.call(&attempt, &policy_for(&url), &caps);
                (outcome, adapter)
            },
            |(outcome, _)| outcome.result.is_ok(),
        );
        assert!(outcome.result.is_ok(), "stub roundtrip succeeds");
        let bodies = stub.bodies();
        let sent: serde_json::Value =
            serde_json::from_str(bodies.last().expect("one body was sent")).expect("body is JSON");
        let user = sent["messages"][1]["content"]
            .as_str()
            .expect("user prompt");
        assert!(
            user.contains(brief),
            "brief crossed the socket whole: {user}"
        );
        assert!(user.contains("[scene brief follows] body_bytes="), "{user}");
        assert!(user.ends_with("[end scene brief]"), "{user}");
    }

    #[test]
    fn request_body_is_openai_shaped() {
        let body = transport::chat_body(&request(1, 3), &RequestPins::default());
        assert!(body.contains("\"messages\""), "chat messages envelope");
        assert!(body.contains("\"role\":\"system\""));
        assert!(body.contains("\"role\":\"user\""));
        assert!(body.contains("revision=3"), "prompt carries the revision");
        assert!(body.contains("\"max_tokens\""), "token bound present");
    }

    #[test]
    fn bodies_carry_the_pins_single_shot_and_stream() {
        let ask = request(1, 3);
        let pins = RequestPins::default()
            .with_system("Answer in JSON.")
            .and_then(|p| p.with_temperature_milli(300))
            .and_then(|p| {
                p.with_tools(vec![crate::pins::ToolDef::new(
                    "move",
                    "Move one object.",
                    crate::ladder::action_schema("noop").expect("noop schema is an object"),
                )?])
            })
            .expect("pins in bounds");
        let single = transport::chat_body(&ask, &pins);
        let stream = super::stream_body(&ask, &pins);
        for (label, body) in [("single", single), ("stream", stream)] {
            assert!(body.contains("\"Answer in JSON.\""), "{label}");
            assert!(body.contains("\"temperature\":0.3"), "{label}");
            assert!(body.contains("\"seed\":2040"), "{label}");
            assert!(body.contains("\"name\":\"move\""), "{label}");
        }
        let default = transport::chat_body(&ask, &RequestPins::default());
        assert!(
            default.contains("You diagnose one staged finding"),
            "no override: the task kind's own prompt"
        );
        assert!(default.contains("\"temperature\":0.0"), "greedy default");
        assert!(!default.contains("\"tools\""), "tools only when given");
    }

    #[test]
    fn secrets_never_appear_in_diagnostics() {
        let canary = "e2040-canary-41aa-openai-secret";
        crate::test_env::set("E2040_AX1_REDACT_OPENAI", canary);
        let stub = StubServer::spawn_raw(401, String::from("{}"));
        let url = stub.url();
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = OpenAiAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            VendorSecret::EnvVar(String::from("E2040_AX1_REDACT_OPENAI")),
        );
        let outcome = adapter.call(
            &request(2, 3),
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        let exposed = format!("{outcome:?} {adapter:?}");
        assert!(!exposed.contains(canary), "refusal leaks the secret");
        let direct_debug = format!("{:?}", VendorSecret::Direct(String::from(canary)));
        assert!(!direct_debug.contains(canary), "Direct Debug redacts");
        crate::test_env::remove("E2040_AX1_REDACT_OPENAI");
    }

    #[test]
    fn missing_credential_refuses_without_socket() {
        crate::test_env::remove("E2040_AX1_MISSING_OPENAI");
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = OpenAiAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            VendorSecret::EnvVar(String::from("E2040_AX1_MISSING_OPENAI")),
        );
        let outcome = adapter.call(
            &request(3, 3),
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert_eq!(adapter.socket_attempts(), 0, "no credential, no dial");
        assert!(!outcome.dispatched_remote);
    }

    #[test]
    fn offline_privacy_makes_no_socket_attempt() {
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = OpenAiAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            VendorSecret::Direct(String::from("offline-token")),
        );
        let mut offline = request(4, 3);
        offline.privacy = PrivacyMode::Offline;
        let outcome = adapter.call(
            &offline,
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert_eq!(adapter.socket_attempts(), 0, "offline dials nothing");
        assert!(!outcome.dispatched_remote);
    }

    #[test]
    fn closed_port_returns_retryable_refusal() {
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let outcome = adapter.call(
            &request(5, 3),
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(AiError::Unavailable.retryable());
        assert_eq!(adapter.socket_attempts(), 1);
        assert_eq!(adapter.transport_calls(), 0, "no exchange completed");
        assert!(!outcome.dispatched_remote);
        assert!(!outcome.mutated);
    }

    #[test]
    fn transport_timeout_maps_to_timeout() {
        let stub = StubServer::spawn_delayed(
            crate::stub::chat_ok(r#"{"actions": [], "diagnostics": []}"#, None),
            400,
        );
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                let mut slow = request(6 + n as u64, 3);
                slow.budget.max_ms = 100;
                adapter.call(&slow, &policy_for(&url), &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::Timeout));
        assert!(!outcome.mutated);
    }

    #[test]
    fn redirect_is_refused_without_following() {
        let nowhere = format!("http://127.0.0.1:{}/elsewhere", closed_port());
        let stub = StubServer::spawn_redirect(&nowhere);
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                adapter.call(&request(7 + n as u64, 3), &policy_for(&url), &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert_eq!(stub.hits(), 1, "redirect target never dialed");
        assert!(!outcome.mutated);
    }

    #[test]
    fn malformed_stub_body_is_malformed_output() {
        let stub = StubServer::spawn_raw(200, "this is not json".to_owned());
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let (outcome, adapter) = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                let outcome = adapter.call(&request(8 + n as u64, 3), &policy_for(&url), &caps);
                (outcome, adapter)
            },
            |(o, _)| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::MalformedOutput));
        assert!(!outcome.mutated);
        assert_eq!(
            adapter.transport_calls(),
            1,
            "exchange completed, parse failed"
        );
    }

    #[test]
    fn server_quota_maps_to_quota_auth() {
        let stub = StubServer::spawn_raw(429, String::from("{}"));
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                adapter.call(&request(9 + n as u64, 3), &policy_for(&url), &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!outcome.mutated);
    }

    #[test]
    fn openai_stream_job_streams_through_adapter() {
        use crate::announce::{announce_from_stream, EventKindLite, EventLite, FigLite};
        use crate::stream::{SourceLabel, StreamConfig, StreamJob, StreamPoll};

        const SSE: &str = "data: {\"choices\": [{\"delta\": {\"content\": \"Steel \"}}]}\n\ndata: {\"choices\": [{\"delta\": {\"content\": \"glints!\"}}]}\n\ndata: [DONE]\n\n";
        let stub = StubServer::spawn_completion(SSE.to_owned());
        let url = stub.url();
        let adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let config = StreamConfig::new(16, 8192, 5_000, 8).expect("fixture config valid");
        // Streaming flag travels in the POST body, not as a provider field.
        let body = super::stream_body(&request(60, 3), &RequestPins::default());
        assert!(
            body.contains("\"stream\":true"),
            "adapter POSTs with streaming flag"
        );
        let mut job: StreamJob = adapter
            .stream_job(&request(60, 3), &config)
            .expect("loopback stream spawns");
        // Loopback stubs are recorded fixtures, never live.
        assert_eq!(job.source(), SourceLabel::Recorded);
        assert!(!job.source().is_live());
        let mut terminal = StreamPoll::Pending;
        for _ in 0..500 {
            terminal = job.poll();
            if terminal != StreamPoll::Pending {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(terminal, StreamPoll::Done);
        assert_eq!(job.assembled(), "Steel glints!");
        assert_eq!(job.chunks(), 2);
        assert_eq!(stub.hits(), 1, "stream crossed real TCP once");
        assert_eq!(stub.auth_hits(), 1, "stream carries session auth");
        // Announcer consumer: completed stream text becomes an announce line.
        let mut announcer = crate::announce::Announcer::new(3);
        let obs = crate::announce::MatchObservation {
            tick: 77,
            fighters: vec![FigLite {
                id: 1,
                hp_permille: 640,
                down: false,
            }],
            last_event: Some(EventLite {
                kind: EventKindLite::Knockdown,
                fighter: 1,
            }),
        };
        let line = announce_from_stream(&mut announcer, &obs, job.assembled());
        assert!(!line.scripted, "stream text reads as a provider line");
        assert_eq!(line.text, "Steel glints!");
        assert_eq!(line.priority, 2);
    }

    #[test]
    fn openai_stream_job_cancel_timeout_offline_through_adapter() {
        use crate::stream::{SourceLabel, StreamConfig, StreamError, StreamJob, StreamPoll};

        fn terminal_of(job: &mut StreamJob) -> StreamPoll {
            let mut last = StreamPoll::Pending;
            for _ in 0..500 {
                last = job.poll();
                if last != StreamPoll::Pending {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            last
        }
        let config = StreamConfig::new(16, 8192, 5_000, 8).expect("fixture config valid");
        // Offline: refused before spawning, no socket.
        let port = crate::stub::closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let offline_adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let mut offline_req = request(70, 3);
        offline_req.privacy = PrivacyMode::Offline;
        assert!(matches!(
            offline_adapter.stream_job(&offline_req, &config),
            Err(StreamError::Offline)
        ));
        // Cancel: 20 ms delayed stub leaves the flag set before the worker
        // pushes, so cancel lands well under 100 ms through the adapter.
        const SSE: &str =
            "data: {\"choices\": [{\"delta\": {\"content\": \"hi\"}}]}\n\ndata: [DONE]\n\n";
        let slow = StubServer::spawn_delayed(SSE.to_owned(), 20);
        let slow_url = slow.url();
        let cancel_adapter = adapter_for(&slow_url, vec![MockStep::Succeed]);
        let mut job = cancel_adapter
            .stream_job(&request(71, 3), &config)
            .expect("cancel stream spawns");
        assert_eq!(job.source(), SourceLabel::Recorded);
        let start = std::time::Instant::now();
        job.cancel();
        assert_eq!(
            terminal_of(&mut job),
            StreamPoll::Failed(StreamError::Cancelled)
        );
        assert!(
            start.elapsed() < std::time::Duration::from_millis(100),
            "adapter cancel under 100 ms"
        );
        assert_eq!(StreamError::Cancelled.to_ai_error(), AiError::Cancelled);
        // Timeout: 400 ms stub against a 100 ms budget fails as Timeout.
        let tardy = StubServer::spawn_delayed(SSE.to_owned(), 400);
        let tardy_url = tardy.url();
        let timeout_adapter = adapter_for(&tardy_url, vec![MockStep::Succeed]);
        let mut slow_req = request(72, 3);
        slow_req.budget.max_ms = 100;
        let mut job = timeout_adapter
            .stream_job(&slow_req, &config)
            .expect("timeout stream spawns");
        assert_eq!(
            terminal_of(&mut job),
            StreamPoll::Failed(StreamError::Timeout)
        );
        // Unavailable: unreachable endpoint never completes an exchange.
        // Port 0 refuses fast (measured 190 us to Connect, no packet
        // leaves the host) and needs no ephemeral port, so this leg never
        // depends on worker-thread dial timing: immune to the slow
        // refused-RST latency some Windows stacks show on freed loopback
        // ports (measured 2.5 s to ConnectionRefused) and to port reuse
        // between parallel tests. Same connect-failure path, same variant.
        let dead_url = "http://127.0.0.1:0/v1/chat/completions";
        let dead_adapter = adapter_for(dead_url, vec![MockStep::Succeed]);
        let mut job = dead_adapter
            .stream_job(&request(73, 3), &config)
            .expect("unavailable stream spawns");
        assert_eq!(
            terminal_of(&mut job),
            StreamPoll::Failed(StreamError::Unavailable)
        );
        // Malformed: 200 with non-SSE bytes yields no delta.
        let bad = StubServer::spawn_raw(200, "this is not json".to_owned());
        let bad_url = bad.url();
        let bad_adapter = adapter_for(&bad_url, vec![MockStep::Succeed]);
        let mut job = bad_adapter
            .stream_job(&request(74, 3), &config)
            .expect("malformed stream spawns");
        assert_eq!(
            terminal_of(&mut job),
            StreamPoll::Failed(StreamError::MalformedOutput)
        );
    }

    /// Needs an operator-configured OpenAI-compatible host: `E2040_AX1_OPENAI_URL`
    /// plus a bearer token in `E2040_AX1_OPENAI_TOKEN`. Unverified in CI by
    /// design, never faked with a stub.
    #[test]
    #[ignore = "live: needs E2040_AX1_OPENAI_URL and a bearer token"]
    fn live_opt_in_only() {
        let url = match std::env::var("E2040_AX1_OPENAI_URL") {
            Ok(url) if !url.is_empty() => url,
            _ => return,
        };
        let endpoint = transport::parse_endpoint(&url).expect("live URL parses");
        let mut adapter = OpenAiAdapter::with_env_token(
            vec![MockStep::Succeed],
            endpoint,
            "E2040_AX1_OPENAI_TOKEN",
        );
        let policy = policy_for(&url);
        let outcome = adapter.call(
            &request(901, 3),
            &policy,
            &ProviderCapabilities::remote_http(),
        );
        assert!(outcome.result.is_ok(), "live host serves: {outcome:?}");
    }
}
