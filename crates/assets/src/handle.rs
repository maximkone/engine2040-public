//! handle: generational asset handles plus load states over the stream queue.
//!
//! Owner: `engine-assets`; planned, no caller yet (verified 2026-10-16):
//! the game kit import plus cook pipeline will stage one handle per kit file,
//! then read ready payloads at init. Planned caller: `crates/game/src/arena_kit.rs`.
//! Units: bytes for payload budgets, counts for caps. Determinism: index
//! order iteration, `BTreeMap` key index, `engine_core::hash_bytes` keys;
//! no wall clock, no floats. Errors: typed [`HandleError`], never panics.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Largest single asset payload in bytes; mirrors the stream payload cap.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
/// Most direct dependencies one asset may declare.
pub const MAX_DEPS: usize = 32;
/// Longest stored failure reason in chars.
pub const MAX_FAIL_CHARS: usize = 256;

/// Content key for one asset: `engine_core::hash_bytes` of its source bytes.
/// Same bytes give the same key on every PC; the game kit import keys each
/// kit file this way before staging a handle.
pub type AssetKey = u64;

/// Generational handle into [`AssetStore`].
/// The index names a slot; the generation rejects stale uses after [`AssetStore::free`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct AssetHandle {
    /// Slot index.
    pub index: u32,
    /// Slot generation; zero never names a live asset.
    pub generation: u32,
}

impl AssetHandle {
    /// Never-live handle; every lookup refuses it with [`HandleError::Unknown`].
    pub const INVALID: Self = Self {
        index: 0,
        generation: 0,
    };

    /// True for a handle that may name a live slot (nonzero generation).
    pub fn is_valid(self) -> bool {
        self.generation != 0
    }
}

impl fmt::Display for AssetHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "asset {}:{}", self.index, self.generation)
    }
}

/// Load state of one asset: present, in flight, resident or refused.
/// Mirrors the Bevy asset-server states the kit pipeline drives at init.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LoadState {
    /// Imported but never requested.
    NotLoaded,
    /// Requested; the stream queue owns the bytes in flight.
    Loading,
    /// Cooked payload resident and readable.
    Loaded,
    /// The load or cook was refused; see [`AssetStore::failure`].
    Failed,
}

impl LoadState {
    /// Lowercase state name for logs and diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotLoaded => "not_loaded",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }

    /// True for [`LoadState::Loaded`].
    pub fn is_loaded(self) -> bool {
        self == Self::Loaded
    }
}

impl fmt::Display for LoadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why an asset-handle call was refused; every failure stages nothing new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandleError {
    /// The handle names no live slot (bad index or stale generation).
    Unknown {
        /// Attempted slot index.
        index: u32,
        /// Attempted generation.
        generation: u32,
    },
    /// The store already holds its asset-slot budget.
    AssetCap {
        /// Slot budget that is full.
        limit: u32,
    },
    /// Payload byte count outside `1..=MAX_PAYLOAD_BYTES`.
    TooLarge {
        /// Offending byte count.
        count: usize,
    },
    /// The import declares more than [`MAX_DEPS`] direct dependencies.
    TooManyDeps {
        /// Dependency budget that is full.
        limit: usize,
    },
    /// A declared dependency names no live slot.
    MissingDependency {
        /// Attempted dependency slot index.
        index: u32,
        /// Attempted dependency generation.
        generation: u32,
    },
    /// Payload byte budget would be exceeded by the cooked bytes.
    OverBudget {
        /// Payload bytes the mark wanted resident.
        want_bytes: u64,
        /// Bytes free under the budget without the old payload.
        free_bytes: u64,
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
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { index, generation } => {
                write!(f, "unknown asset handle {index}:{generation}")
            }
            Self::AssetCap { limit } => write!(f, "asset cap reached: {limit}"),
            Self::TooLarge { count } => write!(f, "asset too large: {count}"),
            Self::TooManyDeps { limit } => write!(f, "too many dependencies: {limit}"),
            Self::MissingDependency { index, generation } => {
                write!(f, "missing dependency {index}:{generation}")
            }
            Self::OverBudget {
                want_bytes,
                free_bytes,
            } => write!(
                f,
                "asset over budget: want {want_bytes} bytes, free {free_bytes}"
            ),
            Self::WrongState {
                index,
                generation,
                state,
            } => write!(f, "asset {index}:{generation} is {state}, want loaded"),
        }
    }
}

