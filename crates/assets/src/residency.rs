//! residency: one priority-driven memory budget for every resident asset
//! class over the AS2 streaming queue.
//!
//! Textures, meshes, meshlets and acceleration structures share one byte
//! budget, so a texture can never hide a mesh overrun. Payloads enter through
//! [`Residency::request`], which forwards the job to the stream queue; a job
//! that does not fit may evict resident payloads of strictly lower priority,
//! lowest priority first, then least recently used, then id ascending. A
//! request never evicts a peer or a higher priority, so a refusal costs
//! nothing resident.
//!
//! Work per call is bounded: one plan picks at most
//! [`MAX_EVICTIONS_PER_CALL`] victims and sorts at most the queue slot cap
//! entries, so one frame cannot stall on eviction. Ordering is deterministic
//! (no clock, no float, no thread), so the same requests plus the same
//! `publish` bytes evict the same payloads on every PC.

use std::fmt;

use crate::stream::{StreamBudget, StreamError, StreamEvent, StreamId, StreamJob, StreamQueue};

/// Bytes in 4.5 GiB: the residency budget of the RTX 3050.
pub const BUDGET_BYTES_RTX_3050: u64 = 4_831_838_208;
/// Job slots on the RTX 3050 manager (pending plus resident payloads).
pub const JOBS_RTX_3050: u32 = 512;
/// Most resident payloads one call may evict; bounds frame work.
pub const MAX_EVICTIONS_PER_CALL: usize = 64;

/// Resident class sharing the one budget.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum ResClass {
    /// Cooked texture bytes with mips.
    Texture = 1,
    /// Cooked mesh vertex and index bytes.
    Mesh = 2,
    /// Meshlet cluster bytes with their bounds.
    Meshlet = 3,
    /// Bottom and top level acceleration structures for ray work.
    AccelStructure = 4,
}

impl ResClass {
    /// Every class, in the report order used by [`Residency::class_bytes`].
    pub const ALL: [Self; 4] = [
        Self::Texture,
        Self::Mesh,
        Self::Meshlet,
        Self::AccelStructure,
    ];

    /// Lowercase class name for receipts and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Texture => "texture",
            Self::Mesh => "mesh",
            Self::Meshlet => "meshlet",
            Self::AccelStructure => "accel_structure",
        }
    }
}

impl fmt::Display for ResClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One payload tracked by the manager: identity, class, priority and use.
#[derive(Clone, Debug)]
struct Entry {
    id: StreamId,
    class: ResClass,
    priority: u8,
    bytes: u64,
    last_used: u64,
    resident: bool,
}

/// One payload dropped to make room: what went, why it was picked, bytes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eviction {
    /// Evicted payload id.
    pub id: StreamId,
    /// Class of the evicted payload.
    pub class: ResClass,
    /// Priority it carried; always below the requesting job's priority.
    pub priority: u8,
    /// Bytes returned to the budget.
    pub freed_bytes: u64,
}

/// Answer to an admitted request: the id plus everything evicted for it, in
/// eviction order (lowest priority first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admitted {
    /// Admitted payload id.
    pub id: StreamId,
    /// Payloads evicted to make room; empty for a cache hit or a free fit.
    pub evicted: Vec<Eviction>,
}

/// Why a residency call was refused. Every refusal stages nothing new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResidencyError {
    /// The stream queue refused: bad id, duplicate, over budget, job cap.
    Stream(StreamError),
    /// Making room needs more evictions than one call may do, so the frame
    /// stays bounded and the caller retries on the next frame.
    EvictCap {
        /// Evictions one call is allowed.
        limit: usize,
    },
}

impl fmt::Display for ResidencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stream(err) => write!(f, "residency refused: {err}"),
            Self::EvictCap { limit } => {
                write!(
                    f,
                    "residency eviction plan needs more than {limit} evictions"
                )
            }
        }
    }
}

impl std::error::Error for ResidencyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Stream(err) => Some(err),
            Self::EvictCap { .. } => None,
        }
    }
}

impl From<StreamError> for ResidencyError {
    fn from(err: StreamError) -> Self {
        Self::Stream(err)
    }
}

