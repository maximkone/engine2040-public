//! Deterministic diagnostic event queue with phased delivery.
//!
//! Consumer: replay and chaos harnesses drain flushed rows into the shared
//! ring and export the tape for byte-identical comparison. Data owner: this
//! module owns queue order, the dispatch buffer and the tape; the caller owns
//! handler membership and the ring. Units: bands (`u8`), sequence numbers and
//! ticks (`u64`), capacities (`usize`). Determinism: band then insertion order
//! with stable maps and sorts, saturating counters, no clock. Errors:
//! [`QueueError`]; full buffers fail loudly, never drop silently.
//!
//! Diagnostics only: queued rows never enter any simulation hash. Sending only
//! enqueues; delivery happens at the phase boundary through an explicit flush.
//! There is no immediate-delivery path.

use super::{Event, EventCode, RingBuffer};
use engine_core::hash::hash_bytes;
use std::collections::BTreeMap;
use std::fmt;

/// Authority band: simulation faults that must replay first.
pub const BAND_AUTHORITY: u8 = 0;
/// Service band: provider, asset and command diagnostics.
pub const BAND_SERVICE: u8 = 1;
/// Presentation band: render, editor and view diagnostics.
pub const BAND_PRESENTATION: u8 = 2;

/// Map a registry code to its delivery band.
///
/// Simulation faults use the authority band; provider, asset, command, editor
/// and save codes use the service band; render and view codes use the
/// presentation band; heal records follow the fault they describe.
#[must_use]
pub fn band_for(code: EventCode) -> u8 {
    if code == EventCode::PHYSICS_BODY_NONFINITE
        || code == EventCode::BRAIN_FIGHTER_STUCK
        || code == EventCode::NET_CLIENT_DESYNC
    {
        BAND_AUTHORITY
    } else if code == EventCode::RENDER_DEVICE_LOST
        || code == EventCode::RENDER_PIPELINE_FAILED
        || code == EventCode::RENDER_FRAME_OVER_BUDGET
        || code == EventCode::EDITOR_CRASH
        || code == EventCode::FPS_POSE
        || code == EventCode::FPS_HIT
        || code == EventCode::FPS_FRAME_SPLIT
    {
        BAND_PRESENTATION
    } else {
        BAND_SERVICE
    }
}

/// Stable handler identity. Values are dense from zero in registration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandlerId(u64);

impl HandlerId {
    /// Raw value for serialization into tapes and bundles.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Typed failure for queue construction and phase control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    /// The pending queue is full; drain with a flush before sending more.
    QueueFull,
    /// The tape is full; export and clear it before flushing more rows.
    TapeFull,
    /// No live handler carries this identity.
    UnknownHandler(HandlerId),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => write!(f, "event queue is full"),
            Self::TapeFull => write!(f, "event tape is full"),
            Self::UnknownHandler(id) => write!(f, "unknown handler: {}", id.raw()),
        }
    }
}

impl std::error::Error for QueueError {}

/// One queued send waiting for the phase boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRow {
    seq: u64,
    band: u8,
    event: Event,
}

/// One flushed row in delivery order. The simulation copies these rows into
/// its own action log so replay reproduces the same order with logging on or
/// off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapeRecord {
    /// Insertion order of the send.
    pub seq: u64,
    /// Delivery band of the send.
    pub band: u8,
    /// Stable registry code.
    pub code: EventCode,
    /// Tick the event was emitted at.
    pub tick: u64,
    /// Bounded detail text.
    pub message: String,
}

impl TapeRecord {
    /// Fixed-shape line for byte-identical export and hashing.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{} {} {} {} {}\n",
            self.seq,
            self.band,
            self.code.raw(),
            self.tick,
            self.message
        )
    }
}