impl std::error::Error for HandleError {}

/// Memory plus slot budget for one store. Units: bytes for `bytes`,
/// live assets for `assets`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandleBudget {
    /// Cap on resident cooked payload bytes.
    pub bytes: u64,
    /// Cap on live assets (imported and not freed).
    pub assets: u32,
}

impl HandleBudget {
    /// Budget with a byte cap and a live-asset cap.
    pub fn new(bytes: u64, assets: u32) -> Self {
        Self { bytes, assets }
    }
}

/// One slot: identity plus load state plus last-good payload.
#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    key: AssetKey,
    state: LoadState,
    deps: Vec<AssetHandle>,
    payload: Vec<u8>,
    fail: String,
    live: bool,
}

/// Generational asset-handle store with load states and dependency readiness.
///
/// The kit pipeline imports one handle per kit file, drives
/// `NotLoaded` to `Loading` to `Loaded` (or `Failed`), and serves a payload
/// only when the asset and every transitive dependency are `Loaded`.
/// Same source bytes import to the same live handle, so repeated kit imports
/// stage once. Single-thread driven; no wall clock, no floats, index order
/// iteration only.
#[derive(Clone, Debug)]
pub struct AssetStore {
    budget: HandleBudget,
    slots: Vec<Slot>,
    free: Vec<u32>,
    keys: BTreeMap<AssetKey, AssetHandle>,
    live: u32,
}

impl AssetStore {
    /// Empty store under `budget`; nothing resident.
    pub fn new(budget: HandleBudget) -> Self {
        Self {
            budget,
            slots: Vec::new(),
            free: Vec::new(),
            keys: BTreeMap::new(),
            live: 0,
        }
    }

    /// Active budget (byte cap plus live-asset cap).
    pub fn budget(&self) -> HandleBudget {
        self.budget
    }

    /// Live asset count.
    pub fn len(&self) -> usize {
        match usize::try_from(self.live) {
            Ok(v) => v,
            Err(_) => self.slots.len(),
        }
    }

    /// True when no assets are live.
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Resident cooked payload bytes; units: bytes.
    pub fn used_bytes(&self) -> u64 {
        let mut used: u64 = 0;
        for slot in &self.slots {
            if slot.live && slot.state == LoadState::Loaded {
                used = used.saturating_add(len_u64(slot.payload.len()));
            }
        }
        used
    }

    /// Bytes still admittable; units: bytes.
    pub fn remaining_bytes(&self) -> u64 {
        self.budget.bytes.saturating_sub(self.used_bytes())
    }

    /// Live slots still admittable.
    pub fn remaining_assets(&self) -> u64 {
        u64::from(self.budget.assets).saturating_sub(u64::from(self.live))
    }

    /// True while the handle names a live slot with a matching generation.
    pub fn contains(&self, handle: AssetHandle) -> bool {
        self.live_slot(handle).is_some()
    }

    /// Current load state; stale handles fail [`HandleError::Unknown`].
    pub fn state(&self, handle: AssetHandle) -> Result<LoadState, HandleError> {
        match self.live_slot(handle) {
            Some(slot) => Ok(slot.state),
            None => Err(unknown_of(handle)),
        }
    }

    /// Content key of one live asset; stale handles fail.
    pub fn key_of(&self, handle: AssetHandle) -> Result<AssetKey, HandleError> {
        match self.live_slot(handle) {
            Some(slot) => Ok(slot.key),
            None => Err(unknown_of(handle)),
        }
    }

