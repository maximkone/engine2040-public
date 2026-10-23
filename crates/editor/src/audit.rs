//! Digest-chained audit receipts over the crash-safe journal.
//!
//! Consumer: the editor dock [`Shell`](crate::shell::Shell) owns one chain and
//! chains a receipt per acknowledged apply through [`AuditChain::record_applied`];
//! [`audit_apply`] stays the WAL-first entry for callers that drive a [`Wal`].
//! The data owner is [`AuditChain`]. Units: seq/revision
//! counters, bytes for entry encodings. Determinism: little-endian canonical
//! bytes, FNV-based `digest_parts` (also used by instant snapshots — no new
//! crypto). Errors:
//! [`AuditError`]; oversize refuses before appending. Budget: at most
//! 0.5 ms per appended receipt (measured mean in tests).
//!
//! The journal files are frozen: this module only reads WAL types and never
//! rewrites them. Rotation evicts the oldest receipt past [`AUDIT_CAP`] and
//! advances the anchor digest, so the retained window stays verifiable.

use std::collections::VecDeque;
use std::fmt;

use engine_core::hash::{FNV64_OFFSET, GOLDEN_GAMMA};
use engine_scene::ids::ObjectId;

use crate::commands::digest::push_text;
use crate::commands::{Applied, Batch, BondStrength, CommandMeta, Document, Journal, Op};
use crate::wal::{wal_apply, Wal, WalError};

/// Bounded rotation cap: at most this many receipts are retained. Past the
/// cap the oldest receipt is evicted and the anchor advances to its chain
/// digest, so verification restarts from a pinned prefix, never unbounded.
pub const AUDIT_CAP: usize = 256;
/// Largest accepted op encoding per receipt (mirrors the WAL frame bound:
/// one 1 MiB batch is far past any real edit; bigger refuses like the WAL).
pub const MAX_AUDIT_OP_BYTES: usize = 1 << 20;
/// Largest accepted op count per receipt (mirrors the WAL decoder bound).
pub const MAX_AUDIT_OPS: usize = 65_536;

/// What rejected an audit append or a verification.
#[derive(Debug)]
pub enum AuditError {
    /// The op encoding exceeds its bound; nothing was appended.
    TooLarge,
    /// The underlying WAL apply failed; nothing was appended.
    Wal(WalError),
    /// A receipt's entry or chain digest does not recompute at this seq.
    ChainBroken {
        /// Sequence number of the first receipt that fails.
        seq: u64,
    },
    /// A receipt's `prev` or `seq` does not continue the chain at this seq.
    OrderBroken {
        /// Sequence number where continuity breaks.
        seq: u64,
    },
    /// The chain head does not match the externally pinned head digest
    /// (a re-sealed suffix or a truncated tail).
    HeadMismatch,
    /// The chain length does not match the externally pinned length
    /// (a truncated or extended tail).
    LengthMismatch {
        /// Length the holder pinned.
        expected: u64,
        /// Length presented.
        found: u64,
    },
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("audit receipt exceeds its size bound"),
            Self::Wal(error) => write!(f, "audit wal apply: {error}"),
            Self::ChainBroken { seq } => {
                write!(f, "audit chain broken at seq {seq}")
            }
            Self::OrderBroken { seq } => {
                write!(f, "audit chain order broken at seq {seq}")
            }
            Self::HeadMismatch => f.write_str("audit head does not match the pinned digest"),
            Self::LengthMismatch { expected, found } => {
                write!(f, "audit length mismatch: pinned {expected}, found {found}")
            }
        }
    }
}

impl std::error::Error for AuditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Wal(error) => Some(error),
            _ => None,
        }
    }
}

impl From<WalError> for AuditError {
    fn from(error: WalError) -> Self {
        Self::Wal(error)
    }
}

/// Multiplier of the audit and instant-snapshot digests. Sealed receipts and
/// snapshots verify against it, so it stays distinct from the standard FNV-1a
/// prime (`engine_core::hash::FNV64_PRIME`); changing it breaks every chain.
const PINNED_PRIME: u64 = 0x1000_0000_01B3;

/// Byte fold from `state` (xor, then multiply by [`PINNED_PRIME`]).
pub(crate) fn fold_pinned(mut state: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(PINNED_PRIME);
    }
    state
}

