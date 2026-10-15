//! suite: shared failure-mode assertions for every adapter. Local and
//! remote adapters run the same modes through [`run_failure_suite`] so the
//! suite cannot drift between backends. Tests only.

use super::core::{validate_proposal, AdapterOutcome, LateOutcome, VISION_MODEL_PREFIX};
use super::mock::{MockStep, ProviderAdapter};
use super::policy::{AiSessionPolicy, CancelReason, JobState, ProviderCapabilities};
use super::protocol::{
    AiError, AiEvent, AiRequest, EventKind, FinishReason, Proposal, ProposalAction, RequestId,
};
use super::MAX_ACTIONS;

/// Test-only control surface every suite adapter exposes. Mirrors the public
/// `MockAdapter` API (`new`, `call`, `cancel`, `begin_job`, `cancel_job`,
/// `deliver_late`, `set_revision`, `set_in_flight`, counters) so the suite
/// ports unchanged.
pub(crate) trait SuiteAdapter: ProviderAdapter {
    fn t_cancel(&mut self, id: RequestId);
    fn t_begin_job(&mut self, id: RequestId);
    fn t_cancel_job(&mut self, id: RequestId, reason: CancelReason);
    fn t_deliver_late(&self, id: RequestId, event: &AiEvent) -> LateOutcome;
    fn t_set_revision(&mut self, revision: u64);
    fn t_set_in_flight(&mut self, count: u32);
    fn t_script_remaining(&self) -> usize;
    fn t_mutations(&self) -> u64;
    fn t_socket_attempts(&self) -> u64;
    fn t_remote_calls(&self) -> u32;
}

/// Backend-specific expectations inside the otherwise identical suite.
pub(crate) struct SuiteExpect {
    /// `dispatched_remote` on a completed stub roundtrip (false for local
    /// loopback, true for the configured remote endpoint).
    pub remote_dispatch_on_success: bool,
    /// Host label the success receipt must carry (the loopback under test).
    pub receipt_host: String,
}

/// Shared inputs for [`call_settled`]: the request factory, policy and
/// capabilities stay identical across every suite site.
pub(crate) struct SettleCtx<'a> {
    pub request: &'a dyn Fn(u64, u64) -> AiRequest,
    pub policy: &'a AiSessionPolicy,
    pub caps: &'a ProviderCapabilities,
}

/// Call through loopback setup races: a fresh adapter plus a fresh request
/// id per attempt, retrying while the outcome is `Unavailable` (the exchange
/// never completed), bounded. Same rationale as `stub::settle`, applied where
/// the shared suite expects a completed exchange; per-adapter socket tests
/// use that helper directly. Returns the settled request, its outcome and
/// its adapter so counter asserts keep working. A genuine failure settles
/// the same wrong result every attempt and still fails below.
/// Cold boundary: the factory plus setup arrive boxed,
/// so the retry loop plus adapter construction live here once instead of
/// once per adapter backend.
pub(crate) fn call_settled(
    new: &dyn Fn(Vec<MockStep>) -> Box<dyn SuiteAdapter>,
    script: Vec<MockStep>,
    setup: &dyn Fn(&mut dyn SuiteAdapter),
    ctx: &SettleCtx<'_>,
    base_id: u64,
    base_revision: u64,
) -> (AiRequest, AdapterOutcome, Box<dyn SuiteAdapter>) {
    const TRIES: u64 = 8;
    let mut n = 0u64;
    loop {
        let asked = (ctx.request)(base_id + n * 1000, base_revision);
        let mut adapter = new(script.clone());
        setup(&mut *adapter);
        let outcome = adapter.call(&asked, ctx.policy, ctx.caps);
        if outcome.result.is_ok() || n + 1 >= TRIES {
            return (asked, outcome, adapter);
        }
        n += 1;
    }
}

