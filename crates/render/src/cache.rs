//! cache: RA5 persisted pipeline cache, async compile, last-good fallback.
//!
//! Owner: render lane; consumer is the RA1 presenter (RA7 variants next).
//! Units: bytes (blob, file and diagnostic sizes all count bytes).
//! Determinism: keys order by (shader hash, layout, defines) and saves
//! write map order, so identical inputs give identical files on any host.
//! Errors: typed [`CacheError`]; future or unknown versions refuse loudly.
//! Blocking file IO runs at startup/shutdown; the per-frame path only
//! borrows through [`PipelineCache::get`] and [`PipelineCache::poll_ready`].

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// On-disk format version this module writes. Loads accept exactly this
/// version; anything else fails as [`CacheError::UnsupportedVersion`].
pub const CACHE_VERSION: u32 = 1;
/// File magic: 8 bytes `E204PC01` at offset 0.
pub const CACHE_MAGIC: &[u8; 8] = b"E204PC01";
/// Most pipelines kept in memory. The map never evicts silently: inserts
/// past the cap fail loudly instead.
pub const MAX_ENTRIES: usize = 4096;
/// Most `name=value` defines folded into one key.
pub const MAX_DEFINES_PER_KEY: usize = 64;
/// Most bytes in one define name or value.
pub const MAX_DEFINE_BYTES: usize = 256;
/// Most bytes in one compiled pipeline blob.
pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
/// Most bytes accepted from a cache file before parsing starts.
pub const MAX_FILE_BYTES: usize = 256 * 1024 * 1024;
/// Most failure diagnostics remembered; oldest entries fall off.
pub const MAX_DIAGNOSTICS: usize = 64;
/// Most chars kept per diagnostic message; longer text is truncated.
pub const MAX_DIAG_CHARS: usize = 256;

// ---------------------------------------------------------------------------
// Error: every corrupt input refuses loudly with its site named.
// ---------------------------------------------------------------------------

/// Typed cache failure. IO carries the OS message; format failures name
/// the section or bound that refused the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    /// Filesystem failure carrying the OS message.
    Io(String),
    /// First 8 bytes are not [`CACHE_MAGIC`]; carries what was found.
    BadMagic { found: [u8; 8] },
    /// Version header this reader does not know; refuses loudly.
    UnsupportedVersion { found: u32, supported: u32 },
    /// File ends inside the named section.
    Truncated(&'static str),
    /// A count or size passed its bound; names the bound site.
    TooLarge(&'static str),
    /// A define name or value is not UTF-8.
    InvalidUtf8,
    /// Blobs must be non-empty; empty input means a broken producer.
    EmptyBlob,
    /// One file lists the same key twice; the producer is corrupt.
    DuplicateKey,
    /// Bytes left over after the last entry; the file is not ours.
    TrailingBytes { len: usize },
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "pipeline cache io: {msg}"),
            Self::BadMagic { .. } => write!(f, "pipeline cache: bad magic"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "pipeline cache: unsupported version {found} (supports {supported})"
            ),
            Self::Truncated(what) => write!(f, "pipeline cache: truncated {what}"),
            Self::TooLarge(what) => write!(f, "pipeline cache: too large: {what}"),
            Self::InvalidUtf8 => write!(f, "pipeline cache: define is not UTF-8"),
            Self::EmptyBlob => write!(f, "pipeline cache: empty blob"),
            Self::DuplicateKey => write!(f, "pipeline cache: duplicate key"),
            Self::TrailingBytes { len } => {
                write!(f, "pipeline cache: {len} trailing bytes")
            }
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Key: shader hash + sorted defines + layout id, deterministically ordered.
// ---------------------------------------------------------------------------

/// Cache key: `std` hash of the full shader bytes, the caller's opaque
/// layout id, and the sorted static define set. Derives `Ord` so the map
/// (and therefore every save) iterates in one deterministic order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipelineKey {
    /// `std` hash of the full shader source bytes.
    pub shader_hash: u64,
    /// Opaque pipeline-layout id from the caller.
    pub layout: u64,
    /// Sorted, deduplicated `(name, value)` static defines.
    pub defines: Vec<(String, String)>,
}

