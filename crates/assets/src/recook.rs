//! recook: bounded debounced recook queue over [`Cooker`] with immutable
//! versioned reload notices.
//!
//! The loop is file watch ([`crate::watcher`]) → `submit` → settled `poll`
//! → caller-published [`ReloadNotice`]. `Recook` owns the [`Cooker`], the
//! path-to-[`AssetId`] map (ids stay stable across recooks by construction:
//! the first submit mints, later submits reimport), and a per-path version
//! counter that only moves on a successful recook.
//!
//! Semantics, all deterministic over a logical tick (one tick per `submit`
//! or `poll` call; no wall clock, `BTreeMap`/`VecDeque` order only):
//!
//! - Bounded queue: `capacity` pending jobs at most; an overflow sheds the
//!   oldest pending job, counts it in [`Recook::dropped`], and marks its
//!   [`JobId`] stale.
//! - Debounce: one pending job per path (a rapid second submit for the same
//!   path merges: latest kind wins, the superseded [`JobId`] goes stale),
//!   and a job cooks only once it settled for `debounce_ticks` ticks, so a
//!   burst of edits to one file cooks once, reading the latest bytes.
//! - Stale jobs never cook: [`Recook::is_stale`] reports superseded, shed
//!   and cancelled ids over a bounded recent history.
//! - Drop: [`Recook::cancel`] removes a pending job; it never cooks.
//! - Retry: [`Recook::retry`] requeues a failed job's path as a fresh job.
//! - Failed cook keeps last-good: the [`Cooker`] runs the job before it
//!   touches registry or cache, so [`RecookDone`] with
//!   [`RecookStatus::Failed`] leaves the version and the cached payload
//!   untouched. A path that never cooked has no id: failures there carry
//!   [`AssetId::NIL`] and version `0`.
//! - [`ReloadNotice`] is an immutable value (`Copy`, no handles). `Recook`
//!   never publishes: the caller converts [`RecookDone`] at its own
//!   boundary (the practice-mode frame) via [`ReloadNotice::for_done`].
//!
//! Running-game latency acceptance (small-fixture edit visible within 1 s,
//! bad reload keeps last-good in play, old tapes replay) belongs to the
//! hot-reload consumer, not here. Cross-asset dependency edges are out of
//! scope too: every cook here declares no deps, so invalidation rides the
//! per-path version; a consumer that needs real dep edges wires them through
//! the registry.

#[cfg(feature = "registry")]
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[cfg(feature = "registry")]
use super::cooker::{CookFailure, Cooker};
#[cfg(feature = "registry")]
use super::registry::AssetId;
#[cfg(feature = "registry")]
use crate::watcher::{FileEvent, FileKind};

/// Upper clamp for [`RecookSettings::capacity`]; the queue never holds more.
#[cfg(feature = "registry")]
pub const MAX_PENDING: usize = 512;
/// Upper clamp for [`RecookSettings::debounce_ticks`].
#[cfg(feature = "registry")]
pub const MAX_DEBOUNCE_TICKS: u64 = 1_000_000;
/// File bytes above this fail at poll time with [`RecookStatus::Failed`]
/// instead of entering the cook closure (8 MiB hot-reload fixture ceiling).
#[cfg(feature = "registry")]
pub const MAX_CONTENT_BYTES: usize = 8 * 1024 * 1024;
/// Cap on the recent-history [`Recook::is_stale`] set; oldest entries fall
/// off and read back as not-stale (unknown).
#[cfg(feature = "registry")]
pub const MAX_STALE_IDS: usize = 512;
/// Cap on remembered failed jobs available to [`Recook::retry`].
#[cfg(feature = "registry")]
pub const MAX_FAILED_RECORDS: usize = 64;

/// Cook closure over file bytes. The default copies input to payload; a real
/// import cook plugs in here (which is where malformed content can fail).
#[cfg(feature = "registry")]
pub type CookFn = fn(&[u8]) -> Result<Vec<u8>, CookFailure>;

/// Default cook: payload equals the source bytes, never fails.
#[cfg(feature = "registry")]
fn identity_cook(bytes: &[u8]) -> Result<Vec<u8>, CookFailure> {
    Ok(bytes.to_vec())
}

/// Opaque recook job handle, minted by `submit`/`retry` in increasing order
/// starting at `1` (`0` is never issued).
#[cfg(feature = "registry")]
pub type JobId = u64;