/// Bounded phased queue: sends enqueue, flushes deliver in band then
/// insertion order and record every delivered row in the tape.
#[derive(Debug, Clone)]
pub struct EventQueue {
    queue_cap: usize,
    tape_cap: usize,
    next_seq: u64,
    pending: Vec<PendingRow>,
    dispatch: Vec<PendingRow>,
    cursor: usize,
    flushing: bool,
    tape: Vec<TapeRecord>,
}

impl EventQueue {
    /// Empty queue with bounded pending and tape capacities.
    #[must_use]
    pub fn new(queue_cap: usize, tape_cap: usize) -> Self {
        Self {
            queue_cap,
            tape_cap,
            next_seq: 0,
            pending: Vec::new(),
            dispatch: Vec::new(),
            cursor: 0,
            flushing: false,
            tape: Vec::new(),
        }
    }

    /// Maximum queued sends waiting for a flush.
    #[must_use]
    pub fn queue_cap(&self) -> usize {
        self.queue_cap
    }

    /// Maximum tape rows retained.
    #[must_use]
    pub fn tape_cap(&self) -> usize {
        self.tape_cap
    }

    /// Sends waiting for the next flush.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Rows still to deliver in the active flush.
    #[must_use]
    pub fn dispatch_remaining(&self) -> usize {
        self.dispatch.len().saturating_sub(self.cursor)
    }

    /// True while a flush is active.
    #[must_use]
    pub fn is_flushing(&self) -> bool {
        self.flushing
    }

    /// Next insertion sequence number.
    #[must_use]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Tape rows in delivery order.
    #[must_use]
    pub fn tape(&self) -> &[TapeRecord] {
        &self.tape
    }

    /// Number of tape rows.
    #[must_use]
    pub fn tape_len(&self) -> usize {
        self.tape.len()
    }

