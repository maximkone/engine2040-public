//! Bounded deterministic re-query store for `play.step`, one per session.
//!
//! Consumer: the session's [`PlaySlot`](super::play_session::PlaySlot), read
//! by [`play_step_with`](super::play_session::play_step_with), the MCP tool
//! path and [`PlaySession::advance`](super::play_session::PlaySession::advance),
//! all through `step_core`. Data owner for simulation stays `engine-arena`; this
//! module owns only the bounded receipt cache: a validated composition in,
//! the identical receipt out. Units follow arena (meters, fixed ticks);
//! receipts are sRGB PNG bytes from the software rasterizer. Determinism
//! scope: the key binds every receipt-varying scalar (seed, participants,
//! half-extent bits, `upto_tick`, receipt size) plus the canonical tape
//! bytes, so the same composition twice returns the same digest and the same
//! PNG bytes without re-rendering. Each session owns its cache, so one
//! session's compositions never fill, evict from or read another's. The table is a `BTreeMap` with FIFO
//! eviction: stable order, no wall clock, no threads, no pointer values in
//! the key. Errors: a poisoned lock fails as `lock_poisoned` naming the
//! restart. Budget: at most [`REQUERY_CACHE_ENTRIES`] live entries; a lookup
//! clones one receipt (service path, never the fixed tick). The hooked GPU
//! path bypasses the store: the device owns those pixels, so the endpoint
//! cannot promise their bytes.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use engine_core::hash_bytes;

use super::play_session::PlayStepOutput;
use super::PlayInputDto;
use crate::mcp::McpToolError;

/// Live receipts kept per session and validated composition (mirrors the
/// `MAX_JOBS` session bound: small enough to hold, big enough for a review sweep).
pub(crate) const REQUERY_CACHE_ENTRIES: usize = 16;

/// Identity of one validated software composition: every scalar that can
/// change the receipt plus the canonical tape hash. Ordered so the table is
/// a `BTreeMap` with stable iteration; the half-extent enters as bits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RequeryKey {
    seed: u64,
    participants: u8,
    half_bits: u32,
    upto: u32,
    width: u32,
    height: u32,
    tape_hash: u64,
}

/// Inputs to [`requery_key`]: the validated composition; the tape is
/// canonicalized inside, so callers pass the slice they validated.
#[derive(Debug)]
pub(crate) struct RequeryParams<'a> {
    /// Match seed the receipt simulates from.
    pub seed: u64,
    /// Participant count, `2..=MAX_PARTICIPANTS`.
    pub participants: u8,
    /// Half-extent bits (`f32::to_bits` of meters, finite and ranged).
    pub half_bits: u32,
    /// Last tick covered, `1..=MAX_PLAY_TICKS`.
    pub upto: u32,
    /// Receipt width after defaults resolve.
    pub width: u32,
    /// Receipt height after defaults resolve.
    pub height: u32,
    /// Validated tape slice in `(tick, fighter)` order.
    pub inputs: &'a [PlayInputDto],
}

/// Canonical tape bytes: DTOs in `(tick, fighter)` order (stable sort, so
/// every valid order maps to one identity), 10 bytes each in the
/// canonical tape layout (little-endian ticks/facing, raw wish bytes).
fn canonical_tape_bytes(inputs: &[PlayInputDto]) -> Vec<u8> {
    let mut sorted = inputs.to_vec();
    sorted.sort_by_key(|dto| (dto.tick, dto.fighter));
    let mut bytes = Vec::with_capacity(sorted.len() * 10);
    for dto in &sorted {
        bytes.extend_from_slice(&dto.tick.to_le_bytes());
        bytes.push(dto.fighter);
        bytes.extend_from_slice(&dto.mx.to_le_bytes());
        bytes.extend_from_slice(&dto.mz.to_le_bytes());
        bytes.extend_from_slice(&dto.facing.to_le_bytes());
        bytes.push(dto.buttons);
    }
    bytes
}

/// Key one validated composition. Callers validate first (the store never
/// sees an invalid tape); the stable sort only canonicalizes valid orders.
pub(crate) fn requery_key(params: &RequeryParams<'_>) -> RequeryKey {
    RequeryKey {
        seed: params.seed,
        participants: params.participants,
        half_bits: params.half_bits,
        upto: params.upto,
        width: params.width,
        height: params.height,
        tape_hash: hash_bytes(&canonical_tape_bytes(params.inputs)),
    }
}

