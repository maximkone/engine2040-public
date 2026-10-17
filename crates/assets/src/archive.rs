//! archive: zero-copy archived bundles over a shared byte mapping.
//!
//! Owner: `engine-assets`; planned, no caller yet (verified 2026-10-16):
//! game and editor startup will load the bundle once through [`map_file`]
//! and serve every entry as a borrowed slice, and the stream queue will bind
//! the same mapping at frame boundaries. Units: payload bytes in and out,
//! open time in whole milliseconds. Same staged entries in the same order
//! always finish to the same bytes. Payloads are opaque: integrity uses
//! FNV-1a64 checks (corruption/truncation detection, never crypto), while
//! digest authority stays with the bundle's blake3 headers. Open timing uses
//! the wall clock at the IO boundary; it never feeds the simulation hash.
//! Opening costs one validation pass with no per-entry copy plus one
//! name-index sort; entry lookups are O(log n) over that index and serve
//! borrowed slices with no allocation.

use super::wire::{bound_chars, push_u16, push_u32, push_u64, Cursor, Short};
use engine_core::hash_bytes;
use std::fmt;
use std::sync::Arc;

/// Archive schema version written by [`ArchiveWriter::finish`].
pub const ARCHIVE_VERSION: u32 = 1;
/// Eight-byte magic opening every archive: `E204ARCV`.
pub const ARCHIVE_MAGIC: &[u8; 8] = b"E204ARCV";
/// Longest entry name in chars; longer names are refused, never cut.
/// Same budget as the bundle so archives wrap bundles 1:1.
pub const MAX_ARCHIVE_NAME_CHARS: usize = 128;
/// Longest entry name accepted on open in bytes (UTF-8 expansion headroom).
/// Same budget as the bundle.
pub const MAX_ARCHIVE_NAME_BYTES: usize = 512;
/// Most entries per archive; bounds table scans.
/// Same budget as the bundle.
pub const MAX_ARCHIVE_ENTRIES: usize = 4096;
/// Largest single payload in bytes.
/// Same budget as the bundle.
pub const MAX_ARCHIVE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
/// Largest whole archive in bytes; bounds the input slice before parsing.
/// Same budget as the bundle.
pub const MAX_ARCHIVE_BYTES: usize = 512 * 1024 * 1024;
/// Bound for echoed names and details carried in [`ArchiveError`].
pub const MAX_ARCHIVE_DETAIL_CHARS: usize = 128;
/// Header bytes before the table of contents: magic plus version plus count.
const ARCHIVE_HEADER_LEN: usize = 16;
/// Trailer bytes after the payload region: FNV-1a64 over all prior bytes.
const ARCHIVE_TRAILER_LEN: usize = 8;

/// Payload kind of one archive entry; one byte on the wire, matching the
/// bundle discriminants so archives wrap bundles without renumbering.
/// Declaration order matches the wire discriminants, so the derived order
/// is the wire order with no numeric cast.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum ArchiveKind {
    /// Opaque cooked-mesh payload bytes.
    Mesh = 1,
    /// Opaque cooked-material payload bytes.
    Material = 2,
    /// Opaque cooked-texture payload bytes.
    Texture = 3,
    /// Opaque skin payload bytes.
    Skin = 4,
    /// Opaque imported-clip payload bytes.
    Clip = 5,
}

impl ArchiveKind {
    /// Decode a wire byte; `None` means the archive is corrupt.
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Mesh),
            2 => Some(Self::Material),
            3 => Some(Self::Texture),
            4 => Some(Self::Skin),
            5 => Some(Self::Clip),
            _ => None,
        }
    }

    /// Lowercase kind name for diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Material => "material",
            Self::Texture => "texture",
            Self::Skin => "skin",
            Self::Clip => "clip",
        }
    }
}

