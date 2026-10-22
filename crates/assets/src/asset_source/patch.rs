//! patch: content ships as a base plus a patch that holds only changed blobs.
//!
//! Owner: `engine-assets`. A content version is a [`Manifest`]: every logical
//! path with its byte length and blob hash. A [`PatchSet`] carries the new
//! manifest plus only the blobs whose bytes differ from the base, so a client
//! downloads the changed files alone. Applying a patch over the base
//! ([`PatchSet::apply`]) gives a [`PatchedSource`]: a patched path serves the
//! patch blob, every other path serves the base bytes once they hash to the
//! manifest, and a patch blob that does not hash to its manifest row is
//! refused by name. Same base and same new content always write the same
//! patch bytes. Std-only; no clock, no RNG, no threads.
//!
//! Hashes are FNV-1a 64 from `engine_core`, the same blob hash the
//! `.e2bundle` index carries: that catches damage and a wrong download, it
//! does not stop a forger. A signature is the transport's job.
//!
//! Layout, every integer little-endian:
//!
//! ```text
//! header  32  magic "E2PATCH\0", u32 version, u32 row count,
//!             u64 manifest byte length, u64 FNV-1a over (version, count,
//!             manifest length, manifest bytes)
//! manifest    rows in path order: u16 path length, path, u8 stored,
//!             u64 byte length, u64 blob hash
//! blobs       stored rows back to back in path order, no padding
//! ```
//!
//! `stored` is 1 when the blob rides in the patch (added or changed) and 0
//! when the base already holds it. Blobs carry no offsets on the wire: stored
//! rows are in path order, so each blob starts where the last one ended. The
//! manifest hash guards the framing; each blob is checked against its own row,
//! so one damaged blob fails by name and the rest still apply.

use super::{validate_path, AssetSource, SourceError};
use crate::wire::{bound_chars, push_u16, push_u32, push_u64, Cursor, Short};
use engine_core::{hash_bytes, Fnv64};
use std::borrow::Cow;
use std::fmt;

/// Eight bytes opening every patch.
pub const PATCH_MAGIC: &[u8; 8] = b"E2PATCH\0";
/// Layout version written by [`PatchSet::to_bytes`].
pub const PATCH_VERSION: u32 = 1;
/// Most files one manifest or patch may list; bounds the rows a reader allocates.
pub const MAX_PATCH_ENTRIES: usize = 65_536;
/// Largest patch accepted in bytes; bounds the input before parsing.
pub const MAX_PATCH_BYTES: usize = 512 * 1024 * 1024;
/// Fixed bytes before the manifest.
const HEADER_LEN: usize = 32;
/// Chars of a path or reason echoed inside an error.
const ECHO_CHARS: usize = 128;

/// Why a patch build, load or apply failed; every bad input maps to one variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// A patch blob's bytes hash to a different value than its manifest row
    /// claims; the blob is refused and the file its path names is never served.
    BlobHash {
        /// Logical path of the damaged blob.
        path: String,
        /// Hash recorded in the manifest row.
        expected: u64,
        /// Hash of the bytes actually present.
        actual: u64,
    },
    /// The base holds a file the manifest keeps, but its bytes moved.
    BaseHash {
        /// Logical path whose base bytes disagree.
        path: String,
        /// Hash the manifest records.
        expected: u64,
        /// Hash of the base bytes.
        actual: u64,
    },
    /// The base holds a file the manifest keeps, but its length moved.
    BaseLength {
        /// Logical path whose base length disagrees.
        path: String,
        /// Length the manifest records.
        expected: u64,
        /// Length of the base bytes.
        actual: usize,
    },
    /// The base does not hold a file the manifest keeps and the patch omits.
    BaseMissing {
        /// Logical path the base lacks.
        path: String,
    },
    /// The bytes are not a patch this reader reads; detail is bounded.
    BadPatch {
        /// Bounded reason, naming the entry when one is at fault.
        detail: String,
    },
    /// The base or the new content could not be read.
    Source(SourceError),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlobHash {
                path,
                expected,
                actual,
            } => write!(
                f,
                "patch blob {path:?} hash mismatch: manifest says {expected:016x}, bytes hash to {actual:016x}"
            ),
            Self::BaseHash {
                path,
                expected,
                actual,
            } => write!(
                f,
                "base file {path:?} hash mismatch: manifest says {expected:016x}, base bytes hash to {actual:016x}"
            ),
            Self::BaseLength {
                path,
                expected,
                actual,
            } => write!(
                f,
                "base file {path:?} is {actual} bytes, the manifest says {expected}"
            ),
            Self::BaseMissing { path } => write!(f, "base is missing {path:?}"),
            Self::BadPatch { detail } => write!(f, "bad patch: {detail}"),
            Self::Source(err) => write!(f, "patch source: {err}"),
        }
    }
}

