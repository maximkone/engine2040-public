//! fallback: provider timeout/refusal to the scripted policy path.
//!
//! Consumer: the provider loop. Data owner: this module owns the
//! cause buckets and the scripted proposal; the caller owns the request,
//! the session policy and the observe ring the notice is filed in.
//! Units: timeouts in milliseconds (`u64`). Determinism: the same error
//! plus the same request gives the same proposal and notice, no clock.
//! Errors: [`FallbackErr`]; cancelled or stale requests propagate and a
//! policy without scripted fallback refuses loudly, never silently.

use super::core::validate_proposal;
use super::policy::AiSessionPolicy;
use super::protocol::{AiError, AiRequest, Proposal, ProposalAction};
use super::stream::truncate_str;

/// Longest fallback notice message, in bytes. The message fits an observe
/// event verbatim (`MAX_MESSAGE_LEN` is 256 there; this crate stays std-only
/// and never imports observe, so the bound is restated
/// rather than mirrored).
pub const MAX_NOTICE_LEN: usize = 256;

/// Why one request takes the scripted path. Timeouts cover exhausted
/// budgets and unreachable endpoints (the supervisor `provider.timeout`
/// row, `Degrade` advice); refusals cover quota, capability and validation
/// failures (the supervisor `provider.malformed` row, `ResetBudget`
/// advice). Cancelled and stale requests are not fallback cases: they
/// propagate so the caller can cancel or rebuild against a fresh revision.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FallbackCause {
    /// Deadline or budget exhausted, or the endpoint unreachable.
    Timeout,
    /// Quota, capability, privacy or validation refusal.
    Refused,
}

impl FallbackCause {
    /// Bucket one adapter error into the scripted path, or `None` when the
    /// error must propagate instead of falling back.
    #[must_use]
    pub fn from_error(error: AiError) -> Option<Self> {
        match error {
            AiError::Timeout | AiError::Unavailable => Some(FallbackCause::Timeout),
            AiError::QuotaAuth | AiError::UnsupportedCapability | AiError::MalformedOutput => {
                Some(FallbackCause::Refused)
            }
            AiError::Cancelled | AiError::StaleContext => None,
        }
    }

    /// Short label used in notices and scripted diagnostics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FallbackCause::Timeout => "timeout",
            FallbackCause::Refused => "refused",
        }
    }

    /// Observe dotted code name the provider loop files this notice under.
    /// Names, not raw codes: the loop resolves them through the real
    /// `engine-observe` registry, so this adapter never mirrors it.
    #[must_use]
    pub fn observe_code_name(self) -> &'static str {
        match self {
            FallbackCause::Timeout => "provider.timeout",
            FallbackCause::Refused => "provider.malformed",
        }
    }
}

/// One fallback notice for the observe ring. The provider loop pushes
/// `message` under [`FallbackCause::observe_code_name`] at the request
/// tick; the text already fits the event bound.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FallbackNotice {
    /// Which scripted-path bucket served this request.
    pub cause: FallbackCause,
    /// The adapter error that triggered the fallback.
    pub error: AiError,
    /// Request budget the provider blew or refused under, in milliseconds.
    pub budget_ms: u64,
    /// Ring-ready notice text, at most [`MAX_NOTICE_LEN`] bytes.
    pub message: String,
}

impl FallbackNotice {
    /// Build the notice for one fallback. The text is fixed labels plus
    /// integers, so it always fits; truncation is defensive only.
    #[must_use]
    pub fn new(cause: FallbackCause, error: AiError, budget_ms: u64) -> Self {
        let message = format!(
            "provider.fallback cause={} error={error:?} budget_ms={budget_ms} scripted",
            cause.label()
        );
        Self {
            cause,
            error,
            budget_ms,
            message: truncate_str(&message, MAX_NOTICE_LEN).to_owned(),
        }
    }
}

/// Scripted service for one failed request: a validating no-op proposal
/// plus the notice the provider loop files. The proposal carries the failed
/// request id and revision so downstream validation and replay see the
/// same shape a provider success would have carried.
#[derive(Clone, PartialEq, Debug)]
pub struct ScriptedFallback {
    /// Which scripted-path bucket served this request.
    pub cause: FallbackCause,
    /// Ring-ready notice for the provider loop to file.
    pub notice: FallbackNotice,
    /// Validating no-op proposal standing in for the provider answer.
    pub proposal: Proposal,
}

