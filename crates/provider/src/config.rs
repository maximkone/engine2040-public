//! config: backend selection without touching scene, tool or gameplay code.
//! One [`AdapterTarget`] (URL plus an optional secret reference)
//! builds the serving adapter; requests, policies and capabilities flow
//! through [`AdapterHandle`] unchanged, so switching local versus remote is a
//! configuration edit, never a code edit.

use super::arbiter::LocalRoute;
use super::core::MockStep;
use super::local::{LocalAdapter, LocalConfigError};
use super::mock::ProviderAdapter;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, ProviderCapabilities};
use super::protocol::AiRequest;
use super::remote::{RemoteAdapter, SecretRef};
use super::transport::{self, EndpointError};

/// Which backend serves: a loopback sidecar or a configured remote host.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AdapterTarget {
    /// Local sidecar URL, e.g. `http://127.0.0.1:8080/v1/chat/completions`.
    Local {
        /// Endpoint URL (loopback only; `http://` always, `https://` with the provider `tls` feature).
        url: String,
    },
    /// Configured remote host URL plus the secret reference for session auth.
    Remote {
        /// Endpoint URL (`http://` always, `https://` with the provider `tls` feature).
        url: String,
        /// Environment variable holding the bearer token at call time.
        auth_env: String,
    },
}

/// Built adapter behind the shared [`ProviderAdapter`] boundary. Callers keep
/// one code path; only the target differs.
#[derive(Clone, Debug)]
pub enum AdapterHandle {
    /// Loopback sidecar adapter.
    Local(LocalAdapter),
    /// Configured remote adapter.
    Remote(RemoteAdapter),
}

impl ProviderAdapter for AdapterHandle {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> super::core::AdapterOutcome {
        match self {
            AdapterHandle::Local(adapter) => adapter.call(request, policy, capabilities),
            AdapterHandle::Remote(adapter) => adapter.call(request, policy, capabilities),
        }
    }

    fn request_pins(&self) -> Option<&RequestPins> {
        match self {
            AdapterHandle::Local(adapter) => adapter.request_pins(),
            AdapterHandle::Remote(adapter) => adapter.request_pins(),
        }
    }
}

impl AdapterHandle {
    /// Attach the inference arbiter's route to the local arm, so each request
    /// is placed (GPU, CPU, deferred, or a remote handoff the session fallback
    /// owns) instead of always dialing the configured sidecar. A remote target
    /// has no local request path, so its arm comes back unchanged: attaching a
    /// local route to it would park calls the remote host still serves.
    /// Determinism: same handle plus same route gives the same handle twice.
    /// Caller: [`adapter_for_routed`] (this file).
    #[must_use]
    pub fn with_route(self, route: LocalRoute) -> Self {
        match self {
            AdapterHandle::Local(adapter) => AdapterHandle::Local(adapter.with_route(route)),
            AdapterHandle::Remote(adapter) => AdapterHandle::Remote(adapter),
        }
    }
}

/// Why a target was rejected. Carries no secrets by construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfigError {
    /// URL shape or scheme rejected (`https://` needs the provider `tls` feature).
    BadUrl(EndpointError),
    /// Local targets must name a loopback host.
    NonLoopbackLocal,
    /// Remote targets need a named credential source.
    MissingAuth,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::BadUrl(error) => write!(f, "adapter URL rejected: {error}"),
            ConfigError::NonLoopbackLocal => f.write_str("local adapter needs a loopback URL"),
            ConfigError::MissingAuth => f.write_str("remote adapter needs an auth variable"),
        }
    }
}

impl From<LocalConfigError> for ConfigError {
    fn from(_: LocalConfigError) -> Self {
        ConfigError::NonLoopbackLocal
    }
}

/// Build the serving adapter for `target`. The script seeds scripted failure
/// arms for tests; production callers pass an empty script.
pub fn adapter_for(
    target: &AdapterTarget,
    script: Vec<MockStep>,
) -> Result<AdapterHandle, ConfigError> {
    adapter_for_routed(target, script, None)
}