/// How one settled job finished.
#[cfg(feature = "registry")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecookStatus {
    /// Same digest as the cached cook: closure did not run, version kept.
    Fresh,
    /// Cooked anew: version bumped, cache refreshed.
    Recooked,
    /// Cook or file read failed: version and last-good payload kept.
    Failed,
}

/// One settled job. `job` traces back to the submitting `submit`/`retry`
/// call, so callers can ignore late results from superseded jobs.
#[cfg(feature = "registry")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecookDone {
    /// The settled job's id.
    pub job: JobId,
    /// Stable asset id, or [`AssetId::NIL`] when the path never cooked.
    pub asset: AssetId,
    /// Per-path version after this job (`0` when the path never cooked).
    pub version: u64,
    /// How the job finished.
    pub status: RecookStatus,
}

/// Immutable versioned reload notification. A plain value: the caller builds
/// it from [`RecookDone`] and publishes it at its own boundary (the caller
/// owns the running-game frame/tick where it lands).
#[cfg(feature = "registry")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReloadNotice {
    /// The recooked asset; stable across recooks by construction.
    pub asset: AssetId,
    /// The new per-path version to load.
    pub version: u64,
}

#[cfg(feature = "registry")]
impl ReloadNotice {
    /// Build the notice for a finished job: `Some` exactly when the job
    /// recooked to a new version. `Fresh` needs no reload (already there)
    /// and `Failed` keeps last-good (nothing new to load).
    pub fn for_done(done: &RecookDone) -> Option<Self> {
        if done.status == RecookStatus::Recooked {
            Some(Self {
                asset: done.asset,
                version: done.version,
            })
        } else {
            None
        }
    }

    /// Collect the publishable notices from one [`Recook::poll`] batch, in
    /// batch order.
    pub fn collect(dones: &[RecookDone]) -> Vec<Self> {
        dones.iter().filter_map(Self::for_done).collect()
    }
}

/// Recook queue options, taken by value at construction.
///
/// `CookFn` is a plain function pointer, so settings compare by address only
/// where pointers compare at all: no `PartialEq` here, compare fields instead.
#[cfg(feature = "registry")]
#[derive(Clone, Copy, Debug)]
pub struct RecookSettings {
    /// Max pending jobs; overflow sheds the oldest (counted, marked stale).
    /// Clamped to `1..=MAX_PENDING`.
    pub capacity: usize,
    /// Settle window in logical ticks before a pending job may cook.
    /// Clamped to `0..=MAX_DEBOUNCE_TICKS`.
    pub debounce_ticks: u64,
    /// Cook closure run on settled jobs.
    pub cook: CookFn,
}

#[cfg(feature = "registry")]
impl Default for RecookSettings {
    fn default() -> Self {
        Self {
            capacity: 64,
            debounce_ticks: 0,
            cook: identity_cook,
        }
    }
}

/// One pending job: path plus latest kind and submit tick. Bytes are read
/// from disk at cook time, so a merged burst cooks the latest file state.
#[cfg(feature = "registry")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingJob {
    job: JobId,
    path: String,
    kind: FileKind,
    tick: u64,
}

/// Remembered failure, so [`Recook::retry`] can requeue the path.
#[cfg(feature = "registry")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FailedRec {
    path: String,
    kind: FileKind,
}

/// Bounded debounced recook queue over [`Cooker`]. Owns stable ids and
/// per-path versions; the caller owns publication of [`ReloadNotice`].
#[cfg(feature = "registry")]
#[derive(Debug)]
pub struct Recook {
    capacity: usize,
    debounce: u64,
    cook: CookFn,
    cooker: Cooker,
    now: u64,
    next_job: JobId,
    pending: VecDeque<PendingJob>,
    by_path: BTreeMap<String, JobId>,
    ids: BTreeMap<String, AssetId>,
    versions: BTreeMap<String, u64>,
    stale: BTreeSet<JobId>,
    failed: BTreeMap<JobId, FailedRec>,
    dropped: u64,
}

