//! stream: prioritized asset-streaming queue with memory budgets.
//!
//! Owner: `engine-assets`; consumer: the editor asset browser and the game
//! and editor loaders at frame boundaries. Units: bytes for budgets, reservations
//! and progress. Higher `priority` completes first, ties break on the blake3
//! hex id ascending, so the same requests plus the same `publish` calls always
//! emit the same events. Refusals are typed errors convertible to browser
//! events, never silent drops. Real bundle bytes bind in the consumer; this
//! module is std-only and owns ordering, budgets and progress. Temporary local
//! mirrors of `bundle.rs` budgets are marked below until the orchestrator
//! registers this module with a real edge.

use std::cmp::Ordering;
use std::fmt;

/// Longest stream entry name in chars.
/// TEMPORARY local mirror of `bundle::MAX_NAME_CHARS`; same budget, no edge.
pub const MAX_STREAM_NAME_CHARS: usize = 128;
/// Largest single stream payload in bytes.
/// TEMPORARY local mirror of `bundle::MAX_PAYLOAD_BYTES`; same budget, no edge.
pub const MAX_STREAM_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
/// Hex length of a blake3 digest id carried by every job and event.
pub const BLAKE3_HEX_LEN: usize = 64;
/// Bound for echoed text carried in errors and refusal events.
/// TEMPORARY local mirror of `bundle::MAX_DETAIL_CHARS`; same budget, no edge.
pub const MAX_STREAM_DETAIL_CHARS: usize = 128;

/// Payload kind of one stream job.
/// TEMPORARY local mirror of `bundle::EntryKind`: same variants plus
/// discriminants so the consumer can map 1:1 at the IO boundary. Do not
/// renumber; replace with the documented `bundle` edge when registered.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum StreamKind {
    /// Encoded cooked mesh bytes.
    Mesh = 1,
    /// Encoded cooked material bytes.
    Material = 2,
    /// Encoded cooked texture bytes with mips.
    Texture = 3,
    /// Encoded skin bytes.
    Skin = 4,
    /// Encoded imported clip bytes.
    Clip = 5,
}

impl StreamKind {
    /// Lowercase kind name for browser rows and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Material => "material",
            Self::Texture => "texture",
            Self::Skin => "skin",
            Self::Clip => "clip",
        }
    }
}

impl fmt::Display for StreamKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Stream identity: the blake3 hex digest of the payload, lowercase ASCII.
/// Lexicographic order is the deterministic tie-break for equal priorities.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamId(String);

impl StreamId {
    /// Validate `text` as a blake3 hex id; casing and length are exact.
    pub fn parse(text: &str) -> Result<Self, StreamError> {
        if text.len() != BLAKE3_HEX_LEN
            || !text
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(StreamError::BadId(bound_detail(text)));
        }
        Ok(Self(text.to_string()))
    }

    /// Borrow the hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a stream request, cancel or eviction failed; every refusal maps to a
/// [`StreamEvent::Refused`] through [`StreamError::refused_event`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// Id is not a 64-char lowercase hex digest; echo bounded.
    BadId(String),
    /// Entry name empty or over [`MAX_STREAM_NAME_CHARS`]; echo bounded.
    BadName(String),
    /// Id already queued or resident; the first slot is untouched.
    Duplicate(String),
    /// Payload byte count outside `1..=MAX_STREAM_PAYLOAD_BYTES`.
    TooLarge {
        /// Offending byte count.
        count: usize,
    },
    /// Admitting the job would exceed the byte budget.
    OverBudget {
        /// Payload bytes the job wanted reserved.
        want_bytes: u64,
        /// Bytes free under the budget at request time.
        free_bytes: u64,
    },
    /// Admitting the job would exceed the job-slot budget.
    JobCap {
        /// Slot budget that is full.
        limit: u32,
    },
    /// Id is neither pending nor resident; echo bounded.
    Unknown(String),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadId(text) => write!(f, "stream bad id {text:?}"),
            Self::BadName(name) => write!(f, "stream bad name {name:?}"),
            Self::Duplicate(id) => write!(f, "stream duplicate job {id:?}"),
            Self::TooLarge { count } => write!(f, "stream too large: {count}"),
            Self::OverBudget {
                want_bytes,
                free_bytes,
            } => write!(
                f,
                "stream over budget: want {want_bytes} bytes, free {free_bytes}"
            ),
            Self::JobCap { limit } => write!(f, "stream job cap reached: {limit}"),
            Self::Unknown(id) => write!(f, "stream unknown job {id:?}"),
        }
    }
}