    /// Direct dependencies of one live asset; stale handles fail.
    pub fn deps_of(&self, handle: AssetHandle) -> Result<Vec<AssetHandle>, HandleError> {
        match self.live_slot(handle) {
            Some(slot) => Ok(slot.deps.clone()),
            None => Err(unknown_of(handle)),
        }
    }

    /// Resident cooked payload; only [`LoadState::Loaded`] reads succeed.
    pub fn payload(&self, handle: AssetHandle) -> Result<&[u8], HandleError> {
        match self.live_slot(handle) {
            Some(slot) if slot.state == LoadState::Loaded => Ok(slot.payload.as_slice()),
            Some(slot) => Err(HandleError::WrongState {
                index: handle.index,
                generation: handle.generation,
                state: slot.state,
            }),
            None => Err(unknown_of(handle)),
        }
    }

    /// Failure reason for [`LoadState::Failed`], `None` for other live states.
    pub fn failure(&self, handle: AssetHandle) -> Result<Option<&str>, HandleError> {
        match self.live_slot(handle) {
            Some(slot) if slot.state == LoadState::Failed => Ok(Some(slot.fail.as_str())),
            Some(_) => Ok(None),
            None => Err(unknown_of(handle)),
        }
    }

    /// True when the asset and every transitive dependency are `Loaded`.
    /// Unknown, stale, loading or failed handles read as not ready, never fail.
    pub fn is_ready(&self, handle: AssetHandle) -> bool {
        if self.live_slot(handle).is_none() {
            return false;
        }
        let mut seen: BTreeSet<AssetHandle> = BTreeSet::new();
        let mut stack: Vec<AssetHandle> = vec![handle];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            let slot = match self.live_slot(current) {
                Some(slot) => slot,
                None => return false,
            };
            if slot.state != LoadState::Loaded {
                return false;
            }
            for dep in &slot.deps {
                stack.push(*dep);
            }
        }
        true
    }

    /// Live handles in slot-index order for deterministic browser rows.
    pub fn live_handles(&self) -> Vec<AssetHandle> {
        let mut out = Vec::new();
        for (at, slot) in self.slots.iter().enumerate() {
            if !slot.live {
                continue;
            }
            let index = match u32::try_from(at) {
                Ok(v) => v,
                Err(_) => continue,
            };
            out.push(AssetHandle {
                index,
                generation: slot.generation,
            });
        }
        out
    }

    /// Ready handles in slot-index order: loaded with all deps loaded.
    pub fn ready_handles(&self) -> Vec<AssetHandle> {
        let mut out = Vec::new();
        for handle in self.live_handles() {
            if self.is_ready(handle) {
                out.push(handle);
            }
        }
        out
    }

    /// Import source bytes with direct dependencies.
    /// Same bytes return the same live handle without staging a duplicate.
    /// Unknown or stale dependencies, oversize payloads and full stores fail
    /// without staging anything.
    pub fn import(
        &mut self,
        source: &[u8],
        deps: &[AssetHandle],
    ) -> Result<AssetHandle, HandleError> {
        if source.is_empty() || source.len() > MAX_PAYLOAD_BYTES {
            return Err(HandleError::TooLarge {
                count: source.len(),
            });
        }
        if deps.len() > MAX_DEPS {
            return Err(HandleError::TooManyDeps { limit: MAX_DEPS });
        }
        for dep in deps {
            if !self.contains(*dep) {
                return Err(HandleError::MissingDependency {
                    index: dep.index,
                    generation: dep.generation,
                });
            }
        }
        let key = engine_core::hash_bytes(source);
        if let Some(existing) = self.keys.get(&key).copied() {
            if self.contains(existing) {
                return Ok(existing);
            }
            self.keys.remove(&key);
        }
        if self.live >= self.budget.assets {
            return Err(HandleError::AssetCap {
                limit: self.budget.assets,
            });
        }
        if let Some(index) = self.free.pop() {
            let at = match usize::try_from(index) {
                Ok(v) => v,
                Err(_) => {
                    return Err(HandleError::Unknown {
                        index,
                        generation: 0,
                    });
                }
            };
            match self.slots.get_mut(at) {
                Some(slot) => {
                    slot.key = key;
                    slot.state = LoadState::NotLoaded;
                    slot.deps = deps.to_vec();
                    slot.payload = Vec::new();
                    slot.fail = String::new();
                    slot.live = true;
                    let handle = AssetHandle {
                        index,
                        generation: slot.generation,
                    };
                    self.keys.insert(key, handle);
                    self.live = self.live.saturating_add(1);
                    return Ok(handle);
                }
                None => {
                    return Err(HandleError::Unknown {
                        index,
                        generation: 0,
                    });
                }
            }
        }
        let index = match u32::try_from(self.slots.len()) {
            Ok(v) => v,
            Err(_) => {
                return Err(HandleError::AssetCap {
                    limit: self.budget.assets,
                });
            }
        };
        self.slots.push(Slot {
            generation: 1,
            key,
            state: LoadState::NotLoaded,
            deps: deps.to_vec(),
            payload: Vec::new(),
            fail: String::new(),
            live: true,
        });
        let handle = AssetHandle {
            index,
            generation: 1,
        };
        self.keys.insert(key, handle);
        self.live = self.live.saturating_add(1);
        Ok(handle)
    }

    /// Move `NotLoaded` (or retry `Failed`) to `Loading`; idempotent otherwise.
    pub fn request_load(&mut self, handle: AssetHandle) -> Result<LoadState, HandleError> {
        let slot = match self.live_slot_mut(handle) {
            Some(slot) => slot,
            None => return Err(unknown_of(handle)),
        };
        match slot.state {
            LoadState::NotLoaded | LoadState::Failed => {
                slot.state = LoadState::Loading;
                slot.fail = String::new();
            }
            LoadState::Loading | LoadState::Loaded => {}
        }
        Ok(slot.state)
    }

    /// Store cooked bytes and move to `Loaded`.
    /// Over-budget cooks fail without touching the stored state or payload.
    pub fn mark_loaded(
        &mut self,
        handle: AssetHandle,
        cooked: &[u8],
    ) -> Result<LoadState, HandleError> {
        if cooked.is_empty() || cooked.len() > MAX_PAYLOAD_BYTES {
            return Err(HandleError::TooLarge {
                count: cooked.len(),
            });
        }
        let cooked_bytes = len_u64(cooked.len());
        let used = self.used_bytes();
        let old = match self.live_slot(handle) {
            Some(slot) => len_u64(slot.payload.len()),
            None => return Err(unknown_of(handle)),
        };
        let without_old = used.saturating_sub(old);
        if without_old.saturating_add(cooked_bytes) > self.budget.bytes {
            return Err(HandleError::OverBudget {
                want_bytes: cooked_bytes,
                free_bytes: self.budget.bytes.saturating_sub(without_old),
            });
        }
        let slot = match self.live_slot_mut(handle) {
            Some(slot) => slot,
            None => return Err(unknown_of(handle)),
        };
        slot.payload = cooked.to_vec();
        slot.state = LoadState::Loaded;
        slot.fail = String::new();
        Ok(slot.state)
    }

    /// Move a non-resident asset to `Failed` with a bounded reason.
    /// Resident assets refuse with [`HandleError::WrongState`] so a failure
    /// never clobbers a good payload.
    pub fn mark_failed(
        &mut self,
        handle: AssetHandle,
        reason: &str,
    ) -> Result<LoadState, HandleError> {
        let slot = match self.live_slot_mut(handle) {
            Some(slot) => slot,
            None => return Err(unknown_of(handle)),
        };
        if slot.state == LoadState::Loaded {
            return Err(HandleError::WrongState {
                index: handle.index,
                generation: handle.generation,
                state: slot.state,
            });
        }
        slot.state = LoadState::Failed;
        slot.fail = bound_fail(reason);
        slot.payload = Vec::new();
        Ok(slot.state)
    }

    /// Free one live handle: stale generations fail afterwards, the slot is
    /// reusable with a bumped generation, and the content key unmaps.
    pub fn free(&mut self, handle: AssetHandle) -> Result<(), HandleError> {
        let at = match usize::try_from(handle.index) {
            Ok(v) => v,
            Err(_) => return Err(unknown_of(handle)),
        };
        let key = match self.slots.get(at) {
            Some(slot)
                if slot.live && slot.generation == handle.generation && handle.is_valid() =>
            {
                slot.key
            }
            _ => return Err(unknown_of(handle)),
        };
        if self.keys.get(&key).copied() == Some(handle) {
            self.keys.remove(&key);
        }
        match self.slots.get_mut(at) {
            Some(slot) => {
                slot.live = false;
                slot.state = LoadState::NotLoaded;
                slot.deps = Vec::new();
                slot.payload = Vec::new();
                slot.fail = String::new();
                slot.generation = slot.generation.wrapping_add(1);
                if slot.generation == 0 {
                    slot.generation = 1;
                }
            }
            None => return Err(unknown_of(handle)),
        }
        self.free.push(handle.index);
        self.live = self.live.saturating_sub(1);
        Ok(())
    }

    /// Live slot view: index in range, marked live, generation matches.
    fn live_slot(&self, handle: AssetHandle) -> Option<&Slot> {
        if !handle.is_valid() {
            return None;
        }
        let at = usize::try_from(handle.index).ok()?;
        let slot = self.slots.get(at)?;
        if slot.live && slot.generation == handle.generation {
            Some(slot)
        } else {
            None
        }
    }

    /// Live slot view for mutation.
    fn live_slot_mut(&mut self, handle: AssetHandle) -> Option<&mut Slot> {
        if !handle.is_valid() {
            return None;
        }
        let at = usize::try_from(handle.index).ok()?;
        let slot = self.slots.get_mut(at)?;
        if slot.live && slot.generation == handle.generation {
            Some(slot)
        } else {
            None
        }
    }
}