impl std::error::Error for PatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SourceError> for PatchError {
    fn from(err: SourceError) -> Self {
        Self::Source(err)
    }
}

impl From<Short> for PatchError {
    fn from(short: Short) -> Self {
        bad_patch(&format!(
            "truncated: need {} bytes, got {}",
            short.want, short.got
        ))
    }
}

/// A framing refusal with a bounded reason.
fn bad_patch(detail: &str) -> PatchError {
    PatchError::BadPatch {
        detail: bound_chars(detail, ECHO_CHARS * 2),
    }
}

/// `value` as a wire `u32`, or a refusal when it does not fit.
fn wire_u32(value: usize, what: &str) -> Result<u32, PatchError> {
    u32::try_from(value).map_err(|_| bad_patch(&format!("{what} do not fit the wire")))
}

/// `value` as a wire `u64`, or a refusal when it does not fit.
fn wire_u64(value: usize, what: &str) -> Result<u64, PatchError> {
    u64::try_from(value).map_err(|_| bad_patch(&format!("{what} do not fit the wire")))
}

/// One row of a content manifest: a path, its byte length and its blob hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestRow {
    /// Logical path of the file.
    pub path: String,
    /// Byte length of the file.
    pub len: u64,
    /// FNV-1a 64 of the file bytes.
    pub hash: u64,
}

/// One content version as a sorted path-to-blob table: what a patch keeps,
/// replaces or adds, and what a client can check a downloaded file against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    rows: Vec<ManifestRow>,
}

impl Manifest {
    /// Survey `source`: every file in path order with its length and blob
    /// hash. Each file is read once.
    ///
    /// # Errors
    /// A source read failure is carried. An empty source, more than
    /// [`MAX_PATCH_ENTRIES`] files, or a path that is not a logical path is
    /// refused, so a manifest always names at least one file.
    pub fn from_source(source: &dyn AssetSource) -> Result<Self, SourceError> {
        let mut paths = source.list("")?;
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return Err(SourceError::bad_pack("no file in the content to survey"));
        }
        if paths.len() > MAX_PATCH_ENTRIES {
            return Err(SourceError::bad_pack(&format!(
                "{} files is over the {MAX_PATCH_ENTRIES} entry cap",
                paths.len()
            )));
        }
        let mut rows = Vec::with_capacity(paths.len());
        for path in paths {
            validate_path(&path)?;
            let bytes = source.read(&path)?;
            rows.push(ManifestRow {
                path,
                len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                hash: hash_bytes(&bytes),
            });
        }
        Ok(Self { rows })
    }

    /// Rows in path order.
    #[must_use]
    pub fn rows(&self) -> &[ManifestRow] {
        &self.rows
    }

    /// Files the manifest lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the manifest lists no file (only an empty `Manifest` value;
    /// [`Manifest::from_source`] never builds one).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row for `path`, when the manifest lists it.
    #[must_use]
    pub fn find(&self, path: &str) -> Option<&ManifestRow> {
        self.rows
            .binary_search_by(|row| row.path.as_str().cmp(path))
            .ok()
            .map(|at| &self.rows[at])
    }
}

/// One blob a patch carries: its logical path and the exact bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredBlob {
    path: String,
    bytes: Vec<u8>,
}

