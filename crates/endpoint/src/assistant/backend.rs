use super::*;

/// EA1e multi-backend loop: the EA1a-EA1d stages with proposals sourced from
/// mock, local and remote adapters on recorded fixtures.
///
/// Consumer: the EV1 harness and external MCP clients. Only the proposal
/// source changes per backend: the same summary plus wants records the same
/// propose row, checks to the same preview receipt, fork-tests to the same
/// metrics and applies to the same revision on every backend. Per-backend
/// latency (ms) and cost (minor units, `None` is Unknown) ride alongside in
/// [`BackendReport`] and in the fork verifier detail of [`ApplyReceipt`];
/// they never enter transcript, preview or fork determinism. Backend
/// failures are provider-taxonomy [`AiError`] values; timeouts and refusals
/// file an AH5 fallback notice as a `report` row via
/// [`AssistantLoop::record_fallback`]. Live adapters stay behind the
/// provider crate's opt-in env tests; this loop only ever reads recorded
/// [`BackendFixture`] data, never the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Deterministic scripted backend (CI, tests, offline default).
    Mock,
    /// Loopback sidecar backend (local-only fixtures).
    Local,
    /// Configured-host backend (remote-allowed fixtures).
    Remote,
}

impl BackendKind {
    /// Stable lowercase label for receipts and verifier details.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    /// Provider id stamped on PR1 receipts for this backend.
    pub fn provider(self) -> &'static str {
        match self {
            Self::Mock => MOCK_PROVIDER,
            Self::Local => LOCAL_PROVIDER,
            Self::Remote => REMOTE_PROVIDER,
        }
    }

    /// Model id stamped on PR1 receipts for this backend.
    pub fn model(self) -> &'static str {
        match self {
            Self::Mock => MOCK_MODEL,
            Self::Local => LOCAL_MODEL,
            Self::Remote => REMOTE_MODEL,
        }
    }

    /// Provider-taxonomy adapter this backend's fixtures stand in for.
    pub fn adapter_kind(self) -> AdapterKind {
        match self {
            Self::Mock => AdapterKind::Mock,
            Self::Local => AdapterKind::Local,
            Self::Remote => AdapterKind::Remote,
        }
    }

    /// Negotiated capabilities the fixtures stand in for: text-parsed
    /// proposals on every backend, never native structured output.
    pub fn capabilities(self) -> ProviderCapabilities {
        match self {
            Self::Mock => ProviderCapabilities::mock(),
            Self::Local => ProviderCapabilities::local_http(),
            Self::Remote => ProviderCapabilities::remote_http(),
        }
    }
}

/// Render a recorded cost: minor units, or `unknown` when the backend does
/// not report it. Unknown stays unknown, never zero-claimed.
pub(super) fn cost_text(cost: Option<u64>) -> String {
    match cost {
        Some(units) => units.to_string(),
        None => "unknown".to_owned(),
    }
}

/// One recorded backend answer: the proposal content plus the serving
/// figures the harness compares. Built by hand or from a recorded adapter
/// exchange; the loop never dials. Shape bounds mirror `propose_custom`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendFixture {
    /// Which backend this recording stands in for.
    pub backend: BackendKind,
    /// Short intent summary, `1..=ASSISTANT_SUMMARY_MAX_CHARS` chars.
    pub summary: String,
    /// Relation wants in the `scene.place` grammar, 1..=16 entries.
    pub wants: Vec<String>,
    /// Serving latency in milliseconds.
    pub latency_ms: u64,
    /// Serving cost in minor cost units; `None` is Unknown.
    pub cost: Option<u64>,
    /// Token usage; unknown stays unknown.
    pub usage: Usage,
}