impl fmt::Display for ArchiveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why an archive stage or open failed; every bad input maps to one variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveError {
    /// Entry name empty or over [`MAX_ARCHIVE_NAME_CHARS`]; echo bounded.
    BadName(String),
    /// Entry name already staged; echo bounded.
    DuplicateName(String),
    /// A count or byte length exceeds its budget; offender carried.
    TooLarge {
        /// Offending count or byte length.
        count: usize,
    },
    /// Payload bytes fail structural checks; entry plus bounded reason.
    InvalidData {
        /// Entry name the payload belongs to.
        entry: String,
        /// Bounded reason (never echoes unbounded input).
        detail: String,
    },
    /// First eight bytes are not [`ARCHIVE_MAGIC`].
    BadMagic,
    /// Version newer than [`ARCHIVE_VERSION`]; readers must update, not guess.
    FutureVersion {
        /// Version found in the header.
        found: u32,
    },
    /// Input ends mid-field; bytes needed versus bytes present.
    Truncated {
        /// Bytes needed from the input start.
        want: usize,
        /// Bytes actually present.
        got: usize,
    },
    /// Framing violation with a bounded reason (bad kind, name, offsets).
    Corrupt(String),
    /// Payload checksum differs from its table checksum; entry plus hex.
    DigestMismatch {
        /// Entry name whose payload failed verification.
        entry: String,
        /// Hex checksum recorded in the table of contents.
        expected: String,
        /// Hex checksum recomputed from the payload.
        actual: String,
    },
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName(name) => write!(f, "archive bad entry name {name:?}"),
            Self::DuplicateName(name) => write!(f, "archive duplicate entry {name:?}"),
            Self::TooLarge { count } => write!(f, "archive too large: {count}"),
            Self::InvalidData { entry, detail } => {
                write!(f, "archive entry {entry:?} invalid: {detail}")
            }
            Self::BadMagic => write!(f, "archive bad magic: want E204ARCV"),
            Self::FutureVersion { found } => {
                write!(
                    f,
                    "archive version {found} is newer than v{ARCHIVE_VERSION}"
                )
            }
            Self::Truncated { want, got } => {
                write!(f, "archive truncated: need {want} bytes, got {got}")
            }
            Self::Corrupt(detail) => write!(f, "archive corrupt: {detail}"),
            Self::DigestMismatch {
                entry,
                expected,
                actual,
            } => write!(
                f,
                "archive entry {entry:?} checksum mismatch: want {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for ArchiveError {}

/// Truncate echoed text to at most [`MAX_ARCHIVE_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_ARCHIVE_DETAIL_CHARS)
}

fn invalid(entry: &str, detail: &str) -> ArchiveError {
    ArchiveError::InvalidData {
        entry: bound_detail(entry),
        detail: bound_detail(detail),
    }
}

/// Lowercase 16-char hex of a checksum for digests and diagnostics.
fn hex16(value: u64) -> String {
    format!("{value:016x}")
}

/// Read a file into one shared mapping. Std-only stand-in for an OS mmap:
/// one kernel-to-user copy, then every entry borrows from the mapping with
/// no further copy. A future `memmap2` backend returns the same `Arc<[u8]>`
/// with no format change (orchestrator decision: license row first).
pub fn map_file(path: &std::path::Path) -> std::io::Result<Arc<[u8]>> {
    let bytes = std::fs::read(path)?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

/// Largest finished archive the writer accepts: every wire offset and length
/// is a `u32`, so the whole file must stay inside that range.
const ARCHIVE_OFFSET_LIMIT: usize = u32::MAX as usize;

/// Table-of-contents bytes for one entry: kind, name length, name, offset,
/// length, checksum.
fn table_row_len(name: &str) -> usize {
    (1 + 2 + 4 + 4 + 8usize).saturating_add(name.len())
}

/// True when header, table, payloads and trailer fit within `limit` bytes.
fn archive_fits(table_len: usize, payload_len: usize, limit: usize) -> bool {
    ARCHIVE_HEADER_LEN
        .saturating_add(table_len)
        .saturating_add(payload_len)
        .saturating_add(ARCHIVE_TRAILER_LEN)
        <= limit
}

/// One staged entry: kind plus name plus opaque payload bytes.
#[derive(Clone, Debug)]
struct Staged {
    kind: ArchiveKind,
    name: String,
    payload: Vec<u8>,
}

/// Stages opaque payloads into one versioned archive. Entries keep insertion
/// order so `finish` is deterministic: same entries in the same order always
/// emit the same bytes.
#[derive(Clone, Debug, Default)]
pub struct ArchiveWriter {
    entries: Vec<Staged>,
}

impl ArchiveWriter {
    /// Empty writer: no entries staged yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Entry count staged so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry is staged yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Stage opaque `payload` bytes under `name` with `kind`. Names are
    /// unique per archive so lookups stay unambiguous; the payload is stored
    /// as given (copy once here, borrowed ever after).
    pub fn stage(
        &mut self,
        kind: ArchiveKind,
        name: &str,
        payload: &[u8],
    ) -> Result<(), ArchiveError> {
        self.stage_within(ARCHIVE_OFFSET_LIMIT, kind, name, payload)
    }

    /// `stage` with an explicit cap on the finished archive size, so the
    /// wire offsets (`u32`) can never wrap.
    fn stage_within(
        &mut self,
        limit: usize,
        kind: ArchiveKind,
        name: &str,
        payload: &[u8],
    ) -> Result<(), ArchiveError> {
        if name.is_empty() || name.chars().count() > MAX_ARCHIVE_NAME_CHARS {
            return Err(ArchiveError::BadName(bound_detail(name)));
        }
        if self.entries.iter().any(|entry| entry.name == name) {
            return Err(ArchiveError::DuplicateName(bound_detail(name)));
        }
        if self.entries.len() >= MAX_ARCHIVE_ENTRIES {
            return Err(ArchiveError::TooLarge {
                count: self.entries.len() + 1,
            });
        }
        if payload.len() > MAX_ARCHIVE_PAYLOAD_BYTES {
            return Err(ArchiveError::TooLarge {
                count: payload.len(),
            });
        }
        let table_len = self
            .entries
            .iter()
            .fold(table_row_len(name), |sum, entry| {
                sum.saturating_add(table_row_len(&entry.name))
            });
        let payload_len = self
            .entries
            .iter()
            .fold(payload.len(), |sum, entry| sum.saturating_add(entry.payload.len()));
        if !archive_fits(table_len, payload_len, limit) {
            return Err(ArchiveError::TooLarge {
                count: ARCHIVE_HEADER_LEN
                    .saturating_add(table_len)
                    .saturating_add(payload_len)
                    .saturating_add(ARCHIVE_TRAILER_LEN),
            });
        }
        self.entries.push(Staged {
            kind,
            name: name.to_string(),
            payload: payload.to_vec(),
        });
        Ok(())
    }

    /// Stage every entry of a verified bundle: kind maps 1:1, names are
    /// already unique, payload bytes move over untouched.
    #[cfg(all(feature = "import", feature = "registry"))]
    pub fn stage_bundle(
        &mut self,
        bundle: &super::bundle::LoadedBundle,
    ) -> Result<(), ArchiveError> {
        for entry in &bundle.entries {
            self.stage(entry.kind, &entry.name, &entry.bytes)?;
        }
        Ok(())
    }

    /// Serialize the table of contents plus the payload region plus the
    /// archive checksum; byte-identical for the same staged entries.
    pub fn finish(self) -> Vec<u8> {
        let mut payload_len: usize = 0;
        for entry in &self.entries {
            payload_len = payload_len.saturating_add(entry.payload.len());
        }
        let mut out = Vec::new();
        out.extend_from_slice(ARCHIVE_MAGIC);
        push_u32(&mut out, ARCHIVE_VERSION);
        push_u32(&mut out, self.entries.len() as u32);
        // Payloads start right after the table; offsets are absolute so an
        // opener can slice the mapping without a second pass.
        let mut table_len: usize = 0;
        for entry in &self.entries {
            table_len = table_len.saturating_add(1 + 2 + entry.name.len() + 4 + 4 + 8);
        }
        // `stage` keeps header + table + payloads inside `u32`, so no clamp fires.
        let wire32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let mut offset = ARCHIVE_HEADER_LEN.saturating_add(table_len);
        for entry in &self.entries {
            out.push(entry.kind as u8);
            push_u16(&mut out, entry.name.len() as u16);
            out.extend_from_slice(entry.name.as_bytes());
            push_u32(&mut out, wire32(offset));
            push_u32(&mut out, wire32(entry.payload.len()));
            push_u64(&mut out, hash_bytes(&entry.payload));
            offset = offset.saturating_add(entry.payload.len());
        }
        for entry in &self.entries {
            out.extend_from_slice(&entry.payload);
        }
        let check = hash_bytes(&out);
        push_u64(&mut out, check);
        out
    }
}

/// Table row parsed on open: where one payload lives in the mapping.
#[derive(Clone, Debug)]
struct EntryMeta {
    kind: ArchiveKind,
    name: String,
    offset: usize,
    len: usize,
}

/// One verified archive entry: payload borrowed from the shared mapping,
/// never copied. Copies are `Copy` so serving an entry allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchivedEntry<'a> {
    /// Which payload encoding `bytes` holds.
    pub kind: ArchiveKind,
    /// Unique entry name from the writer.
    pub name: &'a str,
    /// Payload bytes borrowed from the mapping, bit-identical to staged.
    pub bytes: &'a [u8],
}

/// Load accounting for the bundle-versus-archive chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveStats {
    /// Entry count in the archive.
    pub entries: usize,
    /// Whole archive byte length the bundle opened from.
    pub bytes: usize,
    /// Payload bytes served as borrowed slices (never copied on open).
    pub payload_bytes: usize,
    /// Wall-clock time spent parsing plus verifying, whole milliseconds.
    pub open_ms: u64,
}

