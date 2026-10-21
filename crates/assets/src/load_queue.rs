//! load_queue: one queue that brings content in by logical path.
//!
//! Owner: `engine-assets`. A [`LoadQueue`] pairs an [`AssetSource`] with a
//! generational [`AssetStore`]: a request names a logical path and gets a
//! handle before its bytes arrive, and a pump reads the queue in priority
//! order under a byte budget. A queued load can be cancelled; a landed asset
//! is unloaded by freeing its handle, which gives the bytes back to the
//! budget. Std-only: no clock, no RNG, no threads. The caller drives the
//! pump, so the same code serves the native file source and the browser's
//! fetched bundle. Units: bytes for the budget, counts for the caps.
//! Determinism: priority first, then path order; no wall clock, no floats.
//! Errors: typed [`LoadError`], never panics.

use crate::asset_source::{validate_path, AssetSource, SourceError};
use crate::handle::{AssetHandle, AssetKey, AssetStore, HandleBudget, HandleError, LoadState};
use std::collections::BTreeMap;
use std::fmt;

/// Why a load request was refused; every variant names the path or the handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The path is not a logical path (absolute, `..`, backslash, empty part).
    BadPath {
        /// Offending path (bounded echo).
        path: String,
        /// Which rule it broke.
        reason: &'static str,
    },
    /// The store already holds its asset-slot budget.
    AssetCap {
        /// Slot budget that is full.
        limit: u32,
    },
    /// The request does not fit the byte budget.
    OverBudget {
        /// Bytes the request wanted reserved.
        want_bytes: u64,
        /// Bytes free under the budget.
        free_bytes: u64,
    },
    /// The handle names no live slot.
    Unknown {
        /// Attempted slot index.
        index: u32,
        /// Attempted generation.
        generation: u32,
    },
    /// The handle is live but in the wrong state for this call.
    WrongState {
        /// Live slot index.
        index: u32,
        /// Live generation.
        generation: u32,
        /// Current state.
        state: LoadState,
    },
    /// The source could not answer; the reason names the path.
    Source(SourceError),
    /// The store refused for a reason the queue does not model.
    Store(HandleError),
}

