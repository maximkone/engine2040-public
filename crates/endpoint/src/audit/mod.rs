//! Door audit: tamper-evident log over every gateway call.
//!
//! Consumer: the gateway dispatch helper outside this module chains one
//! record per call, success or refusal, so the log covers the whole tool
//! surface; the linkage lands with that helper, this module owns the writer
//! and the reader. The data owner is
//! [`DoorAudit`]. Units: dense sequence numbers, session ids, document
//! revisions, bytes for encodings, nanoseconds for the budget. Determinism:
//! little-endian canonical bytes with integer-only mixing, no wall clock and
//! no floating point; the same call sequence always seals the same head.
//! Errors: [`AuditError`]: one named variant per tamper class, plus
//! exhaustion for the append counter. Budget: at
//! most [`DOOR_AUDIT_BUDGET_NS`] nanoseconds per appended record. Memory:
//! at most [`DOOR_AUDIT_CAP`] records are retained; the oldest rotates out
//! and the anchor digest advances, so the retained window stays verifiable.

use std::collections::VecDeque;
use std::fmt;

use engine_core::hash::GOLDEN_GAMMA;
use engine_core::Fnv64;

mod verify;
pub use verify::{verify_pinned_records, verify_records, AuditVerifier, VERIFY_BUDGET_NS};

/// Agent-loop consumer: a scripted task drives the door and the log keeps the
/// args (needs the door, so it rides the `mcp` feature with it).
#[cfg(feature = "mcp")]
pub mod agent_run;

/// Nanoseconds per appended record: half a millisecond, measured in tests.
pub const DOOR_AUDIT_BUDGET_NS: u64 = 500_000;
/// Retention cap: at most this many records stay in memory. Past it the
/// oldest record rotates out and [`DoorAudit::anchor`] advances to its chain
/// digest (the pattern of the editor audit chain, with a larger window
/// because every door call appends one record).
pub const DOOR_AUDIT_CAP: usize = 1024;
/// Longest tool name retained verbatim; longer names truncate on a character
/// boundary (every gateway name is far shorter, so this never fires live).
pub const MAX_TOOL_NAME_LEN: usize = 64;
/// Longest call args retained verbatim, in bytes. A call whose args exceed
/// this is refused by name ([`AuditError::ArgsTooLarge`], code
/// `args_too_large`), never truncated: a replay from the log always carries
/// the exact args the call ran with.
pub const MAX_ARGS_BYTES: usize = 512;

/// Mixer step constant: the core SplitMix64 increment (2^64 over phi, odd).
/// An odd multiplier keeps each mixing step bijective, so distinct prefixes
/// stay distinct. The value is the core hash owner's, not a local copy.
const MIX_GAMMA: u64 = GOLDEN_GAMMA;

/// Domain-separated seeds for the four digest passes.
const MIX_SEEDS: [u64; 4] = [mix_seed(1), mix_seed(2), mix_seed(3), mix_seed(4)];

/// Seed for one digest pass: the mixer step over the pass index.
const fn mix_seed(pass: u64) -> u64 {
    MIX_GAMMA.wrapping_mul(pass)
}

/// Fold one byte into the mixer state: add, multiply by the odd constant,
/// then fold the halves so low-bit changes reach the top word.
fn mix_byte(state: u64, byte: u8) -> u64 {
    let sum = state.wrapping_add(u64::from(byte));
    let product = sum.wrapping_mul(MIX_GAMMA);
    product ^ (product >> 32)
}

/// 32-byte digest over byte parts: four mixer passes with distinct seeds.
/// Same parts, same digest, on every platform; no new crypto.
fn digest32(parts: &[&[u8]]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (slot, seed) in out.chunks_mut(8).zip(MIX_SEEDS) {
        let mut state = seed;
        for part in parts {
            for byte in part.iter() {
                state = mix_byte(state, *byte);
            }
        }
        slot.copy_from_slice(&state.to_le_bytes());
    }
    out
}