/// A verified archive over a shared byte mapping: the table is parsed once
/// on [`ArchivedBundle::open`] (one validation pass, no per-entry copy, plus
/// one name-index sort) and every entry afterwards borrows from the mapping.
#[derive(Clone, Debug)]
pub struct ArchivedBundle {
    buffer: Arc<[u8]>,
    index: Vec<EntryMeta>,
    /// Positions into `index` in `(kind, name)` order; `find` binary-searches
    /// this so lookups are O(log n) with no allocation.
    lookup: Vec<usize>,
    open_ms: u64,
}

impl ArchivedBundle {
    /// Parse plus verify `buffer` into an open bundle. Corrupt, truncated
    /// and future-version input is refused; payloads are never copied.
    pub fn open(buffer: Arc<[u8]>) -> Result<Self, ArchiveError> {
        let started = engine_clock::now();
        let index = parse(&buffer)?;
        let lookup = build_lookup(&index);
        Ok(Self {
            buffer,
            index,
            lookup,
            open_ms: engine_clock::since(started).as_millis() as u64,
        })
    }

    /// Copy `bytes` into one shared mapping, then open it. Convenience for
    /// tests and one-shot loads; file startup should use [`map_file`] plus
    /// [`ArchivedBundle::open`] so the mapping is shared, not copied per use.
    pub fn open_slice(bytes: &[u8]) -> Result<Self, ArchiveError> {
        Self::open(Arc::from(bytes.to_vec().into_boxed_slice()))
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True when the archive holds no entries.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Wall-clock open time in whole milliseconds (IO boundary, never hashed).
    pub fn open_ms(&self) -> u64 {
        self.open_ms
    }

    /// Whole archive byte length opened from.
    pub fn bytes(&self) -> usize {
        self.buffer.len()
    }

    /// The shared mapping entries borrow from.
    pub fn buffer(&self) -> &Arc<[u8]> {
        &self.buffer
    }

    /// Chart metadata: entries, archive bytes, borrowed payload bytes, open ms.
    pub fn stats(&self) -> ArchiveStats {
        let mut payload_bytes: usize = 0;
        for meta in &self.index {
            payload_bytes = payload_bytes.saturating_add(meta.len);
        }
        ArchiveStats {
            entries: self.index.len(),
            bytes: self.buffer.len(),
            payload_bytes,
            open_ms: self.open_ms,
        }
    }

    /// Find an entry by kind plus name; names are unique per archive.
    /// O(log n) over the name index built once in [`ArchivedBundle::open`].
    /// The returned slice borrows from the mapping: no copy, no allocation.
    pub fn find(&self, kind: ArchiveKind, name: &str) -> Option<ArchivedEntry<'_>> {
        let want = (kind, name);
        let mut lo = 0usize;
        let mut hi = self.lookup.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let at = self.lookup[mid];
            let meta = &self.index[at];
            let have = (meta.kind, meta.name.as_str());
            if have < want {
                lo = mid + 1;
            } else if have > want {
                hi = mid;
            } else {
                return Some(ArchivedEntry {
                    kind: meta.kind,
                    name: meta.name.as_str(),
                    bytes: &self.buffer[meta.offset..meta.offset + meta.len],
                });
            }
        }
        None
    }

    /// Every entry in writer order, each borrowed from the mapping.
    pub fn iter(&self) -> impl Iterator<Item = ArchivedEntry<'_>> + '_ {
        self.index.iter().map(|meta| ArchivedEntry {
            kind: meta.kind,
            name: meta.name.as_str(),
            bytes: &self.buffer[meta.offset..meta.offset + meta.len],
        })
    }
}

/// Positions into `metas` in `(kind, name)` order for O(log n) `find`.
fn build_lookup(metas: &[EntryMeta]) -> Vec<usize> {
    let mut lookup: Vec<usize> = (0..metas.len()).collect();
    lookup.sort_unstable_by(|&a, &b| {
        (metas[a].kind, metas[a].name.as_str()).cmp(&(metas[b].kind, metas[b].name.as_str()))
    });
    lookup
}