/// Stale-handle refusal for one attempted handle.
fn unknown_of(handle: AssetHandle) -> HandleError {
    HandleError::Unknown {
        index: handle.index,
        generation: handle.generation,
    }
}

/// Payload length as `u64` without truncation; saturates on exotic targets.
fn len_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Truncate a failure reason to at most [`MAX_FAIL_CHARS`] chars.
fn bound_fail(text: &str) -> String {
    if text.chars().count() <= MAX_FAIL_CHARS {
        text.to_string()
    } else {
        text.chars().take(MAX_FAIL_CHARS).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(bytes: u64, assets: u32) -> HandleBudget {
        HandleBudget::new(bytes, assets)
    }

    fn store() -> AssetStore {
        AssetStore::new(budget(1 << 20, 128))
    }

    fn bytes(seed: u8, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        for at in 0..len {
            let mix = engine_core::splitmix64(
                u64::from(seed)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .wrapping_add(at as u64),
            );
            out.push((mix & 0xff) as u8);
        }
        if out.iter().all(|b| *b == 0) {
            out[0] = seed.max(1);
        }
        out
    }

    #[test]
    fn same_bytes_same_key_and_handle() {
        let mut first = store();
        let mut second = store();
        let source = bytes(7, 64);
        let a = first.import(&source, &[]).unwrap();
        let b = first.import(&source, &[]).unwrap();
        assert_eq!(a, b);
        assert_eq!(first.len(), 1);
        assert_eq!(first.key_of(a).unwrap(), engine_core::hash_bytes(&source));
        let c = second.import(&source, &[]).unwrap();
        assert_eq!(second.key_of(c).unwrap(), first.key_of(a).unwrap());
        assert_eq!(
            engine_core::hash_bytes(&source),
            engine_core::hash_bytes(&source)
        );
        assert_ne!(
            engine_core::hash_bytes(&source),
            engine_core::hash_bytes(&bytes(8, 64))
        );
    }

    #[test]
    fn generations_bump_on_free() {
        let mut store = store();
        let source = bytes(3, 32);
        let handle = store.import(&source, &[]).unwrap();
        assert!(store.contains(handle));
        store.free(handle).unwrap();
        assert!(!store.contains(handle));
        assert!(!store.is_ready(handle));
        assert_eq!(
            store.state(handle).unwrap_err(),
            HandleError::Unknown {
                index: handle.index,
                generation: handle.generation,
            }
        );
        let again = store.import(&source, &[]).unwrap();
        assert_eq!(again.index, handle.index);
        assert_ne!(again.generation, handle.generation);
        assert!(store.contains(again));
        assert!(!store.contains(handle));
        assert_eq!(store.state(again).unwrap(), LoadState::NotLoaded);
    }

    #[test]
    fn load_states_advance_and_retry() {
        let mut store = store();
        let handle = store.import(&bytes(1, 16), &[]).unwrap();
        assert_eq!(store.state(handle).unwrap(), LoadState::NotLoaded);
        assert!(!store.is_ready(handle));
        assert_eq!(store.request_load(handle).unwrap(), LoadState::Loading);
        assert_eq!(store.request_load(handle).unwrap(), LoadState::Loading);
        assert!(store.payload(handle).is_err());
        let cooked = bytes(1, 16);
        assert_eq!(
            store.mark_loaded(handle, &cooked).unwrap(),
            LoadState::Loaded
        );
        assert_eq!(store.payload(handle).unwrap(), cooked.as_slice());
        assert!(store.is_ready(handle));
        assert_eq!(store.request_load(handle).unwrap(), LoadState::Loaded);
        assert!(store.mark_failed(handle, "late").is_err());
        assert_eq!(store.state(handle).unwrap(), LoadState::Loaded);
    }

    #[test]
    fn failed_blocks_dependents_until_retry() {
        let mut store = store();
        let a = store.import(&bytes(10, 16), &[]).unwrap();
        let b = store.import(&bytes(11, 16), &[a]).unwrap();
        store.request_load(a).unwrap();
        store.request_load(b).unwrap();
        store.mark_failed(a, "oven exploded").unwrap();
        assert_eq!(store.state(a).unwrap(), LoadState::Failed);
        assert_eq!(store.failure(a).unwrap(), Some("oven exploded"));
        assert!(!store.is_ready(a));
        assert!(!store.is_ready(b));
        store.mark_loaded(b, &bytes(11, 16)).unwrap();
        assert!(!store.is_ready(b));
        store.request_load(a).unwrap();
        assert_eq!(store.failure(a).unwrap(), None);
        store.mark_loaded(a, &bytes(10, 16)).unwrap();
        assert!(store.is_ready(a));
        assert!(store.is_ready(b));
        assert_eq!(store.ready_handles().len(), 2);
    }

    #[test]
    fn missing_dep_and_bad_input_refuse_cleanly() {
        let mut store = store();
        let ghost = AssetHandle {
            index: 90,
            generation: 1,
        };
        let before = store.len();
        assert_eq!(
            store.import(&bytes(1, 8), &[ghost]).unwrap_err(),
            HandleError::MissingDependency {
                index: 90,
                generation: 1,
            }
        );
        assert_eq!(store.len(), before);
        assert!(matches!(
            store.import(&[], &[]).unwrap_err(),
            HandleError::TooLarge { .. }
        ));
        assert!(matches!(
            store
                .import(&vec![1u8; MAX_PAYLOAD_BYTES + 1], &[])
                .unwrap_err(),
            HandleError::TooLarge { .. }
        ));
        let many = vec![AssetHandle::INVALID; MAX_DEPS + 1];
        assert_eq!(
            store.import(&bytes(2, 8), &many).unwrap_err(),
            HandleError::TooManyDeps { limit: MAX_DEPS }
        );
        assert_eq!(store.len(), before);
        assert_eq!(
            store.state(ghost).unwrap_err(),
            HandleError::Unknown {
                index: 90,
                generation: 1,
            }
        );
        assert_eq!(
            store.state(AssetHandle::INVALID).unwrap_err(),
            HandleError::Unknown {
                index: 0,
                generation: 0,
            }
        );
        let handle = store.import(&bytes(4, 8), &[]).unwrap();
        assert!(matches!(
            store.payload(handle).unwrap_err(),
            HandleError::WrongState { .. }
        ));
        assert_eq!(
            store.free(ghost).unwrap_err(),
            store.state(ghost).unwrap_err()
        );
    }

    #[test]
    fn caps_refuse_with_typed_errors() {
        let mut capped = AssetStore::new(budget(1 << 20, 2));
        capped.import(&bytes(1, 8), &[]).unwrap();
        capped.import(&bytes(2, 8), &[]).unwrap();
        assert_eq!(capped.remaining_assets(), 0);
        assert_eq!(
            capped.import(&bytes(3, 8), &[]).unwrap_err(),
            HandleError::AssetCap { limit: 2 }
        );
        assert_eq!(capped.len(), 2);

        let mut small = AssetStore::new(budget(16, 8));
        let a = small.import(&bytes(5, 8), &[]).unwrap();
        small.request_load(a).unwrap();
        small.mark_loaded(a, &bytes(5, 8)).unwrap();
        assert_eq!(small.used_bytes(), 8);
        assert_eq!(small.remaining_bytes(), 8);
        let b = small.import(&bytes(6, 16), &[]).unwrap();
        small.request_load(b).unwrap();
        let err = small.mark_loaded(b, &bytes(6, 16)).unwrap_err();
        assert_eq!(
            err,
            HandleError::OverBudget {
                want_bytes: 16,
                free_bytes: 8,
            }
        );
        assert_eq!(small.state(b).unwrap(), LoadState::Loading);
        assert_eq!(small.used_bytes(), 8);
    }

    #[test]
    fn fifty_asset_matrix_resolves_or_refuses() {
        let mut first = AssetStore::new(budget(1 << 20, 64));
        let mut second = AssetStore::new(budget(1 << 20, 64));
        let mut first_handles = Vec::new();
        let mut second_handles = Vec::new();
        for seed in 1..=50u8 {
            let source = bytes(seed, 32 + (seed as usize % 8));
            let first_deps: Vec<AssetHandle> = if first_handles.is_empty() {
                Vec::new()
            } else {
                vec![first_handles[(first_handles.len() - 1) % first_handles.len()]]
            };
            let second_deps: Vec<AssetHandle> = if second_handles.is_empty() {
                Vec::new()
            } else {
                vec![second_handles[(second_handles.len() - 1) % second_handles.len()]]
            };
            let a = first.import(&source, &first_deps).unwrap();
            let b = second.import(&source, &second_deps).unwrap();
            assert_eq!(first.key_of(a).unwrap(), second.key_of(b).unwrap());
            first_handles.push(a);
            second_handles.push(b);
        }
        assert_eq!(first.len(), 50);
        for (at, handle) in first_handles.iter().enumerate() {
            first.request_load(*handle).unwrap();
            second.request_load(second_handles[at]).unwrap();
            let source = bytes((at as u8) + 1, 32 + (((at as u8) + 1) as usize % 8));
            first.mark_loaded(*handle, &source).unwrap();
            second.mark_loaded(second_handles[at], &source).unwrap();
            assert_eq!(
                first.payload(*handle).unwrap(),
                second.payload(second_handles[at]).unwrap()
            );
        }
        assert_eq!(first.ready_handles().len(), 50);
        assert_eq!(second.ready_handles().len(), 50);
        for handle in &first_handles {
            assert!(first.is_ready(*handle));
        }
        let first_keys: Vec<AssetKey> = first_handles
            .iter()
            .map(|h| first.key_of(*h).unwrap())
            .collect();
        let mut sorted = first_keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 50);
    }

    #[test]
    fn stale_handle_never_panics_and_never_ready() {
        let mut store = store();
        let a = store.import(&bytes(20, 8), &[]).unwrap();
        let b = store.import(&bytes(21, 8), &[a]).unwrap();
        store.free(a).unwrap();
        assert!(!store.is_ready(a));
        assert!(!store.is_ready(b));
        assert_eq!(store.request_load(a).unwrap_err(), unknown_of(a));
        assert_eq!(
            store.mark_loaded(a, &bytes(20, 8)).unwrap_err(),
            unknown_of(a)
        );
        assert_eq!(store.mark_failed(a, "x").unwrap_err(), unknown_of(a));
        assert_eq!(store.free(a).unwrap_err(), unknown_of(a));
        assert_eq!(store.payload(a).unwrap_err(), unknown_of(a));
        assert_eq!(store.key_of(a).unwrap_err(), unknown_of(a));
        assert_eq!(store.deps_of(a).unwrap_err(), unknown_of(a));
        assert_eq!(store.failure(a).unwrap_err(), unknown_of(a));
        assert!(store.contains(b));
    }

    #[test]
    fn order_independent_keys() {
        let mut forward = store();
        let mut backward = store();
        let mut sources = Vec::new();
        for seed in 1..=10u8 {
            sources.push(bytes(seed, 24));
        }
        let mut fwd = Vec::new();
        for source in &sources {
            fwd.push(forward.import(source, &[]).unwrap());
        }
        let mut bwd = Vec::new();
        for source in sources.iter().rev() {
            bwd.push(backward.import(source, &[]).unwrap());
        }
        let mut fwd_keys: Vec<AssetKey> = fwd.iter().map(|h| forward.key_of(*h).unwrap()).collect();
        let mut bwd_keys: Vec<AssetKey> =
            bwd.iter().map(|h| backward.key_of(*h).unwrap()).collect();
        fwd_keys.sort_unstable();
        bwd_keys.sort_unstable();
        assert_eq!(fwd_keys, bwd_keys);
    }

    #[test]
    fn every_error_and_state_displays() {
        assert_eq!(LoadState::NotLoaded.to_string(), "not_loaded");
        assert_eq!(LoadState::Loading.to_string(), "loading");
        assert_eq!(LoadState::Loaded.to_string(), "loaded");
        assert_eq!(LoadState::Failed.to_string(), "failed");
        assert!(LoadState::Loaded.is_loaded());
        assert!(!LoadState::Loading.is_loaded());
        assert_eq!(AssetHandle::INVALID.to_string(), "asset 0:0");
        assert!(!AssetHandle::INVALID.is_valid());
        let errors = [
            HandleError::Unknown {
                index: 1,
                generation: 2,
            },
            HandleError::AssetCap { limit: 1 },
            HandleError::TooLarge { count: 1 },
            HandleError::TooManyDeps { limit: 1 },
            HandleError::MissingDependency {
                index: 1,
                generation: 1,
            },
            HandleError::OverBudget {
                want_bytes: 9,
                free_bytes: 1,
            },
            HandleError::WrongState {
                index: 1,
                generation: 1,
                state: LoadState::Loading,
            },
        ];
        for err in &errors {
            assert!(!format!("{err}").is_empty());
            assert!(!format!("{err:?}").is_empty());
        }
        assert_eq!(store().budget(), budget(1 << 20, 128));
        assert!(store().is_empty());
    }
}
