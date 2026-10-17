use super::*;

/// Loop states in contract order. EA1a emits observe and propose; check,
/// approve, apply, verify and report exist for EA1b-EA1d to fill in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantStep {
    /// Read-only scene brief captured.
    Observe,
    /// Scripted mock-provider proposal recorded.
    Propose,
    /// Schema, range and revision checks (EA1b).
    Check,
    /// Capability approval minted (EA1d).
    Approve,
    /// Atomic apply through the command gate (EA1d).
    Apply,
    /// Fork play-test and verifier evidence (EA1c).
    Verify,
    /// Human-readable outcome (EA1e).
    Report,
}

impl AssistantStep {
    /// Stable transcript tag, one lowercase word.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Propose => "propose",
            Self::Check => "check",
            Self::Approve => "approve",
            Self::Apply => "apply",
            Self::Verify => "verify",
            Self::Report => "report",
        }
    }
}

/// What stopped the assistant loop. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantError {
    /// Budget outside 1..=`ASSISTANT_BUDGET_MAX`.
    BadBudget {
        /// Requested budget.
        budget: u32,
    },
    /// An entry would push usage past the budget; nothing was recorded.
    OverBudget {
        /// Tokens the entry needs.
        needed: u32,
        /// Budget of this loop.
        budget: u32,
    },
    /// Prompt longer than `ASSISTANT_PROMPT_MAX_CHARS`.
    PromptTooLong {
        /// Prompt length in chars.
        len: usize,
    },
    /// Propose before any observe; the loop must observe first.
    OutOfOrder,
    /// The `scene.brief` read failed; carries the tool code and message.
    InspectFailed {
        /// Machine code from the brief tool (`bad_budget`, ...).
        code: &'static str,
        /// Human detail with the legal values embedded.
        message: String,
    },
    /// Transcript already holds `ASSISTANT_TRANSCRIPT_MAX` entries.
    TranscriptFull,
    /// A custom proposal breaks the shape contract (empty or oversize
    /// summary, no wants, too many wants, an empty or oversize want).
    BadProposal {
        /// What breaks the shape, with the legal values embedded.
        reason: String,
    },
    /// A write was requested; EA1a has no write path by construction.
    WritesRefused {
        /// Requested write op.
        op: String,
    },
}

impl AssistantError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadBudget { .. } => "bad_budget",
            Self::OverBudget { .. } => "over_budget",
            Self::PromptTooLong { .. } => "prompt_too_long",
            Self::OutOfOrder => "out_of_order",
            Self::InspectFailed { code, .. } => code,
            Self::TranscriptFull => "transcript_full",
            Self::BadProposal { .. } => "bad_proposal",
            Self::WritesRefused { .. } => "writes_refused",
        }
    }
}

impl std::fmt::Display for AssistantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadBudget { budget } => write!(
                f,
                "bad_budget: budget {budget} is outside 1..={ASSISTANT_BUDGET_MAX} \
                 (default {ASSISTANT_BUDGET_DEFAULT})"
            ),
            Self::OverBudget { needed, budget } => write!(
                f,
                "over_budget: entry needs {needed} tokens over a {budget} budget; \
                 raise the budget (at most {ASSISTANT_BUDGET_MAX}) or shrink the scene"
            ),
            Self::PromptTooLong { len } => write!(
                f,
                "prompt_too_long: prompt holds {len} chars; \
                 at most {ASSISTANT_PROMPT_MAX_CHARS}"
            ),
            Self::OutOfOrder => write!(
                f,
                "out_of_order: propose needs an observe first; \
                 call inspect, then propose_mock"
            ),
            Self::InspectFailed { code, message } => {
                write!(f, "{code}: {message}")
            }
            Self::TranscriptFull => write!(
                f,
                "transcript_full: transcript holds {ASSISTANT_TRANSCRIPT_MAX} entries; \
                 start a new loop"
            ),
            Self::BadProposal { reason } => write!(f, "bad_proposal: {reason}"),
            Self::WritesRefused { op } => write!(
                f,
                "writes_refused: `{op}` writes, and EA1a has no write path; \
                 the loop holds only &McpServer"
            ),
        }
    }
}

impl std::error::Error for AssistantError {}

/// What stopped a proposal check. Every message names the legal values; the
/// conflict set rides attached on [`ProposeError::Conflict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeError {
    /// Check before any proposal; the loop must observe then propose first.
    NoProposal,
    /// The document moved since the loop observed it; nothing was staged.
    StaleRevision {
        /// Current document revision.
        expected: u64,
        /// Revision the proposal was prepared against.
        got: u64,
    },
    /// The proposal cannot land: minimal conflict set attached.
    Conflict(ProposalConflict),
    /// The proposal never reached the solver or the preview: bad id, bad
    /// want grammar, a parented object, a lock failure or a batch refusal.
    PreviewFailed {
        /// Machine code (`bad_id`, `bad_want`, `not_a_root`, ...).
        code: &'static str,
        /// Human detail with the legal values embedded.
        message: String,
    },
    /// The check row would push usage past the budget; nothing was recorded.
    OverBudget {
        /// Tokens the check row needs.
        needed: u32,
        /// Budget of this loop.
        budget: u32,
    },
    /// Transcript already holds `ASSISTANT_TRANSCRIPT_MAX` entries.
    TranscriptFull,
}

impl ProposeError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoProposal => "no_proposal",
            Self::StaleRevision { .. } => "stale_revision",
            Self::Conflict(_) => "conflict",
            Self::PreviewFailed { code, .. } => code,
            Self::OverBudget { .. } => "over_budget",
            Self::TranscriptFull => "transcript_full",
        }
    }
}

impl std::fmt::Display for ProposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProposal => write!(
                f,
                "no_proposal: check needs a proposal first; \
                 call inspect, then propose_mock or propose_custom"
            ),
            Self::StaleRevision { expected, got } => write!(
                f,
                "stale_revision: document is at revision {expected}, proposal expects {got}; \
                 re-read scene.brief, then propose again"
            ),
            Self::Conflict(conflict) => write!(
                f,
                "conflict: {}; conflicts: [{}]",
                conflict.reason,
                conflict.conflicts.join(", ")
            ),
            Self::PreviewFailed { code, message } => {
                write!(f, "{code}: {message}")
            }
            Self::OverBudget { needed, budget } => write!(
                f,
                "over_budget: check row needs {needed} tokens over a {budget} budget; \
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

impl std::error::Error for ProposeError {}
