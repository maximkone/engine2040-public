//! gemini: Gemini generateContent vendor adapter.
//! Request body follows `generateContent` (`contents[].parts[].text` plus
//! `generationConfig.maxOutputTokens`); the assistant text comes from
//! `candidates[0].content.parts[0].text` and usage from
//! `usageMetadata.promptTokenCount`/`candidatesTokenCount`. The bearer token
//! is adapter-local ([`crate::vendor::VendorSecret`]), resolved at call time,
//! sent in the `Authorization` header only, and never appears in errors,
//! events, diagnostics or `Debug` output. Contract guards, jobs and scripted
//! failure arms run through the shared contract state so the contract suite
//! ports unchanged. Default tests use loopback stubs; live hosts are opt-in
//! with `E2040_AX1_GEMINI_URL` plus `E2040_AX1_GEMINI_TOKEN`.

use super::core::{AdapterOutcome, MockStep};
use super::ladder::RungSupport;
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::AiRequest;
use super::transport::Endpoint;
use super::vendor::{http_adapter_api, Envelope, HttpAdapter, VendorSecret, Wire};

/// Production path shape for Gemini generateContent (model segments elided;
/// the configured endpoint carries the full path).
pub const PRODUCTION_PATH: &str = "/v1beta/models/gemini-fixture:generateContent";

/// Where a generateContent response keeps its text and usage counters.
const ENVELOPE: Envelope = Envelope {
    text: "/candidates/0/content/parts/0/text",
    input_tokens: "/usageMetadata/promptTokenCount",
    output_tokens: "/usageMetadata/candidatesTokenCount",
    native_tool: false,
};

/// Single-shot wire: remote dispatch, strict proposal parse.
const WIRE: Wire = Wire {
    label: "gemini",
    body: request_body,
    envelope: &ENVELOPE,
    remote: true,
    rungs: RungSupport::FULL,
};

/// Build the generateContent request body under `pins`: system instruction,
/// temperature and seed in `generationConfig`, function declarations. The
/// prompt carries digests only; no secret ever enters the body. Manual string
/// building over the pin fields keeps the same keys, escaping and order.
fn request_body(request: &AiRequest, pins: &RequestPins) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (key, value) in pins.generate_fields(request) {
        parts.push(format!("\"{key}\":{value}"));
    }
    format!("{{{}}}", parts.join(","))
}

/// Gemini generateContent vendor adapter over the owned `std::net::TcpStream`
/// client, with the contract suite running unchanged.
#[derive(Clone, Debug)]
pub struct GeminiAdapter {
    http: HttpAdapter,
    auth: VendorSecret,
}

impl GeminiAdapter {
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
}

impl ProviderAdapter for GeminiAdapter {
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
crate::vendor::impl_suite_adapter!(GeminiAdapter);

#[cfg(test)]
mod tests {
    use super::super::policy::ProviderCapabilities;
    use super::super::protocol::{AdapterKind, AiError, PrivacyMode};
    use super::super::transport;
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
        inner.model.model = String::from("gemini:gemini-fixture");
        inner.privacy = PrivacyMode::RemoteAllowed;
        inner
    }

    /// generateContent-shaped stub body whose first part text is `inner_json`
    /// (itself a strict proposal object). Usage attaches only when given, so
    /// suites prove unknown stays unknown. Manual string building with JSON
    /// string escaping for the embedded proposal text.
    fn json_escape_into(out: &mut String, text: &str) {
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
    }

