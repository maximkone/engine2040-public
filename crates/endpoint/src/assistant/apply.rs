use super::*;

/// EA1d apply close: CAP1 approval, atomic apply, undo and the PR1 receipt.
///
/// Consumer: EA1e applies fork-passed proposals through [`AssistantLoop::apply`]
/// and reverts through [`AssistantLoop::undo`]; EV1 reads [`ApplyReceipt`].
/// Data owner stays the editor command service: the loop builds the exact
/// batch at check time and the approval binds its canonical hash, so callers
/// supply only the approval plus the fork proof, never ops. Units are
/// revisions (document clock) and tokens. Determinism scope: the approve,
/// apply and undo row texts plus the receipt strings — the same checked batch
/// plus the same proof always lands the same revision with the same receipt
/// id. Errors are typed [`ApplyError`]: stale revisions, bad approvals and
/// apply-time conflicts change neither document nor journal (a refused
/// command still spends its CAP1 budget by design, like `tx.apply`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReceipt {
    /// One line per op, in batch order, from the landed apply.
    pub effects: Vec<String>,
    /// Document revision after the apply.
    pub revision: u64,
    /// Stable undo identity: pass back to [`AssistantLoop::undo`].
    pub undo_token: u64,
    /// Canonical id of the minted provenance record.
    pub receipt_id: u64,
    /// Minted PR1 record: model source with provider/model ids, the proposal
    /// request digest, artifact digest and parent chain. No prompts,
    /// credentials or session secrets by construction.
    pub provenance: Provenance,
    /// Loop verifier evidence: the staged spatial check plus the fork
    /// balance run, both passed (a failing check never reaches apply).
    pub verifiers: Vec<VerifierResult>,
    /// `count_tokens` of the recorded apply row.
    pub tokens: u32,
}

/// Undo receipt for a reverted apply: the new revision, the undone key, what
/// the undo did and the row cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoReceipt {
    /// Document revision after the undo.
    pub revision: u64,
    /// Key of the batch that was undone.
    pub undone_key: u64,
    /// What the undo did.
    pub effects: Vec<String>,
    /// `count_tokens` of the recorded undo row.
    pub tokens: u32,
}

/// What stopped an approve, apply or undo. Every message names the legal
/// values; failures record no transcript row and (except a refused command's
/// CAP1 spend, by design) move no budget, ledger, document or journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// Apply or approve before any passing check; the loop must observe,
    /// propose and check first.
    NoCheck,
    /// The fork proof does not bind the checked receipt: tampered input, a
    /// proof cut for another proposal, or a breaching run. The reason names
    /// the fix.
    ForkMismatch {
        /// What fails to bind, with the legal values embedded.
        reason: String,
    },
    /// A fork-budget overrun quarantined the proposal; it must not apply.
    Quarantined,
    /// The document moved since the check; nothing was staged or applied.
    StaleRevision {
        /// Current document revision.
        expected: u64,
        /// Revision the batch was prepared against.
        got: u64,
    },
    /// The approval is forged, spent, expired, denied, out of scope or over
    /// budget; nothing was staged or applied.
    BadApproval {
        /// Why the approval fails, with the legal values embedded.
        reason: String,
    },
    /// The apply itself failed after the gate: an added spatial finding or a
    /// command refusal. The document and journal are unchanged.
    Conflict {
        /// Why the apply failed, with the legal values embedded.
        reason: String,
    },
    /// Undo with no applied batch on the journal.
    NothingToUndo,
    /// The undo token names another batch than the newest one.
    UndoMismatch {
        /// Newest undoable batch key.
        expected: u64,
        /// Token the caller passed.
        got: u64,
    },
    /// The server lock failed; the call ran nothing.
    Transport {
        /// Machine code from the lock gate (`lock_poisoned`).
        code: &'static str,
        /// Human detail.
        message: String,
    },
    /// The approve/apply/undo row would push usage past the budget; nothing
    /// was recorded and (for apply/undo, reserved first) nothing ran.
    OverBudget {
        /// Tokens the row needs.
        needed: u32,
        /// Budget of this loop.
        budget: u32,
    },
    /// Transcript already holds `ASSISTANT_TRANSCRIPT_MAX` entries.
    TranscriptFull,
}