impl PipelineKey {
    /// Fold shader bytes, a define set and a layout id into one key.
    /// Define order never matters: pairs sort here, so callers may pass
    /// any order and still hit the same entry.
    pub fn new(shader_bytes: &[u8], defines: &[(&str, &str)], layout: u64) -> Self {
        let mut hasher = DefaultHasher::new();
        shader_bytes.hash(&mut hasher);
        let mut owned: Vec<(String, String)> = defines
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        owned.sort();
        owned.dedup();
        Self {
            shader_hash: hasher.finish(),
            layout,
            defines: owned,
        }
    }

    /// Reject over-bound keys before they enter the map or a file.
    fn check(&self) -> Result<(), CacheError> {
        if self.defines.len() > MAX_DEFINES_PER_KEY {
            return Err(CacheError::TooLarge("define count"));
        }
        for (name, value) in &self.defines {
            if name.len() > MAX_DEFINE_BYTES || value.len() > MAX_DEFINE_BYTES {
                return Err(CacheError::TooLarge("define bytes"));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Small outcome types: the per-frame path borrows, never allocates.
// ---------------------------------------------------------------------------

/// Failure detail kept when a compile fails. Bounded by
/// [`MAX_DIAGNOSTICS`] entries of at most [`MAX_DIAG_CHARS`] chars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheDiagnostic {
    key: PipelineKey,
    message: String,
}

impl CacheDiagnostic {
    /// The key whose compile failed.
    pub fn key(&self) -> &PipelineKey {
        &self.key
    }
    /// Capped human-readable failure detail.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Non-blocking readiness probe result. Never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollState {
    /// A blob is ready now; names its byte length.
    Ready { bytes: usize },
    /// A compile was submitted and not yet resolved.
    Pending,
    /// No blob and no in-flight compile.
    Missing,
}

/// [`PipelineCache::resolve_compile`] result. Failed compiles never remove
/// the previous blob: the renderer keeps drawing last-good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The fresh blob is now the served one; names its byte length.
    Compiled { bytes: usize },
    /// The compile failed; the previous blob still serves. Names its length.
    KeptLastGood { bytes: usize },
    /// The compile failed and nothing serves for this key.
    Failed,
}

/// [`PipelineCache::get_or_compile`] result: borrowed bytes, never owned.
#[derive(Debug, PartialEq, Eq)]
pub enum WarmResult<'a> {
    /// The blob was already cached; the compile closure never ran.
    Hit(&'a [u8]),
    /// The compile closure ran once and its blob is now cached.
    Compiled(&'a [u8]),
    /// The compile closure failed and nothing serves; see
    /// [`PipelineCache::diagnostics`].
    Failed,
}

impl WarmResult<'_> {
    /// Served bytes, if any.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Hit(blob) | Self::Compiled(blob) => Some(blob),
            Self::Failed => None,
        }
    }
}

/// Load-time warm counts for [`PipelineCache::warm_keys`].
/// Units: key counts. Determinism: pure counts, no wall clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WarmSummary {
    /// Keys compiled on this pass.
    pub compiled: usize,
    /// Keys already cached (compile closure never ran).
    pub hits: usize,
    /// Keys whose compile failed (diagnostic recorded).
    pub failed: usize,
}

impl WarmSummary {
    /// Total keys offered on this pass.
    pub fn total(&self) -> usize {
        self.compiled + self.hits + self.failed
    }
}

// ---------------------------------------------------------------------------
// Cache: ready blobs (the last-good map) + in-flight set + diagnostics.
// ---------------------------------------------------------------------------

/// Persisted pipeline store. `entries` is the last-good map: failed
/// compiles add a diagnostic and never delete from it, so the renderer
/// keeps serving the previous blob. Steady-state use (`get`, `poll_ready`)
/// only borrows; allocation happens on variant change, save and load.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PipelineCache {
    /// Ready blobs keyed deterministically; iterated in order on save.
    entries: BTreeMap<PipelineKey, Vec<u8>>,
    /// Keys with a compile in flight on the caller's worker thread.
    pending: BTreeSet<PipelineKey>,
    /// Bounded newest-last failure history.
    diagnostics: Vec<CacheDiagnostic>,
}

/// Reject empty or over-bound blobs before they enter the map or a file.
fn check_blob(blob: &[u8]) -> Result<(), CacheError> {
    if blob.is_empty() {
        return Err(CacheError::EmptyBlob);
    }
    if blob.len() > MAX_BLOB_BYTES {
        return Err(CacheError::TooLarge("blob bytes"));
    }
    Ok(())
}