impl LoadError {
    /// Map a store refusal onto the queue's typed error.
    pub fn from_handle(err: HandleError) -> Self {
        match err {
            HandleError::Unknown { index, generation } => Self::Unknown { index, generation },
            HandleError::WrongState {
                index,
                generation,
                state,
            } => Self::WrongState {
                index,
                generation,
                state,
            },
            HandleError::AssetCap { limit } => Self::AssetCap { limit },
            HandleError::OverBudget {
                want_bytes,
                free_bytes,
            } => Self::OverBudget {
                want_bytes,
                free_bytes,
            },
            other => Self::Store(other),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadPath { path, reason } => write!(f, "load path {path:?} refused: {reason}"),
            Self::AssetCap { limit } => write!(f, "load asset cap reached: {limit}"),
            Self::OverBudget {
                want_bytes,
                free_bytes,
            } => write!(
                f,
                "load over budget: want {want_bytes} bytes, free {free_bytes}"
            ),
            Self::Unknown { index, generation } => {
                write!(f, "unknown load handle {index}:{generation}")
            }
            Self::WrongState {
                index,
                generation,
                state,
            } => write!(f, "load handle {index}:{generation} is {state}"),
            Self::Source(err) => write!(f, "load source refused: {err}"),
            Self::Store(err) => write!(f, "load store refused: {err}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<SourceError> for LoadError {
    fn from(err: SourceError) -> Self {
        Self::Source(err)
    }
}

/// What one pump, cancel or unload did, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadEvent {
    /// The asset is resident; its handle reads its bytes.
    Loaded {
        /// Handle of the loaded asset.
        handle: AssetHandle,
        /// Logical path it was read from.
        path: String,
        /// Bytes resident; units: bytes.
        bytes: u64,
    },
    /// The asset did not fit the budget yet; it stays queued.
    Deferred {
        /// Handle of the queued asset.
        handle: AssetHandle,
        /// Logical path it was read from.
        path: String,
        /// Why it waits.
        reason: String,
    },
    /// The source refused the path; the load failed by name.
    Failed {
        /// Handle of the failed asset.
        handle: AssetHandle,
        /// Logical path that was asked for.
        path: String,
        /// Bounded reason, naming the path.
        reason: String,
    },
    /// The request was cancelled before it landed.
    Cancelled {
        /// Handle of the cancelled asset.
        handle: AssetHandle,
        /// Logical path it named.
        path: String,
    },
    /// The asset was unloaded; its bytes went back to the budget.
    Unloaded {
        /// Handle that went stale.
        handle: AssetHandle,
        /// Logical path it was read from.
        path: String,
        /// Bytes given back; units: bytes.
        bytes: u64,
    },
}

/// One queued request: the path, its priority, its handle and its bytes.
#[derive(Clone, Debug)]
struct Pending {
    /// Logical path the source is asked for.
    path: String,
    /// Higher priority reads first.
    priority: u8,
    /// Handle the request holds.
    handle: AssetHandle,
    /// Bytes held for the request: the declared size, then the real size
    /// once the bytes are here; `0` while the size is unknown.
    reserved: u64,
    /// The bytes once they were read; kept so a deferred load reads once.
    fetched: Option<Vec<u8>>,
}

/// One load queue over an asset source: priority order, a byte budget,
/// cancellation and generational handles.
///
/// Consumer: the game kit loader (`crates/game/src/kit_assets.rs`) and the
/// browser page, both of which name content by logical path and read it
/// through one queue. The queue owns no thread and no clock: the caller
/// drives [`LoadQueue::pump`], so the same code runs natively over files and
/// in a browser over a fetched bundle.
#[derive(Clone)]
pub struct LoadQueue<'a> {
    /// Where the bytes come from.
    source: &'a dyn AssetSource,
    /// The generational handles and their payloads.
    store: AssetStore,
    /// Requests that have not landed yet.
    pending: Vec<Pending>,
    /// Logical path to its live handle.
    paths: BTreeMap<String, AssetHandle>,
}

impl fmt::Debug for LoadQueue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadQueue")
            .field("budget", &self.budget())
            .field("pending", &self.pending.len())
            .field("resident_bytes", &self.resident_bytes())
            .finish()
    }
}

impl<'a> LoadQueue<'a> {
    /// Empty queue over `source` under `budget`; nothing resident.
    pub fn new(source: &'a dyn AssetSource, budget: HandleBudget) -> Self {
        Self {
            source,
            store: AssetStore::new(budget),
            pending: Vec::new(),
            paths: BTreeMap::new(),
        }
    }

    /// The budget in force: byte cap plus live-asset cap.
    pub fn budget(&self) -> HandleBudget {
        self.store.budget()
    }

    /// Bytes resident in loaded payloads; units: bytes.
    pub fn resident_bytes(&self) -> u64 {
        self.store.used_bytes()
    }

    /// Bytes held by requests that have not landed; units: bytes.
    pub fn reserved_bytes(&self) -> u64 {
        let mut total: u64 = 0;
        for job in &self.pending {
            total = total.saturating_add(job.reserved);
        }
        total
    }

    /// Bytes still free under the budget; units: bytes.
    pub fn remaining_bytes(&self) -> u64 {
        self.budget()
            .bytes
            .saturating_sub(self.resident_bytes().saturating_add(self.reserved_bytes()))
    }

    /// Requests that have not landed yet.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Live handles in slot order.
    pub fn handles(&self) -> Vec<AssetHandle> {
        self.store.live_handles()
    }

