//! stream: demand-paged tile residency on the texture streaming path.
//! Owner: render texture streaming; consumers: arena match/capture plus the
//! dense fixture. Carries the LADDER:528 ACCEPTED spike (64-tex x16-tile
//! 120-frame arena sweep, 8-tex sliding window, 4 detail tiles/tex:
//! baseline whole-texture FIFO cap-16 posts 13456 tile uploads + 841 misses
//! -> candidate tile-LRU cap-192 + pinned L0 posts 3423 uploads, cut 74.6%,
//! 0 misses without L0 cover, digests x2) into the streaming path.
//! Units: texture ids (`u32`), tile ids (`u32`), counts (`u64`/`usize`).
//! Determinism: insertion/LRU order derives from the request stream only;
//! resident listings are sorted before hashing so digests are stable x2.
//! No float, no clock, no threads; timing lives in the test only (measured
//! with `Instant`, never faked).

use std::collections::{HashMap, HashSet, VecDeque};

// engine-core owns every hash constant (one hash owner): the digests below
// mix the same words in the same order as before, only the constant names
// moved, so `tile_digest`/`bulk_digest` values are bit-identical.
use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME, GOLDEN_GAMMA};

/// Typed streaming-path failure. The order queues track their maps
/// one-to-one, so a desync is an internal invariant break: production sites
/// report it loudly in debug builds and degrade gracefully in release
/// instead of panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// An order queue was empty when its map needed an eviction.
    /// Names the site so the caller can prove which invariant broke.
    OrderDesync {
        /// Call-site context naming the desynced queue.
        at: &'static str,
    },
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrderDesync { at } => {
                write!(f, "stream: order queue desynced from its map at {at}")
            }
        }
    }
}

impl std::error::Error for StreamError {}

/// Pop the oldest tracked key, or name the desynced site as a typed error.
fn pop_oldest<K>(order: &mut VecDeque<K>, at: &'static str) -> Result<K, StreamError> {
    order.pop_front().ok_or(StreamError::OrderDesync { at })
}

// ---------------------------------------------------------------------------
// FROZEN CpuCache stub: minimal FIFO beside the TileResidency hunks.
// Do not modify: the eviction policy here is frozen; the paged residency
// lives in `TileResidency` below. Mirrors the `present.rs:300-356` FIFO
// shape (insertion-order eviction, reinsert of a live key keeps order).
// ---------------------------------------------------------------------------

/// FROZEN minimal whole-entry FIFO cache (stub of `present::CpuCache`).
/// Capacity-bounded key store with insertion-order eviction. Reinserting a
/// live key replaces its value without disturbing the eviction order.
#[derive(Debug)]
pub struct CpuCache<K: Eq + std::hash::Hash + Clone, V> {
    capacity: usize,
    map: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Eq + std::hash::Hash + Clone, V> CpuCache<K, V> {
    /// Build with a nonzero capacity; zero capacity keeps nothing.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            map: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
        }
    }

    /// Insert, evicting the oldest key first when over capacity. Returns the
    /// evicted key, if any.
    pub fn insert(&mut self, key: K, value: V) -> Option<K> {
        use std::collections::hash_map::Entry;
        match self.map.entry(key) {
            Entry::Occupied(mut slot) => {
                slot.insert(value);
                None
            }
            Entry::Vacant(slot) => {
                if self.capacity == 0 {
                    let key = slot.into_key();
                    return Some(key);
                }
                self.order.push_back(slot.key().clone());
                slot.insert(value);
                if self.map.len() > self.capacity {
                    let oldest = match pop_oldest(&mut self.order, "CpuCache::insert") {
                        Ok(oldest) => oldest,
                        Err(error) => {
                            // Unreachable: every map insert pushes one order
                            // entry. Loud in debug, graceful (no eviction)
                            // in release.
                            debug_assert!(false, "{error}");
                            return None;
                        }
                    };
                    self.map.remove(&oldest);
                    return Some(oldest);
                }
                None
            }
        }
    }

    /// Look up a resident value.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    /// Resident entry count.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when nothing is resident.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Budget capacity backing this cache.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