/// Parse the frame, verify the archive checksum plus every entry checksum,
/// return the table in writer order. Reads touch payload bytes but copy none.
fn parse(bytes: &[u8]) -> Result<Vec<EntryMeta>, ArchiveError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(ArchiveError::TooLarge { count: bytes.len() });
    }
    if bytes.len() < ARCHIVE_HEADER_LEN + ARCHIVE_TRAILER_LEN {
        return Err(ArchiveError::Truncated {
            want: ARCHIVE_HEADER_LEN + ARCHIVE_TRAILER_LEN,
            got: bytes.len(),
        });
    }
    let mut reader = Cursor::new(bytes);
    let metas = read_table(&mut reader)?;
    let payload_start = reader.pos();
    let trailer_at = bytes
        .len()
        .checked_sub(ARCHIVE_TRAILER_LEN)
        .ok_or_else(|| ArchiveError::Corrupt("archive shorter than its trailer".to_string()))?;
    if payload_start > trailer_at {
        return Err(ArchiveError::Corrupt(
            "table of contents overruns payload region".to_string(),
        ));
    }
    check_ranges(&metas, payload_start, trailer_at)?;
    // Archive checksum covers every byte before the trailer: a flipped bit
    // anywhere in header, table or payloads refuses here.
    let mut trailer = Cursor::at(bytes, trailer_at);
    let stored_archive = trailer.u64()?;
    if hash_bytes(&bytes[..trailer_at]) != stored_archive {
        return Err(ArchiveError::Corrupt(
            "archive checksum mismatch".to_string(),
        ));
    }
    // Per-entry checksums name the damaged entry for the repair path.
    let mut out = Vec::with_capacity(metas.len());
    for (meta, stored) in metas {
        let payload = &bytes[meta.offset..meta.offset + meta.len];
        let actual = hash_bytes(payload);
        if actual != stored {
            return Err(ArchiveError::DigestMismatch {
                entry: bound_detail(&meta.name),
                expected: hex16(stored),
                actual: hex16(actual),
            });
        }
        out.push(meta);
    }
    Ok(out)
}

/// Read the header plus the table of contents: every entry with its stored
/// checksum, in writer order. Leaves the cursor at the payload region.
fn read_table(reader: &mut Cursor<'_>) -> Result<Vec<(EntryMeta, u64)>, ArchiveError> {
    let magic = reader.take(8)?;
    if magic != ARCHIVE_MAGIC {
        return Err(ArchiveError::BadMagic);
    }
    let version = reader.u32()?;
    if version > ARCHIVE_VERSION {
        return Err(ArchiveError::FutureVersion { found: version });
    }
    if version != ARCHIVE_VERSION {
        return Err(ArchiveError::Corrupt(format!(
            "unsupported archive version {version}"
        )));
    }
    let count = reader.u32()? as usize;
    if count > MAX_ARCHIVE_ENTRIES {
        return Err(ArchiveError::TooLarge { count });
    }
    let mut metas: Vec<(EntryMeta, u64)> = Vec::with_capacity(count);
    for _ in 0..count {
        let kind_byte = reader.u8()?;
        let Some(kind) = ArchiveKind::from_byte(kind_byte) else {
            return Err(ArchiveError::Corrupt(format!(
                "unknown entry kind {kind_byte}"
            )));
        };
        let name_len = usize::from(reader.u16()?);
        if name_len > MAX_ARCHIVE_NAME_BYTES {
            return Err(ArchiveError::Corrupt(format!(
                "entry name too long: {name_len} bytes"
            )));
        }
        let name_bytes = reader.take(name_len)?;
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| ArchiveError::Corrupt("entry name not utf-8".to_string()))?;
        if name.is_empty() || name.chars().count() > MAX_ARCHIVE_NAME_CHARS {
            return Err(ArchiveError::Corrupt("entry name out of range".to_string()));
        }
        if metas.iter().any(|row| row.0.name == name) {
            return Err(ArchiveError::Corrupt(format!(
                "duplicate entry name {}",
                bound_detail(name)
            )));
        }
        let offset = reader.u32()? as usize;
        let len = reader.u32()? as usize;
        if len > MAX_ARCHIVE_PAYLOAD_BYTES {
            return Err(ArchiveError::TooLarge { count: len });
        }
        let stored = reader.u64()?;
        metas.push((
            EntryMeta {
                kind,
                name: name.to_string(),
                offset,
                len,
            },
            stored,
        ));
    }
    Ok(metas)
}

/// Ranges must sit inside the payload region and never overlap, so a crafted
/// table cannot alias the table, the trailer or another payload.
fn check_ranges(
    metas: &[(EntryMeta, u64)],
    payload_start: usize,
    trailer_at: usize,
) -> Result<(), ArchiveError> {
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(metas.len());
    for (meta, _) in metas {
        let end = meta
            .offset
            .checked_add(meta.len)
            .ok_or_else(|| invalid(&meta.name, "entry range overflow"))?;
        if meta.offset < payload_start || end > trailer_at {
            return Err(ArchiveError::Corrupt(format!(
                "entry {} range outside payload region",
                bound_detail(&meta.name)
            )));
        }
        spans.push((meta.offset, end));
    }
    spans.sort_unstable();
    for pair in spans.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(ArchiveError::Corrupt("entry ranges overlap".to_string()));
        }
    }
    Ok(())
}

impl From<Short> for ArchiveError {
    fn from(short: Short) -> Self {
        Self::Truncated {
            want: short.want,
            got: short.got,
        }
    }
}

