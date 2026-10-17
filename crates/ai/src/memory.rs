//! memory: bounded per-NPC memory store over NPC minds.
//!
//! Consumer: ludus NPCs pull [`Recall`] lines and the eval harness reads
//! contradiction counts; the store ingests the [`Mind`] source side via
//! [`MemoryStore::remember_mind`]. Data owner: [`MemoryStore`] owns distilled
//! [`StoredMemory`] records per [`NpcId`]. Units: ticks are fixed steps,
//! salience is q8 (`0..=255`, see [`Salience`]). Determinism: integer math,
//! insertion order and FNV-1a bytes only; providers never run here.
//! Errors: [`MindError`] for ingest, [`MemorySaveError`] for save bytes.
//! Budget: [`MAX_NPCS`] NPCs by [`MAX_STORED_PER_NPC`]
//! records; recall fills caller buffers; save/load are one-shot byte walks,
//! never per-tick work. Save: [`MemoryStore::save_npc`] writes one versioned
//! per-NPC envelope over the memory bytes, [`MemoryStore::save_all_with_provenance`]
//! pairs the whole store with its [`ProvenanceLog`], and
//! [`MemoryStore::audit_caps`] fails past the caps.

use super::cursor::{ByteCursor, ShortRead};
use super::mind::{check_text, MemoryKind, Mind, MindError, NpcId, MAX_EVENTS, MAX_TEXT};
use super::provenance::{ProvenanceError, ProvenanceLog};
use engine_core::hash_bytes;

/// Most NPCs one store tracks; covers the 100-NPC scale gate with headroom.
pub const MAX_NPCS: usize = 128;
/// Most distilled records kept per NPC; the 10k-event soak stays lossless at
/// 100 NPCs times 100 events, and oldest-first eviction holds past that.
pub const MAX_STORED_PER_NPC: usize = 128;
/// Most hits one [`Recall`] carries for dialogue pulls.
pub const MAX_RECALL: usize = 8;
/// [`MemoryStore::memory_bytes`] layout version, little-endian `u32` first.
pub const MEMORY_BYTES_VERSION: u32 = 1;
/// Versioned save-envelope version, little-endian `u32` first. v1 record
/// payloads reuse the [`MemoryStore::memory_bytes`] record layout, so v1
/// stays readable; unknown versions refuse loudly with
/// [`MemorySaveError::UnknownVersion`].
pub const MEMORY_SAVE_VERSION: u32 = 1;
/// Envelope tag for one NPC side; the wrong tag refuses as
/// [`MemorySaveError::Corrupt`], never as a version hit.
const SAVE_TAG_NPC: u8 = 0;
/// Envelope tag for the whole store plus its [`ProvenanceLog`].
const SAVE_TAG_ALL: u8 = 1;
/// Byte ceiling for one decoded text (`MAX_TEXT` chars of UTF-8); longer
/// lengths refuse before any allocation.
const MAX_SAVE_TEXT_BYTES: u64 = (MAX_TEXT as u64) * 4;

/// Integer salience in q8: an 8-bit quantization (`0..=255`) of the mind
/// scale (`0..=1000`). Scoring stays integer-only, so the store ranks in the
/// same shape as [`Mind`] recall without any float or embedding relevance;
/// relevance arrives via ingest order (see [`MemoryStore::remember_mind`]).
/// This mirrors the generative-agents importance-times-recency shape
/// (Park et al. 2304.03442, ideas only): `score = q * 1024 / (1 + age)`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Salience(u8);

impl Salience {
    /// Lowest salience; never recalled above anything remembered.
    pub const MIN: Salience = Salience(0);
    /// Highest salience; a `1000` on the mind scale.
    pub const MAX: Salience = Salience(u8::MAX);

    /// Raw q8 value.
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Quantize a mind-scale salience; inputs above 1000 saturate at 255.
    /// `q = round(s * 255 / 1000)` in `u64` math, so no wrap is possible.
    pub fn from_mind(salience: u32) -> Self {
        let clamped = salience.min(1000);
        let q = (u64::from(clamped) * 255 + 500) / 1000;
        Self(q as u8)
    }

    /// Approximate inverse for mood bands and diagnostics; within 2 of the
    /// input that produced this value via [`Salience::from_mind`].
    pub const fn to_mind(self) -> u32 {
        (self.0 as u32 * 1000 + 127) / 255
    }

    /// Recall ranking score: quantized salience weighted by recency, integer
    /// only. Older ticks saturate, never wrap; same shape as mind recall.
    fn score(self, now_tick: u64, at_tick: u64) -> u64 {
        let age = now_tick.saturating_sub(at_tick);
        let decay = age.saturating_add(1);
        u64::from(self.0).saturating_mul(1024) / decay.max(1)
    }
}

/// One distilled memory: tick-stamped observation plus its ingest sequence.
/// `seq` is store-monotonic and decides every tie, so ranking is total.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StoredMemory {
    /// Fixed tick the event was observed at.
    pub tick: u64,
    /// NPC that owns this memory.
    pub npc: NpcId,
    /// What kind of memory this is.
    pub kind: MemoryKind,
    /// Bounded recollection text (`1..=MAX_TEXT` chars).
    pub text: String,
    /// Quantized importance.
    pub salience: Salience,
    /// Store-monotonic ingest sequence; older entries evict first.
    pub seq: u64,
}

/// One ranked recall hit into an NPC's records.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecallHit {
    /// Index into that NPC's record list in store order.
    pub index: usize,
    /// Integer rank score at the recall tick.
    pub score: u64,
    /// Recollection text.
    pub text: String,
}

/// Owned ranked answer for one NPC at one tick: the NPC pull shape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Recall {
    /// NPC recalled.
    pub npc: NpcId,
    /// Tick the ranking was computed at.
    pub tick: u64,
    /// Rank-ordered hits, at most [`MAX_RECALL`].
    pub hits: Vec<RecallHit>,
}

impl Recall {
    /// Hit count.
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// True when nothing ranked (unknown NPC or empty store side).
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }
}

#[derive(Clone, Debug)]
struct NpcSlot {
    npc: NpcId,
    records: Vec<StoredMemory>,
}

fn kind_byte(kind: MemoryKind) -> u8 {
    match kind {
        MemoryKind::Seen => 0,
        MemoryKind::Heard => 1,
        MemoryKind::Did => 2,
        MemoryKind::Felt => 3,
    }
}

/// Bounded memory over many NPC minds. Ingest comes from direct observation
/// or from NPC minds; ranking is integer salience plus recency with sequence
/// tie-breaks; reflection is a deterministic summary, never a provider call,
/// so models stay off the tick and replays need no adapter.
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    slots: Vec<NpcSlot>,
    next_seq: u64,
}

