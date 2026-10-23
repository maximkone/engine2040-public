//! bundle: content bundle v1 packing cooked assets for the game and editor.
//!
//! Consumer: the game (`arena.exe`) and the editor load one bundle file at
//! startup through [`BundleLoader`]. Data owner: `engine-assets`. Units:
//! payload bytes in and out, load time in whole milliseconds. Same entries
//! in the same order always finish to the same bytes; every payload carries
//! its blake3 digest (via the `engine-scene` canon) and the loader rejects
//! corrupt, truncated and future-version input with typed [`BundleError`].
//! Load timing uses the wall clock at the IO boundary; it never feeds the
//! simulation hash.

use super::cooked::{
    validate_material, validate_mesh, CookedMaterial, CookedMesh, CookedTexture, Mip,
    TextureEncoding, TextureRef, TextureSlot, MATERIAL_KIND_STONE,
};
use super::mesh_import::MAX_VERTICES;
use super::skin_import::{ImportedClip, Skin, MAX_CLIP_NAME_CHARS, MAX_TRACK_KEYS};
use super::texture::{MAX_MIPS, MAX_TEXTURE_DIM, MAX_TEXTURE_PIXELS};
use super::wire::{bound_chars, push_u16, push_u32, push_u64, Cursor, Short};
use engine_anim::contract::MAX_JOINTS;
use engine_scene::ids::content_digest;
use std::fmt;

/// Bundle schema version written by [`BundleWriter::finish`].
pub const BUNDLE_VERSION: u32 = 1;
/// Eight-byte magic opening every bundle: `E204BNDL`.
pub const BUNDLE_MAGIC: &[u8; 8] = b"E204BNDL";
/// Longest entry name in chars; longer names are refused, never cut.
pub const MAX_NAME_CHARS: usize = 128;
/// Longest entry name accepted on load in bytes (UTF-8 expansion headroom).
pub const MAX_NAME_BYTES: usize = 512;
/// Most entries per bundle; bounds header scans.
pub const MAX_ENTRIES: usize = 4096;
/// Largest single payload in bytes (a max-vertex mesh encodes near 60 MiB).
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
/// Largest whole bundle in bytes; bounds the input slice before parsing.
pub const MAX_BUNDLE_BYTES: usize = 512 * 1024 * 1024;
/// Most indices per mesh payload; the 16 MiB import budget holds at most ~8M.
pub const MAX_BUNDLE_INDICES: usize = 8_000_000;
/// High bit of the mesh index count: the block that follows is FIFO/delta
/// coded ([`index_codec`]) rather than one raw `u32` per index. Counts stay
/// under [`MAX_BUNDLE_INDICES`], so the bit is free and old raw payloads keep
/// decoding with it clear.
const INDEX_CODED_FLAG: u32 = 1 << 31;
/// The index count inside a mesh payload, with the codec flag cleared.
const INDEX_COUNT_MASK: u32 = INDEX_CODED_FLAG - 1;
/// Most material texture refs per payload (three slots exist; headroom kept).
pub const MAX_BUNDLE_REFS: usize = 8;
/// Bound for echoed names and details carried in [`BundleError`].
pub const MAX_DETAIL_CHARS: usize = 128;

/// Payload kind of one bundle entry; one byte on the wire. Archives wrap
/// bundles 1:1, so a bundle entry kind is an archive entry kind: one type
/// owns the discriminants, wire decode and labels (mesh, material, texture,
/// skin, clip: encoded [`CookedMesh`], [`CookedMaterial`], [`CookedTexture`],
/// [`Skin`] and [`ImportedClip`] payloads).
pub use super::archive::ArchiveKind as EntryKind;

/// Why a bundle write or load failed; every bad input maps to one variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// Entry name empty or over [`MAX_NAME_CHARS`]; echo bounded.
    BadName(String),
    /// Entry name already packed; echo bounded.
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
    /// First eight bytes are not [`BUNDLE_MAGIC`].
    BadMagic,
    /// Version newer than [`BUNDLE_VERSION`]; readers must update, not guess.
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
    /// Framing violation with a bounded reason (bad kind, name or trailer).
    Corrupt(String),
    /// Payload digest differs from its header digest; entry plus hex carried.
    DigestMismatch {
        /// Entry name whose payload failed verification.
        entry: String,
        /// Hex digest recorded in the header.
        expected: String,
        /// Hex digest recomputed from the payload.
        actual: String,
    },
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName(name) => write!(f, "bundle bad entry name {name:?}"),
            Self::DuplicateName(name) => write!(f, "bundle duplicate entry {name:?}"),
            Self::TooLarge { count } => write!(f, "bundle too large: {count}"),
            Self::InvalidData { entry, detail } => {
                write!(f, "bundle entry {entry:?} invalid: {detail}")
            }
            Self::BadMagic => write!(f, "bundle bad magic: want E204BNDL"),
            Self::FutureVersion { found } => {
                write!(f, "bundle version {found} is newer than v{BUNDLE_VERSION}")
            }
            Self::Truncated { want, got } => {
                write!(f, "bundle truncated: need {want} bytes, got {got}")
            }
            Self::Corrupt(detail) => write!(f, "bundle corrupt: {detail}"),
            Self::DigestMismatch {
                entry,
                expected,
                actual,
            } => write!(
                f,
                "bundle entry {entry:?} digest mismatch: want {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for BundleError {}

/// Truncate echoed text to at most [`MAX_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_DETAIL_CHARS)
}

impl From<Short> for BundleError {
    fn from(short: Short) -> Self {
        Self::Truncated {
            want: short.want,
            got: short.got,
        }
    }
}

fn invalid(entry: &str, detail: &str) -> BundleError {
    BundleError::InvalidData {
        entry: bound_detail(entry),
        detail: bound_detail(detail),
    }
}

/// One packed entry: kind plus name plus payload; digest is blake3 hex.
#[derive(Clone, Debug)]
struct Packed {
    kind: EntryKind,
    name: String,
    payload: Vec<u8>,
    digest: String,
}

/// Packs cooked assets into one versioned bundle; digests are blake3 hex of
/// each payload. Entries keep insertion order so `finish` is deterministic.
#[derive(Clone, Debug, Default)]
pub struct BundleWriter {
    entries: Vec<Packed>,
}

impl BundleWriter {
    /// Empty writer: no entries packed yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Entry count packed so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry is packed yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Pack a cooked mesh under `name`.
    pub fn add_mesh(&mut self, name: &str, mesh: &CookedMesh) -> Result<(), BundleError> {
        validate_mesh(mesh).map_err(|e| invalid(name, &format!("{e:?}")))?;
        self.stage(EntryKind::Mesh, name, encode_mesh(name, mesh)?)
    }