// ---------------------------------------------------------------------------
// TileResidency: demand-paged tile residency.
// ---------------------------------------------------------------------------

/// Resident tile slots under LRU management (non-L0 tiles only).
/// Pinned L0 tiles live beside this budget and are never evicted.
pub const TILE_LRU_CAP: usize = 192;

/// Textures in the dense arena fixture.
pub const FIXTURE_TEXTURES: u32 = 64;
/// Tiles per texture in the dense arena fixture (tile 0 is L0).
pub const FIXTURE_TILES_PER_TEX: u32 = 16;
/// Sliding-window width (textures per frame) in the dense arena fixture.
pub const FIXTURE_WINDOW: u32 = 8;
/// Detail (non-L0) tiles requested per texture per frame.
pub const FIXTURE_DETAIL_TILES: u32 = 4;
/// Frames in the dense arena sweep.
pub const FIXTURE_FRAMES: u64 = 120;
/// Whole-texture FIFO capacity of the baseline (present.rs FIFO shape).
pub const BASELINE_TEX_CAP: usize = 16;

/// One paged tile: texture id plus tile index within the texture.
/// Tile 0 is the L0 base tile (pinned on first touch, never evicted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    /// Texture id (`0..FIXTURE_TEXTURES` in the fixture).
    pub tex: u32,
    /// Tile index within the texture (`0..FIXTURE_TILES_PER_TEX`).
    pub tile: u32,
}

impl TileKey {}

/// Demand-paged tile residency: LRU over non-L0 tiles (cap 192) plus a
/// pinned L0 per touched texture. Every first touch pins L0 first, so a
/// non-resident detail tile always has L0 cover: `misses_uncovered` stays 0
/// by construction and the receipt names covered vs uncovered misses.
#[derive(Debug)]
pub struct TileResidency {
    lru: HashMap<TileKey, ()>,
    order: VecDeque<TileKey>,
    pinned_l0: HashSet<u32>,
    uploads: u64,
    hits: u64,
    misses_covered: u64,
    misses_uncovered: u64,
    evicted: u64,
}

impl TileResidency {
    /// Empty residency: nothing pinned, nothing under LRU.
    pub fn new() -> Self {
        Self {
            lru: HashMap::new(),
            order: VecDeque::new(),
            pinned_l0: HashSet::new(),
            uploads: 0,
            hits: 0,
            misses_covered: 0,
            misses_uncovered: 0,
            evicted: 0,
        }
    }

    /// Request one tile. Returns true on a resident hit (pinned L0 or LRU
    /// resident). A non-resident detail tile uploads under LRU and counts a
    /// covered miss (L0 fallback covers the first touch); an L0 first touch
    /// pins and uploads without counting a miss.
    pub fn request(&mut self, tex: u32, tile: u32) -> bool {
        if tile == 0 {
            if self.pinned_l0.contains(&tex) {
                self.hits = self.hits.saturating_add(1);
                return true;
            }
            self.pinned_l0.insert(tex);
            self.uploads = self.uploads.saturating_add(1);
            return false;
        }
        let key = TileKey { tex, tile };
        if self.lru.contains_key(&key) {
            self.hits = self.hits.saturating_add(1);
            self.order.retain(|k| *k != key);
            self.order.push_back(key);
            return true;
        }
        // Detail miss: L0 cover holds iff this texture's L0 is pinned;
        // callers request L0 first per texture per frame, so cover holds.
        // A detail request ahead of its L0 is an uncovered miss (loud).
        if self.pinned_l0.contains(&tex) {
            self.misses_covered = self.misses_covered.saturating_add(1);
        } else {
            self.misses_uncovered = self.misses_uncovered.saturating_add(1);
        }
        self.uploads = self.uploads.saturating_add(1);
        self.order.push_back(key);
        self.lru.insert(key, ());
        if self.lru.len() > TILE_LRU_CAP {
            let oldest = match pop_oldest(&mut self.order, "TileResidency::request") {
                Ok(oldest) => oldest,
                Err(error) => {
                    // Unreachable: every LRU insert pushes one order entry.
                    // The miss is already counted, so report it as a miss.
                    debug_assert!(false, "{error}");
                    return false;
                }
            };
            self.lru.remove(&oldest);
            self.evicted = self.evicted.saturating_add(1);
        }
        false
    }