#[cfg(feature = "registry")]
impl Recook {
    /// Empty queue with clamped settings and an empty cooker.
    pub fn new(settings: RecookSettings) -> Self {
        Self {
            capacity: settings.capacity.clamp(1, MAX_PENDING),
            debounce: settings.debounce_ticks.min(MAX_DEBOUNCE_TICKS),
            cook: settings.cook,
            cooker: Cooker::new(),
            now: 0,
            next_job: 1,
            pending: VecDeque::new(),
            by_path: BTreeMap::new(),
            ids: BTreeMap::new(),
            versions: BTreeMap::new(),
            stale: BTreeSet::new(),
            failed: BTreeMap::new(),
            dropped: 0,
        }
    }

    /// Effective capacity after clamping.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Effective debounce window after clamping.
    pub fn debounce(&self) -> u64 {
        self.debounce
    }

    /// Swap the cook closure (e.g. plug the real importer, or a failing
    /// closure in tests). Pending jobs cook with the closure set at poll.
    pub fn set_cook(&mut self, cook: CookFn) {
        self.cook = cook;
    }

    /// Pending job count (never above capacity).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Jobs shed by overflow so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Cook closure runs so far (hits and validation misses never run it).
    pub fn cooks(&self) -> u64 {
        self.cooker.cooks
    }

    /// Stable id for a path, if it ever cooked successfully.
    pub fn asset_for(&self, path: &str) -> Option<AssetId> {
        self.ids.get(path).copied()
    }

    /// Current per-path version, if the path ever cooked successfully.
    pub fn version_for(&self, path: &str) -> Option<u64> {
        self.versions.get(path).copied()
    }

    /// Last-good cached payload for an asset, if it ever cooked.
    pub fn cached(&self, asset: AssetId) -> Option<&[u8]> {
        self.cooker
            .cache
            .get(&asset)
            .map(|entry| entry.payload.as_slice())
    }

    /// True when `job` was superseded, shed, or cancelled before cooking.
    /// Bounded recent history: ids that fell off read back as false.
    pub fn is_stale(&self, job: JobId) -> bool {
        self.stale.contains(&job)
    }

    /// Queue a file event. `Removed` drops pending work for the path and
    /// queues nothing (last-good stays live; unpublishing is the caller's
    /// boundary). Any other kind merges with the pending job for the path
    /// when there is one. Always returns the new [`JobId`]; jobs that queue
    /// nothing (removals) are immediately stale.
    pub fn submit(&mut self, event: FileEvent) -> JobId {
        self.tick();
        let job = self.mint();
        if event.kind == FileKind::Removed {
            self.drop_pending_for(&event.path);
            self.mark_stale(job);
            return job;
        }
        if self.by_path.contains_key(&event.path) {
            self.drop_pending_for(&event.path);
        }
        self.push_pending(PendingJob {
            job,
            path: event.path,
            kind: event.kind,
            tick: self.now,
        });
        job
    }

    /// Cancel a pending job. True when a pending job was removed (it never
    /// cooks and reads back stale); false when the id is unknown, already
    /// settled, or already cancelled.
    pub fn cancel(&mut self, job: JobId) -> bool {
        let Some(index) = self.pending.iter().position(|item| item.job == job) else {
            return false;
        };
        let removed = self.pending.remove(index).expect("position found above");
        if self.by_path.get(removed.path.as_str()) == Some(&job) {
            self.by_path.remove(removed.path.as_str());
        }
        self.mark_stale(job);
        true
    }

    /// Requeue a failed job's path as a fresh job with the same kind.
    /// `Some` new [`JobId`] on success; `None` when `job` names no
    /// remembered failure (unknown, succeeded, cancelled, or evicted id).
    pub fn retry(&mut self, job: JobId) -> Option<JobId> {
        let rec = self.failed.remove(&job)?;
        self.tick();
        let fresh = self.mint();
        self.drop_pending_for(&rec.path);
        self.push_pending(PendingJob {
            job: fresh,
            path: rec.path,
            kind: rec.kind,
            tick: self.now,
        });
        Some(fresh)
    }

    /// Cook every settled pending job in queue order and report them.
    /// A job is settled once `now - tick >= debounce_ticks`. `Removed`
    /// kinds never reach here (see `submit`). Reading the file or running
    /// the cook may fail: that job reports [`RecookStatus::Failed`] and
    /// keeps its version and last-good payload.
    pub fn poll(&mut self) -> Vec<RecookDone> {
        self.tick();
        let mut waiting = VecDeque::new();
        let mut ready = Vec::new();
        while let Some(job) = self.pending.pop_front() {
            if self.now.saturating_sub(job.tick) >= self.debounce {
                ready.push(job);
            } else {
                waiting.push_back(job);
            }
        }
        self.pending = waiting;
        let mut out = Vec::with_capacity(ready.len());
        for job in ready {
            if self.by_path.get(job.path.as_str()) == Some(&job.job) {
                self.by_path.remove(job.path.as_str());
            }
            out.push(self.cook_one(job));
        }
        out
    }

