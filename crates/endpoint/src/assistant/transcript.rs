//! Transcript leaf: token cost plus proposal rows.
//!
//! Owner stays the editor command service; this leaf owns only the data
//! shapes the loop records. Units are tokens (chars/4 ceiling). Same text
//! always costs the same tokens on any machine. Errors live in the facade.

use super::AssistantStep;

/// Token cost of text: chars/4 rounded up, empty text costs zero.
pub fn count_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    if chars == 0 {
        return 0;
    }
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

/// One transcript row: step, exact text and its token cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    /// Which loop state produced this row.
    pub step: AssistantStep,
    /// Exact text recorded (brief text for observe, summary for propose).
    pub text: String,
    /// `count_tokens(text)` at record time.
    pub tokens: u32,
}

/// The scripted mock-provider proposal: data only, never applied here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Fixed mock summary.
    pub summary: String,
    /// Fixed mock wants (today exactly one).
    pub wants: Vec<String>,
    /// `count_tokens` of the recorded propose text.
    pub tokens: u32,
}

/// Preview receipt for a good proposal: the `tx.preview` dry-run diff plus
/// the staged spatial evidence. Owns no server handle; EA1c fork-tests this
/// receipt and EA1d applies it, so the check itself writes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewReceipt {
    /// One line per op, in batch order, from `tx.preview`.
    pub effects: Vec<String>,
    /// `base_revision + 1`; the revision an apply would publish.
    pub resulting_revision: u64,
    /// Canonical hash of the exact batch the approval would bind.
    pub action_hash: u64,
    /// Staged-delta finding lines; empty when the staged scene adds nothing.
    pub findings: Vec<String>,
    /// `count_tokens` of the recorded check text.
    pub tokens: u32,
}

/// Minimal conflict set for a bad proposal: the unmet wants plus the reason.
/// Returned instead of a receipt; the document, journal, capability and
/// ledger are untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalConflict {
    /// Source want texts that cannot all hold, in intent order.
    pub conflicts: Vec<String>,
    /// Why the proposal fails (solver or staged-gate wording).
    pub reason: String,
    /// Staged-delta finding lines behind a staged-gate refusal.
    pub findings: Vec<String>,
}
