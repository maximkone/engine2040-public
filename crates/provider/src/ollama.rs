//! ollama: Ollama local chat vendor adapter.
//! Request body follows `/api/chat` (`model`, `messages`, `stream: false`);
//! the assistant text comes from `message.content` and usage from
//! `prompt_eval_count`/`eval_count`. Loopback endpoints only: construction
//! refuses any non-loopback host, the adapter holds no secret and no remote
//! endpoint, so a local-only failure physically cannot escalate to a cloud
//! host. Offline requests are refused before any socket attempt. Contract
//! guards, jobs and scripted failure arms run through the shared
//! contract state so the contract suite ports unchanged. Default tests use
//! loopback stubs; a live daemon is opt-in with `E2040_AX1_OLLAMA_URL`.

use super::core::{AdapterOutcome, MockStep};
use super::ladder::RungSupport;
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::AiRequest;
use super::transport::{self, Endpoint};
use super::vendor::{http_adapter_api, Envelope, HttpAdapter, Wire};

/// Why a local endpoint was rejected at construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OllamaConfigError {
    /// Only loopback hosts serve the Ollama adapter.
    NonLoopback,
}

impl std::fmt::Display for OllamaConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OllamaConfigError::NonLoopback => f.write_str("ollama endpoint must be loopback"),
        }
    }
}

/// Production path for Ollama chat.
pub const PRODUCTION_PATH: &str = "/api/chat";

/// Where an Ollama chat response keeps its text and usage counters.
const ENVELOPE: Envelope = Envelope {
    text: "/message/content",
    input_tokens: "/prompt_eval_count",
    output_tokens: "/eval_count",
    native_tool: false,
};

/// Single-shot wire: loopback only, so no remote dispatch and no credential.
const WIRE: Wire = Wire {
    label: "ollama",
    body: request_body,
    envelope: &ENVELOPE,
    remote: false,
    rungs: RungSupport::FULL,
};

/// Build the Ollama chat request body under `pins`: system and user messages,
/// temperature and seed under `options`, function tools. The prompt carries
/// digests only and streaming stays off so one call yields one terminal
/// outcome.
fn request_body(request: &AiRequest, pins: &RequestPins) -> String {
    serde_json::Value::Object(pins.ollama_fields(request)).to_string()
}

/// Ollama local vendor adapter over the owned `std::net::TcpStream` client,
/// with the contract suite running unchanged.
#[derive(Clone, Debug)]
pub struct OllamaAdapter {
    http: HttpAdapter,
}

impl OllamaAdapter {
    /// Build an adapter for `endpoint`. Non-loopback hosts are refused here,
    /// before any request can name them.
    pub fn new(script: Vec<MockStep>, endpoint: Endpoint) -> Result<Self, OllamaConfigError> {
        if !transport::is_loopback(endpoint.host()) {
            return Err(OllamaConfigError::NonLoopback);
        }
        Ok(Self {
            http: HttpAdapter::new(script, endpoint),
        })
    }

    http_adapter_api!();

    /// Remote endpoint contacts. Always zero: local loopback calls are not
    /// remote dispatches.
    pub fn remote_calls(&self) -> u32 {
        0
    }
}

impl ProviderAdapter for OllamaAdapter {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        self.http.call(&WIRE, None, request, policy, capabilities)
    }

    fn request_pins(&self) -> Option<&RequestPins> {
        Some(&self.http.pins)
    }
}

#[cfg(test)]
crate::vendor::impl_suite_adapter!(OllamaAdapter);

#[cfg(test)]
mod tests {
    use super::super::policy::ProviderCapabilities;
    use super::super::protocol::{AdapterKind, AiError, PrivacyMode};
    use super::*;
    use crate::stub::{closed_port, StubServer};
    use crate::suite::{run_failure_suite, SuiteAdapter, SuiteExpect};

