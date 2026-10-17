//! serving: the live build site of the serving adapter.
//!
//! One place pairs a target with the inference arbiter's published placement,
//! so the handle that serves local inference carries the route: under frame
//! pressure a request parks or dials the CPU sidecar instead of always
//! dialing the GPU sidecar. [`attach`] is the one route-attachment point the
//! routed builder uses; [`serving_adapter`] builds the handle from a live
//! [`Arbiter`].
//!
//! Determinism: the handle depends only on the target, the scripted arms and
//! the arbiter's current placement; nothing here reads a clock or thread
//! order. The route itself is owned by the arbiter (one atomic load on the
//! request path, never a wait on the tick).

use super::arbiter::{Arbiter, LocalRoute};
use super::config::{adapter_for_routed, AdapterHandle, AdapterTarget, ConfigError};
use super::mock::MockStep;
use super::transport::Endpoint;

/// Longest defer queue one serving session holds: the route bound.
pub(crate) const SERVING_DEFER_BOUND: usize = 16;

/// Attach the arbiter's route to a serving handle: the one attachment point
/// on the local arm's request path. `None` leaves the handle plain, so a
/// route-less caller always calls its configured sidecar.
pub(crate) fn attach(handle: AdapterHandle, route: Option<LocalRoute>) -> AdapterHandle {
    match route {
        Some(route) => handle.with_route(route),
        None => handle,
    }
}

/// Build the serving adapter for `target` under the live `arbiter`: the local
/// arm applies the arbiter's route on its request path. `cpu` is the loopback
/// CPU sidecar endpoint, or `None` to park every CPU and deferred placement.
/// Caller: [`Arbiter::serving_adapter`] (`crates/provider/src/arbiter.rs`).
pub(crate) fn serving_adapter(
    target: &AdapterTarget,
    script: Vec<MockStep>,
    arbiter: &Arbiter,
    cpu: Option<Endpoint>,
) -> Result<AdapterHandle, ConfigError> {
    let route = LocalRoute::new(arbiter.reader(), cpu, SERVING_DEFER_BOUND).ok();
    adapter_for_routed(target, script, route)
}

#[cfg(test)]
mod tests {
    use super::super::config::{AdapterHandle, AdapterTarget};
    use super::super::policy::{AiSessionPolicy, Fallback, ProviderCapabilities};
    use super::super::protocol::{AdapterKind, AiError, PrivacyMode};
    use super::super::stub::{chat_ok, StubServer};
    use super::super::transport;
    use crate::arbiter::{Arbiter, ArbiterConfig, Hold, Placement, PressureSample, VramHeadroom};
    use crate::mock::ProviderAdapter;
    use crate::AdapterOutcome;

    const BODY: &str = r#"{"actions": [{"noop": true}], "diagnostics": ["serving:stub"]}"#;

    fn local_policy() -> AiSessionPolicy {
        AiSessionPolicy {
            adapters: vec![AdapterKind::Local],
            mode: PrivacyMode::LocalOnly,
            destinations: Vec::new(),
            scopes: Vec::new(),
            request_limit: 8,
            max_concurrency: 1,
            spend_limit: None,
            fallback: Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        }
    }

    fn sample(load: u32) -> PressureSample {
        PressureSample {
            frame_p95_us: load * 16,
            frame_budget_us: 16_000,
            vram: VramHeadroom::Known {
                budget_mb: 5_000,
                used_mb: 1_000,
            },
        }
    }

    fn call(handle: &mut AdapterHandle, id: u64) -> AdapterOutcome {
        let mut request = crate::mock::fixtures::request(id, 3);
        request.model.adapter = AdapterKind::Local;
        handle.call(
            &request,
            &local_policy(),
            &ProviderCapabilities::local_http(),
        )
    }

    /// The live build site carries the arbiter's placement: cold placement
    /// serves from the CPU sidecar, quiet frames move to the GPU sidecar, and
    /// overload parks the request without dialing either. F2P:
    /// `Arbiter::serving_adapter` did not exist before this test.
    #[test]
    fn arbiter_serving_adapter_carries_the_placement() {
        let body = chat_ok(BODY, None);
        let gpu = StubServer::spawn_completion(body.clone());
        let cpu = StubServer::spawn_completion(body);
        let mut cfg = ArbiterConfig::reference(1_000);
        cfg.degrade_after = 1;
        cfg.recover_after = 1;
        let mut arbiter = Arbiter::new(cfg).expect("config");
        let cpu_endpoint = transport::parse_endpoint(&cpu.url()).expect("cpu url parses");
        let target = AdapterTarget::Local { url: gpu.url() };
        let mut handle = arbiter
            .serving_adapter(&target, Vec::new(), Some(cpu_endpoint))
            .expect("serving adapter builds");

        // Cold: the arbiter starts on the CPU, so the CPU sidecar serves.
        assert!(call(&mut handle, 1).result.is_ok());
        assert_eq!(gpu.hits(), 0, "cold placement keeps the GPU sidecar free");
        assert!(cpu.hits() >= 1, "the placed CPU sidecar served");

        // Quiet frames with room: the arbiter moves to the GPU sidecar.
        assert!(arbiter.observe(sample(500)).is_some());
        let cpu_before = cpu.hits();
        assert!(call(&mut handle, 2).result.is_ok());
        assert!(gpu.hits() >= 1, "the GPU sidecar served");
        assert_eq!(cpu.hits(), cpu_before);

        // Overload parks the request: no socket, a retryable refusal, and the
        // hold the serving route carries is typed.
        arbiter.observe(sample(1_500));
        arbiter.observe(sample(1_500));
        assert_eq!(arbiter.placement(), Placement::Defer);
        let (gpu_before, cpu_before) = (gpu.hits(), cpu.hits());
        assert_eq!(call(&mut handle, 3).result, Err(AiError::Unavailable));
        assert_eq!((gpu.hits(), cpu.hits()), (gpu_before, cpu_before));
        let AdapterHandle::Local(adapter) = &handle else {
            panic!("a local target builds a local handle");
        };
        let route = adapter.route().expect("route attached");
        assert_eq!(route.last(), Some((Placement::Defer, Some(Hold::Deferred))));
    }

    /// A non-loopback CPU endpoint is refused, so the serving route has no CPU
    /// sidecar and the handle comes back plain: it dials the GPU sidecar even
    /// under pressure, never silently parking.
    #[test]
    fn serving_adapter_drops_a_refused_cpu_endpoint() {
        let gpu = StubServer::spawn_completion(chat_ok(BODY, None));
        let far = transport::parse_endpoint("http://203.0.113.7:8080/v1/chat/completions")
            .expect("url parses");
        let mut cfg = ArbiterConfig::reference(1_000);
        cfg.degrade_after = 1;
        let mut arbiter = Arbiter::new(cfg).expect("config");
        let target = AdapterTarget::Local { url: gpu.url() };
        let mut handle = arbiter
            .serving_adapter(&target, Vec::new(), Some(far))
            .expect("serving adapter builds");
        arbiter.observe(sample(1_500));
        arbiter.observe(sample(1_500));
        assert_eq!(arbiter.placement(), Placement::Defer);
        assert!(
            call(&mut handle, 1).result.is_ok(),
            "the plain handle serves"
        );
        assert!(gpu.hits() >= 1, "no route: the GPU sidecar dialed");
    }
}