    fn generate_content_ok(inner_json: &str, usage: Option<(u64, u64)>) -> String {
        let mut body = String::from("{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"");
        json_escape_into(&mut body, inner_json);
        body.push_str("\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]");
        if let Some((input, output)) = usage {
            body.push_str(&format!(
                ",\"usageMetadata\":{{\"promptTokenCount\":{input},\"candidatesTokenCount\":{output}}}"
            ));
        }
        body.push('}');
        body
    }

    fn adapter_for(url: &str, script: Vec<MockStep>) -> GeminiAdapter {
        let endpoint = transport::parse_endpoint(url).expect("test endpoint parses");
        GeminiAdapter::new(
            script,
            endpoint,
            VendorSecret::Direct(String::from("suite-token")),
        )
    }

    #[test]
    fn pv1_failure_suite_passes_through_gemini() {
        // Usage-free stub: unknown cost stays unknown, as in the mock suite.
        let stub = StubServer::spawn_completion(generate_content_ok(
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
        assert!(stub.auth_hits() > 0, "gemini sends session auth");
    }

    #[test]
    fn stub_roundtrip_receipt_and_usage() {
        let stub = StubServer::spawn_completion(generate_content_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["stub:ok"]}"#,
            Some((17, 6)),
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
        assert_eq!(stub.hits(), 1, "exactly one TCP roundtrip happened");
        assert_eq!(stub.auth_hits(), 1, "session auth header present");
        let result = outcome.result.as_ref().expect("success must complete");
        assert_eq!(result.usage.input_tokens, Some(17));
        assert_eq!(result.usage.output_tokens, Some(6));
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line == "gemini:127.0.0.1"),
            "receipt labels the vendor plus host: {proposal:?}"
        );
    }

    #[test]
    fn request_body_is_generate_content_shaped() {
        let body = request_body(&request(1, 3), &RequestPins::default());
        assert!(body.contains("\"contents\""), "contents envelope");
        assert!(body.contains("revision=3"), "prompt carries the revision");
        assert!(
            body.contains("\"generationConfig\"") && body.contains("\"maxOutputTokens\""),
            "generation bound present"
        );
    }

    #[test]
    fn body_carries_the_pins() {
        let ask = request(1, 3);
        let pins = RequestPins::default()
            .with_temperature_milli(300)
            .and_then(|p| {
                p.with_tools(vec![crate::pins::ToolDef::new(
                    "move",
                    "Move one object.",
                    crate::ladder::action_schema("noop").expect("noop schema is an object"),
                )?])
            })
            .expect("pins in bounds");
        let body = request_body(&ask, &pins);
        assert!(
            body.contains("You diagnose one staged finding"),
            "system instruction carries the task kind prompt"
        );
        assert!(body.contains("\"temperature\":0.3"));
        assert!(body.contains("\"seed\":2040"));
        assert!(body.contains("\"functionDeclarations\""));
        assert!(body.contains("\"name\":\"move\""));
        let default = request_body(&ask, &RequestPins::default());
        assert!(default.contains("\"temperature\":0.0"), "greedy default");
    }

    #[test]
    fn messages_envelope_is_rejected_as_malformed() {
        // The wrong vendor shape is not a proposal: strict parsing refuses it.
        let mut wrong = String::from("{\"content\":[{\"type\":\"text\",\"text\":\"");
        json_escape_into(&mut wrong, r#"{"actions": [], "diagnostics": []}"#);
        wrong.push_str("\"}]}");
        let stub = StubServer::spawn_completion(wrong);
        let url = stub.url();
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let outcome = adapter.call(
            &request(2, 3),
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::MalformedOutput));
        assert!(!outcome.mutated);
    }

    #[test]
    fn secrets_never_appear_in_diagnostics() {
        let canary = "e2040-canary-c03d-gemini-secret";
        crate::test_env::set("E2040_AX1_REDACT_GEMINI", canary);
        let stub = StubServer::spawn_raw(401, String::from("{}"));
        let url = stub.url();
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        // Loopback dials can refuse under full-suite parallel load before any
        // byte moves (Unavailable); retry only that setup failure so a
        // genuine mapping regression still fails below.
        let (outcome, adapter) = crate::stub::settle(
            |n| {
                let mut adapter = GeminiAdapter::new(
                    vec![MockStep::Succeed],
                    endpoint.clone(),
                    VendorSecret::EnvVar(String::from("E2040_AX1_REDACT_GEMINI")),
                );
                let outcome = adapter.call(
                    &request(3 + u64::from(n), 3),
                    &policy_for(&url),
                    &ProviderCapabilities::remote_http(),
                );
                (outcome, adapter)
            },
            |(o, _)| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        let exposed = format!("{outcome:?} {adapter:?}");
        assert!(!exposed.contains(canary), "refusal leaks the secret");
        crate::test_env::remove("E2040_AX1_REDACT_GEMINI");
    }

    #[test]
    fn missing_credential_refuses_without_socket() {
        crate::test_env::remove("E2040_AX1_MISSING_GEMINI");
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/v1beta/models/x:generateContent");
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = GeminiAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            VendorSecret::EnvVar(String::from("E2040_AX1_MISSING_GEMINI")),
        );
        let outcome = adapter.call(
            &request(4, 3),
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
        let url = format!("http://127.0.0.1:{port}/v1beta/models/x:generateContent");
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = GeminiAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            VendorSecret::Direct(String::from("offline-token")),
        );
        let mut offline = request(5, 3);
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
        let url = format!("http://127.0.0.1:{port}/v1beta/models/x:generateContent");
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let outcome = adapter.call(
            &request(6, 3),
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(AiError::Unavailable.retryable());
        assert_eq!(adapter.socket_attempts(), 1);
        assert_eq!(adapter.transport_calls(), 0, "no exchange completed");
        assert!(!outcome.mutated);
    }

    #[test]
    fn transport_timeout_maps_to_timeout() {
        let stub = StubServer::spawn_delayed(
            generate_content_ok(r#"{"actions": [], "diagnostics": []}"#, None),
            400,
        );
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                let mut slow = request(7 + n as u64, 3);
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
                adapter.call(&request(8 + n as u64, 3), &policy_for(&url), &caps)
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
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                adapter.call(&request(9 + n as u64, 3), &policy_for(&url), &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::MalformedOutput));
        assert!(!outcome.mutated);
    }

    #[test]
    fn server_quota_maps_to_quota_auth() {
        let stub = StubServer::spawn_raw(429, String::from("{}"));
        let url = stub.url();
        let caps = ProviderCapabilities::remote_http();
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
                adapter.call(&request(10 + n as u64, 3), &policy_for(&url), &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!outcome.mutated);
    }

    /// Needs an operator-configured Gemini host: `E2040_AX1_GEMINI_URL` plus
    /// a bearer token in `E2040_AX1_GEMINI_TOKEN`. Unverified in CI by
    /// design, never faked with a stub.
    #[test]
    #[ignore = "live: needs E2040_AX1_GEMINI_URL and a bearer token"]
    fn live_opt_in_only() {
        let url = match std::env::var("E2040_AX1_GEMINI_URL") {
            Ok(url) if !url.is_empty() => url,
            _ => return,
        };
        let endpoint = transport::parse_endpoint(&url).expect("live URL parses");
        let mut adapter = GeminiAdapter::with_env_token(
            vec![MockStep::Succeed],
            endpoint,
            "E2040_AX1_GEMINI_TOKEN",
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