    pub(crate) fn policy() -> AiSessionPolicy {
        AiSessionPolicy {
            adapters: vec![AdapterKind::Local],
            mode: PrivacyMode::LocalOnly,
            destinations: Vec::new(),
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
        inner.model.adapter = AdapterKind::Local;
        inner.model.model = String::from("ollama:llama-fixture");
        inner
    }

    /// Ollama-shaped stub body whose message content is `inner_json` (itself
    /// a strict proposal object). Usage attaches only when given, so suites
    /// prove unknown stays unknown.
    fn chat_ok(inner_json: &str, usage: Option<(u64, u64)>) -> String {
        let mut envelope = serde_json::json!({
            "model": "llama-fixture",
            "message": {"role": "assistant", "content": inner_json},
            "done": true,
        });
        if let Some((input, output)) = usage {
            envelope["prompt_eval_count"] = serde_json::json!(input);
            envelope["eval_count"] = serde_json::json!(output);
        }
        envelope.to_string()
    }

    fn adapter_for(url: &str, script: Vec<MockStep>) -> OllamaAdapter {
        let endpoint = transport::parse_endpoint(url).expect("test endpoint parses");
        OllamaAdapter::new(script, endpoint).expect("loopback endpoint accepted")
    }

    #[test]
    fn pv1_failure_suite_passes_through_ollama() {
        // Usage-free stub: unknown cost stays unknown, as in the mock suite.
        let stub = StubServer::spawn_completion(chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["suite:stub"]}"#,
            None,
        ));
        let url = stub.url();
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        assert!(!caps.structured_output, "text parse is never native JSON");
        run_failure_suite(
            &|script| Box::new(adapter_for(&url, script)) as Box<dyn SuiteAdapter>,
            &policy,
            &caps,
            &request,
            &|id| {
                let mut downgraded = request(id, 3);
                downgraded.model.adapter = AdapterKind::Remote;
                downgraded.privacy = PrivacyMode::LocalOnly;
                downgraded
            },
            &SuiteExpect {
                remote_dispatch_on_success: false,
                receipt_host: String::from("127.0.0.1"),
            },
        );
        assert!(
            stub.hits() > 0,
            "suite success arms crossed real TCP, not the mock"
        );
    }

    #[test]
    fn stub_roundtrip_receipt_and_usage() {
        let stub = StubServer::spawn_completion(chat_ok(
            r#"{"actions": [{"set_label": {"object": 2, "label": "gate"}}], "diagnostics": ["stub:ok"]}"#,
            Some((7, 3)),
        ));
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let (outcome, adapter) = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                let outcome = adapter.call(&request(1 + n as u64, 3), &policy, &caps);
                (outcome, adapter)
            },
            |(o, _)| o.result.is_ok(),
        );
        let result = outcome.result.as_ref().expect("stub roundtrip succeeds");
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        crate::mock::validate_proposal(proposal).expect("stub proposal validates");
        assert!(outcome.mutated);
        assert!(
            !outcome.dispatched_remote,
            "loopback is not a remote dispatch"
        );
        assert_eq!(adapter.remote_calls(), 0);
        assert_eq!(adapter.socket_attempts(), 1);
        assert_eq!(adapter.transport_calls(), 1);
        assert_eq!(stub.hits(), 1, "exactly one TCP roundtrip happened");
        assert_eq!(stub.auth_hits(), 0, "local sends no session auth");
        assert_eq!(result.usage.input_tokens, Some(7));
        assert_eq!(result.usage.output_tokens, Some(3));
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line == "ollama:127.0.0.1"),
            "receipt labels the vendor plus host: {proposal:?}"
        );
    }

    #[test]
    fn request_body_is_ollama_shaped() {
        let body = request_body(&request(1, 3), &RequestPins::default());
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
        assert!(parsed["messages"].is_array(), "messages envelope");
        assert_eq!(
            parsed["messages"][0]["role"], "system",
            "pinned system prompt"
        );
        assert_eq!(parsed["messages"][1]["role"], "user", "chat role preserved");
        assert!(
            parsed["messages"][1]["content"]
                .as_str()
                .unwrap_or_default()
                .contains("revision=3"),
            "prompt carries the revision"
        );
        assert_eq!(parsed["stream"], false, "one call, one outcome");
    }

    #[test]
    fn body_carries_the_pins() {
        let ask = request(1, 3);
        let pins = RequestPins::default()
            .with_system("Answer in JSON.")
            .and_then(|p| p.with_temperature_milli(300))
            .and_then(|p| {
                p.with_tools(vec![crate::pins::ToolDef::new(
                    "move",
                    "Move one object.",
                    serde_json::json!({"type": "object"}),
                )?])
            })
            .expect("pins in bounds");
        let parsed: serde_json::Value =
            serde_json::from_str(&request_body(&ask, &pins)).expect("body is JSON");
        assert_eq!(parsed["messages"][0]["content"], "Answer in JSON.");
        assert_eq!(parsed["options"]["temperature"].as_f64(), Some(0.3));
        assert_eq!(parsed["options"]["seed"], crate::PINNED_SEED);
        assert_eq!(parsed["tools"][0]["function"]["name"], "move");
        let default: serde_json::Value =
            serde_json::from_str(&request_body(&ask, &RequestPins::default()))
                .expect("body is JSON");
        assert_eq!(
            default["options"]["temperature"].as_f64(),
            Some(0.0),
            "greedy default"
        );
        assert_eq!(default["stream"], false, "one call, one outcome");
    }

    #[test]
    fn chat_completions_envelope_is_rejected_as_malformed() {
        // The wrong vendor shape is not a proposal: strict parsing refuses it.
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": []}"#,
            None,
        ));
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                adapter.call(&request(2 + n as u64, 3), &policy, &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::MalformedOutput));
        assert!(!outcome.mutated);
    }

    #[test]
    fn offline_privacy_makes_no_socket_attempt() {
        // Closed port proves the point: an Offline request is refused before
        // any dial, so even an unreachable endpoint records zero attempts.
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/api/chat");
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let mut offline = request(3, 3);
        offline.privacy = PrivacyMode::Offline;
        let outcome = adapter.call(&offline, &policy(), &ProviderCapabilities::local_http());
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert_eq!(adapter.socket_attempts(), 0, "offline dials nothing");
        assert_eq!(adapter.transport_calls(), 0);
        assert!(!outcome.dispatched_remote);
        assert!(!outcome.mutated);
    }

    #[test]
    fn closed_port_local_only_returns_refusal_without_cloud() {
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/api/chat");
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        assert!(
            transport::is_loopback(adapter.endpoint().host()),
            "ollama adapter only holds loopback endpoints"
        );
        let outcome = adapter.call(
            &request(4, 3),
            &policy(),
            &ProviderCapabilities::local_http(),
        );
        // A refused loopback dial: Windows retries it for about 2 s, so the dial's
        // own timeout can answer first there; both are retryable refusals.
        assert!(
            matches!(outcome.result, Err(AiError::Unavailable | AiError::Timeout)),
            "{:?}",
            outcome.result
        );
        assert!(AiError::Unavailable.retryable() && AiError::Timeout.retryable());
        assert_eq!(adapter.socket_attempts(), 1, "the loopback dial was tried");
        assert_eq!(adapter.transport_calls(), 0, "no exchange completed");
        assert!(!outcome.dispatched_remote);
        assert_eq!(adapter.remote_calls(), 0, "never escalates to cloud");
        assert!(!outcome.mutated);
    }

    #[test]
    fn transport_timeout_maps_to_timeout() {
        let stub =
            StubServer::spawn_delayed(chat_ok(r#"{"actions": [], "diagnostics": []}"#, None), 400);
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                let mut slow = request(5 + n as u64, 3);
                slow.budget.max_ms = 100;
                adapter.call(&slow, &policy, &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::Timeout));
        assert!(!outcome.mutated);
    }

    #[test]
    fn redirect_is_refused_without_following() {
        // 301 with a Location pointing at a closed port: following it would
        // dial again, so hits must stay at exactly one.
        let nowhere = format!("http://127.0.0.1:{}/elsewhere", closed_port());
        let stub = StubServer::spawn_redirect(&nowhere);
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                adapter.call(&request(6 + n as u64, 3), &policy, &caps)
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
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let (outcome, adapter) = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                let outcome = adapter.call(&request(7 + n as u64, 3), &policy, &caps);
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
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                adapter.call(&request(8 + n as u64, 3), &policy, &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!outcome.mutated);
    }

    #[test]
    fn non_loopback_url_is_rejected_before_any_request() {
        let endpoint = transport::parse_endpoint("http://203.0.113.7:11434/api/chat")
            .expect("URL shape parses");
        assert!(
            matches!(
                OllamaAdapter::new(vec![MockStep::Succeed], endpoint),
                Err(OllamaConfigError::NonLoopback)
            ),
            "non-loopback ollama endpoint refused"
        );
    }

    /// Needs an operator-run daemon (`ollama serve` on 127.0.0.1:11434, or a
    /// custom URL in `E2040_AX1_OLLAMA_URL`): unverified in CI by design,
    /// never faked with a stub.
    #[test]
    #[ignore = "live: needs a running Ollama server"]
    fn live_opt_in_only() {
        let url = std::env::var("E2040_AX1_OLLAMA_URL")
            .unwrap_or_else(|_| String::from("http://127.0.0.1:11434/api/chat"));
        let endpoint = transport::parse_endpoint(&url).expect("daemon URL parses");
        let mut adapter = match OllamaAdapter::new(vec![MockStep::Succeed], endpoint) {
            Ok(adapter) => adapter,
            Err(_) => return,
        };
        let outcome = adapter.call(
            &request(900, 3),
            &policy(),
            &ProviderCapabilities::local_http(),
        );
        assert!(outcome.result.is_ok(), "live daemon serves: {outcome:?}");
    }
}