/// One cached receipt: the canonical bytes (verified on lookup, so even a
/// 64-bit hash collision can only miss, never return another composition's
/// bytes) plus the receipt to replay.
#[derive(Debug, Clone)]
struct RequeryEntry {
    tape: Vec<u8>,
    out: PlayStepOutput,
}

/// Bounded table plus FIFO eviction order (oldest key first).
#[derive(Debug, Default)]
struct RequeryStore {
    map: BTreeMap<RequeryKey, RequeryEntry>,
    order: VecDeque<RequeryKey>,
}

impl RequeryStore {
    /// Replay the stored receipt only when the canonical bytes match
    /// exactly; anything else (evicted key, hash collision) misses.
    fn lookup_verified(&self, key: &RequeryKey, tape: &[u8]) -> Option<PlayStepOutput> {
        self.map
            .get(key)
            .and_then(|entry| (entry.tape == tape).then(|| entry.out.clone()))
    }

    /// File one receipt, evicting the oldest key once full. Re-filing an
    /// open key refreshes its receipt without growing the table or moving
    /// its eviction slot, so revisits never churn the order.
    fn insert(&mut self, key: RequeryKey, tape: Vec<u8>, out: PlayStepOutput) {
        use std::collections::btree_map::Entry;
        match self.map.entry(key) {
            Entry::Occupied(mut slot) => {
                slot.insert(RequeryEntry { tape, out });
            }
            Entry::Vacant(slot) => {
                let filed = slot.key().clone();
                slot.insert(RequeryEntry { tape, out });
                self.order.push_back(filed);
                if self.order.len() > REQUERY_CACHE_ENTRIES {
                    if let Some(oldest) = self.order.pop_front() {
                        self.map.remove(&oldest);
                    }
                }
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    fn contains(&self, key: &RequeryKey) -> bool {
        self.map.contains_key(key)
    }
}

/// One session's re-query cache: the bounded store behind a lock, so every
/// reader takes `&self`. Lives in the session's `PlaySlot`.
#[derive(Debug, Default)]
pub(crate) struct RequeryCache {
    store: Mutex<RequeryStore>,
}

impl RequeryCache {
    /// Lock the store; a poisoned lock names the restart.
    fn lock_store(&self) -> Result<MutexGuard<'_, RequeryStore>, McpToolError> {
        self.store.lock().map_err(|_| {
            McpToolError::new(
                "lock_poisoned",
                "re-query store lock poisoned by a failed call; restart the server",
            )
        })
    }

    /// Replay the stored receipt for this validated composition, or `None`
    /// on the first query (callers then simulate and
    /// [`requery_store`](Self::requery_store)).
    pub(crate) fn requery_lookup(
        &self,
        key: &RequeryKey,
        inputs: &[PlayInputDto],
    ) -> Result<Option<PlayStepOutput>, McpToolError> {
        let tape = canonical_tape_bytes(inputs);
        self.lock_store()
            .map(|store| store.lookup_verified(key, &tape))
    }

    /// File one simulated receipt for this validated composition for the
    /// next identical re-query.
    pub(crate) fn requery_store(
        &self,
        key: &RequeryKey,
        inputs: &[PlayInputDto],
        out: &PlayStepOutput,
    ) -> Result<(), McpToolError> {
        let tape = canonical_tape_bytes(inputs);
        self.lock_store()
            .map(|mut store| store.insert(key.clone(), tape, out.clone()))
    }

    /// Receipts filed so far. Test-only: proves cache isolation.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock_store().map_or(0, |store| store.len())
    }

    /// True when this composition's key is filed. Test-only.
    #[cfg(test)]
    pub(crate) fn contains(&self, key: &RequeryKey) -> bool {
        self.lock_store().is_ok_and(|store| store.contains(key))
    }
}

#[cfg(test)]
mod tests {
    use super::super::CaptureManifestDto;
    use super::*;

    fn dto(tick: u32, fighter: u8, buttons: u8) -> PlayInputDto {
        PlayInputDto {
            tick,
            fighter,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons,
        }
    }

    fn params(inputs: &[PlayInputDto]) -> RequeryParams<'_> {
        RequeryParams {
            seed: 95,
            participants: 2,
            half_bits: 20.0_f32.to_bits(),
            upto: 120,
            width: 96,
            height: 64,
            inputs,
        }
    }

    fn output(digest: u64) -> PlayStepOutput {
        PlayStepOutput {
            tick: 120,
            digest,
            participants: 2,
            inputs_applied: 1,
            summary: format!("tick 120 duel 2 participants digest {digest:016x} inputs 1"),
            manifest: CaptureManifestDto {
                adapter: "test-adapter".to_owned(),
                driver: "test-driver".to_owned(),
                width: 96,
                height: 64,
                validation_errors: 0,
            },
            png_bytes: {
                let mut bytes = PNG_MAGIC.to_vec();
                bytes.extend_from_slice(&digest.to_le_bytes());
                bytes
            },
            fighters: Vec::new(),
            events: Vec::new(),
        }
    }