/// One budget over the stream queue plus the class, priority and use record
/// every eviction decision needs.
///
/// Invariants: `used_bytes` never exceeds `budget_bytes` because admission
/// goes through the queue; `resident_bytes` is the sum of [`Eviction`]-able
/// payloads and equals the queue's resident bytes; the four class totals sum
/// to `resident_bytes`. Single-thread driven at frame boundaries.
#[derive(Clone, Debug)]
pub struct Residency {
    queue: StreamQueue,
    entries: Vec<Entry>,
    tick: u64,
    evictions: u64,
}

impl Residency {
    /// Empty manager with one byte budget and a queue slot cap.
    pub fn new(bytes: u64, jobs: u32) -> Self {
        Self {
            queue: StreamQueue::new(StreamBudget::new(bytes, jobs)),
            entries: Vec::new(),
            tick: 0,
            evictions: 0,
        }
    }

    /// Manager at the RTX 3050 budget: 4.5 GiB over 512 slots.
    pub fn rtx_3050() -> Self {
        Self::new(BUDGET_BYTES_RTX_3050, JOBS_RTX_3050)
    }

    /// Byte budget shared by every class.
    pub fn budget_bytes(&self) -> u64 {
        self.queue.budget().bytes
    }

    /// Reserved plus resident bytes of every class; never above the budget.
    pub fn used_bytes(&self) -> u64 {
        self.queue.used_bytes()
    }