/// Call outcome sealed into each record: landed or refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    /// The call succeeded.
    Ok,
    /// The call was refused; nothing was applied.
    Refused,
}

impl AuditOutcome {
    /// Canonical byte for the entry encoding.
    fn as_u8(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Refused => 1,
        }
    }
}

/// What rejected a verification, one named variant per tamper class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditError {
    /// A record's entry or chain digest does not recompute at this seq
    /// (edited content or a spliced foreign record).
    ChainBroken {
        /// Sequence number of the first record that fails.
        seq: u64,
    },
    /// A record's `prev` or `seq` does not continue the chain at this seq
    /// (dropped or reordered records).
    OrderBroken {
        /// Sequence number where continuity breaks.
        seq: u64,
    },
    /// The head does not match the externally pinned head digest
    /// (a re-sealed suffix or a foreign log).
    HeadMismatch,
    /// The length does not match the externally pinned length
    /// (a truncated or extended tail).
    LengthMismatch {
        /// Length the holder pinned.
        expected: u64,
        /// Length presented.
        found: u64,
    },
    /// The sequence counter is exhausted, so no further record can chain
    /// (`u64::MAX` appends; unreachable in practice, but typed so the
    /// append path never panics).
    SequenceExhausted,
    /// The call's args exceed [`MAX_ARGS_BYTES`], so the call is refused
    /// before anything runs: the log never grows past its args bound and no
    /// args are ever truncated into a replay that would differ from the run.
    ArgsTooLarge {
        /// Byte length of the args offered.
        len: usize,
        /// The bound that refused them.
        cap: usize,
    },
}

impl AuditError {
    /// Stable machine name, so a caller can branch on the refusal instead of
    /// matching its prose.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ChainBroken { .. } => "chain_broken",
            Self::OrderBroken { .. } => "order_broken",
            Self::HeadMismatch => "head_mismatch",
            Self::LengthMismatch { .. } => "length_mismatch",
            Self::SequenceExhausted => "sequence_exhausted",
            Self::ArgsTooLarge { .. } => "args_too_large",
        }
    }
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChainBroken { seq } => {
                write!(f, "door audit chain broken at seq {seq}")
            }
            Self::OrderBroken { seq } => {
                write!(f, "door audit chain order broken at seq {seq}")
            }
            Self::HeadMismatch => f.write_str("door audit head does not match the pinned digest"),
            Self::LengthMismatch { expected, found } => {
                write!(
                    f,
                    "door audit length mismatch: pinned {expected}, found {found}"
                )
            }
            Self::SequenceExhausted => f.write_str("door audit sequence exhausted"),
            Self::ArgsTooLarge { len, cap } => write!(
                f,
                "door audit args too large: {len} bytes over the {cap}-byte bound; \
                 shrink the call, the log never truncates args"
            ),
        }
    }
}

impl std::error::Error for AuditError {}

/// One sealed gateway call: dense sequence, session bind, tool, revision,
/// outcome and detail plus the `prev` link and the chain digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRecord {
    /// Dense sequence number (1-based).
    pub seq: u64,
    /// Session that made the call; splices across sessions break the seal.
    pub session: u64,
    /// Tool name, truncated to [`MAX_TOOL_NAME_LEN`].
    pub tool: String,
    /// Document revision observed after the call.
    pub revision: u64,
    /// Whether the call landed or was refused.
    pub outcome: AuditOutcome,
    /// Call-specific witness (counts, keys, hashes, error-code hash).
    pub detail: u64,
    /// Previous chain digest (zeros for the first record).
    pub prev: [u8; 32],
    /// Chain digest: `digest(prev || entry || seq)`.
    pub chain: [u8; 32],
}

/// One agent-loop call: the chain record plus the args that drove it, and the
/// document revision on both sides of the call. Replaying [`Self::args`] in
/// order on a fresh door reproduces the same writes at the same revisions;
/// [`Self::args_digest`] is also the record's `detail` word, so a changed args
/// string breaks the chain like any other edited field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopCall {
    /// Sealed chain record for this call.
    pub record: CallRecord,
    /// Args the call ran with, verbatim, at most [`MAX_ARGS_BYTES`].
    pub args: String,
    /// `Fnv64` of `args`; equals `record.detail`.
    pub args_digest: u64,
    /// Document revision before the call.
    pub revision_before: u64,
    /// Document revision after the call.
    pub revision_after: u64,
}

