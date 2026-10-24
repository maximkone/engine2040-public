//! Asset thumbnails: one 128x128 RGBA8 image per content digest, LRU-bounded.
//!
//! Owner: the editor session. The browser panel asks for a thumbnail per listed
//! asset; the import cooks the pixels ([`engine_assets::thumb_cook`]) and the
//! browser inserts them here ([`ThumbnailCache::insert_cooked`]). This module
//! never invents a pixel: a row without an insert stays missing, so the browser
//! draws a placeholder instead of a fake image.
//! Keying: one entry per content digest, so two asset names whose pixels match
//! share one buffer; a name binds to a digest ([`ThumbnailCache::insert`]) and a
//! read follows the name.
//! Determinism: the digest is FNV-1a 64 over the RGBA bytes, which is the
//! digest the cook seals, and eviction takes the smallest caller-supplied use
//! tick (smallest digest on a tie), so the same inserts and reads always evict
//! the same entry. No clock, no RNG.
//! Budget: one thumbnail is [`THUMB_BYTES`] (64 KiB); the caller's byte cap
//! bounds the whole cache and eviction never leaves a stale name binding.
//! Errors: [`ThumbError`]; a refused insert changes nothing.

use std::collections::{BTreeMap, BTreeSet};

use engine_core::hash_bytes;

/// Thumbnail width and height in pixels: the budget the import cooks at
/// ([`engine_assets::thumb_cook::THUMB_EDGE`]); the size test below keeps the
/// two from drifting apart.
pub const THUMB_EDGE: usize = 128;
/// Bytes of one RGBA8 thumbnail (`128 * 128 * 4`).
pub const THUMB_BYTES: usize = THUMB_EDGE * THUMB_EDGE * 4;
/// Byte cap a caller gets when it names none: eight thumbnails.
pub const DEFAULT_CAP_BYTES: usize = 8 * THUMB_BYTES;
/// Longest asset name in chars: the browser's own bound.
pub const MAX_NAME_CHARS: usize = crate::browser::MAX_NAME_CHARS;

/// One cached thumbnail: the pixels plus their digest and use tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    /// Content digest: FNV-1a 64 ([`hash_bytes`]) over `rgba`, so the same
    /// bytes give the same digest on every run and every machine.
    pub digest: u64,
    /// Pixels, [`THUMB_BYTES`] long, rows top-down, RGBA8.
    pub rgba: Vec<u8>,
    /// Caller-supplied use tick of the last insert or read.
    pub last_use: u64,
}

/// Thumbnails keyed by content digest, evicted least-recently-used at a
/// caller byte cap.
#[derive(Clone, Debug)]
pub struct ThumbnailCache {
    cap_bytes: usize,
    used_bytes: usize,
    /// Asset name -> content digest.
    names: BTreeMap<String, u64>,
    /// One entry per digest, in insert order.
    entries: Vec<Thumbnail>,
    /// Asset names read before any insert bound them.
    missing: BTreeSet<String>,
    evicted: u64,
}

impl ThumbnailCache {
    /// Empty cache with a byte cap (use [`DEFAULT_CAP_BYTES`] when the caller
    /// has no budget of its own).
    pub fn new(cap_bytes: usize) -> Self {
        Self {
            cap_bytes,
            used_bytes: 0,
            names: BTreeMap::new(),
            entries: Vec::new(),
            missing: BTreeSet::new(),
            evicted: 0,
        }
    }

    /// Byte cap.
    pub fn cap_bytes(&self) -> usize {
        self.cap_bytes
    }

    /// Bytes held now by the kept thumbnails.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Kept thumbnail count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no thumbnail is kept.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many thumbnails eviction dropped so far.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Requested-but-absent asset names, sorted by name.
    ///
    /// A read of a name with no entry records it; a later insert of that name
    /// clears it again.
    pub fn missing(&self) -> Vec<&str> {
        self.missing.iter().map(String::as_str).collect()
    }