    /// Tile uploads issued so far (pinned L0 plus LRU detail uploads).
    pub fn uploads(&self) -> u64 {
        self.uploads
    }

    /// Resident hits so far.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Detail misses with L0 cover so far.
    pub fn misses_covered(&self) -> u64 {
        self.misses_covered
    }

    /// Detail misses with no L0 cover (must stay 0 on the streaming path).
    pub fn misses_uncovered(&self) -> u64 {
        self.misses_uncovered
    }

    /// LRU evictions so far (pinned L0 never evicts).
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Resident non-L0 tile count (always `<= TILE_LRU_CAP`).
    pub fn resident_tiles(&self) -> usize {
        self.lru.len()
    }

    /// Pinned L0 count (one per touched texture).
    pub fn pinned_l0_count(&self) -> usize {
        self.pinned_l0.len()
    }

    /// True when this texture's L0 is pinned.
    pub fn is_l0_pinned(&self, tex: u32) -> bool {
        self.pinned_l0.contains(&tex)
    }

    /// True when this non-L0 tile is LRU-resident (L0 answers via
    /// `is_l0_pinned`).
    pub fn is_resident(&self, key: TileKey) -> bool {
        self.lru.contains_key(&key)
    }

    /// FNV-1a 64 digest over the sorted resident tile set plus the sorted
    /// pinned L0 set. Stable x2 by construction (sorted before hashing).
    pub fn tile_digest(&self) -> u64 {
        let mut tiles: Vec<(u32, u32)> = self.lru.keys().map(|k| (k.tex, k.tile)).collect();
        tiles.sort_unstable();
        let mut pinned: Vec<u32> = self.pinned_l0.iter().copied().collect();
        pinned.sort_unstable();
        let mut h: u64 = FNV64_OFFSET;
        for (tex, tile) in tiles {
            h ^= u64::from(tex);
            h = h.wrapping_mul(FNV64_PRIME);
            h ^= u64::from(tile);
            h = h.wrapping_mul(FNV64_PRIME);
        }
        h ^= GOLDEN_GAMMA;
        h = h.wrapping_mul(FNV64_PRIME);
        for tex in pinned {
            h ^= u64::from(tex);
            h = h.wrapping_mul(FNV64_PRIME);
        }
        h
    }

    /// Bulk digest over residency counters plus the tile digest: one integer
    /// receipt for the whole sweep. Stable x2 (pure function of counters).
    pub fn bulk_digest(&self) -> u64 {
        let mut h: u64 = FNV64_OFFSET;
        for word in [
            self.uploads,
            self.hits,
            self.misses_covered,
            self.misses_uncovered,
            self.evicted,
            self.tile_digest(),
        ] {
            h ^= word;
            h = h.wrapping_mul(FNV64_PRIME);
        }
        h
    }
}

impl Default for TileResidency {
    fn default() -> Self {
        Self::new()
    }
}

/// Dense-fixture sweep receipt: baseline whole-texture FIFO against the
/// candidate paged residency over the same request stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SweepStats {
    /// Baseline tile uploads (whole-texture FIFO cap-16, 16 tiles per miss).
    pub baseline_uploads: u64,
    /// Baseline whole-texture misses.
    pub baseline_misses: u64,
    /// Candidate tile uploads (pinned L0 plus LRU detail).
    pub uploads: u64,
    /// Candidate resident hits.
    pub hits: u64,
    /// Candidate detail misses with L0 cover.
    pub misses_covered: u64,
    /// Candidate detail misses with no L0 cover (accept: 0).
    pub misses_uncovered: u64,
    /// Candidate tile digest (stable x2).
    pub tile_digest: u64,
    /// Candidate bulk digest (stable x2).
    pub bulk_digest: u64,
}