impl std::error::Error for StreamError {}

impl StreamError {
    /// Browser-visible refusal for this error; the refused job stages nothing.
    /// Callers pass the attempted job id so no refusal is ever silent.
    pub fn refused_event(&self, id: &str) -> StreamEvent {
        StreamEvent::Refused {
            id: bound_detail(id),
            reason: bound_detail(&self.to_string()),
        }
    }
}

/// Truncate echoed text to at most [`MAX_STREAM_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    if text.chars().count() <= MAX_STREAM_DETAIL_CHARS {
        text.to_string()
    } else {
        text.chars().take(MAX_STREAM_DETAIL_CHARS).collect()
    }
}

/// One async bundle read: payload identity plus its byte cost and priority.
/// Higher `priority` streams first; equal priorities complete in ascending id
/// order. `bytes` reserves budget at request time and stays reserved until
/// the job completes, is cancelled, or the resident is evicted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamJob {
    /// Blake3 hex digest of the payload; also the tie-break key.
    pub id: StreamId,
    /// Bundle entry name, non-empty, at most [`MAX_STREAM_NAME_CHARS`] chars.
    pub name: String,
    /// Which payload encoding the bytes hold (mirrors the bundle entry kind).
    pub kind: StreamKind,
    /// Payload bytes reserved under the budget; units: bytes.
    pub bytes: u64,
    /// Higher value streams first.
    pub priority: u8,
}

impl StreamJob {
    /// Build a validated job; invalid names and byte counts are typed errors.
    pub fn new(
        id: StreamId,
        name: &str,
        kind: StreamKind,
        bytes: u64,
        priority: u8,
    ) -> Result<Self, StreamError> {
        if name.is_empty() || name.chars().count() > MAX_STREAM_NAME_CHARS {
            return Err(StreamError::BadName(bound_detail(name)));
        }
        if bytes == 0 || bytes > MAX_STREAM_PAYLOAD_BYTES as u64 {
            return Err(StreamError::TooLarge {
                count: bytes as usize,
            });
        }
        Ok(Self {
            id,
            name: name.to_string(),
            kind,
            bytes,
            priority,
        })
    }
}

/// Memory plus slot budget for one queue. Units: bytes for `bytes`, admitted
/// jobs (pending plus resident) for `jobs`, which also bounds queue scans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamBudget {
    /// Cap on reserved plus resident payload bytes.
    pub bytes: u64,
    /// Cap on admitted jobs (pending plus resident).
    pub jobs: u32,
}

impl StreamBudget {
    /// Budget with a byte cap and a job-slot cap.
    pub fn new(bytes: u64, jobs: u32) -> Self {
        Self { bytes, jobs }
    }
}

/// Progress events for the asset browser, in emission order. `publish` emits at
/// most three events per touched job per call (`Started` once, then
/// `Progress`, then `Completed`); untouched jobs emit nothing, so driving the
/// queue never blocks the tick beyond the frame byte cap the caller passes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// Job admitted and queued; its bytes are now reserved.
    Queued {
        /// Admitted job id.
        id: StreamId,
    },
    /// Job advanced for the first time.
    Started {
        /// Job id.
        id: StreamId,
    },
    /// Job advanced this frame; units: bytes.
    Progress {
        /// Job id.
        id: StreamId,
        /// Bytes streamed so far.
        done_bytes: u64,
        /// Total payload bytes.
        total_bytes: u64,
    },
    /// Job fully streamed and resident; also the cache-hit answer when the
    /// requested id is already resident (no new bytes reserved).
    Completed {
        /// Job id.
        id: StreamId,
    },
    /// Explicit refusal: the job staged nothing. Every [`StreamError`] maps
    /// here through [`StreamError::refused_event`]; jobs are never silently
    /// dropped.
    Refused {
        /// Attempted job id, bounded.
        id: String,
        /// Bounded human-readable reason.
        reason: String,
    },
    /// Resident evicted by the consumer (distance/use); bytes freed.
    Evicted {
        /// Evicted id.
        id: StreamId,
        /// Bytes returned to the budget.
        freed_bytes: u64,
    },
    /// Pending job cancelled before completion; its reservation is freed.
    Cancelled {
        /// Cancelled id.
        id: StreamId,
    },
}