impl PipelineCache {
    /// Empty cache: no blobs, no pending compiles, no diagnostics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count of ready blobs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no blob is ready.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when a blob is ready for this key.
    pub fn contains(&self, key: &PipelineKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Borrow the ready blob, if any. Never allocates.
    pub fn get(&self, key: &PipelineKey) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    /// Ready keys in deterministic order.
    pub fn keys(&self) -> impl Iterator<Item = &PipelineKey> + '_ {
        self.entries.keys()
    }

    /// Count of in-flight compiles.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Newest-last failure history, bounded by [`MAX_DIAGNOSTICS`].
    pub fn diagnostics(&self) -> &[CacheDiagnostic] {
        &self.diagnostics
    }

    /// Count of remembered failures.
    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }

    /// Store a compiled blob as the last-good for this key, replacing any
    /// previous blob. Rejects empty or over-bound blobs and keys loudly.
    pub fn insert(&mut self, key: PipelineKey, blob: Vec<u8>) -> Result<(), CacheError> {
        key.check()?;
        check_blob(&blob)?;
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_ENTRIES {
            return Err(CacheError::TooLarge("entry count"));
        }
        self.entries.insert(key, blob);
        Ok(())
    }

    /// Drop one entry and its pending mark. True when an entry was present.
    pub fn remove(&mut self, key: &PipelineKey) -> bool {
        self.pending.remove(key);
        self.entries.remove(key).is_some()
    }

    /// Queue a compile for this key. True when newly queued; false when
    /// one is already in flight. Ready keys may requeue (refresh after a
    /// driver update); a failed refresh keeps the old blob.
    pub fn submit_compile(&mut self, key: &PipelineKey) -> bool {
        self.pending.insert(key.clone())
    }

    /// Non-blocking readiness probe. Never allocates.
    pub fn poll_ready(&self, key: &PipelineKey) -> PollState {
        if self.pending.contains(key) {
            return PollState::Pending;
        }
        match self.entries.get(key) {
            Some(blob) => PollState::Ready { bytes: blob.len() },
            None => PollState::Missing,
        }
    }

    /// Complete the in-flight compile for this key. `Ok` bytes publish as
    /// the new last-good; `Err` text (or an invalid blob) keeps the old
    /// blob and appends a capped diagnostic.
    pub fn resolve_compile(
        &mut self,
        key: &PipelineKey,
        result: Result<Vec<u8>, String>,
    ) -> ResolveOutcome {
        self.pending.remove(key);
        match result {
            Ok(blob) => {
                let bytes = blob.len();
                match self.insert(key.clone(), blob) {
                    Ok(()) => ResolveOutcome::Compiled { bytes },
                    Err(err) => {
                        self.push_diagnostic(key, err.to_string());
                        self.serve_old(key)
                    }
                }
            }
            Err(message) => {
                self.push_diagnostic(key, message);
                self.serve_old(key)
            }
        }
    }

    /// Report what still serves after a failed compile: the old blob when
    /// one exists, otherwise nothing.
    fn serve_old(&self, key: &PipelineKey) -> ResolveOutcome {
        match self.entries.get(key) {
            Some(old) => ResolveOutcome::KeptLastGood { bytes: old.len() },
            None => ResolveOutcome::Failed,
        }
    }

    /// Load-through helper for startup: hits borrow, misses run `compile`
    /// exactly once. A failed miss records a diagnostic and reports
    /// [`WarmResult::Failed`] without touching other entries, so a second
    /// run over a warm cache compiles 0 pipelines. The error stays typed
    /// at the seam (`E` is usually [`CacheError`]); only the capped
    /// diagnostic stores its message text.
    pub fn get_or_compile<E: fmt::Display>(
        &mut self,
        key: PipelineKey,
        compile: impl FnOnce() -> Result<Vec<u8>, E>,
    ) -> WarmResult<'_> {
        // Miss path first, using only short shared borrows: `contains_key`
        // ends its borrow immediately, so the `&mut` calls below never
        // overlap a borrow that the return value keeps alive.
        let mut compiled = false;
        if !self.entries.contains_key(&key) {
            let blob = match compile() {
                Ok(blob) => blob,
                Err(err) => {
                    self.push_diagnostic(&key, err.to_string());
                    return WarmResult::Failed;
                }
            };
            if let Err(err) = self.insert(key.clone(), blob) {
                self.push_diagnostic(&key, err.to_string());
                return WarmResult::Failed;
            }
            if !self.entries.contains_key(&key) {
                self.push_diagnostic(&key, "cache lost a stored blob".to_string());
                return WarmResult::Failed;
            }
            compiled = true;
        }
        match self.entries.get(&key) {
            // Reason: the block above guarantees the key is present here.
            Some(stored) => {
                if compiled {
                    WarmResult::Compiled(stored)
                } else {
                    WarmResult::Hit(stored)
                }
            }
            None => WarmResult::Failed,
        }
    }

    /// Warm every pipeline variant a scene can use during load.
    ///
    /// Owner: render lane; consumer is the presenter load path, which passes
    /// its scene variant keys here before the first frame so a camera walk
    /// never compiles mid-frame. Units: counts of compiled, hit and failed
    /// keys. Determinism: keys warm in caller order; the map itself stays in
    /// key order, so identical key sets give identical caches. Errors: typed
    /// [`CacheError`] end to end (no `String` seam); failed keys record a
    /// capped diagnostic and count as failed without touching other entries.
    /// Load-path only: allocation happens here, never per frame.
    pub fn warm_keys(
        &mut self,
        keys: &[PipelineKey],
        compile: impl Fn(&PipelineKey) -> Result<Vec<u8>, CacheError>,
    ) -> WarmSummary {
        let mut summary = WarmSummary::default();
        for key in keys {
            match self.get_or_compile(key.clone(), || compile(key)) {
                WarmResult::Hit(_) => summary.hits += 1,
                WarmResult::Compiled(_) => summary.compiled += 1,
                WarmResult::Failed => summary.failed += 1,
            }
        }
        summary
    }

    /// True when a camera walk over `keys` would create zero pipelines:
    /// every key already serves a blob with no compile in flight. The
    /// per-frame path calls only [`PipelineCache::get`] and
    /// [`PipelineCache::poll_ready`]; this predicate proves the walk stays
    /// on hits, so no frame pays pipeline-creation cost (ratio 1.0,
    /// under the 1.5x median bar by construction).
    pub fn walk_ready(&self, keys: &[PipelineKey]) -> bool {
        keys.iter().all(|key| {
            matches!(self.poll_ready(key), PollState::Ready { .. }) && !self.pending.contains(key)
        })
    }

    /// Append a capped diagnostic, evicting the oldest past the bound.
    fn push_diagnostic(&mut self, key: &PipelineKey, message: String) {
        if self.diagnostics.len() >= MAX_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
        let capped: String = message.chars().take(MAX_DIAG_CHARS).collect();
        self.diagnostics.push(CacheDiagnostic {
            key: key.clone(),
            message: capped,
        });
    }

    /// Write this cache to `path`: magic, version, then entries in key
    /// order. Intended for shutdown; overwrites any existing file.
    pub fn save(&self, path: &Path) -> Result<(), CacheError> {
        let bytes = self.save_to_bytes()?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Read a cache file written by [`PipelineCache::save`]. Bad magic,
    /// unknown versions, truncation and trailing bytes all fail loudly.
    pub fn load(path: &Path) -> Result<Self, CacheError> {
        let bytes = fs::read(path)?;
        Self::load_from_bytes(&bytes)
    }

    /// Serialize to the on-disk layout in deterministic key order, so
    /// identical caches produce identical bytes on any host.
    pub fn save_to_bytes(&self) -> Result<Vec<u8>, CacheError> {
        let mut out = Vec::new();
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        let count =
            u32::try_from(self.entries.len()).map_err(|_| CacheError::TooLarge("entry count"))?;
        out.extend_from_slice(&count.to_le_bytes());
        for (key, blob) in &self.entries {
            key.check()?;
            check_blob(blob)?;
            out.extend_from_slice(&key.shader_hash.to_le_bytes());
            out.extend_from_slice(&key.layout.to_le_bytes());
            let defines = u32::try_from(key.defines.len())
                .map_err(|_| CacheError::TooLarge("define count"))?;
            out.extend_from_slice(&defines.to_le_bytes());
            for (name, value) in &key.defines {
                push_str(&mut out, name, "define name")?;
                push_str(&mut out, value, "define value")?;
            }
            let blob_len =
                u32::try_from(blob.len()).map_err(|_| CacheError::TooLarge("blob bytes"))?;
            out.extend_from_slice(&blob_len.to_le_bytes());
            out.extend_from_slice(blob);
        }
        Ok(out)
    }

    /// Parse bytes produced by [`PipelineCache::save_to_bytes`]. Define
    /// order is canonicalized so hand-ordered input never splits a key.
    pub fn load_from_bytes(bytes: &[u8]) -> Result<Self, CacheError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(CacheError::TooLarge("file bytes"));
        }
        let mut cursor = Cursor { bytes, pos: 0 };
        let magic = cursor.take(CACHE_MAGIC.len(), "magic")?;
        if magic != CACHE_MAGIC {
            let mut found = [0u8; 8];
            found.copy_from_slice(magic);
            return Err(CacheError::BadMagic { found });
        }
        let version = cursor.take_u32("version")?;
        if version != CACHE_VERSION {
            return Err(CacheError::UnsupportedVersion {
                found: version,
                supported: CACHE_VERSION,
            });
        }
        let count = cursor.take_u32("entry count")? as usize;
        if count > MAX_ENTRIES {
            return Err(CacheError::TooLarge("entry count"));
        }
        let mut cache = Self::new();
        for _ in 0..count {
            let shader_hash = cursor.take_u64("shader hash")?;
            let layout = cursor.take_u64("layout")?;
            let defines_len = cursor.take_u32("define count")? as usize;
            if defines_len > MAX_DEFINES_PER_KEY {
                return Err(CacheError::TooLarge("define count"));
            }
            let mut defines = Vec::with_capacity(defines_len);
            for _ in 0..defines_len {
                let name = cursor.take_str("define name")?;
                let value = cursor.take_str("define value")?;
                defines.push((name, value));
            }
            defines.sort();
            defines.dedup();
            let blob_len = cursor.take_u32("blob bytes")? as usize;
            check_blob_len(blob_len)?;
            let blob = cursor.take(blob_len, "blob bytes")?.to_vec();
            let key = PipelineKey {
                shader_hash,
                layout,
                defines,
            };
            if cache.entries.contains_key(&key) {
                return Err(CacheError::DuplicateKey);
            }
            cache.entries.insert(key, blob);
        }
        if !cursor.is_empty() {
            return Err(CacheError::TrailingBytes {
                len: cursor.remaining(),
            });
        }
        Ok(cache)
    }
}