impl ApplyError {
    /// Machine code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoCheck => "no_check",
            Self::ForkMismatch { .. } => "fork_mismatch",
            Self::Quarantined => "quarantined",
            Self::StaleRevision { .. } => "stale_revision",
            Self::BadApproval { .. } => "bad_approval",
            Self::Conflict { .. } => "conflict",
            Self::NothingToUndo => "nothing_to_undo",
            Self::UndoMismatch { .. } => "undo_mismatch",
            Self::Transport { code, .. } => code,
            Self::OverBudget { .. } => "over_budget",
            Self::TranscriptFull => "transcript_full",
        }
    }
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCheck => write!(
                f,
                "no_check: apply needs a checked receipt first; \
                 call inspect, propose_mock or propose_custom, check, then fork_test"
            ),
            Self::ForkMismatch { reason } => write!(f, "fork_mismatch: {reason}"),
            Self::Quarantined => write!(
                f,
                "quarantined: a fork-budget overrun quarantined this proposal; \
                 it must not proceed to apply"
            ),
            Self::StaleRevision { expected, got } => write!(
                f,
                "stale_revision: document is at revision {expected}, batch expects {got}; \
                 re-read scene.brief, then propose again"
            ),
            Self::BadApproval { reason } => write!(f, "bad_approval: {reason}"),
            Self::Conflict { reason } => write!(f, "conflict: {reason}"),
            Self::NothingToUndo => {
                write!(f, "nothing_to_undo: the journal holds no applied batches")
            }
            Self::UndoMismatch { expected, got } => write!(
                f,
                "undo_mismatch: newest undoable batch is key {expected}, not token {got}; \
                 omit undo_token to undo the newest batch"
            ),
            Self::Transport { code, message } => write!(f, "{code}: {message}"),
            Self::OverBudget { needed, budget } => write!(
                f,
                "over_budget: row needs {needed} tokens over a {budget} budget; \
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

impl std::error::Error for ApplyError {}

/// Deterministic apply-row text: landed revision, batch key, receipt id.
pub(super) fn apply_text(revision: u64, key: u64, receipt_id: u64) -> String {
    format!("apply ok rev{revision} key{key} receipt{receipt_id}")
}

/// Deterministic undo-row text: landed revision plus the undone key.
pub(super) fn undo_text(revision: u64, key: u64) -> String {
    format!("undo ok rev{revision} key{key}")
}

/// CAP1 gate failures into the loop error type. Stale revisions stay stale;
/// every other gate refusal (forged, spent, expired, denied op, out-of-scope
/// object, over-budget batch) is a bad approval; post-gate failures (added
/// spatial finding, command refusal) are apply-time conflicts.
pub(super) fn map_cap_error(error: CapError) -> ApplyError {
    match error {
        CapError::StaleRevision { expected, got } => ApplyError::StaleRevision { expected, got },
        CapError::ForgedApproval => ApplyError::BadApproval {
            reason: "approval names another session or batch (forged); \
                     approve the checked receipt, then apply with that approval"
                .to_owned(),
        },
        CapError::ReusedApproval => ApplyError::BadApproval {
            reason: "approval was already spent; \
                     re-run check for a fresh approval"
                .to_owned(),
        },
        CapError::Expired { expiry, current } => ApplyError::BadApproval {
            reason: format!(
                "approval expired at revision {expiry}, document is at {current}; \
                 re-run check for a fresh approval"
            ),
        },
        CapError::OpNotAllowed { op } => ApplyError::BadApproval {
            reason: format!(
                "capability token does not grant `{op}` ops; \
                 the assistant loop moves roots only"
            ),
        },
        CapError::OutOfScope { id } => ApplyError::BadApproval {
            reason: format!("node {id} is outside the token scope"),
        },
        CapError::OverBudget { remaining, needed } => ApplyError::BadApproval {
            reason: format!("budget exhausted: {needed} ops needed, {remaining} left"),
        },
        CapError::Spatial { check, .. } => ApplyError::Conflict {
            reason: format!(
                "staged spatial check `{check}`: proposal adds {check}, document unchanged"
            ),
        },
        CapError::Command(error) => ApplyError::Conflict {
            reason: error.to_string(),
        },
    }
}
