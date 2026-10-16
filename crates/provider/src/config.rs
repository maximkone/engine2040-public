//! config: backend selection without touching scene, tool or gameplay code.
//! One [`AdapterTarget`] (URL plus an optional secret reference)
//! builds the serving adapter; requests, policies and capabilities flow
//! through [`AdapterHandle`] unchanged, so switching local versus remote is a
//! configuration edit, never a code edit.

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
    use super::super::protocol::{AdapterKind, PrivacyMode};
    use super::*;
    use crate::stub::{chat_ok, StubServer};

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