/// One pending slot: the job plus in-memory progress markers.
#[derive(Clone, Debug)]
struct Pending {
    job: StreamJob,
    done: u64,
    started: bool,
}

/// One resident slot: completed payload identity plus its byte cost.
#[derive(Clone, Debug)]
struct Resident {
    id: StreamId,
    bytes: u64,
}

/// Deterministic completion order: higher priority first, then id ascending.
/// Ids are unique, so this is a total order independent of request sequence.
fn completion_order(left: &Pending, right: &Pending) -> Ordering {
    right
        .job
        .priority
        .cmp(&left.job.priority)
        .then_with(|| left.job.id.cmp(&right.job.id))
}

/// Async streaming queue over bundle reads: priority ordering, a memory
/// budget with a high-water mark plus over-budget refusal, chunked progress
/// events published at frame boundaries, and explicit eviction. Single-thread
/// driven: workers do IO, the owner calls `publish` with a per-frame byte cap
/// so the tick never waits. No wall clock, no floats, stable sort order only.
#[derive(Clone, Debug)]
pub struct StreamQueue {
    budget: StreamBudget,
    pending: Vec<Pending>,
    resident: Vec<Resident>,
    high_water_bytes: u64,
}

impl StreamQueue {
    /// Empty queue under `budget`; nothing reserved, high-water zero.
    pub fn new(budget: StreamBudget) -> Self {
        Self {
            budget,
            pending: Vec::new(),
            resident: Vec::new(),
            high_water_bytes: 0,
        }
    }

    /// Active budget (byte cap plus job-slot cap).
    pub fn budget(&self) -> StreamBudget {
        self.budget
    }

    /// Reserved plus resident bytes; units: bytes. Invariant: never above
    /// the byte budget, since admission refuses over-budget jobs.
    pub fn used_bytes(&self) -> u64 {
        let mut used = 0u64;
        for slot in &self.pending {
            used = used.saturating_add(slot.job.bytes);
        }
        for slot in &self.resident {
            used = used.saturating_add(slot.bytes);
        }
        used
    }

    /// Peak of [`StreamQueue::used_bytes`] since construction; never falls.
    /// Units: bytes.
    pub fn high_water_bytes(&self) -> u64 {
        self.high_water_bytes
    }

    /// Bytes still admittable; units: bytes.
    pub fn remaining_bytes(&self) -> u64 {
        self.budget.bytes.saturating_sub(self.used_bytes())
    }

    /// Job slots still admittable (pending plus resident count against it).
    pub fn remaining_jobs(&self) -> u64 {
        u64::from(self.budget.jobs).saturating_sub(self.admitted() as u64)
    }

    /// Count of queued jobs awaiting completion.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// True while the id is queued but not resident: the consumer shows a
    /// placeholder (tinted wireframe, never final-looking, never captured).
    pub fn is_placeholder(&self, id: &StreamId) -> bool {
        self.is_pending(id)
    }

    /// True while the id is queued but not resident.
    pub fn is_pending(&self, id: &StreamId) -> bool {
        self.pending.iter().any(|slot| &slot.job.id == id)
    }

    /// True once the id completed and has not been evicted.
    pub fn is_resident(&self, id: &StreamId) -> bool {
        self.resident.iter().any(|slot| &slot.id == id)
    }

    /// Streamed bytes versus total for one id: pending jobs report partial
    /// progress, resident ids report full, unknown ids report `None`.
    /// Units: bytes.
    pub fn progress_of(&self, id: &StreamId) -> Option<(u64, u64)> {
        if let Some(slot) = self.pending.iter().find(|slot| &slot.job.id == id) {
            return Some((slot.done, slot.job.bytes));
        }
        self.resident
            .iter()
            .find(|slot| &slot.id == id)
            .map(|slot| (slot.bytes, slot.bytes))
    }

