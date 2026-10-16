//! local: llama.cpp-server adapter over the owned HTTP/1.1 client.
//! Loopback endpoints only: construction refuses any non-loopback host, and
//! the adapter holds no remote endpoint, so a local-only failure physically
//! cannot escalate to a cloud host. Offline requests (`PrivacyMode::Offline`)
//! are refused before any socket attempt. Text responses are parsed and
//! validated with [`crate::transport::parse_chat_body`]; the capabilities
//! stay `structured_output: false` because text parsing never claims native
//! structured output.

use super::arbiter::{Admit, LocalRoute};
use super::core::{AdapterOutcome, MockStep};
use super::ladder::RungSupport;
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::{AiError, AiRequest};
use super::transport::{self, Endpoint};
use super::vendor::{http_adapter_api, HttpAdapter, Wire, CHAT_ENVELOPE};

/// Chat-completions wire: loopback only, so no remote dispatch and no
/// credential.
const WIRE: Wire = Wire {
    label: "local",
    body: transport::chat_body,
    envelope: &CHAT_ENVELOPE,
    remote: false,
    rungs: RungSupport::FULL,
};

/// Why a local endpoint was rejected at construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocalConfigError {
    /// Only loopback hosts serve the local adapter.
    NonLoopback,
}

impl std::fmt::Display for LocalConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalConfigError::NonLoopback => f.write_str("local endpoint must be loopback"),
        }
    }
}

/// Local sidecar adapter: OpenAI-compatible `/v1/chat/completions` over the
/// owned `std::net::TcpStream` client, with the contract suite running
/// unchanged through the shared contract state.
#[derive(Clone, Debug)]
pub struct LocalAdapter {
    http: HttpAdapter,
    /// Inference arbiter route; `None` always calls the configured sidecar.
    route: Option<LocalRoute>,
}

impl LocalAdapter {
    /// Build an adapter for `endpoint`. Non-loopback hosts are refused here,
    /// before any request can name them.
    pub fn new(script: Vec<MockStep>, endpoint: Endpoint) -> Result<Self, LocalConfigError> {
        if !transport::is_loopback(endpoint.host()) {
            return Err(LocalConfigError::NonLoopback);
        }
        Ok(Self {
            http: HttpAdapter::new(script, endpoint),
            route: None,
        })
    }

    http_adapter_api!();

    /// Remote endpoint contacts. Always zero: local loopback calls are not
    /// remote dispatches.
    pub fn remote_calls(&self) -> u32 {
        0
    }

    /// Place every request by the inference arbiter: Gpu calls this
    /// adapter's sidecar, Cpu the route's CPU sidecar, Defer parks it, and
    /// Remote refuses so the session fallback serves it.
    #[must_use]
    pub fn with_route(mut self, route: LocalRoute) -> Self {
        self.route = Some(route);
        self
    }

    /// The arbiter route, when one is set.
    pub fn route(&self) -> Option<&LocalRoute> {
        self.route.as_ref()
    }

    /// Run parked requests, oldest first, while the placement lets them
    /// run. Call off the tick (a worker or a frame-boundary drain); each
    /// outcome carries its request id in its events or result.
    pub fn run_deferred(
        &mut self,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> Vec<AdapterOutcome> {
        let mut done = Vec::new();
        while let Some((request, admit)) = self.route.as_mut().and_then(LocalRoute::next_ready) {
            done.push(self.dispatch(admit, &request, policy, capabilities));
        }
        done
    }

    /// Serve one admitted request. A held request gets a retryable
    /// `Unavailable` with no job state touched, so the parked request still
    /// runs once later.
    fn dispatch(
        &mut self,
        admit: Admit,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        match admit {
            Admit::Gpu => self.http.call(&WIRE, None, request, policy, capabilities),
            Admit::Cpu(cpu) => {
                let gpu = std::mem::replace(&mut self.http.endpoint, cpu);
                let outcome = self.http.call(&WIRE, None, request, policy, capabilities);
                self.http.endpoint = gpu;
                outcome
            }
            Admit::Held(_) => AdapterOutcome {
                result: Err(AiError::Unavailable),
                events: Vec::new(),
                dispatched_remote: false,
                duplicate: false,
                mutated: false,
                discarded: false,
            },
        }
    }
}

impl ProviderAdapter for LocalAdapter {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        let Some(route) = self.route.as_mut() else {
            return self.http.call(&WIRE, None, request, policy, capabilities);
        };
        // A request already served replays wherever placement now is.
        if let Some(replay) = self.http.core.replay(request.id) {
            return replay;
        }
        let admit = route.admit(request);
        self.dispatch(admit, request, policy, capabilities)
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
        inner.model.model = String::from("sidecar-1");
        inner
    }