/// Typed fallback failure. Both variants are loud: the caller surfaces
/// them instead of guessing provider output.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FallbackErr {
    /// The session policy forbids scripted service (`scripted_ok: false`),
    /// so even a timeout has no fallback path. Carries the bucket and the
    /// error the policy refused to absorb.
    ScriptedRefused {
        /// Bucket the request would have taken.
        cause: FallbackCause,
        /// The adapter error the policy refused to absorb.
        error: AiError,
    },
    /// Not a fallback case: cancelled requests stay cancelled and stale
    /// requests rebuild against a fresh revision. Carries the error back.
    Propagate(AiError),
}

impl std::fmt::Display for FallbackErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FallbackErr::ScriptedRefused { cause, error } => write!(
                f,
                "scripted fallback refused for {} ({error:?}): policy forbids scripting",
                cause.label()
            ),
            FallbackErr::Propagate(error) => {
                write!(f, "provider error propagates without fallback: {error:?}")
            }
        }
    }
}

impl std::error::Error for FallbackErr {}

/// Serve one failed request on the scripted policy path: timeout and
/// refusal errors become a validating no-op proposal plus a ring-ready
/// notice. A policy without `scripted_ok` refuses loudly; cancelled and
/// stale errors propagate untouched.
pub fn fallback_for(
    error: AiError,
    request: &AiRequest,
    policy: &AiSessionPolicy,
) -> Result<ScriptedFallback, FallbackErr> {
    let Some(cause) = FallbackCause::from_error(error) else {
        return Err(FallbackErr::Propagate(error));
    };
    if !policy.fallback.scripted_ok {
        return Err(FallbackErr::ScriptedRefused { cause, error });
    }
    let notice = FallbackNotice::new(cause, error, request.budget.max_ms);
    let proposal = Proposal {
        request: request.id,
        base_revision: request.base_revision,
        actions: vec![ProposalAction::Noop],
        object_versions: Vec::new(),
        diagnostics: vec![format!(
            "scripted-fallback cause={} budget_ms={}",
            cause.label(),
            request.budget.max_ms
        )],
    };
    debug_assert!(validate_proposal(&proposal).is_ok());
    Ok(ScriptedFallback {
        cause,
        notice,
        proposal,
    })
}

#[cfg(test)]
mod tests {
    use super::super::mock::{fixtures, MockAdapter, MockStep, ProviderAdapter};
    use super::*;

    fn policy() -> AiSessionPolicy {
        fixtures::policy()
    }

    /// Chaos sweep: every timeout/refusal error takes the scripted policy
    /// path with a validating proposal and a ring-ready notice.
    #[test]
    fn every_timeout_or_refusal_falls_back_to_scripted() {
        let rows: Vec<(AiError, FallbackCause, &str)> = vec![
            (AiError::Timeout, FallbackCause::Timeout, "provider.timeout"),
            (
                AiError::Unavailable,
                FallbackCause::Timeout,
                "provider.timeout",
            ),
            (
                AiError::QuotaAuth,
                FallbackCause::Refused,
                "provider.malformed",
            ),
            (
                AiError::UnsupportedCapability,
                FallbackCause::Refused,
                "provider.malformed",
            ),
            (
                AiError::MalformedOutput,
                FallbackCause::Refused,
                "provider.malformed",
            ),
        ];
        for (error, cause, code_name) in rows {
            let mut request = fixtures::request(1, 3);
            request.budget.max_ms = 50;
            let served =
                fallback_for(error, &request, &policy()).expect("timeout/refusal falls back");
            assert_eq!(served.cause, cause, "error {error:?}");
            assert_eq!(served.notice.cause, cause, "error {error:?}");
            assert_eq!(served.notice.error, error, "error {error:?}");
            assert_eq!(served.notice.budget_ms, 50, "error {error:?}");
            assert_eq!(
                served.notice.cause.observe_code_name(),
                code_name,
                "error {error:?}"
            );
            assert!(
                served.notice.message.contains("budget_ms=50"),
                "notice carries ms timeouts: {}",
                served.notice.message
            );
            assert!(
                served.notice.message.contains("scripted"),
                "notice names the scripted path: {}",
                served.notice.message
            );
            assert!(
                served.notice.message.len() <= MAX_NOTICE_LEN,
                "notice fits the ring bound"
            );
            validate_proposal(&served.proposal).expect("scripted proposal validates");
            assert_eq!(served.proposal.request, request.id);
            assert_eq!(served.proposal.base_revision, request.base_revision);
            assert_eq!(served.proposal.actions, vec![ProposalAction::Noop]);
        }
        assert_eq!(FallbackCause::Timeout.label(), "timeout");
        assert_eq!(FallbackCause::Refused.label(), "refused");
    }