    /// Pending ids in the deterministic completion order (priority first,
    /// then id ascending) for the browser list.
    pub fn pending_order(&self) -> Vec<StreamId> {
        let mut order: Vec<&Pending> = self.pending.iter().collect();
        order.sort_by(|left, right| completion_order(left, right));
        order.iter().map(|slot| slot.job.id.clone()).collect()
    }

    /// Queue one bundle read. Resident re-requests are cache hits that answer
    /// [`StreamEvent::Completed`] inline with no new reservation; pending
    /// re-requests fail [`StreamError::Duplicate`]; jobs past either budget
    /// fail [`StreamError::OverBudget`] or [`StreamError::JobCap`]. Every
    /// failure stages nothing and maps to [`StreamEvent::Refused`].
    pub fn request(&mut self, job: StreamJob) -> Result<StreamEvent, StreamError> {
        if self.is_resident(&job.id) {
            return Ok(StreamEvent::Completed { id: job.id });
        }
        if self.is_pending(&job.id) {
            return Err(StreamError::Duplicate(job.id.as_str().to_string()));
        }
        if self.admitted() >= self.budget.jobs as usize {
            return Err(StreamError::JobCap {
                limit: self.budget.jobs,
            });
        }
        let used = self.used_bytes();
        if used.saturating_add(job.bytes) > self.budget.bytes {
            return Err(StreamError::OverBudget {
                want_bytes: job.bytes,
                free_bytes: self.budget.bytes.saturating_sub(used),
            });
        }
        let id = job.id.clone();
        self.pending.push(Pending {
            job,
            done: 0,
            started: false,
        });
        self.high_water_bytes = self.high_water_bytes.max(self.used_bytes());
        Ok(StreamEvent::Queued { id })
    }