    /// Cook one settled job: read current bytes, then cook or recook the
    /// stable id. Failures record a retry slot and report `Failed`.
    fn cook_one(&mut self, job: PendingJob) -> RecookDone {
        let bytes = match std::fs::read(&job.path) {
            Ok(bytes) if bytes.len() <= MAX_CONTENT_BYTES => bytes,
            Ok(_) => {
                self.remember_failed(job.job, &job.path, job.kind);
                return self.failed_done(job.job, &job.path);
            }
            Err(_) => {
                self.remember_failed(job.job, &job.path, job.kind);
                return self.failed_done(job.job, &job.path);
            }
        };
        match self.ids.get(job.path.as_str()).copied() {
            None => match self.cooker.cook(&bytes, &[], self.cook) {
                Ok(outcome) => {
                    self.ids.insert(job.path.clone(), outcome.id);
                    self.versions.insert(job.path.clone(), 1);
                    RecookDone {
                        job: job.job,
                        asset: outcome.id,
                        version: 1,
                        status: RecookStatus::Recooked,
                    }
                }
                Err(_) => {
                    self.remember_failed(job.job, &job.path, job.kind);
                    self.failed_done(job.job, &job.path)
                }
            },
            Some(id) => match self.cooker.recook(id, &bytes, &[], self.cook) {
                Ok(outcome) => {
                    if outcome.fresh {
                        // The cook closure ran: new payload, bump the version.
                        let version =
                            self.versions.get(job.path.as_str()).copied().unwrap_or(0) + 1;
                        self.versions.insert(job.path.clone(), version);
                        RecookDone {
                            job: job.job,
                            asset: id,
                            version,
                            status: RecookStatus::Recooked,
                        }
                    } else {
                        // Cache hit: the closure never ran, version kept.
                        RecookDone {
                            job: job.job,
                            asset: id,
                            version: self.versions.get(job.path.as_str()).copied().unwrap_or(1),
                            status: RecookStatus::Fresh,
                        }
                    }
                }
                Err(_) => {
                    self.remember_failed(job.job, &job.path, job.kind);
                    RecookDone {
                        job: job.job,
                        asset: id,
                        version: self.versions.get(job.path.as_str()).copied().unwrap_or(1),
                        status: RecookStatus::Failed,
                    }
                }
            },
        }
    }

    /// `Failed` report for a path: known version (or `0`), stable id when
    /// the path cooked before, else [`AssetId::NIL`].
    fn failed_done(&self, job: JobId, path: &str) -> RecookDone {
        RecookDone {
            job,
            asset: self.ids.get(path).copied().unwrap_or(AssetId::NIL),
            version: self.versions.get(path).copied().unwrap_or(0),
            status: RecookStatus::Failed,
        }
    }

    /// Advance the logical clock by one tick.
    fn tick(&mut self) {
        self.now = self.now.saturating_add(1);
    }

    /// Mint the next job id (`0` is never issued).
    fn mint(&mut self) -> JobId {
        let job = self.next_job.max(1);
        self.next_job = job.saturating_add(1).max(1);
        job
    }

    /// Push a pending job, shedding the oldest first when at capacity.
    fn push_pending(&mut self, job: PendingJob) {
        while self.pending.len() >= self.capacity {
            if let Some(shed) = self.pending.pop_front() {
                if self.by_path.get(shed.path.as_str()) == Some(&shed.job) {
                    self.by_path.remove(shed.path.as_str());
                }
                self.mark_stale(shed.job);
                self.dropped = self.dropped.saturating_add(1);
            } else {
                break;
            }
        }
        self.by_path.insert(job.path.clone(), job.job);
        self.pending.push_back(job);
    }

    /// Remove the pending job for `path`, if any, and mark it stale.
    /// Used by merge, removal, retry, and cancel paths alike.
    fn drop_pending_for(&mut self, path: &str) {
        if let Some(old) = self.by_path.remove(path) {
            if let Some(index) = self.pending.iter().position(|item| item.job == old) {
                self.pending.remove(index);
            }
            self.mark_stale(old);
        }
    }