    /// Insert the pixels for `asset` and return the content digest.
    ///
    /// The buffer must be exactly [`THUMB_BYTES`] and the byte cap must hold at
    /// least one thumbnail. Inserting bytes the cache already holds only moves
    /// the entry's use tick forward, so the same image never costs twice.
    pub fn insert(&mut self, asset: &str, rgba: &[u8], now: u64) -> Result<u64, ThumbError> {
        let name = asset.trim();
        if name.is_empty() {
            return Err(ThumbError::EmptyName);
        }
        let name_len = name.chars().count();
        if name_len > MAX_NAME_CHARS {
            return Err(ThumbError::NameTooLong {
                len: name_len,
                max: MAX_NAME_CHARS,
            });
        }
        if rgba.len() != THUMB_BYTES {
            return Err(ThumbError::WrongSize {
                got: rgba.len(),
                expected: THUMB_BYTES,
            });
        }
        if self.cap_bytes < THUMB_BYTES {
            return Err(ThumbError::CapTooSmall {
                cap: self.cap_bytes,
                needed: THUMB_BYTES,
            });
        }
        let digest = hash_bytes(rgba);
        self.missing.remove(name);
        self.names.insert(name.to_string(), digest);
        if let Some(entry) = self.entries.iter_mut().find(|row| row.digest == digest) {
            entry.last_use = now;
            return Ok(digest);
        }
        self.entries.push(Thumbnail {
            digest,
            rgba: rgba.to_vec(),
            last_use: now,
        });
        self.used_bytes = self.used_bytes.saturating_add(THUMB_BYTES);
        self.evict_to_cap();
        Ok(digest)
    }

    /// Insert the pixels an import cooked ([`engine_assets::thumb_cook`]).
    ///
    /// This is the one seam between the assets crate's cook and this cache: the
    /// cooked digest is the digest of its own pixels, so inserting here binds
    /// the name to the digest the browser row reports.
    ///
    /// # Errors
    /// Same refusals as [`ThumbnailCache::insert`]: a cooked buffer that is not
    /// [`THUMB_BYTES`] long is refused and nothing is cached.
    pub fn insert_cooked(
        &mut self,
        asset: &str,
        thumb: &engine_assets::thumb_cook::CookedThumb,
        now: u64,
    ) -> Result<u64, ThumbError> {
        self.insert(asset, &thumb.rgba, now)
    }

    /// Read one asset's thumbnail, marking it used at `now`.
    ///
    /// A name with no entry returns `None` and is recorded in [`missing`].
    /// [`missing`]: ThumbnailCache::missing
    pub fn get(&mut self, asset: &str, now: u64) -> Option<&Thumbnail> {
        let name = asset.trim();
        if name.is_empty() {
            return None;
        }
        let Some(digest) = self.names.get(name).copied() else {
            self.missing.insert(name.to_string());
            return None;
        };
        let Some(index) = self.entries.iter().position(|row| row.digest == digest) else {
            // Eviction always drops the name bindings of the entry it removes,
            // so a binding without a row cannot happen; a corrupt one reads as
            // missing rather than as a stale image.
            self.missing.insert(name.to_string());
            return None;
        };
        self.entries[index].last_use = now;
        Some(&self.entries[index])
    }

    /// Drop least-recently-used entries until the byte cap holds.
    fn evict_to_cap(&mut self) {
        while self.used_bytes > self.cap_bytes {
            let Some(victim) = self
                .entries
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.last_use.cmp(&b.last_use).then(a.digest.cmp(&b.digest)))
                .map(|(index, _)| index)
            else {
                break;
            };
            let removed = self.entries.remove(victim);
            self.used_bytes = self.used_bytes.saturating_sub(removed.rgba.len());
            self.evicted = self.evicted.saturating_add(1);
            self.names.retain(|_, digest| *digest != removed.digest);
        }
    }
}