/// Run every contract failure mode against `new`: timeout, malformed, cancel,
/// stale revision, duplicate, unsupported capability, quota/auth, queue
/// overflow, late output and privacy downgrade. Success arms run against the
/// caller's stub server, so real TCP roundtrips back the receipts.
/// Cold boundary: the factory arrives as `&dyn Fn` and
/// every adapter crosses it boxed, so this whole suite compiles once for
/// all six backends instead of monomorphizing per adapter.
pub(crate) fn run_failure_suite(
    new: &dyn Fn(Vec<MockStep>) -> Box<dyn SuiteAdapter>,
    policy: &AiSessionPolicy,
    caps: &ProviderCapabilities,
    request: &dyn Fn(u64, u64) -> AiRequest,
    downgraded_request: &dyn Fn(u64) -> AiRequest,
    expect: &SuiteExpect,
) {
    let env = SuiteEnv {
        new,
        policy,
        caps,
        request,
        downgraded_request,
        expect,
    };
    suite_success_timeout_malformed(&env);
    suite_cancel_stale_duplicate(&env);
    suite_capability_quota_overflow(&env);
    suite_late_and_privacy(&env);
    suite_done_jobs_and_determinism(&env);
}

/// Everything one suite section needs; every field is a shared reference, so
/// sections copy it out instead of threading six parameters.
#[derive(Clone, Copy)]
struct SuiteEnv<'a> {
    new: &'a dyn Fn(Vec<MockStep>) -> Box<dyn SuiteAdapter>,
    policy: &'a AiSessionPolicy,
    caps: &'a ProviderCapabilities,
    request: &'a dyn Fn(u64, u64) -> AiRequest,
    downgraded_request: &'a dyn Fn(u64) -> AiRequest,
    expect: &'a SuiteExpect,
}

/// Success with a real roundtrip receipt, timeout and malformed output.
fn suite_success_timeout_malformed(env: &SuiteEnv<'_>) {
    let SuiteEnv {
        new,
        policy,
        caps,
        request,
        expect,
        ..
    } = *env;
    // Success completes with a valid proposal backed by a real roundtrip.
    {
        let ctx = SettleCtx {
            request,
            policy,
            caps,
        };
        let (asked, outcome, adapter) =
            call_settled(new, vec![MockStep::Succeed], &|_| {}, &ctx, 1, 3);
        assert!(!outcome.duplicate && !outcome.discarded);
        assert_eq!(outcome.dispatched_remote, expect.remote_dispatch_on_success);
        assert!(outcome.mutated);
        let result = outcome.result.as_ref().expect("success must complete");
        assert_eq!(result.reason, FinishReason::Complete);
        let proposal = result
            .proposal
            .as_ref()
            .expect("complete carries a proposal");
        assert_eq!(proposal.request, asked.id);
        assert_eq!(proposal.base_revision, asked.base_revision);
        validate_proposal(proposal).expect("adapter proposals validate");
        assert_eq!(outcome.events.len(), 1);
        assert!(matches!(outcome.events[0].kind, EventKind::Completed(_)));
        assert_eq!(
            result.usage.input_tokens, None,
            "suite stub reports no usage"
        );
        assert_eq!(result.usage.output_tokens, None, "unknown stays unknown");
        assert!(
            proposal
                .diagnostics
                .iter()
                .any(|line| line.contains(&expect.receipt_host)),
            "receipt labels the loopback host: {proposal:?}"
        );
        assert_eq!(adapter.t_mutations(), 1);
        assert_eq!(
            adapter.t_remote_calls(),
            u32::from(expect.remote_dispatch_on_success)
        );
    }
    // Timeout exceeds the budget.
    {
        let simulated_ms = 9_000u64;
        let mut adapter = new(vec![MockStep::Timeout { simulated_ms }]);
        let asked = request(2, 3);
        assert!(
            simulated_ms > asked.budget.max_ms,
            "fixture must exceed the budget it claims to blow"
        );
        let outcome = adapter.call(&asked, policy, caps);
        assert_eq!(outcome.result, Err(AiError::Timeout));
        assert!(AiError::Timeout.retryable());
        assert!(!outcome.mutated && !outcome.dispatched_remote && !outcome.duplicate);
        assert_eq!(adapter.t_mutations(), 0);
    }
    // Malformed output fails validation.
    {
        let mut adapter = new(vec![MockStep::Malformed]);
        let asked = request(3, 3);
        let outcome = adapter.call(&asked, policy, caps);
        assert_eq!(outcome.result, Err(AiError::MalformedOutput));
        assert!(!AiError::MalformedOutput.retryable());
        assert!(!outcome.mutated);
        let bad = Proposal {
            request: asked.id,
            base_revision: asked.base_revision,
            actions: vec![ProposalAction::Noop; MAX_ACTIONS + 1],
            object_versions: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert_eq!(validate_proposal(&bad), Err(AiError::MalformedOutput));
        let bad_label = Proposal {
            request: asked.id,
            base_revision: asked.base_revision,
            actions: vec![ProposalAction::SetLabel {
                object: 1,
                label: String::new(),
            }],
            object_versions: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert_eq!(validate_proposal(&bad_label), Err(AiError::MalformedOutput));
    }
}

/// Armed and scripted cancel, stale revision and duplicate replay.
fn suite_cancel_stale_duplicate(env: &SuiteEnv<'_>) {
    let SuiteEnv {
        new,
        policy,
        caps,
        request,
        ..
    } = *env;
    // Armed cancel refuses before the script runs.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        let asked = request(4, 3);
        adapter.t_cancel(asked.id);
        let outcome = adapter.call(&asked, policy, caps);
        assert_eq!(outcome.result, Err(AiError::Cancelled));
        assert!(!AiError::Cancelled.retryable());
        assert!(!outcome.mutated);
        assert_eq!(adapter.t_script_remaining(), 1);
        assert_eq!(adapter.t_mutations(), 0);
    }
    // Scripted cancel reports cancelled.
    {
        let mut adapter = new(vec![MockStep::Cancelled]);
        let outcome = adapter.call(&request(44, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::Cancelled));
        assert!(!outcome.mutated && !outcome.dispatched_remote);
    }
    // Stale revision on mismatch; fresh revision serves again.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        adapter.t_set_revision(10);
        let outcome = adapter.call(&request(5, 9), policy, caps);
        assert_eq!(outcome.result, Err(AiError::StaleContext));
        assert!(AiError::StaleContext.retryable());
        assert!(!outcome.mutated);
        assert_eq!(adapter.t_script_remaining(), 1);
        let (_, retry, _) = call_settled(
            new,
            vec![MockStep::Succeed],
            &|a| a.t_set_revision(10),
            &SettleCtx {
                request,
                policy,
                caps,
            },
            55,
            10,
        );
        assert!(retry.result.is_ok());
    }
    // Scripted stale context reports stale.
    {
        let mut adapter = new(vec![MockStep::StaleContext]);
        let outcome = adapter.call(&request(56, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::StaleContext));
        assert!(!outcome.mutated);
    }
    // Duplicate replays the recorded result with no second mutation.
    {
        let (asked, first, mut adapter) = call_settled(
            new,
            vec![MockStep::Succeed],
            &|_| {},
            &SettleCtx {
                request,
                policy,
                caps,
            },
            6,
            3,
        );
        assert!(first.result.is_ok() && first.mutated && !first.duplicate);
        assert_eq!(adapter.t_mutations(), 1);
        let second = adapter.call(&asked, policy, caps);
        assert!(!second.mutated, "duplicate performs no second mutation");
        assert!(second.duplicate);
        assert!(second.events.is_empty(), "duplicate emits no new events");
        assert_eq!(
            second.result, first.result,
            "duplicate replays the first result"
        );
        assert_eq!(adapter.t_mutations(), 1, "mutation counter stays at one");
        assert_eq!(adapter.t_script_remaining(), 0);
    }
}