impl MemoryStore {
    /// Empty store.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            next_seq: 0,
        }
    }

    /// NPCs currently tracked.
    pub fn npc_count(&self) -> usize {
        self.slots.len()
    }

    /// Records held across all NPCs.
    pub fn total_records(&self) -> usize {
        self.slots.iter().map(|s| s.records.len()).sum()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|s| s.records.is_empty())
    }

    /// Records held for `npc`; unknown NPCs read 0.
    pub fn len(&self, npc: NpcId) -> usize {
        self.slot(npc).map_or(0, |s| s.records.len())
    }

    fn slot(&self, npc: NpcId) -> Option<&NpcSlot> {
        self.slots.iter().find(|s| s.npc == npc)
    }

    fn slot_mut(&mut self, npc: NpcId) -> Result<&mut NpcSlot, MindError> {
        if self.slots.iter().all(|s| s.npc != npc) {
            if self.slots.len() >= MAX_NPCS {
                return Err(MindError::TooManyActions);
            }
            self.slots.push(NpcSlot {
                npc,
                records: Vec::new(),
            });
        }
        self.slots
            .iter_mut()
            .find(|s| s.npc == npc)
            .ok_or(MindError::TooManyActions)
    }

    fn push(
        &mut self,
        npc: NpcId,
        tick: u64,
        kind: MemoryKind,
        text: &str,
        salience: u32,
    ) -> Result<(), MindError> {
        check_text(text)?;
        if salience > 1000 {
            return Err(MindError::BadSalience);
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let slot = self.slot_mut(npc)?;
        if slot.records.len() >= MAX_STORED_PER_NPC {
            slot.records.remove(0);
        }
        slot.records.push(StoredMemory {
            tick,
            npc,
            kind,
            text: text.to_owned(),
            salience: Salience::from_mind(salience),
            seq,
        });
        Ok(())
    }

    /// Observe one event for `npc`. Validates bounds, then pushes; a full
    /// per-NPC side evicts oldest-first so the cap always holds.
    pub fn observe(
        &mut self,
        npc: NpcId,
        tick: u64,
        kind: MemoryKind,
        text: &str,
        salience: u32,
    ) -> Result<(), MindError> {
        self.push(npc, tick, kind, text, salience)
    }

    /// Distill one mind into the store: ranks its stream with
    /// [`Mind::recall_into`] at `now_tick` and ingests in that rank order,
    /// so the mind's snapshot lines (its top-ranked prefix) land first.
    /// Exact `(tick, kind, text)` replays dedup, making repeated snapshots of
    /// one log byte-identical. Returns newly stored records.
    pub fn remember_mind(&mut self, mind: &Mind, now_tick: u64) -> usize {
        let mut order = [0usize; MAX_EVENTS];
        let found = mind.recall_into(now_tick, &mut order);
        let mut fresh = 0usize;
        for slot in order.iter().take(found) {
            let Some(event) = mind.events().get(*slot) else {
                continue;
            };
            let duplicate = self.slot(event.npc).is_some_and(|s| {
                s.records
                    .iter()
                    .any(|r| r.tick == event.tick && r.kind == event.kind && r.text == event.text)
            });
            if duplicate {
                continue;
            }
            if self
                .push(
                    event.npc,
                    event.tick,
                    event.kind,
                    &event.text,
                    event.salience,
                )
                .is_ok()
            {
                fresh += 1;
            }
        }
        fresh
    }

    /// Fill `out` with indices into `npc`'s records in rank order and return
    /// how many were written. Strict `>` keeps the earlier-ingested record on
    /// ties, so one store plus one tick always recalls one order. Writes into
    /// a caller buffer and holds no lock.
    pub fn recall_into(&self, npc: NpcId, now_tick: u64, out: &mut [usize]) -> usize {
        let Some(slot) = self.slot(npc) else {
            return 0;
        };
        let mut picked = 0usize;
        let slots = out.len().min(slot.records.len());
        let mut used = [false; MAX_STORED_PER_NPC];
        for target in out.iter_mut().take(slots) {
            let mut best: Option<usize> = None;
            let mut best_score = 0u64;
            for (index, record) in slot.records.iter().enumerate() {
                if used[index] {
                    continue;
                }
                let score = record.salience.score(now_tick, record.tick);
                let wins = match best {
                    None => true,
                    Some(_) => score > best_score,
                };
                if wins {
                    best = Some(index);
                    best_score = score;
                }
            }
            match best {
                Some(index) => {
                    used[index] = true;
                    *target = index;
                    picked += 1;
                }
                None => break,
            }
        }
        picked
    }

    /// Owned ranked answer for NPC pulls: at most `limit` hits capped at
    /// [`MAX_RECALL`], rank order. Unknown NPCs recall empty.
    pub fn recall(&self, npc: NpcId, now_tick: u64, limit: usize) -> Recall {
        let mut order = [0usize; MAX_RECALL];
        let found = self.recall_into(npc, now_tick, &mut order);
        let take = found.min(limit);
        let mut hits = Vec::with_capacity(take);
        if let Some(slot) = self.slot(npc) {
            for step in order.iter().take(take) {
                if let Some(record) = slot.records.get(*step) {
                    hits.push(RecallHit {
                        index: *step,
                        score: record.salience.score(now_tick, record.tick),
                        text: record.text.clone(),
                    });
                }
            }
        }
        Recall {
            npc,
            tick: now_tick,
            hits,
        }
    }

    /// Deterministic reflection: summarizes the top recalled record into a
    /// `Felt` entry stamped at `tick`, recorded like any observation (it can
    /// evict the oldest entry at cap). Needs no provider; empty or unknown
    /// sides report [`MindError::NothingToReflect`].
    pub fn reflect(&mut self, npc: NpcId, tick: u64) -> Result<(), MindError> {
        self.reflect_scored(npc, tick, false)
    }

    /// Fixed-cadence reflection, the store twin of [`Mind::reflect_due`]: on a
    /// [`crate::reflect::due`] tick with no `Felt` at or after it, reflect
    /// with capped surprise. Returns whether it reflected.
    pub fn reflect_due(&mut self, npc: NpcId, tick: u64) -> Result<bool, MindError> {
        let reflected_now = self.slot(npc).is_some_and(|s| {
            s.records
                .iter()
                .any(|r| r.kind == MemoryKind::Felt && r.tick >= tick)
        });
        if !crate::reflect::due(tick) || reflected_now {
            return Ok(false);
        }
        match self.reflect_scored(npc, tick, true) {
            Ok(()) => Ok(true),
            Err(MindError::NothingToReflect) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn reflect_scored(&mut self, npc: NpcId, tick: u64, cadence: bool) -> Result<(), MindError> {
        let (text, salience) = {
            let slot = self.slot(npc).ok_or(MindError::NothingToReflect)?;
            if slot.records.is_empty() {
                return Err(MindError::NothingToReflect);
            }
            let mut rank = [0usize; 1];
            if self.recall_into(npc, tick, &mut rank) == 0 {
                return Err(MindError::NothingToReflect);
            }
            let top = &slot.records[rank[0]];
            let mood = match top.salience.to_mind() {
                700..=1000 => "restless",
                400..=699 => "watchful",
                _ => "calm",
            };
            let line = format!(
                "Feels {mood} after {} memories; foremost: {}",
                slot.records.len(),
                top.text
            );
            let bounded: String = line.chars().take(MAX_TEXT).collect();
            let last_felt = slot
                .records
                .iter()
                .rev()
                .find(|r| cadence && r.kind == MemoryKind::Felt)
                .map(|r| r.salience.to_mind());
            let top_salience = top.salience.to_mind();
            (
                bounded,
                crate::reflect::importance(top_salience, top_salience, last_felt),
            )
        };
        self.observe(npc, tick, MemoryKind::Felt, &text, salience)
    }

    /// Contradictory pairs for the eval harness: same tick plus same kind with
    /// different text. Both records are kept; ranking decides which reads
    /// first, and this count feeds the contradiction rate.
    pub fn contradictions(&self, npc: NpcId) -> usize {
        let Some(slot) = self.slot(npc) else {
            return 0;
        };
        let mut pairs = 0usize;
        for (index, first) in slot.records.iter().enumerate() {
            for second in slot.records.iter().skip(index + 1) {
                if first.tick == second.tick
                    && first.kind == second.kind
                    && first.text != second.text
                {
                    pairs += 1;
                }
            }
        }
        pairs
    }

    /// Deterministic bytes: version, NPC count, then per NPC (id, count, then
    /// tick, kind, salience, seq, byte length plus UTF-8) in insertion order;
    /// NPCs sorted by id so ingest order never moves the bytes.
    pub fn memory_bytes(&self) -> Vec<u8> {
        let mut order: Vec<usize> = (0..self.slots.len()).collect();
        order.sort_by_key(|index| self.slots[*index].npc.0);
        let mut out = Vec::new();
        out.extend_from_slice(&MEMORY_BYTES_VERSION.to_le_bytes());
        out.extend_from_slice(&(order.len() as u64).to_le_bytes());
        for index in order {
            let slot = &self.slots[index];
            out.extend_from_slice(&slot.npc.0.to_le_bytes());
            out.extend_from_slice(&(slot.records.len() as u64).to_le_bytes());
            for record in &slot.records {
                out.extend_from_slice(&record.tick.to_le_bytes());
                out.push(kind_byte(record.kind));
                out.push(record.salience.raw());
                out.extend_from_slice(&record.seq.to_le_bytes());
                out.extend_from_slice(&(record.text.len() as u64).to_le_bytes());
                out.extend_from_slice(record.text.as_bytes());
            }
        }
        out
    }

    /// Deterministic FNV-1a checkpoint over [`MemoryStore::memory_bytes`].
    /// One event log always yields one checkpoint.
    pub fn checkpoint(&self) -> u64 {
        hash_bytes(&self.memory_bytes())
    }

    /// Cap audit: fails when the store tracks more than [`MAX_NPCS`] NPCs or
    /// any NPC side holds more than [`MAX_STORED_PER_NPC`] records. The tick
    /// path evicts oldest-first so a live store passes; crafted or future
    /// bytes fail loudly through the load path first. The smallest offender
    /// NPC is named so one log always audits to one reason.
    pub fn audit_caps(&self) -> Result<(), MemorySaveError> {
        if self.slots.len() > MAX_NPCS {
            return Err(MemorySaveError::TooManyNpcs {
                count: self.slots.len(),
            });
        }
        let mut offender: Option<(u32, usize)> = None;
        for slot in &self.slots {
            if slot.records.len() > MAX_STORED_PER_NPC {
                let candidate = (slot.npc.0, slot.records.len());
                let keeps_current = offender.is_some_and(|current| current.0 <= candidate.0);
                if !keeps_current {
                    offender = Some(candidate);
                }
            }
        }
        if let Some((npc, records)) = offender {
            return Err(MemorySaveError::OverCap { npc, records });
        }
        Ok(())
    }

    /// Versioned per-NPC save envelope over the memory bytes: version, tag,
    /// NPC id, record count, then the record payloads in store order.
    /// Unknown NPCs save an empty side, which loads back to nothing.
    pub fn save_npc(&self, npc: NpcId) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MEMORY_SAVE_VERSION.to_le_bytes());
        out.push(SAVE_TAG_NPC);
        out.extend_from_slice(&npc.0.to_le_bytes());
        match self.slot(npc) {
            Some(slot) => {
                out.extend_from_slice(&(slot.records.len() as u64).to_le_bytes());
                for record in &slot.records {
                    write_record(&mut out, record);
                }
            }
            None => out.extend_from_slice(&0u64.to_le_bytes()),
        }
        out
    }

    /// Restore one per-NPC envelope: validates version, tag, counts, kinds
    /// and text, then replaces that NPC's side (an empty side removes it, so
    /// round-trips stay digest-identical). Over-cap counts refuse with
    /// [`MemorySaveError::OverCap`]; anything short, mistyped or trailing
    /// refuses as [`MemorySaveError::Corrupt`]. Loaded sequences join the
    /// store clock, so later ingests never reuse a sequence.
    pub fn load_npc_bytes(&mut self, bytes: &[u8]) -> Result<NpcId, MemorySaveError> {
        let mut cursor = ByteCursor::new(bytes);
        if bytes.len() < 5 {
            return Err(MemorySaveError::Corrupt);
        }
        let version = cursor.take_u32()?;
        if version != MEMORY_SAVE_VERSION {
            return Err(MemorySaveError::UnknownVersion);
        }
        if cursor.take_u8()? != SAVE_TAG_NPC {
            return Err(MemorySaveError::Corrupt);
        }
        let npc = NpcId(cursor.take_u32()?);
        let count = cursor.take_u64()?;
        if count > MAX_STORED_PER_NPC as u64 {
            return Err(MemorySaveError::OverCap {
                npc: npc.0,
                records: count as usize,
            });
        }
        let mut records = Vec::with_capacity(count as usize);
        let mut top_seq: Option<u64> = None;
        for _ in 0..count {
            let record = read_record(&mut cursor, npc)?;
            top_seq = Some(top_seq.map_or(record.seq, |top| top.max(record.seq)));
            records.push(record);
        }
        if !cursor.is_done() {
            return Err(MemorySaveError::Corrupt);
        }
        self.slots.retain(|slot| slot.npc != npc);
        if !records.is_empty() {
            self.slots.push(NpcSlot { npc, records });
        }
        if let Some(top) = top_seq {
            self.next_seq = self.next_seq.max(top.saturating_add(1));
        }
        self.audit_caps()?;
        Ok(npc)
    }

    /// Durable-minds envelope: the whole store plus its [`ProvenanceLog`]
    /// in one versioned byte string, so one save carries every memory and its
    /// write records together. Sides sort by NPC id, matching
    /// [`MemoryStore::memory_bytes`]; the provenance tail is the
    /// [`ProvenanceLog::bytes`] encoding, never a mirrored copy.
    pub fn save_all_with_provenance(&self, log: &ProvenanceLog) -> Vec<u8> {
        let mut order: Vec<usize> = (0..self.slots.len()).collect();
        order.sort_by_key(|index| self.slots[*index].npc.0);
        let mut out = Vec::new();
        out.extend_from_slice(&MEMORY_SAVE_VERSION.to_le_bytes());
        out.push(SAVE_TAG_ALL);
        out.extend_from_slice(&self.next_seq.to_le_bytes());
        out.extend_from_slice(&(order.len() as u64).to_le_bytes());
        for index in order {
            let slot = &self.slots[index];
            out.extend_from_slice(&slot.npc.0.to_le_bytes());
            out.extend_from_slice(&(slot.records.len() as u64).to_le_bytes());
            for record in &slot.records {
                write_record(&mut out, record);
            }
        }
        let provenance = log.bytes();
        out.extend_from_slice(&(provenance.len() as u64).to_le_bytes());
        out.extend_from_slice(&provenance);
        out
    }

    /// Restore a durable-minds envelope: validates the memory sides and the
    /// provenance tail together, then hands back both. Over-cap sides refuse
    /// with [`MemorySaveError::OverCap`], an over-full side table with
    /// [`MemorySaveError::TooManyNpcs`], provenance damage maps from its
    /// own error, and anything else short, duplicated, mistyped or trailing
    /// refuses as [`MemorySaveError::Corrupt`]. Empty sides are preserved, so
    /// save, load and save-again are byte-identical.
    pub fn load_all_with_provenance(
        bytes: &[u8],
    ) -> Result<(MemoryStore, ProvenanceLog), MemorySaveError> {
        let mut cursor = ByteCursor::new(bytes);
        if bytes.len() < 5 {
            return Err(MemorySaveError::Corrupt);
        }
        let version = cursor.take_u32()?;
        if version != MEMORY_SAVE_VERSION {
            return Err(MemorySaveError::UnknownVersion);
        }
        if cursor.take_u8()? != SAVE_TAG_ALL {
            return Err(MemorySaveError::Corrupt);
        }
        let next_seq = cursor.take_u64()?;
        let npc_count = cursor.take_u64()?;
        if npc_count > MAX_NPCS as u64 {
            return Err(MemorySaveError::TooManyNpcs {
                count: usize::try_from(npc_count).unwrap_or(usize::MAX),
            });
        }
        let mut store = MemoryStore {
            slots: Vec::with_capacity(npc_count as usize),
            next_seq,
        };
        let mut top_seq: Option<u64> = None;
        for _ in 0..npc_count {
            let npc = NpcId(cursor.take_u32()?);
            if store.slots.iter().any(|slot| slot.npc == npc) {
                return Err(MemorySaveError::Corrupt);
            }
            let count = cursor.take_u64()?;
            if count > MAX_STORED_PER_NPC as u64 {
                return Err(MemorySaveError::OverCap {
                    npc: npc.0,
                    records: count as usize,
                });
            }
            let mut records = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let record = read_record(&mut cursor, npc)?;
                top_seq = Some(top_seq.map_or(record.seq, |top| top.max(record.seq)));
                records.push(record);
            }
            store.slots.push(NpcSlot { npc, records });
        }
        // The writer clock always runs past every live sequence; anything
        // else is damage, and accepting it would reuse sequences.
        if top_seq.is_some_and(|top| top >= next_seq) {
            return Err(MemorySaveError::Corrupt);
        }
        let provenance_len = cursor.take_u64()?;
        if provenance_len > bytes.len() as u64 {
            return Err(MemorySaveError::Corrupt);
        }
        let provenance_raw = cursor.take(provenance_len as usize)?;
        let log = ProvenanceLog::from_bytes(provenance_raw)?;
        if !cursor.is_done() {
            return Err(MemorySaveError::Corrupt);
        }
        store.audit_caps()?;
        Ok((store, log))
    }
}

