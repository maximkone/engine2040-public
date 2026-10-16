//! core: shared contract state for every provider adapter. The mock,
//! local and remote adapters run identical guards, jobs and scripted failure
//! arms here so the contract suite ports unchanged between backends. Only the
//! `Succeed` arm differs: the mock completes a canned proposal while the
//! local/remote adapters perform an HTTP roundtrip first.

use std::collections::{HashMap, HashSet};

use super::policy::{AiSessionPolicy, CancelReason, JobState, ProviderCapabilities};
use super::protocol::{
    validate_request, AiError, AiEvent, AiRequest, AiResult, EventKind, FinishReason, Proposal,
    ProposalAction, RequestId, Usage,
};
use super::{MAX_ACTIONS, MAX_TEXT_LEN};

/// Model names with this prefix need the `vision` capability. Adapters whose
/// capabilities report `vision: false` fail such requests with
/// [`AiError::UnsupportedCapability`]; an adapter with vision passes them
/// through to its script. This keeps vision needs visible without changing
/// the committed request shape.
pub const VISION_MODEL_PREFIX: &str = "vision:";

/// Longest label text a proposal may carry, in bytes and characters.
/// Mirrors the `1..=128` chars bound on [`ProposalAction::SetLabel`].
pub(crate) const MAX_LABEL_LEN: usize = 128;

/// Grammar bound on one position axis, in centimeters (±10 km). Out-of-range
/// coordinates fail fast with no new dependency.
pub(crate) const MAX_COORD_CM: i32 = 1_000_000;

/// Adapter outcome envelope shared by every backend so one contract suite
/// covers all of them. `ProviderAdapter::call` returns this type.
pub type AdapterOutcome = MockOutcome;

/// One scripted behavior. Each failure mode in the provider contract has a step;
/// guards (duplicate, cancel, revision, queue, vision, spend, privacy) run
/// before the script so they hold regardless of the scripted sequence.
#[derive(Clone, PartialEq, Debug)]
pub enum MockStep {
    /// Complete validated proposal with unknown usage.
    Succeed,
    /// Deadline/budget exhausted. The fixture must set `simulated_ms` above
    /// the request `budget.max_ms` so the script stays coherent.
    Timeout {
        /// Pretend elapsed milliseconds for the attempt.
        simulated_ms: u64,
    },
    /// Output fails proposal validation.
    Malformed,
    /// Job cancelled mid-call.
    Cancelled,
    /// Base revision moved under the request.
    StaleContext,
    /// Capability the adapter does not offer.
    UnsupportedCapability,
    /// Spend cap plus unknown cost, or quota/auth refusal.
    QuotaAuth,
    /// In-flight load at or above `max_concurrency`. Surfaces as
    /// [`AiError::Unavailable`]: the contract has no queue variant and a busy adapter
    /// is unreachable-but-retryable.
    QueueOverflow,
    /// Event arriving after cancel: discarded, never a mutation.
    LateOutput,
    /// Remote adapter under a non-remote privacy mode. Surfaces as
    /// [`AiError::UnsupportedCapability`] with no remote dispatch.
    PrivacyDowngrade,
}

/// Adapter outcome envelope shared by every backend so one suite runs
/// unchanged against all. Failures carry the terminal [`AiEvent`] the adapter emitted;
/// late output carries none because it was dropped before dispatch.
#[derive(Clone, PartialEq, Debug)]
pub struct MockOutcome {
    /// Terminal result for the request, or the recorded first result when
    /// `duplicate` is set.
    pub result: Result<AiResult, AiError>,
    /// Terminal events emitted by this call. Empty for duplicates (nothing
    /// new happened) and for late output (the event was discarded).
    pub events: Vec<AiEvent>,
    /// True only when a remote endpoint completed an exchange for this call.
    /// Always false for the mock and for local loopback calls; privacy
    /// refusals must observe `false` here.
    pub dispatched_remote: bool,
    /// True when this request id was already served: the recorded first
    /// result is returned and nothing runs twice.
    pub duplicate: bool,
    /// True only when this call newly produced a complete proposal. Never
    /// true for errors, duplicates or discarded late output.
    pub mutated: bool,
    /// True when a post-cancel event was dropped before dispatch.
    pub discarded: bool,
}

/// Result of offering one event to a job after the fact. Delivery never
/// mutates: only the command service applies proposals.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LateOutcome {
    /// True when the event was dropped (job cancelled, done or unknown).
    pub discarded: bool,
    /// Always false: offering output is never a world write.
    pub mutated: bool,
}

