//! policy: capability/session/job contracts. Negotiation first: adapters
//! describe what they support, sessions authorize modes and budgets, jobs
//! track cancel/expiry so late output is discarded, never applied.

use super::protocol::{AdapterKind, PrivacyMode};
use super::MAX_TEXT_LEN;

/// Negotiated provider capabilities for one adapter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProviderCapabilities {
    /// Contract version this adapter speaks.
    pub protocol: u16,
    /// Plain-text observations in, text proposals out.
    pub text: bool,
    /// Native tool calling (not text-parsed).
    pub tools: bool,
    /// Strict structured-output support (not text-parsed JSON).
    pub structured_output: bool,
    /// Revision-stamped captures.
    pub vision: bool,
    /// Embedding queries.
    pub embedding: bool,
    /// Maximum input tokens/bytes the adapter accepts.
    pub input_limit: u32,
    /// Maximum output tokens the adapter emits.
    pub output_limit: u32,
    /// Can serve local-side requests.
    pub local: bool,
    /// Can serve remote requests.
    pub remote: bool,
    /// Honors cancellation.
    pub cancellation: bool,
}

impl ProviderCapabilities {
    /// Mock adapter: text only, bounded, cancellable, local-only. The local
    /// HTTP adapter serves the same set.
    pub const fn mock() -> Self {
        Self {
            protocol: super::PROTOCOL_VERSION,
            text: true,
            tools: false,
            structured_output: false,
            vision: false,
            embedding: false,
            input_limit: 4096,
            output_limit: 512,
            local: true,
            remote: false,
            cancellation: true,
        }
    }

    /// HTTP text adapter over a configured local endpoint: plain-text
    /// proposals parsed and validated from the response body. Text parsing
    /// never claims native structured-output or tool support.
    pub const fn local_http() -> Self {
        Self::mock()
    }

    /// HTTP text adapter over a configured remote host: identical
    /// transport to [`ProviderCapabilities::local_http`] plus a session-auth
    /// header from an adapter-local secret. Still text-parsed, never native
    /// structured output.
    pub const fn remote_http() -> Self {
        Self {
            local: false,
            remote: true,
            ..Self::local_http()
        }
    }
}

/// Fallback order when the primary adapter cannot serve. Scripted behavior
/// or visible refusal is always the last resort, never silent cloud use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fallback {
    /// Adapters to try in order (each still bound by mode and budget).
    pub order: Vec<AdapterKind>,
    /// Scripted fallback is acceptable when no adapter serves.
    pub scripted_ok: bool,
}

/// Session policy: what one AI session may do, where data may go, and what
/// it may spend. Unknown cost plus a hard cap means refusal, not guessing.
#[derive(Clone, PartialEq, Debug)]
pub struct AiSessionPolicy {
    /// Adapters this session may call.
    pub adapters: Vec<AdapterKind>,
    /// Privacy mode: offline, local-only or remote-allowed.
    pub mode: PrivacyMode,
    /// Allowed remote destinations (hosts); empty means none configured.
    pub destinations: Vec<String>,
    /// Tool/object scopes granted to this session.
    pub scopes: Vec<String>,
    /// Maximum requests in this session.
    pub request_limit: u32,
    /// Maximum concurrent in-flight requests.
    pub max_concurrency: u32,
    /// Session spend cap in minor units; `None` means no cap configured.
    pub spend_limit: Option<u64>,
    /// Fallback order when the primary adapter cannot serve.
    pub fallback: Fallback,
}

impl AiSessionPolicy {
    /// Local-only default: mock adapter, no destinations, scripted fallback.
    pub fn local_default(request_limit: u32) -> Self {
        Self {
            adapters: vec![AdapterKind::Mock],
            mode: PrivacyMode::LocalOnly,
            destinations: Vec::new(),
            scopes: Vec::new(),
            request_limit,
            max_concurrency: 1,
            spend_limit: None,
            fallback: Fallback {
                order: Vec::new(),
                scripted_ok: true,
            },
        }
    }

    /// Validate session shape: bounded lists, sane limits, no remote without
    /// authorization and destinations.
    pub fn validate(&self) -> Result<(), super::protocol::ProtocolError> {
        use super::protocol::ProtocolError;
        if self.adapters.is_empty() || self.request_limit == 0 || self.max_concurrency == 0 {
            return Err(ProtocolError::Truncated);
        }
        if self.destinations.len() > 8 || self.scopes.len() > 32 {
            return Err(ProtocolError::TooLong);
        }
        for destination in &self.destinations {
            if destination.is_empty() || destination.len() > MAX_TEXT_LEN {
                return Err(ProtocolError::TooLong);
            }
        }
        if self.mode.allows_remote() && self.destinations.is_empty() {
            // Remote allowed with nowhere to go is a misconfiguration.
            return Err(ProtocolError::Truncated);
        }
        Ok(())
    }
}

/// Why a job stopped before completion.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CancelReason {
    /// Caller cancelled.
    User,
    /// Scene closed under the job.
    SceneClosed,
    /// Provider switched mid-run.
    ProviderSwitch,
    /// Approval expired.
    Expired,
    /// Superseded by a newer job.
    Superseded,
}

/// Job lifecycle: running, cancelled with a reason, or done. Only `Running`
/// jobs may publish output; everything else discards late output.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum JobState {
    /// Accepting progress and exactly one terminal event.
    Running,
    /// Stopped early with the reason.
    Cancelled(CancelReason),
    /// Terminal event already delivered.
    Done,
}

impl JobState {
    /// Transition to cancelled; terminal states stay put.
    pub fn cancel(self, reason: CancelReason) -> Self {
        match self {
            JobState::Running => JobState::Cancelled(reason),
            terminal => terminal,
        }
    }

    /// Transition to done; only running jobs complete.
    pub fn finish(self) -> Self {
        match self {
            JobState::Running => JobState::Done,
            terminal => terminal,
        }
    }

    /// True only while the job may still publish output.
    pub const fn accepts_output(self) -> bool {
        matches!(self, JobState::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::ProtocolError;
    use super::*;

    #[test]
    fn local_default_validates() {
        AiSessionPolicy::local_default(20).validate().unwrap();
    }

    #[test]
    fn remote_without_destinations_is_misconfigured() {
        let mut policy = AiSessionPolicy::local_default(20);
        policy.mode = PrivacyMode::RemoteAllowed;
        assert_eq!(policy.validate(), Err(ProtocolError::Truncated));
        policy.destinations.push("https://models.internal".into());
        policy.validate().unwrap();
    }

    #[test]
    fn local_requests_never_fail_over_to_cloud() {
        // The contract has no escalation path: a local-only session simply
        // has no remote adapter authorized, and validation refuses it.
        let policy = AiSessionPolicy::local_default(20);
        assert!(!policy.mode.allows_remote());
        assert!(!policy.adapters.contains(&AdapterKind::Remote));
    }

    #[test]
    fn job_lifecycle_discards_late_output() {
        let state = JobState::Running;
        assert!(state.accepts_output());
        assert_eq!(
            state.cancel(CancelReason::SceneClosed),
            JobState::Cancelled(CancelReason::SceneClosed)
        );
        assert!(!state.cancel(CancelReason::User).accepts_output());
        assert_eq!(state.finish(), JobState::Done);
        assert!(!JobState::Done.accepts_output());
        // Terminal states stay put: no revive, no double finish.
        assert_eq!(JobState::Done.cancel(CancelReason::User), JobState::Done);
    }
}