    /// PNG file magic, mirrored from the software receipt (never imported).
    const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

    #[test]
    fn key_stable_canonical_and_sensitive() {
        let tape = vec![dto(0, 0, 1), dto(30, 0, 0), dto(60, 1, 1)];
        let once = requery_key(&params(&tape));
        let twice = requery_key(&params(&tape));
        assert_eq!(once, twice, "same composition keys identically");

        // Valid-order permutations canonicalize to one identity.
        let mut shuffled = tape.clone();
        shuffled.reverse();
        assert_eq!(
            requery_key(&params(&shuffled)),
            once,
            "tape order never splits a composition"
        );

        // Every receipt-varying scalar splits the identity.
        let mut other_seed = params(&tape);
        other_seed.seed = 96;
        assert_ne!(requery_key(&other_seed), once, "seed splits");
        let mut other_upto = params(&tape);
        other_upto.upto = 60;
        assert_ne!(requery_key(&other_upto), once, "upto splits");
        let mut other_size = params(&tape);
        other_size.width = 64;
        assert_ne!(requery_key(&other_size), once, "size splits");
        let changed = vec![dto(0, 0, 2), dto(30, 0, 0), dto(60, 1, 1)];
        assert_ne!(requery_key(&params(&changed)), once, "tape bytes split");
    }

    #[test]
    fn store_bounds_fifo_and_refreshes() {
        let mut store = RequeryStore::default();
        let mut keys = Vec::new();
        for index in 0..REQUERY_CACHE_ENTRIES + 1 {
            let tape = vec![dto(index as u32, 0, 1)];
            let key = requery_key(&params(&tape));
            store.insert(
                key.clone(),
                canonical_tape_bytes(&tape),
                output(index as u64),
            );
            keys.push((key, tape));
        }
        assert_eq!(
            store.len(),
            REQUERY_CACHE_ENTRIES,
            "the table never grows past its bound"
        );
        assert!(
            !store.contains(&keys[0].0),
            "the oldest composition evicts first"
        );
        assert!(
            store.contains(&keys[keys.len() - 1].0),
            "the newest composition stays"
        );

        // Re-filing an open key refreshes its receipt without growing.
        let (refiled_key, refiled_tape) = keys[1].clone();
        store.insert(
            refiled_key.clone(),
            canonical_tape_bytes(&refiled_tape),
            output(999),
        );
        assert_eq!(store.len(), REQUERY_CACHE_ENTRIES);
        let replayed = store
            .lookup_verified(&refiled_key, &canonical_tape_bytes(&refiled_tape))
            .expect("refiled key replays");
        assert_eq!(replayed.digest, 999, "re-file refreshes the receipt");
    }

    #[test]
    fn lookup_replays_exact_bytes_only() {
        let cache = RequeryCache::default();
        let tape = vec![dto(0, 0, 1), dto(100, 1, 2)];
        let key = requery_key(&params(&tape));
        let out = output(0x6365_6637_1440_1405);
        cache.requery_store(&key, &tape, &out).expect("store files");
        let hit = cache
            .requery_lookup(&key, &tape)
            .expect("lookup serves")
            .expect("filed composition hits");
        assert_eq!(hit.digest, out.digest);
        assert_eq!(hit.png_bytes, out.png_bytes);

        // Same key shape with different bytes never replays another
        // composition's receipt: a 64-bit hash collision can only miss.
        let other = vec![dto(0, 0, 1), dto(100, 1, 4)];
        let miss = cache.requery_lookup(&key, &other).expect("lookup serves");
        assert!(miss.is_none(), "byte mismatch misses, never replays");
    }

    #[test]
    fn caches_are_separate_per_owner() {
        let (first, second) = (RequeryCache::default(), RequeryCache::default());
        let tape = vec![dto(0, 0, 1)];
        let key = requery_key(&params(&tape));
        first
            .requery_store(&key, &tape, &output(7))
            .expect("store files");
        assert_eq!((first.len(), second.len()), (1, 0));
        assert!(first.contains(&key) && !second.contains(&key));
        assert!(
            second
                .requery_lookup(&key, &tape)
                .expect("lookup serves")
                .is_none(),
            "another cache never replays this cache's receipt"
        );
    }
}