/// Shared guard, job and script state behind every adapter. State (seen ids,
/// jobs, cancellation, revision, load, counters) is explicit and
/// inspectable; the same script plus the same request sequence always yields
/// the same outcome sequence.
#[derive(Clone, Debug)]
pub(crate) struct ContractCore {
    script: Vec<MockStep>,
    cursor: usize,
    seen: HashMap<RequestId, MockOutcome>,
    cancelled: HashSet<RequestId>,
    jobs: HashMap<RequestId, JobState>,
    current_revision: Option<u64>,
    in_flight: u32,
    mutations: u64,
}

impl ContractCore {
    /// Build shared state that plays `script` in order. Calls past the end
    /// of the script succeed, so success-anchored tests may pass an empty
    /// script.
    pub(crate) fn new(script: Vec<MockStep>) -> Self {
        Self {
            script,
            cursor: 0,
            seen: HashMap::new(),
            cancelled: HashSet::new(),
            jobs: HashMap::new(),
            current_revision: None,
            in_flight: 0,
            mutations: 0,
        }
    }

    /// Recorded first result for an already-served id, with duplicate flags
    /// applied. Returns `None` for a fresh id.
    pub(crate) fn replay(&self, id: RequestId) -> Option<MockOutcome> {
        self.seen.get(&id).map(|recorded| {
            let mut replay = recorded.clone();
            replay.duplicate = true;
            replay.mutated = false;
            replay.events = Vec::new();
            replay
        })
    }

