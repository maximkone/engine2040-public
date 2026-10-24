//! bundle: the `.e2bundle` file, a packed copy of a content directory that a
//! browser can fetch in one request and read through [`AssetSource`].
//!
//! Layout, every integer little-endian:
//!
//! ```text
//! header  32  magic "E2BUNDLE", u32 version, u32 entry count,
//!             u64 index byte length, u64 FNV-1a over (version, count,
//!             index length, index bytes)
//! index       entries sorted by path: u16 path length, path, u8 priority,
//!             u64 offset, u64 length, u64 FNV-1a of the blob
//! blobs       back to back in (priority, path) order, no padding
//! ```
//!
//! Priority 0 is what the first frame needs (the coarse level); higher
//! numbers refine it. Because blobs sit in priority order, the bytes of
//! tiers `0..=p` are one prefix of the file, so a streaming client can start
//! on the arena after fetching only that prefix. Offsets are absolute, so a
//! range request needs no arithmetic. The layout is canonical: the same
//! files and rules always give the same bytes, and a reader refuses any file
//! whose blobs are not exactly where the rules put them.
//!
//! The index hash guards the framing; each blob carries its own hash, checked
//! on every read, so one damaged blob fails by name and the rest still load.
//! Integrity only: FNV-1a catches damage, it does not stop a forger.

use super::{validate_path, AssetSource, ContentDigest, SourceError};
use crate::wire::{push_u16, push_u32, push_u64, Cursor, Short};
use engine_core::{hash_bytes, Fnv64};
use std::borrow::Cow;
use std::io::Write;
use std::sync::Arc;

/// Eight bytes opening every bundle.
pub const E2BUNDLE_MAGIC: &[u8; 8] = b"E2BUNDLE";
/// Layout version written by [`pack_to`].
pub const E2BUNDLE_VERSION: u32 = 1;
/// Most files in one bundle; bounds the index a reader allocates.
pub const MAX_E2BUNDLE_ENTRIES: usize = 65_536;
/// Fixed bytes before the index.
const HEADER_LEN: usize = 32;
/// Index bytes of one entry besides its path.
const ENTRY_FIXED_LEN: usize = 2 + 1 + 8 + 8 + 8;

/// Which files to pack and in which order: later tiers refine earlier ones.
#[derive(Clone, Debug, Default)]
pub struct PackRules {
    include: Vec<String>,
    priority: Vec<(String, u8)>,
}

impl PackRules {
    /// Pack every file, all in tier 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pack only files whose path starts with `prefix` (repeatable; plain
    /// string prefix, so `textures/` is a directory and `src/crate.gltf` one
    /// file).
    #[must_use]
    pub fn include(mut self, prefix: &str) -> Self {
        self.include.push(prefix.to_string());
        self
    }

    /// Put files whose path starts with `prefix` in tier `level`. The
    /// longest matching prefix decides; a later call wins on an equal one;
    /// unmatched files stay in tier 0.
    #[must_use]
    pub fn priority(mut self, prefix: &str, level: u8) -> Self {
        self.priority.push((prefix.to_string(), level));
        self
    }

    /// [`PackRules::priority`] from a `<prefix>=<level>` flag value.
    pub fn priority_spec(self, spec: &str) -> Result<Self, SourceError> {
        let refuse =
            || SourceError::bad_pack(&format!("priority {spec:?} is not <prefix>=<0-255>"));
        let (prefix, level) = spec.rsplit_once('=').ok_or_else(refuse)?;
        let level = level.parse::<u8>().map_err(|_| refuse())?;
        Ok(self.priority(prefix, level))
    }

    /// True when `path` is selected for packing.
    pub fn wants(&self, path: &str) -> bool {
        self.include.is_empty()
            || self
                .include
                .iter()
                .any(|prefix| path.starts_with(prefix.as_str()))
    }

    /// Tier of `path`.
    pub fn priority_of(&self, path: &str) -> u8 {
        self.priority
            .iter()
            .filter(|(prefix, _)| path.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map_or(0, |(_, level)| *level)
    }
}

/// What one pack wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackReport {
    /// Files packed.
    pub entries: usize,
    /// Bytes written, header and index included.
    pub bytes: u64,
    /// [`super::content_digest`] of the packed files, read from the source
    /// while packing: a later read of the bundle must reproduce it.
    pub digest: u64,
}

/// One file as the pack surveyed it.
struct Row {
    path: String,
    priority: u8,
    len: usize,
    hash: u64,
}

fn wire_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn put(out: &mut dyn Write, bytes: &[u8]) -> Result<(), SourceError> {
    out.write_all(bytes).map_err(|err| SourceError::Io {
        path: "(bundle output)".to_string(),
        kind: err.kind(),
    })
}

