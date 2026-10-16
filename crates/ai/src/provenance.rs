//! provenance: one record per living-world and brief-chain write.
//!
//! Consumer: ludus NPC wiring and the brief chain record every applied
//! write here, so each edit traces to its tick, sequence, source and
//! request id, undoes against its digest pair, and survives save/load.
//! Data owner: [`ProvenanceLog`] owns [`ProvenanceRecord`]s only; minds,
//! stores and scenes keep their own state and hand digests over.
//! Units: ticks are fixed steps, sequences are log-monotonic from 0.
//! Determinism: insertion order plus FNV-1a only, no clock, no hash order.
//! Errors: [`ProvenanceError`]; invalid bytes or text never panic.
//! Budget: at most [`MAX_RECORDS`] records; record and undo are O(1)
//! amortized; bytes are versioned little-endian.

use super::cursor::{ByteCursor, ShortRead};
use super::mind::{ActionSource, MindAction, NpcId, MAX_TEXT};
use engine_core::hash_bytes;

/// Save-envelope layout version, little-endian `u32` first. Version 1
/// stays readable; unknown versions refuse loudly so a future layout
/// never parses as this one.
pub const PROVENANCE_BYTES_VERSION: u32 = 1;
/// Most records one log keeps. Past this [`ProvenanceLog::record`]
/// refuses with [`ProvenanceError::Full`] so no write ever lands
/// without its record; callers archive via [`ProvenanceLog::bytes`].
pub const MAX_RECORDS: usize = 16_384;

/// Who made the write. NPC minds use [`Writer::Npc`]; brief stages and
/// other out-of-crate writers use [`Writer::External`] with the id they
/// own (the log never interprets it), so no foreign type is mirrored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Writer {
    /// A living-world mind write for this NPC.
    Npc(NpcId),
    /// A brief-chain or other external write, caller-owned id.
    External(u64),
}

/// One witnessed write: where it was observed, where it was recorded,
/// what made it, and the digest pair its undo restores.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProvenanceRecord {
    /// Fixed tick the change was observed at (action stamp or ask tick).
    pub observed_tick: u64,
    /// Log-monotonic recorded sequence, assigned at record time.
    pub seq: u64,
    /// Scripted fallback or validated provider output.
    pub source: ActionSource,
    /// Ask or brief-envelope id that produced this change.
    pub request: u64,
    /// Mind or external writer that owns the change.
    pub writer: Writer,
    /// Guarded-state checkpoint before the change; undo restores this.
    pub digest_before: u64,
    /// Guarded-state checkpoint after the change.
    pub digest_after: u64,
    /// Bounded trace label (`1..=MAX_TEXT` chars): action line or stage.
    pub detail: String,
}

/// Typed provenance failures. Refusals record nothing and move no seq.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProvenanceError {
    /// Detail empty or whitespace-only.
    EmptyDetail,
    /// Detail longer than [`MAX_TEXT`] characters.
    DetailTooLong,
    /// Log holds [`MAX_RECORDS`]; archive before recording more.
    Full,
    /// Bytes truncated, mistyped, mis-ordered or trailing.
    Corrupt,
    /// Envelope version is not [`PROVENANCE_BYTES_VERSION`].
    UnknownVersion,
}

impl core::fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            ProvenanceError::EmptyDetail => "provenance: empty detail",
            ProvenanceError::DetailTooLong => "provenance: detail over cap",
            ProvenanceError::Full => "provenance: log full",
            ProvenanceError::Corrupt => "provenance: corrupt bytes",
            ProvenanceError::UnknownVersion => "provenance: unknown version",
        };
        f.write_str(label)
    }
}

impl std::error::Error for ProvenanceError {}

/// A short read of log bytes is corruption.
impl From<ShortRead> for ProvenanceError {
    fn from(_: ShortRead) -> Self {
        ProvenanceError::Corrupt
    }
}

fn check_detail(detail: &str) -> Result<(), ProvenanceError> {
    if detail.trim().is_empty() {
        return Err(ProvenanceError::EmptyDetail);
    }
    if detail.chars().count() > MAX_TEXT {
        return Err(ProvenanceError::DetailTooLong);
    }
    Ok(())
}

fn source_byte(source: ActionSource) -> u8 {
    match source {
        ActionSource::Scripted => 0,
        ActionSource::Provider => 1,
    }
}