impl Default for ThumbnailCache {
    /// Cache at [`DEFAULT_CAP_BYTES`].
    fn default() -> Self {
        Self::new(DEFAULT_CAP_BYTES)
    }
}

/// What refused an insert. Nothing was cached on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbError {
    /// The pixel buffer was not one thumbnail long.
    WrongSize {
        /// Bytes given.
        got: usize,
        /// Bytes needed.
        expected: usize,
    },
    /// The asset name held no text.
    EmptyName,
    /// The asset name went over the char bound.
    NameTooLong {
        /// Chars given.
        len: usize,
        /// Chars allowed.
        max: usize,
    },
    /// The byte cap cannot hold one thumbnail.
    CapTooSmall {
        /// Cap given.
        cap: usize,
        /// Bytes one thumbnail needs.
        needed: usize,
    },
}

impl std::fmt::Display for ThumbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongSize { got, expected } => write!(
                f,
                "thumbnail holds {got} bytes; pass {expected} bytes (256x256 RGBA8)"
            ),
            Self::EmptyName => f.write_str("asset name holds no text; pass the bundle name"),
            Self::NameTooLong { len, max } => write!(
                f,
                "asset name holds {len} chars over the {max} bound; shorten it to {max} chars"
            ),
            Self::CapTooSmall { cap, needed } => write!(
                f,
                "thumbnail cap holds {cap} bytes under the {needed} one image needs; raise it to at least {needed}"
            ),
        }
    }
}

