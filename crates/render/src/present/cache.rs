use std::collections::{BTreeMap, VecDeque};

use super::surface::PresentError;
use super::MAX_TEXTURE_DIM;

// ---------------------------------------------------------------------------
// Cache keys: stable identity plus content digest. `id` is the asset-registry
// identity (C3/C5: same file keeps its id across recooks); `digest` is the
// blake3 of the cooked bytes (a changed digest re-uploads). The consumer
// (AS1 bundle loader) maps `AssetId` plus hex digest into these keys; the
// proposed `render -> assets` edge would move that mapping next to the types.
// ---------------------------------------------------------------------------

/// GPU mesh key: stable asset id plus blake3 digest of the cooked mesh bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct MeshKey {
    /// Stable asset identity.
    pub id: u64,
    /// Blake3 digest of the cooked bytes that must be resident.
    pub digest: [u8; 32],
}

/// GPU texture key: stable asset id plus blake3 digest of the cooked bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct TextureKey {
    /// Stable asset identity.
    pub id: u64,
    /// Blake3 digest of the cooked bytes that must be resident.
    pub digest: [u8; 32],
}

fn parse_blake3_hex(hex: &str) -> Result<[u8; 32], PresentError> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return Err(PresentError::BadDigest(hex.to_string()));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in out.iter_mut().enumerate() {
        let (Some(hi), Some(lo)) = (nibble(bytes[2 * i]), nibble(bytes[2 * i + 1])) else {
            return Err(PresentError::BadDigest(hex.to_string()));
        };
        *chunk = (hi << 4) | lo;
    }
    Ok(out)
}

macro_rules! impl_asset_key {
    ($key:ty) => {
        impl $key {
            /// Build from a stable id plus a raw 32-byte blake3 digest.
            pub fn new(id: u64, digest: [u8; 32]) -> Self {
                Self { id, digest }
            }

            /// Build from a stable id plus a 64-char blake3 hex digest.
            pub fn from_hex(id: u64, hex: &str) -> Result<Self, PresentError> {
                Ok(Self {
                    id,
                    digest: parse_blake3_hex(hex)?,
                })
            }
        }
    };
}

impl_asset_key!(MeshKey);
impl_asset_key!(TextureKey);

// ---------------------------------------------------------------------------
// Bounded cache: insertion-order eviction, never map-order dependent.
// The GPU presenter stores buffers/textures behind this; the policy itself is
// std-only so the eviction order is unit-tested without a GPU. The map is a
// `BTreeMap` (sorted, no hashing) so the lib test target never instantiates
// `RandomState`/`DefaultHasher` for `MeshKey`/`TextureKey` and receipts stay
// deterministic on any host.
// ---------------------------------------------------------------------------

/// Bounded key store with insertion-order (FIFO) eviction. Reinserting a live
/// key replaces its value without disturbing the eviction order. Budget
/// counters (`inserted`, `evicted`, `high_water`, `last_evicted`) are kept
/// inline so overflow fixtures prove eviction without error and the receipt
/// names the evicted asset; they never change the eviction order.
pub struct CpuCache<K: Ord + Clone, V> {
    capacity: usize,
    map: BTreeMap<K, V>,
    order: VecDeque<K>,
    inserted: u64,
    evicted_total: u64,
    high_water: usize,
    last_evicted: Option<K>,
}

impl<K: Ord + Clone, V> CpuCache<K, V> {
    /// Build with a nonzero capacity; zero capacity keeps nothing.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            map: BTreeMap::new(),
            order: VecDeque::new(),
            inserted: 0,
            evicted_total: 0,
            high_water: 0,
            last_evicted: None,
        }
    }

    /// Insert, evicting the oldest key first when over capacity. Returns the
    /// evicted key, if any, so GPU owners can drop its buffers.
    pub fn insert(&mut self, key: K, value: V) -> Option<K> {
        use std::collections::btree_map::Entry;
        match self.map.entry(key) {
            Entry::Occupied(mut slot) => {
                slot.insert(value);
                None
            }
            Entry::Vacant(slot) => {
                if self.capacity == 0 {
                    let key = slot.into_key();
                    self.inserted = self.inserted.saturating_add(1);
                    self.evicted_total = self.evicted_total.saturating_add(1);
                    self.last_evicted = Some(key.clone());
                    return Some(key);
                }
                self.order.push_back(slot.key().clone());
                slot.insert(value);
                self.inserted = self.inserted.saturating_add(1);
                if self.map.len() > self.capacity {
                    let oldest = self
                        .order
                        .pop_front()
                        .expect("order tracks map inserts one-to-one");
                    self.map.remove(&oldest);
                    self.evicted_total = self.evicted_total.saturating_add(1);
                    self.last_evicted = Some(oldest.clone());
                    self.high_water = self.high_water.max(self.map.len());
                    return Some(oldest);
                }
                self.high_water = self.high_water.max(self.map.len());
                None
            }
        }
    }

    /// Look up a resident value.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    /// Drop one key; true when it was resident. Manual drops count toward
    /// the eviction telemetry so the receipt names the asset either way.
    pub fn evict(&mut self, key: &K) -> bool {
        if self.map.remove(key).is_none() {
            return false;
        }
        self.order.retain(|k| k != key);
        self.evicted_total = self.evicted_total.saturating_add(1);
        self.last_evicted = Some(key.clone());
        true
    }

    /// Resident entry count.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when nothing is resident.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Budget capacity (the `MAX_CACHED_*` bound backing this cache).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// New keys accepted so far (re-inserts of live keys do not count).
    pub fn inserted(&self) -> u64 {
        self.inserted
    }

    /// Evictions so far, capacity plus manual drops.
    pub fn evicted(&self) -> u64 {
        self.evicted_total
    }

    /// Most entries ever resident at once.
    pub fn high_water(&self) -> usize {
        self.high_water
    }

    /// Most recently evicted key, if any (names the asset for telemetry).
    pub fn last_evicted(&self) -> Option<&K> {
        self.last_evicted.as_ref()
    }
}