/// `Fnv64` over the args bytes: the one digest both the chain detail word and
/// the replay check use.
#[must_use]
pub fn args_digest(args: &str) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(args.as_bytes());
    hasher.finish()
}

/// Refuse args over [`MAX_ARGS_BYTES`] before anything runs, so the bound has
/// one owner for both the pre-run check and the append.
/// Errors: [`AuditError::ArgsTooLarge`] with the real length and the cap.
pub fn check_args(args: &str) -> Result<(), AuditError> {
    if args.len() > MAX_ARGS_BYTES {
        return Err(AuditError::ArgsTooLarge {
            len: args.len(),
            cap: MAX_ARGS_BYTES,
        });
    }
    Ok(())
}

/// Canonical entry bytes for one record (little-endian integers,
/// length-prefixed tool text: same call, same bytes).
fn entry_bytes(
    session: u64,
    tool: &str,
    revision: u64,
    outcome: AuditOutcome,
    detail: u64,
    seq: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(41 + tool.len());
    out.extend_from_slice(&session.to_le_bytes());
    let tool_len = u64::try_from(tool.len()).unwrap_or(u64::MAX);
    out.extend_from_slice(&tool_len.to_le_bytes());
    out.extend_from_slice(tool.as_bytes());
    out.extend_from_slice(&revision.to_le_bytes());
    out.push(outcome.as_u8());
    out.extend_from_slice(&detail.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out
}

/// Link digest: `digest(prev || entry || seq)`.
fn link(prev: &[u8; 32], entry: &[u8; 32], seq: u64) -> [u8; 32] {
    digest32(&[prev, entry, &seq.to_le_bytes()])
}

/// Append-only digest chain over gateway calls. Every call appends exactly
/// one record; at most [`DOOR_AUDIT_CAP`] stay retained. Past the cap the
/// oldest record rotates out and the anchor advances to its chain digest, so
/// memory is bounded while the retained window still verifies from the
/// anchor and holders pin the head plus [`Self::total_appended`].
#[derive(Clone, Debug)]
pub struct DoorAudit {
    records: VecDeque<CallRecord>,
    /// Agent-loop calls beside the chain: the args plus the revisions on both
    /// sides, kept under the same cap as the records.
    calls: VecDeque<LoopCall>,
    /// Chain digest of the newest rotated-out record (zeros while nothing
    /// rotated out): verification of the retained window starts here.
    anchor: [u8; 32],
    /// Records rotated out so far; the first retained record carries
    /// sequence `dropped + 1`.
    dropped: u64,
    /// Total records ever appended (also the next sequence number).
    appended: u64,
}

impl DoorAudit {
    /// Empty log at the zero anchor.
    pub fn new() -> Self {
        Self {
            records: VecDeque::new(),
            calls: VecDeque::new(),
            anchor: [0u8; 32],
            dropped: 0,
            appended: 0,
        }
    }

    /// Retained record count.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when no call has been recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Records rotated out past [`DOOR_AUDIT_CAP`] so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Anchor digest the retained window chains from (zeros until the first
    /// rotation).
    pub fn anchor(&self) -> [u8; 32] {
        self.anchor
    }

    /// Total records ever appended.
    pub fn total_appended(&self) -> u64 {
        self.appended
    }

    /// Head chain digest, if any record is retained.
    pub fn head(&self) -> Option<[u8; 32]> {
        self.records.back().map(|record| record.chain)
    }

    /// Retained records, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &CallRecord> {
        self.records.iter()
    }

    /// Chain one record over its canonical bytes and hand back the sealed
    /// record. The tool name truncates to its bound; the only failure is
    /// an exhausted sequence counter, so a logging error is always fatal
    /// for the caller and never a silent drop.
    pub fn append(
        &mut self,
        session: u64,
        tool: &str,
        revision: u64,
        outcome: AuditOutcome,
        detail: u64,
    ) -> Result<&CallRecord, AuditError> {
        let seq = self
            .appended
            .checked_add(1)
            .ok_or(AuditError::SequenceExhausted)?;
        let end = tool.floor_char_boundary(MAX_TOOL_NAME_LEN);
        let prev = self.head().unwrap_or(self.anchor);
        let bytes = entry_bytes(session, &tool[..end], revision, outcome, detail, seq);
        let entry = digest32(&[&bytes]);
        let chain = link(&prev, &entry, seq);
        if self.records.len() >= DOOR_AUDIT_CAP {
            if let Some(rotated) = self.records.pop_front() {
                self.anchor = rotated.chain;
                self.dropped = rotated.seq;
            }
        }
        self.records.push_back(CallRecord {
            seq,
            session,
            tool: tool[..end].to_owned(),
            revision,
            outcome,
            detail,
            prev,
            chain,
        });
        self.appended = seq;
        self.records.back().ok_or(AuditError::SequenceExhausted)
    }

    /// Chain one record for an agent-loop call: the args are kept verbatim
    /// beside the record (never truncated) and their digest becomes the
    /// record's `detail` word, so a changed args string breaks the chain like
    /// any other edited field. Replay reads [`Self::loop_calls`] in order.
    /// Errors: [`AuditError::ArgsTooLarge`] when the args exceed
    /// [`MAX_ARGS_BYTES`] (the refusal happens before anything is kept, so no
    /// call is ever logged with different args than it ran with), and
    /// [`AuditError::SequenceExhausted`].
    pub fn append_call(
        &mut self,
        session: u64,
        tool: &str,
        args: &str,
        revision_before: u64,
        revision_after: u64,
        outcome: AuditOutcome,
    ) -> Result<&LoopCall, AuditError> {
        check_args(args)?;
        let digest = args_digest(args);
        let record = self
            .append(session, tool, revision_after, outcome, digest)?
            .clone();
        if self.calls.len() >= DOOR_AUDIT_CAP {
            self.calls.pop_front();
        }
        self.calls.push_back(LoopCall {
            record,
            args: args.to_owned(),
            args_digest: digest,
            revision_before,
            revision_after,
        });
        self.calls.back().ok_or(AuditError::SequenceExhausted)
    }

    /// Agent-loop calls retained beside the chain, oldest first.
    pub fn loop_calls(&self) -> impl Iterator<Item = &LoopCall> {
        self.calls.iter()
    }

    /// Streaming checker positioned at the start of the retained window: feed
    /// it [`Self::iter`] (or a cloned export of it) record by record. A log
    /// that never rotated starts at the zero anchor, like [`AuditVerifier::new`].
    pub fn window_verifier(&self) -> AuditVerifier {
        AuditVerifier::from_anchor(self.dropped.saturating_add(1), self.anchor)
    }

    /// Verify the retained window: dense sequence from the first retained
    /// record, linked `prev` digests starting at the anchor, recomputed entry
    /// digests from the stored fields, and recomputed chain links. Any naive
    /// delete, modify, reorder or splice fails. A truncated tail still
    /// verifies as a prefix, so holders pin with [`Self::verify_pinned`].
    pub fn verify(&self) -> Result<(), AuditError> {
        let mut checker = self.window_verifier();
        self.records
            .iter()
            .try_for_each(|record| checker.feed(record))
    }

    /// Verify plus pin: the head digest and the total appended length must
    /// match the holder's values, so a truncated tail or a re-sealed
    /// suffix (which always moves the head) fails loudly.
    pub fn verify_pinned(&self, head: [u8; 32], total: u64) -> Result<(), AuditError> {
        self.verify()?;
        let found = self.total_appended();
        if found != total {
            return Err(AuditError::LengthMismatch {
                expected: total,
                found,
            });
        }
        if self.head() != Some(head) {
            return Err(AuditError::HeadMismatch);
        }
        Ok(())
    }
}