    /// Pack a cooked material under `name`.
    pub fn add_material(
        &mut self,
        name: &str,
        material: &CookedMaterial,
    ) -> Result<(), BundleError> {
        validate_material(material).map_err(|e| invalid(name, &format!("{e:?}")))?;
        self.stage(EntryKind::Material, name, encode_material(material))
    }

    /// Pack a cooked texture (with its mip chain) under `name`.
    pub fn add_texture(&mut self, name: &str, texture: &CookedTexture) -> Result<(), BundleError> {
        self.stage(EntryKind::Texture, name, encode_texture(texture, name)?)
    }

    /// Pack a validated skin under `name`.
    pub fn add_skin(&mut self, name: &str, skin: &Skin) -> Result<(), BundleError> {
        self.stage(EntryKind::Skin, name, encode_skin(skin, name)?)
    }

    /// Pack a validated clip under `name`.
    pub fn add_clip(&mut self, name: &str, clip: &ImportedClip) -> Result<(), BundleError> {
        self.stage(EntryKind::Clip, name, encode_clip(clip, name)?)
    }

    /// Serialize every entry after the version header; byte-identical for
    /// the same entries added in the same order.
    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(BUNDLE_MAGIC);
        push_u32(&mut out, BUNDLE_VERSION);
        push_u32(&mut out, self.entries.len() as u32);
        for entry in &self.entries {
            out.push(entry.kind as u8);
            push_u16(&mut out, entry.name.len() as u16);
            out.extend_from_slice(entry.name.as_bytes());
            push_u32(&mut out, entry.payload.len() as u32);
            out.extend_from_slice(&entry.payload);
            out.extend_from_slice(entry.digest.as_bytes());
        }
        out
    }

    /// Check the name, digest the payload and stage the entry. Every `add_*`
    /// path funnels through here so a bad or duplicate name stages nothing.
    fn stage(&mut self, kind: EntryKind, name: &str, payload: Vec<u8>) -> Result<(), BundleError> {
        check_name(&self.entries, name)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(BundleError::TooLarge {
                count: payload.len(),
            });
        }
        let digest = content_digest(&payload);
        self.entries.push(Packed {
            kind,
            name: name.to_string(),
            payload,
            digest,
        });
        Ok(())
    }
}

/// Check an entry name before packing: non-empty, at most [`MAX_NAME_CHARS`]
/// chars, unique across the bundle so game and editor lookups stay unambiguous.
fn check_name(staged: &[Packed], name: &str) -> Result<(), BundleError> {
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(BundleError::BadName(bound_detail(name)));
    }
    if staged.iter().any(|entry| entry.name == name) {
        return Err(BundleError::DuplicateName(bound_detail(name)));
    }
    Ok(())
}

/// One verified bundle entry: payload bytes plus their blake3 hex digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleEntry {
    /// Which payload encoding `bytes` holds.
    pub kind: EntryKind,
    /// Unique entry name from the writer.
    pub name: String,
    /// Payload bytes, bit-identical to the writer's encoding.
    pub bytes: Vec<u8>,
    /// Blake3 hex digest of `bytes`, verified on load.
    pub digest: String,
}

impl BundleEntry {
    /// Decode a mesh entry; wrong-kind entries are typed errors, not panics.
    pub fn decode_mesh(&self) -> Result<CookedMesh, BundleError> {
        self.require_kind(EntryKind::Mesh)?;
        decode_mesh(&self.bytes, &self.name)
    }

    /// Decode a material entry.
    pub fn decode_material(&self) -> Result<CookedMaterial, BundleError> {
        self.require_kind(EntryKind::Material)?;
        decode_material(&self.bytes, &self.name)
    }

    /// Decode a texture entry with its mip chain.
    pub fn decode_texture(&self) -> Result<CookedTexture, BundleError> {
        self.require_kind(EntryKind::Texture)?;
        decode_texture(&self.bytes, &self.name)
    }

    /// Decode a skin entry.
    pub fn decode_skin(&self) -> Result<Skin, BundleError> {
        self.require_kind(EntryKind::Skin)?;
        decode_skin(&self.bytes, &self.name)
    }

    /// Decode a clip entry.
    pub fn decode_clip(&self) -> Result<ImportedClip, BundleError> {
        self.require_kind(EntryKind::Clip)?;
        decode_clip(&self.bytes, &self.name)
    }

    fn require_kind(&self, want: EntryKind) -> Result<(), BundleError> {
        if self.kind == want {
            Ok(())
        } else {
            Err(invalid(
                &self.name,
                &format!("want {want} entry, got {}", self.kind),
            ))
        }
    }
}

/// A verified bundle: entries in writer order plus load accounting.
#[derive(Clone, Debug)]
pub struct LoadedBundle {
    /// Verified entries in writer order.
    pub entries: Vec<BundleEntry>,
    /// Wall-clock time spent parsing plus verifying, in whole milliseconds.
    pub load_time_ms: u64,
    /// Input byte length the bundle was loaded from.
    pub bytes: usize,
    /// True when the ship manifest is present; set on load, never refuses.
    pub ship_ready: bool,
}

impl LoadedBundle {
    /// Entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the bundle holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Find an entry by kind plus name; names are unique per bundle.
    pub fn find(&self, kind: EntryKind, name: &str) -> Option<&BundleEntry> {
        self.entries
            .iter()
            .find(|entry| entry.kind == kind && entry.name == name)
    }
}

/// Loads bundle v1 bytes for game and editor startup: verifies the magic,
/// rejects future versions, checks every blake3 digest and records the
/// wall-clock load time in [`LoadedBundle::load_time_ms`].
#[derive(Clone, Copy, Debug, Default)]
pub struct BundleLoader {
    _private: (),
}

impl BundleLoader {
    /// Loader with default budgets; stateless across loads.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Parse plus verify `bytes` into a [`LoadedBundle`].
    pub fn load(&self, bytes: &[u8]) -> Result<LoadedBundle, BundleError> {
        let started = engine_clock::now();
        let entries = parse(bytes)?;
        let ship_ready = verify_ship_entries(&entries).is_ok();
        Ok(LoadedBundle {
            entries,
            load_time_ms: engine_clock::since(started).as_millis() as u64,
            bytes: bytes.len(),
            ship_ready,
        })
    }
}

/// Parse plus verify `bytes` with default budgets (see [`BundleLoader`]).
pub fn load_bundle(bytes: &[u8]) -> Result<LoadedBundle, BundleError> {
    BundleLoader::new().load(bytes)
}