    /// The source the queue reads from.
    pub fn source(&self) -> &'a dyn AssetSource {
        self.source
    }

    /// Queue a load.
    ///
    /// `path` is a logical path (`/` separated, relative), `priority` higher
    /// reads first and `bytes` is the expected size (`0` when the size is
    /// unknown, so the budget is checked when the bytes arrive). The same
    /// path twice returns the same handle; a path that does not fit the
    /// budget is refused with [`LoadError::OverBudget`].
    pub fn enqueue(
        &mut self,
        path: &str,
        priority: u8,
        bytes: u64,
    ) -> Result<AssetHandle, LoadError> {
        validate_path(path).map_err(|err| match err {
            SourceError::BadPath { path, reason } => LoadError::BadPath { path, reason },
            other => LoadError::Source(other),
        })?;
        if let Some(handle) = self.paths.get(path).copied() {
            if self.store.contains(handle) {
                return Ok(handle);
            }
            self.paths.remove(path);
        }
        if bytes > 0 {
            let free = self.remaining_bytes();
            if bytes > free {
                return Err(LoadError::OverBudget {
                    want_bytes: bytes,
                    free_bytes: free,
                });
            }
        }
        let handle = self
            .store
            .stage(path_key(path))
            .map_err(LoadError::from_handle)?;
        self.paths.insert(path.to_string(), handle);
        self.pending.push(Pending {
            path: path.to_string(),
            priority,
            handle,
            reserved: bytes,
            fetched: None,
        });
        Ok(handle)
    }

    /// Read queued requests in priority order until `max_bytes` are admitted.
    ///
    /// The highest priority request reads first; equal priorities read in
    /// path order. A request that does not fit the byte budget waits
    /// ([`LoadEvent::Deferred`]) and the pump stops, so the priority order
    /// never breaks. A source that refuses a path fails that request by name
    /// ([`LoadEvent::Failed`]) and the pump carries on with the next one.
    pub fn pump(&mut self, max_bytes: u64) -> Vec<LoadEvent> {
        let mut events = Vec::new();
        let mut admitted: u64 = 0;
        while let Some(path) = self.next_path() {
            let Some(index) = self.pending.iter().position(|job| job.path == path) else {
                continue;
            };
            if self.pending[index].fetched.is_none() {
                match self.source.read(&path) {
                    Ok(bytes) => {
                        let bytes = bytes.into_owned();
                        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                        self.pending[index].reserved = len;
                        self.pending[index].fetched = Some(bytes);
                    }
                    Err(err) => {
                        let reason = err.to_string();
                        let handle = self.pending[index].handle;
                        self.pending.remove(index);
                        self.paths.remove(&path);
                        let _ = self.store.mark_failed(handle, &reason);
                        events.push(LoadEvent::Failed {
                            handle,
                            path,
                            reason,
                        });
                        continue;
                    }
                }
            }
            let handle = self.pending[index].handle;
            let bytes = match self.pending[index].fetched.as_ref() {
                Some(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                None => 0,
            };
            if admitted.saturating_add(bytes) > max_bytes {
                events.push(LoadEvent::Deferred {
                    handle,
                    path: path.clone(),
                    reason: format!("frame cap {max_bytes} bytes"),
                });
                break;
            }
            if self.resident_bytes().saturating_add(self.reserved_bytes()) > self.budget().bytes {
                events.push(LoadEvent::Deferred {
                    handle,
                    path: path.clone(),
                    reason: format!("byte budget {} bytes", self.budget().bytes),
                });
                break;
            }
            let payload = self.pending[index].fetched.take().unwrap_or_default();
            self.pending.remove(index);
            match self.store.mark_loaded_owned(handle, payload) {
                Ok(_) => {
                    admitted = admitted.saturating_add(bytes);
                    events.push(LoadEvent::Loaded {
                        handle,
                        path,
                        bytes,
                    });
                }
                Err(err) => {
                    let reason = err.to_string();
                    let _ = self.store.mark_failed(handle, &reason);
                    self.paths.remove(&path);
                    events.push(LoadEvent::Failed {
                        handle,
                        path,
                        reason,
                    });
                }
            }
        }
        events
    }

    /// Cancel a queued load: the request never lands, its reservation goes
    /// back to the budget and the handle reads as cancelled.
    pub fn cancel(&mut self, handle: AssetHandle) -> Result<LoadEvent, LoadError> {
        let Some(index) = self.pending.iter().position(|job| job.handle == handle) else {
            return Err(self.state_error(handle));
        };
        let job = self.pending.remove(index);
        self.paths.remove(&job.path);
        let _ = self.store.mark_cancelled(handle, "cancelled by the caller");
        Ok(LoadEvent::Cancelled {
            handle,
            path: job.path,
        })
    }

    /// Unload a resident asset: the handle goes stale and its bytes go back
    /// to the budget. A queued request is not resident yet; cancel it.
    pub fn unload(&mut self, handle: AssetHandle) -> Result<LoadEvent, LoadError> {
        let state = self.state(handle)?;
        if state != LoadState::Loaded {
            return Err(LoadError::WrongState {
                index: handle.index,
                generation: handle.generation,
                state,
            });
        }
        let bytes = match self.store.payload(handle) {
            Ok(payload) => u64::try_from(payload.len()).unwrap_or(u64::MAX),
            Err(err) => return Err(LoadError::from_handle(err)),
        };
        let path = self.path_of(handle).unwrap_or_default();
        self.store.free(handle).map_err(LoadError::from_handle)?;
        self.paths.retain(|_, live| *live != handle);
        Ok(LoadEvent::Unloaded {
            handle,
            path,
            bytes,
        })
    }

    /// The bytes of a loaded asset; only a loaded handle reads.
    pub fn payload(&self, handle: AssetHandle) -> Result<&[u8], LoadError> {
        self.store.payload(handle).map_err(LoadError::from_handle)
    }

    /// The load state of a handle.
    pub fn state(&self, handle: AssetHandle) -> Result<LoadState, LoadError> {
        self.store.state(handle).map_err(LoadError::from_handle)
    }

    /// True when the handle names a live, loaded asset.
    pub fn is_ready(&self, handle: AssetHandle) -> bool {
        self.store.is_ready(handle)
    }

    /// The failure or cancel reason of a refused load; `None` for a load
    /// that is still queued or already landed.
    pub fn failure_reason(&self, handle: AssetHandle) -> Option<&str> {
        self.store.failure(handle).ok().flatten()
    }

    /// The next path to read: highest priority first, then path order.
    fn next_path(&self) -> Option<String> {
        let mut best: Option<&Pending> = None;
        for job in &self.pending {
            let take = match best {
                None => true,
                Some(current) => {
                    job.priority > current.priority
                        || (job.priority == current.priority && job.path < current.path)
                }
            };
            if take {
                best = Some(job);
            }
        }
        best.map(|job| job.path.clone())
    }

    /// The path a handle was read from.
    fn path_of(&self, handle: AssetHandle) -> Option<String> {
        self.paths
            .iter()
            .find(|(_, live)| **live == handle)
            .map(|(path, _)| path.clone())
    }

    /// Why this handle cannot serve the call: unknown or the wrong state.
    fn state_error(&self, handle: AssetHandle) -> LoadError {
        match self.store.state(handle) {
            Ok(state) => LoadError::WrongState {
                index: handle.index,
                generation: handle.generation,
                state,
            },
            Err(HandleError::Unknown { index, generation }) => {
                LoadError::Unknown { index, generation }
            }
            Err(err) => LoadError::from_handle(err),
        }
    }
}