    fn adapter_for(url: &str, script: Vec<MockStep>) -> LocalAdapter {
        let endpoint = transport::parse_endpoint(url).expect("test endpoint parses");
        LocalAdapter::new(script, endpoint).expect("loopback endpoint accepted")
    }

    #[test]
    fn pv1_failure_suite_passes_unchanged_through_local() {
        // Usage-free stub: unknown cost stays unknown, as in the mock suite.
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
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
    fn stub_roundtrip_receipt_is_labeled_loopback() {
        let stub = StubServer::spawn_completion(crate::stub::chat_ok(
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
        assert_eq!(stub.paths(), vec![String::from("/v1/chat/completions")]);
        assert_eq!(stub.auth_hits(), 0, "local sends no session auth");
        assert_eq!(result.usage.input_tokens, Some(7));
        assert_eq!(result.usage.output_tokens, Some(3));
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line == "local:127.0.0.1"),
            "receipt labels the loopback host: {proposal:?}"
        );
    }

    #[test]
    fn offline_privacy_makes_no_socket_attempt() {
        // Closed port proves the point: an Offline request is refused before
        // any dial, so even an unreachable endpoint records zero attempts.
        let port = closed_port();
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        let mut offline = request(2, 3);
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
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let mut adapter = adapter_for(&url, vec![MockStep::Succeed]);
        assert!(
            transport::is_loopback(adapter.endpoint().host()),
            "local adapter only holds loopback endpoints"
        );
        let outcome = adapter.call(
            &request(3, 3),
            &policy(),
            &ProviderCapabilities::local_http(),
        );
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(AiError::Unavailable.retryable());
        assert_eq!(adapter.socket_attempts(), 1, "the loopback dial was tried");
        assert_eq!(adapter.transport_calls(), 0, "no exchange completed");
        assert!(!outcome.dispatched_remote);
        assert_eq!(adapter.remote_calls(), 0, "never escalates to cloud");
        assert!(!outcome.mutated);
    }

    #[test]
    fn transport_timeout_maps_to_timeout() {
        let stub = StubServer::spawn_delayed(
            crate::stub::chat_ok(r#"{"actions": [], "diagnostics": []}"#, None),
            400,
        );
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        // Setup races report Unavailable without testing the mapping; retry
        // only while unsettled, then assert the settled mapping exactly.
        let outcome = crate::stub::settle(
            |n| {
                let mut adapter = adapter_for(&stub.url(), vec![MockStep::Succeed]);
                let mut slow = request(4 + n as u64, 3);
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
                adapter.call(&request(5 + n as u64, 3), &policy, &caps)
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
                let outcome = adapter.call(&request(6 + n as u64, 3), &policy, &caps);
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
                adapter.call(&request(7 + n as u64, 3), &policy, &caps)
            },
            |o| o.result != Err(AiError::Unavailable),
        );
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!outcome.mutated);
    }

    #[test]
    fn non_loopback_url_is_rejected_before_any_request() {
        let endpoint = transport::parse_endpoint("http://203.0.113.7:8080/v1/chat/completions")
            .expect("URL shape parses");
        assert!(
            matches!(
                LocalAdapter::new(vec![MockStep::Succeed], endpoint),
                Err(LocalConfigError::NonLoopback)
            ),
            "non-loopback local endpoint refused"
        );
    }

    #[test]
    fn arbiter_route_picks_the_sidecar_and_parks_deferred_requests() {
        use crate::arbiter::{
            Arbiter, ArbiterConfig, Hold, Placement, PressureSample, VramHeadroom,
        };
        let body = crate::stub::chat_ok(
            r#"{"actions": [{"noop": true}], "diagnostics": ["route"]}"#,
            None,
        );
        let gpu = StubServer::spawn_completion(body.clone());
        let cpu = StubServer::spawn_completion(body);
        let mut cfg = ArbiterConfig::reference(1_000);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut arbiter = Arbiter::new(cfg).expect("config");
        let at = |load: u32| PressureSample {
            frame_p95_us: load * 16,
            frame_budget_us: 16_000,
            vram: VramHeadroom::Known {
                budget_mb: 5_000,
                used_mb: 1_000,
            },
        };
        let cpu_endpoint = transport::parse_endpoint(&cpu.url()).expect("cpu url parses");
        let route = LocalRoute::new(arbiter.reader(), Some(cpu_endpoint), 2).expect("route");
        let mut adapter = adapter_for(&gpu.url(), Vec::new()).with_route(route);
        let policy = policy();
        let caps = ProviderCapabilities::local_http();
        let mut next_id = 100;
        let mut serve = |adapter: &mut LocalAdapter| {
            crate::stub::settle(
                |_| {
                    next_id += 1;
                    adapter.call(&request(next_id, 3), &policy, &caps)
                },
                |o| o.result.is_ok(),
            )
        };

        // Cold start places on the CPU sidecar.
        assert!(serve(&mut adapter).result.is_ok());
        assert_eq!(gpu.hits(), 0);
        assert!(cpu.hits() >= 1, "cpu sidecar served the cold start");
        // Quiet frames with room: the GPU sidecar serves.
        assert!(arbiter.observe(at(500)).is_some());
        let cpu_before = cpu.hits();
        assert!(serve(&mut adapter).result.is_ok());
        assert!(gpu.hits() >= 1, "gpu sidecar served");
        assert_eq!(cpu.hits(), cpu_before);

        // Overload defers: no socket, a retryable refusal, parked once.
        arbiter.observe(at(1_500));
        arbiter.observe(at(1_500));
        assert_eq!(arbiter.placement(), Placement::Defer);
        let (gpu_before, cpu_before) = (gpu.hits(), cpu.hits());
        let parked = request(7, 3);
        let held = adapter.call(&parked, &policy, &caps);
        assert_eq!(held.result, Err(AiError::Unavailable));
        assert!(!held.mutated && held.events.is_empty());
        // A retry of the same id stays parked and never queues twice.
        adapter.call(&parked, &policy, &caps);
        adapter.call(&request(8, 3), &policy, &caps);
        let full = adapter.call(&request(9, 3), &policy, &caps);
        assert_eq!(full.result, Err(AiError::Unavailable));
        let route = adapter.route().expect("route set");
        assert_eq!(route.deferred_len(), 2);
        assert_eq!(route.refused_full(), 1, "a full queue refuses and counts");
        assert_eq!(
            route.last(),
            Some((Placement::Defer, Some(Hold::QueueFull)))
        );
        assert_eq!((gpu.hits(), cpu.hits()), (gpu_before, cpu_before));
        // Still deferred: draining runs nothing.
        assert!(adapter.run_deferred(&policy, &caps).is_empty());

        // Pressure eases to the CPU band: the parked requests run there, in
        // order, exactly once.
        assert!(arbiter.observe(at(900)).is_some());
        assert_eq!(arbiter.placement(), Placement::Cpu);
        let ran = adapter.run_deferred(&policy, &caps);
        assert_eq!(ran.len(), 2);
        assert_eq!(adapter.route().map(LocalRoute::deferred_len), Some(0));
        assert_eq!(gpu.hits(), gpu_before, "nothing ran on the gpu sidecar");
        // A served request replays wherever placement now is.
        if ran[0].result.is_ok() {
            let again = adapter.call(&parked, &policy, &caps);
            assert!(again.duplicate);
        }
    }

    /// Needs an operator-run sidecar (`llama-server` on 127.0.0.1:8080):
    /// unverified in CI by design, never faked with a stub.
    #[test]
    #[ignore = "live: needs a llama-server sidecar on 127.0.0.1:8080"]
    fn local_sidecar_live_llama_cpp() {
        let endpoint = transport::parse_endpoint("http://127.0.0.1:8080/v1/chat/completions")
            .expect("sidecar URL parses");
        let mut adapter = LocalAdapter::new(vec![MockStep::Succeed], endpoint)
            .expect("loopback sidecar accepted");
        let outcome = adapter.call(
            &request(900, 3),
            &policy(),
            &ProviderCapabilities::local_http(),
        );
        assert!(outcome.result.is_ok(), "live sidecar serves: {outcome:?}");
    }
}