impl BackendFixture {
    /// Record one backend answer. Shape failures reuse the proposal
    /// contract; latency, cost and usage are recorded verbatim.
    pub fn new(
        backend: BackendKind,
        summary: &str,
        wants: Vec<String>,
        latency_ms: u64,
        cost: Option<u64>,
        usage: Usage,
    ) -> Result<Self, AssistantError> {
        let summary_len = summary.chars().count();
        if summary.is_empty() || summary_len > ASSISTANT_SUMMARY_MAX_CHARS {
            return Err(AssistantError::BadProposal {
                reason: format!(
                    "summary holds {summary_len} chars; \
                     submit 1..={ASSISTANT_SUMMARY_MAX_CHARS} chars"
                ),
            });
        }
        if wants.is_empty() || wants.len() > ASSISTANT_WANTS_MAX {
            return Err(AssistantError::BadProposal {
                reason: format!(
                    "wants holds {} entries; submit 1..={ASSISTANT_WANTS_MAX} wants",
                    wants.len()
                ),
            });
        }
        for (index, want) in wants.iter().enumerate() {
            let len = want.chars().count();
            if want.is_empty() || len > ASSISTANT_WANT_MAX_CHARS {
                return Err(AssistantError::BadProposal {
                    reason: format!(
                        "wants[{index}] holds {len} chars; \
                         submit 1..={ASSISTANT_WANT_MAX_CHARS} chars per want"
                    ),
                });
            }
        }
        Ok(Self {
            backend,
            summary: summary.to_owned(),
            wants,
            latency_ms,
            cost,
            usage,
        })
    }
}

/// Per-backend serving figures for the EV1 harness: latency in ms, cost in
/// minor units (`None` is Unknown), token usage, and the AH5 notice when the
/// backend failed over to the scripted path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendReport {
    /// Which backend served (or failed over).
    pub backend: BackendKind,
    /// Serving latency in milliseconds.
    pub latency_ms: u64,
    /// Serving cost in minor cost units; `None` is Unknown.
    pub cost: Option<u64>,
    /// Token usage; unknown stays unknown.
    pub usage: Usage,
    /// AH5 fallback notice, set only after [`AssistantLoop::record_fallback`].
    pub fallback: Option<FallbackNotice>,
}

impl BackendReport {
    /// Cost units, or `unknown` when the backend does not report cost.
    pub fn cost_text(&self) -> String {
        cost_text(self.cost)
    }
}

/// What stopped a backend fallback report. The provider-taxonomy error rides
/// inside [`BackendError::Propagate`]; budget and transcript refusals record
/// nothing, like every other loop stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// Cancelled and stale requests never fall back: the caller cancels or
    /// rebuilds against a fresh revision. Carries the provider error back.
    Propagate(AiError),
    /// The report row would push usage past the budget; nothing was recorded.
    OverBudget {
        /// Tokens the report row needs.
        needed: u32,
        /// Budget of this loop.
        budget: u32,
    },
    /// Transcript already holds `ASSISTANT_TRANSCRIPT_MAX` entries.
    TranscriptFull,
}

impl BackendError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Propagate(_) => "propagate",
            Self::OverBudget { .. } => "over_budget",
            Self::TranscriptFull => "transcript_full",
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Propagate(error) => write!(
                f,
                "propagate: provider error {error:?} takes no fallback; \
                 cancel the request or rebuild against a fresh revision"
            ),
            Self::OverBudget { needed, budget } => write!(
                f,
                "over_budget: report row needs {needed} tokens over a {budget} budget; \
                 raise the budget (at most {ASSISTANT_BUDGET_MAX}) or start a new loop"
            ),
            Self::TranscriptFull => write!(
                f,
                "transcript_full: transcript holds {ASSISTANT_TRANSCRIPT_MAX} entries; \
                 start a new loop"
            ),
        }
    }
}

impl std::error::Error for BackendError {}

impl AssistantLoop {
    /// Backend that sourced the current proposal.
    pub fn backend(&self) -> BackendKind {
        self.backend
    }

    /// Serving figures for the current proposal, for the EV1 harness.
    pub fn backend_report(&self) -> BackendReport {
        BackendReport {
            backend: self.backend,
            latency_ms: self.backend_latency_ms,
            cost: self.backend_cost,
            usage: self.backend_usage,
            fallback: self.fallback_notice.clone(),
        }
    }

    /// Latest AH5 fallback notice, if [`AssistantLoop::record_fallback`] ran.
    pub fn fallback_notice(&self) -> Option<&FallbackNotice> {
        self.fallback_notice.as_ref()
    }

    /// Record the backend behind a mock or custom proposal: the serving
    /// figures plus the usage the fixture carried. A fresh proposal clears
    /// any earlier fallback notice.
    pub(super) fn note_backend(
        &mut self,
        backend: BackendKind,
        latency_ms: u64,
        cost: Option<u64>,
        usage: Usage,
    ) {
        self.backend = backend;
        self.backend_latency_ms = latency_ms;
        self.backend_cost = cost;
        self.backend_usage = usage;
        self.fallback_notice = None;
    }