/// 32-byte digest over byte parts: four folds whose start states differ by
/// pass number, one little-endian word each. Same parts, same digest; no new
/// crypto.
pub(crate) fn digest_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (pass, slot) in (1u64..).zip(out.chunks_mut(8)) {
        let mut hash = FNV64_OFFSET ^ GOLDEN_GAMMA.wrapping_mul(pass);
        for part in parts {
            hash = fold_pinned(hash, part);
        }
        slot.copy_from_slice(&hash.to_le_bytes());
    }
    out
}

/// One tamper-evident receipt: the entry digest covers the edit, the chain
/// digest covers the previous chain digest plus the entry digest plus the
/// sequence number, so any delete, modify or reorder breaks the link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// Dense sequence number (1-based across rotations).
    pub seq: u64,
    /// Fixed tick the edit was witnessed on (caller clock, 0 when none).
    pub tick: u64,
    /// Document revision the edit published.
    pub revision: u64,
    /// Idempotency key the receipt answers.
    pub key: u64,
    /// Command name (bounded by the command service).
    pub name: String,
    /// Durable-journal flag of the command.
    pub significant: bool,
    /// Digest over the op encoding (entry content beyond the header).
    pub ops_digest: [u8; 32],
    /// Digest over the header plus the ops digest.
    pub entry_digest: [u8; 32],
    /// Previous chain digest (the anchor for the first retained receipt).
    pub prev: [u8; 32],
    /// Chain digest: `digest(prev || entry_digest || seq)`.
    pub chain: [u8; 32],
}

/// Canonical header bytes for one entry (little-endian, length-prefixed
/// text, no maps or pointers: same edit, same bytes).
fn header_bytes(
    tick: u64,
    revision: u64,
    key: u64,
    name: &str,
    significant: bool,
    ops_digest: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(41 + name.len());
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&revision.to_le_bytes());
    out.extend_from_slice(&key.to_le_bytes());
    out.push(u8::from(significant));
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(ops_digest);
    out
}

/// Canonical op encoding (tag plus node/parent ids plus raw pose bits).
fn encode_ops(ops: &[Op]) -> Result<Vec<u8>, AuditError> {
    if ops.len() > MAX_AUDIT_OPS {
        return Err(AuditError::TooLarge);
    }
    let mut out = Vec::new();
    for op in ops {
        match op {
            Op::Insert { id, parent, local } => {
                out.push(0);
                out.extend_from_slice(id.as_uuid().as_bytes());
                out.extend_from_slice(parent.as_uuid().as_bytes());
                for lane in local
                    .position
                    .iter()
                    .chain(local.rotation.iter())
                    .chain(local.scale.iter())
                {
                    out.extend_from_slice(&lane.to_bits().to_le_bytes());
                }
            }
            Op::Remove { id } => {
                out.push(1);
                out.extend_from_slice(id.as_uuid().as_bytes());
            }
            Op::SetLocal { id, local } => {
                out.push(2);
                out.extend_from_slice(id.as_uuid().as_bytes());
                for lane in local
                    .position
                    .iter()
                    .chain(local.rotation.iter())
                    .chain(local.scale.iter())
                {
                    out.extend_from_slice(&lane.to_bits().to_le_bytes());
                }
            }
            Op::Reparent { id, new_parent } => {
                out.push(3);
                out.extend_from_slice(id.as_uuid().as_bytes());
                out.extend_from_slice(new_parent.as_uuid().as_bytes());
            }
            Op::SetMaterial { id, material } => {
                out.push(4);
                out.extend_from_slice(id.as_uuid().as_bytes());
                match material {
                    Some(name) => {
                        let text = name.as_str();
                        let len = name.len_bytes();
                        out.extend_from_slice(&len.to_le_bytes());
                        out.extend_from_slice(text.as_bytes());
                    }
                    None => out.extend_from_slice(&0u64.to_le_bytes()),
                }
            }
            Op::SetComponent { id, component } => {
                out.push(5);
                out.extend_from_slice(id.as_uuid().as_bytes());
                let bytes = component.to_bytes();
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(&bytes);
            }
            Op::RemoveComponent { id, kind } => {
                out.push(6);
                out.extend_from_slice(id.as_uuid().as_bytes());
                out.push(kind.tag());
            }
            Op::PutMaterial { text } => {
                out.push(7);
                push_text(&mut out, text);
            }
            Op::SetMaterialParam {
                material,
                path,
                value,
            } => {
                out.push(8);
                push_text(&mut out, material);
                push_text(&mut out, path);
                push_text(&mut out, value);
            }
            Op::RemoveLastMaterial => out.push(9),
            Op::SetBond { id, a, b, strength } => {
                encode_set_bond(&mut out, *id, *a, *b, *strength);
            }
        }
        if out.len() > MAX_AUDIT_OP_BYTES {
            return Err(AuditError::TooLarge);
        }
    }
    Ok(out)
}

