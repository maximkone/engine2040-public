//! mock: deterministic scripted adapter plus the contract suite.
//! In-code scripted behavior over the committed contract types: no network, no
//! writes, no new dependencies. The mock never dispatches a remote call and
//! never mutates world state; `mutated` only records that a call newly
//! produced a complete proposal. World writes stay behind the command
//! service, which this crate never touches.
//!
//! Consumer note: implement [`ProviderAdapter`] for the local/remote
//! adapters and reuse [`fixtures`] plus the per-mode assertions in the test
//! module below. The tests only use the public API (`new`, `call`,
//! `cancel`, `begin_job`, `cancel_job`, `deliver_late`, `set_revision`,
//! `set_in_flight`, counters), so the suite ports unchanged to any adapter
//! that honors the same failure semantics. All adapters share guards, jobs
//! and scripted arms through `crate::core::ContractCore`.

use super::core::ContractCore;
use super::pins::RequestPins;
use super::policy::{AiSessionPolicy, CancelReason, JobState, ProviderCapabilities};
use super::protocol::{AiEvent, AiRequest, RequestId, Usage};

pub use super::core::{
    validate_proposal, AdapterOutcome, LateOutcome, MockOutcome, MockStep, VISION_MODEL_PREFIX,
};

/// Substitution boundary for provider adapters (mock, local, remote).
/// Every adapter serves [`MockOutcome`] envelopes (aliased as
/// [`AdapterOutcome`]) so one contract suite covers all backends.
pub trait ProviderAdapter {
    /// Serve one request under a session policy and negotiated capabilities.
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome;

    /// The pins this adapter puts on every model request it sends, or `None`
    /// for an adapter that sends none (the mock). A recorder reads it so a
    /// changed prompt, temperature, seed or tool set changes the cassette key.
    fn request_pins(&self) -> Option<&RequestPins> {
        None
    }
}

/// Deterministic scripted adapter. State (seen ids, jobs, cancellation,
/// revision, load, counters) lives in the shared [`ContractCore`]; the same
/// script plus the same request sequence always yields the same outcome
/// sequence.
#[derive(Clone, Debug)]
pub struct MockAdapter {
    core: ContractCore,
    remote_calls: u32,
}

impl MockAdapter {
    /// Build an adapter that plays `script` in order. Calls past the end of
    /// the script succeed, so success-anchored tests may pass an empty script.
    pub fn new(script: Vec<MockStep>) -> Self {
        Self {
            core: ContractCore::new(script),
            remote_calls: 0,
        }
    }

    /// Pin the live document/world revision. Calls whose `base_revision`
    /// differs fail with [`super::protocol::AiError::StaleContext`] before
    /// the script runs.
    pub fn set_revision(&mut self, revision: u64) {
        self.core.set_revision(revision);
    }

    /// Stop enforcing revision freshness.
    pub fn clear_revision_check(&mut self) {
        self.core.clear_revision_check();
    }

    /// Pretend `count` requests are already in flight. A call made while
    /// `count >= policy.max_concurrency` fails as queue overflow.
    pub fn set_in_flight(&mut self, count: u32) {
        self.core.set_in_flight(count);
    }

    /// Track a job as running so cancel/late-output tests have a live job.
    pub fn begin_job(&mut self, id: RequestId) {
        self.core.begin_job(id);
    }

    /// Cancel a job with a reason. The next `call` for the id refuses with
    /// [`super::protocol::AiError::Cancelled`]; later `deliver_late` reports
    /// discarded.
    pub fn cancel_job(&mut self, id: RequestId, reason: CancelReason) {
        self.core.cancel_job(id, reason);
    }

    /// Cancel as the calling user. Shorthand for `cancel_job(id, User)`.
    pub fn cancel(&mut self, id: RequestId) {
        self.core.cancel(id);
    }