/// Read every selected file once for its length and hash.
fn survey(source: &dyn AssetSource, rules: &PackRules) -> Result<(Vec<Row>, u64), SourceError> {
    let mut paths: Vec<String> = source
        .list("")?
        .into_iter()
        .filter(|path| rules.wants(path))
        .collect();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(SourceError::bad_pack("no file selected to pack"));
    }
    if paths.len() > MAX_E2BUNDLE_ENTRIES {
        return Err(SourceError::bad_pack(&format!(
            "{} files is over the {MAX_E2BUNDLE_ENTRIES} entry cap",
            paths.len()
        )));
    }
    let mut digest = ContentDigest::new();
    let mut rows = Vec::with_capacity(paths.len());
    for path in paths {
        validate_path(&path)?;
        let bytes = source.read(&path)?;
        let hash = hash_bytes(&bytes);
        digest.add_hashed(&path, bytes.len(), hash);
        rows.push(Row {
            priority: rules.priority_of(&path),
            len: bytes.len(),
            hash,
            path,
        });
    }
    Ok((rows, digest.finish()))
}

/// Row indices in blob order: tier first, path order inside a tier (rows are
/// already in path order, and the sort is stable).
fn blob_order(rows: &[Row]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| (rows[i].priority, i));
    order
}

/// Index bytes with each blob's absolute offset in `order`.
fn encode_index(rows: &[Row], order: &[usize]) -> Result<Vec<u8>, SourceError> {
    let index_len: usize = rows
        .iter()
        .map(|row| ENTRY_FIXED_LEN + row.path.len())
        .sum();
    let too_big = || SourceError::bad_pack("bundle is larger than this target addresses");
    let mut at = HEADER_LEN.checked_add(index_len).ok_or_else(too_big)?;
    let mut offsets = vec![0usize; rows.len()];
    for &i in order {
        offsets[i] = at;
        at = at.checked_add(rows[i].len).ok_or_else(too_big)?;
    }
    let mut index = Vec::with_capacity(index_len);
    for (row, offset) in rows.iter().zip(offsets) {
        let path_len = u16::try_from(row.path.len())
            .map_err(|_| SourceError::bad_pack("path length does not fit u16"))?;
        push_u16(&mut index, path_len);
        index.extend_from_slice(row.path.as_bytes());
        index.push(row.priority);
        push_u64(&mut index, wire_u64(offset));
        push_u64(&mut index, wire_u64(row.len));
        push_u64(&mut index, row.hash);
    }
    Ok(index)
}

/// Hash that guards the header fields and the index.
fn framing_hash(count: u32, index: &[u8]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(E2BUNDLE_VERSION);
    hasher.write_u32(count);
    hasher.write_u64(wire_u64(index.len()));
    hasher.write(index);
    hasher.finish()
}

/// Write a bundle of the files `rules` selects from `source` to `out`. Files
/// are read twice (survey, then blobs), one in memory at a time; a file that
/// changed between the two reads fails by name. Same files and rules, same
/// bytes, whatever order the source lists them in.
pub fn pack_to(
    source: &dyn AssetSource,
    rules: &PackRules,
    out: &mut dyn Write,
) -> Result<PackReport, SourceError> {
    let (rows, digest) = survey(source, rules)?;
    let order = blob_order(&rows);
    let index = encode_index(&rows, &order)?;
    let count = u32::try_from(rows.len())
        .map_err(|_| SourceError::bad_pack("entry count does not fit u32"))?;
    let mut head = Vec::with_capacity(HEADER_LEN);
    head.extend_from_slice(E2BUNDLE_MAGIC);
    push_u32(&mut head, E2BUNDLE_VERSION);
    push_u32(&mut head, count);
    push_u64(&mut head, wire_u64(index.len()));
    push_u64(&mut head, framing_hash(count, &index));
    put(out, &head)?;
    put(out, &index)?;
    let mut total = head.len() + index.len();
    for &i in &order {
        let row = &rows[i];
        let bytes = source.read(&row.path)?;
        let actual = hash_bytes(&bytes);
        if bytes.len() != row.len || actual != row.hash {
            return Err(SourceError::HashMismatch {
                path: row.path.clone(),
                expected: row.hash,
                actual,
            });
        }
        put(out, &bytes)?;
        total += bytes.len();
    }
    Ok(PackReport {
        entries: rows.len(),
        bytes: wire_u64(total),
        digest,
    })
}

/// Pack the files `rules` selects under directory `root` into the file
/// `out`. Native only. A failed pack leaves no partial file behind, and an
/// output inside `root` is refused (a rerun would pack its own last output).
#[cfg(not(target_arch = "wasm32"))]
pub fn bundle_pack(
    root: &std::path::Path,
    out: &std::path::Path,
    rules: &PackRules,
) -> Result<PackReport, SourceError> {
    use std::path::Path;
    let label = out.display().to_string();
    let io = |err: &std::io::Error| SourceError::from_io(&label, err);
    let parent = out
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|err| io(&err))?;
    if let (Ok(parent_abs), Ok(root_abs)) = (parent.canonicalize(), root.canonicalize()) {
        if parent_abs.starts_with(root_abs) {
            return Err(SourceError::BadPath {
                path: label.chars().take(128).collect(),
                reason: "output lies inside the content root",
            });
        }
    }
    let file = std::fs::File::create(out).map_err(|err| io(&err))?;
    let mut writer = std::io::BufWriter::new(file);
    let packed = pack_to(&super::FsSource::new(root), rules, &mut writer)
        .and_then(|report| writer.flush().map(|()| report).map_err(|err| io(&err)));
    drop(writer);
    if packed.is_err() {
        // Best effort: the error being returned is the one that matters.
        let _ = std::fs::remove_file(out);
    }
    packed
}