/// Typed save-envelope failures. Refusals build nothing and move no clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemorySaveError {
    /// Bytes truncated, mistyped, mis-ordered, duplicated or trailing.
    Corrupt,
    /// Envelope version is not [`MEMORY_SAVE_VERSION`]; v1 stays readable.
    UnknownVersion,
    /// One NPC side holds more than [`MAX_STORED_PER_NPC`] records; the NPC
    /// id and the refused count are carried for the audit line.
    OverCap { npc: u32, records: usize },
    /// The side table names more than [`MAX_NPCS`] NPCs.
    TooManyNpcs { count: usize },
}

impl core::fmt::Display for MemorySaveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MemorySaveError::Corrupt => f.write_str("memory save: corrupt bytes"),
            MemorySaveError::UnknownVersion => {
                f.write_str("memory save: unknown version (expected v1)")
            }
            MemorySaveError::OverCap { npc, records } => write!(
                f,
                "memory save: npc {npc} holds {records} records over cap {MAX_STORED_PER_NPC}"
            ),
            MemorySaveError::TooManyNpcs { count } => {
                write!(f, "memory save: {count} npcs over cap {MAX_NPCS}")
            }
        }
    }
}

impl std::error::Error for MemorySaveError {}

/// Provenance damage maps onto the save envelope: version drift stays loud,
/// everything else is corruption. Detail and fullness refusals cannot arise
/// from [`ProvenanceLog::from_bytes`], so they map to corruption as well.
impl From<ProvenanceError> for MemorySaveError {
    fn from(error: ProvenanceError) -> Self {
        match error {
            ProvenanceError::UnknownVersion => MemorySaveError::UnknownVersion,
            _ => MemorySaveError::Corrupt,
        }
    }
}