/// Tag 10: one bond paint, its canonical pair and optional strength.
fn encode_set_bond(
    out: &mut Vec<u8>,
    id: ObjectId,
    a: u32,
    b: u32,
    strength: Option<BondStrength>,
) {
    out.push(10);
    out.extend_from_slice(id.as_uuid().as_bytes());
    out.extend_from_slice(&a.to_le_bytes());
    out.extend_from_slice(&b.to_le_bytes());
    match strength {
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.permille().to_le_bytes());
        }
        None => out.push(0),
    }
}

/// Link digest: `digest(prev || entry_digest || seq)`.
fn link(prev: &[u8; 32], entry_digest: &[u8; 32], seq: u64) -> [u8; 32] {
    digest_parts(&[prev, entry_digest, &seq.to_le_bytes()])
}

/// Append-only digest chain with a bounded rotation cap.
///
/// Units: seq/revision counters, bytes for entry encodings. Ownership: the
/// editor [`Shell`](crate::shell::Shell) owns one chain and calls
/// [`Self::record_applied`] after each acknowledged apply; tests call
/// [`audit_apply`] after each acknowledged WAL apply. Threading: single-threaded, no locks; send the receipts, not the
/// chain, across threads. Determinism: little-endian canonical bytes with the
/// FNV-based `digest_parts`, so the same edits always chain the same
/// digests. Errors: [`AuditError`]; oversize refuses before appending.
/// Budget: at most 0.5 ms per appended receipt; at most [`AUDIT_CAP`]
/// receipts retained, then the oldest is evicted and the anchor advances.
///
/// # Example
///
/// ```rust
/// use engine_editor::audit::AuditChain;
///
/// let mut chain = AuditChain::new();
/// chain
///     .append_entry(0, 1, 7, "remove", true, &[])
///     .expect("empty-op entry appends");
/// chain.verify().expect("fresh chain verifies");
/// let head = chain.head().expect("head exists");
/// chain.verify_pinned(head, 1).expect("pinned head verifies");
/// ```
#[derive(Clone, Debug)]
pub struct AuditChain {
    receipts: VecDeque<Receipt>,
    /// Chain digest of the newest evicted receipt (zeros while nothing
    /// evicted): verification of the retained window starts here.
    anchor: [u8; 32],
    /// Sequence number of the first retained receipt (1-based).
    base_seq: u64,
    /// Count of receipts evicted past the cap.
    dropped: u64,
    /// Total receipts ever appended (dense seq source).
    appended: u64,
}

impl AuditChain {
    /// Empty chain at the zero anchor.
    pub fn new() -> Self {
        Self {
            receipts: VecDeque::new(),
            anchor: [0u8; 32],
            base_seq: 1,
            dropped: 0,
            appended: 0,
        }
    }

    /// Retention cap ([`AUDIT_CAP`]).
    pub fn cap(&self) -> usize {
        AUDIT_CAP
    }

