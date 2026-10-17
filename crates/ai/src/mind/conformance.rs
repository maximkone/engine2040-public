//! Conformance suite for the backend adapters, run by the mind tests.

use super::*;

/// One conformance suite every backend adapter must pass: success, timeout,
/// malformed, armed cancel, scripted cancel, stale revision, duplicate
/// replay, unsupported capability, quota, queue overflow, late output,
/// privacy downgrade, offline fallback and deterministic replay. Panics name
/// the failing arm and backend.
pub(super) fn conformance_suite<A: MindAdapter>(
    adapter: &mut A,
    new: &dyn Fn(Vec<ScriptStep>) -> A,
    make_req: &dyn Fn(u64, u64) -> MindRequest,
) {
    let backend = adapter.name();
    let mut mind = Mind::new(NpcId(3), 11);
    mind.observe(1, MemoryKind::Seen, "a stranger enters the ludus", 800)
        .expect("fixture observes");
    mind.observe(2, MemoryKind::Heard, "the crowd chants at dusk", 300)
        .expect("fixture observes");
    mind.add_goal("guard the gate", 700).expect("fixture goal");

    // Success completes with one validated provider action.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let req = make_req(1, 3);
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("success asks");
        assert_eq!(out.status, AskStatus::Provider, "{backend}: success status");
        assert_eq!(out.actions.len(), 1, "{backend}: one action");
        assert_eq!(
            out.actions[0].source,
            ActionSource::Provider,
            "{backend}: source"
        );
        assert_eq!(out.actions[0].request, req.id, "{backend}: request stamp");
        assert!(
            out.actions[0].text.contains(backend),
            "{backend}: receipt tags backend"
        );
        assert_eq!(backend_adapter.mutations(), 1, "{backend}: one mutation");
        assert_eq!(rt.mutations(), 1, "{backend}: runtime counts one");
    }
    // Timeout (simulated ms over budget) falls back scripted.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Timeout {
            simulated_ms: 9_000,
        }]);
        let req = make_req(2, 3);
        assert!(
            9_000 > req.budget_ms,
            "{backend}: fixture must blow the budget it claims"
        );
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("timeout asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Timeout),
            "{backend}: timeout status"
        );
        assert_eq!(
            out.actions[0].source,
            ActionSource::Scripted,
            "{backend}: fallback scripted"
        );
        assert_eq!(rt.mutations(), 0, "{backend}: no mutation on timeout");
    }
    // Malformed output fails validation into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Malformed]);
        let req = make_req(3, 3);
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("malformed asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Malformed),
            "{backend}: malformed status"
        );
        assert!(validate_proposal(&Proposal {
            request: req.id,
            drafts: Vec::new(),
            note: String::new(),
        })
        .is_err());
        let bad = ActionDraft {
            kind: ActionKind::Say,
            text: String::new(),
            delay_ticks: 0,
        };
        assert!(check_text(&bad.text).is_err());
    }
    // Armed cancel refuses before the script runs.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let req = make_req(4, 3);
        rt.cancel(req.id);
        let before = backend_adapter.script_remaining();
        assert_eq!(
            rt.ask(&mind, &req, &mut backend_adapter),
            Err(MindError::Cancelled)
        );
        assert_eq!(
            backend_adapter.script_remaining(),
            before,
            "{backend}: cancel consumes no script"
        );
        assert!(
            rt.recorded().is_empty(),
            "{backend}: cancel records nothing"
        );
    }
    // Scripted cancel reports cancelled into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Cancelled]);
        let out = rt
            .ask(&mind, &make_req(44, 3), &mut backend_adapter)
            .expect("cancel asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Cancelled),
            "{backend}: cancel status"
        );
        assert!(
            backend_adapter.is_cancelled(44),
            "{backend}: cancel tracked"
        );
    }
    // Stale revision on mismatch; fresh revision serves again.
    {
        let mut rt = MindRuntime::new();
        rt.set_revision(10);
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        assert_eq!(
            rt.ask(&mind, &make_req(5, 9), &mut backend_adapter),
            Err(MindError::StaleContext),
            "{backend}: stale refused"
        );
        let mut fresh = new(vec![ScriptStep::Succeed]);
        let out = rt
            .ask(&mind, &make_req(55, 10), &mut fresh)
            .expect("fresh asks");
        assert_eq!(out.status, AskStatus::Provider, "{backend}: fresh serves");
    }
    // Scripted stale arm reports stale into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Stale]);
        let out = rt
            .ask(&mind, &make_req(56, 3), &mut backend_adapter)
            .expect("stale asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::StaleContext),
            "{backend}: stale status"
        );
    }
    // Duplicate replays the recorded answer with no second mutation.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let req = make_req(6, 3);
        let first = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("first asks");
        assert_eq!(first.status, AskStatus::Provider);
        let calls = backend_adapter.calls();
        let second = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("replay asks");
        assert_eq!(
            second.status,
            AskStatus::Duplicate,
            "{backend}: duplicate status"
        );
        assert_eq!(
            second.actions, first.actions,
            "{backend}: duplicate replays answer"
        );
        assert_eq!(backend_adapter.calls(), calls, "{backend}: no second call");
        assert_eq!(
            rt.mutations(),
            1,
            "{backend}: mutation counter stays at one"
        );
    }
    // Unsupported capability falls back without serving.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Unsupported]);
        let out = rt
            .ask(&mind, &make_req(7, 3), &mut backend_adapter)
            .expect("unsupported asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::UnsupportedCapability),
            "{backend}: unsupported status"
        );
        assert_eq!(rt.mutations(), 0);
    }
    // Quota refusal falls back without serving.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Quota]);
        let out = rt
            .ask(&mind, &make_req(8, 3), &mut backend_adapter)
            .expect("quota asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Quota),
            "{backend}: quota status"
        );
    }
    // Queue overflow over max concurrency falls back; draining serves.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        backend_adapter.set_in_flight(4);
        let out = rt
            .ask(&mind, &make_req(9, 3), &mut backend_adapter)
            .expect("overflow asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Unavailable),
            "{backend}: overflow status"
        );
        backend_adapter.set_in_flight(0);
        let served = rt
            .ask(&mind, &make_req(99, 3), &mut backend_adapter)
            .expect("drained asks");
        assert_eq!(
            served.status,
            AskStatus::Provider,
            "{backend}: drained serves"
        );
    }
    // Scripted overflow arm is unavailable.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Overflow]);
        let out = rt
            .ask(&mind, &make_req(98, 3), &mut backend_adapter)
            .expect("overflow asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Unavailable),
            "{backend}: overflow status"
        );
    }
    // Late output after cancel is discarded; unknown jobs too.
    {
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        backend_adapter.begin_job(10);
        backend_adapter.cancel(10);
        let delivery = backend_adapter.deliver_late(10);
        assert!(
            delivery.discarded,
            "{backend}: event after cancel is discarded"
        );
        assert!(!delivery.mutated, "{backend}: delivery never mutates");
        assert!(
            backend_adapter.deliver_late(999).discarded,
            "{backend}: unknown job discarded"
        );
    }
    // Scripted late arm discards into fallback.
    {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Late]);
        let out = rt
            .ask(&mind, &make_req(101, 3), &mut backend_adapter)
            .expect("late asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Cancelled),
            "{backend}: late status"
        );
        assert_eq!(out.actions[0].source, ActionSource::Scripted);
    }
    // Privacy downgrade: remote under local-only refuses without dispatch.
    // Mock/local backends ignore this arm (their contract has no remote leg).
    if backend == "remote" {
        let mut rt = MindRuntime::new();
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let mut req = make_req(11, 3);
        req.privacy = Privacy::LocalOnly;
        let out = rt
            .ask(&mind, &req, &mut backend_adapter)
            .expect("downgrade asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::UnsupportedCapability),
            "{backend}: downgrade status"
        );
        assert_eq!(
            backend_adapter.remote_calls(),
            0,
            "{backend}: downgrade never dispatches"
        );
    }
    // Offline runtime never calls the adapter.
    {
        let mut rt = MindRuntime::new();
        rt.set_online(false);
        let mut backend_adapter = new(vec![ScriptStep::Succeed]);
        let calls = backend_adapter.calls();
        let out = rt
            .ask(&mind, &make_req(12, 3), &mut backend_adapter)
            .expect("offline asks");
        assert_eq!(
            out.status,
            AskStatus::Fallback(MindError::Offline),
            "{backend}: offline status"
        );
        assert_eq!(
            backend_adapter.calls(),
            calls,
            "{backend}: offline performs no call"
        );
        assert!(out.actions[0].scripted_or_provider_is_scripted());
    }
    // Same script twice gives the same outcome sequence.
    {
        let script = vec![
            ScriptStep::Succeed,
            ScriptStep::Timeout {
                simulated_ms: 9_000,
            },
            ScriptStep::Malformed,
            ScriptStep::Quota,
        ];
        let run = |base: u64| {
            let mut rt = MindRuntime::new();
            let mut backend_adapter = new(script.clone());
            (20..24)
                .map(|id| {
                    rt.ask(&mind, &make_req(id, base), &mut backend_adapter)
                        .expect("suite asks")
                        .status
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(3), run(3), "{backend}: deterministic fixtures replay");
    }
    let _ = adapter.calls();
}

impl MindAction {
    fn scripted_or_provider_is_scripted(&self) -> bool {
        self.source == ActionSource::Scripted
    }
}