impl<K: Ord + Clone, V> CpuCache<K, V> {
    /// Resident keys in sorted order (deterministic receipts).
    pub fn sorted_keys(&self) -> Vec<K> {
        let mut keys: Vec<K> = self.map.keys().cloned().collect();
        keys.sort();
        keys
    }
}

/// Loud refusal for a texture key with no resident upload: names the asset
/// id so the content gate fails with the culprit, never silent magenta.
/// Shared by the headless `check_texture_resident` gate and the GPU
/// `require_texture_view` ship path.
pub fn missing_texture_error(id: u64) -> PresentError {
    PresentError::MissingTexture {
        id,
        reason: format!("texture {id} not resident"),
    }
}

/// Headless content-gate check: `Ok` while `key` is in `resident`, else the
/// loud `MissingTexture` refusal. The GPU ship path decides the same way
/// over its resident map.
pub fn check_texture_resident(
    resident: &[TextureKey],
    key: &TextureKey,
) -> Result<(), PresentError> {
    if resident.contains(key) {
        return Ok(());
    }
    Err(missing_texture_error(key.id))
}

// ---------------------------------------------------------------------------
// Texture upload: validated RGBA8 pixels. The forward PBR shader has no
// texture slot yet, so resident textures are held for the RL2 material bind
// group; `texture_keys` reports residency for receipts.
// ---------------------------------------------------------------------------

/// Validated RGBA8 texture upload in pixels.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextureUpload {
    /// Width in pixels, `1..=MAX_TEXTURE_DIM`.
    pub width: u32,
    /// Height in pixels, `1..=MAX_TEXTURE_DIM`.
    pub height: u32,
    /// Exactly `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl TextureUpload {
    /// Decode encoded image bytes (PNG) into a validated RGBA8 upload.
    /// Undecodable bytes or out-of-range sizes fail with `InvalidTexture`.
    /// Cold load path only (one decode per texture).
    #[cfg(feature = "v1")]
    pub fn from_encoded(bytes: &[u8]) -> Result<Self, PresentError> {
        let image = image::load_from_memory(bytes)
            .map_err(|err| PresentError::InvalidTexture(format!("decode: {err}")))?;
        let rgba = image.to_rgba8();
        let upload = Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        };
        upload.validate()?;
        Ok(upload)
    }

    /// Box-filtered mip levels below the base, largest first, down to 1x1
    /// (odd edges clamp). Averages the stored sRGB bytes directly: a
    /// cheap, deterministic chain that stops distant texels shimmering.
    /// Cold upload path only.
    pub fn mip_chain(&self) -> Vec<(u32, u32, Vec<u8>)> {
        let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        let (mut w, mut h) = (
            usize::try_from(self.width).unwrap_or(1),
            usize::try_from(self.height).unwrap_or(1),
        );
        while w > 1 || h > 1 {
            let src: &[u8] = levels.last().map_or(&self.rgba, |level| &level.2);
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut out = vec![0u8; nw * nh * 4];
            for y in 0..nh {
                for x in 0..nw {
                    let (x0, y0) = ((2 * x).min(w - 1), (2 * y).min(h - 1));
                    let (x1, y1) = ((2 * x + 1).min(w - 1), (2 * y + 1).min(h - 1));
                    for c in 0..4 {
                        let sum = u32::from(src[(y0 * w + x0) * 4 + c])
                            + u32::from(src[(y0 * w + x1) * 4 + c])
                            + u32::from(src[(y1 * w + x0) * 4 + c])
                            + u32::from(src[(y1 * w + x1) * 4 + c]);
                        out[(y * nw + x) * 4 + c] = u8::try_from((sum + 2) / 4).unwrap_or(u8::MAX);
                    }
                }
            }
            let lw = u32::try_from(nw).unwrap_or(1);
            let lh = u32::try_from(nh).unwrap_or(1);
            levels.push((lw, lh, out));
            w = nw;
            h = nh;
        }
        levels
    }

    /// The largest mip level whose longest edge is at most `max_edge`, as an upload of its own
    /// (the tier's `texture_max_px`: a bigger texture loads a smaller mip). `None` when the
    /// texture already fits, or when `max_edge` is zero. Cold upload path only.
    pub fn capped_to(&self, max_edge: u32) -> Option<TextureUpload> {
        if max_edge == 0 || self.width.max(self.height) <= max_edge {
            return None;
        }
        self.mip_chain()
            .into_iter()
            .find(|(w, h, _)| (*w).max(*h) <= max_edge)
            .map(|(width, height, rgba)| TextureUpload {
                width,
                height,
                rgba,
            })
    }

    /// Validate dimensions and byte length before any GPU upload.
    pub fn validate(&self) -> Result<(), PresentError> {
        if self.width == 0 || self.height == 0 {
            return Err(PresentError::InvalidTexture(format!(
                "zero area {}x{}",
                self.width, self.height
            )));
        }
        if self.width > MAX_TEXTURE_DIM || self.height > MAX_TEXTURE_DIM {
            return Err(PresentError::InvalidTexture(format!(
                "size {}x{} exceeds {MAX_TEXTURE_DIM}",
                self.width, self.height
            )));
        }
        let want = (self.width as usize) * (self.height as usize) * 4;
        if self.rgba.len() != want {
            return Err(PresentError::InvalidTexture(format!(
                "byte length {} differs from {}x{} RGBA8 {want}",
                self.rgba.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }
}