    /// Offer one event to a job after the fact. Only a `Running` job accepts
    /// output; cancelled, done or unknown jobs discard it without mutation.
    pub fn deliver_late(&self, id: RequestId, event: &AiEvent) -> LateOutcome {
        self.core.deliver_late(id, event)
    }

    /// Script steps not yet consumed.
    pub fn script_remaining(&self) -> usize {
        self.core.script_remaining()
    }

    /// Calls that newly produced a complete proposal. Duplicates, errors and
    /// discarded late output never increment this.
    pub fn mutations(&self) -> u64 {
        self.core.mutations()
    }

    /// Remote endpoint contacts. Always zero for the mock; the privacy tests
    /// pin this alongside `dispatched_remote`.
    pub fn remote_calls(&self) -> u32 {
        self.remote_calls
    }
}

impl ProviderAdapter for MockAdapter {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> MockOutcome {
        // Same request identity twice returns the recorded first result and
        // performs no second mutation, without consuming script.
        if let Some(replay) = self.core.replay(request.id) {
            return replay;
        }
        if let Err((error, state)) = self.core.precheck(request, policy, capabilities) {
            return self.core.fail(request, error, state, false);
        }
        match self.core.next_step() {
            MockStep::Succeed => {
                let proposal = super::protocol::Proposal {
                    request: request.id,
                    base_revision: request.base_revision,
                    actions: vec![super::protocol::ProposalAction::Noop],
                    object_versions: Vec::new(),
                    diagnostics: vec![String::from("mock:ok")],
                };
                self.core
                    .succeed_with(request, proposal, Usage::default(), false)
            }
            step => match self.core.scripted(request, step) {
                Some(outcome) => outcome,
                // Unreachable: `Succeed` matched above, and every other arm
                // records. Malformed keeps the envelope shape on surprise.
                None => self.core.fail(
                    request,
                    super::protocol::AiError::MalformedOutput,
                    JobState::Done,
                    false,
                ),
            },
        }
    }
}

/// Shared fixtures for the contract suite. Every adapter reuses these builders
/// unchanged; only the adapter under test changes.
pub mod fixtures {
    use super::super::policy::{AiSessionPolicy, ProviderCapabilities};
    use super::super::protocol::{
        AdapterKind, AiRequest, Budget, ContextRef, DataClass, ModelSelector, PrivacyMode,
        RequestId, TaskKind,
    };
    use super::super::PROTOCOL_VERSION;

    /// Minimal valid local mock request with `id` and `base_revision`.
    pub fn request(id: u64, base_revision: u64) -> AiRequest {
        AiRequest {
            id: RequestId(id),
            kind: TaskKind::Diagnose,
            model: ModelSelector {
                adapter: AdapterKind::Mock,
                model: String::from("mock-1"),
            },
            context: vec![ContextRef {
                class: DataClass::Project,
                digest: [3u8; 32],
                revision: base_revision,
            }],
            schema_version: PROTOCOL_VERSION,
            budget: Budget {
                max_output_tokens: 512,
                max_ms: 5_000,
                spend_limit: None,
            },
            deadline_ms: 60_000,
            privacy: PrivacyMode::LocalOnly,
            base_revision,
        }
    }

    /// Local-only session over the mock adapter, concurrency 1.
    pub fn policy() -> AiSessionPolicy {
        AiSessionPolicy::local_default(8)
    }

    /// Negotiated mock capabilities: text only, cancellable, local-only.
    pub fn caps() -> ProviderCapabilities {
        ProviderCapabilities::mock()
    }
}

#[cfg(test)]
mod tests {
    use super::super::policy::{AiSessionPolicy, CancelReason, JobState};
    use super::super::protocol::{
        AdapterKind, AiError, AiEvent, EventKind, FinishReason, PrivacyMode, Proposal,
        ProposalAction, RequestId,
    };
    use super::fixtures;
    use super::*;
    use crate::MAX_ACTIONS;