/// Unsupported capability, spend cap and queue overflow refusals.
fn suite_capability_quota_overflow(env: &SuiteEnv<'_>) {
    let SuiteEnv {
        new,
        policy,
        caps,
        request,
        ..
    } = *env;
    // Vision need without vision capability is unsupported.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        let mut asked = request(7, 3);
        asked.model.model = format!("{VISION_MODEL_PREFIX}gate-scene");
        assert!(!caps.vision, "HTTP text caps stay vision-free");
        let outcome = adapter.call(&asked, policy, caps);
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!AiError::UnsupportedCapability.retryable());
        assert!(!outcome.mutated && !outcome.dispatched_remote);
    }
    // Scripted unsupported capability reports refusal.
    {
        let mut adapter = new(vec![MockStep::UnsupportedCapability]);
        let outcome = adapter.call(&request(77, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!outcome.mutated && !outcome.dispatched_remote);
    }
    // Hard spend cap with unknown cost refuses; bounded cost serves.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        let mut capped = policy.clone();
        capped.spend_limit = Some(500);
        let asked = request(8, 3);
        assert_eq!(asked.budget.spend_limit, None, "fixture cost stays unknown");
        let outcome = adapter.call(&asked, &capped, caps);
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!AiError::QuotaAuth.retryable());
        assert!(!outcome.mutated && !outcome.dispatched_remote);
        let (_, ok, _) = call_settled(
            new,
            vec![MockStep::Succeed],
            &|_| {},
            &SettleCtx {
                request: &|id, _| {
                    let mut funded = request(88 + id, 3);
                    funded.budget.spend_limit = Some(50);
                    funded
                },
                policy: &capped,
                caps,
            },
            0,
            3,
        );
        assert!(ok.result.is_ok());
    }
    // Scripted quota/auth reports refusal.
    {
        let mut adapter = new(vec![MockStep::QuotaAuth]);
        let outcome = adapter.call(&request(89, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::QuotaAuth));
        assert!(!outcome.mutated);
    }
    // Queue overflow over max concurrency is unavailable; draining serves.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        adapter.t_set_in_flight(policy.max_concurrency);
        let outcome = adapter.call(&request(9, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(AiError::Unavailable.retryable());
        assert!(!outcome.mutated);
        assert_eq!(adapter.t_script_remaining(), 1);
        adapter.t_set_in_flight(0);
        let (_, retry, _) = call_settled(
            new,
            vec![MockStep::Succeed],
            &|_| {},
            &SettleCtx {
                request,
                policy,
                caps,
            },
            99,
            3,
        );
        assert!(retry.result.is_ok());
    }
    // Scripted queue overflow is unavailable.
    {
        let mut adapter = new(vec![MockStep::QueueOverflow]);
        let outcome = adapter.call(&request(98, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert!(!outcome.mutated);
    }
}

/// Late output discard and privacy downgrade refusals.
fn suite_late_and_privacy(env: &SuiteEnv<'_>) {
    let SuiteEnv {
        new,
        policy,
        caps,
        request,
        downgraded_request,
        ..
    } = *env;
    // Late output after cancel is discarded; unknown jobs too.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        let asked = request(10, 3);
        adapter.t_begin_job(asked.id);
        adapter.t_cancel_job(asked.id, CancelReason::SceneClosed);
        let late = AiEvent {
            request: asked.id,
            seq: 7,
            kind: EventKind::Progress(String::from("late preview")),
        };
        let delivery = adapter.t_deliver_late(asked.id, &late);
        assert!(delivery.discarded, "event after cancel is discarded");
        assert!(!delivery.mutated, "delivery never mutates");
        assert_eq!(adapter.t_mutations(), 0);
        let unknown = AiEvent {
            request: RequestId(999),
            seq: 0,
            kind: EventKind::Progress(String::from("orphan")),
        };
        assert!(adapter.t_deliver_late(RequestId(999), &unknown).discarded);
    }
    // Scripted late output is discarded.
    {
        let mut adapter = new(vec![MockStep::LateOutput]);
        let outcome = adapter.call(&request(101, 3), policy, caps);
        assert!(outcome.discarded, "late output is discarded");
        assert!(!outcome.mutated && !outcome.dispatched_remote);
        assert!(
            outcome.events.is_empty(),
            "discarded output emits no events"
        );
        assert_eq!(outcome.result, Err(AiError::Cancelled));
        assert_eq!(adapter.t_mutations(), 0);
    }
    // Privacy downgrade refuses before any new socket attempt.
    {
        let mut adapter = new(vec![MockStep::Succeed]);
        let before = adapter.t_socket_attempts();
        let outcome = adapter.call(&downgraded_request(11), policy, caps);
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!outcome.dispatched_remote, "refusal records no dispatch");
        assert_eq!(adapter.t_remote_calls(), 0, "downgrade never dispatches");
        assert_eq!(adapter.t_socket_attempts(), before, "no new socket attempt");
        assert!(!outcome.mutated);
    }
    // Scripted privacy downgrade records refusal without dispatch.
    {
        let mut adapter = new(vec![MockStep::PrivacyDowngrade]);
        let outcome = adapter.call(&request(111, 3), policy, caps);
        assert_eq!(outcome.result, Err(AiError::UnsupportedCapability));
        assert!(!outcome.dispatched_remote);
        assert_eq!(adapter.t_remote_calls(), 0);
        assert!(!outcome.mutated);
    }
}