impl std::error::Error for ThumbError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// One thumbnail's pixels: `seed` in the first channel of every pixel, so
    /// two seeds differ and the same seed gives the same bytes.
    fn pixels(seed: u8) -> Vec<u8> {
        let mut rgba = vec![0u8; THUMB_BYTES];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[0] = seed;
            pixel[3] = 255;
        }
        rgba
    }

    #[test]
    fn three_inserts_at_two_thumb_cap_evict_the_oldest() {
        let mut cache = ThumbnailCache::new(2 * THUMB_BYTES);
        assert!(cache.is_empty());
        for (seed, now) in [(1u8, 10u64), (2, 20), (3, 30)] {
            cache
                .insert(&format!("asset{seed}"), &pixels(seed), now)
                .unwrap();
        }
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.used_bytes(), 2 * THUMB_BYTES);
        assert_eq!(cache.evicted(), 1);
        // The oldest use went first; its name reads as missing now.
        assert!(cache.get("asset1", 40).is_none());
        assert_eq!(cache.missing(), vec!["asset1"]);
        assert!(cache.get("asset2", 41).is_some());
        assert!(cache.get("asset3", 42).is_some());
        // A read moves the entry forward: the less recently used one leaves.
        assert!(cache.get("asset2", 41).is_some());
        assert!(cache.get("asset3", 42).is_some());
        cache.insert("asset4", &pixels(4), 50).unwrap();
        assert!(cache.get("asset3", 51).is_some(), "read at 42 outranks 41");
        assert!(cache.get("asset2", 52).is_none(), "last use 41 left");
    }

    #[test]
    fn same_bytes_give_the_same_digest_and_one_entry() {
        let first = hash_bytes(&pixels(9));
        assert_eq!(first, hash_bytes(&pixels(9)));
        assert_ne!(first, hash_bytes(&pixels(8)));

        let mut cache = ThumbnailCache::new(DEFAULT_CAP_BYTES);
        let a = cache.insert("wall", &pixels(9), 1).unwrap();
        let b = cache.insert("wall-copy", &pixels(9), 2).unwrap();
        assert_eq!(a, first);
        assert_eq!(b, first);
        assert_eq!(cache.len(), 1, "one entry per content digest");
        assert_eq!(cache.used_bytes(), THUMB_BYTES);
        let wall = cache.get("wall", 3).expect("bound name");
        assert_eq!(wall.digest, first);
        assert_eq!(wall.rgba, pixels(9));
        assert!(cache.get("wall-copy", 4).is_some());
    }

    #[test]
    fn missing_list_is_sorted_and_clears_on_insert() {
        let mut cache = ThumbnailCache::new(DEFAULT_CAP_BYTES);
        assert!(cache.get("zeta", 1).is_none());
        assert!(cache.get("alpha", 2).is_none());
        assert!(cache.get("zeta", 3).is_none(), "a repeat is kept once");
        assert_eq!(cache.missing(), vec!["alpha", "zeta"]);
        cache.insert("alpha", &pixels(5), 4).unwrap();
        assert_eq!(cache.missing(), vec!["zeta"]);
        assert!(cache.get("alpha", 5).is_some());
    }

    #[test]
    fn bad_inserts_refuse_and_change_nothing() {
        let mut cache = ThumbnailCache::new(DEFAULT_CAP_BYTES);
        assert_eq!(
            cache.insert("wall", &[0u8; 16], 1),
            Err(ThumbError::WrongSize {
                got: 16,
                expected: THUMB_BYTES
            })
        );
        assert_eq!(
            cache.insert("  ", &pixels(1), 1),
            Err(ThumbError::EmptyName)
        );
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.used_bytes(), 0);
        let long = "n".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(
            cache.insert(&long, &pixels(1), 1),
            Err(ThumbError::NameTooLong {
                len: MAX_NAME_CHARS + 1,
                max: MAX_NAME_CHARS
            })
        );
        let mut small = ThumbnailCache::new(THUMB_BYTES - 1);
        let error = small.insert("wall", &pixels(1), 1).unwrap_err();
        assert_eq!(
            error,
            ThumbError::CapTooSmall {
                cap: THUMB_BYTES - 1,
                needed: THUMB_BYTES
            }
        );
        assert!(error.to_string().contains("65536"), "{error}");
        assert!(small.is_empty());
    }

    // The import cooks at the size this cache holds: the two constants are one
    // budget, so a drift on either side fails here.
    #[test]
    fn the_cache_holds_exactly_the_size_the_import_cooks() {
        assert_eq!(
            THUMB_BYTES,
            engine_assets::thumb_cook::THUMB_BYTES,
            "cache and cook share one thumbnail budget"
        );
        assert_eq!(
            u32::try_from(THUMB_EDGE).unwrap(),
            engine_assets::thumb_cook::THUMB_EDGE
        );
    }

    // WG14: the cache reads the pixels the import cooked, keys them by the
    // cook's own digest, and a second insert of the same content costs nothing.
    #[test]
    fn the_cache_reads_the_import_cooked_thumbnail() {
        // One 4x4 source, so the cook upsamples it to the 128 px budget.
        let source = [90u8, 120, 60, 255].repeat(16);
        let cooked = engine_assets::thumb_cook::cook_thumbnail(&source, 4, 4).expect("cooks");
        let mut cache = ThumbnailCache::default();
        let digest = cache
            .insert_cooked("banner", &cooked, 1)
            .expect("the cooked size fits");
        assert_eq!(digest, cooked.digest, "the cook's digest is the cache key");
        let row = cache.get("banner", 2).expect("bound name");
        assert_eq!(row.rgba, cooked.rgba, "the cache holds the cooked pixels");
        assert_eq!(row.digest, cooked.digest);
        assert_eq!(cache.used_bytes(), engine_assets::thumb_cook::THUMB_BYTES);
        // The same content under another name is one entry, not two.
        assert_eq!(cache.insert_cooked("banner-copy", &cooked, 3), Ok(digest));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used_bytes(), engine_assets::thumb_cook::THUMB_BYTES);
        // A buffer that is not one thumbnail is refused, nothing cached.
        let mut wrong = cooked.clone();
        wrong.rgba.truncate(16);
        assert_eq!(
            cache.insert_cooked("crate", &wrong, 4),
            Err(ThumbError::WrongSize {
                got: 16,
                expected: THUMB_BYTES
            })
        );
        assert!(cache.get("crate", 5).is_none());
    }
}