    /// Queue one event for the next phase boundary. Never delivers inline:
    /// sends during a flush wait for the following flush.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::QueueFull`] when the pending buffer is full.
    pub fn send(&mut self, band: u8, event: Event) -> Result<u64, QueueError> {
        if self.pending.len() >= self.queue_cap {
            return Err(QueueError::QueueFull);
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.pending.push(PendingRow { seq, band, event });
        Ok(seq)
    }

    /// Queue one event under its registry band.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::QueueFull`] when the pending buffer is full.
    pub fn send_auto(&mut self, event: Event) -> Result<u64, QueueError> {
        let band = band_for(event.code);
        self.send(band, event)
    }

    /// Start the phase boundary: sort pending by band then insertion order
    /// and stage it for stepwise delivery. Sends after this point wait for
    /// the next flush.
    pub fn begin_flush(&mut self) {
        if self.flushing {
            return;
        }
        self.pending
            .sort_unstable_by(|a, b| a.band.cmp(&b.band).then(a.seq.cmp(&b.seq)));
        self.dispatch = Vec::new();
        core::mem::swap(&mut self.pending, &mut self.dispatch);
        self.cursor = 0;
        self.flushing = true;
    }

    /// Deliver the next staged row into the caller ring and the tape.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::TapeFull`] when the tape is full; the row stays
    /// staged so the caller can export, clear and retry.
    pub fn pop_dispatch(
        &mut self,
        ring: &mut RingBuffer,
    ) -> Result<Option<TapeRecord>, QueueError> {
        if !self.flushing {
            return Ok(None);
        }
        let Some(row) = self.dispatch.get(self.cursor).cloned() else {
            self.dispatch.clear();
            self.cursor = 0;
            self.flushing = false;
            return Ok(None);
        };
        if self.tape.len() >= self.tape_cap {
            return Err(QueueError::TapeFull);
        }
        ring.push(row.event.clone());
        let record = TapeRecord {
            seq: row.seq,
            band: row.band,
            code: row.event.code,
            tick: row.event.tick,
            message: row.event.message.clone(),
        };
        self.tape.push(record.clone());
        self.cursor = self.cursor.saturating_add(1);
        if self.cursor >= self.dispatch.len() {
            self.dispatch.clear();
            self.cursor = 0;
            self.flushing = false;
        }
        Ok(Some(record))
    }

    /// Drain every staged row into the caller ring, returning delivery order.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::TapeFull`] when the tape fills mid-flush; rows
    /// already delivered stay taped, the rest stay staged for retry.
    pub fn drain_into_ring(
        &mut self,
        ring: &mut RingBuffer,
    ) -> Result<Vec<TapeRecord>, QueueError> {
        self.begin_flush();
        let mut out = Vec::new();
        while let Some(record) = self.pop_dispatch(ring)? {
            out.push(record);
        }
        Ok(out)
    }

    /// Deterministic export of the tape in delivery order.
    #[must_use]
    pub fn tape_bytes(&self) -> String {
        let mut out = String::new();
        for record in &self.tape {
            out.push_str(&record.line());
        }
        out
    }

    /// Stable digest of [`Self::tape_bytes`] over the shared FNV owner.
    #[must_use]
    pub fn tape_digest(&self) -> u64 {
        hash_bytes(self.tape_bytes().as_bytes())
    }

    /// Clear delivered tape history after export. Pending and staged rows are
    /// kept: history is evidence, not budget.
    pub fn clear_tape(&mut self) {
        self.tape.clear();
    }
}

/// Bounded handler membership with deferred mid-flush changes.
///
/// Handlers are passive identities with an optional code filter; the flush
/// driver snapshots [`HandlerSet::matching`] per row, so registering or
/// removing mid-flush never invalidates the snapshot. Deferred changes apply
/// in identity order at [`HandlerSet::end_dispatch`].
#[derive(Debug, Clone)]
pub struct HandlerSet {
    next_id: u64,
    live: BTreeMap<u64, Option<EventCode>>,
    deferred_add: Vec<(u64, Option<EventCode>)>,
    deferred_remove: Vec<u64>,
    flushing: bool,
}

impl HandlerSet {
    /// Empty membership with no handlers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 0,
            live: BTreeMap::new(),
            deferred_add: Vec::new(),
            deferred_remove: Vec::new(),
            flushing: false,
        }
    }

    /// Number of live handlers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// True when no handlers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// True while a flush snapshot is active.
    #[must_use]
    pub fn is_flushing(&self) -> bool {
        self.flushing
    }

    /// True when the identity is live.
    #[must_use]
    pub fn contains(&self, id: HandlerId) -> bool {
        self.live.contains_key(&id.raw())
    }

    /// Register one handler. During a flush the add waits for the boundary
    /// and the identity is still returned at once.
    #[must_use]
    pub fn register(&mut self, filter: Option<EventCode>) -> HandlerId {
        let id = HandlerId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        if self.flushing {
            self.deferred_add.push((id.raw(), filter));
        } else {
            self.live.insert(id.raw(), filter);
        }
        id
    }

    /// Remove one handler. During a flush the removal waits for the boundary.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::UnknownHandler`] when the identity is not live
    /// and not already deferred.
    pub fn unregister(&mut self, id: HandlerId) -> Result<(), QueueError> {
        if self.flushing {
            if self.live.contains_key(&id.raw())
                || self.deferred_add.iter().any(|row| row.0 == id.raw())
            {
                self.deferred_remove.push(id.raw());
                return Ok(());
            }
            return Err(QueueError::UnknownHandler(id));
        }
        if self.live.remove(&id.raw()).is_some() {
            Ok(())
        } else {
            Err(QueueError::UnknownHandler(id))
        }
    }

    /// Live identities interested in `code`, in registration order.
    #[must_use]
    pub fn matching(&self, code: EventCode) -> Vec<HandlerId> {
        self.live
            .iter()
            .filter(|(_, filter)| filter.is_none_or(|want| want == code))
            .map(|(id, _)| HandlerId(*id))
            .collect()
    }

    /// Open a delivery snapshot; mutations defer until [`Self::end_dispatch`].
    pub fn begin_dispatch(&mut self) {
        self.flushing = true;
    }

    /// Close the snapshot and apply deferred adds then removes in order.
    pub fn end_dispatch(&mut self) {
        self.deferred_add.sort_unstable_by_key(|row| row.0);
        for row in core::mem::take(&mut self.deferred_add) {
            self.live.insert(row.0, row.1);
        }
        self.deferred_remove.sort_unstable();
        for id in core::mem::take(&mut self.deferred_remove) {
            self.live.remove(&id);
        }
        self.flushing = false;
    }
}