/// Fixed ship manifest: prefab spawn rows the export must include; missing rows fail loudly.
pub const SHIP_MANIFEST: [(EntryKind, &'static str); 5] = [
    (EntryKind::Mesh, "hero"),
    (EntryKind::Material, "hero-mat"),
    (EntryKind::Texture, "hero-albedo"),
    (EntryKind::Skin, "hero-skin"),
    (EntryKind::Clip, "hero-step"),
];

/// Verify the ship manifest over `entries`; loud on a missing dependency. Pure.
pub fn verify_ship_entries(entries: &[BundleEntry]) -> Result<(), BundleError> {
    for (kind, name) in SHIP_MANIFEST {
        let found = entries
            .iter()
            .any(|entry| entry.kind == kind && entry.name == name);
        if !found {
            return Err(invalid(name, "ship manifest missing entry"));
        }
    }
    Ok(())
}

/// Parse the frame, verify every digest, return entries in writer order.
fn parse(bytes: &[u8]) -> Result<Vec<BundleEntry>, BundleError> {
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(BundleError::TooLarge { count: bytes.len() });
    }
    let mut reader = Cursor::new(bytes);
    let magic = reader.take(8)?;
    if magic != BUNDLE_MAGIC {
        return Err(BundleError::BadMagic);
    }
    let version = reader.u32()?;
    if version > BUNDLE_VERSION {
        return Err(BundleError::FutureVersion { found: version });
    }
    if version != BUNDLE_VERSION {
        return Err(BundleError::Corrupt(format!(
            "unsupported bundle version {version}"
        )));
    }
    let count = reader.u32()? as usize;
    if count > MAX_ENTRIES {
        return Err(BundleError::TooLarge { count });
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let kind_byte = reader.u8()?;
        let Some(kind) = EntryKind::from_byte(kind_byte) else {
            return Err(BundleError::Corrupt(format!(
                "unknown entry kind {kind_byte}"
            )));
        };
        let name_len = usize::from(reader.u16()?);
        if name_len > MAX_NAME_BYTES {
            return Err(BundleError::Corrupt(format!(
                "entry name too long: {name_len} bytes"
            )));
        }
        let name_bytes = reader.take(name_len)?;
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| BundleError::Corrupt("entry name not utf-8".to_string()))?;
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(BundleError::Corrupt("entry name out of range".to_string()));
        }
        let payload_len = reader.u32()? as usize;
        if payload_len > MAX_PAYLOAD_BYTES {
            return Err(BundleError::TooLarge { count: payload_len });
        }
        let payload = reader.take(payload_len)?;
        let digest_bytes = reader.take(64)?;
        if digest_bytes.iter().any(|b| !b.is_ascii_hexdigit()) {
            return Err(BundleError::Corrupt("entry digest not hex".to_string()));
        }
        // ASCII hex digest: byte compare is exact string compare here.
        let stored = std::str::from_utf8(digest_bytes)
            .map_err(|_| BundleError::Corrupt("entry digest not hex".to_string()))?;
        let actual = content_digest(payload);
        if stored != actual {
            return Err(BundleError::DigestMismatch {
                entry: bound_detail(name),
                expected: stored.to_string(),
                actual,
            });
        }
        entries.push(BundleEntry {
            kind,
            name: name.to_string(),
            bytes: payload.to_vec(),
            digest: actual,
        });
    }
    if reader.pos() != bytes.len() {
        return Err(BundleError::Corrupt(format!(
            "trailing bytes: {}",
            bytes.len() - reader.pos()
        )));
    }
    Ok(entries)
}

/// Cursor over a verified payload; short reads are [`BundleError::InvalidData`]
/// naming the entry.
struct Payload<'a> {
    cursor: Cursor<'a>,
    len: usize,
    entry: &'a str,
}

impl<'a> Payload<'a> {
    fn new(bytes: &'a [u8], entry: &'a str) -> Self {
        Self {
            cursor: Cursor::new(bytes),
            len: bytes.len(),
            entry,
        }
    }

    fn short(&self) -> BundleError {
        invalid(self.entry, "payload ends mid-field")
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], BundleError> {
        self.cursor.take(len).map_err(|_| self.short())
    }