    /// Retained receipt count (never past the cap).
    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    /// True when no receipt is retained.
    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }

    /// Receipts evicted past the cap so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Anchor digest the retained window chains from.
    pub fn anchor(&self) -> [u8; 32] {
        self.anchor
    }

    /// Head chain digest, if any receipt is retained.
    pub fn head(&self) -> Option<[u8; 32]> {
        self.receipts.back().map(|receipt| receipt.chain)
    }

    /// Total receipts ever appended (retained plus evicted).
    pub fn total_appended(&self) -> u64 {
        self.appended
    }

    /// Retained receipts, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Receipt> {
        self.receipts.iter()
    }

    /// Append one entry over its canonical bytes. Evicts the oldest receipt
    /// past the cap, advancing the anchor to the evicted chain digest.
    pub fn append_entry(
        &mut self,
        tick: u64,
        revision: u64,
        key: u64,
        name: &str,
        significant: bool,
        ops: &[Op],
    ) -> Result<&Receipt, AuditError> {
        let ops_bytes = encode_ops(ops)?;
        let ops_digest = digest_parts(&[&ops_bytes]);
        let header = header_bytes(tick, revision, key, name, significant, &ops_digest);
        let entry_digest = digest_parts(&[&header]);
        let seq = self.appended + 1;
        let prev = self.head().unwrap_or(self.anchor);
        let chain = link(&prev, &entry_digest, seq);
        self.appended = seq;
        if self.receipts.len() >= AUDIT_CAP {
            if let Some(evicted) = self.receipts.pop_front() {
                self.anchor = evicted.chain;
                self.base_seq = evicted.seq + 1;
                self.dropped += 1;
            }
        }
        self.receipts.push_back(Receipt {
            seq,
            tick,
            revision,
            key,
            name: name.to_owned(),
            significant,
            ops_digest,
            entry_digest,
            prev,
            chain,
        });
        Ok(self.receipts.back().expect("just pushed"))
    }

    /// Record an acknowledged apply: the canonical entry covers the WAL
    /// tick, the published revision, the batch key, the command name and
    /// the batch ops. Call only after the WAL apply succeeded.
    pub fn record_applied(
        &mut self,
        tick: u64,
        batch: &Batch,
        meta: &CommandMeta,
        applied: &Applied,
    ) -> Result<&Receipt, AuditError> {
        self.append_entry(
            tick,
            applied.revision,
            batch.idempotency_key,
            meta.name(),
            meta.is_significant(),
            &batch.ops,
        )
    }

    /// Verify the retained window: dense seq from the base, `prev` links,
    /// recomputed entry digests from the stored header fields, and
    /// recomputed chain links. Any naive delete, modify or reorder fails.
    /// A truncated tail or a fully re-sealed suffix needs [`Self::verify_pinned`].
    pub fn verify(&self) -> Result<(), AuditError> {
        let mut prev = self.anchor;
        for (index, receipt) in self.receipts.iter().enumerate() {
            let want_seq = self.base_seq + index as u64;
            if receipt.seq != want_seq {
                return Err(AuditError::OrderBroken { seq: receipt.seq });
            }
            if receipt.prev != prev {
                return Err(AuditError::OrderBroken { seq: receipt.seq });
            }
            let header = header_bytes(
                receipt.tick,
                receipt.revision,
                receipt.key,
                &receipt.name,
                receipt.significant,
                &receipt.ops_digest,
            );
            if digest_parts(&[&header]) != receipt.entry_digest {
                return Err(AuditError::ChainBroken { seq: receipt.seq });
            }
            if link(&receipt.prev, &receipt.entry_digest, receipt.seq) != receipt.chain {
                return Err(AuditError::ChainBroken { seq: receipt.seq });
            }
            prev = receipt.chain;
        }
        Ok(())
    }

    /// Verify plus pin: the head digest and the total appended length must
    /// match the holder's values, so a truncated tail or a re-sealed
    /// suffix (which always moves the head) fails loudly.
    pub fn verify_pinned(&self, head: [u8; 32], total: u64) -> Result<(), AuditError> {
        self.verify()?;
        if self.total_appended() != total {
            return Err(AuditError::LengthMismatch {
                expected: total,
                found: self.total_appended(),
            });
        }
        match self.head() {
            Some(live) if live == head => Ok(()),
            _ => Err(AuditError::HeadMismatch),
        }
    }
}

impl Default for AuditChain {
    fn default() -> Self {
        Self::new()
    }
}