/// A content patch: the new manifest plus only the blobs the base lacks.
#[derive(Clone, Debug)]
pub struct PatchSet {
    manifest: Manifest,
    /// Stored blobs in path order; every path also has a manifest row.
    stored: Vec<StoredBlob>,
}

/// What one patch build found when it compared base and new content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PatchStats {
    /// Files whose length and hash match the base.
    pub unchanged: usize,
    /// Files the base holds with other bytes.
    pub changed: usize,
    /// Files the base does not hold.
    pub added: usize,
    /// Base files the new manifest no longer lists.
    pub removed: usize,
    /// Files in the new manifest.
    pub entries: usize,
    /// Blobs the patch carries (added plus changed).
    pub blobs: usize,
    /// Bytes of those blobs, before header and manifest.
    pub blob_bytes: usize,
    /// Bytes of the serialized patch, header and manifest included.
    pub patch_bytes: usize,
    /// FNV-1a 64 of the serialized patch, for a receipt.
    pub digest: u64,
}

impl PatchSet {
    /// Compare `new` against `base` and stage the blobs whose bytes differ.
    ///
    /// The survey pass reads every new file once for its manifest row; the
    /// blob pass re-reads only added and changed files and refuses one that
    /// moved between the passes by name.
    ///
    /// # Errors
    /// [`PatchError::Source`] carries a read failure; a file edited during the
    /// build arrives as [`SourceError::HashMismatch`].
    pub fn build(base: &Manifest, new: &dyn AssetSource) -> Result<(Self, PatchStats), PatchError> {
        let manifest = Manifest::from_source(new)?;
        let mut stats = PatchStats {
            entries: manifest.len(),
            ..PatchStats::default()
        };
        let mut stored = Vec::new();
        for row in manifest.rows() {
            match base.find(&row.path) {
                Some(old) if old.hash == row.hash && old.len == row.len => {
                    stats.unchanged += 1;
                    continue;
                }
                Some(_) => stats.changed += 1,
                None => stats.added += 1,
            }
            let bytes = new.read(&row.path)?;
            let actual = hash_bytes(&bytes);
            if actual != row.hash || u64::try_from(bytes.len()).unwrap_or(u64::MAX) != row.len {
                return Err(PatchError::Source(SourceError::HashMismatch {
                    path: row.path.clone(),
                    expected: row.hash,
                    actual,
                }));
            }
            stats.blobs += 1;
            stats.blob_bytes = stats.blob_bytes.saturating_add(bytes.len());
            stored.push(StoredBlob {
                path: row.path.clone(),
                bytes: bytes.into_owned(),
            });
        }
        stats.removed = base
            .rows()
            .iter()
            .filter(|old| manifest.find(&old.path).is_none())
            .count();
        let set = Self { manifest, stored };
        let bytes = set.to_bytes()?;
        stats.patch_bytes = bytes.len();
        stats.digest = hash_bytes(&bytes);
        Ok((set, stats))
    }