impl SweepStats {
    /// Upload cut in whole percent: `100 * (baseline - uploads) / baseline`.
    /// Accept bar: `>= 30` (spike posts 74).
    pub fn upload_cut_percent(&self) -> u64 {
        if self.baseline_uploads == 0 {
            return 0;
        }
        let saved = self.baseline_uploads.saturating_sub(self.uploads);
        saved.saturating_mul(100) / self.baseline_uploads
    }
}

/// Detail tile set for one texture in one frame: 4 consecutive non-L0 tiles
/// rotating with `(frame + tex)`, deterministic from integers only.
pub fn detail_tiles_for(frame: u64, tex: u32) -> [u32; FIXTURE_DETAIL_TILES as usize] {
    let scatter = FIXTURE_TILES_PER_TEX - 1;
    let base = ((frame as u32).wrapping_add(tex) % scatter) + 1;
    [
        base,
        (base % scatter) + 1,
        ((base + 1) % scatter) + 1,
        ((base + 2) % scatter) + 1,
    ]
}

/// Run the dense arena sweep: 64 textures x 16 tiles, 120 frames, an 8-tex
/// sliding window per frame, L0 plus 4 detail tiles per texture. The
/// baseline loads whole textures through a FIFO cap-16 (16 tile uploads per
/// miss); the candidate pages tiles through `TileResidency` (L0 first per
/// texture per frame, so every detail miss has L0 cover).
pub fn arena_sweep() -> SweepStats {
    let mut baseline: VecDeque<u32> = VecDeque::new();
    let mut baseline_resident = HashSet::new();
    let mut baseline_uploads: u64 = 0;
    let mut baseline_misses: u64 = 0;
    let mut paged = TileResidency::new();

    for frame in 0..FIXTURE_FRAMES {
        // Stride-7 slide: 7 new texs/frame over the 8-tex window (1-tex
        // overlap), the LADDER:528 spike shape (841 whole-tex misses).
        let start = ((frame * 7) % u64::from(FIXTURE_TEXTURES)) as u32;
        for w in 0..FIXTURE_WINDOW {
            let tex = (start + w) % FIXTURE_TEXTURES;
            // Baseline: whole-texture FIFO cap-16.
            if !baseline_resident.contains(&tex) {
                baseline_misses = baseline_misses.saturating_add(1);
                baseline_uploads =
                    baseline_uploads.saturating_add(u64::from(FIXTURE_TILES_PER_TEX));
                baseline.push_back(tex);
                baseline_resident.insert(tex);
                if baseline.len() > BASELINE_TEX_CAP {
                    let oldest = match pop_oldest(&mut baseline, "arena_sweep baseline") {
                        Ok(oldest) => oldest,
                        Err(error) => {
                            // Unreachable: every resident insert pushes one
                            // baseline entry. Skip the eviction, keep the miss.
                            debug_assert!(false, "{error}");
                            continue;
                        }
                    };
                    baseline_resident.remove(&oldest);
                }
            }
            // Candidate: L0 first (pins cover), then the 4 detail tiles.
            paged.request(tex, 0);
            for tile in detail_tiles_for(frame, tex) {
                paged.request(tex, tile);
            }
        }
    }

    SweepStats {
        baseline_uploads,
        baseline_misses,
        uploads: paged.uploads(),
        hits: paged.hits(),
        misses_covered: paged.misses_covered(),
        misses_uncovered: paged.misses_uncovered(),
        tile_digest: paged.tile_digest(),
        bulk_digest: paged.bulk_digest(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l0_pins_once_and_never_evicts() {
        let mut r = TileResidency::new();
        assert!(!r.request(7, 0));
        assert!(r.request(7, 0));
        assert_eq!((r.uploads(), r.pinned_l0_count()), (1, 1));
        // Flood the LRU far past cap: L0 stays pinned, never evicted.
        for i in 0..(TILE_LRU_CAP as u32 + 64) {
            r.request(7, 1 + (i % 15));
            r.request(1000 + i, 3);
        }
        assert!(r.is_l0_pinned(7));
        assert!(r.resident_tiles() <= TILE_LRU_CAP);
    }

    #[test]
    fn lru_refresh_beats_fifo_order() {
        let mut r = TileResidency::new();
        r.request(1, 0);
        let keeper = TileKey { tex: 1, tile: 5 };
        r.request(1, 5);
        // Fill to exactly cap with the keeper oldest: 1 + 191 = 192.
        for i in 0..(TILE_LRU_CAP as u32 - 1) {
            r.request(5000 + i, 0);
            r.request(5000 + i, 2);
        }
        assert_eq!(r.resident_tiles(), TILE_LRU_CAP);
        // Refresh the keeper so it is most-recent, then overflow by one:
        // a no-refresh FIFO would evict the keeper; LRU evicts filler.
        assert!(r.request(1, 5));
        r.request(6000, 0);
        r.request(6000, 2);
        assert!(
            r.is_resident(keeper),
            "refreshed tile must survive LRU overflow"
        );
        assert!(r.resident_tiles() <= TILE_LRU_CAP);
    }

    #[test]
    fn detail_ahead_of_l0_is_uncovered() {
        let mut r = TileResidency::new();
        r.request(9, 4);
        assert_eq!(r.misses_uncovered(), 1);
        r.request(9, 0);
        // (9,4) is LRU-resident now; a *new* detail tile posts the covered miss.
        r.request(9, 5);
        assert_eq!((r.misses_covered(), r.misses_uncovered()), (1, 1));
    }

    #[test]
    fn dense_arena_sweep_posts_upload_cut_with_zero_uncovered() {
        let t0 = std::time::Instant::now();
        let s = arena_sweep();
        let elapsed_ms = t0.elapsed().as_millis();
        // Per-pass receipt: measured wall ms, never faked (sweep is
        // 4800 requests; it must finish fast and the number is recorded).
        eprintln!(
            "OPT35 arena sweep: baseline={} uploads ({} misses) candidate={} uploads cut={}% uncovered={} tile_digest={:016x} bulk_digest={:016x} sweep_ms={}",
            s.baseline_uploads,
            s.baseline_misses,
            s.uploads,
            s.upload_cut_percent(),
            s.misses_uncovered,
            s.tile_digest,
            s.bulk_digest,
            elapsed_ms,
        );
        assert!(
            s.baseline_uploads > 0,
            "baseline must issue uploads, got {}",
            s.baseline_uploads
        );
        assert!(
            s.upload_cut_percent() >= 30,
            "upload cut {}% below the 30% bar (baseline={} candidate={})",
            s.upload_cut_percent(),
            s.baseline_uploads,
            s.uploads,
        );
        assert_eq!(
            s.misses_uncovered, 0,
            "every detail miss must carry L0 cover"
        );
        // Digests reproduce x2: same request stream, same bytes.
        let again = arena_sweep();
        assert_eq!(s.tile_digest, again.tile_digest, "tile digest drifted x2");
        assert_eq!(s.bulk_digest, again.bulk_digest, "bulk digest drifted x2");
        assert_eq!(s, again, "sweep receipt drifted x2");
    }

    #[test]
    fn residency_bounds_hold_after_sweep_stream() {
        let mut r = TileResidency::new();
        for frame in 0..FIXTURE_FRAMES {
            // Stride-7 slide: 7 new texs/frame over the 8-tex window (1-tex
            // overlap), the LADDER:528 spike shape (841 whole-tex misses).
            let start = ((frame * 7) % u64::from(FIXTURE_TEXTURES)) as u32;
            for w in 0..FIXTURE_WINDOW {
                let tex = (start + w) % FIXTURE_TEXTURES;
                r.request(tex, 0);
                for tile in detail_tiles_for(frame, tex) {
                    r.request(tex, tile);
                }
            }
        }
        assert!(r.resident_tiles() <= TILE_LRU_CAP);
        assert_eq!(
            r.pinned_l0_count(),
            FIXTURE_TEXTURES as usize,
            "120-frame sliding window touches every texture, all L0 pinned"
        );
        assert_eq!(r.misses_uncovered(), 0);
        assert_eq!(r.tile_digest(), r.tile_digest());
        assert_eq!(r.bulk_digest(), r.bulk_digest());
    }
}