/// Journaled apply with a chained receipt: [`wal_apply`] runs
/// first, and only its acknowledged receipt is chained. A refused batch
/// appends nothing, so the chain never leads the journal.
pub fn audit_apply(
    wal: &mut Wal,
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
    meta: &CommandMeta,
    tick: u64,
    chain: &mut AuditChain,
) -> Result<Applied, AuditError> {
    let applied = wal_apply(wal, doc, journal, batch, meta, tick)?;
    chain.record_applied(tick, batch, meta, &applied)?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::MAX_COMMAND_NAME_LEN;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-editor-audit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir creates");
        dir
    }

    fn id(n: u64) -> ObjectId {
        ObjectId::parse(&format!("00000000-0000-0000-0000-{n:012}")).expect("fixture id parses")
    }

    fn pose(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn insert_batch(node: ObjectId, parent: ObjectId, x: f32, rev: u64, key: u64) -> Batch {
        Batch {
            ops: vec![Op::Insert {
                id: node,
                parent,
                local: pose(x),
            }],
            base_revision: rev,
            idempotency_key: key,
            provenance: None,
        }
    }

    #[test]
    fn chained_receipts_cover_the_wal_journal() {
        let dir = scratch("wal-chain");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh wal");
        let mut chain = AuditChain::new();
        let mut parent = ObjectId::NIL;
        for i in 0..20u64 {
            let node = id(i + 1);
            let batch = insert_batch(node, parent, i as f32, doc.revision, 900 + i);
            let meta = CommandMeta::significant("insert");
            let applied = audit_apply(
                &mut wal,
                &mut doc,
                &mut journal,
                &batch,
                &meta,
                i,
                &mut chain,
            )
            .expect("applies and chains");
            assert_eq!(applied.revision, doc.revision);
            parent = node;
        }
        assert_eq!(chain.len(), 20);
        assert_eq!(chain.total_appended(), 20);
        // Each receipt covers its predecessor: dense seq, linked prev.
        for (index, receipt) in chain.iter().enumerate() {
            assert_eq!(receipt.seq, index as u64 + 1);
            if index == 0 {
                assert_eq!(receipt.prev, [0u8; 32]);
            } else {
                let prev_chain = chain.iter().nth(index - 1).expect("prev").chain;
                assert_eq!(receipt.prev, prev_chain);
            }
        }
        // Verify is deterministic: twice gives the same verdict.
        chain.verify().expect("chain verifies");
        chain.verify().expect("chain verifies twice");
        let head = chain.head().expect("head");
        chain.verify_pinned(head, 20).expect("pinned head verifies");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refused_batch_chains_nothing() {
        let dir = scratch("refused");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh wal");
        let mut chain = AuditChain::new();
        let batch = insert_batch(id(1), ObjectId::NIL, 0.0, doc.revision, 1);
        audit_apply(
            &mut wal,
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::significant("insert"),
            0,
            &mut chain,
        )
        .expect("first applies");
        // Stale base revision: the WAL refuses, so the chain stays put.
        let stale = insert_batch(id(2), ObjectId::NIL, 1.0, 0, 2);
        let failed = audit_apply(
            &mut wal,
            &mut doc,
            &mut journal,
            &stale,
            &CommandMeta::significant("insert"),
            1,
            &mut chain,
        );
        assert!(failed.is_err(), "stale batch refuses");
        assert_eq!(chain.len(), 1, "refused batch chains nothing");
        chain.verify().expect("chain still verifies");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 100 sessions, one tamper each, rotating delete/modify/reorder/tail-cut.
    /// Every tamper must fail verification: 100 attempted, 100 detected.
    #[test]
    fn hundred_sessions_tamper_detected() {
        const SESSIONS: usize = 100;
        const ENTRIES: u64 = 6;
        let mut detected = 0usize;
        let mut delete_caught = 0usize;
        let mut modify_caught = 0usize;
        let mut reorder_caught = 0usize;
        let mut tail_caught = 0usize;
        for session in 0..SESSIONS {
            let mut chain = AuditChain::new();
            for i in 0..ENTRIES {
                let node = id(session as u64 * 100 + i + 1);
                let ops = vec![Op::Insert {
                    id: node,
                    parent: ObjectId::NIL,
                    local: pose(i as f32),
                }];
                chain
                    .append_entry(
                        i,
                        i + 1,
                        10_000 + session as u64 * 100 + i,
                        "insert",
                        true,
                        &ops,
                    )
                    .expect("appends");
            }
            chain.verify().expect("untampered session verifies");
            let head = chain.head().expect("head");
            let total = chain.total_appended();
            let mut tampered = chain.clone();
            match session % 4 {
                0 => {
                    // Delete a middle receipt.
                    tampered.receipts.remove(2);
                    if tampered.verify().is_err() {
                        detected += 1;
                        delete_caught += 1;
                    }
                }
                1 => {
                    // Modify one entry digest byte in place.
                    tampered.receipts[3].entry_digest[0] ^= 0xFF;
                    if tampered.verify().is_err() {
                        detected += 1;
                        modify_caught += 1;
                    }
                }
                2 => {
                    // Reorder two receipts.
                    tampered.receipts.swap(1, 4);
                    if tampered.verify().is_err() {
                        detected += 1;
                        reorder_caught += 1;
                    }
                }
                _ => {
                    // Cut the tail: still link-valid, so pin the live head.
                    tampered.receipts.pop_back();
                    if tampered.verify_pinned(head, total).is_err() {
                        detected += 1;
                        tail_caught += 1;
                    }
                }
            }
        }
        eprintln!(
            "audit-tamper sessions={SESSIONS} detected={detected} \
             delete={delete_caught} modify={modify_caught} \
             reorder={reorder_caught} tail={tail_caught}"
        );
        assert_eq!(detected, SESSIONS, "every tamper is detected");
        assert_eq!(
            delete_caught + modify_caught + reorder_caught + tail_caught,
            SESSIONS
        );
    }

    #[test]
    fn header_field_edit_breaks_the_link() {
        let mut chain = AuditChain::new();
        for i in 0..4u64 {
            let ops = vec![Op::Remove { id: id(i + 1) }];
            chain
                .append_entry(i, i + 1, 500 + i, "remove", true, &ops)
                .expect("appends");
        }
        let mut tampered = chain.clone();
        tampered.receipts[1].name = "renamed".to_string();
        assert!(
            matches!(tampered.verify(), Err(AuditError::ChainBroken { seq: 2 })),
            "renamed receipt fails at its seq"
        );
        let mut tampered = chain.clone();
        tampered.receipts[0].tick ^= 1;
        assert!(tampered.verify().is_err(), "tick edit fails");
    }

    #[test]
    fn rotation_cap_holds_with_anchor() {
        let mut chain = AuditChain::new();
        assert_eq!(chain.cap(), AUDIT_CAP);
        let extra: u64 = 40;
        let total = AUDIT_CAP as u64 + extra;
        for i in 0..total {
            let ops = vec![Op::Remove { id: id(i + 1) }];
            chain
                .append_entry(i, i + 1, 700_000 + i, "remove", true, &ops)
                .expect("appends");
        }
        assert_eq!(chain.len(), AUDIT_CAP, "retention never passes the cap");
        assert_eq!(chain.dropped(), extra, "evictions are counted");
        assert_eq!(chain.total_appended(), total);
        assert_ne!(chain.anchor(), [0u8; 32], "anchor advanced past genesis");
        chain
            .verify()
            .expect("retained window verifies from the anchor");
        let head = chain.head().expect("head");
        chain
            .verify_pinned(head, total)
            .expect("pinned rotated chain verifies");
        // Tamper inside the retained window still fails after rotation.
        let mut tampered = chain.clone();
        tampered.receipts[10].ops_digest[7] ^= 0x01;
        assert!(tampered.verify().is_err(), "rotated window tamper fails");
        // A longer forged tail moves the head and fails the pin.
        let mut tampered = chain.clone();
        let ops = vec![Op::Remove { id: id(999_999) }];
        tampered
            .append_entry(total, total + 1, 1, "forged", true, &ops)
            .expect("appends");
        assert!(
            tampered.verify_pinned(head, total).is_err(),
            "forged tail fails the pin"
        );
        eprintln!(
            "audit-rotation cap={} len={} dropped={} total={total}",
            AUDIT_CAP,
            chain.len(),
            chain.dropped()
        );
    }

    #[test]
    fn oversize_entry_refuses_before_append() {
        let mut chain = AuditChain::new();
        let ops = vec![Op::Remove { id: id(1) }; MAX_AUDIT_OPS + 1];
        assert!(
            matches!(
                chain.append_entry(0, 1, 1, "huge", true, &ops),
                Err(AuditError::TooLarge)
            ),
            "over-count entry refuses"
        );
        assert!(chain.is_empty(), "refused entry appends nothing");
        chain.verify().expect("empty chain verifies");
        assert_eq!(chain.head(), None);
    }

    #[test]
    fn append_overhead_under_half_a_millisecond() {
        const N: u64 = 2_000;
        let mut chain = AuditChain::new();
        let start = std::time::Instant::now();
        for i in 0..N {
            let ops = vec![Op::Remove { id: id(i + 1) }];
            chain
                .append_entry(i, i + 1, 800_000 + i, "remove", true, &ops)
                .expect("appends");
        }
        let mean_ns = start.elapsed().as_nanos() as f64 / N as f64;
        eprintln!("audit-append-overhead mean={mean_ns:.0}ns over {N} appends");
        assert!(
            mean_ns < 500_000.0,
            "append mean {mean_ns:.0}ns exceeds the 0.5 ms budget"
        );
        chain.verify().expect("measured chain verifies");
    }

    #[test]
    fn long_names_stay_within_the_command_bound() {
        let mut chain = AuditChain::new();
        let long = "n".repeat(MAX_COMMAND_NAME_LEN + 40);
        let ops = vec![Op::Remove { id: id(1) }];
        chain
            .append_entry(0, 1, 1, &long, true, &ops)
            .expect("appends");
        chain.verify().expect("long-name receipt verifies");
    }
}