    #[test]
    fn success_completes_with_valid_proposal() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let request = fixtures::request(1, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert!(!outcome.duplicate && !outcome.discarded && !outcome.dispatched_remote);
        assert!(outcome.mutated);
        let result = outcome.result.as_ref().expect("success must complete");
        assert_eq!(result.reason, FinishReason::Complete);
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        assert_eq!(proposal.request, request.id);
        assert_eq!(proposal.base_revision, request.base_revision);
        validate_proposal(proposal).expect("mock proposals validate");
        assert_eq!(outcome.events.len(), 1);
        assert!(matches!(outcome.events[0].kind, EventKind::Completed(_)));
        assert_eq!(adapter.mutations(), 1);
        assert_eq!(adapter.remote_calls(), 0);
    }

    #[test]
    fn timeout_exceeds_budget() {
        let simulated_ms = 9_000u64;
        let mut adapter = MockAdapter::new(vec![MockStep::Timeout { simulated_ms }]);
        let request = fixtures::request(2, 3);
        assert!(
            simulated_ms > request.budget.max_ms,
            "fixture must exceed the budget it claims to blow"
        );
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::Timeout));
        assert!(AiError::Timeout.retryable());
        assert!(!outcome.mutated && !outcome.dispatched_remote && !outcome.duplicate);
        assert_eq!(adapter.mutations(), 0);
    }

    #[test]
    fn malformed_output_fails_validation() {
        let mut adapter = MockAdapter::new(vec![MockStep::Malformed]);
        let request = fixtures::request(3, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::MalformedOutput));
        assert!(!AiError::MalformedOutput.retryable());
        assert!(!outcome.mutated);
        // The scripted bad shape really is invalid, not just labeled so.
        let bad = Proposal {
            request: request.id,
            base_revision: request.base_revision,
            actions: vec![ProposalAction::Noop; MAX_ACTIONS + 1],
            object_versions: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert_eq!(validate_proposal(&bad), Err(AiError::MalformedOutput));
        let bad_label = Proposal {
            request: request.id,
            base_revision: request.base_revision,
            actions: vec![ProposalAction::SetLabel {
                object: 1,
                label: String::new(),
            }],
            object_versions: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert_eq!(validate_proposal(&bad_label), Err(AiError::MalformedOutput));
    }

    #[test]
    fn cancelled_mid_call_reports_cancelled() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let request = fixtures::request(4, 3);
        adapter.cancel(request.id);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::Cancelled));
        assert!(!AiError::Cancelled.retryable());
        assert!(!outcome.mutated);
        // The armed cancel refused before the script ran.
        assert_eq!(adapter.script_remaining(), 1);
        assert_eq!(adapter.mutations(), 0);
    }

    #[test]
    fn scripted_cancel_reports_cancelled() {
        let mut adapter = MockAdapter::new(vec![MockStep::Cancelled]);
        let request = fixtures::request(44, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::Cancelled));
        assert!(!outcome.mutated && !outcome.dispatched_remote);
    }

    #[test]
    fn stale_context_on_revision_mismatch() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        adapter.set_revision(10);
        let request = fixtures::request(5, 9);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::StaleContext));
        assert!(AiError::StaleContext.retryable());
        assert!(!outcome.mutated);
        assert_eq!(adapter.script_remaining(), 1);
        // A fresh revision match serves normally again.
        let fresh = fixtures::request(55, 10);
        let retry = adapter.call(&fresh, &fixtures::policy(), &fixtures::caps());
        assert!(retry.result.is_ok());
    }

    #[test]
    fn scripted_stale_context_reports_stale() {
        let mut adapter = MockAdapter::new(vec![MockStep::StaleContext]);
        let request = fixtures::request(56, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::StaleContext));
        assert!(!outcome.mutated);
    }

    #[test]
    fn duplicate_returns_recorded_first_result_without_second_mutation() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let policy = fixtures::policy();
        let caps = fixtures::caps();
        let request = fixtures::request(6, 3);
        let first = adapter.call(&request, &policy, &caps);
        assert!(first.result.is_ok() && first.mutated && !first.duplicate);
        assert_eq!(adapter.mutations(), 1);
        let second = adapter.call(&request, &policy, &caps);
        assert!(!second.mutated, "duplicate performs no second mutation");
        assert!(second.duplicate);
        assert!(second.events.is_empty(), "duplicate emits no new events");
        assert_eq!(
            second.result, first.result,
            "duplicate replays the recorded result"
        );
        assert_eq!(adapter.mutations(), 1, "mutation counter stays at one");
        assert_eq!(adapter.script_remaining(), 0);
    }

    #[test]
    fn unsupported_capability_for_vision_on_mock_caps() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let mut request = fixtures::request(7, 3);
        request.model.model = format!("{VISION_MODEL_PREFIX}gate-scene");
        let caps = fixtures::caps();
        assert!(!caps.vision, "mock caps stay text-only");
        let outcome = adapter.call(&request, &fixtures::policy(), &caps);
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!AiError::UnsupportedCapability.retryable());
        assert!(!outcome.mutated && !outcome.dispatched_remote);
    }

    #[test]
    fn scripted_unsupported_capability_reports_refusal() {
        let mut adapter = MockAdapter::new(vec![MockStep::UnsupportedCapability]);
        let request = fixtures::request(77, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!outcome.mutated && !outcome.dispatched_remote);
    }

    #[test]
    fn quota_auth_on_hard_cap_with_unknown_cost() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let mut policy = fixtures::policy();
        policy.spend_limit = Some(500);
        let request = fixtures::request(8, 3);
        assert_eq!(
            request.budget.spend_limit, None,
            "fixture cost must stay unknown, never zero-claimed"
        );
        let outcome = adapter.call(&request, &policy, &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!AiError::QuotaAuth.retryable());
        assert!(!outcome.mutated && !outcome.dispatched_remote);
        // A bounded cost with the same cap is not refused by this rule.
        let mut funded = fixtures::request(88, 3);
        funded.budget.spend_limit = Some(50);
        let ok = adapter.call(&funded, &policy, &fixtures::caps());
        assert!(ok.result.is_ok());
    }

    #[test]
    fn scripted_quota_auth_reports_refusal() {
        let mut adapter = MockAdapter::new(vec![MockStep::QuotaAuth]);
        let request = fixtures::request(89, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!outcome.mutated);
    }

    #[test]
    fn queue_overflow_over_max_concurrency_is_unavailable() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let policy = fixtures::policy();
        adapter.set_in_flight(policy.max_concurrency);
        let request = fixtures::request(9, 3);
        let outcome = adapter.call(&request, &policy, &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(AiError::Unavailable.retryable());
        assert!(!outcome.mutated);
        assert_eq!(adapter.script_remaining(), 1);
        // Drained queue serves normally.
        adapter.set_in_flight(0);
        let retry = fixtures::request(99, 3);
        assert!(adapter
            .call(&retry, &policy, &fixtures::caps())
            .result
            .is_ok());
    }

    #[test]
    fn scripted_queue_overflow_is_unavailable() {
        let mut adapter = MockAdapter::new(vec![MockStep::QueueOverflow]);
        let request = fixtures::request(98, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(!outcome.mutated);
    }

    #[test]
    fn late_output_after_cancel_is_discarded() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let request = fixtures::request(10, 3);
        adapter.begin_job(request.id);
        adapter.cancel_job(request.id, CancelReason::SceneClosed);
        let late = AiEvent {
            request: request.id,
            seq: 7,
            kind: EventKind::Progress(String::from("late preview")),
        };
        let delivery = adapter.deliver_late(request.id, &late);
        assert!(delivery.discarded, "event after cancel is discarded");
        assert!(!delivery.mutated, "delivery never mutates");
        assert_eq!(adapter.mutations(), 0);
        // Unknown jobs are ineligible too: no receipt, no apply.
        let unknown = AiEvent {
            request: RequestId(999),
            seq: 0,
            kind: EventKind::Progress(String::from("orphan")),
        };
        assert!(adapter.deliver_late(RequestId(999), &unknown).discarded);
    }

    #[test]
    fn scripted_late_output_is_discarded() {
        let mut adapter = MockAdapter::new(vec![MockStep::LateOutput]);
        let request = fixtures::request(101, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert!(outcome.discarded, "late output is discarded");
        assert!(!outcome.mutated && !outcome.dispatched_remote);
        assert!(
            outcome.events.is_empty(),
            "discarded output emits no events"
        );
        assert_eq!(outcome.result, Err(AiError::Cancelled));
        assert_eq!(adapter.mutations(), 0);
    }

    #[test]
    fn privacy_downgrade_never_dispatches() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let mut request = fixtures::request(11, 3);
        request.model.adapter = AdapterKind::Remote;
        request.privacy = PrivacyMode::LocalOnly;
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(
            !outcome.dispatched_remote,
            "refusal records no remote dispatch"
        );
        assert_eq!(adapter.remote_calls(), 0, "mock never calls the network");
        assert!(!outcome.mutated);
    }

    #[test]
    fn scripted_privacy_downgrade_records_refusal_without_dispatch() {
        let mut adapter = MockAdapter::new(vec![MockStep::PrivacyDowngrade]);
        let request = fixtures::request(111, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!outcome.dispatched_remote);
        assert_eq!(adapter.remote_calls(), 0);
        assert!(!outcome.mutated);
    }

    #[test]
    fn unknown_usage_stays_unknown() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let request = fixtures::request(12, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        let result = outcome.result.as_ref().expect("success must complete");
        assert_eq!(result.usage.input_tokens, None);
        assert_eq!(result.usage.output_tokens, None);
    }

    #[test]
    fn same_script_twice_gives_same_outcome_sequence() {
        let script = vec![
            MockStep::Succeed,
            MockStep::Timeout {
                simulated_ms: 9_000,
            },
            MockStep::Malformed,
            MockStep::QuotaAuth,
        ];
        let policy = fixtures::policy();
        let caps = fixtures::caps();
        let run = |base: u64| {
            let mut adapter = MockAdapter::new(script.clone());
            (20..24)
                .map(|n| adapter.call(&fixtures::request(n, base), &policy, &caps))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3), "deterministic failure fixtures replay");
    }

    #[test]
    fn done_jobs_reject_late_output_and_running_accepts() {
        let mut adapter = MockAdapter::new(vec![MockStep::Succeed]);
        let request = fixtures::request(13, 3);
        let outcome = adapter.call(&request, &fixtures::policy(), &fixtures::caps());
        assert!(outcome.result.is_ok());
        let late = AiEvent {
            request: request.id,
            seq: 1,
            kind: EventKind::Progress(String::from("after done")),
        };
        assert!(adapter.deliver_late(request.id, &late).discarded);
        // A still-running job remains eligible.
        let live = RequestId(14);
        adapter.begin_job(live);
        let preview = AiEvent {
            request: live,
            seq: 0,
            kind: EventKind::Progress(String::from("streaming")),
        };
        let delivery = adapter.deliver_late(live, &preview);
        assert!(!delivery.discarded && !delivery.mutated);
        // Terminal states stay put: cancelling a done job keeps it done.
        assert_eq!(JobState::Done.cancel(CancelReason::User), JobState::Done);
    }

    #[test]
    fn session_policy_local_default_needs_no_remote() {
        // Pins the privacy posture the downgrade tests rely on.
        let policy = AiSessionPolicy::local_default(8);
        assert!(!policy.mode.allows_remote());
        assert!(!policy.adapters.contains(&AdapterKind::Remote));
        assert_eq!(super::super::PROTOCOL_VERSION, 1);
    }
}