    /// Record a stale id, pruning the oldest when over the history cap.
    fn mark_stale(&mut self, job: JobId) {
        self.stale.insert(job);
        while self.stale.len() > MAX_STALE_IDS {
            if self.stale.pop_first().is_none() {
                break;
            }
        }
    }

    /// Remember a failure for `retry`, pruning oldest when over cap.
    fn remember_failed(&mut self, job: JobId, path: &str, kind: FileKind) {
        self.failed.insert(
            job,
            FailedRec {
                path: path.to_string(),
                kind,
            },
        );
        while self.failed.len() > MAX_FAILED_RECORDS {
            if self.failed.pop_first().is_none() {
                break;
            }
        }
    }
}

#[cfg(all(test, feature = "registry"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    /// Unique scratch path per test (unit tests run in parallel threads).
    fn scratch(name: &str) -> String {
        let n = SCRATCH.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "engine-assets-hr1-recook-{name}-{}-{n}.tmp",
            std::process::id()
        ));
        path.to_string_lossy().into_owned()
    }

    fn remove(path: &str) {
        let _ = std::fs::remove_file(path);
    }

    fn created(path: &str) -> FileEvent {
        FileEvent {
            path: path.to_string(),
            kind: FileKind::Created,
        }
    }

    fn modified(path: &str) -> FileEvent {
        FileEvent {
            path: path.to_string(),
            kind: FileKind::Modified,
        }
    }

    fn removed(path: &str) -> FileEvent {
        FileEvent {
            path: path.to_string(),
            kind: FileKind::Removed,
        }
    }

    fn settings() -> RecookSettings {
        RecookSettings {
            capacity: 64,
            debounce_ticks: 0,
            cook: identity_cook,
        }
    }

    /// Poll until `job` settles or the bound runs out; returns all dones.
    fn settle(queue: &mut Recook, bound: u64) -> Vec<RecookDone> {
        let mut all = Vec::new();
        for _ in 0..bound {
            let dones = queue.poll();
            all.extend(dones);
            if queue.pending_len() == 0 {
                break;
            }
        }
        all
    }

    #[test]
    fn first_submit_mints_and_recook_keeps_stable_id() {
        let path = scratch("stable");
        remove(&path);
        std::fs::write(&path, b"coat v1").unwrap();
        let mut queue = Recook::new(settings());

        let j1 = queue.submit(created(&path));
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        let first = dones[0];
        assert_eq!(first.job, j1);
        assert_eq!(first.status, RecookStatus::Recooked);
        assert_eq!(first.version, 1);
        assert!(!first.asset.is_nil());
        assert!(!queue.is_stale(j1));

        // Same bytes: cache hit, Fresh, version and id unchanged, no cook.
        let cooks_before = queue.cooks();
        let j2 = queue.submit(modified(&path));
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].job, j2);
        assert_eq!(dones[0].status, RecookStatus::Fresh);
        assert_eq!(dones[0].asset, first.asset);
        assert_eq!(dones[0].version, 1);
        assert_eq!(queue.cooks(), cooks_before);

        // Changed bytes: Recooked, version 2, same stable id.
        std::fs::write(&path, b"coat v2!!").unwrap();
        queue.submit(modified(&path));
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].status, RecookStatus::Recooked);
        assert_eq!(dones[0].asset, first.asset);
        assert_eq!(dones[0].version, 2);
        assert_eq!(queue.asset_for(&path), Some(first.asset));
        assert_eq!(queue.version_for(&path), Some(2));
        assert_eq!(queue.cached(first.asset), Some(b"coat v2!!".as_slice()));

        // Reload notices publish only for real recooks, in batch order.
        let notices = ReloadNotice::collect(&dones);
        assert_eq!(
            notices,
            vec![ReloadNotice {
                asset: first.asset,
                version: 2,
            }]
        );
        assert!(ReloadNotice::for_done(&RecookDone {
            job: j2,
            asset: first.asset,
            version: 1,
            status: RecookStatus::Fresh,
        })
        .is_none());
        remove(&path);
    }

    #[test]
    fn rapid_edits_coalesce_into_one_cook() {
        let path = scratch("debounce");
        remove(&path);
        std::fs::write(&path, b"burst v1").unwrap();
        let mut queue = Recook::new(RecookSettings {
            capacity: 64,
            debounce_ticks: 4,
            cook: identity_cook,
        });

        // Burst of three submits before any poll: each supersedes the last.
        let j1 = queue.submit(created(&path));
        std::fs::write(&path, b"burst v2").unwrap();
        let j2 = queue.submit(modified(&path));
        std::fs::write(&path, b"burst v3!!!!").unwrap();
        let j3 = queue.submit(modified(&path));
        assert_eq!(queue.pending_len(), 1);
        assert!(queue.is_stale(j1));
        assert!(queue.is_stale(j2));
        assert!(!queue.is_stale(j3));

        // Settle window has not passed: early polls cook nothing.
        assert!(queue.poll().is_empty());
        assert_eq!(queue.pending_len(), 1);

        // Once settled: exactly one cook with the latest bytes.
        let cooks_before = queue.cooks();
        let dones = settle(&mut queue, 8);
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].job, j3);
        assert_eq!(dones[0].status, RecookStatus::Recooked);
        assert_eq!(dones[0].version, 1);
        assert_eq!(queue.cooks(), cooks_before + 1);
        assert_eq!(
            queue.cached(dones[0].asset),
            Some(b"burst v3!!!!".as_slice())
        );
        remove(&path);
    }

    #[test]
    fn failed_cook_keeps_last_good_version() {
        fn fail_all(_: &[u8]) -> Result<Vec<u8>, CookFailure> {
            Err(CookFailure::new("oven exploded"))
        }
        let path = scratch("lastgood");
        remove(&path);
        std::fs::write(&path, b"good bytes").unwrap();
        let mut queue = Recook::new(settings());

        queue.submit(created(&path));
        let dones = queue.poll();
        assert_eq!(dones[0].status, RecookStatus::Recooked);
        let asset = dones[0].asset;
        assert_eq!(queue.cached(asset), Some(b"good bytes".as_slice()));

        // Same path, new bytes, failing cook: Failed, version + payload kept.
        queue.set_cook(fail_all);
        std::fs::write(&path, b"bad bytes!").unwrap();
        let jbad = queue.submit(modified(&path));
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].job, jbad);
        assert_eq!(dones[0].status, RecookStatus::Failed);
        assert_eq!(dones[0].asset, asset);
        assert_eq!(dones[0].version, 1);
        assert_eq!(queue.version_for(&path), Some(1));
        assert_eq!(queue.cached(asset), Some(b"good bytes".as_slice()));
        assert!(ReloadNotice::for_done(&dones[0]).is_none());

        // Retry with a working cook recooks the same path to version 2.
        queue.set_cook(identity_cook);
        let retried = queue.retry(jbad).expect("failed job is retryable");
        assert!(!queue.is_stale(retried));
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].job, retried);
        assert_eq!(dones[0].status, RecookStatus::Recooked);
        assert_eq!(dones[0].asset, asset);
        assert_eq!(dones[0].version, 2);
        assert_eq!(queue.cached(asset), Some(b"bad bytes!".as_slice()));

        // Unknown and succeeded ids are not retryable.
        assert_eq!(queue.retry(jbad), None);
        assert_eq!(queue.retry(999_999), None);
        assert_eq!(queue.retry(dones[0].job), None);
        remove(&path);
    }

    #[test]
    fn missing_file_fails_without_id_and_removal_queues_nothing() {
        let path = scratch("missing");
        remove(&path);
        let mut queue = Recook::new(settings());

        // Submit for a file that never appears: Failed with NIL id, v0.
        let job = queue.submit(created(&path));
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].status, RecookStatus::Failed);
        assert_eq!(dones[0].asset, AssetId::NIL);
        assert_eq!(dones[0].version, 0);
        assert_eq!(queue.asset_for(&path), None);

        // A later appearance cooks normally and mints the stable id.
        std::fs::write(&path, b"late arrival").unwrap();
        let retried = queue.retry(job).expect("missing-file failure is retryable");
        let dones = queue.poll();
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].job, retried);
        assert_eq!(dones[0].status, RecookStatus::Recooked);
        assert_eq!(dones[0].version, 1);

        // Removal drops pending work and queues nothing itself.
        std::fs::write(&path, b"changed before delete").unwrap();
        let mut held = Recook::new(RecookSettings {
            capacity: 64,
            debounce_ticks: 100,
            cook: identity_cook,
        });
        std::fs::write(&path, b"held bytes").unwrap();
        let pending = held.submit(modified(&path));
        assert_eq!(held.pending_len(), 1);
        let tombstone = held.submit(removed(&path));
        assert!(held.is_stale(tombstone));
        assert!(held.is_stale(pending));
        assert_eq!(held.pending_len(), 0);
        assert!(settle(&mut held, 120).is_empty());
        remove(&path);
    }

    #[test]
    fn overflow_sheds_oldest_and_counts_dropped() {
        let a = scratch("drop-a");
        let b = scratch("drop-b");
        let c = scratch("drop-c");
        for (path, bytes) in [(&a, b"a"), (&b, b"b"), (&c, b"c")] {
            remove(path);
            std::fs::write(path, bytes).unwrap();
        }
        let mut queue = Recook::new(RecookSettings {
            capacity: 2,
            debounce_ticks: 100,
            cook: identity_cook,
        });

        let ja = queue.submit(created(&a));
        let jb = queue.submit(created(&b));
        assert_eq!(queue.dropped(), 0);
        let jc = queue.submit(created(&c));
        // Oldest shed: pending holds b, c; a is stale and never cooks.
        assert_eq!(queue.pending_len(), 2);
        assert_eq!(queue.dropped(), 1);
        assert!(queue.is_stale(ja));
        assert!(!queue.is_stale(jb));
        assert!(!queue.is_stale(jc));

        let dones = settle(&mut queue, 120);
        assert_eq!(dones.len(), 2);
        assert_eq!(dones[0].job, jb);
        assert_eq!(dones[1].job, jc);
        assert_eq!(queue.asset_for(&a), None);
        assert!(queue.asset_for(&b).is_some());
        assert!(queue.asset_for(&c).is_some());
        for path in [&a, &b, &c] {
            remove(path);
        }
    }

    #[test]
    fn cancel_drops_pending_and_rejects_double_cancel() {
        let path = scratch("cancel");
        remove(&path);
        std::fs::write(&path, b"doomed").unwrap();
        let mut queue = Recook::new(RecookSettings {
            capacity: 64,
            debounce_ticks: 100,
            cook: identity_cook,
        });

        let job = queue.submit(created(&path));
        assert!(queue.cancel(job));
        assert!(!queue.cancel(job));
        assert!(!queue.cancel(424_242));
        assert!(queue.is_stale(job));
        assert_eq!(queue.pending_len(), 0);

        // Nothing cooks after the window passes.
        assert!(settle(&mut queue, 120).is_empty());
        assert_eq!(queue.asset_for(&path), None);

        // A fresh submit after cancel cooks normally.
        queue.submit(created(&path));
        let dones = settle(&mut queue, 120);
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].status, RecookStatus::Recooked);
        remove(&path);
    }

    #[test]
    fn settings_clamp_and_outputs_stay_in_queue_order() {
        let a = scratch("order-a");
        let b = scratch("order-b");
        for path in [&a, &b] {
            remove(path);
            std::fs::write(path, b"order bytes!").unwrap();
        }
        let mut queue = Recook::new(RecookSettings {
            capacity: 0,
            debounce_ticks: u64::MAX,
            cook: identity_cook,
        });
        assert_eq!(queue.capacity(), 1);
        assert_eq!(queue.debounce(), MAX_DEBOUNCE_TICKS);

        // Capacity 1: the second submit sheds the first.
        let ja = queue.submit(created(&a));
        let jb = queue.submit(created(&b));
        assert_eq!(queue.dropped(), 1);
        assert!(queue.is_stale(ja));
        assert!(!queue.is_stale(jb));

        // Huge debounce still settles deterministically on repeated polls.
        queue = Recook::new(settings());
        let ja = queue.submit(created(&a));
        let jb = queue.submit(created(&b));
        let dones = queue.poll();
        assert_eq!(dones.len(), 2);
        assert_eq!(dones[0].job, ja);
        assert_eq!(dones[1].job, jb);
        let notices = ReloadNotice::collect(&dones);
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].asset, dones[0].asset);
        assert_eq!(notices[1].asset, dones[1].asset);
        assert_ne!(notices[0].asset, notices[1].asset);
        for path in [&a, &b] {
            remove(path);
        }
    }
}
