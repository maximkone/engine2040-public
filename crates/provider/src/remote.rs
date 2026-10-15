//! remote: configured-host adapter with adapter-local credentials.
//! Identical transport to the local adapter plus a session-auth header: the
//! bearer token comes from an adapter-local [`SecretRef`] (environment
//! variable or held value), is resolved at call time, travels in the
//! `Authorization` header only, and never appears in errors, events,
//! diagnostics or `Debug` output. Missing or rejected credentials surface as
//! [`AiError::QuotaAuth`] without a payload. Like the local adapter, text
//! responses are parsed and validated, never claimed as native structured
//! output.

use super::core::{AdapterOutcome, MockStep};
use super::ladder::RungSupport;
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::AiRequest;
use super::transport::{self, Endpoint};
use super::vendor::{http_adapter_api, HttpAdapter, VendorSecret, Wire, CHAT_ENVELOPE};

/// Adapter-local secret reference: the one credential holder shared with the
/// vendor adapters ([`VendorSecret`]). The value leaves it only as header
/// bytes on the wire: `Debug` prints the reference kind (and the variable
/// name for `EnvVar`), never the value.
pub type SecretRef = VendorSecret;

/// Chat-completions wire against the configured host: completed exchanges
/// are remote dispatches.
const WIRE: Wire = Wire {
    label: "remote",
    body: transport::chat_body,
    envelope: &CHAT_ENVELOPE,
    remote: true,
    rungs: RungSupport::FULL,
};

/// Why credentials were unusable. Carries no payload by construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SecretError {
    /// Variable unset/empty, or the held token empty.
    Unavailable,
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::Unavailable => f.write_str("session credential unavailable"),
        }
    }
}

/// Remote adapter: OpenAI-compatible `/v1/chat/completions` against a
/// configured host, authenticated per session, with the contract suite
/// running unchanged through the shared contract state.
#[derive(Clone, Debug)]
pub struct RemoteAdapter {
    http: HttpAdapter,
    auth: SecretRef,
}

impl RemoteAdapter {
    /// Build an adapter for `endpoint` with an adapter-local credential.
    /// Session authorization (remote-allowed privacy and policy) is enforced
    /// per call, before any socket attempt.
    pub fn new(script: Vec<MockStep>, endpoint: Endpoint, auth: SecretRef) -> Self {
        Self {
            http: HttpAdapter::new(script, endpoint),
            auth,
        }
    }

    http_adapter_api!();

    /// Completed HTTP exchanges with the configured endpoint.
    pub fn remote_calls(&self) -> u32 {
        self.http.remote_calls()
    }
}

impl ProviderAdapter for RemoteAdapter {
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
        inner.model.model = String::from("hosted-1");
        inner.privacy = PrivacyMode::RemoteAllowed;
        // Cost stays unknown unless a case funds it: the suite's spend-cap
        // case asserts the unknown-cost refusal, then funds explicitly.
        inner
    }

    fn adapter_for(url: &str, script: Vec<MockStep>) -> RemoteAdapter {
        let endpoint = transport::parse_endpoint(url).expect("test endpoint parses");
        RemoteAdapter::new(
            script,
            endpoint,
            SecretRef::Direct(String::from("suite-token")),
        )
    }

    #[test]
    fn pv1_failure_suite_passes_unchanged_through_remote() {
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
        assert!(stub.auth_hits() > 0, "remote sends session auth");
    }

    #[test]
    fn stub_roundtrip_receipt_is_labeled_loopback() {
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
        assert_eq!(stub.paths(), vec![String::from("/v1/chat/completions")]);
        assert_eq!(stub.auth_hits(), 1, "session auth header present");
        let result = outcome.result.as_ref().expect("success must complete");
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line == "remote:127.0.0.1"),
            "receipt labels the loopback host: {proposal:?}"
        );
        assert_eq!(result.usage.input_tokens, Some(11));
    }

    #[test]
    fn secrets_never_appear_in_diagnostics() {
        let canary = "e2040-canary-9f2c-remote-secret";
        crate::test_env::set("E2040_PV2_REDACT_A", canary);
        let stub = StubServer::spawn_raw(401, String::from("{}"));
        let url = stub.url();
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = RemoteAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            SecretRef::EnvVar(String::from("E2040_PV2_REDACT_A")),
        );
        let outcome = adapter.call(
            &request(2, 3),
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        // The 401 refusal, the adapter state and the secret itself: none may
        // carry the value.
        let exposed = format!("{outcome:?} {adapter:?}");
        assert!(!exposed.contains(canary), "refusal leaks the secret");
        let secret_debug = format!("{:?}", SecretRef::Direct(String::from(canary)));
        assert!(!secret_debug.contains(canary), "Direct Debug redacts");
        let env_debug = format!(
            "{:?}",
            SecretRef::EnvVar(String::from("E2040_PV2_REDACT_A"))
        );
        assert!(!env_debug.contains(canary), "EnvVar Debug names only");
        crate::test_env::remove("E2040_PV2_REDACT_A");
    }

    #[test]
    fn missing_credential_refuses_without_socket() {
        crate::test_env::remove("E2040_PV2_REDACT_MISSING");
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = RemoteAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            SecretRef::EnvVar(String::from("E2040_PV2_REDACT_MISSING")),
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
        let mut adapter = RemoteAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            SecretRef::Direct(String::from("offline-token")),
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
    fn local_only_request_never_reaches_the_remote_socket() {
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
            r#"{"actions": [], "diagnostics": []}"#,
            None,
        ));
        let url = stub.url();
        let endpoint = transport::parse_endpoint(&url).expect("test endpoint parses");
        let mut adapter = RemoteAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            SecretRef::Direct(String::from("downgrade-token")),
        );
        let mut downgraded = request(5, 3);
        downgraded.privacy = PrivacyMode::LocalOnly;
        let outcome = adapter.call(
            &downgraded,
            &policy_for(&url),
            &ProviderCapabilities::remote_http(),
        );
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert_eq!(stub.hits(), 0, "downgrade refused before the socket");
        assert_eq!(adapter.socket_attempts(), 0);
        assert!(!outcome.dispatched_remote);
    }

    /// Needs an operator-configured remote host with a bearer token in
    /// `E2040_PV2_LIVE_TOKEN`: unverified in CI by design, never faked.
    #[test]
    #[ignore = "live: needs an operator remote host and E2040_PV2_LIVE_TOKEN"]
    fn remote_live_configured_host() {
        let endpoint = transport::parse_endpoint("http://127.0.0.1:8080/v1/chat/completions")
            .expect("live URL parses");
        let mut adapter = RemoteAdapter::new(
            vec![MockStep::Succeed],
            endpoint,
            SecretRef::EnvVar(String::from("E2040_PV2_LIVE_TOKEN")),
        );
        let policy = policy_for("http://127.0.0.1:8080");
        let outcome = adapter.call(
            &request(901, 3),
            &policy,
            &ProviderCapabilities::remote_http(),
        );
        assert!(outcome.result.is_ok(), "live remote serves: {outcome:?}");
    }
}