/// A short read of untrusted save bytes is corruption.
impl From<ShortRead> for MemorySaveError {
    fn from(_: ShortRead) -> Self {
        MemorySaveError::Corrupt
    }
}

/// One length-prefixed, bounded, non-blank UTF-8 text from save bytes.
fn take_text(cursor: &mut ByteCursor<'_>) -> Result<String, MemorySaveError> {
    let len = cursor.take_u64()?;
    if len > MAX_SAVE_TEXT_BYTES {
        return Err(MemorySaveError::Corrupt);
    }
    // `len` fits the text ceiling, so the cast keeps every byte.
    let raw = cursor.take(len as usize)?;
    let text = core::str::from_utf8(raw).map_err(|_| MemorySaveError::Corrupt)?;
    if text.trim().is_empty() || text.chars().count() > MAX_TEXT {
        return Err(MemorySaveError::Corrupt);
    }
    Ok(text.to_owned())
}

fn kind_of(byte: u8) -> Result<MemoryKind, MemorySaveError> {
    match byte {
        0 => Ok(MemoryKind::Seen),
        1 => Ok(MemoryKind::Heard),
        2 => Ok(MemoryKind::Did),
        3 => Ok(MemoryKind::Felt),
        _ => Err(MemorySaveError::Corrupt),
    }
}

fn write_record(out: &mut Vec<u8>, record: &StoredMemory) {
    out.extend_from_slice(&record.tick.to_le_bytes());
    out.push(kind_byte(record.kind));
    out.push(record.salience.raw());
    out.extend_from_slice(&record.seq.to_le_bytes());
    out.extend_from_slice(&(record.text.len() as u64).to_le_bytes());
    out.extend_from_slice(record.text.as_bytes());
}