/// Build the serving adapter for `target` under the inference arbiter's
/// placement: [`adapter_for`] plus the route the local arm applies on its
/// request path, so local inference moves to the CPU sidecar, parks, or hands
/// the request to the session fallback while the game holds the frame budget.
/// `None` builds a plain adapter that always calls its host. The route itself
/// is owned by the caller (`crates/provider/src/arbiter.rs`): one atomic load
/// per request, never a wait on the tick.
/// Caller: [`adapter_for`] (this file), which passes `None`.
pub fn adapter_for_routed(
    target: &AdapterTarget,
    script: Vec<MockStep>,
    route: Option<LocalRoute>,
) -> Result<AdapterHandle, ConfigError> {
    let handle = build_adapter(target, script)?;
    Ok(super::serving::attach(handle, route))
}

/// The one place an adapter is built from a target, so the plain and routed
/// builders cannot drift apart.
fn build_adapter(
    target: &AdapterTarget,
    script: Vec<MockStep>,
) -> Result<AdapterHandle, ConfigError> {
    match target {
        AdapterTarget::Local { url } => {
            let endpoint = transport::parse_endpoint(url).map_err(ConfigError::BadUrl)?;
            LocalAdapter::new(script, endpoint)
                .map(AdapterHandle::Local)
                .map_err(ConfigError::from)
        }
        AdapterTarget::Remote { url, auth_env } => {
            if auth_env.is_empty() {
                return Err(ConfigError::MissingAuth);
            }
            let endpoint = transport::parse_endpoint(url).map_err(ConfigError::BadUrl)?;
            Ok(AdapterHandle::Remote(RemoteAdapter::new(
                script,
                endpoint,
                SecretRef::EnvVar(auth_env.clone()),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::policy::{AiSessionPolicy, ProviderCapabilities};
    use super::super::protocol::{AdapterKind, AiError, PrivacyMode};
    use super::*;
    use crate::stub::{chat_ok, StubServer};

    /// Local-only policy: the local sidecar arm of the session.
    fn local_policy() -> AiSessionPolicy {
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

    /// Remote-allowed policy for the configured remote host.
    fn remote_policy() -> AiSessionPolicy {
        AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![String::from("http://127.0.0.1")],
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

    const BODY: &str = r#"{"actions": [{"noop": true}], "diagnostics": ["cfg:stub"]}"#;

    #[test]
    fn local_target_serves_without_code_changes() {
        let stub = StubServer::spawn_completion(chat_ok(BODY, None));
        let target = AdapterTarget::Local { url: stub.url() };
        let mut adapter = adapter_for(&target, Vec::new()).expect("local target builds");
        let mut request = crate::mock::fixtures::request(1, 3);
        request.model.adapter = AdapterKind::Local;
        let policy = AiSessionPolicy {
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
        };
        let outcome = adapter.call(&request, &policy, &ProviderCapabilities::local_http());
        assert!(outcome.result.is_ok(), "local target serves: {outcome:?}");
        assert_eq!(stub.hits(), 1);
    }

    #[test]
    fn remote_target_sends_session_auth() {
        crate::test_env::set("E2040_PV2_CFG_TOKEN", "cfg-token");
        let stub = StubServer::spawn_completion(chat_ok(BODY, None));
        let target = AdapterTarget::Remote {
            url: stub.url(),
            auth_env: String::from("E2040_PV2_CFG_TOKEN"),
        };
        let mut adapter = adapter_for(&target, Vec::new()).expect("remote target builds");
        let mut request = crate::mock::fixtures::request(2, 3);
        request.model.adapter = AdapterKind::Remote;
        request.privacy = PrivacyMode::RemoteAllowed;
        request.budget.spend_limit = Some(50);
        let policy = AiSessionPolicy {
            adapters: vec![AdapterKind::Remote],
            mode: PrivacyMode::RemoteAllowed,
            destinations: vec![String::from("http://127.0.0.1")],
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: crate::policy::Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        };
        let outcome = adapter.call(&request, &policy, &ProviderCapabilities::remote_http());
        assert!(outcome.result.is_ok(), "remote target serves: {outcome:?}");
        assert_eq!(stub.auth_hits(), 1, "config remote authenticates");
        crate::test_env::remove("E2040_PV2_CFG_TOKEN");
    }

    #[test]
    fn url_switch_needs_no_request_change() {
        // Same request, two local URLs, identical outcomes: the switch is
        // configuration, never scene/tool/gameplay code.
        let first = StubServer::spawn_completion(chat_ok(BODY, None));
        let second = StubServer::spawn_completion(chat_ok(BODY, None));
        let mut request = crate::mock::fixtures::request(3, 3);
        request.model.adapter = AdapterKind::Local;
        let policy = AiSessionPolicy {
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
        };
        let caps = ProviderCapabilities::local_http();
        let mut a = adapter_for(&AdapterTarget::Local { url: first.url() }, Vec::new())
            .expect("first target builds");
        let mut b = adapter_for(&AdapterTarget::Local { url: second.url() }, Vec::new())
            .expect("second target builds");
        let left = a.call(&request, &policy, &caps);
        let right = b.call(&request, &policy, &caps);
        assert!(left.result.is_ok() && right.result.is_ok());
        assert_eq!(left, right, "URL switch changes no outcome");
    }

    #[test]
    fn bad_targets_fail_before_any_request() {
        assert!(
            matches!(
                adapter_for(
                    &AdapterTarget::Local {
                        url: String::from("http://203.0.113.7:8080/v1/chat/completions"),
                    },
                    Vec::new()
                ),
                Err(ConfigError::NonLoopbackLocal)
            ),
            "non-loopback local target refused"
        );
        // Without `tls` an `https://` local URL fails at parse; with `tls`
        // it builds (a loopback TLS endpoint) and any refusal moves to dial
        // time.
        #[cfg(not(feature = "tls"))]
        assert!(matches!(
            adapter_for(
                &AdapterTarget::Local {
                    url: String::from("https://127.0.0.1:8080/v1/chat/completions"),
                },
                Vec::new()
            ),
            Err(ConfigError::BadUrl(_))
        ));
        #[cfg(feature = "tls")]
        https_local_url_policy();
        assert!(
            matches!(
                adapter_for(
                    &AdapterTarget::Remote {
                        url: String::from("http://127.0.0.1:8080/v1/chat/completions"),
                        auth_env: String::new(),
                    },
                    Vec::new()
                ),
                Err(ConfigError::MissingAuth)
            ),
            "remote target without credential refused"
        );
    }

    /// The inference arbiter's route reaches the production boundary: a routed
    /// local target parks a request under frame pressure (no socket dialed, a
    /// retryable refusal, the hold typed on the route) and serves from the
    /// placed sidecar once pressure eases. F2P: `adapter_for_routed` and
    /// `AdapterHandle::with_route` do not exist before this test.
    #[test]
    fn routed_local_target_applies_the_arbiter_placement() {
        use crate::arbiter::{
            Arbiter, ArbiterConfig, Hold, LocalRoute, Placement, PressureSample, VramHeadroom,
        };
        let body = chat_ok(BODY, None);
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
        let route = LocalRoute::new(arbiter.reader(), Some(cpu_endpoint), 1).expect("route");
        let target = AdapterTarget::Local { url: gpu.url() };
        let mut handle = adapter_for_routed(&target, Vec::new(), Some(route))
            .expect("routed local target builds");
        let policy = local_policy();
        let caps = ProviderCapabilities::local_http();
        let call = |handle: &mut AdapterHandle, id: u64| {
            let mut request = crate::mock::fixtures::request(id, 3);
            request.model.adapter = AdapterKind::Local;
            handle.call(&request, &policy, &caps)
        };

        // Cold: the model is not on the GPU yet, so the CPU sidecar serves.
        assert!(call(&mut handle, 1).result.is_ok());
        assert_eq!(gpu.hits(), 0, "cold placement keeps the GPU sidecar free");
        assert!(cpu.hits() >= 1, "the placed CPU sidecar served");
        // Quiet frames with room: the GPU sidecar serves.
        assert!(arbiter.observe(at(500)).is_some());
        let cpu_before = cpu.hits();
        assert!(call(&mut handle, 2).result.is_ok());
        assert!(gpu.hits() >= 1, "the GPU sidecar served");
        assert_eq!(cpu.hits(), cpu_before);
        // Overload parks the request: no socket, a retryable refusal, and the
        // hold the request path carries is typed.
        arbiter.observe(at(1_500));
        arbiter.observe(at(1_500));
        assert_eq!(arbiter.placement(), Placement::Defer);
        let (gpu_before, cpu_before) = (gpu.hits(), cpu.hits());
        assert_eq!(call(&mut handle, 3).result, Err(AiError::Unavailable));
        assert_eq!(
            (gpu.hits(), cpu.hits()),
            (gpu_before, cpu_before),
            "a parked request dials no socket"
        );
        let AdapterHandle::Local(adapter) = &handle else {
            panic!("a local target builds a local handle");
        };
        let route = adapter.route().expect("route attached");
        assert_eq!(route.deferred_len(), 1, "parked once, never dialed");
        assert_eq!(
            route.last(),
            Some((Placement::Defer, Some(Hold::Deferred))),
            "the hold is typed"
        );
    }

    /// A remote target has no local request path: a route attached at the
    /// boundary leaves the remote arm serving its host, never parking.
    #[test]
    fn remote_target_never_takes_a_local_route() {
        use crate::arbiter::{Arbiter, ArbiterConfig, LocalRoute};
        crate::test_env::set("E2040_PV2_CFG_ROUTE_TOKEN", "route-token");
        let stub = StubServer::spawn_completion(chat_ok(BODY, None));
        let arbiter = Arbiter::new(ArbiterConfig::reference(1_000)).expect("config");
        // Without a CPU sidecar this route parks every request, so a remote
        // handle that took it could not serve at all.
        let route = LocalRoute::new(arbiter.reader(), None, 1).expect("route");
        let target = AdapterTarget::Remote {
            url: stub.url(),
            auth_env: String::from("E2040_PV2_CFG_ROUTE_TOKEN"),
        };
        let mut handle =
            adapter_for_routed(&target, Vec::new(), Some(route)).expect("routed remote builds");
        let mut request = crate::mock::fixtures::request(4, 3);
        request.model.adapter = AdapterKind::Remote;
        request.privacy = PrivacyMode::RemoteAllowed;
        request.budget.spend_limit = Some(50);
        let outcome = handle.call(
            &request,
            &remote_policy(),
            &ProviderCapabilities::remote_http(),
        );
        assert!(outcome.result.is_ok(), "remote arm serves: {outcome:?}");
        assert_eq!(stub.auth_hits(), 1, "the route never parks a remote call");
        assert!(
            matches!(handle, AdapterHandle::Remote(_)),
            "the remote arm is unchanged by a local route"
        );
        crate::test_env::remove("E2040_PV2_CFG_ROUTE_TOKEN");
    }

    /// With `tls` an `https://` loopback URL is a valid local target: parse
    /// accepts it and the adapter builds, so construction proves the policy
    /// (no request is made here).
    #[cfg(feature = "tls")]
    fn https_local_url_policy() {
        assert!(
            adapter_for(
                &AdapterTarget::Local {
                    url: String::from("https://127.0.0.1:8080/v1/chat/completions"),
                },
                Vec::new()
            )
            .is_ok(),
            "loopback https local target builds under `tls`"
        );
    }
}