    /// The manifest this patch installs: every file of the new content.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Paths whose bytes ride in this patch, in path order: the files a client
    /// fetches.
    pub fn stored_paths(&self) -> impl Iterator<Item = &str> + '_ {
        self.stored.iter().map(|blob| blob.path.as_str())
    }

    /// Blobs the patch carries.
    #[must_use]
    pub fn stored_len(&self) -> usize {
        self.stored.len()
    }

    /// Bytes the patch carries before its header and manifest.
    #[must_use]
    pub fn stored_bytes(&self) -> usize {
        self.stored.iter().map(|blob| blob.bytes.len()).sum()
    }

    /// Serialize the patch: header, new manifest, stored blobs. The same set
    /// always gives the same bytes.
    ///
    /// # Errors
    /// [`PatchError::BadPatch`] when a row no longer fits the wire; a manifest
    /// built by [`Manifest::from_source`] or [`PatchSet::parse`] always does.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PatchError> {
        let manifest = self.encode_manifest()?;
        let count = wire_u32(self.manifest.len(), "manifest rows")?;
        let manifest_len = wire_u64(manifest.len(), "manifest bytes")?;
        let mut out = Vec::with_capacity(HEADER_LEN + manifest.len() + self.stored_bytes());
        out.extend_from_slice(PATCH_MAGIC);
        push_u32(&mut out, PATCH_VERSION);
        push_u32(&mut out, count);
        push_u64(&mut out, manifest_len);
        push_u64(&mut out, framing_hash(count, &manifest));
        out.extend_from_slice(&manifest);
        for blob in &self.stored {
            out.extend_from_slice(&blob.bytes);
        }
        Ok(out)
    }

    /// Manifest rows with their stored flags, in path order.
    fn encode_manifest(&self) -> Result<Vec<u8>, PatchError> {
        let mut out = Vec::new();
        for row in self.manifest.rows() {
            let path_len = u16::try_from(row.path.len())
                .map_err(|_| bad_patch(&format!("path {:?} does not fit the wire", row.path)))?;
            push_u16(&mut out, path_len);
            out.extend_from_slice(row.path.as_bytes());
            out.push(u8::from(self.stored_blob(&row.path).is_some()));
            push_u64(&mut out, row.len);
            push_u64(&mut out, row.hash);
        }
        Ok(out)
    }

    /// Stored bytes of `path`, when this patch carries them.
    fn stored_blob(&self, path: &str) -> Option<&[u8]> {
        self.stored
            .binary_search_by(|blob| blob.path.as_str().cmp(path))
            .ok()
            .map(|at| self.stored[at].bytes.as_slice())
    }

    /// Read a serialized patch back. Every stored blob must hash to its
    /// manifest row; a damaged blob is refused by name.
    ///
    /// # Errors
    /// [`PatchError::BlobHash`] names the blob whose bytes do not match; a
    /// bad magic, version, framing hash, row or length is [`PatchError::BadPatch`].
    pub fn parse(bytes: &[u8]) -> Result<Self, PatchError> {
        if bytes.len() > MAX_PATCH_BYTES {
            return Err(bad_patch(&format!(
                "{} bytes is over the {MAX_PATCH_BYTES} byte cap",
                bytes.len()
            )));
        }
        let header = parse_header(bytes)?;
        let rows = parse_manifest(header.count, header.manifest)?;
        let mut stored = Vec::new();
        let mut manifest_rows = Vec::with_capacity(rows.len());
        let mut at = header.data_start;
        for (row, is_stored) in rows {
            if is_stored {
                let len = usize::try_from(row.len)
                    .map_err(|_| bad_patch(&format!("blob {:?} is too large here", row.path)))?;
                let end = at
                    .checked_add(len)
                    .ok_or_else(|| bad_patch("blob lengths overflow"))?;
                let Some(blob) = bytes.get(at..end) else {
                    return Err(bad_patch(&format!(
                        "patch ends inside blob {:?}: need {end} bytes, got {}",
                        row.path,
                        bytes.len()
                    )));
                };
                let actual = hash_bytes(blob);
                if actual != row.hash {
                    return Err(PatchError::BlobHash {
                        path: row.path,
                        expected: row.hash,
                        actual,
                    });
                }
                at = end;
                stored.push(StoredBlob {
                    path: row.path.clone(),
                    bytes: blob.to_vec(),
                });
            }
            manifest_rows.push(row);
        }
        if at != bytes.len() {
            return Err(bad_patch(&format!(
                "{} bytes after the last blob",
                bytes.len() - at
            )));
        }
        Ok(Self {
            manifest: Manifest {
                rows: manifest_rows,
            },
            stored,
        })
    }

    /// Check that `base` holds every file this patch keeps. Stored paths need
    /// nothing from the base; a kept path must exist with the manifest's
    /// length and hash. Stops at the first mismatch, naming it.
    ///
    /// # Errors
    /// [`PatchError::BaseMissing`], [`PatchError::BaseLength`],
    /// [`PatchError::BaseHash`] or the source failure.
    pub fn verify_base(&self, base: &dyn AssetSource) -> Result<(), PatchError> {
        for row in self.manifest.rows() {
            if self.stored_blob(&row.path).is_some() {
                continue;
            }
            let bytes = match base.read(&row.path) {
                Ok(bytes) => bytes,
                Err(SourceError::Missing { .. }) => {
                    return Err(PatchError::BaseMissing {
                        path: row.path.clone(),
                    })
                }
                Err(err) => return Err(PatchError::Source(err)),
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != row.len {
                return Err(PatchError::BaseLength {
                    path: row.path.clone(),
                    expected: row.len,
                    actual: bytes.len(),
                });
            }
            let actual = hash_bytes(&bytes);
            if actual != row.hash {
                return Err(PatchError::BaseHash {
                    path: row.path.clone(),
                    expected: row.hash,
                    actual,
                });
            }
        }
        Ok(())
    }

    /// Apply the patch over `base`: a source that serves a patched path from
    /// this patch and every other path from the base.
    pub fn apply<'a>(&'a self, base: &'a dyn AssetSource) -> PatchedSource<'a> {
        PatchedSource { patch: self, base }
    }
}