    /// Propose from a recorded backend fixture: the same content records the
    /// same propose row as `propose_custom`, so transcripts, previews, fork
    /// runs and applies compare across backends while the backend label plus
    /// latency, cost and usage ride alongside for the harness. Records one
    /// propose row; a fresh proposal voids any earlier check and clears any
    /// earlier fallback notice.
    pub fn propose_backend(
        &mut self,
        fixture: &BackendFixture,
    ) -> Result<Proposal, AssistantError> {
        if self.observed_revision.is_none() {
            return Err(AssistantError::OutOfOrder);
        }
        let summary_len = fixture.summary.chars().count();
        if fixture.summary.is_empty() || summary_len > ASSISTANT_SUMMARY_MAX_CHARS {
            return Err(AssistantError::BadProposal {
                reason: format!(
                    "summary holds {summary_len} chars; \
                     submit 1..={ASSISTANT_SUMMARY_MAX_CHARS} chars"
                ),
            });
        }
        if fixture.wants.is_empty() || fixture.wants.len() > ASSISTANT_WANTS_MAX {
            return Err(AssistantError::BadProposal {
                reason: format!(
                    "wants holds {} entries; submit 1..={ASSISTANT_WANTS_MAX} wants",
                    fixture.wants.len()
                ),
            });
        }
        for (index, want) in fixture.wants.iter().enumerate() {
            let len = want.chars().count();
            if want.is_empty() || len > ASSISTANT_WANT_MAX_CHARS {
                return Err(AssistantError::BadProposal {
                    reason: format!(
                        "wants[{index}] holds {len} chars; \
                         submit 1..={ASSISTANT_WANT_MAX_CHARS} chars per want"
                    ),
                });
            }
        }
        let text = format!("{} | {}", fixture.summary, fixture.wants.join(" + "));
        let tokens = self.record(AssistantStep::Propose, text)?;
        let proposal = Proposal {
            summary: fixture.summary.clone(),
            wants: fixture.wants.clone(),
            tokens,
        };
        self.proposal = Some(proposal.clone());
        // A fresh proposal voids any earlier check: approve/apply bind the
        // latest check only.
        self.checked = None;
        self.note_backend(
            fixture.backend,
            fixture.latency_ms,
            fixture.cost,
            fixture.usage,
        );
        Ok(proposal)
    }

    /// File a provider timeout or refusal on the AH5 scripted path: records
    /// the fallback notice as one `report` row and returns it for the
    /// observe ring. Timeouts (`Timeout`, `Unavailable`) file under
    /// `provider.timeout`; refusals (`QuotaAuth`, `UnsupportedCapability`,
    /// `MalformedOutput`) file under `provider.malformed`. Cancelled and
    /// stale errors propagate untouched, recording nothing.
    pub fn record_fallback(
        &mut self,
        error: AiError,
        budget_ms: u64,
    ) -> Result<FallbackNotice, BackendError> {
        let Some(cause) = FallbackCause::from_error(error) else {
            return Err(BackendError::Propagate(error));
        };
        let notice = FallbackNotice::new(cause, error, budget_ms);
        if self.transcript.len() >= ASSISTANT_TRANSCRIPT_MAX {
            return Err(BackendError::TranscriptFull);
        }
        let needed = count_tokens(&notice.message);
        if needed > self.remaining_tokens() {
            return Err(BackendError::OverBudget {
                needed,
                budget: self.budget,
            });
        }
        self.used += needed;
        self.transcript.push(TranscriptEntry {
            step: AssistantStep::Report,
            text: notice.message.clone(),
            tokens: needed,
        });
        self.fallback_notice = Some(notice.clone());
        Ok(notice)
    }

    /// Record a human-readable outcome as one `report` row: the loop's
    /// closing step for harness summaries. Empty and oversize texts fail
    /// with the proposal shape error; refusals record nothing.
    pub fn report(&mut self, text: &str) -> Result<u32, AssistantError> {
        let len = text.chars().count();
        if text.is_empty() || len > ASSISTANT_SUMMARY_MAX_CHARS {
            return Err(AssistantError::BadProposal {
                reason: format!(
                    "report holds {len} chars; \
                     submit 1..={ASSISTANT_SUMMARY_MAX_CHARS} chars"
                ),
            });
        }
        self.record(AssistantStep::Report, text.to_owned())
    }
}