/// Done-job late rejection and deterministic failure-fixture replay.
fn suite_done_jobs_and_determinism(env: &SuiteEnv<'_>) {
    let SuiteEnv {
        new,
        policy,
        caps,
        request,
        ..
    } = *env;
    // Done jobs reject late output; running jobs still accept.
    {
        let (asked, outcome, mut adapter) = call_settled(
            new,
            vec![MockStep::Succeed],
            &|_| {},
            &SettleCtx {
                request,
                policy,
                caps,
            },
            13,
            3,
        );
        assert!(outcome.result.is_ok());
        let late = AiEvent {
            request: asked.id,
            seq: 1,
            kind: EventKind::Progress(String::from("after done")),
        };
        assert!(adapter.t_deliver_late(asked.id, &late).discarded);
        let live = RequestId(14);
        adapter.t_begin_job(live);
        let preview = AiEvent {
            request: live,
            seq: 0,
            kind: EventKind::Progress(String::from("streaming")),
        };
        let delivery = adapter.t_deliver_late(live, &preview);
        assert!(!delivery.discarded && !delivery.mutated);
        assert_eq!(JobState::Done.cancel(CancelReason::User), JobState::Done);
    }
    // Same script twice gives the same outcome sequence.
    {
        let script = vec![
            MockStep::Succeed,
            MockStep::Timeout {
                simulated_ms: 9_000,
            },
            MockStep::Malformed,
            MockStep::QuotaAuth,
        ];
        let run = |base: u64| {
            let mut adapter = new(script.clone());
            (20..24)
                .map(|id| adapter.call(&request(id, base), policy, caps))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3), "deterministic failure fixtures replay");
    }
}