    /// Contract guards in suite order: policy validation, armed cancel, live
    /// revision, queue load, vision need, then the unknown-cost spend rule.
    /// Every adapter runs these before its script so refusals never dispatch.
    pub(crate) fn precheck(
        &self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> Result<(), (AiError, JobState)> {
        // Privacy downgrade and every other contract refusal surface here:
        // a remote adapter under a non-remote request or session privacy is
        // `UnsupportedCapability`, and no adapter dispatches on refusal.
        if let Err(error) = validate_request(request, capabilities, policy) {
            return Err((error, JobState::Done));
        }
        if self.cancelled.contains(&request.id) {
            return Err((AiError::Cancelled, JobState::Cancelled(CancelReason::User)));
        }
        if let Some(current) = self.current_revision {
            if request.base_revision != current {
                return Err((AiError::StaleContext, JobState::Done));
            }
        }
        if self.in_flight >= policy.max_concurrency.max(1) {
            return Err((AiError::Unavailable, JobState::Done));
        }
        if request.model.model.starts_with(VISION_MODEL_PREFIX) && !capabilities.vision {
            return Err((AiError::UnsupportedCapability, JobState::Done));
        }
        // A hard session spend cap plus unknown request cost is a refusal,
        // never a guess. The contract enforces this for remote adapters; the shared
        // core honors it for every adapter because unknown cost is unbounded.
        if policy.spend_limit.is_some() && request.budget.spend_limit.is_none() {
            return Err((AiError::QuotaAuth, JobState::Done));
        }
        Ok(())
    }

    pub(crate) fn record(&mut self, id: RequestId, outcome: MockOutcome) -> MockOutcome {
        self.seen.insert(id, outcome.clone());
        outcome
    }

    pub(crate) fn fail(
        &mut self,
        request: &AiRequest,
        error: AiError,
        done_state: JobState,
        dispatched_remote: bool,
    ) -> MockOutcome {
        let event = AiEvent {
            request: request.id,
            seq: 0,
            kind: EventKind::Failed(error),
        };
        self.jobs.insert(request.id, done_state);
        self.record(
            request.id,
            MockOutcome {
                result: Err(error),
                events: vec![event],
                dispatched_remote,
                duplicate: false,
                mutated: false,
                discarded: false,
            },
        )
    }

    pub(crate) fn next_step(&mut self) -> MockStep {
        if self.cursor < self.script.len() {
            let step = self.script[self.cursor].clone();
            self.cursor += 1;
            step
        } else {
            MockStep::Succeed
        }
    }

    /// Apply one scripted failure arm. Returns `None` for `Succeed` so each
    /// adapter runs its own success path (canned proposal or HTTP
    /// roundtrip); every other arm records its outcome without dispatching.
    pub(crate) fn scripted(&mut self, request: &AiRequest, step: MockStep) -> Option<MockOutcome> {
        match step {
            MockStep::Succeed => None,
            MockStep::Timeout { .. } => {
                Some(self.fail(request, AiError::Timeout, JobState::Done, false))
            }
            MockStep::Malformed => {
                let bad = Proposal {
                    request: request.id,
                    base_revision: request.base_revision,
                    actions: vec![ProposalAction::Noop; MAX_ACTIONS + 1],
                    object_versions: Vec::new(),
                    diagnostics: Vec::new(),
                };
                // The scripted bad proposal must actually be invalid, else
                // the fixture lies about what it covers.
                debug_assert!(validate_proposal(&bad).is_err());
                let _ = bad;
                Some(self.fail(request, AiError::MalformedOutput, JobState::Done, false))
            }
            MockStep::Cancelled => {
                self.cancelled.insert(request.id);
                Some(self.fail(
                    request,
                    AiError::Cancelled,
                    JobState::Cancelled(CancelReason::User),
                    false,
                ))
            }
            MockStep::StaleContext => {
                Some(self.fail(request, AiError::StaleContext, JobState::Done, false))
            }
            MockStep::UnsupportedCapability => Some(self.fail(
                request,
                AiError::UnsupportedCapability,
                JobState::Done,
                false,
            )),
            MockStep::QuotaAuth => {
                Some(self.fail(request, AiError::QuotaAuth, JobState::Done, false))
            }
            MockStep::QueueOverflow => {
                Some(self.fail(request, AiError::Unavailable, JobState::Done, false))
            }
            MockStep::LateOutput => {
                let outcome = MockOutcome {
                    result: Err(AiError::Cancelled),
                    events: Vec::new(),
                    dispatched_remote: false,
                    duplicate: false,
                    mutated: false,
                    discarded: true,
                };
                self.jobs
                    .insert(request.id, JobState::Cancelled(CancelReason::User));
                Some(self.record(request.id, outcome))
            }
            MockStep::PrivacyDowngrade => Some(self.fail(
                request,
                AiError::UnsupportedCapability,
                JobState::Done,
                false,
            )),
        }
    }

    /// Complete with a validated proposal. A proposal that fails validation
    /// is reported as [`AiError::MalformedOutput`], never applied.
    pub(crate) fn succeed_with(
        &mut self,
        request: &AiRequest,
        proposal: Proposal,
        usage: Usage,
        dispatched_remote: bool,
    ) -> MockOutcome {
        // Locally built and therefore valid by construction; a validation
        // miss here is an adapter bug, reported as malformed, never applied.
        if validate_proposal(&proposal).is_err() {
            return self.fail(request, AiError::MalformedOutput, JobState::Done, false);
        }
        let result = AiResult {
            request: request.id,
            reason: FinishReason::Complete,
            proposal: Some(proposal),
            usage,
        };
        let event = AiEvent {
            request: request.id,
            seq: 0,
            kind: EventKind::Completed(result.clone()),
        };
        self.mutations += 1;
        self.jobs.insert(request.id, JobState::Done);
        self.record(
            request.id,
            MockOutcome {
                result: Ok(result),
                events: vec![event],
                dispatched_remote,
                duplicate: false,
                mutated: true,
                discarded: false,
            },
        )
    }

    /// Pin the live document/world revision. Calls whose `base_revision`
    /// differs fail with [`AiError::StaleContext`] before the script runs.
    pub(crate) fn set_revision(&mut self, revision: u64) {
        self.current_revision = Some(revision);
    }

    /// Stop enforcing revision freshness.
    pub(crate) fn clear_revision_check(&mut self) {
        self.current_revision = None;
    }

    /// Pretend `count` requests are already in flight. A call made while
    /// `count >= policy.max_concurrency` fails as queue overflow.
    pub(crate) fn set_in_flight(&mut self, count: u32) {
        self.in_flight = count;
    }

    /// Track a job as running so cancel/late-output tests have a live job.
    pub(crate) fn begin_job(&mut self, id: RequestId) {
        self.jobs.entry(id).or_insert(JobState::Running);
    }

    /// Cancel a job with a reason. The next `call` for the id refuses with
    /// [`AiError::Cancelled`]; later `deliver_late` reports discarded.
    pub(crate) fn cancel_job(&mut self, id: RequestId, reason: CancelReason) {
        self.cancelled.insert(id);
        self.jobs.insert(
            id,
            self.jobs
                .get(&id)
                .copied()
                .unwrap_or(JobState::Running)
                .cancel(reason),
        );
    }

    /// Cancel as the calling user. Shorthand for `cancel_job(id, User)`.
    pub(crate) fn cancel(&mut self, id: RequestId) {
        self.cancel_job(id, CancelReason::User);
    }

    /// Offer one event to a job after the fact. Only a `Running` job accepts
    /// output; cancelled, done or unknown jobs discard it without mutation.
    pub(crate) fn deliver_late(&self, id: RequestId, _event: &AiEvent) -> LateOutcome {
        match self.jobs.get(&id) {
            Some(JobState::Running) => LateOutcome {
                discarded: false,
                mutated: false,
            },
            _ => LateOutcome {
                discarded: true,
                mutated: false,
            },
        }
    }

    /// Script steps not yet consumed.
    pub(crate) fn script_remaining(&self) -> usize {
        self.script.len().saturating_sub(self.cursor)
    }

    /// Calls that newly produced a complete proposal. Duplicates, errors and
    /// discarded late output never increment this.
    pub(crate) fn mutations(&self) -> u64 {
        self.mutations
    }
}

/// Validate a complete proposal the way the consume path must before apply:
/// closed action grammar with bounded labels, coordinates and diagnostics.
/// Anything else is [`AiError::MalformedOutput`]. Pure and allocation-free:
/// it only inspects slices, so ledger counters stay outside the tick.
pub fn validate_proposal(proposal: &Proposal) -> Result<(), AiError> {
    if proposal.actions.len() > MAX_ACTIONS {
        return Err(AiError::MalformedOutput);
    }
    for action in &proposal.actions {
        match action {
            ProposalAction::SetPosition { xyz_cm, .. } => {
                for axis in xyz_cm {
                    if i64::from(*axis).abs() > i64::from(MAX_COORD_CM) {
                        return Err(AiError::MalformedOutput);
                    }
                }
            }
            ProposalAction::SetLabel { label, .. } => {
                if !is_bounded_label(label) {
                    return Err(AiError::MalformedOutput);
                }
            }
            ProposalAction::Noop => {}
        }
    }
    if proposal.object_versions.len() > MAX_ACTIONS {
        return Err(AiError::MalformedOutput);
    }
    if proposal.diagnostics.len() > MAX_ACTIONS {
        return Err(AiError::MalformedOutput);
    }
    for line in &proposal.diagnostics {
        if line.len() > MAX_TEXT_LEN || has_control(line) {
            return Err(AiError::MalformedOutput);
        }
    }
    Ok(())
}

/// True when a label meets the closed grammar: `1..=128` bytes and chars,
/// no control characters.
fn is_bounded_label(label: &str) -> bool {
    if label.is_empty() || label.len() > MAX_LABEL_LEN {
        return false;
    }
    if label.chars().count() > MAX_LABEL_LEN {
        return false;
    }
    !has_control(label)
}

/// True when the text holds a control character. Labels and diagnostics are
/// single lines: controls would smuggle framing through the receipt.
fn has_control(text: &str) -> bool {
    text.chars().any(|c| c.is_control())
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{Proposal, ProposalAction, RequestId};
    use super::*;

    fn proposal(
        actions: Vec<ProposalAction>,
        diagnostics: Vec<String>,
        object_versions: Vec<(u64, u64)>,
    ) -> Proposal {
        Proposal {
            request: RequestId(7),
            base_revision: 3,
            actions,
            object_versions,
            diagnostics,
        }
    }

    #[test]
    fn grammar_accepts_closed_shapes() {
        let valid = proposal(
            vec![
                ProposalAction::Noop,
                ProposalAction::SetPosition {
                    object: 2,
                    xyz_cm: [10, -20, 30],
                },
                ProposalAction::SetLabel {
                    object: 3,
                    label: String::from("gate"),
                },
            ],
            vec![String::from("mock:ok")],
            Vec::new(),
        );
        assert_eq!(validate_proposal(&valid), Ok(()));
    }

    #[test]
    fn grammar_rejects_bad_labels_coords_versions_and_diagnostics() {
        let empty_label = proposal(
            vec![ProposalAction::SetLabel {
                object: 1,
                label: String::new(),
            }],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            validate_proposal(&empty_label),
            Err(AiError::MalformedOutput)
        );
        let long_label = proposal(
            vec![ProposalAction::SetLabel {
                object: 1,
                label: "x".repeat(MAX_LABEL_LEN + 1),
            }],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            validate_proposal(&long_label),
            Err(AiError::MalformedOutput)
        );
        let control_label = proposal(
            vec![ProposalAction::SetLabel {
                object: 1,
                label: String::from("gate\nbreak"),
            }],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            validate_proposal(&control_label),
            Err(AiError::MalformedOutput)
        );
        let far_coord = proposal(
            vec![ProposalAction::SetPosition {
                object: 1,
                xyz_cm: [MAX_COORD_CM + 1, 0, 0],
            }],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(validate_proposal(&far_coord), Err(AiError::MalformedOutput));
        let many_versions = proposal(
            vec![ProposalAction::Noop],
            Vec::new(),
            vec![(1u64, 1u64); MAX_ACTIONS + 1],
        );
        assert_eq!(
            validate_proposal(&many_versions),
            Err(AiError::MalformedOutput)
        );
        let control_diagnostic = proposal(
            vec![ProposalAction::Noop],
            vec![String::from("line\nbreak")],
            Vec::new(),
        );
        assert_eq!(
            validate_proposal(&control_diagnostic),
            Err(AiError::MalformedOutput)
        );
        let long_diagnostic = proposal(
            vec![ProposalAction::Noop],
            vec!["y".repeat(MAX_TEXT_LEN + 1)],
            Vec::new(),
        );
        assert_eq!(
            validate_proposal(&long_diagnostic),
            Err(AiError::MalformedOutput)
        );
    }
}
