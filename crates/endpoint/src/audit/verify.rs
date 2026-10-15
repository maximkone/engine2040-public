//! Streaming seal check for the door audit log.
//!
//! Consumer: any holder with borrowed records (the live log, a cloned export
//! sent to another client) feeds them in order without retaining the whole
//! log. The data owner stays [`super::DoorAudit`]; this module only reads.
//! Units: dense sequence numbers, 32-byte digests, nanoseconds for the budget.
//! Determinism: same records in the same order give the same verdict on every
//! platform; integer-only mixing, no wall clock, no floating point. Errors:
//! [`super::AuditError`], one named variant per tamper class. Budget: at most
//! [`super::DOOR_AUDIT_BUDGET_NS`] nanoseconds per verified record; state is
//! 48 bytes plus one bounded entry buffer, independent of log length.

use super::{AuditError, CallRecord, DOOR_AUDIT_BUDGET_NS};

/// Constant-memory seal checker: one digest plus two counters.
///
/// Feed borrowed records in order with [`Self::feed`]; the checker keeps only
/// the expected sequence number, the previous chain digest and the count, so
/// holders can verify logs far larger than memory by paging records through.
#[derive(Clone, Debug)]
pub struct AuditVerifier {
    next_seq: u64,
    prev: [u8; 32],
    seen: u64,
}

impl AuditVerifier {
    /// Empty checker at the zero anchor, expecting sequence 1.
    pub fn new() -> Self {
        Self {
            next_seq: 1,
            prev: [0u8; 32],
            seen: 0,
        }
    }

    /// Records checked so far.
    pub fn verified(&self) -> u64 {
        self.seen
    }

    /// Chain digest of the last accepted record, if any record was fed.
    pub fn head_digest(&self) -> Option<[u8; 32]> {
        if self.seen == 0 {
            None
        } else {
            Some(self.prev)
        }
    }

    /// Check one borrowed record against the running seal and advance.
    ///
    /// Errors: [`AuditError::OrderBroken`] when the sequence or `prev` link
    /// does not continue the chain; [`AuditError::ChainBroken`] when the
    /// recomputed entry or link digest differs; [`AuditError::SequenceExhausted`]
    /// when the dense counter cannot advance further.
    pub fn feed(&mut self, record: &CallRecord) -> Result<(), AuditError> {
        if record.seq != self.next_seq || record.prev != self.prev {
            return Err(AuditError::OrderBroken { seq: record.seq });
        }
        let bytes = super::entry_bytes(
            record.session,
            &record.tool,
            record.revision,
            record.outcome,
            record.detail,
            record.seq,
        );
        let entry = super::digest32(&[&bytes]);
        if super::link(&record.prev, &entry, record.seq) != record.chain {
            return Err(AuditError::ChainBroken { seq: record.seq });
        }
        self.prev = record.chain;
        self.seen = self.seen.wrapping_add(1);
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or(AuditError::SequenceExhausted)?;
        Ok(())
    }

    /// Pin check over the streamed prefix: the fed count and the streamed
    /// head must match the holder's values, so a truncated tail, a re-sealed
    /// suffix or a forged extension fails loudly.
    ///
    /// Errors: [`AuditError::LengthMismatch`] on a moved total,
    /// [`AuditError::HeadMismatch`] on a moved head.
    pub fn verify_pinned(&self, head: [u8; 32], total: u64) -> Result<(), AuditError> {
        if self.seen != total {
            return Err(AuditError::LengthMismatch {
                expected: total,
                found: self.seen,
            });
        }
        if self.head_digest() != Some(head) {
            return Err(AuditError::HeadMismatch);
        }
        Ok(())
    }
}

impl Default for AuditVerifier {
    /// Empty checker at the zero anchor.
    fn default() -> Self {
        Self::new()
    }
}