/// Content key of a logical path: the queue keys its slots by name, so the
/// same path always names the same staged handle.
fn path_key(path: &str) -> AssetKey {
    engine_core::hash_bytes(path.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_source::MemorySource;

    /// A source with three named files of known size.
    fn source() -> MemorySource {
        let mut source = MemorySource::new();
        source.insert("src/a.gltf", vec![1u8; 10]).expect("path");
        source.insert("src/b.gltf", vec![2u8; 20]).expect("path");
        source.insert("src/c.gltf", vec![3u8; 30]).expect("path");
        source
    }

    fn queue(source: &MemorySource, bytes: u64) -> LoadQueue<'_> {
        LoadQueue::new(source, HandleBudget::new(bytes, 16))
    }

    #[test]
    fn queue_reads_the_highest_priority_first() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        let low = queue.enqueue("src/a.gltf", 1, 0).expect("enqueue");
        let high = queue.enqueue("src/b.gltf", 9, 0).expect("enqueue");
        let mid = queue.enqueue("src/c.gltf", 5, 0).expect("enqueue");
        let events = queue.pump(1 << 20);
        let order: Vec<(AssetHandle, u64)> = events
            .iter()
            .map(|event| match event {
                LoadEvent::Loaded { handle, bytes, .. } => (*handle, *bytes),
                other => panic!("expected a load, got {other:?}"),
            })
            .collect();
        assert_eq!(order, vec![(high, 20), (mid, 30), (low, 10)]);
        assert_eq!(queue.resident_bytes(), 60);
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.payload(high).expect("payload").len(), 20);
        assert!(queue.is_ready(mid));
    }

    #[test]
    fn equal_priorities_read_in_path_order() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        queue.enqueue("src/c.gltf", 4, 0).expect("enqueue");
        queue.enqueue("src/a.gltf", 4, 0).expect("enqueue");
        queue.enqueue("src/b.gltf", 4, 0).expect("enqueue");
        let events = queue.pump(1 << 20);
        let paths: Vec<&str> = events
            .iter()
            .map(|event| match event {
                LoadEvent::Loaded { path, .. } => path.as_str(),
                other => panic!("expected a load, got {other:?}"),
            })
            .collect();
        assert_eq!(paths, ["src/a.gltf", "src/b.gltf", "src/c.gltf"]);
    }

    #[test]
    fn queue_refuses_a_request_over_the_byte_budget() {
        let source = source();
        let mut queue = queue(&source, 25);
        let err = queue.enqueue("src/c.gltf", 1, 30).unwrap_err();
        assert_eq!(
            err,
            LoadError::OverBudget {
                want_bytes: 30,
                free_bytes: 25,
            }
        );
        assert_eq!(queue.pending_count(), 0, "a refused request stages nothing");
        assert!(queue.enqueue("src/a.gltf", 1, 10).is_ok());
    }

    #[test]
    fn queue_defers_a_load_that_does_not_fit_the_budget() {
        let source = source();
        let mut queue = queue(&source, 25);
        let handle = queue.enqueue("src/c.gltf", 1, 0).expect("enqueue");
        let events = queue.pump(1 << 20);
        assert!(matches!(
            events.first(),
            Some(LoadEvent::Deferred { path, .. }) if path == "src/c.gltf"
        ));
        assert_eq!(queue.resident_bytes(), 0, "nothing is resident yet");
        assert_eq!(queue.state(handle).expect("state"), LoadState::NotLoaded);
        // The bytes are here and the budget says no: the request waits and
        // its reservation holds the room.
        assert_eq!(queue.reserved_bytes(), 30);
        assert!(matches!(
            queue.enqueue("src/a.gltf", 2, 10).unwrap_err(),
            LoadError::OverBudget { .. }
        ));
        // Cancelling the waiting request frees the bytes again.
        queue.cancel(handle).expect("cancel");
        assert_eq!(queue.reserved_bytes(), 0);
        let small = queue.enqueue("src/a.gltf", 2, 10).expect("enqueue");
        let events = queue.pump(1 << 20);
        assert!(matches!(
            events.first(),
            Some(LoadEvent::Loaded { handle: got, .. }) if *got == small
        ));
    }

    #[test]
    fn queue_cancels_a_queued_load_and_frees_its_bytes() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        let handle = queue.enqueue("src/a.gltf", 1, 10).expect("enqueue");
        assert_eq!(queue.reserved_bytes(), 10);
        let event = queue.cancel(handle).expect("cancel");
        assert_eq!(
            event,
            LoadEvent::Cancelled {
                handle,
                path: "src/a.gltf".to_string(),
            }
        );
        assert_eq!(queue.reserved_bytes(), 0);
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.state(handle).expect("state"), LoadState::Cancelled);
        assert!(queue.payload(handle).is_err());
        // A cancelled handle is not resident, so unloading it is refused.
        assert!(matches!(
            queue.unload(handle).unwrap_err(),
            LoadError::WrongState { .. }
        ));
    }

    #[test]
    fn queue_unload_gives_the_bytes_back_and_stales_the_handle() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        let handle = queue.enqueue("src/a.gltf", 1, 0).expect("enqueue");
        queue.pump(1 << 20);
        assert_eq!(queue.resident_bytes(), 10);
        let event = queue.unload(handle).expect("unload");
        assert_eq!(
            event,
            LoadEvent::Unloaded {
                handle,
                path: "src/a.gltf".to_string(),
                bytes: 10,
            }
        );
        assert_eq!(queue.resident_bytes(), 0);
        assert_eq!(queue.remaining_bytes(), 1 << 20);
        assert!(!queue.is_ready(handle));
        assert!(matches!(
            queue.payload(handle).unwrap_err(),
            LoadError::Unknown { .. }
        ));
        // The same path stages again with a fresh generation.
        let again = queue.enqueue("src/a.gltf", 1, 0).expect("enqueue");
        assert_eq!(again.index, handle.index);
        assert_ne!(again.generation, handle.generation);
    }

    #[test]
    fn queue_names_the_file_the_source_refuses() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        let handle = queue.enqueue("src/gone.gltf", 1, 0).expect("enqueue");
        let events = queue.pump(1 << 20);
        match events.first() {
            Some(LoadEvent::Failed { path, reason, .. }) => {
                assert_eq!(path, "src/gone.gltf");
                assert!(reason.contains("src/gone.gltf"), "{reason}");
            }
            other => panic!("expected a named failure, got {other:?}"),
        }
        assert_eq!(queue.state(handle).expect("state"), LoadState::Failed);
        assert_eq!(
            queue.failure_reason(handle),
            Some("asset \"src/gone.gltf\" not found")
        );
    }

    #[test]
    fn queue_keeps_one_handle_per_path() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        let first = queue.enqueue("src/a.gltf", 1, 0).expect("enqueue");
        let again = queue.enqueue("src/a.gltf", 2, 0).expect("enqueue");
        assert_eq!(first, again);
        assert_eq!(queue.pending_count(), 1);
        assert_eq!(queue.handles().len(), 1);
    }

    #[test]
    fn queue_refuses_a_bad_path_and_never_panics() {
        let source = source();
        let mut queue = queue(&source, 1 << 20);
        assert!(matches!(
            queue.enqueue("../escape", 1, 0).unwrap_err(),
            LoadError::BadPath { .. }
        ));
        assert!(matches!(
            queue.enqueue("/abs", 1, 0).unwrap_err(),
            LoadError::BadPath { .. }
        ));
        assert_eq!(queue.pending_count(), 0);
        let ghost = AssetHandle {
            index: 42,
            generation: 1,
        };
        assert!(matches!(
            queue.cancel(ghost).unwrap_err(),
            LoadError::Unknown { index: 42, .. }
        ));
        assert!(matches!(
            queue.unload(ghost).unwrap_err(),
            LoadError::Unknown { index: 42, .. }
        ));
    }

    #[test]
    fn every_error_and_event_displays() {
        let errors = [
            LoadError::BadPath {
                path: "x".to_string(),
                reason: "empty",
            },
            LoadError::AssetCap { limit: 2 },
            LoadError::OverBudget {
                want_bytes: 9,
                free_bytes: 1,
            },
            LoadError::Unknown {
                index: 1,
                generation: 2,
            },
            LoadError::WrongState {
                index: 1,
                generation: 2,
                state: LoadState::Loading,
            },
            LoadError::Source(SourceError::missing("a.gltf")),
            LoadError::Store(HandleError::TooManyDeps { limit: 1 }),
        ];
        for err in &errors {
            assert!(!format!("{err}").is_empty());
            assert!(!format!("{err:?}").is_empty());
        }
        let handle = AssetHandle {
            index: 3,
            generation: 4,
        };
        let events = [
            LoadEvent::Loaded {
                handle,
                path: "a.gltf".to_string(),
                bytes: 5,
            },
            LoadEvent::Deferred {
                handle,
                path: "a.gltf".to_string(),
                reason: "budget".to_string(),
            },
            LoadEvent::Failed {
                handle,
                path: "a.gltf".to_string(),
                reason: "gone".to_string(),
            },
            LoadEvent::Cancelled {
                handle,
                path: "a.gltf".to_string(),
            },
            LoadEvent::Unloaded {
                handle,
                path: "a.gltf".to_string(),
                bytes: 5,
            },
        ];
        for event in &events {
            assert!(!format!("{event:?}").is_empty());
        }
    }
}