fn source_of(byte: u8) -> Result<ActionSource, ProvenanceError> {
    match byte {
        0 => Ok(ActionSource::Scripted),
        1 => Ok(ActionSource::Provider),
        _ => Err(ProvenanceError::Corrupt),
    }
}

/// One write offered for recording: every record field except `seq`,
/// which the log assigns. Built by [`PendingRecord::mind_action`] for
/// living-world writes or [`PendingRecord::external`] for brief-chain
/// writes, then handed to [`ProvenanceLog::record`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PendingRecord<'a> {
    /// Fixed tick the change was observed at.
    pub observed_tick: u64,
    /// Scripted fallback or validated provider output.
    pub source: ActionSource,
    /// Ask or brief-envelope id that produced this change.
    pub request: u64,
    /// Mind or external writer that owns the change.
    pub writer: Writer,
    /// Guarded-state checkpoint before the change.
    pub digest_before: u64,
    /// Guarded-state checkpoint after the change.
    pub digest_after: u64,
    /// Bounded trace label (`1..=MAX_TEXT` chars).
    pub detail: &'a str,
}

impl<'a> PendingRecord<'a> {
    /// Pending record for one accepted mind action: the living-world
    /// door. Tick, source, request, writer and label come from the
    /// action itself; digests come from the guarded state around apply.
    pub fn mind_action(action: &'a MindAction, digest_before: u64, digest_after: u64) -> Self {
        Self {
            observed_tick: action.tick_apply,
            source: action.source,
            request: action.request,
            writer: Writer::Npc(action.npc),
            digest_before,
            digest_after,
            detail: &action.text,
        }
    }

    /// Pending record for one brief-stage write: the brief-chain door.
    /// The caller owns `writer_id` (envelope or stage id) and the digest
    /// pair; the log checks bounds and order only, never foreign state.
    pub fn external(
        observed_tick: u64,
        source: ActionSource,
        request: u64,
        writer_id: u64,
        digest_before: u64,
        digest_after: u64,
        detail: &'a str,
    ) -> Self {
        Self {
            observed_tick,
            source,
            request,
            writer: Writer::External(writer_id),
            digest_before,
            digest_after,
            detail,
        }
    }
}

/// Ordered record of every living-world and brief-chain write. Sequences
/// are dense (`seq` equals the record index), so undo pops a suffix and
/// save/load verifies order instead of trusting it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ProvenanceLog {
    records: Vec<ProvenanceRecord>,
    next_seq: u64,
}