    /// Advance queued jobs by at most `frame_bytes` at a frame boundary, in
    /// deterministic completion order. Partial jobs emit `Progress` and stay
    /// pending; finished jobs become resident and emit `Completed`. A zero
    /// cap emits nothing and touches nothing.
    pub fn publish(&mut self, frame_bytes: u64) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if frame_bytes == 0 || self.pending.is_empty() {
            return events;
        }
        self.pending.sort_by(completion_order);
        let mut left = frame_bytes;
        let mut index = 0;
        while index < self.pending.len() && left > 0 {
            let total = self.pending[index].job.bytes;
            let chunk = total.saturating_sub(self.pending[index].done).min(left);
            if !self.pending[index].started {
                self.pending[index].started = true;
                events.push(StreamEvent::Started {
                    id: self.pending[index].job.id.clone(),
                });
            }
            self.pending[index].done = self.pending[index].done.saturating_add(chunk);
            left = left.saturating_sub(chunk);
            events.push(StreamEvent::Progress {
                id: self.pending[index].job.id.clone(),
                done_bytes: self.pending[index].done,
                total_bytes: total,
            });
            if self.pending[index].done >= total {
                let done = self.pending.remove(index);
                self.resident.push(Resident {
                    id: done.job.id.clone(),
                    bytes: total,
                });
                events.push(StreamEvent::Completed { id: done.job.id });
            } else {
                index += 1;
            }
        }
        events
    }

    /// Drop a pending job before completion and free its reservation.
    pub fn cancel(&mut self, id: &StreamId) -> Result<StreamEvent, StreamError> {
        let at = self
            .pending
            .iter()
            .position(|slot| &slot.job.id == id)
            .ok_or_else(|| StreamError::Unknown(id.as_str().to_string()))?;
        let done = self.pending.remove(at);
        Ok(StreamEvent::Cancelled { id: done.job.id })
    }

    /// Free one resident payload chosen by the consumer (distance/use) and
    /// return its bytes to the budget. Pending ids need `cancel`, not evict.
    pub fn evict(&mut self, id: &StreamId) -> Result<StreamEvent, StreamError> {
        let at = self
            .resident
            .iter()
            .position(|slot| &slot.id == id)
            .ok_or_else(|| StreamError::Unknown(id.as_str().to_string()))?;
        let done = self.resident.remove(at);
        Ok(StreamEvent::Evicted {
            id: done.id,
            freed_bytes: done.bytes,
        })
    }

    /// Admitted job count (pending plus resident) against the slot budget.
    fn admitted(&self) -> usize {
        self.pending.len().saturating_add(self.resident.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_id(seed: u8) -> StreamId {
        StreamId::parse(&format!("{seed:02x}").repeat(32)).unwrap()
    }

    fn job(seed: u8, bytes: u64, priority: u8) -> StreamJob {
        StreamJob::new(
            hex_id(seed),
            &format!("entry-{seed:02}"),
            StreamKind::Mesh,
            bytes,
            priority,
        )
        .unwrap()
    }

    fn completed_ids(events: &[StreamEvent]) -> Vec<StreamId> {
        events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Completed { id } => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    fn queue(bytes: u64, jobs: u32) -> StreamQueue {
        StreamQueue::new(StreamBudget::new(bytes, jobs))
    }

    #[test]
    fn completion_order_is_priority_then_id() {
        let mut stream = queue(1 << 20, 8);
        // Requested scrambled: lowest priority first, highest last.
        stream.request(job(3, 10, 1)).unwrap();
        stream.request(job(1, 10, 9)).unwrap();
        stream.request(job(2, 10, 9)).unwrap();
        let events = stream.publish(u64::MAX);
        // Equal priority 9 breaks on ascending id: seed 1 before seed 2.
        assert_eq!(
            completed_ids(&events),
            vec![hex_id(1), hex_id(2), hex_id(3)]
        );
        assert_eq!(stream.pending_count(), 0);
    }

    #[test]
    fn order_ignores_request_sequence() {
        let mut first = queue(1 << 20, 8);
        let mut second = queue(1 << 20, 8);
        for seed in [3u8, 1, 2] {
            first.request(job(seed, 40, seed)).unwrap();
        }
        for seed in [2u8, 3, 1] {
            second.request(job(seed, 40, seed)).unwrap();
        }
        assert_eq!(first.publish(25), second.publish(25));
        assert_eq!(first.publish(u64::MAX), second.publish(u64::MAX));
        assert_eq!(first.pending_order(), second.pending_order());
    }

    #[test]
    fn over_budget_refusal_stages_nothing() {
        let mut stream = queue(100, 8);
        assert_eq!(
            stream.request(job(1, 60, 5)).unwrap(),
            StreamEvent::Queued { id: hex_id(1) }
        );
        let err = stream.request(job(2, 50, 9)).unwrap_err();
        assert_eq!(
            err,
            StreamError::OverBudget {
                want_bytes: 50,
                free_bytes: 40,
            }
        );
        // Never silent: the refused id is nowhere and maps to a browser event.
        assert!(!stream.is_pending(&hex_id(2)));
        assert!(!stream.is_resident(&hex_id(2)));
        assert!(matches!(
            err.refused_event(hex_id(2).as_str()),
            StreamEvent::Refused { .. }
        ));
        assert_eq!(stream.used_bytes(), 60);
        assert_eq!(stream.high_water_bytes(), 60);
        assert_eq!(stream.remaining_bytes(), 40);
    }

    #[test]
    fn high_water_never_falls_after_evict() {
        let mut stream = queue(200, 8);
        stream.request(job(1, 60, 5)).unwrap();
        assert_eq!(stream.high_water_bytes(), 60);
        stream.publish(u64::MAX);
        stream.request(job(2, 30, 5)).unwrap();
        assert_eq!(stream.used_bytes(), 90);
        assert_eq!(stream.high_water_bytes(), 90);
        let evicted = stream.evict(&hex_id(1)).unwrap();
        assert_eq!(
            evicted,
            StreamEvent::Evicted {
                id: hex_id(1),
                freed_bytes: 60,
            }
        );
        assert_eq!(stream.used_bytes(), 30);
        assert_eq!(stream.high_water_bytes(), 90);
        assert!(!stream.is_resident(&hex_id(1)));
    }

    #[test]
    fn progress_chunks_until_complete() {
        let mut stream = queue(1 << 20, 8);
        stream.request(job(1, 100, 5)).unwrap();
        assert!(stream.is_placeholder(&hex_id(1)));
        assert!(!stream.is_resident(&hex_id(1)));

        let first = stream.publish(30);
        assert_eq!(
            first,
            vec![
                StreamEvent::Started { id: hex_id(1) },
                StreamEvent::Progress {
                    id: hex_id(1),
                    done_bytes: 30,
                    total_bytes: 100,
                },
            ]
        );
        assert_eq!(stream.progress_of(&hex_id(1)), Some((30, 100)));

        // No second Started event; still a placeholder, not resident.
        let second = stream.publish(30);
        assert_eq!(
            second,
            vec![StreamEvent::Progress {
                id: hex_id(1),
                done_bytes: 60,
                total_bytes: 100,
            }]
        );
        assert!(stream.is_placeholder(&hex_id(1)));

        let third = stream.publish(40);
        assert_eq!(
            third,
            vec![
                StreamEvent::Progress {
                    id: hex_id(1),
                    done_bytes: 100,
                    total_bytes: 100,
                },
                StreamEvent::Completed { id: hex_id(1) },
            ]
        );
        assert!(!stream.is_placeholder(&hex_id(1)));
        assert!(stream.is_resident(&hex_id(1)));
        assert_eq!(stream.progress_of(&hex_id(1)), Some((100, 100)));
    }

    #[test]
    fn frame_cap_spreads_across_jobs_in_order() {
        let mut stream = queue(1 << 20, 8);
        stream.request(job(2, 100, 1)).unwrap();
        stream.request(job(1, 100, 9)).unwrap();
        let events = stream.publish(150);
        // High priority finishes first; the leftover 50 bytes start the next.
        assert_eq!(
            events,
            vec![
                StreamEvent::Started { id: hex_id(1) },
                StreamEvent::Progress {
                    id: hex_id(1),
                    done_bytes: 100,
                    total_bytes: 100,
                },
                StreamEvent::Completed { id: hex_id(1) },
                StreamEvent::Started { id: hex_id(2) },
                StreamEvent::Progress {
                    id: hex_id(2),
                    done_bytes: 50,
                    total_bytes: 100,
                },
            ]
        );
        let rest = stream.publish(50);
        assert_eq!(completed_ids(&rest), vec![hex_id(2)]);
    }

    #[test]
    fn zero_frame_publish_touches_nothing() {
        let mut stream = queue(1 << 20, 8);
        stream.request(job(1, 10, 5)).unwrap();
        assert!(stream.publish(0).is_empty());
        assert_eq!(stream.progress_of(&hex_id(1)), Some((0, 10)));
        assert!(stream.is_pending(&hex_id(1)));
    }

    #[test]
    fn duplicate_pending_keeps_first_slot() {
        let mut stream = queue(1 << 20, 8);
        stream.request(job(1, 60, 5)).unwrap();
        let err = stream
            .request(StreamJob::new(hex_id(1), "other", StreamKind::Clip, 10, 9).unwrap())
            .unwrap_err();
        assert_eq!(err, StreamError::Duplicate(hex_id(1).as_str().to_string()));
        assert_eq!(stream.pending_count(), 1);
        assert_eq!(stream.used_bytes(), 60);
        assert_eq!(stream.pending_order(), vec![hex_id(1)]);
    }

    #[test]
    fn resident_rerequest_is_a_cache_hit() {
        let mut stream = queue(1 << 20, 8);
        stream.request(job(1, 60, 5)).unwrap();
        stream.publish(u64::MAX);
        // Re-requested chunks hit the cache: inline completion, no new bytes.
        assert_eq!(
            stream.request(job(1, 60, 5)).unwrap(),
            StreamEvent::Completed { id: hex_id(1) }
        );
        assert_eq!(stream.used_bytes(), 60);
        assert_eq!(stream.pending_count(), 0);
    }

    #[test]
    fn job_cap_refused_with_event() {
        let mut stream = queue(1 << 20, 1);
        stream.request(job(1, 10, 5)).unwrap();
        assert_eq!(stream.remaining_jobs(), 0);
        let err = stream.request(job(2, 10, 5)).unwrap_err();
        assert_eq!(err, StreamError::JobCap { limit: 1 });
        assert!(matches!(
            err.refused_event(hex_id(2).as_str()),
            StreamEvent::Refused { .. }
        ));
        assert!(!stream.is_pending(&hex_id(2)));
    }

    #[test]
    fn cancel_frees_reservation_unknown_fails() {
        let mut stream = queue(100, 8);
        stream.request(job(1, 60, 5)).unwrap();
        assert_eq!(
            stream.cancel(&hex_id(1)).unwrap(),
            StreamEvent::Cancelled { id: hex_id(1) }
        );
        assert_eq!(stream.used_bytes(), 0);
        // The freed bytes admit the job that never fit before.
        stream.request(job(2, 100, 5)).unwrap();
        assert_eq!(
            stream.cancel(&hex_id(9)).unwrap_err(),
            StreamError::Unknown(hex_id(9).as_str().to_string())
        );
        assert_eq!(
            stream.evict(&hex_id(9)).unwrap_err(),
            StreamError::Unknown(hex_id(9).as_str().to_string())
        );
        // Resident ids evict; pending ids do not.
        assert!(matches!(
            stream.evict(&hex_id(2)).unwrap_err(),
            StreamError::Unknown(_)
        ));
    }

    #[test]
    fn invalid_input_fails_typed_and_stages_nothing() {
        for bad in ["", "zz", &"0".repeat(63), &"0".repeat(65), &"AA".repeat(32)] {
            assert!(
                matches!(StreamId::parse(bad).unwrap_err(), StreamError::BadId(_)),
                "id {bad:?} must fail"
            );
        }
        let id = hex_id(1);
        assert!(matches!(
            StreamJob::new(id.clone(), "", StreamKind::Mesh, 10, 1).unwrap_err(),
            StreamError::BadName(_)
        ));
        assert!(matches!(
            StreamJob::new(
                id.clone(),
                &"n".repeat(MAX_STREAM_NAME_CHARS + 1),
                StreamKind::Mesh,
                10,
                1
            )
            .unwrap_err(),
            StreamError::BadName(_)
        ));
        assert!(matches!(
            StreamJob::new(id.clone(), "ok", StreamKind::Mesh, 0, 1).unwrap_err(),
            StreamError::TooLarge { .. }
        ));
        assert!(matches!(
            StreamJob::new(
                id.clone(),
                "ok",
                StreamKind::Mesh,
                MAX_STREAM_PAYLOAD_BYTES as u64 + 1,
                1
            )
            .unwrap_err(),
            StreamError::TooLarge { .. }
        ));
        let stream = queue(1 << 20, 8);
        assert_eq!(stream.pending_count(), 0);
        assert_eq!(stream.used_bytes(), 0);
        assert_eq!(stream.high_water_bytes(), 0);
    }

    #[test]
    fn browser_snapshots_are_stable() {
        let mut stream = queue(1 << 20, 8);
        assert_eq!(stream.budget(), StreamBudget::new(1 << 20, 8));
        stream.request(job(2, 10, 1)).unwrap();
        stream.request(job(1, 10, 9)).unwrap();
        // Pending snapshot follows completion order, not request order.
        assert_eq!(stream.pending_order(), vec![hex_id(1), hex_id(2)]);
        stream.publish(u64::MAX);
        assert!(stream.pending_order().is_empty());
        assert_eq!(stream.progress_of(&hex_id(9)), None);
    }

    #[test]
    fn every_kind_and_error_displays() {
        for kind in [
            StreamKind::Mesh,
            StreamKind::Material,
            StreamKind::Texture,
            StreamKind::Skin,
            StreamKind::Clip,
        ] {
            assert!(!format!("{kind}").is_empty());
        }
        assert_eq!(format!("{}", StreamKind::Mesh), "mesh");
        assert_eq!(hex_id(1).to_string(), hex_id(1).as_str());
        let errors = [
            StreamError::BadId("x".to_string()),
            StreamError::BadName("x".to_string()),
            StreamError::Duplicate("x".to_string()),
            StreamError::TooLarge { count: 1 },
            StreamError::OverBudget {
                want_bytes: 9,
                free_bytes: 1,
            },
            StreamError::JobCap { limit: 1 },
            StreamError::Unknown("x".to_string()),
        ];
        for err in &errors {
            assert!(!format!("{err}").is_empty());
            assert!(!format!("{err:?}").is_empty());
            assert!(matches!(
                err.refused_event("ab"),
                StreamEvent::Refused { .. }
            ));
        }
        assert_eq!(
            format!("{}", StreamError::JobCap { limit: 3 }),
            "stream job cap reached: 3"
        );
    }
}