/// One bundle entry as the index records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryInfo<'a> {
    /// Logical path.
    pub path: &'a str,
    /// Tier; 0 is fetched first.
    pub priority: u8,
    /// Absolute byte offset of the blob.
    pub offset: usize,
    /// Blob length in bytes.
    pub len: usize,
    /// FNV-1a of the blob bytes.
    pub hash: u64,
}

/// Index row held by an open bundle.
#[derive(Clone, Debug)]
struct Entry {
    path: String,
    priority: u8,
    offset: usize,
    len: usize,
    hash: u64,
}

/// A packed bundle held in memory, read as an [`AssetSource`]. Opening
/// checks the framing and the blob layout; each blob's own hash is checked
/// when it is read, and `read` borrows the bytes with no copy.
#[derive(Clone, Debug)]
pub struct BundleSource {
    bytes: Arc<[u8]>,
    /// Index rows in path order.
    entries: Vec<Entry>,
    /// Indices into `entries` in file (blob) order.
    file_order: Vec<usize>,
    /// True when the bytes are a fetched prefix of a bigger bundle: the index
    /// names every file, but the files of the later tiers are not here yet.
    prefix: bool,
}

impl BundleSource {
    /// Check the framing of `bytes` and index their files. Damaged or
    /// truncated input is refused with the reason (and the entry, when one is
    /// at fault); nothing is guessed.
    pub fn open(bytes: impl Into<Arc<[u8]>>) -> Result<Self, SourceError> {
        let bytes: Arc<[u8]> = bytes.into();
        let header = parse_header(&bytes)?;
        let entries = parse_index(header.index, header.count)?;
        let file_order = check_layout(&entries, header.data_start, bytes.len())?;
        Ok(Self {
            bytes,
            entries,
            file_order,
            prefix: false,
        })
    }

    /// Open a fetched prefix of a bundle: the header, the index and the blobs
    /// the bytes hold. The index names every file of the bundle, so the files
    /// of the later tiers are known but not here yet; reading one fails with
    /// its name.
    ///
    /// The prefix must end on a blob boundary: bytes that stop inside a blob
    /// are refused, so a reader never serves half a file.
    pub fn open_prefix(bytes: impl Into<Arc<[u8]>>) -> Result<Self, SourceError> {
        let bytes: Arc<[u8]> = bytes.into();
        let header = parse_header(&bytes)?;
        let entries = parse_index(header.index, header.count)?;
        let file_order = check_prefix_layout(&entries, header.data_start, bytes.len())?;
        Ok(Self {
            bytes,
            entries,
            file_order,
            prefix: true,
        })
    }

    /// True when this source holds a fetched prefix of a bigger bundle.
    #[must_use]
    pub fn is_prefix(&self) -> bool {
        self.prefix
    }

    /// Files in the bundle.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the bundle holds no file.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whole bundle size in bytes.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Entries in file order: tier 0 first. The bytes of tiers `0..=p` end
    /// at the last such entry's `offset + len`, which is how much a
    /// streaming client fetches before it can serve them.
    pub fn entries_in_file_order(&self) -> impl Iterator<Item = EntryInfo<'_>> + '_ {
        self.file_order.iter().map(|&i| {
            let entry = &self.entries[i];
            EntryInfo {
                path: entry.path.as_str(),
                priority: entry.priority,
                offset: entry.offset,
                len: entry.len,
                hash: entry.hash,
            }
        })
    }

    fn find(&self, path: &str) -> Option<&Entry> {
        self.entries
            .binary_search_by(|entry| entry.path.as_str().cmp(path))
            .ok()
            .map(|at| &self.entries[at])
    }
}