impl ProvenanceLog {
    /// Empty log.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            next_seq: 0,
        }
    }

    /// Records held.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Records in recorded order.
    pub fn records(&self) -> &[ProvenanceRecord] {
        &self.records
    }

    /// Records for one request id, in recorded order.
    pub fn iter_for(&self, request: u64) -> impl Iterator<Item = &ProvenanceRecord> {
        self.records.iter().filter(move |r| r.request == request)
    }

    /// Record one offered write. Assigns the next sequence and returns
    /// it; a full log or a bad detail records nothing and moves no
    /// sequence, so no write ever lands without its record.
    pub fn record(&mut self, pending: PendingRecord<'_>) -> Result<u64, ProvenanceError> {
        check_detail(pending.detail)?;
        if self.records.len() >= MAX_RECORDS {
            return Err(ProvenanceError::Full);
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.records.push(ProvenanceRecord {
            observed_tick: pending.observed_tick,
            seq,
            source: pending.source,
            request: pending.request,
            writer: pending.writer,
            digest_before: pending.digest_before,
            digest_after: pending.digest_after,
            detail: pending.detail.to_owned(),
        });
        Ok(seq)
    }

    /// Undo back to `seq`: pop every record with `seq` or newer and hand
    /// them back newest-first, so the caller can unwind in apply order.
    /// `seq == 0` clears the log; past-the-tip values pop nothing.
    pub fn undo_to_seq(&mut self, seq: u64) -> Vec<ProvenanceRecord> {
        let mut undone = Vec::new();
        while self.records.last().is_some_and(|r| r.seq >= seq) {
            if let Some(record) = self.records.pop() {
                undone.push(record);
            } else {
                break;
            }
        }
        self.next_seq = self.records.len() as u64;
        undone
    }

    /// Deterministic bytes: version, count, then per record (tick, seq,
    /// source, request, writer tag plus id, digest pair, byte length plus
    /// UTF-8 detail) in recorded order.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PROVENANCE_BYTES_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.records.len() as u64).to_le_bytes());
        for record in &self.records {
            out.extend_from_slice(&record.observed_tick.to_le_bytes());
            out.extend_from_slice(&record.seq.to_le_bytes());
            out.push(source_byte(record.source));
            out.extend_from_slice(&record.request.to_le_bytes());
            match record.writer {
                Writer::Npc(npc) => {
                    out.push(0);
                    out.extend_from_slice(&npc.0.to_le_bytes());
                }
                Writer::External(id) => {
                    out.push(1);
                    out.extend_from_slice(&id.to_le_bytes());
                }
            }
            out.extend_from_slice(&record.digest_before.to_le_bytes());
            out.extend_from_slice(&record.digest_after.to_le_bytes());
            out.extend_from_slice(&(record.detail.len() as u64).to_le_bytes());
            out.extend_from_slice(record.detail.as_bytes());
        }
        out
    }

    /// Rebuild a log from [`ProvenanceLog::bytes`]: validates the version,
    /// every field and the dense sequence order. Corrupt or future bytes
    /// refuse loudly and build nothing.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProvenanceError> {
        let mut cursor = ByteCursor::new(bytes);
        if bytes.len() < 12 {
            return Err(ProvenanceError::Corrupt);
        }
        let version = cursor.take_u32()?;
        if version != PROVENANCE_BYTES_VERSION {
            return Err(ProvenanceError::UnknownVersion);
        }
        let count = cursor.take_u64()?;
        if count > MAX_RECORDS as u64 {
            return Err(ProvenanceError::Corrupt);
        }
        let mut log = ProvenanceLog::new();
        for index in 0..count {
            let observed_tick = cursor.take_u64()?;
            let seq = cursor.take_u64()?;
            // Dense order is the writer invariant; anything else is damage.
            if seq != index {
                return Err(ProvenanceError::Corrupt);
            }
            let source = source_of(cursor.take_u8()?)?;
            let request = cursor.take_u64()?;
            let writer = match cursor.take_u8()? {
                0 => Writer::Npc(NpcId(cursor.take_u32()?)),
                1 => Writer::External(cursor.take_u64()?),
                _ => return Err(ProvenanceError::Corrupt),
            };
            let digest_before = cursor.take_u64()?;
            let digest_after = cursor.take_u64()?;
            let detail_len = cursor.take_u64()?;
            if detail_len > bytes.len() as u64 {
                return Err(ProvenanceError::Corrupt);
            }
            let detail_raw = cursor.take(detail_len as usize)?;
            let detail = core::str::from_utf8(detail_raw).map_err(|_| ProvenanceError::Corrupt)?;
            if check_detail(detail).is_err() {
                return Err(ProvenanceError::Corrupt);
            }
            log.records.push(ProvenanceRecord {
                observed_tick,
                seq,
                source,
                request,
                writer,
                digest_before,
                digest_after,
                detail: detail.to_owned(),
            });
            log.next_seq = log.next_seq.saturating_add(1);
        }
        if !cursor.is_done() {
            return Err(ProvenanceError::Corrupt);
        }
        Ok(log)
    }

    /// Deterministic FNV-1a checkpoint over [`ProvenanceLog::bytes`]. One
    /// write sequence always yields one checkpoint.
    pub fn checkpoint(&self) -> u64 {
        hash_bytes(&self.bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::super::memory::MemoryStore;
    use super::super::mind::{
        ActionLog, ActionSource, MemoryKind, Mind, MindRequest, MindRuntime, MockAdapter, NpcId,
        Privacy, ScriptStep,
    };
    use super::*;

    fn npc(id: u32) -> NpcId {
        NpcId(id)
    }

    fn ask_req(id: u64, npc_id: u32) -> MindRequest {
        MindRequest {
            id,
            npc: npc(npc_id),
            issued_tick: 10,
            deadline_tick: 20,
            budget_ms: 1_000,
            base_revision: 1,
            privacy: Privacy::LocalOnly,
        }
    }

    fn write_log() -> ProvenanceLog {
        let mut log = ProvenanceLog::new();
        log.record(PendingRecord {
            observed_tick: 10,
            source: ActionSource::Provider,
            request: 1,
            writer: Writer::Npc(npc(3)),
            digest_before: 100,
            digest_after: 101,
            detail: "npc3 recalls the gate",
        })
        .expect("fixture records");
        log.record(PendingRecord::external(
            11,
            ActionSource::Scripted,
            7,
            900,
            200,
            201,
            "level draft placed",
        ))
        .expect("fixture records");
        log
    }

    #[test]
    fn every_lw_write_carries_a_record() {
        // Mind stream writes: one record per observation.
        let mut mind = Mind::new(npc(3), 11);
        let mut log = ProvenanceLog::new();
        let mut digest = 0u64;
        for (tick, line) in [
            (1u64, "a stranger enters the ludus"),
            (2, "the crowd chants at dusk"),
        ] {
            mind.observe(tick, MemoryKind::Seen, line, 800)
                .expect("fixture observes");
            let after = digest.wrapping_add(tick);
            log.record(PendingRecord {
                observed_tick: tick,
                source: ActionSource::Scripted,
                request: 0,
                writer: Writer::Npc(npc(3)),
                digest_before: digest,
                digest_after: after,
                detail: line,
            })
            .expect("observation records");
            digest = after;
        }
        // Ask writes: one record per recorded action through the mind door.
        let mut runtime = MindRuntime::new();
        let mut adapter = MockAdapter::new(vec![ScriptStep::Succeed]);
        let outcome = runtime
            .ask(&mind, &ask_req(1, 3), &mut adapter)
            .expect("fixture asks");
        assert_eq!(outcome.actions.len(), 1);
        let before = runtime.recorded().checkpoint();
        for action in &outcome.actions {
            log.record(PendingRecord::mind_action(
                action,
                before,
                before.wrapping_add(7),
            ))
            .expect("action records");
        }
        // Memory store writes: one record per observation through the door.
        let mut store = MemoryStore::new();
        store
            .observe(npc(3), 3, MemoryKind::Heard, "the gate holds", 620)
            .expect("fixture observes");
        log.record(PendingRecord::external(
            3,
            ActionSource::Scripted,
            2,
            u64::from(npc(3).0),
            before,
            store.checkpoint(),
            "the gate holds",
        ))
        .expect("store write records");
        // Every write traced: 2 stream + 1 ask action + 1 store write.
        assert_eq!(log.len(), 4);
        assert_eq!(log.iter_for(0).count(), 2);
        assert_eq!(log.iter_for(1).count(), 1);
        assert_eq!(log.iter_for(2).count(), 1);
        let asked: Vec<&ProvenanceRecord> = log.iter_for(1).collect();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].writer, Writer::Npc(npc(3)));
        assert_eq!(asked[0].source, ActionSource::Provider);
        assert_eq!(asked[0].observed_tick, outcome.actions[0].tick_apply);
        assert_eq!(asked[0].detail, outcome.actions[0].text);
    }

    #[test]
    fn same_ask_twice_keeps_distinct_sequences() {
        let mut log = ProvenanceLog::new();
        let first = log
            .record(PendingRecord {
                observed_tick: 5,
                source: ActionSource::Provider,
                request: 9,
                writer: Writer::Npc(npc(1)),
                digest_before: 1,
                digest_after: 2,
                detail: "first apply",
            })
            .expect("fixture records");
        let second = log
            .record(PendingRecord {
                observed_tick: 5,
                source: ActionSource::Provider,
                request: 9,
                writer: Writer::Npc(npc(1)),
                digest_before: 2,
                digest_after: 3,
                detail: "second apply",
            })
            .expect("fixture records");
        assert_ne!(first, second, "one ask with two actions keeps two seqs");
        assert_eq!(log.iter_for(9).count(), 2);
    }

    #[test]
    fn undo_restores_digest() {
        // Guarded state: three accepted actions with a checkpoint per prefix.
        let mut guarded = ActionLog::new();
        let mut mind = Mind::new(npc(5), 3);
        mind.observe(1, MemoryKind::Seen, "the sand is raked", 500)
            .expect("fixture observes");
        let mut runtime = MindRuntime::new();
        let mut adapter = MockAdapter::new(vec![ScriptStep::Succeed]);
        let mut log = ProvenanceLog::new();
        let mut digests = vec![guarded.checkpoint()];
        for id in [11u64, 12, 13] {
            let outcome = runtime
                .ask(&mind, &ask_req(id, 5), &mut adapter)
                .expect("fixture asks");
            for action in &outcome.actions {
                guarded.push(action.clone()).expect("fixture pushes");
            }
            let after = guarded.checkpoint();
            for action in &outcome.actions {
                log.record(PendingRecord::mind_action(
                    action,
                    digests[digests.len() - 1],
                    after,
                ))
                .expect("action records");
            }
            digests.push(after);
        }
        assert_eq!(log.len(), 3);
        // Undo one: pop the record, rebuild the guarded prefix, match digest.
        let mut undone_vec = log.undo_to_seq(2);
        assert_eq!(undone_vec.len(), 1);
        let undone = undone_vec.pop().expect("undo pops one");
        assert_eq!(undone.request, 13);
        let (_, checkpoint) =
            MindRuntime::replay_offline(&guarded.entries()[..guarded.entries().len() - 1])
                .expect("prefix replays");
        assert_eq!(checkpoint, undone.digest_before);
        assert_eq!(checkpoint, digests[2]);
        // Undo to seq 1: the seq-1 record comes back alone.
        let wound = log.undo_to_seq(1);
        assert_eq!(wound.len(), 1);
        assert_eq!(wound[0].seq, 1);
        let (_, checkpoint) =
            MindRuntime::replay_offline(&guarded.entries()[..1]).expect("prefix replays");
        assert_eq!(checkpoint, wound[0].digest_before);
        assert_eq!(checkpoint, digests[1]);
        assert_eq!(log.len(), 1);
        // Undo to zero clears; empty logs undo to nothing.
        assert_eq!(log.undo_to_seq(0).len(), 1);
        assert!(log.is_empty());
        assert!(log.undo_to_seq(99).is_empty());
        // Multi-pop on a fresh log comes back newest-first.
        for seq in 0..3u64 {
            log.record(PendingRecord::external(
                seq,
                ActionSource::Scripted,
                4,
                8,
                seq,
                seq + 1,
                "replay",
            ))
            .expect("fixture records");
        }
        let wound = log.undo_to_seq(1);
        assert_eq!(wound.len(), 2);
        assert_eq!((wound[0].seq, wound[1].seq), (2, 1));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn save_load_roundtrip_identical_x2() {
        let first = write_log();
        let second = write_log();
        let bytes_a = first.bytes();
        let bytes_b = second.bytes();
        assert_eq!(bytes_a, bytes_b, "same writes give same bytes");
        assert_eq!(first.checkpoint(), second.checkpoint());
        let loaded = ProvenanceLog::from_bytes(&bytes_a).expect("round-trips");
        assert_eq!(loaded, first);
        assert_eq!(loaded.checkpoint(), first.checkpoint());
        assert_eq!(loaded.records()[0].detail, "npc3 recalls the gate");
        assert_eq!(loaded.records()[1].writer, Writer::External(900));
        // New records keep the loaded sequence, never restart it.
        let mut grown = loaded;
        let seq = grown
            .record(PendingRecord {
                observed_tick: 12,
                source: ActionSource::Scripted,
                request: 8,
                writer: Writer::Npc(npc(4)),
                digest_before: 1,
                digest_after: 2,
                detail: "late line",
            })
            .expect("grows after load");
        assert_eq!(seq, 2);
        println!(
            "provenance-bytes len={} checkpoint={:016x} records={}",
            bytes_a.len(),
            first.checkpoint(),
            first.len()
        );
    }

    #[test]
    fn corrupt_bytes_refuse() {
        let good = write_log().bytes();
        // Every truncation refuses.
        for cut in [0usize, 5, 11, 12, 13, 20, 40, 80] {
            let cut = cut.min(good.len().saturating_sub(1));
            assert_eq!(
                ProvenanceLog::from_bytes(&good[..cut]),
                Err(ProvenanceError::Corrupt),
                "truncation at {cut} refuses"
            );
        }
        // Unknown version refuses loudly, not as generic corruption.
        let mut versioned = good.clone();
        versioned[0] = versioned[0].wrapping_add(1);
        if versioned[0..4] == good[0..4] {
            versioned[1] = versioned[1].wrapping_add(1);
        }
        assert_eq!(
            ProvenanceLog::from_bytes(&versioned),
            Err(ProvenanceError::UnknownVersion)
        );
        // Bad source discriminant refuses.
        let mut bad_source = good.clone();
        bad_source[12 + 8 + 8] = 9;
        assert_eq!(
            ProvenanceLog::from_bytes(&bad_source),
            Err(ProvenanceError::Corrupt)
        );
        // Bad writer tag refuses.
        let mut bad_writer = good.clone();
        bad_writer[12 + 8 + 8 + 1 + 8] = 9;
        assert_eq!(
            ProvenanceLog::from_bytes(&bad_writer),
            Err(ProvenanceError::Corrupt)
        );
        // Trailing byte refuses.
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            ProvenanceLog::from_bytes(&trailing),
            Err(ProvenanceError::Corrupt)
        );
        // Swapped dense sequences refuse.
        let mut swapped = good.clone();
        let first_seq = 12 + 8;
        let second_start = 12 + (8 + 8 + 1 + 8 + 1 + 4 + 8 + 8 + 8 + "npc3 recalls the gate".len());
        let second_seq = second_start + 8;
        if second_seq + 8 <= swapped.len() {
            swapped.copy_within(first_seq..first_seq + 8, second_seq);
            assert_eq!(
                ProvenanceLog::from_bytes(&swapped),
                Err(ProvenanceError::Corrupt),
                "broken dense order refuses"
            );
        }
        // Bad UTF-8 detail refuses.
        let mut bad_text = good;
        if let Some(last) = bad_text.last_mut() {
            *last = 0xFF;
        }
        assert_eq!(
            ProvenanceLog::from_bytes(&bad_text),
            Err(ProvenanceError::Corrupt)
        );
    }

    #[test]
    fn rejects_bad_detail_and_full_log() {
        let mut log = ProvenanceLog::new();
        fn bad(detail: &str) -> PendingRecord<'_> {
            PendingRecord {
                observed_tick: 1,
                source: ActionSource::Scripted,
                request: 1,
                writer: Writer::Npc(NpcId(1)),
                digest_before: 0,
                digest_after: 1,
                detail,
            }
        }
        assert_eq!(log.record(bad("")), Err(ProvenanceError::EmptyDetail));
        assert_eq!(log.record(bad("   ")), Err(ProvenanceError::EmptyDetail));
        let long: String = "x".repeat(MAX_TEXT + 1);
        assert_eq!(log.record(bad(&long)), Err(ProvenanceError::DetailTooLong));
        assert!(log.is_empty(), "refusals record nothing");
        for index in 0..MAX_RECORDS {
            log.record(PendingRecord::external(
                index as u64,
                ActionSource::Scripted,
                1,
                3,
                index as u64,
                index as u64 + 1,
                "fill",
            ))
            .expect("fills to cap");
        }
        assert_eq!(log.len(), MAX_RECORDS);
        assert_eq!(
            log.record(PendingRecord::external(
                0,
                ActionSource::Scripted,
                1,
                3,
                0,
                1,
                "one more"
            )),
            Err(ProvenanceError::Full)
        );
        assert_eq!(log.len(), MAX_RECORDS, "full log keeps nothing new");
    }

    #[test]
    fn soak_10k_holds_cap_and_replays() {
        let build = || {
            let mut log = ProvenanceLog::new();
            for index in 0..10_000u64 {
                let writer = if index % 3 == 0 {
                    Writer::External(index)
                } else {
                    Writer::Npc(npc((index % 128) as u32))
                };
                let source = if index % 2 == 0 {
                    ActionSource::Provider
                } else {
                    ActionSource::Scripted
                };
                log.record(PendingRecord {
                    observed_tick: index % 500,
                    source,
                    request: index % 64,
                    writer,
                    digest_before: index,
                    digest_after: index + 1,
                    detail: &format!("write {index}"),
                })
                .expect("soak records");
            }
            log
        };
        let first = build();
        let second = build();
        assert_eq!(first.len(), 10_000);
        assert!(first.len() <= MAX_RECORDS);
        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.checkpoint(), second.checkpoint());
        let counted: usize = (0..64u64).map(|id| first.iter_for(id).count()).sum();
        assert_eq!(counted, first.len(), "per-request counts cover the log");
        let loaded = ProvenanceLog::from_bytes(&first.bytes()).expect("soak round-trips");
        assert_eq!(loaded, first);
        println!(
            "provenance-soak records={} bytes={} checkpoint={:016x}",
            first.len(),
            first.bytes().len(),
            first.checkpoint()
        );
    }
}