    /// Every byte left in the payload, consumed in one go.
    fn rest(&mut self) -> Result<&'a [u8], BundleError> {
        let left = self.len.saturating_sub(self.cursor.pos());
        self.take(left)
    }

    /// True when every payload byte was consumed, so an optional trailing
    /// field is absent rather than short.
    fn at_end(&self) -> bool {
        self.cursor.pos() == self.len
    }

    /// Refuse before allocating for `bytes` more bytes the payload cannot hold.
    fn need(&self, bytes: usize) -> Result<(), BundleError> {
        if self.len.saturating_sub(self.cursor.pos()) < bytes {
            return Err(self.short());
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, BundleError> {
        self.cursor.u8().map_err(|_| self.short())
    }

    fn u16(&mut self) -> Result<u16, BundleError> {
        self.cursor.u16().map_err(|_| self.short())
    }

    fn u32(&mut self) -> Result<u32, BundleError> {
        self.cursor.u32().map_err(|_| self.short())
    }

    fn u64(&mut self) -> Result<u64, BundleError> {
        self.cursor.u64().map_err(|_| self.short())
    }

    fn f32(&mut self) -> Result<f32, BundleError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn finish(&self) -> Result<(), BundleError> {
        if self.cursor.pos() != self.len {
            return Err(invalid(self.entry, "payload has trailing bytes"));
        }
        Ok(())
    }
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn slot_byte(slot: TextureSlot) -> u8 {
    match slot {
        TextureSlot::BaseColor => 0,
        TextureSlot::MetallicRoughness => 1,
        TextureSlot::Normal => 2,
        TextureSlot::Occlusion => 3,
        TextureSlot::Emissive => 4,
    }
}

fn slot_of(byte: u8) -> Option<TextureSlot> {
    match byte {
        0 => Some(TextureSlot::BaseColor),
        1 => Some(TextureSlot::MetallicRoughness),
        2 => Some(TextureSlot::Normal),
        3 => Some(TextureSlot::Occlusion),
        4 => Some(TextureSlot::Emissive),
        _ => None,
    }
}

fn encoding_byte(encoding: TextureEncoding) -> u8 {
    match encoding {
        TextureEncoding::Srgb => 0,
        TextureEncoding::Linear => 1,
    }
}

fn encoding_of(byte: u8) -> Option<TextureEncoding> {
    match byte {
        0 => Some(TextureEncoding::Srgb),
        1 => Some(TextureEncoding::Linear),
        _ => None,
    }
}

/// Encode one mesh payload. The index block is FIFO/delta coded when that is
/// smaller than raw `u32`s (the count word carries [`INDEX_CODED_FLAG`]),
/// otherwise raw, so a tiny mesh never grows. The whole block is
/// deterministic: the same mesh gives the same bytes.
fn encode_mesh(name: &str, mesh: &CookedMesh) -> Result<Vec<u8>, BundleError> {
    if mesh.positions.len() > MAX_VERTICES {
        return Err(BundleError::TooLarge {
            count: mesh.positions.len(),
        });
    }
    if mesh.indices.len() > MAX_BUNDLE_INDICES {
        return Err(BundleError::TooLarge {
            count: mesh.indices.len(),
        });
    }
    let mut out = Vec::new();
    push_u32(&mut out, mesh.positions.len() as u32);
    for v in &mesh.positions {
        for c in v {
            push_f32(&mut out, *c);
        }
    }
    for v in &mesh.normals {
        for c in v {
            push_f32(&mut out, *c);
        }
    }
    for v in &mesh.uv0 {
        for c in v {
            push_f32(&mut out, *c);
        }
    }
    for v in &mesh.tangents {
        for c in v {
            push_f32(&mut out, *c);
        }
    }
    let count = u32::try_from(mesh.indices.len()).map_err(|_| BundleError::TooLarge {
        count: mesh.indices.len(),
    })?;
    let coded = super::index_codec::encode(&mesh.indices, mesh.positions.len())
        .map_err(|e| invalid(name, &format!("index block: {e}")))?;
    if coded.len() < mesh.indices.len().saturating_mul(4) {
        push_u32(&mut out, INDEX_CODED_FLAG | count);
        out.extend_from_slice(&coded);
    } else {
        push_u32(&mut out, count);
        for index in &mesh.indices {
            push_u32(&mut out, *index);
        }
    }
    if out.len() > MAX_PAYLOAD_BYTES {
        return Err(BundleError::TooLarge { count: out.len() });
    }
    Ok(out)
}

fn decode_mesh(bytes: &[u8], entry: &str) -> Result<CookedMesh, BundleError> {
    let mut payload = Payload::new(bytes, entry);
    let verts = payload.u32()? as usize;
    if verts == 0 || verts > MAX_VERTICES {
        return Err(invalid(entry, "mesh vertex count out of range"));
    }
    // Positions, normals, uv0 and tangents: (3 + 3 + 2 + 4) f32 per vertex.
    payload.need(verts.saturating_mul(48))?;
    let mut positions = Vec::with_capacity(verts);
    for _ in 0..verts {
        positions.push([payload.f32()?, payload.f32()?, payload.f32()?]);
    }
    let mut normals = Vec::with_capacity(verts);
    for _ in 0..verts {
        normals.push([payload.f32()?, payload.f32()?, payload.f32()?]);
    }
    let mut uv0 = Vec::with_capacity(verts);
    for _ in 0..verts {
        uv0.push([payload.f32()?, payload.f32()?]);
    }
    let mut tangents = Vec::with_capacity(verts);
    for _ in 0..verts {
        tangents.push([
            payload.f32()?,
            payload.f32()?,
            payload.f32()?,
            payload.f32()?,
        ]);
    }
    let word = payload.u32()?;
    let coded = word & INDEX_CODED_FLAG != 0;
    let indices_len = usize::try_from(word & INDEX_COUNT_MASK)
        .map_err(|_| invalid(entry, "mesh index count out of range"))?;
    if indices_len > MAX_BUNDLE_INDICES {
        return Err(invalid(entry, "mesh index count out of range"));
    }
    let indices = if coded {
        let block = payload.rest()?;
        super::index_codec::decode(block, indices_len, verts)
            .map_err(|e| invalid(entry, &format!("index block: {e}")))?
    } else {
        payload.need(indices_len.saturating_mul(4))?;
        let mut raw = Vec::with_capacity(indices_len);
        for _ in 0..indices_len {
            raw.push(payload.u32()?);
        }
        raw
    };
    payload.finish()?;
    let mesh = CookedMesh {
        positions,
        normals,
        uv0,
        tangents,
        indices,
    };
    validate_mesh(&mesh).map_err(|e| invalid(entry, &format!("{e:?}")))?;
    Ok(mesh)
}

fn encode_material(material: &CookedMaterial) -> Vec<u8> {
    let mut out = Vec::new();
    for c in &material.base_color_factor {
        push_f32(&mut out, *c);
    }
    push_f32(&mut out, material.metallic_factor);
    push_f32(&mut out, material.roughness_factor);
    push_u32(&mut out, material.textures.len() as u32);
    for reference in &material.textures {
        push_u64(&mut out, reference.asset);
        out.push(slot_byte(reference.slot));
    }
    out.push(material.kind);
    out
}

fn decode_material(bytes: &[u8], entry: &str) -> Result<CookedMaterial, BundleError> {
    let mut payload = Payload::new(bytes, entry);
    let base_color_factor = [
        payload.f32()?,
        payload.f32()?,
        payload.f32()?,
        payload.f32()?,
    ];
    let metallic_factor = payload.f32()?;
    let roughness_factor = payload.f32()?;
    let ref_count = payload.u32()? as usize;
    if ref_count > MAX_BUNDLE_REFS {
        return Err(invalid(entry, "material ref count out of range"));
    }
    let mut textures = Vec::with_capacity(ref_count);
    for _ in 0..ref_count {
        let asset = payload.u64()?;
        let slot = slot_of(payload.u8()?).ok_or_else(|| invalid(entry, "bad texture slot"))?;
        textures.push(TextureRef { asset, slot });
    }
    // LF8: the kind byte grew after the texture list. A payload cooked
    // before it ends here and loads as stone, so old bundles need no re-cook.
    let kind = if payload.at_end() {
        MATERIAL_KIND_STONE
    } else {
        payload.u8()?
    };
    payload.finish()?;
    let material = CookedMaterial {
        base_color_factor,
        metallic_factor,
        roughness_factor,
        textures,
        kind,
    };
    validate_material(&material).map_err(|e| invalid(entry, &format!("{e:?}")))?;
    Ok(material)
}

fn encode_texture(texture: &CookedTexture, name: &str) -> Result<Vec<u8>, BundleError> {
    if texture.width == 0
        || texture.height == 0
        || texture.width > MAX_TEXTURE_DIM
        || texture.height > MAX_TEXTURE_DIM
    {
        return Err(invalid(name, "texture dimensions out of range"));
    }
    if texture.mips.is_empty() || texture.mips.len() > MAX_MIPS {
        return Err(invalid(name, "texture mip count out of range"));
    }
    let mut out = Vec::new();
    push_u32(&mut out, texture.width);
    push_u32(&mut out, texture.height);
    out.push(slot_byte(texture.slot));
    out.push(encoding_byte(texture.encoding));
    push_u32(&mut out, texture.mips.len() as u32);
    let mut total: usize = 0;
    for mip in &texture.mips {
        if mip.width == 0 || mip.height == 0 {
            return Err(invalid(name, "texture mip dimensions out of range"));
        }
        let want = mip.width as usize * mip.height as usize;
        if mip.texels.len() != want {
            return Err(invalid(name, "texture mip texel count mismatch"));
        }
        total = total.saturating_add(want);
        if total > MAX_TEXTURE_PIXELS {
            return Err(BundleError::TooLarge { count: total });
        }
        push_u32(&mut out, mip.width);
        push_u32(&mut out, mip.height);
        for texel in &mip.texels {
            for c in texel {
                push_f32(&mut out, *c);
            }
        }
    }
    if out.len() > MAX_PAYLOAD_BYTES {
        return Err(BundleError::TooLarge { count: out.len() });
    }
    Ok(out)
}

fn decode_texture(bytes: &[u8], entry: &str) -> Result<CookedTexture, BundleError> {
    let mut payload = Payload::new(bytes, entry);
    let width = payload.u32()?;
    let height = payload.u32()?;
    if width == 0 || height == 0 || width > MAX_TEXTURE_DIM || height > MAX_TEXTURE_DIM {
        return Err(invalid(entry, "texture dimensions out of range"));
    }
    let slot = slot_of(payload.u8()?).ok_or_else(|| invalid(entry, "bad texture slot"))?;
    let encoding = encoding_of(payload.u8()?).ok_or_else(|| invalid(entry, "bad encoding"))?;
    let mip_count = payload.u32()? as usize;
    if mip_count == 0 || mip_count > MAX_MIPS {
        return Err(invalid(entry, "texture mip count out of range"));
    }
    let mut mips = Vec::with_capacity(mip_count);
    let mut total: usize = 0;
    for _ in 0..mip_count {
        let w = payload.u32()?;
        let h = payload.u32()?;
        if w == 0 || h == 0 || w > MAX_TEXTURE_DIM || h > MAX_TEXTURE_DIM {
            return Err(invalid(entry, "texture mip dimensions out of range"));
        }
        let want = w as usize * h as usize;
        total = total.saturating_add(want);
        if total > MAX_TEXTURE_PIXELS {
            return Err(invalid(entry, "texture pixel budget exceeded"));
        }
        payload.need(want.saturating_mul(16))?;
        let mut texels = Vec::with_capacity(want);
        for _ in 0..want {
            let texel = [
                payload.f32()?,
                payload.f32()?,
                payload.f32()?,
                payload.f32()?,
            ];
            if texel.iter().any(|v| !v.is_finite()) {
                return Err(invalid(entry, "non-finite texel"));
            }
            texels.push(texel);
        }
        mips.push(Mip {
            width: w,
            height: h,
            texels,
        });
    }
    payload.finish()?;
    Ok(CookedTexture {
        width,
        height,
        mips,
        slot,
        encoding,
    })
}

fn encode_skin(skin: &Skin, name: &str) -> Result<Vec<u8>, BundleError> {
    if skin.joints.len() != skin.weights.len() {
        return Err(invalid(name, "skin row count mismatch"));
    }
    if skin.joints.len() > MAX_VERTICES {
        return Err(BundleError::TooLarge {
            count: skin.joints.len(),
        });
    }
    let mut out = Vec::new();
    push_u32(&mut out, skin.joints.len() as u32);
    for row in &skin.joints {
        for joint in row {
            push_u16(&mut out, *joint);
        }
    }
    for row in &skin.weights {
        for weight in row {
            push_f32(&mut out, *weight);
        }
    }
    if out.len() > MAX_PAYLOAD_BYTES {
        return Err(BundleError::TooLarge { count: out.len() });
    }
    Ok(out)
}

fn decode_skin(bytes: &[u8], entry: &str) -> Result<Skin, BundleError> {
    let mut payload = Payload::new(bytes, entry);
    let rows = payload.u32()? as usize;
    if rows > MAX_VERTICES {
        return Err(invalid(entry, "skin row count out of range"));
    }
    // Four u16 joints plus four f32 weights per row.
    payload.need(rows.saturating_mul(24))?;
    let mut joints = Vec::with_capacity(rows);
    for _ in 0..rows {
        joints.push([
            payload.u16()?,
            payload.u16()?,
            payload.u16()?,
            payload.u16()?,
        ]);
    }
    let mut weights = Vec::with_capacity(rows);
    for _ in 0..rows {
        let row = [
            payload.f32()?,
            payload.f32()?,
            payload.f32()?,
            payload.f32()?,
        ];
        if row.iter().any(|v| !v.is_finite()) {
            return Err(invalid(entry, "non-finite skin weight"));
        }
        weights.push(row);
    }
    payload.finish()?;
    Ok(Skin { joints, weights })
}

fn encode_clip(clip: &ImportedClip, name: &str) -> Result<Vec<u8>, BundleError> {
    if !clip.duration.is_finite() {
        return Err(invalid(name, "clip duration non-finite"));
    }
    if clip.name.chars().count() > MAX_CLIP_NAME_CHARS {
        return Err(invalid(name, "clip name too long"));
    }
    if clip.positions.tracks.len() > MAX_JOINTS || clip.rotations.len() > MAX_JOINTS {
        return Err(BundleError::TooLarge {
            count: clip.positions.tracks.len().max(clip.rotations.len()),
        });
    }
    let mut out = Vec::new();
    push_u16(&mut out, clip.name.len() as u16);
    out.extend_from_slice(clip.name.as_bytes());
    push_f32(&mut out, clip.duration);
    push_u32(&mut out, clip.positions.tracks.len() as u32);
    for track in &clip.positions.tracks {
        let refusal = "clip position track out of range";
        encode_track(&mut out, &track.times, &track.values, name, refusal)?;
    }
    push_u32(&mut out, clip.rotations.len() as u32);
    for track in &clip.rotations {
        let refusal = "clip rotation track out of range";
        encode_track(&mut out, &track.times, &track.values, name, refusal)?;
    }
    if out.len() > MAX_PAYLOAD_BYTES {
        return Err(BundleError::TooLarge { count: out.len() });
    }
    Ok(out)
}

/// Encode one sampled track: key count, then time plus `N` components per
/// key. `refusal` is the bounded reason when the key arrays disagree or
/// exceed [`MAX_TRACK_KEYS`].
fn encode_track<const N: usize>(
    out: &mut Vec<u8>,
    times: &[f32],
    values: &[[f32; N]],
    name: &str,
    refusal: &str,
) -> Result<(), BundleError> {
    if times.len() != values.len() || times.len() > MAX_TRACK_KEYS {
        return Err(invalid(name, refusal));
    }
    push_u32(out, times.len() as u32);
    for (time, value) in times.iter().zip(values.iter()) {
        push_f32(out, *time);
        for c in value {
            push_f32(out, *c);
        }
    }
    Ok(())
}

fn decode_clip(bytes: &[u8], entry: &str) -> Result<ImportedClip, BundleError> {
    let mut payload = Payload::new(bytes, entry);
    let name_len = usize::from(payload.u16()?);
    if name_len > MAX_NAME_BYTES {
        return Err(invalid(entry, "clip name too long"));
    }
    let name_bytes = payload.take(name_len)?;
    let name =
        std::str::from_utf8(name_bytes).map_err(|_| invalid(entry, "clip name not utf-8"))?;
    if name.chars().count() > MAX_CLIP_NAME_CHARS {
        return Err(invalid(entry, "clip name too long"));
    }
    let duration = payload.f32()?;
    if !duration.is_finite() {
        return Err(invalid(entry, "clip duration non-finite"));
    }
    let pos_count = payload.u32()? as usize;
    if pos_count > MAX_JOINTS {
        return Err(invalid(entry, "clip joint count out of range"));
    }
    let mut positions = engine_anim::Clip {
        tracks: Vec::with_capacity(pos_count),
    };
    for _ in 0..pos_count {
        positions.tracks.push(decode_vec3_track(&mut payload)?);
    }
    let rot_count = payload.u32()? as usize;
    if rot_count > MAX_JOINTS {
        return Err(invalid(entry, "clip joint count out of range"));
    }
    let mut rotations = Vec::with_capacity(rot_count);
    for _ in 0..rot_count {
        rotations.push(decode_quat_track(&mut payload)?);
    }
    payload.finish()?;
    Ok(ImportedClip {
        name: name.to_string(),
        duration,
        positions,
        rotations,
    })
}

fn decode_vec3_track(payload: &mut Payload<'_>) -> Result<engine_anim::Track, BundleError> {
    let entry = payload.entry;
    let keys = payload.u32()? as usize;
    if keys > MAX_TRACK_KEYS {
        return Err(invalid(entry, "clip position track out of range"));
    }
    payload.need(keys.saturating_mul(16))?;
    let mut times = Vec::with_capacity(keys);
    let mut values = Vec::with_capacity(keys);
    for _ in 0..keys {
        let time = payload.f32()?;
        let value = [payload.f32()?, payload.f32()?, payload.f32()?];
        if !time.is_finite() || value.iter().any(|v| !v.is_finite()) {
            return Err(invalid(&entry, "non-finite clip key"));
        }
        times.push(time);
        values.push(value);
    }
    Ok(engine_anim::Track { times, values })
}

fn decode_quat_track(
    payload: &mut Payload<'_>,
) -> Result<engine_anim::pose::QuatTrack, BundleError> {
    let entry = payload.entry;
    let keys = payload.u32()? as usize;
    if keys > MAX_TRACK_KEYS {
        return Err(invalid(entry, "clip rotation track out of range"));
    }
    payload.need(keys.saturating_mul(20))?;
    let mut times = Vec::with_capacity(keys);
    let mut values = Vec::with_capacity(keys);
    for _ in 0..keys {
        let time = payload.f32()?;
        let value = [
            payload.f32()?,
            payload.f32()?,
            payload.f32()?,
            payload.f32()?,
        ];
        if !time.is_finite() || value.iter().any(|v| !v.is_finite()) {
            return Err(invalid(&entry, "non-finite clip key"));
        }
        times.push(time);
        values.push(value);
    }
    Ok(engine_anim::pose::QuatTrack { times, values })
}

// Name validation lives behind one choke point (`BundleWriter::stage`):
// `check_name` refuses bad or duplicate names before anything is staged.

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::pose::QuatTrack;
    use engine_anim::{Clip, Track};

    #[test]
    fn decode_texture_refuses_a_header_the_bytes_cannot_back() {
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 256);
        push_u32(&mut bytes, 256);
        bytes.push(slot_byte(TextureSlot::BaseColor));
        bytes.push(encoding_byte(TextureEncoding::Srgb));
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 256);
        push_u32(&mut bytes, 256);
        // 65,536 texels claimed, none present: refused before any allocation.
        let refused = decode_texture(&bytes, "big").err();
        assert_eq!(refused, Some(invalid("big", "payload ends mid-field")));
    }

    fn triangle() -> CookedMesh {
        CookedMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
        }
    }

    fn textured_material() -> CookedMaterial {
        CookedMaterial {
            base_color_factor: [0.5, 0.25, 0.125, 1.0],
            metallic_factor: 0.2,
            roughness_factor: 0.8,
            textures: vec![
                TextureRef {
                    asset: 11,
                    slot: TextureSlot::BaseColor,
                },
                TextureRef {
                    asset: 22,
                    slot: TextureSlot::Normal,
                },
            ],
            kind: MATERIAL_KIND_STONE,
        }
    }

    fn checker() -> CookedTexture {
        CookedTexture {
            width: 2,
            height: 2,
            mips: vec![
                Mip {
                    width: 2,
                    height: 2,
                    texels: vec![
                        [1.0, 0.0, 0.0, 1.0],
                        [0.0, 1.0, 0.0, 1.0],
                        [0.0, 0.0, 1.0, 1.0],
                        [1.0, 1.0, 1.0, 1.0],
                    ],
                },
                Mip {
                    width: 1,
                    height: 1,
                    texels: vec![[0.5, 0.5, 0.5, 1.0]],
                },
            ],
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

    fn clip() -> ImportedClip {
        ImportedClip {
            name: "step".to_string(),
            duration: 1.0,
            positions: Clip {
                tracks: vec![
                    Track {
                        times: vec![0.0, 1.0],
                        values: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                    },
                    Track {
                        times: vec![0.0],
                        values: vec![[0.0, 1.0, 0.0]],
                    },
                ],
            },
            rotations: vec![
                QuatTrack {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]],
                },
                QuatTrack {
                    times: vec![0.0],
                    values: vec![[0.0, 0.0, 0.0, 1.0]],
                },
            ],
        }
    }

    fn full_writer() -> BundleWriter {
        let mut writer = BundleWriter::new();
        writer.add_mesh("hero", &triangle()).unwrap();
        writer
            .add_material("hero-mat", &textured_material())
            .unwrap();
        writer.add_texture("hero-albedo", &checker()).unwrap();
        writer.add_skin("hero-skin", &skin()).unwrap();
        writer.add_clip("hero-step", &clip()).unwrap();
        writer
    }

    #[test]
    fn round_trip_all_kinds_identical() {
        let bytes = full_writer().finish();
        let loaded = load_bundle(&bytes).unwrap();
        assert_eq!(loaded.len(), 5);
        assert_eq!(loaded.bytes, bytes.len());
        assert!(!loaded.is_empty());

        let mesh = loaded.find(EntryKind::Mesh, "hero").unwrap();
        assert_eq!(mesh.decode_mesh().unwrap(), triangle());
        assert_eq!(mesh.digest, content_digest(&mesh.bytes));

        let material = loaded.find(EntryKind::Material, "hero-mat").unwrap();
        assert_eq!(material.decode_material().unwrap(), textured_material());

        let texture = loaded.find(EntryKind::Texture, "hero-albedo").unwrap();
        assert_eq!(texture.decode_texture().unwrap(), checker());

        let skin_entry = loaded.find(EntryKind::Skin, "hero-skin").unwrap();
        assert_eq!(skin_entry.decode_skin().unwrap(), skin());

        let clip_entry = loaded.find(EntryKind::Clip, "hero-step").unwrap();
        assert_eq!(clip_entry.decode_clip().unwrap(), clip());

        assert!(loaded.find(EntryKind::Mesh, "hero-mat").is_none());
        assert!(loaded.find(EntryKind::Clip, "missing").is_none());
    }

    #[test]
    fn finish_is_deterministic() {
        assert_eq!(full_writer().finish(), full_writer().finish());
    }

    #[test]
    fn material_kind_round_trips_and_an_old_payload_loads_stone() {
        let mut cloth = textured_material();
        cloth.kind = crate::cooked::MATERIAL_KIND_CLOTH;
        assert_eq!(
            decode_material(&encode_material(&cloth), "mat").unwrap(),
            cloth,
            "the kind byte survives the payload"
        );
        // A payload cooked before the kind byte ends after the texture list.
        let mut old = encode_material(&textured_material());
        assert_eq!(old.pop(), Some(MATERIAL_KIND_STONE), "the byte is last");
        let back = decode_material(&old, "mat").unwrap();
        assert_eq!(back, textured_material(), "an old payload loads as stone");
        assert_eq!(back.kind, MATERIAL_KIND_STONE);
        // A payload cut inside the kind byte is short, never silently stone.
        old.pop();
        assert!(decode_material(&old, "mat").is_err());
    }

    #[test]
    fn empty_bundle_round_trips() {
        let bytes = BundleWriter::new().finish();
        assert_eq!(bytes.len(), 16);
        let loaded = load_bundle(&bytes).unwrap();
        assert!(loaded.is_empty());
        assert_eq!(loaded.len(), 0);
        assert_eq!(loaded.bytes, bytes.len());
    }

    #[test]
    fn ship_manifest_present_on_full_bundle() {
        let loaded = load_bundle(&full_writer().finish()).unwrap();
        assert!(loaded.ship_ready);
        verify_ship_entries(&loaded.entries).unwrap();
    }

    #[test]
    fn ship_manifest_missing_fails_loudly() {
        let loaded = load_bundle(&BundleWriter::new().finish()).unwrap();
        assert!(!loaded.ship_ready);
        let err = verify_ship_entries(&loaded.entries).unwrap_err();
        assert!(format!("{err}").contains("hero"), "names culprit: {err}");
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = full_writer().finish();
        bytes[0] ^= 0xFF;
        assert_eq!(load_bundle(&bytes).unwrap_err(), BundleError::BadMagic);
    }

    #[test]
    fn future_version_rejected() {
        let mut bytes = full_writer().finish();
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            load_bundle(&bytes).unwrap_err(),
            BundleError::FutureVersion { found: 2 }
        );
        let mut maxed = full_writer().finish();
        maxed[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            load_bundle(&maxed).unwrap_err(),
            BundleError::FutureVersion { found: u32::MAX }
        );
    }

    #[test]
    fn truncated_rejected_at_every_cut() {
        let bytes = full_writer().finish();
        for cut in [0usize, 5, 15, 16, 17, 24, 40, bytes.len() - 1] {
            let err = load_bundle(&bytes[..cut]).unwrap_err();
            assert!(
                matches!(err, BundleError::Truncated { .. }),
                "cut {cut} must truncate, got {err}"
            );
        }
        assert!(load_bundle(&[]).is_err());
    }

    #[test]
    fn payload_flip_fails_digest_with_entry_name() {
        let mut bytes = full_writer().finish();
        // First payload starts after the 16-byte header plus the entry
        // framing (kind 1 + name_len 2 + name + payload_len 4).
        let name = "hero";
        let at = 16 + 1 + 2 + name.len() + 4;
        bytes[at] ^= 0xFF;
        let err = load_bundle(&bytes).unwrap_err();
        match err {
            BundleError::DigestMismatch {
                entry,
                expected,
                actual,
            } => {
                assert_eq!(entry, name);
                assert_eq!(expected.len(), 64);
                assert_eq!(actual.len(), 64);
                assert_ne!(expected, actual);
            }
            other => panic!("want DigestMismatch, got {other}"),
        }
    }

    #[test]
    fn unknown_kind_and_trailer_rejected() {
        let mut bytes = full_writer().finish();
        bytes[16] = 9;
        assert!(matches!(
            load_bundle(&bytes).unwrap_err(),
            BundleError::Corrupt(_)
        ));

        let mut trailed = full_writer().finish();
        trailed.push(0xAA);
        assert!(matches!(
            load_bundle(&trailed).unwrap_err(),
            BundleError::Corrupt(_)
        ));
    }

    #[test]
    fn writer_rejects_bad_and_duplicate_names() {
        let mut writer = BundleWriter::new();
        assert!(matches!(
            writer.add_mesh("", &triangle()).unwrap_err(),
            BundleError::BadName(_)
        ));
        let long = "n".repeat(MAX_NAME_CHARS + 1);
        assert!(matches!(
            writer.add_mesh(&long, &triangle()).unwrap_err(),
            BundleError::BadName(_)
        ));
        writer.add_mesh("hero", &triangle()).unwrap();
        assert_eq!(
            writer.add_mesh("hero", &triangle()).unwrap_err(),
            BundleError::DuplicateName("hero".to_string())
        );
        // Nothing staged by the refused writes except the first entry.
        assert_eq!(writer.len(), 1);
        assert!(!writer.is_empty());
    }

    #[test]
    fn writer_rejects_invalid_mesh() {
        let mut writer = BundleWriter::new();
        let mut bad = triangle();
        bad.normals.pop();
        let err = writer.add_mesh("bad", &bad).unwrap_err();
        assert!(
            matches!(err, BundleError::InvalidData { .. }),
            "want InvalidData, got {err}"
        );
        assert!(writer.is_empty());
    }

    #[test]
    fn wrong_kind_decode_rejected() {
        let loaded = load_bundle(&full_writer().finish()).unwrap();
        let mesh = loaded.find(EntryKind::Mesh, "hero").unwrap();
        assert!(matches!(
            mesh.decode_clip().unwrap_err(),
            BundleError::InvalidData { .. }
        ));
        assert!(matches!(
            mesh.decode_material().unwrap_err(),
            BundleError::InvalidData { .. }
        ));
    }

    #[test]
    fn garbage_payload_rejected_as_invalid() {
        // A well-framed entry with structural garbage still decodes typed.
        let mut writer = BundleWriter::new();
        writer.add_mesh("hero", &triangle()).unwrap();
        let mut bytes = writer.finish();
        // Corrupt the vertex count to an out-of-range value, then re-stamp
        // the digest so framing passes and the payload check must catch it.
        let at = 16 + 1 + 2 + "hero".len() + 4;
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let payload_len = u32::from_le_bytes(bytes[at - 4..at].try_into().unwrap()) as usize;
        let payload = &bytes[at..at + payload_len];
        let digest = content_digest(payload);
        let digest_at = at + payload_len;
        bytes[digest_at..digest_at + 64].copy_from_slice(digest.as_bytes());
        let loaded = load_bundle(&bytes).unwrap();
        let entry = loaded.find(EntryKind::Mesh, "hero").unwrap();
        assert!(matches!(
            entry.decode_mesh().unwrap_err(),
            BundleError::InvalidData { .. }
        ));
    }

    /// The pre-codec mesh payload: raw `f32` channels, then raw `u32` indices.
    /// Kept in the test so the old layout stays provable, not assumed.
    fn raw_mesh_payload(mesh: &CookedMesh) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, u32::try_from(mesh.positions.len()).unwrap());
        for channel in [&mesh.positions, &mesh.normals] {
            for vertex in channel {
                for c in vertex {
                    push_f32(&mut out, *c);
                }
            }
        }
        for vertex in &mesh.uv0 {
            for c in vertex {
                push_f32(&mut out, *c);
            }
        }
        for vertex in &mesh.tangents {
            for c in vertex {
                push_f32(&mut out, *c);
            }
        }
        push_u32(&mut out, u32::try_from(mesh.indices.len()).unwrap());
        for index in &mesh.indices {
            push_u32(&mut out, *index);
        }
        out
    }

    #[test]
    fn the_real_kit_meshes_re_encode_smaller_and_decode_the_same() {
        // Two real kit files cooked through the import lane: real geometry,
        // real triangle counts, no synthetic stand-in.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena/src");
        let mut meshes = 0usize;
        let mut raw_index_bytes = 0usize;
        let mut coded_index_bytes = 0usize;
        let mut old_payloads = 0usize;
        let mut new_payloads = 0usize;
        for file in ["crate.gltf", "walls.gltf"] {
            let path = root.join(file);
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("read the committed {}: {e}", path.display()));
            let piece = crate::kit_import::import_kit_gltf(file, &bytes)
                .unwrap_or_else(|e| panic!("{file} cooks: {e}"));
            let mesh = piece.mesh;
            assert!(mesh.indices.len() > 3, "{file} is a real mesh");
            let old = raw_mesh_payload(&mesh);
            let new = encode_mesh(file, &mesh).unwrap();
            // Old raw payloads keep decoding (a bundle cooked before the
            // codec still loads), and the new block decodes to the same mesh.
            assert_eq!(decode_mesh(&old, file).unwrap(), mesh, "{file} raw decodes");
            assert_eq!(
                decode_mesh(&new, file).unwrap(),
                mesh,
                "{file} coded decodes the same"
            );
            let index_at = 4 + mesh.positions.len() * 48;
            let new_word = u32::from_le_bytes(new[index_at..index_at + 4].try_into().unwrap());
            assert_ne!(new_word & INDEX_CODED_FLAG, 0, "{file} chose the codec");
            assert!(
                new.len() < old.len(),
                "{file} grew from {} to {} B",
                old.len(),
                new.len()
            );
            meshes += 1;
            raw_index_bytes += mesh.indices.len() * 4;
            coded_index_bytes += new.len() - index_at - 4;
            old_payloads += old.len();
            new_payloads += new.len();
        }
        assert_eq!(meshes, 2);
        assert!(new_payloads < old_payloads);
        println!(
            "index_codec kit meshes:{meshes} raw_index:{raw_index_bytes} coded_index:{coded_index_bytes} payloads:{old_payloads}->{new_payloads}"
        );
    }

    #[test]
    fn a_corrupt_coded_index_block_is_refused() {
        let mut writer = BundleWriter::new();
        writer.add_mesh("hero", &triangle()).unwrap();
        let mut bytes = writer.finish();
        let at = 16 + 1 + 2 + "hero".len() + 4;
        let payload_len =
            usize::try_from(u32::from_le_bytes(bytes[at - 4..at].try_into().unwrap())).unwrap();
        // Triangle: 3 vertices, then the flagged count word, then one token
        // per index. Break the first token; re-stamp the digest so framing
        // passes and the index block itself must refuse.
        let first_token = at + 4 + 3 * 48 + 4;
        assert!(first_token < at + payload_len);
        bytes[first_token] = 0x7f;
        let payload = &bytes[at..at + payload_len];
        let digest = content_digest(payload);
        bytes[at + payload_len..at + payload_len + 64].copy_from_slice(digest.as_bytes());
        let loaded = load_bundle(&bytes).unwrap();
        let entry = loaded.find(EntryKind::Mesh, "hero").unwrap();
        assert!(matches!(
            entry.decode_mesh().unwrap_err(),
            BundleError::InvalidData { .. }
        ));
    }

    #[test]
    fn errors_display_without_panic() {
        for err in [
            BundleError::BadName("x".to_string()),
            BundleError::DuplicateName("x".to_string()),
            BundleError::TooLarge { count: 1 },
            BundleError::InvalidData {
                entry: "e".to_string(),
                detail: "d".to_string(),
            },
            BundleError::BadMagic,
            BundleError::FutureVersion { found: 9 },
            BundleError::Truncated { want: 9, got: 1 },
            BundleError::Corrupt("why".to_string()),
            BundleError::DigestMismatch {
                entry: "e".to_string(),
                expected: "a".to_string(),
                actual: "b".to_string(),
            },
        ] {
            assert!(!format!("{err}").is_empty());
            assert!(!format!("{err:?}").is_empty());
        }
        assert_eq!(
            format!("{}", BundleError::BadMagic),
            "bundle bad magic: want E204BNDL"
        );
    }

    /// Wall-clock probe for the load receipt: packs a fixed reference bundle
    /// and reports entries, bytes and load milliseconds.
    #[test]
    #[ignore = "timing: bundle load wall clock"]
    fn bundle_load_timing_probe() {
        let mut writer = BundleWriter::new();
        for i in 0..16 {
            writer
                .add_mesh(&format!("mesh-{i:02}"), &triangle())
                .unwrap();
        }
        for i in 0..4 {
            writer
                .add_texture(&format!("tex-{i:02}"), &checker())
                .unwrap();
        }
        writer.add_material("mat", &textured_material()).unwrap();
        writer.add_skin("skin", &skin()).unwrap();
        writer.add_clip("clip-a", &clip()).unwrap();
        writer.add_clip("clip-b", &clip()).unwrap();
        let count = writer.len();
        let bytes = writer.finish();
        let loader = BundleLoader::new();
        let loaded = loader.load(&bytes).unwrap();
        assert_eq!(loaded.len(), count);
        println!(
            "bundle_load entries:{} bytes:{} load_ms:{}",
            loaded.len(),
            loaded.bytes,
            loaded.load_time_ms
        );
    }
}