impl AssetSource for BundleSource {
    fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
        validate_path(path)?;
        let entry = self.find(path).ok_or_else(|| SourceError::missing(path))?;
        let blob = self
            .bytes
            .get(entry.offset..entry.offset + entry.len)
            .ok_or_else(|| {
                if self.prefix {
                    SourceError::bad_bundle(&format!(
                        "blob {path:?} is not in the fetched prefix ({} bytes)",
                        self.bytes.len()
                    ))
                } else {
                    SourceError::bad_bundle(&format!("blob {path:?} left the bundle"))
                }
            })?;
        let actual = hash_bytes(blob);
        if actual != entry.hash {
            return Err(SourceError::HashMismatch {
                path: entry.path.clone(),
                expected: entry.hash,
                actual,
            });
        }
        Ok(Cow::Borrowed(blob))
    }

    fn exists(&self, path: &str) -> bool {
        validate_path(path).is_ok() && self.find(path).is_some()
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
        if dir.is_empty() {
            return Ok(self
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect());
        }
        validate_path(dir)?;
        let prefix = format!("{dir}/");
        let first = self
            .entries
            .partition_point(|entry| entry.path.as_str() < prefix.as_str());
        let found: Vec<String> = self.entries[first..]
            .iter()
            .take_while(|entry| entry.path.starts_with(prefix.as_str()))
            .map(|entry| entry.path.clone())
            .collect();
        if found.is_empty() {
            return Err(SourceError::missing(dir));
        }
        Ok(found)
    }
}

/// Header fields plus the index slice they guard.
struct Header<'a> {
    count: usize,
    index: &'a [u8],
    data_start: usize,
}

fn truncated(short: Short) -> SourceError {
    SourceError::bad_bundle(&format!(
        "truncated: need {} bytes, got {}",
        short.want, short.got
    ))
}

fn parse_header(bytes: &[u8]) -> Result<Header<'_>, SourceError> {
    let mut cur = Cursor::new(bytes);
    if cur.take(E2BUNDLE_MAGIC.len()).map_err(truncated)? != E2BUNDLE_MAGIC {
        return Err(SourceError::bad_bundle("magic is not E2BUNDLE"));
    }
    let version = cur.u32().map_err(truncated)?;
    if version != E2BUNDLE_VERSION {
        return Err(SourceError::bad_bundle(&format!(
            "version {version} unreadable (this reader reads {E2BUNDLE_VERSION})"
        )));
    }
    let count32 = cur.u32().map_err(truncated)?;
    let index_len = cur.u64().map_err(truncated)?;
    let index_hash = cur.u64().map_err(truncated)?;
    let count = usize::try_from(count32).unwrap_or(usize::MAX);
    if count > MAX_E2BUNDLE_ENTRIES {
        return Err(SourceError::bad_bundle(&format!(
            "{count} entries is over the {MAX_E2BUNDLE_ENTRIES} cap"
        )));
    }
    let data_start = usize::try_from(index_len)
        .ok()
        .and_then(|len| HEADER_LEN.checked_add(len))
        .ok_or_else(|| SourceError::bad_bundle("index length is out of range"))?;
    let index = bytes.get(HEADER_LEN..data_start).ok_or_else(|| {
        truncated(Short {
            want: data_start,
            got: bytes.len(),
        })
    })?;
    if framing_hash(count32, index) != index_hash {
        return Err(SourceError::bad_bundle(
            "header or index damaged (index hash mismatch)",
        ));
    }
    Ok(Header {
        count,
        index,
        data_start,
    })
}

fn parse_index(index: &[u8], count: usize) -> Result<Vec<Entry>, SourceError> {
    let ended = |n: usize| SourceError::bad_bundle(&format!("index ends inside entry {n}"));
    let to_usize = |value: u64, path: &str| {
        usize::try_from(value).map_err(|_| {
            SourceError::bad_bundle(&format!(
                "entry {path:?} is larger than this target addresses"
            ))
        })
    };
    let mut cur = Cursor::new(index);
    let mut entries: Vec<Entry> = Vec::with_capacity(count);
    for n in 0..count {
        let path_len = usize::from(cur.u16().map_err(|_| ended(n))?);
        let raw = cur.take(path_len).map_err(|_| ended(n))?;
        let path = std::str::from_utf8(raw)
            .map_err(|_| SourceError::bad_bundle(&format!("entry {n} path is not UTF-8")))?;
        validate_path(path)?;
        if entries
            .last()
            .is_some_and(|prev| prev.path.as_str() >= path)
        {
            return Err(SourceError::bad_bundle(&format!(
                "index not strictly sorted at {path:?}"
            )));
        }
        let priority = cur.u8().map_err(|_| ended(n))?;
        let offset = cur.u64().map_err(|_| ended(n))?;
        let len = cur.u64().map_err(|_| ended(n))?;
        let hash = cur.u64().map_err(|_| ended(n))?;
        entries.push(Entry {
            path: path.to_string(),
            priority,
            offset: to_usize(offset, path)?,
            len: to_usize(len, path)?,
            hash,
        });
    }
    if cur.pos() != index.len() {
        return Err(SourceError::bad_bundle(
            "index has bytes after its last entry",
        ));
    }
    Ok(entries)
}