/// Verify borrowed records in order with constant memory.
///
/// Same verdict as [`super::DoorAudit::verify`]: dense sequence from 1, linked
/// `prev` digests and recomputed seals. A truncated tail still verifies as a
/// prefix, so holders pin with [`verify_pinned_records`].
///
/// Errors: [`AuditError::OrderBroken`] or [`AuditError::ChainBroken`] at the
/// first failing sequence number.
pub fn verify_records<'a>(
    records: impl IntoIterator<Item = &'a CallRecord>,
) -> Result<(), AuditError> {
    let mut checker = AuditVerifier::new();
    for record in records {
        checker.feed(record)?;
    }
    Ok(())
}

/// Verify plus pin over borrowed records: the head digest and the total length
/// must match the holder's values.
///
/// Errors: the [`verify_records`] failures plus [`AuditError::LengthMismatch`]
/// on a moved total and [`AuditError::HeadMismatch`] on a moved head.
pub fn verify_pinned_records<'a>(
    records: impl IntoIterator<Item = &'a CallRecord>,
    head: [u8; 32],
    total: u64,
) -> Result<(), AuditError> {
    let mut checker = AuditVerifier::new();
    for record in records {
        checker.feed(record)?;
    }
    checker.verify_pinned(head, total)
}

/// Per-record verify budget, shared with the append path: half a millisecond.
pub const VERIFY_BUDGET_NS: u64 = DOOR_AUDIT_BUDGET_NS;

#[cfg(test)]
mod tests {
    use super::super::{AuditOutcome, DoorAudit};
    use super::*;

    /// Six calls across the outcome and detail space, mirroring the writer's
    /// own fixture so both consumers assert the same tamper vector.
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
    fn clean_logs_and_regens_pass_through_the_stream() {
        let mut log = DoorAudit::new();
        verify_records(log.iter()).expect("empty log verifies");
        fill(&mut log, 7);
        verify_records(log.iter()).expect("clean log verifies");
        let head = log.head().expect("head");
        verify_pinned_records(log.iter(), head, 6).expect("pinned head verifies");
        // A regen from the same call sequence seals the same head and passes.
        let mut regen = DoorAudit::new();
        fill(&mut regen, 7);
        assert_eq!(regen.head(), log.head());
        verify_pinned_records(regen.iter(), head, 6).expect("regen verifies");
        // The incremental feed agrees record by record.
        let mut checker = AuditVerifier::new();
        assert_eq!(checker.verified(), 0);
        assert_eq!(checker.head_digest(), None);
        for record in log.iter() {
            checker.feed(record).expect("ordered feed advances");
        }
        assert_eq!(checker.verified(), 6);
        assert_eq!(checker.head_digest(), Some(head));
        checker.verify_pinned(head, 6).expect("fed pin verifies");
    }

    /// 100 sessions, one tamper each, rotating drop/edit/reorder/splice/
    /// tail-cut through the streaming checker. Every tamper must fail loudly:
    /// 100 attempted, 100 detected, 20 per class.
    #[test]
    fn hundred_sessions_detect_every_tamper_class_through_the_stream() {
        const SESSIONS: usize = 100;
        let mut detected = 0usize;
        let mut per_class = [0usize; 5];
        for session in 0..SESSIONS {
            let mut log = DoorAudit::new();
            fill(&mut log, session as u64);
            verify_records(log.iter()).expect("untampered session verifies");
            let head = log.head().expect("head");
            let total = log.total_appended();
            // Holders verify borrowed records, so tamper on an owned copy.
            let mut tampered: Vec<CallRecord> = log.iter().cloned().collect();
            match session % 5 {
                0 => {
                    tampered.remove(2);
                    if verify_records(tampered.iter()).is_err() {
                        detected += 1;
                        per_class[0] += 1;
                    }
                }
                1 => {
                    tampered[3].detail ^= 0xFF;
                    if verify_records(tampered.iter()).is_err() {
                        detected += 1;
                        per_class[1] += 1;
                    }
                }
                2 => {
                    tampered.swap(1, 4);
                    if verify_records(tampered.iter()).is_err() {
                        detected += 1;
                        per_class[2] += 1;
                    }
                }
                3 => {
                    let mut foreign = DoorAudit::new();
                    fill(&mut foreign, 10_000 + session as u64);
                    let suffix: Vec<CallRecord> = foreign.iter().cloned().collect();
                    tampered[4] = suffix[4].clone();
                    tampered[5] = suffix[5].clone();
                    if verify_records(tampered.iter()).is_err() {
                        detected += 1;
                        per_class[3] += 1;
                    }
                }
                _ => {
                    tampered.pop();
                    if verify_pinned_records(tampered.iter(), head, total).is_err() {
                        detected += 1;
                        per_class[4] += 1;
                    }
                }
            }
        }
        eprintln!(
            "door-audit-verify-stream sessions={SESSIONS} detected={detected} \
             drop={} edit={} reorder={} splice={} tail={}",
            per_class[0], per_class[1], per_class[2], per_class[3], per_class[4]
        );
        assert_eq!(detected, SESSIONS, "every tamper is detected");
        assert_eq!(per_class.iter().sum::<usize>(), SESSIONS);
        assert!(per_class.iter().all(|count| *count == SESSIONS / 5));
    }