/// Hash that guards the header fields and the manifest.
fn framing_hash(count: u32, manifest: &[u8]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(PATCH_VERSION);
    hasher.write_u32(count);
    hasher.write_u64(u64::try_from(manifest.len()).unwrap_or(u64::MAX));
    hasher.write(manifest);
    hasher.finish()
}

/// Header fields plus the manifest slice they guard.
struct Header<'a> {
    count: usize,
    manifest: &'a [u8],
    data_start: usize,
}

fn parse_header(bytes: &[u8]) -> Result<Header<'_>, PatchError> {
    let mut cur = Cursor::new(bytes);
    if cur.take(PATCH_MAGIC.len())? != PATCH_MAGIC {
        return Err(bad_patch("magic is not E2PATCH"));
    }
    let version = cur.u32()?;
    if version != PATCH_VERSION {
        return Err(bad_patch(&format!(
            "version {version} unreadable (this reader reads {PATCH_VERSION})"
        )));
    }
    let count32 = cur.u32()?;
    let manifest_len = cur.u64()?;
    let framing = cur.u64()?;
    let count = usize::try_from(count32).unwrap_or(usize::MAX);
    if count > MAX_PATCH_ENTRIES {
        return Err(bad_patch(&format!(
            "{count} rows is over the {MAX_PATCH_ENTRIES} cap"
        )));
    }
    let data_start = usize::try_from(manifest_len)
        .ok()
        .and_then(|len| HEADER_LEN.checked_add(len))
        .ok_or_else(|| bad_patch("manifest length is out of range"))?;
    let manifest = bytes.get(HEADER_LEN..data_start).ok_or(Short {
        want: data_start,
        got: bytes.len(),
    })?;
    if framing_hash(count32, manifest) != framing {
        return Err(bad_patch(
            "header or manifest damaged (manifest hash mismatch)",
        ));
    }
    Ok(Header {
        count,
        manifest,
        data_start,
    })
}

/// Parse the manifest rows in path order; returns each row and its stored flag.
fn parse_manifest(count: usize, bytes: &[u8]) -> Result<Vec<(ManifestRow, bool)>, PatchError> {
    let ended = |n: usize| bad_patch(&format!("manifest ends inside row {n}"));
    let mut cur = Cursor::new(bytes);
    let mut rows: Vec<(ManifestRow, bool)> = Vec::with_capacity(count);
    for n in 0..count {
        let path_len = usize::from(cur.u16().map_err(|_| ended(n))?);
        let raw = cur.take(path_len).map_err(|_| ended(n))?;
        let path = std::str::from_utf8(raw)
            .map_err(|_| bad_patch(&format!("row {n} path is not UTF-8")))?;
        validate_path(path)?;
        if rows
            .last()
            .is_some_and(|(prev, _)| prev.path.as_str() >= path)
        {
            return Err(bad_patch(&format!(
                "manifest not strictly sorted at {path:?}"
            )));
        }
        let stored = match cur.u8().map_err(|_| ended(n))? {
            0 => false,
            1 => true,
            other => return Err(bad_patch(&format!("row {path:?} stored flag {other}"))),
        };
        let len = cur.u64().map_err(|_| ended(n))?;
        let hash = cur.u64().map_err(|_| ended(n))?;
        rows.push((
            ManifestRow {
                path: path.to_string(),
                len,
                hash,
            },
            stored,
        ));
    }
    if cur.pos() != bytes.len() {
        return Err(bad_patch("manifest has bytes after its last row"));
    }
    Ok(rows)
}