// Name validation lives behind one choke point (`ArchiveWriter::stage`):
// `stage` refuses bad, duplicate or over-budget entries before anything moves.

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(byte: u8, len: usize) -> Vec<u8> {
        // Deterministic non-trivial bytes: a rotated counter, never all zero
        // so a flipped bit always changes the checksum.
        (0..len).map(|i| byte.wrapping_add(i as u8)).collect()
    }

    fn full_writer() -> ArchiveWriter {
        let mut writer = ArchiveWriter::new();
        writer
            .stage(ArchiveKind::Mesh, "hero", &payload(0x11, 1024))
            .unwrap();
        writer
            .stage(ArchiveKind::Material, "hero-mat", &payload(0x22, 64))
            .unwrap();
        writer
            .stage(ArchiveKind::Texture, "hero-albedo", &payload(0x33, 4096))
            .unwrap();
        writer
            .stage(ArchiveKind::Skin, "hero-skin", &payload(0x44, 512))
            .unwrap();
        writer
            .stage(ArchiveKind::Clip, "hero-step", &payload(0x55, 2048))
            .unwrap();
        writer
    }

    #[test]
    fn stage_refuses_an_archive_past_the_offset_limit() {
        // One entry "a" with 10 payload bytes: 16 header + 20 row + 10 + 8 trailer.
        assert_eq!(table_row_len("a"), 20);
        assert!(archive_fits(20, 10, 54));
        assert!(!archive_fits(20, 10, 53));
        assert!(!archive_fits(usize::MAX, usize::MAX, usize::MAX));
        assert_eq!(ARCHIVE_OFFSET_LIMIT, u32::MAX as usize);

        let mut writer = ArchiveWriter::new();
        let refused = writer.stage_within(53, ArchiveKind::Mesh, "a", &[7; 10]);
        assert_eq!(refused, Err(ArchiveError::TooLarge { count: 54 }));
        assert!(writer.is_empty(), "a refused stage keeps nothing");
        writer
            .stage_within(54, ArchiveKind::Mesh, "a", &[7; 10])
            .unwrap();
        // A second entry adds a row and payload: 16 + 40 + 11 + 8 = 75.
        let refused = writer.stage_within(74, ArchiveKind::Mesh, "b", &[8; 1]);
        assert_eq!(refused, Err(ArchiveError::TooLarge { count: 75 }));
        assert_eq!(writer.len(), 1);
        writer
            .stage_within(75, ArchiveKind::Mesh, "b", &[8; 1])
            .unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes.len(), 75);
        assert!(ArchivedBundle::open_slice(&bytes).is_ok());
    }

    #[test]
    fn round_trip_entries_identical() {
        let bytes = full_writer().finish();
        let open = ArchivedBundle::open_slice(&bytes).unwrap();
        assert_eq!(open.len(), 5);
        assert!(!open.is_empty());
        assert_eq!(open.bytes(), bytes.len());
        for (kind, name, seed) in [
            (ArchiveKind::Mesh, "hero", 0x11),
            (ArchiveKind::Material, "hero-mat", 0x22),
            (ArchiveKind::Texture, "hero-albedo", 0x33),
            (ArchiveKind::Skin, "hero-skin", 0x44),
            (ArchiveKind::Clip, "hero-step", 0x55),
        ] {
            let entry = open.find(kind, name).unwrap();
            assert_eq!(entry.kind, kind);
            assert_eq!(entry.name, name);
            assert_eq!(entry.bytes, payload(seed, entry.bytes.len()).as_slice());
        }
        assert!(open.find(ArchiveKind::Mesh, "hero-mat").is_none());
        assert!(open.find(ArchiveKind::Clip, "missing").is_none());
        let names: Vec<&str> = open.iter().map(|entry| entry.name).collect();
        assert_eq!(
            names,
            vec!["hero", "hero-mat", "hero-albedo", "hero-skin", "hero-step"]
        );
    }

    #[test]
    fn empty_archive_round_trips() {
        let bytes = ArchiveWriter::new().finish();
        assert_eq!(bytes.len(), ARCHIVE_HEADER_LEN + ARCHIVE_TRAILER_LEN);
        let open = ArchivedBundle::open_slice(&bytes).unwrap();
        assert!(open.is_empty());
        assert_eq!(open.len(), 0);
        assert_eq!(open.iter().count(), 0);
    }

    #[test]
    fn empty_payload_entry_round_trips() {
        let mut writer = ArchiveWriter::new();
        writer.stage(ArchiveKind::Clip, "silent", &[]).unwrap();
        let bytes = writer.finish();
        let open = ArchivedBundle::open_slice(&bytes).unwrap();
        assert!(open
            .find(ArchiveKind::Clip, "silent")
            .unwrap()
            .bytes
            .is_empty());
    }

    #[test]
    fn finish_is_deterministic() {
        assert_eq!(full_writer().finish(), full_writer().finish());
        // Order is significant: a different stage order is a different archive.
        let mut swapped = ArchiveWriter::new();
        swapped
            .stage(ArchiveKind::Clip, "hero-step", &payload(0x55, 2048))
            .unwrap();
        swapped
            .stage(ArchiveKind::Mesh, "hero", &payload(0x11, 1024))
            .unwrap();
        assert_ne!(swapped.finish(), {
            let mut writer = ArchiveWriter::new();
            writer
                .stage(ArchiveKind::Mesh, "hero", &payload(0x11, 1024))
                .unwrap();
            writer
                .stage(ArchiveKind::Clip, "hero-step", &payload(0x55, 2048))
                .unwrap();
            writer.finish()
        });
    }

    #[test]
    fn entries_borrow_the_mapping_without_copy() {
        let bytes = full_writer().finish();
        let mapping: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let open = ArchivedBundle::open(Arc::clone(&mapping)).unwrap();
        // Same allocation serves the bundle: no copy on open.
        assert!(Arc::ptr_eq(open.buffer(), &mapping));
        let base = mapping.as_ptr() as usize;
        let end = base + mapping.len();
        for entry in open.iter() {
            let at = entry.bytes.as_ptr() as usize;
            assert!(
                base <= at && at + entry.bytes.len() <= end,
                "entry slice must sit inside the mapping"
            );
        }
        // Two lookups serve the same slice: no per-access copy either.
        let first = open.find(ArchiveKind::Texture, "hero-albedo").unwrap();
        let second = open.find(ArchiveKind::Texture, "hero-albedo").unwrap();
        assert!(std::ptr::eq(first.bytes, second.bytes));
    }

    #[test]
    fn stats_reports_chart_metadata() {
        let bytes = full_writer().finish();
        let open = ArchivedBundle::open_slice(&bytes).unwrap();
        let stats = open.stats();
        assert_eq!(stats.entries, 5);
        assert_eq!(stats.bytes, bytes.len());
        assert_eq!(stats.payload_bytes, 1024 + 64 + 4096 + 512 + 2048);
        assert_eq!(stats.open_ms, open.open_ms());
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = full_writer().finish();
        bytes[0] ^= 0xFF;
        assert_eq!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::BadMagic
        );
    }

    #[test]
    fn future_version_rejected() {
        let mut bytes = full_writer().finish();
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::FutureVersion { found: 2 }
        );
    }

    #[test]
    fn version_zero_rejected_as_corrupt() {
        let mut bytes = full_writer().finish();
        bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn truncated_rejected_at_every_cut() {
        let bytes = full_writer().finish();
        for cut in [0usize, 5, 15, 23, 24, 25, 40, bytes.len() - 1] {
            // Mid-file cuts truncate a field; shaving the trailer instead
            // keeps the frame parseable but breaks ranges/checksums. Both
            // must refuse.
            let err = ArchivedBundle::open_slice(&bytes[..cut]).unwrap_err();
            assert!(
                matches!(
                    err,
                    ArchiveError::Truncated { .. }
                        | ArchiveError::BadMagic
                        | ArchiveError::Corrupt(_)
                ),
                "cut {cut} must refuse, got {err}"
            );
        }
        assert!(ArchivedBundle::open_slice(&[]).is_err());
    }

    #[test]
    fn payload_flip_names_the_damaged_entry() {
        let mut bytes = full_writer().finish();
        // First payload starts right after the table of contents.
        let table_end = table_end(&bytes);
        bytes[table_end] ^= 0xFF;
        let err = ArchivedBundle::open_slice(&bytes).unwrap_err();
        // The archive checksum trips first and refuses the whole file.
        assert!(
            matches!(err, ArchiveError::Corrupt(_)),
            "payload flip must refuse, got {err}"
        );
    }

    #[test]
    fn restamped_payload_flip_names_the_entry() {
        // Re-stamp the archive checksum so framing passes and the per-entry
        // checksum must catch the damage with the entry name attached.
        let mut bytes = full_writer().finish();
        let table_end = table_end(&bytes);
        bytes[table_end] ^= 0xFF;
        restamp(&mut bytes);
        let err = ArchivedBundle::open_slice(&bytes).unwrap_err();
        match err {
            ArchiveError::DigestMismatch {
                entry,
                expected,
                actual,
            } => {
                assert_eq!(entry, "hero");
                assert_eq!(expected.len(), 16);
                assert_eq!(actual.len(), 16);
                assert_ne!(expected, actual);
            }
            other => panic!("want DigestMismatch, got {other}"),
        }
    }

    #[test]
    fn trailer_flip_rejected() {
        let mut bytes = full_writer().finish();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn trailing_byte_rejected() {
        let mut bytes = full_writer().finish();
        bytes.push(0xAA);
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn unknown_kind_rejected() {
        let mut bytes = full_writer().finish();
        bytes[ARCHIVE_HEADER_LEN] = 9;
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn overlapping_ranges_rejected() {
        // Hand-built two-entry archive whose ranges overlap; checksums are
        // valid so the overlap check itself must refuse.
        let first = payload(0x11, 64);
        let second = payload(0x22, 64);
        let table_len = (1 + 2 + 1 + 4 + 4 + 8) * 2;
        let payload_start = ARCHIVE_HEADER_LEN + table_len;
        let bytes = build_raw(vec![
            (ArchiveKind::Mesh, "a", payload_start as u32, first.clone()),
            (
                ArchiveKind::Texture,
                "b",
                (payload_start + 32) as u32,
                second.clone(),
            ),
        ]);
        // Overlapping payload region: concatenate with the overlap laid out.
        let mut bytes = bytes;
        let _ = &mut bytes;
        let err = ArchivedBundle::open_slice(&bytes).unwrap_err();
        assert!(
            matches!(err, ArchiveError::Corrupt(_)),
            "overlap must refuse, got {err}"
        );
    }

    #[test]
    fn range_outside_payload_rejected() {
        let first = payload(0x11, 64);
        let bytes = build_raw(vec![(ArchiveKind::Mesh, "a", u32::MAX, first)]);
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn duplicate_table_name_rejected() {
        let first = payload(0x11, 64);
        let second = payload(0x22, 32);
        let table_len = (1 + 2 + 4 + 4 + 4 + 8) * 2;
        let payload_start = ARCHIVE_HEADER_LEN + table_len;
        let bytes = build_raw(vec![
            (ArchiveKind::Mesh, "same", payload_start as u32, first),
            (
                ArchiveKind::Clip,
                "same",
                (payload_start + 64) as u32,
                second,
            ),
        ]);
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn non_utf8_name_rejected() {
        let mut bytes = full_writer().finish();
        // First table row: kind(1) + name_len(2) then name bytes.
        let at = ARCHIVE_HEADER_LEN + 1 + 2;
        bytes[at] = 0xFF;
        assert!(matches!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::Corrupt(_)
        ));
    }

    #[test]
    fn overcount_header_rejected() {
        let mut bytes = full_writer().finish();
        bytes[12..16].copy_from_slice(&(MAX_ARCHIVE_ENTRIES as u32 + 1).to_le_bytes());
        assert_eq!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::TooLarge {
                count: MAX_ARCHIVE_ENTRIES + 1
            }
        );
    }

    #[test]
    fn oversize_input_rejected() {
        let bytes = vec![0u8; MAX_ARCHIVE_BYTES + 1];
        assert_eq!(
            ArchivedBundle::open_slice(&bytes).unwrap_err(),
            ArchiveError::TooLarge {
                count: MAX_ARCHIVE_BYTES + 1
            }
        );
    }

    #[test]
    fn writer_rejects_bad_and_duplicate_names() {
        let mut writer = ArchiveWriter::new();
        assert!(matches!(
            writer.stage(ArchiveKind::Mesh, "", &[1]).unwrap_err(),
            ArchiveError::BadName(_)
        ));
        let long = "n".repeat(MAX_ARCHIVE_NAME_CHARS + 1);
        assert!(matches!(
            writer.stage(ArchiveKind::Mesh, &long, &[1]).unwrap_err(),
            ArchiveError::BadName(_)
        ));
        writer.stage(ArchiveKind::Mesh, "hero", &[1]).unwrap();
        assert_eq!(
            writer.stage(ArchiveKind::Clip, "hero", &[2]).unwrap_err(),
            ArchiveError::DuplicateName("hero".to_string())
        );
        assert_eq!(writer.len(), 1);
        assert!(!writer.is_empty());
        assert!(ArchiveWriter::new().is_empty());
    }

    #[test]
    fn writer_rejects_over_budget_payload() {
        let mut writer = ArchiveWriter::new();
        let big = vec![0u8; MAX_ARCHIVE_PAYLOAD_BYTES + 1];
        assert_eq!(
            writer
                .stage(ArchiveKind::Texture, "huge", &big)
                .unwrap_err(),
            ArchiveError::TooLarge {
                count: MAX_ARCHIVE_PAYLOAD_BYTES + 1
            }
        );
        assert!(writer.is_empty());
    }

    #[test]
    fn wrong_kind_lookup_misses() {
        let open = ArchivedBundle::open_slice(&full_writer().finish()).unwrap();
        assert!(open.find(ArchiveKind::Clip, "hero").is_none());
    }

    #[test]
    fn kind_labels_and_bytes_round_trip() {
        for (kind, label) in [
            (ArchiveKind::Mesh, "mesh"),
            (ArchiveKind::Material, "material"),
            (ArchiveKind::Texture, "texture"),
            (ArchiveKind::Skin, "skin"),
            (ArchiveKind::Clip, "clip"),
        ] {
            assert_eq!(kind.label(), label);
            assert_eq!(format!("{kind}"), label);
            assert_eq!(ArchiveKind::from_byte(kind as u8), Some(kind));
        }
        assert_eq!(ArchiveKind::from_byte(0), None);
        assert_eq!(ArchiveKind::from_byte(6), None);
    }

    #[test]
    fn errors_display_without_panic() {
        for err in [
            ArchiveError::BadName("x".to_string()),
            ArchiveError::DuplicateName("x".to_string()),
            ArchiveError::TooLarge { count: 1 },
            ArchiveError::InvalidData {
                entry: "e".to_string(),
                detail: "d".to_string(),
            },
            ArchiveError::BadMagic,
            ArchiveError::FutureVersion { found: 9 },
            ArchiveError::Truncated { want: 9, got: 1 },
            ArchiveError::Corrupt("why".to_string()),
            ArchiveError::DigestMismatch {
                entry: "e".to_string(),
                expected: "a".to_string(),
                actual: "b".to_string(),
            },
        ] {
            assert!(!format!("{err}").is_empty());
            assert!(!format!("{err:?}").is_empty());
        }
        assert_eq!(
            format!("{}", ArchiveError::BadMagic),
            "archive bad magic: want E204ARCV"
        );
    }

    /// Wall-clock probe for the load receipt: packs a fixed reference archive
    /// and reports entries, bytes and open milliseconds.
    #[test]
    #[ignore = "timing: archive open wall clock"]
    fn archive_open_timing_probe() {
        let mut writer = ArchiveWriter::new();
        for i in 0..16 {
            writer
                .stage(
                    ArchiveKind::Mesh,
                    &format!("mesh-{i:02}"),
                    &payload(0x11 + i as u8, 4096),
                )
                .unwrap();
        }
        for i in 0..4 {
            writer
                .stage(
                    ArchiveKind::Texture,
                    &format!("tex-{i:02}"),
                    &payload(0x33 + i as u8, 16_384),
                )
                .unwrap();
        }
        writer
            .stage(ArchiveKind::Material, "mat", &payload(0x22, 64))
            .unwrap();
        writer
            .stage(ArchiveKind::Skin, "skin", &payload(0x44, 2048))
            .unwrap();
        writer
            .stage(ArchiveKind::Clip, "clip-a", &payload(0x55, 8192))
            .unwrap();
        writer
            .stage(ArchiveKind::Clip, "clip-b", &payload(0x66, 8192))
            .unwrap();
        let count = writer.len();
        let bytes = writer.finish();
        let mapping: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let open = ArchivedBundle::open(mapping).unwrap();
        assert_eq!(open.len(), count);
        let stats = open.stats();
        println!(
            "archive_open entries:{} bytes:{} payload:{} open_ms:{}",
            stats.entries, stats.bytes, stats.payload_bytes, stats.open_ms
        );
    }

    /// Table end of a writer-built archive: header plus one row per entry.
    fn table_end(bytes: &[u8]) -> usize {
        let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut pos = ARCHIVE_HEADER_LEN;
        for _ in 0..count {
            let name_len = u16::from_le_bytes(bytes[pos + 1..pos + 3].try_into().unwrap()) as usize;
            pos += 1 + 2 + name_len + 4 + 4 + 8;
        }
        pos
    }

    /// Re-stamp the trailer after crafting damage, so framing passes and the
    /// targeted check must catch it.
    fn restamp(bytes: &mut [u8]) {
        let at = bytes.len() - ARCHIVE_TRAILER_LEN;
        let check = hash_bytes(&bytes[..at]);
        bytes[at..].copy_from_slice(&check.to_le_bytes());
    }

    /// Hand-built archive for framing tests: rows carry caller offsets so
    /// overlap and out-of-region tables can be constructed with valid checks.
    fn build_raw(rows: Vec<(ArchiveKind, &str, u32, Vec<u8>)>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(ARCHIVE_MAGIC);
        push_u32(&mut out, ARCHIVE_VERSION);
        push_u32(&mut out, rows.len() as u32);
        // Payload region starts after the table; lay payloads out back to
        // back from there regardless of the declared offsets.
        let mut table_len: usize = 0;
        for (_, name, _, _) in &rows {
            table_len += 1 + 2 + name.len() + 4 + 4 + 8;
        }
        let payload_start = ARCHIVE_HEADER_LEN + table_len;
        for (kind, name, offset, payload) in &rows {
            out.push(*kind as u8);
            push_u16(&mut out, name.len() as u16);
            out.extend_from_slice(name.as_bytes());
            push_u32(&mut out, *offset);
            push_u32(&mut out, payload.len() as u32);
            push_u64(&mut out, hash_bytes(payload));
        }
        assert_eq!(out.len(), payload_start);
        let mut at = payload_start;
        for (_, _, _, payload) in &rows {
            // Overlapping or wild offsets still need backing bytes so the
            // range check (not a truncation) is what refuses.
            if at < out.len() + payload.len() {
                out.resize(at + payload.len(), 0);
            }
            out[at..at + payload.len()].copy_from_slice(payload);
            at += payload.len();
        }
        let check = hash_bytes(&out);
        push_u64(&mut out, check);
        out
    }
}

#[cfg(all(test, all(feature = "import", feature = "registry")))]
mod bridge_tests {
    //! Bundle consumer wiring: archives wrap verified bundles 1:1 and the
    //! load-time chart (bundle copy-load versus archive zero-copy open).

    use super::super::bundle::{BundleLoader, BundleWriter as As1Writer};
    use super::super::cooked::{CookedMaterial, CookedMesh, CookedTexture, Mip};
    use super::super::cooked::{TextureEncoding, TextureRef, TextureSlot};
    use super::super::skin_import::Skin;
    use super::*;
    use engine_anim::pose::QuatTrack;
    use engine_anim::{Clip, Track};
    use std::time::Instant;

    fn triangle(offset: f32) -> CookedMesh {
        CookedMesh {
            positions: vec![
                [offset, 0.0, 0.0],
                [1.0 + offset, 0.0, 0.0],
                [offset, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
        }
    }

    fn material() -> CookedMaterial {
        CookedMaterial {
            base_color_factor: [0.5, 0.25, 0.125, 1.0],
            metallic_factor: 0.2,
            roughness_factor: 0.8,
            textures: vec![TextureRef {
                asset: 11,
                slot: TextureSlot::BaseColor,
            }],
        }
    }

    fn checker() -> CookedTexture {
        CookedTexture {
            width: 2,
            height: 2,
            mips: vec![Mip {
                width: 2,
                height: 2,
                texels: vec![
                    [1.0, 0.0, 0.0, 1.0],
                    [0.0, 1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0, 1.0],
                ],
            }],
            slot: TextureSlot::BaseColor,
            encoding: TextureEncoding::Srgb,
        }
    }

    fn skin() -> Skin {
        Skin {
            joints: vec![[0, 1, 0, 0], [1, 0, 0, 0]],
            weights: vec![[0.7, 0.3, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]],
        }
    }

    fn clip() -> super::super::skin_import::ImportedClip {
        super::super::skin_import::ImportedClip {
            name: "step".to_string(),
            duration: 1.0,
            positions: Clip {
                tracks: vec![Track {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                }],
            },
            rotations: vec![QuatTrack {
                times: vec![0.0, 1.0],
                values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]],
            }],
        }
    }

    fn as1_bundle(num_meshes: usize) -> Vec<u8> {
        let mut writer = As1Writer::new();
        for i in 0..num_meshes {
            writer
                .add_mesh(&format!("mesh-{i:03}"), &triangle(i as f32))
                .unwrap();
        }
        writer.add_material("mat", &material()).unwrap();
        writer.add_texture("albedo", &checker()).unwrap();
        writer.add_skin("skin", &skin()).unwrap();
        writer.add_clip("step", &clip()).unwrap();
        writer.finish()
    }

    #[test]
    fn archive_wraps_verified_bundle_losslessly() {
        let bundle_bytes = as1_bundle(4);
        let loaded = BundleLoader::new().load(&bundle_bytes).unwrap();
        let mut writer = ArchiveWriter::new();
        writer.stage_bundle(&loaded).unwrap();
        assert_eq!(writer.len(), loaded.len());
        let archive_bytes = writer.finish();
        let open = ArchivedBundle::open_slice(&archive_bytes).unwrap();
        assert_eq!(open.len(), loaded.len());
        for entry in &loaded.entries {
            let served = open.find(entry.kind, &entry.name).unwrap();
            assert_eq!(served.bytes, entry.bytes.as_slice());
        }
        // Payloads still decode through the bundle path: the archive moved bytes,
        // never meaning.
        let mesh = open.find(ArchiveKind::Mesh, "mesh-000").unwrap();
        let decoded = loaded
            .find(super::super::bundle::EntryKind::Mesh, "mesh-000")
            .unwrap()
            .decode_mesh()
            .unwrap();
        assert_eq!(decoded, triangle(0.0));
        assert_eq!(mesh.bytes.len(), loaded.entries[0].bytes.len());
    }

    #[test]
    fn stage_bundle_rejects_duplicate_bundle_names() {
        // A bundle can never hold duplicates (its writer refuses), so staging
        // the same loaded bundle twice must refuse on the archive side.
        let loaded = BundleLoader::new().load(&as1_bundle(1)).unwrap();
        let mut writer = ArchiveWriter::new();
        writer.stage_bundle(&loaded).unwrap();
        assert!(matches!(
            writer.stage_bundle(&loaded).unwrap_err(),
            ArchiveError::DuplicateName(_)
        ));
    }

    /// Acceptance chart: bundle copy-load versus archive zero-copy open on
    /// the same reference bundle. Wall clock; run with `-- --ignored`.
    #[test]
    #[ignore = "timing: as1 versus archive load chart"]
    fn as1_vs_archive_chart() {
        let bundle_bytes = as1_bundle(64);
        let bundle_loader = BundleLoader::new();
        let loaded = bundle_loader.load(&bundle_bytes).unwrap();
        let mut writer = ArchiveWriter::new();
        writer.stage_bundle(&loaded).unwrap();
        let archive_bytes = writer.finish();
        let mapping: Arc<[u8]> = Arc::from(archive_bytes.into_boxed_slice());

        let bundle_ms = {
            let started = Instant::now();
            let reloaded = bundle_loader.load(&bundle_bytes).unwrap();
            assert_eq!(reloaded.len(), loaded.len());
            started.elapsed().as_micros()
        };
        let archive_us = {
            let started = Instant::now();
            let open = ArchivedBundle::open(Arc::clone(&mapping)).unwrap();
            assert_eq!(open.len(), loaded.len());
            // Touch every entry the way a startup load does: borrowed slices,
            // no copy. The sum keeps the reads from optimizing away.
            let mut touched: usize = 0;
            for entry in open.iter() {
                touched = touched.wrapping_add(entry.bytes.len());
                assert!(!entry.bytes.is_empty() || entry.name == "step");
            }
            let _ = touched;
            started.elapsed().as_micros()
        };
        let open = ArchivedBundle::open(mapping).unwrap();
        let stats = open.stats();
        println!(
            "chart as1_load_us:{} archive_open_us:{} bundle_bytes:{} archive_bytes:{} payload_bytes:{} entries:{}",
            bundle_ms,
            archive_us,
            bundle_bytes.len(),
            stats.bytes,
            stats.payload_bytes,
            stats.entries
        );
    }
}