fn read_record(cursor: &mut ByteCursor<'_>, npc: NpcId) -> Result<StoredMemory, MemorySaveError> {
    let tick = cursor.take_u64()?;
    let kind = kind_of(cursor.take_u8()?)?;
    let salience = Salience(cursor.take_u8()?);
    let seq = cursor.take_u64()?;
    let text = take_text(cursor)?;
    Ok(StoredMemory {
        tick,
        npc,
        kind,
        text,
        salience,
        seq,
    })
}

#[cfg(test)]
mod tests {
    use super::super::mind::{ActionSource, MindContext, NpcId as MindNpcId};
    use super::super::provenance::{PendingRecord, ProvenanceError, ProvenanceLog, Writer};
    use super::*;

    fn npc(id: u32) -> NpcId {
        NpcId(id)
    }

    fn kinds() -> [MemoryKind; 4] {
        [
            MemoryKind::Seen,
            MemoryKind::Heard,
            MemoryKind::Did,
            MemoryKind::Felt,
        ]
    }

    fn log_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        for round in 0..6u64 {
            for id in 0..4u32 {
                let kind = kinds()[(round as usize + id as usize) % kinds().len()];
                let salience = 150 + ((round * 137 + u64::from(id) * 311) % 800) as u32;
                store
                    .observe(
                        npc(id),
                        round * 10 + u64::from(id),
                        kind,
                        &format!("npc{id} saw portent {round}"),
                        salience,
                    )
                    .expect("fixture observes");
            }
        }
        // A same-tick tie pair and one contradiction for the edge tests.
        store
            .observe(npc(0), 500, MemoryKind::Seen, "twin omen alpha", 500)
            .expect("fixture tie");
        store
            .observe(npc(0), 500, MemoryKind::Seen, "twin omen beta", 500)
            .expect("fixture tie");
        store
            .observe(npc(1), 77, MemoryKind::Heard, "the gate holds", 620)
            .expect("fixture clash");
        store
            .observe(npc(1), 77, MemoryKind::Heard, "the gate falls", 640)
            .expect("fixture clash");
        store
    }

    #[test]
    fn same_log_same_bytes_x2() {
        let first = log_store();
        let second = log_store();
        assert_eq!(first.memory_bytes(), second.memory_bytes());
        assert_eq!(first.checkpoint(), second.checkpoint());
        assert_eq!(first.total_records(), 28);
        println!(
            "lw2-bytes len={} checkpoint={:016x} records={}",
            first.memory_bytes().len(),
            first.checkpoint(),
            first.total_records()
        );
    }

    #[test]
    fn soak_10k_holds_cap() {
        let build = || {
            let mut store = MemoryStore::new();
            for id in 0..100u32 {
                for event in 0..100u64 {
                    store
                        .observe(
                            npc(id),
                            event,
                            kinds()[(event as usize + id as usize) % kinds().len()],
                            &format!("npc{id} memory {event}"),
                            (event * 13 + u64::from(id) * 7) as u32 % 1001,
                        )
                        .expect("soak observes");
                }
            }
            store
        };
        let first = build();
        let second = build();
        assert_eq!(first.npc_count(), 100);
        for id in 0..100u32 {
            assert_eq!(first.len(npc(id)), 100, "lossless under cap");
        }
        assert_eq!(first.total_records(), 10_000);
        assert!(first.total_records() <= MAX_NPCS * MAX_STORED_PER_NPC);
        assert_eq!(first.memory_bytes(), second.memory_bytes());
        assert_eq!(first.checkpoint(), second.checkpoint());
        // Every NPC still recalls deterministically at scale.
        for id in 0..100u32 {
            let mut slot = [0usize; 2];
            assert_eq!(first.recall_into(npc(id), 200, &mut slot), 2);
            let again = first.recall(npc(id), 200, 2);
            assert_eq!(again.hits.len(), 2);
        }
        println!(
            "lw2-soak npcs={} per_npc={} total={} bytes={} checkpoint={:016x}",
            first.npc_count(),
            first.len(npc(0)),
            first.total_records(),
            first.memory_bytes().len(),
            first.checkpoint()
        );
    }

    #[test]
    fn lw1_snapshot_recall_integration() {
        let mut mind = Mind::new(MindNpcId(7), 42);
        mind.observe(10, MemoryKind::Seen, "a stranger enters the ludus", 900)
            .expect("fixture observes");
        mind.observe(11, MemoryKind::Heard, "the crowd chants at dusk", 300)
            .expect("fixture observes");
        mind.observe(12, MemoryKind::Did, "won the morning bout", 600)
            .expect("fixture observes");
        let mut store = MemoryStore::new();
        assert_eq!(store.remember_mind(&mind, 20), 3);
        // Re-ingesting the same stream stores nothing new: byte-identical.
        let before = store.memory_bytes();
        assert_eq!(store.remember_mind(&mind, 20), 0);
        assert_eq!(store.memory_bytes(), before);
        // Store top recall matches the mind's own top recall.
        let mut rank = [0usize; 1];
        assert_eq!(mind.recall_into(20, &mut rank), 1);
        let top = mind.events()[rank[0]].text.clone();
        let recalled = store.recall(MindNpcId(7), 20, 4);
        assert_eq!(recalled.hits[0].text, top);
        // Store order matches the mind snapshot line order.
        let ctx: MindContext = mind.snapshot(20);
        let lines: Vec<&str> = recalled.hits.iter().map(|hit| hit.text.as_str()).collect();
        let ctx_lines: Vec<&str> = ctx.memories.iter().map(String::as_str).collect();
        assert_eq!(lines, ctx_lines);
        // Reflection records a deterministic Felt summary with no provider.
        let checkpoint = store.checkpoint();
        store.reflect(MindNpcId(7), 21).expect("reflects");
        assert_eq!(store.len(MindNpcId(7)), 4);
        assert_ne!(store.checkpoint(), checkpoint);
        let again = store.recall(MindNpcId(7), 21, MAX_RECALL);
        assert!(again.hits.iter().any(|hit| hit.text.starts_with("Feels ")));
    }

    #[test]
    fn cap_overflow_evicts_oldest() {
        let mut store = MemoryStore::new();
        for tick in 0..(MAX_STORED_PER_NPC as u64 + 1) {
            store
                .observe(
                    npc(9),
                    tick,
                    MemoryKind::Seen,
                    &format!("event {tick}"),
                    500,
                )
                .expect("fills to cap");
        }
        assert_eq!(store.len(npc(9)), MAX_STORED_PER_NPC);
        // Oldest is gone: at tick 0 the top hit is event 1, not event 0.
        let top_old = store.recall(npc(9), 0, 1);
        assert_eq!(top_old.hits.len(), 1);
        assert_eq!(top_old.hits[0].text, "event 1", "oldest evicted past cap");
        // Newest is kept: late ticks rank the last event first.
        let top_new = store.recall(npc(9), 500, 1);
        assert_eq!(top_new.hits.len(), 1);
        assert_eq!(top_new.hits[0].text, "event 128", "newest kept");
    }

    #[test]
    fn contradictory_memories_kept_and_counted() {
        let store = log_store();
        assert_eq!(store.contradictions(npc(1)), 1);
        assert_eq!(store.contradictions(npc(2)), 0);
        assert_eq!(store.contradictions(npc(99)), 0);
        // Both rivals survive and rank deterministically twice.
        let first = store.recall(npc(1), 200, MAX_RECALL);
        let second = store.recall(npc(1), 200, MAX_RECALL);
        assert_eq!(first, second);
        let texts: Vec<&str> = first.hits.iter().map(|hit| hit.text.as_str()).collect();
        assert!(texts.contains(&"the gate holds"));
        assert!(texts.contains(&"the gate falls"));
    }

    #[test]
    fn recall_tie_keeps_earlier_record() {
        let store = log_store();
        let mut slot = [0usize; MAX_STORED_PER_NPC];
        let found = store.recall_into(npc(0), 600, &mut slot);
        assert!(found >= 2);
        let again = store.recall(npc(0), 600, MAX_STORED_PER_NPC);
        let texts: Vec<&str> = again.hits.iter().map(|hit| hit.text.as_str()).collect();
        let alpha = texts.iter().position(|t| *t == "twin omen alpha");
        let beta = texts.iter().position(|t| *t == "twin omen beta");
        assert!(alpha < beta, "equal scores keep ingest order");
    }

    #[test]
    fn rejects_bad_input() {
        let mut store = MemoryStore::new();
        assert_eq!(
            store.observe(npc(1), 0, MemoryKind::Seen, "", 10),
            Err(MindError::EmptyText)
        );
        assert_eq!(
            store.observe(npc(1), 0, MemoryKind::Seen, "   ", 10),
            Err(MindError::EmptyText)
        );
        let long: String = "x".repeat(MAX_TEXT + 1);
        assert_eq!(
            store.observe(npc(1), 0, MemoryKind::Seen, &long, 10),
            Err(MindError::TextTooLong)
        );
        assert_eq!(
            store.observe(npc(1), 0, MemoryKind::Seen, "fine", 1001),
            Err(MindError::BadSalience)
        );
        assert_eq!(store.len(npc(1)), 0, "failures store nothing");
        assert_eq!(store.recall_into(npc(1), 0, &mut [0usize; 1]), 0);
        assert!(store.recall(npc(1), 0, 4).is_empty());
        assert_eq!(store.reflect(npc(1), 0), Err(MindError::NothingToReflect));
    }

    #[test]
    fn npc_cap_refuses_cleanly() {
        let mut store = MemoryStore::new();
        for id in 0..MAX_NPCS as u32 {
            store
                .observe(npc(id), 0, MemoryKind::Seen, "hello", 10)
                .expect("fills npc cap");
        }
        assert_eq!(
            store.observe(npc(9999), 0, MemoryKind::Seen, "hello", 10),
            Err(MindError::TooManyActions)
        );
        assert_eq!(store.npc_count(), MAX_NPCS);
    }

    #[test]
    fn q8_quantize_roundtrip() {
        assert_eq!(Salience::from_mind(0), Salience::MIN);
        assert_eq!(Salience::from_mind(1000), Salience::MAX);
        assert_eq!(Salience::from_mind(9999).raw(), 255);
        let mut last = 0u8;
        for s in 0..=1000u32 {
            let q = Salience::from_mind(s).raw();
            assert!(q >= last, "quantize monotonic at {s}");
            last = q;
            let back = Salience::from_mind(s).to_mind();
            let gap = back.abs_diff(s);
            assert!(gap <= 2, "roundtrip within 2 at {s}");
        }
    }

    #[test]
    fn reflection_needs_no_provider() {
        let mut store = log_store();
        let first = store.checkpoint();
        store.reflect(npc(2), 600).expect("reflects offline");
        store.reflect(npc(2), 601).expect("reflects again");
        let second = store.checkpoint();
        assert_ne!(first, second);
        // Same log plus same reflections replays byte-identical with no
        // adapter in sight: models never enter this path.
        let mut replay = log_store();
        replay.reflect(npc(2), 600).expect("replays offline");
        replay.reflect(npc(2), 601).expect("replays again");
        assert_eq!(replay.memory_bytes(), store.memory_bytes());
        assert_eq!(replay.checkpoint(), second);
    }

    fn lived_log() -> ProvenanceLog {
        let mut log = ProvenanceLog::new();
        log.record(PendingRecord::external(
            10,
            ActionSource::Scripted,
            1,
            7,
            100,
            101,
            "npc3 recalls the gate",
        ))
        .expect("fixture records");
        log.record(PendingRecord {
            observed_tick: 11,
            source: ActionSource::Provider,
            request: 2,
            writer: Writer::Npc(npc(3)),
            digest_before: 101,
            digest_after: 102,
            detail: "the gate holds",
        })
        .expect("fixture records");
        log
    }

    #[test]
    fn save_npc_roundtrip_digest_identical_x2() {
        let first = log_store();
        first.audit_caps().expect("fixture passes audit");
        let mut second = MemoryStore::new();
        let mut third = MemoryStore::new();
        for id in 0..4u32 {
            let bytes = first.save_npc(npc(id));
            assert_eq!(second.load_npc_bytes(&bytes), Ok(npc(id)));
            assert_eq!(third.load_npc_bytes(&bytes), Ok(npc(id)));
        }
        assert_eq!(second.memory_bytes(), first.memory_bytes());
        assert_eq!(third.memory_bytes(), first.memory_bytes());
        assert_eq!(second.checkpoint(), first.checkpoint());
        assert_eq!(third.checkpoint(), first.checkpoint());
        // Save-again bytes are stable, and reloading one side is idempotent.
        for id in 0..4u32 {
            assert_eq!(second.save_npc(npc(id)), first.save_npc(npc(id)));
        }
        let mut again = second.clone();
        let bytes = second.save_npc(npc(1));
        assert_eq!(again.load_npc_bytes(&bytes), Ok(npc(1)));
        assert_eq!(again.memory_bytes(), second.memory_bytes());
        // Unknown NPCs save an empty side that loads back to nothing.
        let empty = first.save_npc(npc(99));
        let mut probe = first.clone();
        assert_eq!(probe.load_npc_bytes(&empty), Ok(npc(99)));
        assert_eq!(probe.memory_bytes(), first.memory_bytes());
        println!(
            "lw9-npc-save npcs={} bytes={} checkpoint={:016x}",
            first.npc_count(),
            first.save_npc(npc(0)).len(),
            first.checkpoint()
        );
    }

    #[test]
    fn save_all_with_provenance_roundtrip_x2() {
        let store = log_store();
        let log = lived_log();
        let bytes_a = store.save_all_with_provenance(&log);
        let bytes_b = store.save_all_with_provenance(&log);
        assert_eq!(bytes_a, bytes_b, "same minds give same envelope");
        let (loaded_a, log_a) =
            MemoryStore::load_all_with_provenance(&bytes_a).expect("envelope loads");
        let (loaded_b, log_b) =
            MemoryStore::load_all_with_provenance(&bytes_b).expect("envelope loads twice");
        assert_eq!(loaded_a.memory_bytes(), store.memory_bytes());
        assert_eq!(loaded_b.memory_bytes(), store.memory_bytes());
        assert_eq!(loaded_a.checkpoint(), store.checkpoint());
        assert_eq!(loaded_b.checkpoint(), store.checkpoint());
        assert_eq!(log_a.bytes(), log.bytes(), "LW6 records survive");
        assert_eq!(log_a.checkpoint(), log.checkpoint());
        assert_eq!(log_b.bytes(), log.bytes());
        // Save-again is byte-identical, and the restored clock stays ahead.
        assert_eq!(loaded_a.save_all_with_provenance(&log_a), bytes_a);
        let tip = loaded_a.next_seq;
        let mut grown = loaded_a;
        grown
            .observe(npc(0), 999, MemoryKind::Seen, "a late omen", 500)
            .expect("grows after load");
        assert_eq!(grown.next_seq, tip + 1, "sequences never reuse");
        assert_ne!(
            grown.save_all_with_provenance(&log_a),
            bytes_a,
            "growth changes the envelope"
        );
        println!(
            "lw9-durable bytes={} mem={:016x} prov={:016x}",
            bytes_a.len(),
            store.checkpoint(),
            log.checkpoint()
        );
    }

    #[test]
    fn soak_10k_holds_caps_and_roundtrips() {
        // Wide soak: 100 NPCs times 100 events stays lossless.
        let mut wide = MemoryStore::new();
        for id in 0..100u32 {
            for event in 0..100u64 {
                wide.observe(
                    npc(id),
                    event,
                    kinds()[(event as usize + id as usize) % kinds().len()],
                    &format!("npc{id} memory {event}"),
                    (event * 13 + u64::from(id) * 7) as u32 % 1001,
                )
                .expect("soak observes");
            }
        }
        wide.audit_caps().expect("wide soak passes audit");
        assert_eq!(wide.total_records(), 10_000);
        // Deep soak: 2 NPCs times 10k events evict down to the cap.
        let mut deep = MemoryStore::new();
        for id in 0..2u32 {
            for event in 0..10_000u64 {
                deep.observe(
                    npc(id),
                    event,
                    MemoryKind::Seen,
                    &format!("npc{id} deep {event}"),
                    500,
                )
                .expect("deep observes");
            }
        }
        for id in 0..2u32 {
            assert_eq!(deep.len(npc(id)), MAX_STORED_PER_NPC);
        }
        deep.audit_caps().expect("deep soak passes audit");
        // Both soaks round-trip digest-identical twice, memories plus provenance.
        let log = lived_log();
        for store in [&wide, &deep] {
            let bytes = store.save_all_with_provenance(&log);
            let (first, _) = MemoryStore::load_all_with_provenance(&bytes).expect("loads");
            let (second, _) = MemoryStore::load_all_with_provenance(&bytes).expect("loads twice");
            assert_eq!(first.memory_bytes(), store.memory_bytes());
            assert_eq!(second.memory_bytes(), store.memory_bytes());
            assert_eq!(first.checkpoint(), store.checkpoint());
            assert_eq!(second.checkpoint(), store.checkpoint());
            first.audit_caps().expect("loaded store passes audit");
        }
        println!(
            "lw9-soak wide={} deep_per_npc={} checkpoint={:016x}",
            wide.total_records(),
            deep.len(npc(0)),
            deep.checkpoint()
        );
    }

    #[test]
    fn corrupt_save_refuses_with_reason() {
        let store = log_store();
        let log = lived_log();
        let good_npc = store.save_npc(npc(0));
        let good_all = store.save_all_with_provenance(&log);
        // Every truncation refuses.
        for cut in [0usize, 1, 4, 5, 9, 13, 17, 20, 40, 80] {
            let cut = cut.min(good_npc.len().saturating_sub(1));
            assert_eq!(
                MemoryStore::new().load_npc_bytes(&good_npc[..cut]),
                Err(MemorySaveError::Corrupt),
                "npc truncation at {cut} refuses"
            );
            let cut = cut.min(good_all.len().saturating_sub(1));
            assert_eq!(
                load_all_err(&good_all[..cut]),
                MemorySaveError::Corrupt,
                "durable truncation at {cut} refuses"
            );
        }
        // Unknown versions refuse loudly, not as generic corruption.
        let mut versioned = good_npc.clone();
        versioned[0] = versioned[0].wrapping_add(1);
        assert_eq!(
            MemoryStore::new().load_npc_bytes(&versioned),
            Err(MemorySaveError::UnknownVersion)
        );
        let mut versioned = good_all.clone();
        versioned[0] = versioned[0].wrapping_add(1);
        assert_eq!(load_all_err(&versioned), MemorySaveError::UnknownVersion);
        // The wrong envelope kind refuses as corruption, never a version hit.
        assert_eq!(load_all_err(&good_npc), MemorySaveError::Corrupt);
        assert_eq!(
            MemoryStore::new().load_npc_bytes(&good_all),
            Err(MemorySaveError::Corrupt)
        );
        // Bad kind discriminant refuses (first record kind at 17 + 8).
        let mut bad_kind = good_npc.clone();
        bad_kind[25] = 9;
        assert_eq!(
            MemoryStore::new().load_npc_bytes(&bad_kind),
            Err(MemorySaveError::Corrupt)
        );
        // Bad UTF-8 detail refuses.
        let mut bad_text = good_npc.clone();
        if let Some(last) = bad_text.last_mut() {
            *last = 0xFF;
        }
        assert_eq!(
            MemoryStore::new().load_npc_bytes(&bad_text),
            Err(MemorySaveError::Corrupt)
        );
        // Trailing bytes refuse.
        let mut trailing = good_npc.clone();
        trailing.push(0);
        assert_eq!(
            MemoryStore::new().load_npc_bytes(&trailing),
            Err(MemorySaveError::Corrupt)
        );
        let mut trailing = good_all.clone();
        trailing.push(0);
        assert_eq!(load_all_err(&trailing), MemorySaveError::Corrupt);
        // Over-cap sides refuse with the audit reason naming NPC and cap.
        let over = over_cap_npc_bytes();
        let refused = MemoryStore::new()
            .load_npc_bytes(&over)
            .expect_err("129 records refuse");
        assert_eq!(
            refused,
            MemorySaveError::OverCap {
                npc: 0,
                records: 129
            }
        );
        let reason = format!("{refused}");
        assert!(
            reason.contains("npc 0") && reason.contains("129") && reason.contains("128"),
            "audit names npc, count and cap: {reason}"
        );
        // An over-full side table refuses with its own reason.
        let too_many = too_many_npcs_bytes();
        let refused =
            MemoryStore::load_all_with_provenance(&too_many).expect_err("129 npcs refuse");
        assert_eq!(refused, MemorySaveError::TooManyNpcs { count: 129 });
        assert!(
            format!("{refused}").contains("129"),
            "refusal carries the count"
        );
        // A duplicated NPC side refuses: one side must win, never both.
        assert_eq!(
            load_all_err(&duplicated_side_bytes()),
            MemorySaveError::Corrupt
        );
        // Provenance damage maps from its own error, never parsed as memory.
        let mut bad_prov = good_all.clone();
        if let Some(last) = bad_prov.last_mut() {
            *last = 0xFF;
        }
        assert_eq!(load_all_err(&bad_prov), MemorySaveError::Corrupt);
        let mut prov_version = good_all.clone();
        let start = prov_version.len() - log.bytes().len();
        prov_version[start] = prov_version[start].wrapping_add(1);
        assert_eq!(load_all_err(&prov_version), MemorySaveError::UnknownVersion);
        // The provenance mapping keeps version drift loud.
        assert_eq!(
            MemorySaveError::from(ProvenanceError::UnknownVersion),
            MemorySaveError::UnknownVersion
        );
        assert_eq!(
            MemorySaveError::from(ProvenanceError::Corrupt),
            MemorySaveError::Corrupt
        );
        assert!(!format!("{}", MemorySaveError::Corrupt).is_empty());
        assert!(!format!("{}", MemorySaveError::UnknownVersion).is_empty());
    }

    /// Unwrap the refusal from a durable envelope load: `MemoryStore` keeps
    /// no `PartialEq`, so error-only asserts go through here.
    fn load_all_err(bytes: &[u8]) -> MemorySaveError {
        MemoryStore::load_all_with_provenance(bytes).expect_err("fixture refuses")
    }

    /// One NPC side claiming 129 records: header plus one record payload
    /// repeated past the cap.
    fn over_cap_npc_bytes() -> Vec<u8> {
        let mut single = MemoryStore::new();
        single
            .observe(npc(0), 7, MemoryKind::Seen, "lone omen", 500)
            .expect("fixture observes");
        let one = single.save_npc(npc(0));
        let record = one[17..].to_vec();
        let mut out = Vec::new();
        out.extend_from_slice(&one[..9]);
        out.extend_from_slice(&129u64.to_le_bytes());
        for _ in 0..129 {
            out.extend_from_slice(&record);
        }
        out
    }

    /// A durable envelope naming 129 NPC sides.
    fn too_many_npcs_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MEMORY_SAVE_VERSION.to_le_bytes());
        out.push(SAVE_TAG_ALL);
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&129u64.to_le_bytes());
        out
    }

    /// A durable envelope with the same NPC side twice plus a valid provenance tail.
    fn duplicated_side_bytes() -> Vec<u8> {
        let mut single = MemoryStore::new();
        single
            .observe(npc(5), 7, MemoryKind::Seen, "lone omen", 500)
            .expect("fixture observes");
        let side = single.save_npc(npc(5))[5..].to_vec();
        let provenance = lived_log().bytes();
        let mut out = Vec::new();
        out.extend_from_slice(&MEMORY_SAVE_VERSION.to_le_bytes());
        out.push(SAVE_TAG_ALL);
        out.extend_from_slice(&3u64.to_le_bytes());
        out.extend_from_slice(&2u64.to_le_bytes());
        out.extend_from_slice(&side);
        out.extend_from_slice(&side);
        out.extend_from_slice(&(provenance.len() as u64).to_le_bytes());
        out.extend_from_slice(&provenance);
        out
    }

    /// LW12: 64 minds each remember one event a tick, recall their top 8 into
    /// a caller buffer (zero allocations) and drain a recorded `Say` through
    /// `apply_at` at the tick boundary. Caps hold, bytes match across runs;
    /// the p95 times print for the record and are never asserted.
    #[test]
    fn soak_64_minds_recall_and_drain_hold_caps() {
        use super::super::apply::{apply_at, Applied};
        use super::super::mind::{ActionKind, ActionLog, MindAction};
        use std::time::Instant;
        const MINDS: u32 = 64;
        const TICKS: u64 = 600;
        let p95 = |mut us: Vec<u128>| {
            us.sort_unstable();
            us[us.len() * 95 / 100]
        };
        let run = || {
            let mut store = MemoryStore::new();
            let mut log = ActionLog::new();
            let mut fold = engine_core::Fnv64::new();
            let (mut recall_us, mut drain_us) = (Vec::new(), Vec::new());
            let mut presented = 0usize;
            for tick in 1..=TICKS {
                for id in 0..MINDS {
                    let salience = ((tick * 31 + u64::from(id) * 17) % 1001) as u32;
                    let kind = kinds()[(tick as usize + id as usize) % kinds().len()];
                    store
                        .observe(npc(id), tick, kind, &format!("n{id} t{tick}"), salience)
                        .expect("soak observes");
                    log.push(MindAction {
                        tick_apply: tick,
                        npc: npc(id),
                        kind: ActionKind::Say,
                        text: format!("line {id} {tick}"),
                        source: ActionSource::Scripted,
                        request: tick * 100 + u64::from(id),
                    })
                    .expect("line records");
                }
                let mut top = [0usize; MAX_RECALL];
                let started = Instant::now();
                let ((), allocs) = engine_core::alloc_probe::measure(|| {
                    for id in 0..MINDS {
                        let found = store.recall_into(npc(id), tick, &mut top);
                        assert_eq!(found, store.len(npc(id)).min(MAX_RECALL));
                        for slot in &top[..found] {
                            fold.write_u64(*slot as u64);
                        }
                    }
                });
                recall_us.push(started.elapsed().as_micros());
                assert_eq!(allocs.calls, 0, "recall allocates nothing at tick {tick}");
                let started = Instant::now();
                for action in log.drain_due(tick) {
                    match apply_at(&action, tick, tick) {
                        Ok(Applied::Presented(event)) => {
                            presented += 1;
                            fold.write_u64(event.request);
                        }
                        other => panic!("say must present at its tick: {other:?}"),
                    }
                }
                drain_us.push(started.elapsed().as_micros());
                assert!(log.is_empty(), "drain leaves nothing due");
            }
            store.audit_caps().expect("caps hold");
            assert_eq!(store.npc_count(), MINDS as usize);
            for id in 0..MINDS {
                assert_eq!(store.len(npc(id)), MAX_STORED_PER_NPC);
            }
            println!(
                "lw12-soak minds={MINDS} ticks={TICKS} recall64_p95_us={} drain64_p95_us={}",
                p95(recall_us),
                p95(drain_us)
            );
            (store.memory_bytes(), fold.finish(), presented)
        };
        let first = run();
        assert_eq!(first.2, (MINDS as usize) * (TICKS as usize));
        assert_eq!(first, run(), "bytes and drain fold equal x2");
    }
}