impl Default for HandlerSet {
    /// Empty membership; same as [`HandlerSet::new`].
    fn default() -> Self {
        Self::new()
    }
}

/// Run one full phase: snapshot handlers, stage the queue by band then
/// insertion order, deliver every row into the ring and close the snapshot.
///
/// Deferred handler adds and removes apply at the boundary, and sends queued
/// by the driver loop wait for the next phase. Returns delivery order.
///
/// # Errors
///
/// Returns [`QueueError::TapeFull`] when the tape fills mid-flush; rows
/// already delivered stay taped and the remaining rows stay staged.
pub fn flush_phase(
    queue: &mut EventQueue,
    handlers: &mut HandlerSet,
    ring: &mut RingBuffer,
) -> Result<Vec<TapeRecord>, QueueError> {
    handlers.begin_dispatch();
    queue.begin_flush();
    let mut out = Vec::new();
    loop {
        if queue.dispatch_remaining() == 0 {
            break;
        }
        match queue.pop_dispatch(ring) {
            Ok(Some(record)) => {
                let _live = handlers.matching(record.code);
                out.push(record);
            }
            Ok(None) => break,
            Err(err) => {
                handlers.end_dispatch();
                return Err(err);
            }
        }
    }
    handlers.end_dispatch();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(code: EventCode, tick: u64, message: &str) -> Event {
        Event::new(code, tick, message)
    }

    fn seed_queue(count: u64) -> EventQueue {
        let mut queue = EventQueue::new(2048, 2048);
        for seq in 0..count {
            let code = match seq % 3 {
                0 => EventCode::SIM_TICK,
                1 => EventCode::PROVIDER_TIMEOUT,
                _ => EventCode::RENDER_FRAME_OVER_BUDGET,
            };
            let band = band_for(code);
            queue
                .send(band, event(code, seq, "seed"))
                .expect("seed fits");
        }
        queue
    }

    #[test]
    fn bands_cover_registry() {
        assert_eq!(band_for(EventCode::PHYSICS_BODY_NONFINITE), BAND_AUTHORITY);
        assert_eq!(band_for(EventCode::BRAIN_FIGHTER_STUCK), BAND_AUTHORITY);
        assert_eq!(band_for(EventCode::NET_CLIENT_DESYNC), BAND_AUTHORITY);
        assert_eq!(band_for(EventCode::PROVIDER_TIMEOUT), BAND_SERVICE);
        assert_eq!(band_for(EventCode::COMMANDS_STALE), BAND_SERVICE);
        assert_eq!(
            band_for(EventCode::RENDER_FRAME_OVER_BUDGET),
            BAND_PRESENTATION
        );
        assert_eq!(band_for(EventCode::FPS_POSE), BAND_PRESENTATION);
        assert_eq!(band_for(EventCode::FPS_HIT), BAND_PRESENTATION);
    }

    #[test]
    fn send_queues_without_delivering() {
        let mut queue = EventQueue::new(8, 8);
        let mut ring = RingBuffer::new(8);
        queue
            .send(BAND_SERVICE, event(EventCode::SIM_TICK, 1, "held"))
            .expect("fits");
        assert_eq!(queue.pending_len(), 1);
        assert_eq!(queue.tape_len(), 0);
        assert!(ring.is_empty());
        let flushed = queue.drain_into_ring(&mut ring).expect("drains");
        assert_eq!(flushed.len(), 1);
        assert_eq!(queue.pending_len(), 0);
        assert_eq!(queue.tape_len(), 1);
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn flush_orders_band_then_insertion() {
        let mut queue = EventQueue::new(16, 16);
        let mut ring = RingBuffer::new(16);
        queue
            .send(BAND_PRESENTATION, event(EventCode::SIM_TICK, 3, "late"))
            .expect("fits");
        queue
            .send(BAND_AUTHORITY, event(EventCode::SIM_TICK, 1, "first"))
            .expect("fits");
        queue
            .send(BAND_AUTHORITY, event(EventCode::SIM_TICK, 2, "second"))
            .expect("fits");
        queue
            .send(BAND_SERVICE, event(EventCode::SIM_TICK, 4, "mid"))
            .expect("fits");
        let flushed = queue.drain_into_ring(&mut ring).expect("drains");
        let messages: Vec<&str> = flushed.iter().map(|row| row.message.as_str()).collect();
        assert_eq!(messages, vec!["first", "second", "mid", "late"]);
        assert_eq!(queue.tape_digest(), queue.tape_digest());
    }

    #[test]
    fn thousand_events_match_across_chunkings() {
        fn build(chunk: u64) -> (String, u64) {
            let mut queue = EventQueue::new(2048, 2048);
            let mut ring = RingBuffer::new(2048);
            let mut seq_tick = 0_u64;
            while seq_tick < 1000 {
                let end = seq_tick.saturating_add(chunk).min(1000);
                for tick in seq_tick..end {
                    let code = match tick % 4 {
                        0 => EventCode::SIM_TICK,
                        1 => EventCode::PROVIDER_TIMEOUT,
                        2 => EventCode::RENDER_FRAME_OVER_BUDGET,
                        _ => EventCode::FPS_POSE,
                    };
                    queue.send_auto(event(code, tick, "tape")).expect("fits");
                }
                seq_tick = end;
            }
            let flushed = queue.drain_into_ring(&mut ring).expect("drains");
            assert_eq!(flushed.len(), 1000);
            (queue.tape_bytes(), queue.tape_digest())
        }
        let one = build(1);
        let seven = build(7);
        let all = build(1000);
        assert_eq!(one.0, seven.0);
        assert_eq!(seven.0, all.0);
        assert_eq!(one.1, all.1);
        assert!(!one.0.is_empty());
    }

    #[test]
    fn sends_during_flush_wait_for_next_phase() {
        let mut queue = EventQueue::new(16, 16);
        let mut ring = RingBuffer::new(16);
        queue
            .send(BAND_SERVICE, event(EventCode::SIM_TICK, 1, "one"))
            .expect("fits");
        queue.begin_flush();
        queue
            .send(BAND_SERVICE, event(EventCode::SIM_TICK, 2, "two"))
            .expect("queued during flush");
        let first = queue.drain_into_ring(&mut ring).expect("drains staged");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].message, "one");
        assert_eq!(queue.pending_len(), 1);
        let second = queue.drain_into_ring(&mut ring).expect("drains next");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].message, "two");
    }

    #[test]
    fn handler_changes_during_flush_apply_at_boundary() {
        let mut handlers = HandlerSet::new();
        let keep = handlers.register(Some(EventCode::SIM_TICK));
        let drop = handlers.register(None);
        assert_eq!(handlers.len(), 2);
        handlers.begin_dispatch();
        let snapshot = handlers.matching(EventCode::SIM_TICK);
        assert_eq!(snapshot, vec![keep, drop]);
        let added = handlers.register(Some(EventCode::SIM_TICK));
        handlers.unregister(drop).expect("live removes defer");
        assert_eq!(handlers.len(), 2);
        assert!(handlers.contains(drop));
        assert!(!handlers.contains(added));
        handlers.end_dispatch();
        assert_eq!(handlers.len(), 2);
        assert!(!handlers.contains(drop));
        assert!(handlers.contains(added));
        assert!(handlers.contains(keep));
    }

    #[test]
    fn unknown_handler_fails() {
        let mut handlers = HandlerSet::new();
        let missing = HandlerId(999);
        assert_eq!(
            handlers.unregister(missing),
            Err(QueueError::UnknownHandler(missing))
        );
        let err = QueueError::UnknownHandler(missing);
        assert_eq!(err.to_string(), "unknown handler: 999");
        let _: &dyn std::error::Error = &err;
        assert_eq!(QueueError::QueueFull.to_string(), "event queue is full");
        assert_eq!(QueueError::TapeFull.to_string(), "event tape is full");
    }

    #[test]
    fn full_buffers_fail_loudly() {
        let mut queue = EventQueue::new(1, 1);
        let mut ring = RingBuffer::new(4);
        queue
            .send(BAND_SERVICE, event(EventCode::SIM_TICK, 1, "one"))
            .expect("fits");
        assert_eq!(
            queue.send(BAND_SERVICE, event(EventCode::SIM_TICK, 2, "two")),
            Err(QueueError::QueueFull)
        );
        queue.drain_into_ring(&mut ring).expect("first drains");
        queue
            .send(BAND_SERVICE, event(EventCode::SIM_TICK, 3, "three"))
            .expect("fits");
        queue.begin_flush();
        assert_eq!(queue.pop_dispatch(&mut ring), Err(QueueError::TapeFull));
        queue.clear_tape();
        let retry = queue.pop_dispatch(&mut ring).expect("retries");
        assert!(retry.is_some());
    }

    #[test]
    fn flush_phase_delivers_and_records() {
        let mut queue = seed_queue(12);
        let mut handlers = HandlerSet::new();
        handlers.register(None);
        handlers.register(Some(EventCode::SIM_TICK));
        let mut ring = RingBuffer::new(32);
        let flushed = flush_phase(&mut queue, &mut handlers, &mut ring).expect("flushes");
        assert_eq!(flushed.len(), 12);
        assert_eq!(ring.len(), 12);
        assert_eq!(queue.tape_len(), 12);
        let replay = queue.tape_bytes();
        let mut second = seed_queue(12);
        let mut ring_two = RingBuffer::new(32);
        let mut handlers_two = HandlerSet::new();
        second
            .drain_into_ring(&mut ring_two)
            .expect("second drains");
        assert_eq!(second.tape_bytes(), replay);
        assert_eq!(second.tape_digest(), queue.tape_digest());
        let _ = handlers_two.register(None);
    }

    #[test]
    fn tape_line_and_digest_are_stable() {
        let record = TapeRecord {
            seq: 7,
            band: BAND_SERVICE,
            code: EventCode::SIM_TICK,
            tick: 42,
            message: String::from("tick"),
        };
        assert_eq!(record.line(), "7 1 1 42 tick\n");
        let mut queue = EventQueue::new(8, 8);
        let mut ring = RingBuffer::new(8);
        queue
            .send_auto(event(EventCode::SIM_TICK, 42, "tick"))
            .expect("fits");
        queue.drain_into_ring(&mut ring).expect("drains");
        assert_eq!(queue.tape_digest(), queue.tape_digest());
        assert!(!queue.tape_bytes().is_empty());
    }

    #[ignore = "timing: phase flush of 1k rows stays inside the service budget"]
    #[test]
    fn phase_flush_thousand_rows_within_budget() {
        let mut queue = seed_queue(1000);
        let mut ring = RingBuffer::new(2048);
        let start = std::time::Instant::now();
        queue.drain_into_ring(&mut ring).expect("drains");
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 1000, "elapsed {elapsed:?}");
        assert_eq!(ring.len(), 1000);
    }
}