/// The blobs must sit exactly where (priority, path) order puts them and end
/// at the end of the file; returns entry indices in that order.
fn check_layout(
    entries: &[Entry],
    data_start: usize,
    total: usize,
) -> Result<Vec<usize>, SourceError> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| (entries[i].priority, i));
    let mut at = data_start;
    for &i in &order {
        let entry = &entries[i];
        if entry.offset != at {
            return Err(SourceError::bad_bundle(&format!(
                "blob {:?} is at {} but the layout puts it at {at}",
                entry.path, entry.offset
            )));
        }
        at = at
            .checked_add(entry.len)
            .ok_or_else(|| SourceError::bad_bundle("blob lengths overflow"))?;
        if at > total {
            return Err(SourceError::bad_bundle(&format!(
                "truncated: blob {:?} ends at {at}, bundle has {total} bytes",
                entry.path
            )));
        }
    }
    if at != total {
        return Err(SourceError::bad_bundle(&format!(
            "{} bytes after the last blob",
            total - at
        )));
    }
    Ok(order)
}

/// The blobs of a fetched prefix: the same (priority, path) order as the whole
/// file, with the blobs the bytes hold at the front. The index gives every
/// blob's place, so the layout is checked whole; the prefix itself must end on
/// a blob boundary, and bytes that stop inside a blob are refused by name.
fn check_prefix_layout(
    entries: &[Entry],
    data_start: usize,
    total: usize,
) -> Result<Vec<usize>, SourceError> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| (entries[i].priority, i));
    let mut at = data_start;
    let mut prefix_end = data_start;
    for &i in &order {
        let entry = &entries[i];
        if entry.offset != at {
            return Err(SourceError::bad_bundle(&format!(
                "blob {:?} is at {} but the layout puts it at {at}",
                entry.path, entry.offset
            )));
        }
        let end = at
            .checked_add(entry.len)
            .ok_or_else(|| SourceError::bad_bundle("blob lengths overflow"))?;
        if end <= total {
            prefix_end = end;
        }
        at = end;
    }
    if total != prefix_end {
        let cut = order
            .iter()
            .map(|&i| &entries[i])
            .find(|entry| entry.offset + entry.len > total);
        let name = cut.map_or("a blob", |entry| entry.path.as_str());
        return Err(SourceError::bad_bundle(&format!(
            "the prefix ends inside blob {name:?} (have {total} bytes, the file has {at})"
        )));
    }
    Ok(order)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::super::fs_tests::fixture;
    use super::super::{content_digest, FsSource};
    use super::*;

    /// Bundle of the whole fixture dir under `rules`, as bytes.
    fn packed(dir: &std::path::Path, rules: &PackRules) -> Vec<u8> {
        let mut bytes = Vec::new();
        pack_to(&FsSource::new(dir), rules, &mut bytes).expect("fixture packs");
        bytes
    }

    fn tiered() -> PackRules {
        PackRules::new()
            .priority("textures/", 2)
            .priority("src/", 1)
            .priority("textures/deep/", 3)
    }

    #[test]
    fn bundle_pack_twice_gives_equal_bytes() {
        let (a, b) = (fixture("pack_a"), fixture("pack_b"));
        let first = packed(&a, &tiered());
        assert_eq!(first, packed(&b, &tiered()), "two dirs, same bytes");
        assert_eq!(first, packed(&a, &tiered()), "same dir twice, same bytes");
        let flat = packed(&a, &PackRules::new());
        assert_ne!(first, flat, "the rules are part of the bytes");
        // A bundle read back and packed again is the same file.
        let again = BundleSource::open(first.clone()).expect("opens");
        let mut repacked = Vec::new();
        pack_to(&again, &tiered(), &mut repacked).expect("repacks");
        assert_eq!(first, repacked);
        for dir in [a, b] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_prefix_bundle_reads_its_first_tier_and_names_what_it_lacks() {
        let dir = fixture("prefix");
        let bytes = packed(&dir, &tiered());
        let whole = BundleSource::open(bytes.clone()).expect("opens");
        assert!(!whole.is_prefix(), "the whole file is not a prefix");
        // The bytes of the first tier are one prefix of the file.
        let cut = whole
            .entries_in_file_order()
            .filter(|entry| entry.priority == 0)
            .map(|entry| entry.offset + entry.len)
            .max()
            .expect("a tier 0 file");
        let prefix = BundleSource::open_prefix(bytes[..cut].to_vec()).expect("prefix opens");
        assert!(prefix.is_prefix(), "the prefix says so");
        assert_eq!(
            prefix.read("placement.json").expect("tier 0"),
            whole.read("placement.json").expect("whole"),
            "the tier 0 file is the same bytes"
        );
        let err = prefix
            .read("src/crate.gltf")
            .expect_err("a later tier is not here");
        let text = err.to_string();
        assert!(
            text.contains("src/crate.gltf"),
            "the refusal names the file: {text}"
        );
        assert!(text.contains("prefix"), "the refusal says why: {text}");
        // The index still knows every file of the bundle.
        assert_eq!(
            prefix.list("").expect("list").len(),
            whole.list("").expect("list").len(),
            "the prefix knows the whole index"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// QW4: the HDRI tier the page streams behind the progress bar is the first
    /// bytes of the bundle, so one prefix cut serves every HDRI file and the
    /// tiers behind it still refuse by name.
    #[test]
    fn the_hdri_tier_is_first_in_the_bundle() {
        let dir = fixture("hdri_tier");
        std::fs::create_dir_all(dir.join("hdri")).expect("mkdir");
        for (i, len) in [(0usize, 4096usize), (1, 8), (2, 1024)] {
            std::fs::write(dir.join(format!("hdri/set{i}.hdr")), vec![0xABu8; len]).expect("write");
        }
        let rules = PackRules::new()
            .priority("hdri/", 0)
            .priority("src/", 1)
            .priority("textures/", 2);
        let bytes = packed(&dir, &rules);
        let whole = BundleSource::open(bytes.clone()).expect("opens");
        let hdri: Vec<EntryInfo<'_>> = whole
            .entries_in_file_order()
            .filter(|entry| entry.path.starts_with("hdri/"))
            .collect();
        assert_eq!(hdri.len(), 3, "the three HDRI blobs are packed");
        let last_hdri = hdri
            .iter()
            .map(|entry| entry.offset + entry.len)
            .max()
            .expect("an hdri blob");
        let first_later = whole
            .entries_in_file_order()
            .filter(|entry| entry.priority > 0)
            .map(|entry| entry.offset)
            .min()
            .expect("a later tier");
        assert!(
            last_hdri <= first_later,
            "the hdri tier ends at {last_hdri} B but a later tier starts at {first_later} B"
        );
        // The prefix that stops on that boundary holds every HDRI file.
        let prefix = BundleSource::open_prefix(bytes[..last_hdri].to_vec()).expect("prefix opens");
        assert!(prefix.is_prefix(), "the prefix says so");
        for i in 0..3 {
            let name = format!("hdri/set{i}.hdr");
            assert_eq!(
                prefix.read(&name).expect("hdri bytes"),
                whole.read(&name).expect("whole file"),
                "{name}"
            );
        }
        let err = prefix
            .read("src/crate.gltf")
            .expect_err("a later tier is not here");
        assert!(
            err.to_string().contains("src/crate.gltf"),
            "the refusal names the file: {err}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_prefix_that_stops_inside_a_blob_is_refused() {
        let dir = fixture("cut");
        let bytes = packed(&dir, &tiered());
        let whole = BundleSource::open(bytes.clone()).expect("opens");
        let first = whole.entries_in_file_order().next().expect("a file");
        let cut = first.offset + first.len - 1;
        let err =
            BundleSource::open_prefix(bytes[..cut].to_vec()).expect_err("half a file is no file");
        let text = err.to_string();
        assert!(
            text.contains(first.path),
            "the refusal names the blob: {text}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_whole_file_opens_as_a_prefix_and_reads_everything() {
        let dir = fixture("whole_prefix");
        let bytes = packed(&dir, &tiered());
        let prefix = BundleSource::open_prefix(bytes.clone()).expect("opens");
        let whole = BundleSource::open(bytes).expect("opens");
        for name in whole.list("").expect("list") {
            assert_eq!(
                prefix.read(&name).expect("read"),
                whole.read(&name).expect("read"),
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_returns_the_bytes_the_file_system_does() {
        let dir = fixture("same");
        let fs = FsSource::new(&dir);
        let bundle = BundleSource::open(packed(&dir, &tiered())).expect("opens");
        let names = fs.list("").expect("fs list");
        assert_eq!(bundle.list("").expect("bundle list"), names);
        assert_eq!(bundle.len(), names.len());
        for name in &names {
            assert_eq!(
                bundle.read(name).expect("bundle"),
                fs.read(name).expect("fs"),
                "{name}"
            );
            assert!(bundle.exists(name) && fs.exists(name));
        }
        for dir_name in ["src", "textures", "textures/deep"] {
            assert_eq!(bundle.list(dir_name), fs.list(dir_name), "{dir_name}");
        }
        for gone in ["src/none.gltf", "src", "../escape", ""] {
            assert!(!bundle.exists(gone) && !fs.exists(gone), "{gone:?}");
        }
        assert_eq!(bundle.list("nowhere"), fs.list("nowhere"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_content_digest_equals_the_file_system_digest() {
        let dir = fixture("digest");
        let mut bytes = Vec::new();
        let report = pack_to(&FsSource::new(&dir), &tiered(), &mut bytes).expect("packs");
        let bundle = BundleSource::open(bytes.clone()).expect("opens");
        let native = content_digest(&FsSource::new(&dir), "").expect("fs digest");
        assert_eq!(content_digest(&bundle, "").expect("bundle digest"), native);
        assert_eq!(report.digest, native, "the pack folds like a later read");
        assert_eq!(report.bytes, wire_u64(bytes.len()));
        assert_eq!(report.entries, 5);
        assert_eq!(
            content_digest(&bundle, "src").expect("subset"),
            content_digest(&FsSource::new(&dir), "src").expect("subset")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_corrupt_blob_fails_with_its_name_and_spares_the_rest() {
        let dir = fixture("corrupt");
        let mut bytes = packed(&dir, &tiered());
        let victim = BundleSource::open(bytes.clone())
            .expect("opens")
            .entries_in_file_order()
            .find(|entry| entry.path == "src/crate.gltf")
            .map(|entry| entry.offset + 3)
            .expect("victim entry");
        bytes[victim] ^= 0xFF;
        let bundle = BundleSource::open(bytes).expect("framing is intact, so it opens");
        match bundle.read("src/crate.gltf").map(|bytes| bytes.len()) {
            Err(SourceError::HashMismatch {
                path,
                expected,
                actual,
            }) => {
                assert_eq!(path, "src/crate.gltf");
                assert_ne!(expected, actual);
            }
            other => panic!("damaged blob must fail by name, got {other:?}"),
        }
        let text = bundle
            .read("src/crate.gltf")
            .expect_err("still damaged")
            .to_string();
        assert!(text.contains("src/crate.gltf"), "{text}");
        assert!(
            bundle.read("src/banner.gltf").is_ok(),
            "other blobs still load"
        );
        let digest = content_digest(&bundle, "").expect_err("a whole-source read hits it");
        assert!(digest.to_string().contains("src/crate.gltf"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_missing_path_fails_with_its_name() {
        let dir = fixture("absent");
        let bundle = BundleSource::open(packed(&dir, &PackRules::new())).expect("opens");
        let err = bundle
            .read("textures/forgotten.jpg")
            .expect_err("not packed");
        assert_eq!(err, SourceError::missing("textures/forgotten.jpg"));
        assert!(err.to_string().contains("textures/forgotten.jpg"));
        assert!(matches!(
            bundle.read("../x"),
            Err(SourceError::BadPath { .. })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_priority_puts_coarse_blobs_first_in_the_file() {
        let dir = fixture("order");
        let bundle = BundleSource::open(packed(&dir, &tiered())).expect("opens");
        let order: Vec<(&str, u8)> = bundle
            .entries_in_file_order()
            .map(|entry| (entry.path, entry.priority))
            .collect();
        assert_eq!(
            order,
            [
                ("placement.json", 0),
                ("src/banner.gltf", 1),
                ("src/crate.gltf", 1),
                ("textures/sand_2k.jpg", 2),
                ("textures/deep/stone.jpg", 3),
            ],
            "longest prefix decides the tier; tiers sort, then paths"
        );
        // Tiers are file prefixes: offsets run on with no gap.
        let mut at = bundle
            .entries_in_file_order()
            .next()
            .map(|e| e.offset)
            .expect("first");
        for entry in bundle.entries_in_file_order() {
            assert_eq!(entry.offset, at, "{}", entry.path);
            at += entry.len;
        }
        assert_eq!(at, bundle.byte_len());
        // The index stays in path order whatever the blob order is.
        assert_eq!(bundle.list("").expect("list"), {
            let mut sorted = bundle.list("").expect("list");
            sorted.sort();
            sorted
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_framing_damage_is_refused_with_a_reason() {
        let dir = fixture("framing");
        let good = packed(&dir, &tiered());
        let reason = |bytes: Vec<u8>| match BundleSource::open(bytes) {
            Err(SourceError::BadBundle { detail }) => detail,
            other => panic!("expected a bad bundle, got {other:?}"),
        };
        let mut wrong_magic = good.clone();
        wrong_magic[0] = b'X';
        assert!(reason(wrong_magic).contains("magic"));
        let mut future = good.clone();
        future[8] = 9;
        assert!(reason(future).contains("version 9"));
        let mut index_flip = good.clone();
        index_flip[HEADER_LEN + 5] ^= 1;
        assert!(reason(index_flip).contains("index hash"));
        let mut tail = good.clone();
        tail.push(0);
        assert!(reason(tail).contains("after the last blob"));
        assert!(reason(good[..10].to_vec()).contains("truncated"));
        assert!(reason(Vec::new()).contains("truncated"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_truncated_download_names_the_first_blob_it_cuts() {
        let dir = fixture("cut");
        let good = packed(&dir, &tiered());
        let bundle = BundleSource::open(good.clone()).expect("opens");
        // Empty blobs add no bytes, so the file ends with the last non-empty one.
        let entries: Vec<EntryInfo<'_>> = bundle.entries_in_file_order().collect();
        let tail = entries
            .iter()
            .rev()
            .find(|entry| entry.len > 0)
            .expect("a non-empty blob");
        assert_eq!(tail.offset + tail.len, good.len());
        match BundleSource::open(good[..good.len() - 1].to_vec()) {
            Err(SourceError::BadBundle { detail }) => {
                assert!(
                    detail.contains("truncated") && detail.contains(tail.path),
                    "{detail}"
                );
            }
            other => panic!("a cut file must not open, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One index entry as bytes, for hand-built bad indexes.
    fn raw_entry(path: &str, offset: u64) -> Vec<u8> {
        let mut out = Vec::new();
        push_u16(&mut out, u16::try_from(path.len()).expect("short path"));
        out.extend_from_slice(path.as_bytes());
        out.push(0);
        push_u64(&mut out, offset);
        push_u64(&mut out, 0);
        push_u64(&mut out, hash_bytes(b""));
        out
    }

    #[test]
    fn bundle_index_rows_must_be_sorted_unique_and_valid_paths() {
        let rows = |names: &[&str]| -> Vec<u8> {
            names.iter().flat_map(|name| raw_entry(name, 0)).collect()
        };
        assert!(parse_index(&rows(&["a", "b"]), 2).is_ok());
        for (names, why) in [
            (&["b", "a"][..], "out of order"),
            (&["a", "a"][..], "duplicate"),
            (&["a", "../b"][..], "escaping path"),
        ] {
            assert!(parse_index(&rows(names), names.len()).is_err(), "{why}");
        }
        assert!(
            parse_index(&rows(&["a"]), 2).is_err(),
            "count past the index"
        );
        let mut extra = rows(&["a"]);
        extra.push(0);
        assert!(
            parse_index(&extra, 1).is_err(),
            "bytes after the last entry"
        );
    }

    #[test]
    fn bundle_pack_refuses_nothing_to_pack_and_changed_files() {
        let dir = fixture("refuse");
        let mut sink = Vec::new();
        let none = pack_to(
            &FsSource::new(&dir),
            &PackRules::new().include("nothing/"),
            &mut sink,
        );
        assert!(matches!(none, Err(SourceError::BadPack { .. })), "{none:?}");
        assert!(sink.is_empty(), "nothing is written for an empty selection");

        // A file edited between the survey and the blob pass fails by name.
        struct Shifty<'a> {
            inner: FsSource,
            reads: std::cell::Cell<usize>,
            wobble: &'a str,
        }
        impl AssetSource for Shifty<'_> {
            fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
                let seen = self.reads.get();
                self.reads.set(seen + 1);
                let mut bytes = self.inner.read(path)?.into_owned();
                if path == self.wobble && seen >= 5 {
                    bytes.push(1);
                }
                Ok(Cow::Owned(bytes))
            }
            fn exists(&self, path: &str) -> bool {
                self.inner.exists(path)
            }
            fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
                self.inner.list(dir)
            }
        }
        let shifty = Shifty {
            inner: FsSource::new(&dir),
            reads: std::cell::Cell::new(0),
            wobble: "src/banner.gltf",
        };
        let err = pack_to(&shifty, &PackRules::new(), &mut Vec::new()).expect_err("file changed");
        assert!(
            matches!(&err, SourceError::HashMismatch { path, .. } if path == "src/banner.gltf"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundle_rules_select_and_rank_by_prefix() {
        let rules = PackRules::new().include("src/").include("placement");
        assert!(rules.wants("src/a.gltf") && rules.wants("placement.json"));
        assert!(!rules.wants("textures/a.jpg"));
        assert!(PackRules::new().wants("anything"), "no include means all");
        let ranked = PackRules::new()
            .priority_spec("textures/=2")
            .and_then(|r| r.priority_spec("textures/deep/=5"))
            .and_then(|r| r.priority_spec("textures/deep/=6"))
            .expect("specs parse");
        assert_eq!(ranked.priority_of("textures/a.jpg"), 2);
        assert_eq!(
            ranked.priority_of("textures/deep/b.jpg"),
            6,
            "longest, then latest"
        );
        assert_eq!(ranked.priority_of("src/a.gltf"), 0, "unmatched is tier 0");
        for bad in ["textures", "textures=", "textures=300", "textures=-1"] {
            assert!(PackRules::new().priority_spec(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bundle_pack_file_matches_the_in_memory_pack_and_cleans_up_failures() {
        let dir = fixture("file");
        let out_dir = std::env::temp_dir().join(format!("asset_source_out_{}", std::process::id()));
        let out = out_dir.join("nested").join("arena.e2bundle");
        let report = bundle_pack(&dir, &out, &tiered()).expect("packs to a file");
        let on_disk = std::fs::read(&out).expect("bundle file");
        assert_eq!(on_disk, packed(&dir, &tiered()));
        assert_eq!(report.bytes, wire_u64(on_disk.len()));

        let failed = out_dir.join("failed.e2bundle");
        let err = bundle_pack(&dir, &failed, &PackRules::new().include("nothing/"))
            .expect_err("empty selection");
        assert!(matches!(err, SourceError::BadPack { .. }));
        assert!(!failed.exists(), "a failed pack leaves no file");

        let inside = dir.join("self.e2bundle");
        let err = bundle_pack(&dir, &inside, &PackRules::new()).expect_err("output inside root");
        assert!(matches!(err, SourceError::BadPath { .. }), "{err}");
        assert!(!inside.exists());
        for path in [dir, out_dir] {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