    #[test]
    fn cancelled_and_stale_propagate_without_fallback() {
        let request = fixtures::request(2, 3);
        for error in [AiError::Cancelled, AiError::StaleContext] {
            assert_eq!(FallbackCause::from_error(error), None);
            assert_eq!(
                fallback_for(error, &request, &policy()),
                Err(FallbackErr::Propagate(error))
            );
        }
        let propagated = FallbackErr::Propagate(AiError::Cancelled);
        assert!(propagated.to_string().contains("propagates"));
        let _: &dyn std::error::Error = &propagated;
    }

    #[test]
    fn policy_without_scripted_path_refuses_loudly() {
        let mut strict = policy();
        strict.fallback.scripted_ok = false;
        let request = fixtures::request(3, 3);
        for error in [
            AiError::Timeout,
            AiError::Unavailable,
            AiError::QuotaAuth,
            AiError::UnsupportedCapability,
            AiError::MalformedOutput,
        ] {
            let refused = fallback_for(error, &request, &strict)
                .expect_err("policy without scripting must refuse");
            assert!(
                matches!(refused, FallbackErr::ScriptedRefused { .. }),
                "error {error:?} refuses, never scripts"
            );
            assert!(refused.to_string().contains("refused"));
        }
        // Propagation still wins over the policy flag: cancelled is never
        // scripted, even where scripting is allowed or not.
        assert_eq!(
            fallback_for(AiError::Cancelled, &request, &strict),
            Err(FallbackErr::Propagate(AiError::Cancelled))
        );
    }

    /// End to end through the real adapter contract: scripted adapter
    /// failures become scripted proposals the provider loop can validate.
    #[test]
    fn adapter_outcomes_reach_the_scripted_path() {
        let script: Vec<(MockStep, AiError)> = vec![
            (
                MockStep::Timeout {
                    simulated_ms: 9_000,
                },
                AiError::Timeout,
            ),
            (MockStep::QueueOverflow, AiError::Unavailable),
            (MockStep::QuotaAuth, AiError::QuotaAuth),
            (
                MockStep::UnsupportedCapability,
                AiError::UnsupportedCapability,
            ),
            (MockStep::Malformed, AiError::MalformedOutput),
            (MockStep::PrivacyDowngrade, AiError::UnsupportedCapability),
        ];
        let session = policy();
        let caps = fixtures::caps();
        for (step, error) in script {
            let mut adapter = MockAdapter::new(vec![step]);
            let request = fixtures::request(40 + error as u64, 3);
            let outcome = adapter.call(&request, &session, &caps);
            assert_eq!(outcome.result, Err(error), "adapter surfaces {error:?}");
            assert!(!outcome.mutated, "failure mutates nothing");
            let served = fallback_for(error, &request, &session).expect("consumer falls back");
            validate_proposal(&served.proposal).expect("consumer proposal validates");
            assert_eq!(served.notice.error, error);
        }
    }

    #[test]
    fn same_error_twice_gives_same_proposal_and_notice() {
        let request = fixtures::request(9, 3);
        let session = policy();
        let first = fallback_for(AiError::Timeout, &request, &session).expect("falls back");
        let second = fallback_for(AiError::Timeout, &request, &session).expect("falls back");
        assert_eq!(first, second);
    }

    #[test]
    fn notice_stays_bounded_on_extreme_budgets() {
        let mut request = fixtures::request(10, 3);
        request.budget.max_ms = u64::MAX;
        let served = fallback_for(AiError::Timeout, &request, &policy()).expect("falls back");
        assert_eq!(served.notice.budget_ms, u64::MAX);
        assert!(served.notice.message.len() <= MAX_NOTICE_LEN);
        assert!(served.notice.message.contains(&u64::MAX.to_string()));
    }
}