/// A base source with a patch applied: the patched files come from the patch,
/// every other file comes from the base once its bytes match the manifest.
#[derive(Clone, Copy)]
pub struct PatchedSource<'a> {
    patch: &'a PatchSet,
    base: &'a dyn AssetSource,
}

impl PatchedSource<'_> {
    /// The manifest the patched source serves: the patch's new content.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        self.patch.manifest()
    }

    /// True when this path's bytes come from the patch, not the base.
    #[must_use]
    pub fn patched(&self, path: &str) -> bool {
        self.patch.stored_blob(path).is_some()
    }
}

impl AssetSource for PatchedSource<'_> {
    fn read(&self, path: &str) -> Result<Cow<'_, [u8]>, SourceError> {
        validate_path(path)?;
        let row = self
            .patch
            .manifest()
            .find(path)
            .ok_or_else(|| SourceError::missing(path))?;
        if let Some(blob) = self.patch.stored_blob(path) {
            return Ok(Cow::Borrowed(blob));
        }
        let bytes = self.base.read(path)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != row.len {
            return Err(SourceError::HashMismatch {
                path: row.path.clone(),
                expected: row.hash,
                actual: hash_bytes(&bytes),
            });
        }
        let actual = hash_bytes(&bytes);
        if actual != row.hash {
            return Err(SourceError::HashMismatch {
                path: row.path.clone(),
                expected: row.hash,
                actual,
            });
        }
        Ok(bytes)
    }

    fn exists(&self, path: &str) -> bool {
        validate_path(path).is_ok() && self.patch.manifest().find(path).is_some()
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, SourceError> {
        if dir.is_empty() {
            return Ok(self
                .patch
                .manifest()
                .rows()
                .iter()
                .map(|row| row.path.clone())
                .collect());
        }
        validate_path(dir)?;
        let prefix = format!("{dir}/");
        let found: Vec<String> = self
            .patch
            .manifest()
            .rows()
            .iter()
            .filter(|row| row.path.starts_with(prefix.as_str()))
            .map(|row| row.path.clone())
            .collect();
        if found.is_empty() {
            return Err(SourceError::missing(dir));
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_source::{bundle::pack_to, BundleSource, MemorySource, PackRules};

    const TEXTURE: &str = "textures/sand.bmp";

    fn base_files() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("placement.json", b"{\"actors\":[]}".to_vec()),
            ("src/crate.gltf", vec![7u8; 48]),
            (TEXTURE, vec![1u8; 32]),
            ("textures/stone.bmp", vec![2u8; 24]),
        ]
    }

    fn source(files: &[(&str, Vec<u8>)]) -> MemorySource {
        let mut source = MemorySource::new();
        for (path, bytes) in files {
            source.insert(path, bytes.clone()).expect("logical path");
        }
        source
    }

    /// Base files with the sand texture changed to a 40-byte version.
    fn changed_texture() -> Vec<(&'static str, Vec<u8>)> {
        let mut files = base_files();
        files[2].1 = vec![9u8; 40];
        files
    }

    /// A patch from `base` onto `new`, the base source, and the build stats.
    fn patched(
        base: &[(&str, Vec<u8>)],
        new: &[(&str, Vec<u8>)],
    ) -> (PatchSet, MemorySource, PatchStats) {
        let base = source(base);
        let manifest = Manifest::from_source(&base).expect("base survey");
        let (patch, stats) = PatchSet::build(&manifest, &source(new)).expect("patch builds");
        (patch, base, stats)
    }

    #[test]
    fn a_patch_carries_only_the_changed_blob_and_applies_byte_for_byte() {
        let base_list = base_files();
        let new_list = changed_texture();
        let (patch, base, stats) = patched(&base_list, &new_list);
        assert_eq!(stats.unchanged, 3);
        assert_eq!(stats.changed, 1);
        assert_eq!((stats.added, stats.removed), (0, 0));
        assert_eq!((stats.entries, stats.blobs), (4, 1));
        assert_eq!(stats.blob_bytes, 40);
        assert_eq!(
            stats.patch_bytes,
            patch.to_bytes().expect("patch bytes").len()
        );
        assert_eq!(
            stats.digest,
            hash_bytes(&patch.to_bytes().expect("patch bytes"))
        );
        assert!(stats.patch_bytes > stats.blob_bytes, "header and manifest");
        assert_eq!(patch.stored_paths().collect::<Vec<_>>(), [TEXTURE]);

        let reloaded =
            PatchSet::parse(&patch.to_bytes().expect("patch bytes")).expect("patch parses");
        assert_eq!(reloaded.manifest(), patch.manifest());
        assert_eq!(reloaded.stored_paths().collect::<Vec<_>>(), [TEXTURE]);

        let patched = reloaded.apply(&base);
        assert_eq!(
            patched.list("").expect("list"),
            source(&new_list).list("").expect("list")
        );
        for (path, bytes) in &new_list {
            assert_eq!(
                patched.read(path).expect("patched read").as_ref(),
                bytes.as_slice(),
                "{path}"
            );
        }
        for (path, bytes) in &base_list {
            if *path == TEXTURE {
                assert!(patched.patched(path), "the changed file rides the patch");
                assert_eq!(
                    patched.read(path).expect("patched read").as_ref(),
                    new_list[2].1.as_slice(),
                    "{path}"
                );
            } else {
                assert!(!patched.patched(path), "{path} stays in the base");
                assert_eq!(
                    patched.read(path).expect("base read").as_ref(),
                    bytes.as_slice(),
                    "{path}"
                );
            }
        }
    }

    #[test]
    fn a_patch_blob_that_does_not_hash_is_refused_by_name() {
        let (patch, _, _) = patched(&base_files(), &changed_texture());
        let mut bytes = patch.to_bytes().expect("patch bytes");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        match PatchSet::parse(&bytes) {
            Err(PatchError::BlobHash {
                path,
                expected,
                actual,
            }) => {
                assert_eq!(path, TEXTURE);
                assert_ne!(expected, actual);
            }
            other => panic!("want BlobHash, got {other:?}"),
        }
        let text = PatchSet::parse(&bytes)
            .expect_err("still damaged")
            .to_string();
        assert!(text.contains(TEXTURE), "{text}");
        assert!(text.contains("patch blob"), "{text}");

        // A manifest byte flip is framing damage, caught before any blob.
        let mut framed = patch.to_bytes().expect("patch bytes");
        framed[HEADER_LEN + 3] ^= 0xFF;
        assert!(matches!(
            PatchSet::parse(&framed),
            Err(PatchError::BadPatch { .. })
        ));
        // A cut patch names the blob it ends inside; empty input is refused.
        let cut = &bytes[..bytes.len() - 1];
        match PatchSet::parse(cut) {
            Err(PatchError::BadPatch { detail }) => {
                assert!(detail.contains(TEXTURE), "{detail}");
            }
            other => panic!("want BadPatch, got {other:?}"),
        }
        assert!(matches!(
            PatchSet::parse(&[]),
            Err(PatchError::BadPatch { .. })
        ));
    }

    #[test]
    fn a_base_that_moved_is_refused_by_name_and_the_rest_still_loads() {
        let base_list = base_files();
        let new_list = changed_texture();
        let (patch, _, _) = patched(&base_list, &new_list);

        let mut stale_list = base_list.clone();
        stale_list[1].1 = vec![8u8; 48];
        let stale = source(&stale_list);
        let patched = patch.apply(&stale);
        assert_eq!(
            patched.read(TEXTURE).expect("patch blob").as_ref(),
            new_list[2].1.as_slice()
        );
        assert_eq!(
            patched
                .read("placement.json")
                .expect("other base file")
                .as_ref(),
            base_list[0].1.as_slice(),
            "a kept file still loads"
        );
        match patch.verify_base(&stale) {
            Err(PatchError::BaseHash { path, .. }) => assert_eq!(path, "src/crate.gltf"),
            other => panic!("want BaseHash, got {other:?}"),
        }
        match patched.read("src/crate.gltf") {
            Err(SourceError::HashMismatch { path, .. }) => assert_eq!(path, "src/crate.gltf"),
            other => panic!("want HashMismatch, got {other:?}"),
        }

        let mut thin_list = base_list;
        thin_list.remove(0);
        let thin = source(&thin_list);
        match patch.verify_base(&thin) {
            Err(PatchError::BaseMissing { path }) => assert_eq!(path, "placement.json"),
            other => panic!("want BaseMissing, got {other:?}"),
        }
    }

    #[test]
    fn same_base_and_new_content_write_the_same_patch_bytes() {
        let first = patched(&base_files(), &changed_texture());
        let second = patched(&base_files(), &changed_texture());
        assert_eq!(
            first.0.to_bytes().expect("patch bytes"),
            second.0.to_bytes().expect("patch bytes")
        );
        let round_trip =
            PatchSet::parse(&first.0.to_bytes().expect("patch bytes")).expect("parses");
        assert_eq!(
            round_trip.to_bytes().expect("patch bytes"),
            first.0.to_bytes().expect("patch bytes")
        );
    }

    #[test]
    fn a_patch_adds_new_paths_and_drops_removed_ones() {
        let mut new_list = base_files();
        new_list.retain(|(path, _)| *path != "textures/stone.bmp");
        new_list.push(("textures/basalt.bmp", vec![3u8; 12]));
        let base = source(&base_files());
        let manifest = Manifest::from_source(&base).expect("base survey");
        let new = source(&new_list);
        let (patch, stats) = PatchSet::build(&manifest, &new).expect("patch builds");
        assert_eq!(
            (stats.unchanged, stats.changed, stats.added, stats.removed),
            (3, 0, 1, 1)
        );
        assert_eq!(
            patch.stored_paths().collect::<Vec<_>>(),
            ["textures/basalt.bmp"]
        );
        let patched = patch.apply(&base);
        assert_eq!(
            patched.list("").expect("list"),
            new.list("").expect("new list")
        );
        assert_eq!(
            patched.read("textures/basalt.bmp").expect("added").as_ref(),
            [3u8; 12].as_slice()
        );
        assert!(!patched.exists("textures/stone.bmp"), "removed");
        assert_eq!(
            patched.read("textures/stone.bmp").expect_err("removed"),
            SourceError::missing("textures/stone.bmp")
        );
        assert_eq!(patch.stored_bytes(), 12);
    }

    #[test]
    fn a_patch_applies_over_a_packed_base_bundle() {
        let base_list = base_files();
        let new_list = changed_texture();
        let base = source(&base_list);
        let mut bundle_bytes = Vec::new();
        pack_to(&base, &PackRules::new(), &mut bundle_bytes).expect("packs");
        let bundle = BundleSource::open(bundle_bytes).expect("opens");
        let manifest = Manifest::from_source(&bundle).expect("bundle survey");
        let (patch, _) = PatchSet::build(&manifest, &source(&new_list)).expect("patch builds");
        patch.verify_base(&bundle).expect("base matches");
        let patched = patch.apply(&bundle);
        assert_eq!(
            patched.list("").expect("list"),
            source(&new_list).list("").expect("list")
        );
        for (path, bytes) in &new_list {
            assert_eq!(
                patched.read(path).expect("read").as_ref(),
                bytes.as_slice(),
                "{path}"
            );
        }
    }

    #[test]
    fn an_empty_content_tree_has_no_manifest() {
        let empty = MemorySource::new();
        assert!(matches!(
            Manifest::from_source(&empty),
            Err(SourceError::BadPack { .. })
        ));
    }
}