impl SuiteAdapter for crate::local::LocalAdapter {
    fn t_cancel(&mut self, id: RequestId) {
        self.cancel(id);
    }
    fn t_begin_job(&mut self, id: RequestId) {
        self.begin_job(id);
    }
    fn t_cancel_job(&mut self, id: RequestId, reason: CancelReason) {
        self.cancel_job(id, reason);
    }
    fn t_deliver_late(&self, id: RequestId, event: &AiEvent) -> LateOutcome {
        self.deliver_late(id, event)
    }
    fn t_set_revision(&mut self, revision: u64) {
        self.set_revision(revision);
    }
    fn t_set_in_flight(&mut self, count: u32) {
        self.set_in_flight(count);
    }
    fn t_script_remaining(&self) -> usize {
        self.script_remaining()
    }
    fn t_mutations(&self) -> u64 {
        self.mutations()
    }
    fn t_socket_attempts(&self) -> u64 {
        self.socket_attempts()
    }
    fn t_remote_calls(&self) -> u32 {
        self.remote_calls()
    }
}

impl SuiteAdapter for crate::remote::RemoteAdapter {
    fn t_cancel(&mut self, id: RequestId) {
        self.cancel(id);
    }
    fn t_begin_job(&mut self, id: RequestId) {
        self.begin_job(id);
    }
    fn t_cancel_job(&mut self, id: RequestId, reason: CancelReason) {
        self.cancel_job(id, reason);
    }
    fn t_deliver_late(&self, id: RequestId, event: &AiEvent) -> LateOutcome {
        self.deliver_late(id, event)
    }
    fn t_set_revision(&mut self, revision: u64) {
        self.set_revision(revision);
    }
    fn t_set_in_flight(&mut self, count: u32) {
        self.set_in_flight(count);
    }
    fn t_script_remaining(&self) -> usize {
        self.script_remaining()
    }
    fn t_mutations(&self) -> u64 {
        self.mutations()
    }
    fn t_socket_attempts(&self) -> u64 {
        self.socket_attempts()
    }
    fn t_remote_calls(&self) -> u32 {
        self.remote_calls()
    }
}