    #[test]
    fn streaming_agrees_with_direct_verify_on_every_class() {
        let mut log = DoorAudit::new();
        fill(&mut log, 7);
        let head = log.head().expect("head");
        let total = log.total_appended();
        assert!(log.verify().is_ok());
        assert!(verify_records(log.iter()).is_ok());
        // Edit names its sequence on both paths.
        let mut edited: Vec<CallRecord> = log.iter().cloned().collect();
        edited[1].tool = "renamed".to_owned();
        assert!(matches!(
            verify_records(edited.iter()),
            Err(AuditError::ChainBroken { seq: 2 })
        ));
        // Drop breaks order at the next sequence.
        let mut dropped: Vec<CallRecord> = log.iter().cloned().collect();
        dropped.remove(2);
        assert!(matches!(
            verify_records(dropped.iter()),
            Err(AuditError::OrderBroken { seq: 4 })
        ));
        // Resealed suffix moves the head, so the old pin fails loudly.
        let mut resealed: Vec<CallRecord> = log.iter().cloned().collect();
        resealed[5].detail = resealed[5].detail.wrapping_add(1);
        let bytes = super::super::entry_bytes(
            resealed[5].session,
            &resealed[5].tool,
            resealed[5].revision,
            resealed[5].outcome,
            resealed[5].detail,
            resealed[5].seq,
        );
        let entry = super::super::digest32(&[&bytes]);
        resealed[5].chain = super::super::link(&resealed[5].prev, &entry, 6);
        assert!(matches!(
            verify_pinned_records(resealed.iter(), head, total),
            Err(AuditError::HeadMismatch)
        ));
    }

    #[test]
    fn checker_state_stays_bounded_independent_of_length() {
        assert!(
            std::mem::size_of::<AuditVerifier>() <= 64,
            "checker holds two counters plus one digest, got {} bytes",
            std::mem::size_of::<AuditVerifier>()
        );
        let mut log = DoorAudit::new();
        for i in 0..2_000 {
            log.append(7, "scene.read", i, AuditOutcome::Ok, i)
                .expect("bounded append chains");
        }
        verify_records(log.iter()).expect("long log verifies in a stream");
        let mut checker = AuditVerifier::new();
        for record in log.iter() {
            checker.feed(record).expect("ordered feed advances");
        }
        assert_eq!(checker.verified(), 2_000);
        assert_eq!(checker.head_digest(), log.head());
        assert!(
            std::mem::size_of_val(&checker) <= 64,
            "fed checker stays bounded"
        );
    }

    #[test]
    fn verify_cost_stays_within_budget() {
        const N: u64 = 2_000;
        let mut log = DoorAudit::new();
        for i in 0..N {
            log.append(7, "scene.read", i, AuditOutcome::Ok, i)
                .expect("budget append chains");
        }
        let start = std::time::Instant::now();
        verify_records(log.iter()).expect("measured log verifies");
        // Integer mean only: floating point never enters this module.
        let mean = start.elapsed().as_nanos() / u128::from(N);
        let budget = u128::from(VERIFY_BUDGET_NS);
        eprintln!("door-audit-verify mean={mean}ns over {N} records (budget {budget}ns)");
        assert!(mean < budget, "verify mean {mean}ns exceeds {budget}ns");
    }
}