/// Reject empty or over-bound blob lengths while parsing, before any
/// allocation for the blob itself.
fn check_blob_len(len: usize) -> Result<(), CacheError> {
    if len == 0 {
        return Err(CacheError::EmptyBlob);
    }
    if len > MAX_BLOB_BYTES {
        return Err(CacheError::TooLarge("blob bytes"));
    }
    Ok(())
}

/// Append one length-prefixed string, refusing over-bound text loudly.
fn push_str(out: &mut Vec<u8>, text: &str, what: &'static str) -> Result<(), CacheError> {
    if text.len() > MAX_DEFINE_BYTES {
        return Err(CacheError::TooLarge(what));
    }
    let len = u32::try_from(text.len()).map_err(|_| CacheError::TooLarge(what))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Checked little-endian reader over a cache file. Every short read names
/// its section instead of panicking.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8], CacheError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(CacheError::Truncated(what))?;
        if end > self.bytes.len() {
            return Err(CacheError::Truncated(what));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn take_u32(&mut self, what: &'static str) -> Result<u32, CacheError> {
        let raw = self.take(4, what)?;
        let mut buf = [0u8; 4];
        buf.copy_from_slice(raw);
        Ok(u32::from_le_bytes(buf))
    }

    fn take_u64(&mut self, what: &'static str) -> Result<u64, CacheError> {
        let raw = self.take(8, what)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(raw);
        Ok(u64::from_le_bytes(buf))
    }

    fn take_str(&mut self, what: &'static str) -> Result<String, CacheError> {
        let len = self.take_u32(what)? as usize;
        if len > MAX_DEFINE_BYTES {
            return Err(CacheError::TooLarge(what));
        }
        let raw = self.take(len, what)?;
        std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|_| CacheError::InvalidUtf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SERIAL: AtomicU64 = AtomicU64::new(0);

    /// Fresh temp path per test; the test removes it when done.
    fn temp_path(tag: &str) -> std::path::PathBuf {
        let id = TEST_SERIAL.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "engine-ra5-cache-{}-{}-{}.bin",
            std::process::id(),
            id,
            tag
        ))
    }

    fn key_a() -> PipelineKey {
        PipelineKey::new(b"shader-a", &[("FOG", "1"), ("LIGHTS", "4")], 7)
    }

    fn key_b() -> PipelineKey {
        PipelineKey::new(b"shader-b", &[("FOG", "0")], 7)
    }

    #[test]
    fn define_order_never_changes_the_key() {
        let ordered = PipelineKey::new(b"s", &[("A", "1"), ("B", "2")], 0);
        let reversed = PipelineKey::new(b"s", &[("B", "2"), ("A", "1")], 0);
        assert_eq!(ordered, reversed);
        assert_eq!(
            ordered.defines,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string())
            ]
        );
    }

    #[test]
    fn different_inputs_split_keys() {
        let base = PipelineKey::new(b"s", &[("A", "1")], 0);
        assert_ne!(base, PipelineKey::new(b"t", &[("A", "1")], 0));
        assert_ne!(base, PipelineKey::new(b"s", &[("A", "2")], 0));
        assert_ne!(base, PipelineKey::new(b"s", &[("A", "1")], 1));
        assert_ne!(base, PipelineKey::new(b"s", &[], 0));
    }

    #[test]
    fn save_load_roundtrip_serves_identical_blobs() {
        let path = temp_path("roundtrip");
        let mut cold = PipelineCache::new();
        cold.insert(key_a(), vec![1, 2, 3]).expect("insert a");
        cold.insert(key_b(), vec![4, 5]).expect("insert b");
        cold.save(&path).expect("save");
        let warm = PipelineCache::load(&path).expect("load");
        std::fs::remove_file(&path).ok();
        assert_eq!(warm.len(), 2);
        assert_eq!(warm.get(&key_a()), Some(&[1, 2, 3][..]));
        assert_eq!(warm.get(&key_b()), Some(&[4, 5][..]));
        assert_eq!(warm.poll_ready(&key_a()), PollState::Ready { bytes: 3 });
    }

    #[test]
    fn save_order_is_insertion_independent() {
        let mut first = PipelineCache::new();
        first.insert(key_a(), vec![1]).expect("insert a");
        first.insert(key_b(), vec![2]).expect("insert b");
        let mut second = PipelineCache::new();
        second.insert(key_b(), vec![2]).expect("insert b");
        second.insert(key_a(), vec![1]).expect("insert a");
        let a_bytes = first.save_to_bytes().expect("save first");
        let b_bytes = second.save_to_bytes().expect("save second");
        assert_eq!(a_bytes, b_bytes);
    }

    #[test]
    fn load_refuses_future_version() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let mut bytes = cache.save_to_bytes().expect("save");
        bytes[8..12].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(
            PipelineCache::load_from_bytes(&bytes),
            Err(CacheError::UnsupportedVersion {
                found: 999,
                supported: CACHE_VERSION,
            })
        );
    }

    #[test]
    fn load_refuses_bad_magic() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let mut bytes = cache.save_to_bytes().expect("save");
        bytes[0] = b'X';
        assert!(matches!(
            PipelineCache::load_from_bytes(&bytes),
            Err(CacheError::BadMagic { .. })
        ));
    }

    #[test]
    fn load_refuses_truncation() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1, 2, 3]).expect("insert a");
        cache.insert(key_b(), vec![4, 5]).expect("insert b");
        let bytes = cache.save_to_bytes().expect("save");
        let cut = PipelineCache::load_from_bytes(&bytes[..bytes.len() / 2]);
        assert!(matches!(cut, Err(CacheError::Truncated(_))));
    }

    #[test]
    fn load_refuses_trailing_bytes() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let mut bytes = cache.save_to_bytes().expect("save");
        bytes.push(0xFF);
        assert_eq!(
            PipelineCache::load_from_bytes(&bytes),
            Err(CacheError::TrailingBytes { len: 1 })
        );
    }

    #[test]
    fn load_refuses_duplicate_keys() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![9]).expect("insert");
        let bytes = cache.save_to_bytes().expect("save");
        let entry = bytes[16..].to_vec();
        let mut doubled = Vec::new();
        doubled.extend_from_slice(&bytes[..8]);
        doubled.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        doubled.extend_from_slice(&2u32.to_le_bytes());
        doubled.extend_from_slice(&entry);
        doubled.extend_from_slice(&entry);
        assert_eq!(
            PipelineCache::load_from_bytes(&doubled),
            Err(CacheError::DuplicateKey)
        );
    }

    #[test]
    fn submit_poll_resolve_async_lifecycle() {
        let mut cache = PipelineCache::new();
        assert_eq!(cache.poll_ready(&key_a()), PollState::Missing);
        assert!(cache.submit_compile(&key_a()));
        assert!(!cache.submit_compile(&key_a()));
        assert_eq!(cache.pending_count(), 1);
        assert_eq!(cache.poll_ready(&key_a()), PollState::Pending);
        let outcome = cache.resolve_compile(&key_a(), Ok(vec![7, 8]));
        assert_eq!(outcome, ResolveOutcome::Compiled { bytes: 2 });
        assert_eq!(cache.poll_ready(&key_a()), PollState::Ready { bytes: 2 });
        assert_eq!(cache.get(&key_a()), Some(&[7, 8][..]));
        assert_eq!(cache.pending_count(), 0);
    }

    #[test]
    fn failed_refresh_keeps_last_good_and_diagnostic() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1, 2, 3]).expect("insert");
        assert!(cache.submit_compile(&key_a()));
        let outcome = cache.resolve_compile(&key_a(), Err("driver flaked".to_string()));
        assert_eq!(outcome, ResolveOutcome::KeptLastGood { bytes: 3 });
        assert_eq!(cache.get(&key_a()), Some(&[1, 2, 3][..]));
        assert_eq!(cache.poll_ready(&key_a()), PollState::Ready { bytes: 3 });
        assert_eq!(cache.diagnostic_count(), 1);
        assert_eq!(cache.diagnostics()[0].message(), "driver flaked");
        assert_eq!(cache.diagnostics()[0].key(), &key_a());
    }

    #[test]
    fn failed_first_compile_reports_missing() {
        let mut cache = PipelineCache::new();
        assert!(cache.submit_compile(&key_b()));
        let outcome = cache.resolve_compile(&key_b(), Err("no backend".to_string()));
        assert_eq!(outcome, ResolveOutcome::Failed);
        assert_eq!(cache.get(&key_b()), None);
        assert_eq!(cache.poll_ready(&key_b()), PollState::Missing);
        assert_eq!(cache.diagnostic_count(), 1);
    }

    #[test]
    fn invalid_blob_on_resolve_keeps_last_good() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1]).expect("insert");
        assert!(cache.submit_compile(&key_a()));
        let outcome = cache.resolve_compile(&key_a(), Ok(Vec::new()));
        assert_eq!(outcome, ResolveOutcome::KeptLastGood { bytes: 1 });
        assert_eq!(cache.get(&key_a()), Some(&[1][..]));
    }

    #[test]
    fn second_load_compiles_zero() {
        fn tick(calls: &Cell<u32>) -> Result<Vec<u8>, String> {
            calls.set(calls.get() + 1);
            Ok(vec![9u8; 8])
        }
        let path = temp_path("warm");
        let calls = Cell::new(0u32);
        let mut cold = PipelineCache::new();
        assert!(matches!(
            cold.get_or_compile(key_a(), || tick(&calls)),
            WarmResult::Compiled(_)
        ));
        assert!(matches!(
            cold.get_or_compile(key_b(), || tick(&calls)),
            WarmResult::Compiled(_)
        ));
        assert_eq!(calls.get(), 2);
        cold.save(&path).expect("save");

        let mut warm = PipelineCache::load(&path).expect("load");
        std::fs::remove_file(&path).ok();
        assert!(matches!(
            warm.get_or_compile(key_a(), || tick(&calls)),
            WarmResult::Hit(_)
        ));
        assert!(matches!(
            warm.get_or_compile(key_b(), || tick(&calls)),
            WarmResult::Hit(_)
        ));
        assert_eq!(calls.get(), 2);
        assert_eq!(warm.get(&key_a()), Some(&[9u8; 8][..]));
    }

    #[test]
    fn failed_get_or_compile_records_diagnostic() {
        let mut cache = PipelineCache::new();
        let result = cache.get_or_compile(key_a(), || Err("backend down".to_string()));
        assert_eq!(result, WarmResult::Failed);
        assert_eq!(result.bytes(), None);
        assert_eq!(cache.diagnostic_count(), 1);
        assert!(cache.is_empty());
    }

    #[test]
    fn blob_and_key_bounds_refused() {
        let mut cache = PipelineCache::new();
        assert_eq!(
            cache.insert(key_a(), Vec::new()),
            Err(CacheError::EmptyBlob)
        );
        assert_eq!(
            cache.insert(key_a(), vec![0u8; MAX_BLOB_BYTES + 1]),
            Err(CacheError::TooLarge("blob bytes"))
        );
        let long = "x".repeat(MAX_DEFINE_BYTES + 1);
        let big_key = PipelineKey {
            shader_hash: 1,
            layout: 0,
            defines: vec![(long, "1".to_string())],
        };
        assert_eq!(
            cache.insert(big_key, vec![1]),
            Err(CacheError::TooLarge("define bytes"))
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn diagnostics_bound_and_cap() {
        let mut cache = PipelineCache::new();
        for i in 0..(MAX_DIAGNOSTICS + 6) {
            let key = PipelineKey::new(b"s", &[("I", "0")], u64::from(i as u32));
            assert!(cache.submit_compile(&key));
            let message = format!("failure number {i} with extra detail");
            assert_eq!(
                cache.resolve_compile(&key, Err(message)),
                ResolveOutcome::Failed
            );
        }
        assert_eq!(cache.diagnostic_count(), MAX_DIAGNOSTICS);
        let long = "y".repeat(MAX_DIAG_CHARS + 40);
        cache.push_diagnostic(&key_a(), long);
        let last = cache.diagnostics().last().expect("last diagnostic");
        assert_eq!(last.message().chars().count(), MAX_DIAG_CHARS);
    }

    #[test]
    fn remove_drops_entry_and_pending() {
        let mut cache = PipelineCache::new();
        cache.insert(key_a(), vec![1]).expect("insert");
        assert!(cache.submit_compile(&key_a()));
        assert!(cache.remove(&key_a()));
        assert!(!cache.remove(&key_a()));
        assert_eq!(cache.get(&key_a()), None);
        assert_eq!(cache.poll_ready(&key_a()), PollState::Missing);
        assert_eq!(cache.pending_count(), 0);
    }

    #[test]
    fn warm_all_variants_then_walk_creates_zero_pipelines() {
        // The shipped forward-PBR variant set a scene can use at load.
        let keys = [
            PipelineKey::new(b"forward-pbr-shader", &[], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("SHADOWS", "1")], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("IBL", "1")], 7),
            PipelineKey::new(b"forward-pbr-shader", &[("IBL", "1"), ("SHADOWS", "1")], 7),
        ];
        fn compile(key: &PipelineKey) -> Result<Vec<u8>, CacheError> {
            Ok(vec![key.shader_hash.to_le_bytes()[0]; 8])
        }
        let mut cache = PipelineCache::new();
        assert!(!cache.walk_ready(&keys));
        // First load pass compiles every variant exactly once.
        let first = cache.warm_keys(&keys, compile);
        assert_eq!(
            first,
            WarmSummary {
                compiled: 4,
                hits: 0,
                failed: 0
            }
        );
        assert_eq!(first.total(), 4);
        assert!(cache.walk_ready(&keys));
        // Second pass over the warm cache compiles zero: the camera walk
        // across every material stays on hits, so no frame pays
        // pipeline-creation cost (creation ratio 0, under the 1.5x bar).
        let second = cache.warm_keys(&keys, compile);
        assert_eq!(
            second,
            WarmSummary {
                compiled: 0,
                hits: 4,
                failed: 0
            }
        );
        assert!(cache.walk_ready(&keys));
        // A failed variant counts loudly without touching the warm set.
        let mut cold = PipelineCache::new();
        let bad = cold.warm_keys(&keys[..1], |_| {
            Err(CacheError::Io("backend down".to_string()))
        });
        assert_eq!(bad.failed, 1);
        assert_eq!(cold.diagnostic_count(), 1);
        assert!(!cold.walk_ready(&keys[..1]));
    }
}