#[cfg(test)]
mod settle_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::core::{AdapterOutcome, LateOutcome, MockStep};
    use crate::mock::{fixtures, ProviderAdapter};
    use crate::policy::{AiSessionPolicy, CancelReason, ProviderCapabilities};
    use crate::protocol::{
        AiError, AiEvent, AiRequest, AiResult, EventKind, FinishReason, Proposal, ProposalAction,
        RequestId, Usage,
    };

    use super::{call_settled, SettleCtx, SuiteAdapter};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum FakeMode {
        Succeed,
        Unavailable,
    }

    struct FakeSettle {
        mode: FakeMode,
        seen: Rc<RefCell<Vec<u64>>>,
    }

    impl ProviderAdapter for FakeSettle {
        fn call(
            &mut self,
            request: &AiRequest,
            _policy: &AiSessionPolicy,
            _capabilities: &ProviderCapabilities,
        ) -> AdapterOutcome {
            self.seen.borrow_mut().push(request.id.0);
            match self.mode {
                FakeMode::Succeed => {
                    let proposal = Proposal {
                        request: request.id,
                        base_revision: request.base_revision,
                        actions: vec![ProposalAction::Noop],
                        object_versions: Vec::new(),
                        diagnostics: vec![String::from("mock:ok")],
                    };
                    let result = AiResult {
                        request: request.id,
                        reason: FinishReason::Complete,
                        proposal: Some(proposal),
                        usage: Usage::default(),
                    };
                    let event = AiEvent {
                        request: request.id,
                        seq: 0,
                        kind: EventKind::Completed(result.clone()),
                    };
                    AdapterOutcome {
                        result: Ok(result),
                        events: vec![event],
                        dispatched_remote: false,
                        duplicate: false,
                        mutated: true,
                        discarded: false,
                    }
                }
                FakeMode::Unavailable => {
                    let event = AiEvent {
                        request: request.id,
                        seq: 0,
                        kind: EventKind::Failed(AiError::Unavailable),
                    };
                    AdapterOutcome {
                        result: Err(AiError::Unavailable),
                        events: vec![event],
                        dispatched_remote: false,
                        duplicate: false,
                        mutated: false,
                        discarded: false,
                    }
                }
            }
        }
    }

    impl SuiteAdapter for FakeSettle {
        fn t_cancel(&mut self, _id: RequestId) {}
        fn t_begin_job(&mut self, _id: RequestId) {}
        fn t_cancel_job(&mut self, _id: RequestId, _reason: CancelReason) {}
        fn t_deliver_late(&self, _id: RequestId, _event: &AiEvent) -> LateOutcome {
            LateOutcome {
                discarded: true,
                mutated: false,
            }
        }
        fn t_set_revision(&mut self, _revision: u64) {}
        fn t_set_in_flight(&mut self, _count: u32) {}
        fn t_script_remaining(&self) -> usize {
            0
        }
        fn t_mutations(&self) -> u64 {
            0
        }
        fn t_socket_attempts(&self) -> u64 {
            0
        }
        fn t_remote_calls(&self) -> u32 {
            0
        }
    }

    #[test]
    fn settle_returns_first_success_with_base_id() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let factory_seen = Rc::clone(&seen);
        let new = move |_script: Vec<MockStep>| {
            Box::new(FakeSettle {
                mode: FakeMode::Succeed,
                seen: Rc::clone(&factory_seen),
            }) as Box<dyn SuiteAdapter>
        };
        let policy = fixtures::policy();
        let caps = fixtures::caps();
        let ctx = SettleCtx {
            request: &fixtures::request,
            policy: &policy,
            caps: &caps,
        };
        let (asked, outcome, _adapter) = call_settled(&new, Vec::new(), &|_| {}, &ctx, 11, 3);
        assert_eq!(asked.id.0, 11, "first attempt uses the base id");
        assert!(outcome.result.is_ok(), "success settles at once");
        assert!(!outcome.duplicate && !outcome.discarded);
        assert_eq!(
            *seen.borrow(),
            vec![11],
            "one fresh adapter for one success"
        );
    }

    #[test]
    fn settle_retries_unavailable_eight_times_with_stepped_ids() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let factory_seen = Rc::clone(&seen);
        let new = move |_script: Vec<MockStep>| {
            Box::new(FakeSettle {
                mode: FakeMode::Unavailable,
                seen: Rc::clone(&factory_seen),
            }) as Box<dyn SuiteAdapter>
        };
        let policy = fixtures::policy();
        let caps = fixtures::caps();
        let ctx = SettleCtx {
            request: &fixtures::request,
            policy: &policy,
            caps: &caps,
        };
        let (asked, outcome, _adapter) = call_settled(&new, Vec::new(), &|_| {}, &ctx, 7, 3);
        assert_eq!(outcome.result, Err(AiError::Unavailable));
        assert_eq!(asked.id.0, 7007, "eighth attempt steps by 1000 each retry");
        assert_eq!(
            *seen.borrow(),
            vec![7, 1007, 2007, 3007, 4007, 5007, 6007, 7007],
            "unavailable retries with a fresh id per attempt, bounded at eight"
        );
    }
}