    /// Bytes held by completed payloads; the only eviction candidates.
    pub fn resident_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.resident)
            .map(|entry| entry.bytes)
            .fold(0u64, u64::saturating_add)
    }

    /// Bytes held by payloads still streaming.
    pub fn pending_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| !entry.resident)
            .map(|entry| entry.bytes)
            .fold(0u64, u64::saturating_add)
    }

    /// Bytes still admittable under the shared budget.
    pub fn remaining_bytes(&self) -> u64 {
        self.queue.remaining_bytes()
    }

    /// Peak of [`Residency::used_bytes`] since construction; never falls.
    pub fn high_water_bytes(&self) -> u64 {
        self.queue.high_water_bytes()
    }

    /// Completed and not yet evicted payloads.
    pub fn resident_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.resident).count()
    }

    /// Payloads still streaming.
    pub fn pending_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.resident).count()
    }

    /// Total payloads dropped by plan or by [`Residency::evict`].
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// True once the id completed and has not been evicted.
    pub fn is_resident(&self, id: &StreamId) -> bool {
        self.entries
            .iter()
            .any(|entry| &entry.id == id && entry.resident)
    }

    /// Class recorded for one id while it is known to the manager.
    pub fn class_of(&self, id: &StreamId) -> Option<ResClass> {
        self.entries
            .iter()
            .find(|entry| &entry.id == id)
            .map(|entry| entry.class)
    }

    /// Resident bytes of one class; the four totals sum to `resident_bytes`.
    pub fn class_bytes(&self, class: ResClass) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.resident && entry.class == class)
            .map(|entry| entry.bytes)
            .fold(0u64, u64::saturating_add)
    }

    /// Mark a resident payload as used now, so equal priorities evict it
    /// last. False when the id is not resident.
    pub fn touch(&mut self, id: &StreamId) -> bool {
        let tick = self.bump_tick();
        match self
            .entries
            .iter_mut()
            .find(|entry| &entry.id == id && entry.resident)
        {
            Some(entry) => {
                entry.last_used = tick;
                true
            }
            None => false,
        }
    }

    /// Ask the shared budget for one payload of `class`.
    ///
    /// A resident id is a cache hit: its use is stamped and no bytes move.
    /// A pending id is [`StreamError::Duplicate`]. When the payload does not
    /// fit, resident payloads of strictly lower priority are evicted first,
    /// then the queue admits the job. A request that lower priorities cannot
    /// cover evicts nothing and returns the queue's typed refusal.
    pub fn request(&mut self, job: StreamJob, class: ResClass) -> Result<Admitted, ResidencyError> {
        if let Some(at) = self.entries.iter().position(|entry| entry.id == job.id) {
            if !self.entries[at].resident {
                return Err(ResidencyError::Stream(StreamError::Duplicate(
                    job.id.as_str().to_string(),
                )));
            }
            let last_used = self.bump_tick();
            self.entries[at].last_used = last_used;
            return Ok(Admitted {
                id: job.id,
                evicted: Vec::new(),
            });
        }
        let id = job.id.clone();
        let priority = job.priority;
        if job.bytes > self.budget_bytes() {
            return Err(ResidencyError::Stream(StreamError::OverBudget {
                want_bytes: job.bytes,
                free_bytes: self.remaining_bytes(),
            }));
        }
        let need_bytes = job.bytes.saturating_sub(self.remaining_bytes());
        let need_slots = usize::from(self.queue.remaining_jobs() == 0);
        let victims = self.plan_evictions(need_bytes, need_slots, priority)?;
        let mut evicted = Vec::new();
        for victim in &victims {
            if let Some(eviction) = self.evict_one(victim) {
                evicted.push(eviction);
            }
        }
        self.queue.request(job.clone())?;
        let resident = self.queue.is_resident(&id);
        let last_used = if resident { self.bump_tick() } else { 0 };
        self.entries.push(Entry {
            id: id.clone(),
            class,
            priority,
            bytes: job.bytes,
            last_used,
            resident,
        });
        Ok(Admitted { id, evicted })
    }

    /// Advance the queue by at most `frame_bytes`, then mark completed
    /// payloads resident. Raw queue events are returned unchanged, so the
    /// browser keeps its Started/Progress/Completed order.
    pub fn publish(&mut self, frame_bytes: u64) -> Vec<StreamEvent> {
        let events = self.queue.publish(frame_bytes);
        let mut tick = self.tick;
        for event in &events {
            if let StreamEvent::Completed { id } = event {
                tick = tick.saturating_add(1);
                if let Some(entry) = self.entries.iter_mut().find(|entry| &entry.id == id) {
                    entry.resident = true;
                    entry.last_used = tick;
                }
            }
        }
        self.tick = tick;
        events
    }

    /// Drop one resident payload now and return its bytes to the budget.
    /// Unknown and pending ids are refused; AS2 cancels pending jobs.
    pub fn evict(&mut self, id: &StreamId) -> Result<Eviction, ResidencyError> {
        self.evict_one(id)
            .ok_or_else(|| ResidencyError::Stream(StreamError::Unknown(id.as_str().to_string())))
    }

    /// Choose the lowest-priority resident payloads that free `need_bytes`
    /// and `need_slots` jobs, all strictly below `priority`. Order: priority
    /// ascending, then least recently used, then id ascending. Empty when the
    /// need is zero or nothing lower can cover it; [`ResidencyError::EvictCap`]
    /// when covering it would take more than [`MAX_EVICTIONS_PER_CALL`]
    /// evictions, checked before anything is dropped.
    fn plan_evictions(
        &self,
        need_bytes: u64,
        need_slots: usize,
        priority: u8,
    ) -> Result<Vec<StreamId>, ResidencyError> {
        if need_bytes == 0 && need_slots == 0 {
            return Ok(Vec::new());
        }
        let mut candidates: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.resident && entry.priority < priority)
            .collect();
        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then(left.last_used.cmp(&right.last_used))
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut picked = Vec::new();
        let mut freed = 0u64;
        for entry in candidates {
            if freed >= need_bytes && picked.len() >= need_slots {
                break;
            }
            if picked.len() >= MAX_EVICTIONS_PER_CALL {
                return Err(ResidencyError::EvictCap {
                    limit: MAX_EVICTIONS_PER_CALL,
                });
            }
            freed = freed.saturating_add(entry.bytes);
            picked.push(entry.id.clone());
        }
        if freed < need_bytes || picked.len() < need_slots {
            picked.clear();
        }
        Ok(picked)
    }

    /// Drop one resident entry from the queue and from the record together.
    fn evict_one(&mut self, id: &StreamId) -> Option<Eviction> {
        let at = self
            .entries
            .iter()
            .position(|entry| &entry.id == id && entry.resident)?;
        match self.queue.evict(id) {
            Ok(StreamEvent::Evicted { .. }) => {}
            _ => return None,
        }
        let entry = self.entries.remove(at);
        self.evictions = self.evictions.saturating_add(1);
        Some(Eviction {
            id: entry.id,
            class: entry.class,
            priority: entry.priority,
            freed_bytes: entry.bytes,
        })
    }

    /// Stamp one use; saturates so a very long run keeps a stable order.
    fn bump_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::StreamKind;
    use std::time::{Duration, Instant};

    fn hex_id(seed: u32) -> StreamId {
        StreamId::parse(&format!("{seed:08x}").repeat(8)).unwrap()
    }

    fn job(seed: u32, bytes: u64, priority: u8) -> StreamJob {
        StreamJob::new(
            hex_id(seed),
            &format!("entry-{seed:08}"),
            StreamKind::Mesh,
            bytes,
            priority,
        )
        .unwrap()
    }

    fn admit(resident: &mut Residency, seed: u32, bytes: u64, priority: u8, class: ResClass) {
        resident
            .request(job(seed, bytes, priority), class)
            .expect("admits");
        resident.publish(u64::MAX);
        assert!(resident.is_resident(&hex_id(seed)));
    }

    #[test]
    fn rtx_3050_budget_is_four_and_a_half_gib() {
        assert_eq!(BUDGET_BYTES_RTX_3050, 4_831_838_208);
        assert_eq!(BUDGET_BYTES_RTX_3050, 4_608 * 1024 * 1024);
        let resident = Residency::rtx_3050();
        assert_eq!(resident.budget_bytes(), BUDGET_BYTES_RTX_3050);
        assert_eq!(resident.used_bytes(), 0);
        assert_eq!(resident.resident_count(), 0);
    }

    #[test]
    fn evictions_follow_priority_lowest_first_then_least_recent() {
        let mut resident = Residency::new(1000, 8);
        for seed in 1..=4u32 {
            admit(&mut resident, seed, 200, 5, ResClass::Mesh);
        }
        assert_eq!(resident.used_bytes(), 800);
        // 200 bytes are free, a 500-byte texture needs 300 more: two 200-byte
        // peers of priority 5 go, in completion order (id ascending).
        let admitted = resident
            .request(job(9, 500, 9), ResClass::Texture)
            .expect("higher priority fits after eviction");
        let ids: Vec<StreamId> = admitted.evicted.iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids, vec![hex_id(1), hex_id(2)], "least recently used first");
        assert!(admitted.evicted.iter().all(|e| e.priority == 5));
        resident.publish(u64::MAX);
        assert_eq!(resident.resident_count(), 3);
        assert_eq!(resident.used_bytes(), 900);
        assert!(resident.used_bytes() <= resident.budget_bytes());
    }

    #[test]
    fn evictions_take_lowest_priority_before_recent_use() {
        let mut resident = Residency::new(1000, 8);
        admit(&mut resident, 1, 300, 1, ResClass::Mesh);
        admit(&mut resident, 2, 300, 3, ResClass::AccelStructure);
        admit(&mut resident, 3, 300, 7, ResClass::Meshlet);
        // Priority 1 is the newest by use and still leaves first.
        assert!(resident.touch(&hex_id(1)));
        let admitted = resident
            .request(job(4, 500, 9), ResClass::Texture)
            .expect("higher priority fits after eviction");
        let order: Vec<u8> = admitted.evicted.iter().map(|e| e.priority).collect();
        assert_eq!(order, vec![1, 3]);
        let classes: Vec<ResClass> = admitted.evicted.iter().map(|e| e.class).collect();
        assert_eq!(classes, vec![ResClass::Mesh, ResClass::AccelStructure]);
        assert_eq!(resident.evictions(), 2);
        assert_eq!(resident.class_bytes(ResClass::Meshlet), 300);
    }

    #[test]
    fn low_priority_request_never_evicts_higher_priority() {
        let mut resident = Residency::new(1000, 8);
        admit(&mut resident, 1, 400, 9, ResClass::Texture);
        admit(&mut resident, 2, 400, 8, ResClass::Mesh);
        let err = resident
            .request(job(3, 500, 1), ResClass::Meshlet)
            .expect_err("a peer or higher priority is never evicted");
        assert!(matches!(
            err,
            ResidencyError::Stream(StreamError::OverBudget { .. })
        ));
        assert_eq!(resident.used_bytes(), 800);
        assert_eq!(resident.evictions(), 0);
        assert!(!resident.is_resident(&hex_id(3)));
        assert!(resident.is_resident(&hex_id(1)));
        assert!(resident.is_resident(&hex_id(2)));
    }

    #[test]
    fn cache_hit_reuses_bytes_and_unknown_evict_is_typed() {
        let mut resident = Residency::new(1 << 20, 8);
        admit(&mut resident, 1, 1000, 5, ResClass::Texture);
        let hit = resident
            .request(job(1, 1000, 5), ResClass::Texture)
            .expect("cache hit");
        assert!(hit.evicted.is_empty());
        assert_eq!(resident.used_bytes(), 1000);
        assert_eq!(resident.resident_count(), 1);
        let err = resident.evict(&hex_id(99)).expect_err("unknown id");
        assert!(matches!(
            err,
            ResidencyError::Stream(StreamError::Unknown(_))
        ));
        resident
            .request(job(2, 1000, 5), ResClass::Mesh)
            .expect("pending");
        let dup = resident
            .request(job(2, 1000, 5), ResClass::Mesh)
            .expect_err("pending duplicate");
        assert!(matches!(
            dup,
            ResidencyError::Stream(StreamError::Duplicate(_))
        ));
        assert_eq!(resident.pending_count(), 1);
        assert_eq!(resident.pending_bytes(), 1000);
    }

    #[test]
    fn streaming_soak_never_exceeds_the_budget() {
        let mut resident = Residency::rtx_3050();
        let mut seed = 1u32;
        let mut refusals = 0u32;
        for _ in 0..900 {
            for _ in 0..2 {
                let bytes = (8 + u64::from(seed % 5)) * 1024 * 1024;
                let priority = u8::try_from(seed % 9).unwrap() + 1;
                let class = ResClass::ALL[usize::try_from(seed % 4).unwrap()];
                if resident.request(job(seed, bytes, priority), class).is_err() {
                    refusals = refusals.saturating_add(1);
                }
                assert!(resident.used_bytes() <= resident.budget_bytes());
                let split = resident
                    .resident_bytes()
                    .saturating_add(resident.pending_bytes());
                assert_eq!(split, resident.used_bytes());
                let per_class: u64 = ResClass::ALL
                    .iter()
                    .map(|class| resident.class_bytes(*class))
                    .sum();
                assert_eq!(per_class, resident.resident_bytes());
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            }
            resident.publish(64 * 1024 * 1024);
            assert!(resident.used_bytes() <= resident.budget_bytes());
            assert!(resident.high_water_bytes() <= resident.budget_bytes());
        }
        assert!(
            resident.evictions() > 0,
            "the soak must evict to stay inside"
        );
        assert!(resident.resident_bytes() > 0);
        assert!(refusals > 0, "a soaked budget also refuses lower priority");
    }

    #[test]
    fn residency_work_never_hitches_over_two_ms() {
        let mut resident = Residency::rtx_3050();
        let mut seed = 3u32;
        let mut worst = Duration::ZERO;
        for frame in 0..900u32 {
            let started = Instant::now();
            let bytes = (8 + u64::from(seed % 5)) * 1024 * 1024;
            let priority = u8::try_from(seed % 9).unwrap() + 1;
            let class = ResClass::ALL[usize::try_from(seed % 4).unwrap()];
            let _ = resident.request(job(seed, bytes, priority), class);
            resident.publish(64 * 1024 * 1024);
            worst = worst.max(started.elapsed());
            assert!(
                resident.used_bytes() <= resident.budget_bytes(),
                "frame {frame} exceeds the budget"
            );
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        }
        assert!(
            resident.evictions() > 0,
            "the run must include eviction work"
        );
        assert!(
            worst < Duration::from_millis(2),
            "worst residency frame took {worst:?}"
        );
    }
}