impl Default for DoorAudit {
    fn default() -> Self {
        Self::new()
    }
}

/// Detail witness for a refused call: the outcome plus the machine-code hash,
/// so refusals stay distinguishable without storing free text.
pub fn code_hash(code: &str) -> u64 {
    let mut state = MIX_GAMMA;
    for byte in code.bytes() {
        state = mix_byte(state, byte);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six calls across the outcome and detail space, like a small session.
    fn fill(log: &mut DoorAudit, session: u64) {
        const TOOLS: [&str; 6] = [
            "scene.read",
            "scene.brief",
            "tx.preview",
            "tx.apply",
            "tx.undo",
            "log.digest",
        ];
        for (index, tool) in TOOLS.iter().enumerate() {
            let call = index as u64;
            let outcome = if call % 3 == 2 {
                AuditOutcome::Refused
            } else {
                AuditOutcome::Ok
            };
            log.append(session, tool, call, outcome, call * 7919)
                .expect("test append chains");
        }
    }

    #[test]
    fn chained_records_cover_every_call() {
        let mut log = DoorAudit::new();
        assert!(log.is_empty());
        assert_eq!(log.head(), None);
        log.verify().expect("empty log verifies");
        fill(&mut log, 7);
        assert_eq!(log.len(), 6);
        assert_eq!(log.total_appended(), 6);
        // Dense sequence with linked predecessors, both outcomes present.
        for (index, record) in log.iter().enumerate() {
            assert_eq!(record.seq, index as u64 + 1);
            assert_eq!(record.session, 7);
            if index == 0 {
                assert_eq!(record.prev, [0u8; 32]);
            } else {
                let prev_chain = log.records[index - 1].chain;
                assert_eq!(record.prev, prev_chain);
            }
        }
        assert!(log.iter().any(|record| record.outcome == AuditOutcome::Ok));
        assert!(log
            .iter()
            .any(|record| record.outcome == AuditOutcome::Refused));
        // Verify is deterministic: twice gives the same verdict.
        log.verify().expect("log verifies");
        log.verify().expect("log verifies twice");
        let head = log.head().expect("head");
        log.verify_pinned(head, 6).expect("pinned head verifies");
    }

    #[test]
    fn append_hands_back_the_sealed_record() {
        let mut log = DoorAudit::new();
        let record = log
            .append(7, "scene.read", 0, AuditOutcome::Ok, 11)
            .expect("first append chains");
        assert_eq!(record.seq, 1);
        assert_eq!(record.session, 7);
        assert_eq!(record.tool, "scene.read");
        log.verify().expect("single record verifies");
        assert_eq!(
            AuditError::SequenceExhausted.to_string(),
            "door audit sequence exhausted"
        );
    }

    #[test]
    fn same_calls_seal_same_head() {
        let mut first = DoorAudit::new();
        fill(&mut first, 7);
        let mut second = DoorAudit::new();
        fill(&mut second, 7);
        assert_eq!(first.head(), second.head());
        // The session binds the seal: identical calls from another
        // session seal a different head.
        let mut other = DoorAudit::new();
        fill(&mut other, 9);
        assert_ne!(first.head(), other.head());
    }

    /// 100 sessions, one tamper each, rotating drop/edit/reorder/splice/
    /// tail-cut. Every tamper must fail loudly: 100 attempted, 100 detected.
    #[test]
    fn hundred_sessions_detect_every_tamper_class() {
        const SESSIONS: usize = 100;
        let mut detected = 0usize;
        let mut per_class = [0usize; 5];
        for session in 0..SESSIONS {
            let mut log = DoorAudit::new();
            fill(&mut log, session as u64);
            log.verify().expect("untampered session verifies");
            let head = log.head().expect("head");
            let total = log.total_appended();
            let mut tampered = log.clone();
            match session % 5 {
                0 => {
                    // Drop a middle record.
                    tampered.records.remove(2);
                    if tampered.verify().is_err() {
                        detected += 1;
                        per_class[0] += 1;
                    }
                }
                1 => {
                    // Edit one detail witness in place.
                    tampered.records[3].detail ^= 0xFF;
                    if tampered.verify().is_err() {
                        detected += 1;
                        per_class[1] += 1;
                    }
                }
                2 => {
                    // Reorder two records.
                    tampered.records.swap(1, 4);
                    if tampered.verify().is_err() {
                        detected += 1;
                        per_class[2] += 1;
                    }
                }
                3 => {
                    // Splice a foreign suffix from another session.
                    let mut foreign = DoorAudit::new();
                    fill(&mut foreign, 10_000 + session as u64);
                    tampered.records[4] = foreign.records[4].clone();
                    tampered.records[5] = foreign.records[5].clone();
                    if tampered.verify().is_err() {
                        detected += 1;
                        per_class[3] += 1;
                    }
                }
                _ => {
                    // Cut the tail: still link-valid, so pin the live head.
                    tampered.records.pop_back();
                    if tampered.verify_pinned(head, total).is_err() {
                        detected += 1;
                        per_class[4] += 1;
                    }
                }
            }
        }
        eprintln!(
            "door-audit-tamper sessions={SESSIONS} detected={detected} \
             drop={} edit={} reorder={} splice={} tail={}",
            per_class[0], per_class[1], per_class[2], per_class[3], per_class[4]
        );
        assert_eq!(detected, SESSIONS, "every tamper is detected");
        assert_eq!(per_class.iter().sum::<usize>(), SESSIONS);
        assert!(per_class.iter().all(|count| *count == SESSIONS / 5));
    }

    #[test]
    fn drop_and_reorder_break_order_loudly() {
        let mut log = DoorAudit::new();
        fill(&mut log, 7);
        let mut dropped = log.clone();
        dropped.records.remove(2);
        assert!(
            matches!(dropped.verify(), Err(AuditError::OrderBroken { seq: 4 })),
            "dropped record fails at the next seq"
        );
        let mut reordered = log.clone();
        reordered.records.swap(0, 1);
        assert!(
            matches!(reordered.verify(), Err(AuditError::OrderBroken { .. })),
            "reordered records fail continuity"
        );
    }

    #[test]
    fn edit_breaks_chain_loudly() {
        let mut log = DoorAudit::new();
        fill(&mut log, 7);
        let mut renamed = log.clone();
        renamed.records[1].tool = "renamed".to_owned();
        assert!(
            matches!(renamed.verify(), Err(AuditError::ChainBroken { seq: 2 })),
            "renamed record fails at its seq"
        );
        let mut rescoped = log.clone();
        rescoped.records[0].session ^= 1;
        assert!(
            matches!(rescoped.verify(), Err(AuditError::ChainBroken { seq: 1 })),
            "resessioned record fails at its seq"
        );
    }

    #[test]
    fn foreign_suffix_fails_against_pinned_head() {
        let mut log = DoorAudit::new();
        fill(&mut log, 7);
        let head = log.head().expect("head");
        let total = log.total_appended();
        // A re-sealed suffix over the same prefix moves the head.
        let mut resealed = log.clone();
        resealed.records[5].detail = resealed.records[5].detail.wrapping_add(1);
        let bytes = entry_bytes(
            resealed.records[5].session,
            &resealed.records[5].tool,
            resealed.records[5].revision,
            resealed.records[5].outcome,
            resealed.records[5].detail,
            resealed.records[5].seq,
        );
        let entry = digest32(&[&bytes]);
        let prev = resealed.records[5].prev;
        resealed.records[5].chain = link(&prev, &entry, 6);
        assert!(
            matches!(
                resealed.verify_pinned(head, total),
                Err(AuditError::HeadMismatch)
            ),
            "re-sealed suffix fails the pin"
        );
        // A foreign log of the same length fails the pin too.
        let mut foreign = DoorAudit::new();
        fill(&mut foreign, 9);
        assert!(
            matches!(
                foreign.verify_pinned(head, total),
                Err(AuditError::HeadMismatch)
            ),
            "foreign log fails the pin"
        );
        // A cut tail verifies as a prefix but moves the head, so it fails
        // the pin; a forged extension instead moves the total.
        let mut cut = log.clone();
        cut.records.pop_back();
        cut.verify().expect("cut tail is a valid prefix");
        assert!(
            matches!(
                cut.verify_pinned(head, total),
                Err(AuditError::HeadMismatch)
            ),
            "cut tail fails the pinned head"
        );
        let mut extended = log.clone();
        extended
            .append(7, "scene.read", total, AuditOutcome::Ok, 0)
            .expect("extension append chains");
        assert!(
            matches!(
                extended.verify_pinned(head, total),
                Err(AuditError::LengthMismatch { .. })
            ),
            "extended tail fails the pinned length"
        );
    }

    #[test]
    fn overlong_tool_names_truncate_deterministically() {
        let long = "t".repeat(MAX_TOOL_NAME_LEN + 40);
        let mut first = DoorAudit::new();
        first
            .append(7, &long, 3, AuditOutcome::Ok, 11)
            .expect("truncated append chains");
        first.verify().expect("truncated record verifies");
        let record = first.iter().next().expect("one record");
        assert_eq!(record.tool.len(), MAX_TOOL_NAME_LEN);
        let mut second = DoorAudit::new();
        second
            .append(7, &long, 3, AuditOutcome::Ok, 11)
            .expect("truncated append chains");
        assert_eq!(first.head(), second.head());
    }

    /// Memory is bounded: ten caps of appends retain one cap of records, the
    /// anchor advances, the window verifies from it, and tamper inside the
    /// window or against the pin still fails.
    #[test]
    fn rotation_cap_bounds_memory_and_keeps_the_window_verifiable() {
        let total = DOOR_AUDIT_CAP as u64 * 10 + 3;
        let mut log = DoorAudit::new();
        for i in 0..total {
            log.append(7, "scene.read", i, AuditOutcome::Ok, i)
                .expect("rotating append chains");
            assert!(
                log.len() <= DOOR_AUDIT_CAP,
                "retention never passes the cap"
            );
        }
        assert_eq!(log.len(), DOOR_AUDIT_CAP);
        assert_eq!(log.total_appended(), total);
        assert_eq!(log.dropped(), total - DOOR_AUDIT_CAP as u64);
        assert_ne!(log.anchor(), [0u8; 32], "anchor advanced past genesis");
        assert_eq!(log.iter().next().expect("window").seq, log.dropped() + 1);
        log.verify()
            .expect("retained window verifies from the anchor");
        let head = log.head().expect("head");
        log.verify_pinned(head, total).expect("pinned rotated log");
        let mut edited = log.clone();
        edited.records[5].detail ^= 1;
        assert!(edited.verify().is_err(), "edit inside the window fails");
        let mut dropped = log.clone();
        dropped.records.pop_front();
        assert!(dropped.verify().is_err(), "dropping the oldest fails");
        let mut cut = log.clone();
        cut.records.pop_back();
        assert!(
            cut.verify_pinned(head, total).is_err(),
            "cut tail fails the pin"
        );
    }

    #[test]
    fn append_cost_stays_within_budget() {
        const N: u64 = 2_000;
        let mut log = DoorAudit::new();
        let start = std::time::Instant::now();
        for i in 0..N {
            log.append(7, "scene.read", i, AuditOutcome::Ok, i)
                .expect("budget append chains");
        }
        // Integer mean only: floating point never enters this module.
        let mean = start.elapsed().as_nanos() / u128::from(N);
        let budget = u128::from(DOOR_AUDIT_BUDGET_NS);
        eprintln!("door-audit-append mean={mean}ns over {N} appends (budget {budget}ns)");
        assert!(
            mean < budget,
            "append mean {mean}ns exceeds the budget {budget}ns"
        );
        log.verify().expect("measured log verifies");
    }
}
