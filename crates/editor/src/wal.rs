//! Crash-safe journal: a write-ahead log over the command service.
//!
//! Status: test-only with no production Shell caller yet; the live gizmo
//! path uses [`begin_group`]/[`end_group`]. [`wal_apply`], [`wal_undo`],
//! [`wal_redo`] and the group wrappers keep every acknowledged edit in
//! `journal.wal`; [`Wal::open`] replays to the last good state and
//! [`Wal::checkpoint`] seals a snapshot plus the full undo/redo stacks.
//! Data owner: [`Wal`] owns the two files in its directory only.
//! Units: file offsets and lengths are bytes; stamps are fixed ticks plus
//! monotonic editor revisions. Determinism: little-endian frames in append
//! order, hyphenated ids, CRC32 per frame; replay is a pure fold, so the
//! same files always reopen to the same document and journal.
//! Errors: [`WalError`]; torn tails truncate to the last good frame, damaged
//! snapshots refuse loudly. Budget: one bounded frame per acknowledged edit
//! (see `wal_frame_size_bounded`); no steady-state work beyond the edit path.

use std::fmt;
use std::path::{Path, PathBuf};

use engine_scene::ids::ObjectId;
use engine_scene::provenance::Provenance;

use crate::commands::{
    apply_named, begin_group, cancel_group, end_group, open_group_depth, open_group_id,
    open_group_key, redo, undo, Batch, BondStrength, CommandMeta, Document, Journal, JournalEntry,
    MaterialName, Op, ReceiptSource, MAX_COMMAND_NAME_LEN,
};
use crate::durable::TMP_SUFFIX;
use crate::save::{
    crc32, load_from_bytes, push_id, push_transform, save_to_bytes, SaveError, MAX_SAVE_BYTES,
};

/// Log file name inside the [`Wal`] directory.
pub const WAL_FILE_NAME: &str = "journal.wal";
/// Snapshot file name inside the [`Wal`] directory.
pub const SNAP_FILE_NAME: &str = "snapshot.wal";
/// Log frame magic (`EEDW`: editor WAL). Distinct from `EDOC` saves and
/// `EEDJ` durable generations.
pub const WAL_MAGIC: [u8; 4] = *b"EEDW";
/// Snapshot magic (`EESP`: editor snapshot).
pub const SNAP_MAGIC: [u8; 4] = *b"EESP";
/// Newest log/snapshot layout this module writes and reads. Version 2 appends
/// the three content digests to every snapshot journal entry; version 1
/// snapshots still open with zero digests (unknown, not invented). Log frames
/// are unchanged between the two.
pub const WAL_VERSION: u32 = 2;
/// Oldest log/snapshot layout this module still opens.
pub const MIN_WAL_VERSION: u32 = 1;
/// Largest accepted frame payload: one 1 MiB batch (~7k single ops) is far
/// past any real edit; bigger batches refuse before touching the document.
pub const MAX_FRAME_BYTES: usize = 1 << 20;
/// Header bytes: magic (4) + version (4) + snapshot revision (8) + end mark
/// (8) + header CRC (4).
pub const HEADER_LEN: usize = 28;
/// Cap for one decoded string (names, stamp texts); effects lines allow more
/// (see decode). Bounds every allocation the decoder performs.
const MAX_SHORT_TEXT: usize = 1_024;
/// Cap for one effects line.
const MAX_EFFECT_TEXT: usize = 4_096;
/// Cap for ops in one frame, entries per snapshot stack, effects per entry.
const MAX_OPS: usize = 65_536;
/// Cap for snapshot journal stacks (live stacks cap at 100 each).
const MAX_SNAP_ENTRIES: usize = 4_096;

/// Frame kinds (SQLite WAL pattern: redo log of committed mutations).
const KIND_APPLY: u8 = 1;
const KIND_UNDO: u8 = 2;
const KIND_REDO: u8 = 3;
const KIND_BEGIN_GROUP: u8 = 4;
const KIND_END_GROUP: u8 = 5;
const KIND_CANCEL_GROUP: u8 = 6;

/// Stamp on every logged edit: the caller's fixed tick plus the editor
/// revision the edit published.
///
/// This mirrors the `engine-ai` record shape: `tick` is the `observed_tick`
/// analog (the fixed clock the edit was witnessed on; 0 when the caller has
/// no clock) and `revision` is the recorded-order analog (here exactly the
/// [`Document`] revision after apply, dense like the record sequence). It
/// stays a two-field projection instead of naming the `engine-ai` record
/// type because that crate cannot enter the headless editor graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalStamp {
    /// Fixed tick the edit was witnessed on (caller clock, 0 when none).
    pub tick: u64,
    /// Editor revision the edit published (`base_revision + 1`).
    pub revision: u64,
}

/// What rejected a WAL operation. IO and batch failures pass through with
/// their own types; damaged containers refuse without guessing.
#[derive(Debug)]
pub enum WalError {
    /// Filesystem failure; the in-memory document and journal are unchanged
    /// (wrappers truncate or roll back the unlogged mutation first).
    Io(std::io::Error),
    /// The batch itself was refused; nothing was logged, nothing ran.
    Batch(crate::commands::BatchError),
    /// The snapshot document payload failed to decode.
    Snapshot(SaveError),
    /// Truncated frame, CRC mismatch, bad discriminant or over-cap length:
    /// replay stops before the bad frame, the snapshot refuses outright.
    Corrupt,
    /// A frame, snapshot or text exceeds its bound; nothing was logged.
    TooLarge,
    /// [`Wal::checkpoint`] with a command group still open; close the drag
    /// (end or cancel) and checkpoint again.
    OpenGroup,
    /// Compensation truncate after a refused edit failed; the log may still
    /// hold the refused frame, so the caller must treat the WAL as ahead.
    Truncate(std::io::Error),
    /// Refused edit plus a failed compensation truncate: the batch cause
    /// plus the cleanup failure, so neither is swallowed.
    BatchTruncate {
        /// Why the edit was refused.
        batch: crate::commands::BatchError,
        /// Why the compensation truncate failed.
        cleanup: std::io::Error,
    },
    /// An open group id without its retry key (in-memory invariant broke);
    /// replaces the former panic with a typed refusal.
    OrphanGroup,
    /// Layout newer than [`WAL_VERSION`]; refuses instead of guessing.
    UnsupportedVersion {
        /// Version stamped on the bytes.
        got: u32,
    },
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "editor wal io: {error}"),
            Self::Batch(error) => write!(f, "editor wal batch: {error}"),
            Self::Snapshot(error) => write!(f, "editor wal snapshot: {error}"),
            Self::Corrupt => f.write_str("editor wal: corrupt frame or snapshot"),
            Self::TooLarge => f.write_str("editor wal: entry exceeds its size bound"),
            Self::OpenGroup => f.write_str("editor wal: checkpoint with an open group"),
            Self::Truncate(error) => write!(f, "editor wal truncate: {error}"),
            Self::BatchTruncate { batch, cleanup } => {
                write!(f, "editor wal batch: {batch} (truncate: {cleanup})")
            }
            Self::OrphanGroup => f.write_str("editor wal: open group without its retry key"),
            Self::UnsupportedVersion { got } => {
                write!(
                    f,
                    "editor wal: unsupported version v{got} (reader opens v1)"
                )
            }
        }
    }
}

impl std::error::Error for WalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Batch(error) => Some(error),
            Self::Snapshot(error) => Some(error),
            Self::Truncate(error) => Some(error),
            Self::BatchTruncate { cleanup, .. } => Some(cleanup),
            Self::Corrupt
            | Self::TooLarge
            | Self::OpenGroup
            | Self::OrphanGroup
            | Self::UnsupportedVersion { .. } => None,
        }
    }
}

impl From<std::io::Error> for WalError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<crate::commands::BatchError> for WalError {
    fn from(error: crate::commands::BatchError) -> Self {
        Self::Batch(error)
    }
}

/// Offset reader over untrusted bytes; every overrun is [`WalError::Corrupt`].
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn rest(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], WalError> {
        if count > self.rest() {
            return Err(WalError::Corrupt);
        }
        let slice = &self.bytes[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, WalError> {
        Ok(self.take(1)?[0])
    }

    /// The next `N` bytes as an array; a short input is corrupt.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], WalError> {
        self.take(N)?.try_into().map_err(|_| WalError::Corrupt)
    }

    fn u32(&mut self) -> Result<u32, WalError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, WalError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn short_text(&mut self) -> Result<String, WalError> {
        let len = self.u32()? as usize;
        if len > MAX_SHORT_TEXT {
            return Err(WalError::Corrupt);
        }
        let raw = self.take(len)?;
        core::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| WalError::Corrupt)
    }

    fn effect_text(&mut self) -> Result<String, WalError> {
        let len = self.u32()? as usize;
        if len > MAX_EFFECT_TEXT {
            return Err(WalError::Corrupt);
        }
        let raw = self.take(len)?;
        core::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| WalError::Corrupt)
    }

    fn id(&mut self) -> Result<ObjectId, WalError> {
        let raw = self.take(36)?;
        let text = core::str::from_utf8(raw).map_err(|_| WalError::Corrupt)?;
        ObjectId::parse(text).map_err(|_| WalError::Corrupt)
    }

    fn transform(&mut self) -> Result<engine_scene::transform::Transform, WalError> {
        let mut parts = [0f32; 10];
        for part in &mut parts {
            let raw = self.take(4)?;
            *part = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        }
        Ok(engine_scene::transform::Transform::new(
            [parts[0], parts[1], parts[2]],
            [parts[3], parts[4], parts[5], parts[6]],
            [parts[7], parts[8], parts[9]],
        ))
    }
}

fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_text(out: &mut Vec<u8>, text: &str) {
    push_u32(out, text.len() as u32);
    out.extend_from_slice(text.as_bytes());
}

fn push_op(out: &mut Vec<u8>, op: &Op) {
    match op {
        Op::Insert { id, parent, local } => {
            push_u8(out, 0);
            push_id(out, *id);
            push_id(out, *parent);
            push_transform(out, *local);
        }
        Op::Remove { id } => {
            push_u8(out, 1);
            push_id(out, *id);
        }
        Op::SetLocal { id, local } => {
            push_u8(out, 2);
            push_id(out, *id);
            push_transform(out, *local);
        }
        Op::Reparent { id, new_parent } => {
            push_u8(out, 3);
            push_id(out, *id);
            push_id(out, *new_parent);
        }
        Op::SetMaterial { id, material } => {
            push_u8(out, 4);
            push_id(out, *id);
            match material {
                Some(name) => push_text(out, name.as_str()),
                None => push_u32(out, 0),
            }
        }
        Op::SetComponent { id, component } => {
            push_u8(out, 5);
            push_id(out, *id);
            let bytes = component.to_bytes();
            // Canonical component bytes are under 200; the saturating form
            // keeps the encoder total (the reader bounds the length anyway).
            push_u32(out, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
            out.extend_from_slice(&bytes);
        }
        Op::RemoveComponent { id, kind } => {
            push_u8(out, 6);
            push_id(out, *id);
            push_u8(out, kind.tag());
        }
        Op::PutMaterial { text } => {
            push_u8(out, 7);
            push_text(out, text);
        }
        Op::SetMaterialParam {
            material,
            path,
            value,
        } => {
            push_u8(out, 8);
            push_text(out, material);
            push_text(out, path);
            push_text(out, value);
        }
        Op::RemoveLastMaterial => push_u8(out, 9),
        Op::SetBond { id, a, b, strength } => {
            push_u8(out, 10);
            push_id(out, *id);
            push_u32(out, *a);
            push_u32(out, *b);
            match strength {
                Some(value) => {
                    push_u8(out, 1);
                    push_u32(out, u32::from(value.permille()));
                }
                None => push_u8(out, 0),
            }
        }
    }
}

fn read_op(cursor: &mut Cursor<'_>) -> Result<Op, WalError> {
    match cursor.u8()? {
        0 => Ok(Op::Insert {
            id: cursor.id()?,
            parent: cursor.id()?,
            local: cursor.transform()?,
        }),
        1 => Ok(Op::Remove { id: cursor.id()? }),
        2 => Ok(Op::SetLocal {
            id: cursor.id()?,
            local: cursor.transform()?,
        }),
        3 => Ok(Op::Reparent {
            id: cursor.id()?,
            new_parent: cursor.id()?,
        }),
        4 => {
            let id = cursor.id()?;
            let text = cursor.short_text()?;
            if text.is_empty() {
                Ok(Op::SetMaterial { id, material: None })
            } else {
                let name = MaterialName::try_from(text.as_str()).map_err(|_| WalError::Corrupt)?;
                Ok(Op::SetMaterial {
                    id,
                    material: Some(name),
                })
            }
        }
        5 => {
            let id = cursor.id()?;
            let len = usize::try_from(cursor.u32()?).map_err(|_| WalError::Corrupt)?;
            if len > MAX_SHORT_TEXT {
                return Err(WalError::Corrupt);
            }
            let raw = cursor.take(len)?;
            let component = engine_scene::component::Component::from_bytes(raw)
                .map_err(|_| WalError::Corrupt)?;
            Ok(Op::SetComponent { id, component })
        }
        6 => {
            let id = cursor.id()?;
            let kind = engine_scene::component::ComponentKind::from_tag(cursor.u8()?)
                .ok_or(WalError::Corrupt)?;
            Ok(Op::RemoveComponent { id, kind })
        }
        7 => Ok(Op::PutMaterial {
            text: cursor.effect_text()?,
        }),
        8 => Ok(Op::SetMaterialParam {
            material: cursor.effect_text()?,
            path: cursor.effect_text()?,
            value: cursor.effect_text()?,
        }),
        9 => Ok(Op::RemoveLastMaterial),
        10 => {
            let id = cursor.id()?;
            let a = cursor.u32()?;
            let b = cursor.u32()?;
            let strength = match cursor.u8()? {
                0 => None,
                1 => Some(BondStrength::new(cursor.u32()?).map_err(|_| WalError::Corrupt)?),
                _ => return Err(WalError::Corrupt),
            };
            Ok(Op::SetBond { id, a, b, strength })
        }
        _ => Err(WalError::Corrupt),
    }
}

fn push_stamp(out: &mut Vec<u8>, stamp: &Option<crate::commands::ProvenanceStamp>) {
    match stamp {
        None => push_u8(out, 0),
        Some(stamp) => {
            push_u8(out, 1);
            match &stamp.source {
                ReceiptSource::Human => push_u8(out, 0),
                ReceiptSource::Script { name } => {
                    push_u8(out, 1);
                    push_text(out, name);
                }
                ReceiptSource::Model { provider, model } => {
                    push_u8(out, 2);
                    push_text(out, provider);
                    push_text(out, model);
                }
            }
            out.extend_from_slice(&stamp.request_digest);
        }
    }
}

fn read_stamp(
    cursor: &mut Cursor<'_>,
) -> Result<Option<crate::commands::ProvenanceStamp>, WalError> {
    match cursor.u8()? {
        0 => Ok(None),
        1 => {
            let source = match cursor.u8()? {
                0 => ReceiptSource::Human,
                1 => ReceiptSource::Script {
                    name: cursor.short_text()?,
                },
                2 => ReceiptSource::Model {
                    provider: cursor.short_text()?,
                    model: cursor.short_text()?,
                },
                _ => return Err(WalError::Corrupt),
            };
            let raw = cursor.take(32)?;
            let mut request_digest = [0u8; 32];
            request_digest.copy_from_slice(raw);
            Ok(Some(crate::commands::ProvenanceStamp {
                source,
                request_digest,
            }))
        }
        _ => Err(WalError::Corrupt),
    }
}

fn push_entry(out: &mut Vec<u8>, entry: &JournalEntry, with_digests: bool) -> Result<(), WalError> {
    push_u64(out, entry.key);
    push_u64(out, entry.base_revision);
    push_u64(out, entry.result_revision);
    push_text(out, &entry.name);
    push_u8(out, u8::from(entry.significant));
    push_u64(out, entry.group);
    if entry.keys.len() > MAX_OPS || entry.forward.len() > MAX_OPS {
        return Err(WalError::TooLarge);
    }
    push_u32(out, entry.keys.len() as u32);
    for key in &entry.keys {
        push_u64(out, *key);
    }
    push_u32(out, entry.forward.len() as u32);
    for op in &entry.forward {
        push_op(out, op);
    }
    if entry.inverse_groups.len() > MAX_OPS {
        return Err(WalError::TooLarge);
    }
    push_u32(out, entry.inverse_groups.len() as u32);
    for group in &entry.inverse_groups {
        if group.len() > MAX_OPS {
            return Err(WalError::TooLarge);
        }
        push_u32(out, group.len() as u32);
        for op in group {
            push_op(out, op);
        }
    }
    if entry.effects.len() > MAX_OPS {
        return Err(WalError::TooLarge);
    }
    push_u32(out, entry.effects.len() as u32);
    for line in &entry.effects {
        if line.len() > MAX_EFFECT_TEXT {
            return Err(WalError::TooLarge);
        }
        push_text(out, line);
    }
    let provenance = entry.provenance.encode().map_err(|_| WalError::Corrupt)?;
    if provenance.len() > engine_scene::provenance::MAX_PROVENANCE_BYTES {
        return Err(WalError::TooLarge);
    }
    push_u32(out, provenance.len() as u32);
    out.extend_from_slice(&provenance);
    if with_digests {
        out.extend_from_slice(&entry.before_digest);
        out.extend_from_slice(&entry.staged_digest);
        out.extend_from_slice(&entry.after_digest);
    }
    Ok(())
}

fn read_entry(cursor: &mut Cursor<'_>, with_digests: bool) -> Result<JournalEntry, WalError> {
    let key = cursor.u64()?;
    let base_revision = cursor.u64()?;
    let result_revision = cursor.u64()?;
    let name = cursor.short_text()?;
    if name.len() > MAX_COMMAND_NAME_LEN {
        return Err(WalError::Corrupt);
    }
    let significant = match cursor.u8()? {
        0 => false,
        1 => true,
        _ => return Err(WalError::Corrupt),
    };
    let group = cursor.u64()?;
    let keys_len = cursor.u32()? as usize;
    if keys_len > MAX_OPS {
        return Err(WalError::Corrupt);
    }
    let mut keys = Vec::with_capacity(keys_len.min(1024));
    for _ in 0..keys_len {
        keys.push(cursor.u64()?);
    }
    let fwd_len = cursor.u32()? as usize;
    if fwd_len > MAX_OPS {
        return Err(WalError::Corrupt);
    }
    let mut forward = Vec::with_capacity(fwd_len.min(1024));
    for _ in 0..fwd_len {
        forward.push(read_op(cursor)?);
    }
    let groups_len = cursor.u32()? as usize;
    if groups_len > MAX_OPS {
        return Err(WalError::Corrupt);
    }
    let mut inverse_groups = Vec::with_capacity(groups_len.min(1024));
    for _ in 0..groups_len {
        let len = cursor.u32()? as usize;
        if len > MAX_OPS {
            return Err(WalError::Corrupt);
        }
        let mut group_ops = Vec::with_capacity(len.min(1024));
        for _ in 0..len {
            group_ops.push(read_op(cursor)?);
        }
        inverse_groups.push(group_ops);
    }
    // The flattened inverse is exactly the reversed flatten of the groups
    // (see `stage` and `end_group` in the command service).
    let inverse: Vec<Op> = inverse_groups.iter().rev().flatten().cloned().collect();
    let effects_len = cursor.u32()? as usize;
    if effects_len > MAX_OPS {
        return Err(WalError::Corrupt);
    }
    let mut effects = Vec::with_capacity(effects_len.min(1024));
    for _ in 0..effects_len {
        effects.push(cursor.effect_text()?);
    }
    let prov_len = cursor.u32()? as usize;
    if prov_len > engine_scene::provenance::MAX_PROVENANCE_BYTES {
        return Err(WalError::Corrupt);
    }
    let prov_raw = cursor.take(prov_len)?;
    let provenance = Provenance::decode(prov_raw).map_err(|_| WalError::Corrupt)?;
    if keys.is_empty() || keys[0] != key {
        return Err(WalError::Corrupt);
    }
    let (before_digest, staged_digest, after_digest) = if with_digests {
        (
            cursor.array::<32>()?,
            cursor.array::<32>()?,
            cursor.array::<32>()?,
        )
    } else {
        ([0u8; 32], [0u8; 32], [0u8; 32])
    };
    Ok(JournalEntry {
        key,
        forward,
        inverse,
        inverse_groups,
        base_revision,
        result_revision,
        before_digest,
        staged_digest,
        after_digest,
        effects,
        provenance,
        name,
        significant,
        group,
        keys,
    })
}

/// One writer over a directory holding `journal.wal` plus `snapshot.wal`.
///
/// The writer is single by construction: every mutation takes `&mut self`,
/// the type is not `Clone`, and the protocol is append-then-acknowledge —
/// each frame is written and flushed before its edit is acknowledged, and
/// the header end mark only advances past a fully written frame. Readers
/// (here: [`Wal::open`] and [`Wal::stamps`]) are pinned to that end mark, so
/// a kill between the frame write and the mark update leaves an ignored
/// tail, never a half-read edit.
///
/// `flush` (not `sync_all`) per frame is enough for the killed-process gate:
/// the OS keeps flushed bytes. Power-loss durability wants `sync_all` on the
/// frame, the mark and the directory; that upgrade is open work, not claimed.
pub struct Wal {
    dir: PathBuf,
    file: std::fs::File,
    snapshot_revision: u64,
    valid_len: u64,
}

/// What [`Wal::open`] recovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenReport {
    /// Revision sealed in the snapshot the replay started from.
    pub snapshot_revision: u64,
    /// Undo-stack depth carried by the snapshot.
    pub snap_past: usize,
    /// Redo-stack depth carried by the snapshot.
    pub snap_future: usize,
    /// Frames replayed after the snapshot.
    pub replayed: usize,
    /// Bytes past the end mark or torn frames, truncated on open.
    pub truncated_bytes: u64,
    /// Document revision after replay (the last good state).
    pub revision: u64,
}

/// What [`Wal::checkpoint`] sealed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointReport {
    /// Document revision sealed (also the new snapshot revision).
    pub revision: u64,
    /// Snapshot file bytes written.
    pub snap_bytes: usize,
    /// Undo-stack depth sealed.
    pub past: usize,
    /// Redo-stack depth sealed.
    pub future: usize,
}

fn header_bytes(snapshot_revision: u64, valid_len: u64) -> [u8; HEADER_LEN] {
    let mut header = [0u8; HEADER_LEN];
    header[0..4].copy_from_slice(&WAL_MAGIC);
    header[4..8].copy_from_slice(&WAL_VERSION.to_le_bytes());
    header[8..16].copy_from_slice(&snapshot_revision.to_le_bytes());
    header[16..24].copy_from_slice(&valid_len.to_le_bytes());
    let crc = crc32(&header[0..24]);
    header[24..28].copy_from_slice(&crc.to_le_bytes());
    header
}

fn parse_header(bytes: &[u8]) -> Result<(u64, u64), WalError> {
    if bytes.len() < HEADER_LEN {
        return Err(WalError::Corrupt);
    }
    if bytes[0..4] != WAL_MAGIC {
        return Err(WalError::Corrupt);
    }
    let mut version = [0u8; 4];
    version.copy_from_slice(&bytes[4..8]);
    let version = u32::from_le_bytes(version);
    if !(MIN_WAL_VERSION..=WAL_VERSION).contains(&version) {
        return Err(WalError::UnsupportedVersion { got: version });
    }
    let mut revision = [0u8; 8];
    revision.copy_from_slice(&bytes[8..16]);
    let mut valid = [0u8; 8];
    valid.copy_from_slice(&bytes[16..24]);
    let mut crc = [0u8; 4];
    crc.copy_from_slice(&bytes[24..28]);
    if u32::from_le_bytes(crc) != crc32(&bytes[0..24]) {
        return Err(WalError::Corrupt);
    }
    let valid_len = u64::from_le_bytes(valid);
    if valid_len < HEADER_LEN as u64 {
        return Err(WalError::Corrupt);
    }
    Ok((u64::from_le_bytes(revision), valid_len))
}

/// Frame boundary scan: every `(offset, total_len)` reachable by
/// length+CRC from `start`, stopping at the first torn frame.
fn scan_frames(bytes: &[u8], start: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos < end {
        if pos + 8 > bytes.len() || pos + 8 > end {
            break;
        }
        let raw = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if len > MAX_FRAME_BYTES {
            break;
        }
        let total = 8 + len;
        if pos + total > bytes.len() || pos + total > end {
            break;
        }
        let crc_raw = &bytes[pos + 4..pos + 8];
        let want = u32::from_le_bytes([crc_raw[0], crc_raw[1], crc_raw[2], crc_raw[3]]);
        if want != crc32(&bytes[pos + 8..pos + total]) {
            break;
        }
        out.push((pos, total));
        pos += total;
    }
    out
}

fn frame_payload(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(1 + body.len());
    payload.push(kind);
    payload.extend_from_slice(body);
    payload
}

fn encode_apply(
    tick: u64,
    batch: &Batch,
    meta: &CommandMeta,
    result_revision: u64,
) -> Result<Vec<u8>, WalError> {
    if batch.ops.len() > MAX_OPS {
        return Err(WalError::TooLarge);
    }
    let mut body = Vec::new();
    push_u64(&mut body, tick);
    push_u64(&mut body, batch.idempotency_key);
    push_u64(&mut body, batch.base_revision);
    push_u64(&mut body, result_revision);
    push_text(&mut body, meta.name());
    push_u8(&mut body, u8::from(meta.is_significant()));
    push_stamp(&mut body, &batch.provenance);
    push_u32(&mut body, batch.ops.len() as u32);
    for op in &batch.ops {
        push_op(&mut body, op);
    }
    Ok(frame_payload(KIND_APPLY, &body))
}

impl Wal {
    /// Open (or create) the journal in `dir`: load the snapshot, replay the
    /// log prefix pinned to the end mark, truncate any torn tail, and hand
    /// back the single writer plus what was recovered.
    ///
    /// Missing files open fresh at revision zero. A damaged snapshot refuses
    /// outright; a damaged log tail truncates to the last good frame.
    pub fn open(
        dir: &Path,
        doc: &mut Document,
        journal: &mut Journal,
    ) -> Result<(Self, OpenReport), WalError> {
        let snap_path = dir.join(SNAP_FILE_NAME);
        let snap_raw = match std::fs::read(&snap_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(WalError::Io(error)),
        };
        let (snap_revision, snap_past, snap_future) = match snap_raw {
            None => {
                *doc = Document::new();
                *journal = Journal::new();
                (0, 0, 0)
            }
            Some(bytes) => {
                let (loaded_doc, loaded_journal) = decode_snapshot(&bytes)?;
                let revision = loaded_doc.revision;
                let past = loaded_journal.past.len();
                let future = loaded_journal.future.len();
                *doc = loaded_doc;
                *journal = loaded_journal;
                (revision, past, future)
            }
        };

        let log_path = dir.join(WAL_FILE_NAME);
        let log_raw = match std::fs::read(&log_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(WalError::Io(error)),
        };
        // Fresh file, or a torn header shorter than one header: rewrite.
        if log_raw.len() < HEADER_LEN {
            if let Some(parent) = log_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&log_path)?;
            let header = header_bytes(snap_revision, HEADER_LEN as u64);
            use std::io::{Seek, SeekFrom, Write};
            file.write_all(&header)?;
            file.flush()?;
            file.seek(SeekFrom::End(0))?;
            let truncated = log_raw.len() as u64;
            return Ok((
                Self {
                    dir: dir.to_owned(),
                    file,
                    snapshot_revision: snap_revision,
                    valid_len: HEADER_LEN as u64,
                },
                OpenReport {
                    snapshot_revision: snap_revision,
                    snap_past,
                    snap_future,
                    replayed: 0,
                    truncated_bytes: truncated,
                    revision: doc.revision,
                },
            ));
        }
        // Header damage (or an end mark past EOF) falls back to a salvage
        // scan: the longest CRC-clean frame prefix from offset 28.
        let (log_snap_rev, mut valid) = match parse_header(&log_raw) {
            Ok((revision, valid)) if (valid as usize) <= log_raw.len() => (revision, valid),
            Err(WalError::UnsupportedVersion { got }) => {
                return Err(WalError::UnsupportedVersion { got });
            }
            _ => {
                let salvage = scan_frames(&log_raw, HEADER_LEN, log_raw.len());
                let end = salvage
                    .last()
                    .map(|(offset, total)| (*offset + *total) as u64)
                    .unwrap_or(HEADER_LEN as u64);
                (snap_revision, end)
            }
        };
        if log_snap_rev != snap_revision {
            // The snapshot on disk is newer than the one the log replays from
            // (crash between the snapshot rename and the log reset): the
            // snapshot already holds every logged edit, so the stale prefix
            // cannot replay and is dropped.
            valid = HEADER_LEN as u64;
        } else {
            // Normal case: scan the pinned prefix for torn frames.
            let frames = scan_frames(&log_raw, HEADER_LEN, valid as usize);
            valid = frames
                .last()
                .map(|(offset, total)| (*offset + *total) as u64)
                .unwrap_or(HEADER_LEN as u64);
        }
        let frames = scan_frames(&log_raw, HEADER_LEN, valid as usize);
        let mut replayed = 0usize;
        let mut good_end = HEADER_LEN as u64;
        for (offset, total) in &frames {
            let payload = &log_raw[offset + 8..offset + total];
            if replay_frame(doc, journal, payload).is_err() {
                break;
            }
            replayed += 1;
            good_end = (*offset + *total) as u64;
        }
        let claimed = valid;
        valid = good_end;
        let truncated_bytes = (log_raw.len() as u64)
            .saturating_sub(valid)
            .max(claimed.saturating_sub(valid));
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&log_path)?;
        use std::io::{Seek, SeekFrom, Write};
        let header = header_bytes(snap_revision, valid);
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.set_len(valid)?;
        file.flush()?;
        file.seek(SeekFrom::End(0))?;
        Ok((
            Self {
                dir: dir.to_owned(),
                file,
                snapshot_revision: snap_revision,
                valid_len: valid,
            },
            OpenReport {
                snapshot_revision: snap_revision,
                snap_past,
                snap_future,
                replayed,
                truncated_bytes,
                revision: doc.revision,
            },
        ))
    }

    /// Directory holding the two journal files.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Snapshot revision the log replays from.
    pub fn snapshot_revision(&self) -> u64 {
        self.snapshot_revision
    }

    /// Reader end mark: replay and inspection never read past this offset.
    pub fn end_mark(&self) -> u64 {
        self.valid_len
    }

    /// Stamps of the frames pinned under the end mark, oldest first.
    /// Undo/redo/group markers carry no stamp and are skipped.
    pub fn stamps(&self) -> Result<Vec<WalStamp>, WalError> {
        let bytes = std::fs::read(self.dir.join(WAL_FILE_NAME))?;
        let end = (self.valid_len as usize).min(bytes.len());
        let mut out = Vec::new();
        for (offset, total) in scan_frames(&bytes, HEADER_LEN, end) {
            let payload = &bytes[offset + 8..offset + total];
            if payload.is_empty() || payload[0] != KIND_APPLY {
                continue;
            }
            let mut cursor = Cursor::new(&payload[1..]);
            let tick = cursor.u64()?;
            cursor.u64()?;
            let base = cursor.u64()?;
            let result = cursor.u64()?;
            if result != base + 1 {
                return Err(WalError::Corrupt);
            }
            out.push(WalStamp {
                tick,
                revision: result,
            });
        }
        Ok(out)
    }

    /// Append one encoded frame, then advance the end mark past it. One
    /// `flush` covers both writes: bytes a killed process already `write()`n
    /// stay in the kernel, so a kill in between leaves an ignored tail
    /// rather than a half-acknowledged edit (the mark only names fully
    /// written frames, and a torn header still carries its CRC).
    fn append_frame(&mut self, payload: &[u8]) -> Result<(), WalError> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(WalError::TooLarge);
        }
        use std::io::{Seek, SeekFrom, Write};
        self.file.seek(SeekFrom::End(0))?;
        push_frame(&mut self.file, payload)?;
        self.valid_len += (8 + payload.len()) as u64;
        self.file.seek(SeekFrom::Start(0))?;
        self.file
            .write_all(&header_bytes(self.snapshot_revision, self.valid_len))?;
        self.file.flush()?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    /// Forget the last appended frame (compensation when the in-memory
    /// mutation that the frame describes fails after the write).
    fn forget_last(&mut self, payload_len: usize) -> Result<(), WalError> {
        use std::io::{Seek, SeekFrom, Write};
        self.valid_len = self
            .valid_len
            .saturating_sub((8 + payload_len) as u64)
            .max(HEADER_LEN as u64);
        self.file.seek(SeekFrom::Start(0))?;
        self.file
            .write_all(&header_bytes(self.snapshot_revision, self.valid_len))?;
        self.file.set_len(self.valid_len)?;
        self.file.flush()?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    /// Seal the current document plus the full undo/redo stacks into
    /// `snapshot.wal` (tmp file plus rename), then truncate the log.
    ///
    /// Refuses with [`WalError::OpenGroup`] while a command group is open:
    /// ids for a fresh journal restart at the same sequence, which is only
    /// sound when every live entry is ungrouped.
    pub fn checkpoint(
        &mut self,
        doc: &Document,
        journal: &Journal,
    ) -> Result<CheckpointReport, WalError> {
        if open_group_depth(journal) > 0 {
            return Err(WalError::OpenGroup);
        }
        let bytes = encode_snapshot(doc, journal)?;
        let snap_path = self.dir.join(SNAP_FILE_NAME);
        if let Some(parent) = snap_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.dir.join(format!("{SNAP_FILE_NAME}{TMP_SUFFIX}"));
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&bytes)?;
            file.flush()?;
        }
        std::fs::rename(&tmp, &snap_path)?;
        self.snapshot_revision = doc.revision;
        self.valid_len = HEADER_LEN as u64;
        use std::io::{Seek, SeekFrom, Write};
        self.file.seek(SeekFrom::Start(0))?;
        self.file
            .write_all(&header_bytes(self.snapshot_revision, self.valid_len))?;
        self.file.set_len(self.valid_len)?;
        self.file.flush()?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(CheckpointReport {
            revision: doc.revision,
            snap_bytes: bytes.len(),
            past: journal.past.len(),
            future: journal.future.len(),
        })
    }
}

fn push_frame(file: &mut std::fs::File, payload: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    file.write_all(&(payload.len() as u32).to_le_bytes())?;
    file.write_all(&crc32(payload).to_le_bytes())?;
    file.write_all(payload)?;
    Ok(())
}

fn encode_snapshot(doc: &Document, journal: &Journal) -> Result<Vec<u8>, WalError> {
    encode_snapshot_at(doc, journal, WAL_VERSION)
}

/// Snapshot bytes at an explicit layout version. The writer always calls this
/// with [`WAL_VERSION`]; the older version is for the reopen test that proves
/// a version 1 snapshot still loads with zero digests.
fn encode_snapshot_at(
    doc: &Document,
    journal: &Journal,
    version: u32,
) -> Result<Vec<u8>, WalError> {
    let with_digests = version >= 2;
    let save = save_to_bytes(doc);
    if save.len() > MAX_SAVE_BYTES {
        return Err(WalError::TooLarge);
    }
    if journal.past.len() > MAX_SNAP_ENTRIES || journal.future.len() > MAX_SNAP_ENTRIES {
        return Err(WalError::TooLarge);
    }
    let mut entries = Vec::new();
    push_u32(&mut entries, journal.past.len() as u32);
    for entry in &journal.past {
        push_entry(&mut entries, entry, with_digests)?;
    }
    push_u32(&mut entries, journal.future.len() as u32);
    for entry in &journal.future {
        push_entry(&mut entries, entry, with_digests)?;
    }
    let mut out = Vec::with_capacity(32 + save.len() + entries.len());
    out.extend_from_slice(&SNAP_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    push_u64(&mut out, doc.revision);
    push_u32(&mut out, save.len() as u32);
    out.extend_from_slice(&save);
    out.extend_from_slice(&crc32(&save).to_le_bytes());
    out.extend_from_slice(&entries);
    out.extend_from_slice(&crc32(&entries).to_le_bytes());
    Ok(out)
}

fn decode_snapshot(bytes: &[u8]) -> Result<(Document, Journal), WalError> {
    let mut cursor = Cursor::new(bytes);
    let magic = cursor.take(4)?;
    if magic != SNAP_MAGIC {
        return Err(WalError::Corrupt);
    }
    let version = cursor.u32()?;
    if !(MIN_WAL_VERSION..=WAL_VERSION).contains(&version) {
        return Err(WalError::UnsupportedVersion { got: version });
    }
    let with_digests = version >= 2;
    let revision = cursor.u64()?;
    let save_len = cursor.u32()? as usize;
    if save_len > MAX_SAVE_BYTES {
        return Err(WalError::Corrupt);
    }
    let save_raw = cursor.take(save_len)?;
    let want_raw = cursor.take(4)?;
    let want = u32::from_le_bytes([want_raw[0], want_raw[1], want_raw[2], want_raw[3]]);
    if want != crc32(save_raw) {
        return Err(WalError::Corrupt);
    }
    let doc = load_from_bytes(save_raw).map_err(WalError::Snapshot)?;
    if doc.revision != revision {
        return Err(WalError::Corrupt);
    }
    let entries_start = cursor.pos;
    let past_len = cursor.u32()? as usize;
    if past_len > MAX_SNAP_ENTRIES {
        return Err(WalError::Corrupt);
    }
    let mut past = Vec::with_capacity(past_len.min(128));
    for _ in 0..past_len {
        past.push(read_entry(&mut cursor, with_digests)?);
    }
    let future_len = cursor.u32()? as usize;
    if future_len > MAX_SNAP_ENTRIES {
        return Err(WalError::Corrupt);
    }
    let mut future = Vec::with_capacity(future_len.min(128));
    for _ in 0..future_len {
        future.push(read_entry(&mut cursor, with_digests)?);
    }
    let entries_bytes = &bytes[entries_start..cursor.pos];
    let crc_raw = cursor.take(4)?;
    let crc = u32::from_le_bytes([crc_raw[0], crc_raw[1], crc_raw[2], crc_raw[3]]);
    if crc != crc32(entries_bytes) {
        return Err(WalError::Corrupt);
    }
    if cursor.rest() != 0 {
        return Err(WalError::Corrupt);
    }
    let mut journal = Journal::new();
    journal.past = past;
    journal.future = future;
    Ok((doc, journal))
}

/// Replay one pinned frame. Any refusal stops the replay before the frame.
fn replay_frame(doc: &mut Document, journal: &mut Journal, payload: &[u8]) -> Result<(), WalError> {
    if payload.is_empty() {
        return Err(WalError::Corrupt);
    }
    let mut cursor = Cursor::new(&payload[1..]);
    match payload[0] {
        KIND_APPLY => {
            let tick = cursor.u64()?;
            let _ = tick;
            let key = cursor.u64()?;
            let base = cursor.u64()?;
            let result = cursor.u64()?;
            if result != base + 1 || base != doc.revision {
                return Err(WalError::Corrupt);
            }
            let name = cursor.short_text()?;
            if name.len() > MAX_COMMAND_NAME_LEN {
                return Err(WalError::Corrupt);
            }
            let significant = match cursor.u8()? {
                0 => false,
                1 => true,
                _ => return Err(WalError::Corrupt),
            };
            let provenance = read_stamp(&mut cursor)?;
            let ops_len = cursor.u32()? as usize;
            if ops_len > MAX_OPS {
                return Err(WalError::Corrupt);
            }
            let mut ops = Vec::with_capacity(ops_len.min(1024));
            for _ in 0..ops_len {
                ops.push(read_op(&mut cursor)?);
            }
            if cursor.rest() != 0 {
                return Err(WalError::Corrupt);
            }
            let batch = Batch {
                ops,
                base_revision: base,
                idempotency_key: key,
                provenance,
            };
            let meta = CommandMeta::new(name, significant);
            let applied = apply_named(doc, journal, &batch, &meta)?;
            if applied.revision != result {
                return Err(WalError::Corrupt);
            }
            Ok(())
        }
        KIND_UNDO => {
            if cursor.rest() != 0 {
                return Err(WalError::Corrupt);
            }
            undo(doc, journal).ok_or(WalError::Corrupt)?;
            Ok(())
        }
        KIND_REDO => {
            if cursor.rest() != 0 {
                return Err(WalError::Corrupt);
            }
            redo(doc, journal).ok_or(WalError::Corrupt)?;
            Ok(())
        }
        KIND_BEGIN_GROUP => {
            let name = cursor.short_text()?;
            let significant = match cursor.u8()? {
                0 => false,
                1 => true,
                _ => return Err(WalError::Corrupt),
            };
            let key = cursor.u64()?;
            if cursor.rest() != 0 {
                return Err(WalError::Corrupt);
            }
            begin_group(journal, &CommandMeta::new(name, significant), key);
            Ok(())
        }
        KIND_END_GROUP => {
            let key = cursor.u64()?;
            if cursor.rest() != 0 {
                return Err(WalError::Corrupt);
            }
            let applied = end_group(journal).ok_or(WalError::Corrupt)?;
            if applied.key != key {
                return Err(WalError::Corrupt);
            }
            Ok(())
        }
        KIND_CANCEL_GROUP => {
            if cursor.rest() != 0 {
                return Err(WalError::Corrupt);
            }
            if !cancel_group(journal) {
                return Err(WalError::Corrupt);
            }
            Ok(())
        }
        _ => Err(WalError::Corrupt),
    }
}

/// Key state for the idempotency pre-check (mirrors [`apply_named`] without
/// mutating, so the frame is only logged when the apply will really run).
enum KeyState {
    /// Same key plus same payload replays the stored receipt: no new frame.
    Replay,
    /// Same key with another payload, or an undone key: refused, no frame.
    Refused,
    /// Fresh key: log, then apply.
    Fresh,
}

fn key_state(journal: &Journal, batch: &Batch) -> KeyState {
    for entry in journal.past.iter() {
        if entry.key == batch.idempotency_key || entry.keys.contains(&batch.idempotency_key) {
            if entry.key == batch.idempotency_key
                && entry.forward == batch.ops
                && entry.base_revision == batch.base_revision
            {
                return KeyState::Replay;
            }
            return KeyState::Refused;
        }
    }
    if journal.future.iter().any(|entry| {
        entry.key == batch.idempotency_key || entry.keys.contains(&batch.idempotency_key)
    }) {
        return KeyState::Refused;
    }
    KeyState::Fresh
}

/// Validated, atomic, journaled apply: the batch path plus the WAL frame.
///
/// The frame is written and the end mark advanced BEFORE the in-memory apply
/// runs (write-ahead); when the apply then reports its receipt, the edit is
/// already durable. A refused batch truncates the frame it just wrote, so a
/// failed edit leaves document, journal and log exactly as they were.
pub fn wal_apply(
    wal: &mut Wal,
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
    meta: &CommandMeta,
    tick: u64,
) -> Result<crate::commands::Applied, WalError> {
    if !matches!(key_state(journal, batch), KeyState::Fresh) {
        // Idempotent retry or key conflict: `apply_named` answers without
        // mutating, so nothing is logged and the answer passes through.
        return apply_named(doc, journal, batch, meta).map_err(WalError::Batch);
    }
    // Log before validating: the frame precedes the mutation (write-ahead),
    // and a refusal below truncates it. This also stages the batch only once
    // (inside `apply_named`), keeping the logged path near the bare cost.
    let payload = encode_apply(tick, batch, meta, doc.revision + 1)?;
    wal.append_frame(&payload)?;
    match apply_named(doc, journal, batch, meta) {
        Ok(applied) => Ok(applied),
        Err(error) => {
            wal.forget_last(payload.len())
                .map_err(|cleanup| match cleanup {
                    WalError::Io(io) => WalError::BatchTruncate {
                        batch: error.clone(),
                        cleanup: io,
                    },
                    other => other,
                })?;
            Err(WalError::Batch(error))
        }
    }
}

/// Journaled [`undo`]: the marker is logged before the in-memory undo runs;
/// a failed undo truncates its marker, so the log never leads the journal.
pub fn wal_undo(
    wal: &mut Wal,
    doc: &mut Document,
    journal: &mut Journal,
) -> Result<Option<crate::commands::Applied>, WalError> {
    if journal.past.is_empty() {
        return Ok(None);
    }
    let payload = frame_payload(KIND_UNDO, &[]);
    wal.append_frame(&payload)?;
    match undo(doc, journal) {
        Some(applied) => Ok(Some(applied)),
        None => {
            wal.forget_last(payload.len())
                .map_err(|error| match error {
                    WalError::Io(io) => WalError::Truncate(io),
                    other => other,
                })?;
            Ok(None)
        }
    }
}

/// Journaled [`redo`]: same marker discipline as [`wal_undo`].
pub fn wal_redo(
    wal: &mut Wal,
    doc: &mut Document,
    journal: &mut Journal,
) -> Result<Option<crate::commands::Applied>, WalError> {
    if journal.future.is_empty() {
        return Ok(None);
    }
    let payload = frame_payload(KIND_REDO, &[]);
    wal.append_frame(&payload)?;
    match redo(doc, journal) {
        Some(applied) => Ok(Some(applied)),
        None => {
            wal.forget_last(payload.len())
                .map_err(|error| match error {
                    WalError::Io(io) => WalError::Truncate(io),
                    other => other,
                })?;
            Ok(None)
        }
    }
}

/// Journaled [`begin_group`]: on a frame-write failure the just-opened group
/// is cancelled again, so the log never trails the journal.
pub fn wal_begin_group(
    wal: &mut Wal,
    journal: &mut Journal,
    meta: &CommandMeta,
    idempotency_key: u64,
) -> Result<u64, WalError> {
    let id = begin_group(journal, meta, idempotency_key);
    let mut body = Vec::new();
    push_text(&mut body, meta.name());
    push_u8(&mut body, u8::from(meta.is_significant()));
    push_u64(&mut body, idempotency_key);
    let payload = frame_payload(KIND_BEGIN_GROUP, &body);
    if let Err(error) = wal.append_frame(&payload) {
        cancel_group(journal);
        return Err(error);
    }
    Ok(id)
}

/// Journaled [`end_group`]: an empty-group commit (which still pops the
/// caller's frame) logs a cancel marker so replay pops the same frame.
/// Log-before-pop: the marker is staged from a read-only peek (group key
/// plus the same trailing-entry scan [`end_group`] runs), so a frame-write
/// failure returns with the journal untouched.
pub fn wal_end_group(
    wal: &mut Wal,
    journal: &mut Journal,
) -> Result<Option<crate::commands::Applied>, WalError> {
    let group_id = match open_group_id(journal) {
        Some(id) => id,
        None => return Ok(None),
    };
    let group_key = open_group_key(journal).ok_or(WalError::OrphanGroup)?;
    // Read-only rehearsal of the commit scan below: trailing entries tagged
    // with this group survive interleaved undo/redo the same way.
    let start = journal
        .past
        .iter()
        .rposition(|entry| entry.group != group_id)
        .map(|index| index + 1)
        .unwrap_or(0);
    if start >= journal.past.len() {
        let payload = frame_payload(KIND_CANCEL_GROUP, &[]);
        wal.append_frame(&payload)?;
        let popped = end_group(journal);
        debug_assert!(popped.is_none(), "rehearsed empty commit stays empty");
        return Ok(None);
    }
    let mut body = Vec::new();
    push_u64(&mut body, group_key);
    let payload = frame_payload(KIND_END_GROUP, &body);
    wal.append_frame(&payload)?;
    match end_group(journal) {
        Some(applied) => {
            debug_assert_eq!(
                applied.key, group_key,
                "rehearsed commit answers the staged key"
            );
            Ok(Some(applied))
        }
        None => {
            // Unreachable single-threaded (the rehearsal above read the
            // same journal), but the logged END marker already names the
            // key, so replay still pops the same frame.
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{durable_count, ProvenanceStamp};
    use engine_scene::transform::Transform;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-editor-wal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir creates");
        dir
    }

    /// Deterministic fixture ids (decimal digits are valid hex).
    fn id(n: u64) -> ObjectId {
        ObjectId::parse(&format!("00000000-0000-0000-0000-{n:012}")).expect("fixture id parses")
    }

    fn pose(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn stamp_for(i: u64) -> Option<ProvenanceStamp> {
        match i % 3 {
            0 => None,
            1 => Some(ProvenanceStamp::script(format!("script-{i}"))),
            _ => Some(ProvenanceStamp::model("mock", "tiny", [i as u8; 32])),
        }
    }

    fn insert_batch(
        node: ObjectId,
        parent: ObjectId,
        x: f32,
        rev: u64,
        key: u64,
        stamp: Option<ProvenanceStamp>,
    ) -> Batch {
        Batch {
            ops: vec![Op::Insert {
                id: node,
                parent,
                local: pose(x),
            }],
            base_revision: rev,
            idempotency_key: key,
            provenance: stamp,
        }
    }

    #[test]
    fn landed_digest_v1_snapshot_opens_with_zero_digests() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        crate::commands::apply_named(
            &mut doc,
            &mut journal,
            &insert_batch(id(1), ObjectId::NIL, 1.0, 0, 1, None),
            &CommandMeta::default(),
        )
        .expect("insert lands");

        let v1 = encode_snapshot_at(&doc, &journal, 1).expect("v1 snapshot encodes");
        let (doc1, journal1) = decode_snapshot(&v1).expect("v1 snapshot opens");
        assert_eq!(doc1.revision, doc.revision);
        let entry = journal1.past.last().expect("v1 entry survives");
        assert_eq!(entry.key, 1);
        assert_eq!(entry.before_digest, [0u8; 32]);
        assert_eq!(entry.staged_digest, [0u8; 32]);
        assert_eq!(entry.after_digest, [0u8; 32]);

        let v2 = encode_snapshot(&doc, &journal).expect("v2 snapshot encodes");
        let (_, journal2) = decode_snapshot(&v2).expect("v2 snapshot opens");
        let seen = journal2.past.last().expect("v2 entry survives");
        assert_eq!(seen.before_digest, journal.past[0].before_digest);
        assert_eq!(seen.staged_digest, journal.past[0].staged_digest);
        assert_eq!(seen.after_digest, journal.past[0].after_digest);
        assert_ne!(seen.after_digest, [0u8; 32]);
    }

    struct State {
        doc: Document,
        past: Vec<JournalEntry>,
        future: Vec<JournalEntry>,
        durable: usize,
        depth: usize,
    }

    fn capture(doc: &Document, journal: &Journal) -> State {
        State {
            doc: doc.clone(),
            past: journal.past.clone(),
            future: journal.future.clone(),
            durable: durable_count(journal),
            depth: open_group_depth(journal),
        }
    }

    fn assert_state_eq(expected: &State, doc: &Document, journal: &Journal) {
        assert_eq!(expected.doc, *doc, "document matches");
        assert_eq!(expected.past, journal.past, "undo stack matches");
        assert_eq!(expected.future, journal.future, "redo stack matches");
        assert_eq!(
            expected.durable,
            durable_count(journal),
            "durable count matches"
        );
        assert_eq!(
            expected.depth,
            open_group_depth(journal),
            "group depth matches"
        );
    }

    #[test]
    fn wal_hundred_edits_undo_redo_reopen_x2() {
        let dir = scratch("hundred");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, report) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh open");
        assert_eq!(report.revision, 0);
        assert_eq!(report.replayed, 0);
        let mut parent = ObjectId::NIL;
        let mut key = 1u64;
        let mut expected_ticks: Vec<u64> = Vec::new();
        for i in 0..100u64 {
            let node = id(i + 1);
            let meta = if i % 7 == 6 {
                CommandMeta::insignificant("tick-step")
            } else {
                CommandMeta::significant("insert")
            };
            let batch = insert_batch(node, parent, i as f32, doc.revision, key, stamp_for(i));
            wal_apply(&mut wal, &mut doc, &mut journal, &batch, &meta, i).expect("edit applies");
            expected_ticks.push(i);
            key += 1;
            parent = node;
            if i == 50 {
                // Grouped drag: ten insignificant steps fold into one entry.
                wal_begin_group(
                    &mut wal,
                    &mut journal,
                    &CommandMeta::significant("drag"),
                    key,
                )
                .expect("begin");
                key += 1;
                let target = id(1);
                for step in 0..10u64 {
                    let batch = Batch {
                        ops: vec![Op::SetLocal {
                            id: target,
                            local: pose(50.0 + step as f32 * 0.1),
                        }],
                        base_revision: doc.revision,
                        idempotency_key: key,
                        provenance: None,
                    };
                    wal_apply(
                        &mut wal,
                        &mut doc,
                        &mut journal,
                        &batch,
                        &CommandMeta::insignificant("drag-tick"),
                        1000 + step,
                    )
                    .expect("drag step applies");
                    expected_ticks.push(1000 + step);
                    key += 1;
                }
                let merged = wal_end_group(&mut wal, &mut journal)
                    .expect("end group")
                    .expect("group merges");
                assert_eq!(merged.revision, doc.revision);
            }
        }
        assert_eq!(doc.revision, 110);
        for _ in 0..10 {
            wal_undo(&mut wal, &mut doc, &mut journal)
                .expect("undo logs")
                .expect("undoes");
        }
        for _ in 0..5 {
            wal_redo(&mut wal, &mut doc, &mut journal)
                .expect("redo logs")
                .expect("redoes");
        }
        let live = capture(&doc, &journal);
        assert_eq!(live.doc.revision, 125);
        // Every apply frame carries its tick plus its published revision.
        let stamps = wal.stamps().expect("stamps read");
        assert_eq!(stamps.len(), expected_ticks.len());
        for (index, stamp) in stamps.iter().enumerate() {
            assert_eq!(stamp.tick, expected_ticks[index], "stamp tick at {index}");
            assert_eq!(
                stamp.revision,
                index as u64 + 1,
                "stamp revision at {index}"
            );
        }
        drop(wal); // Killed process: no checkpoint, flushed frames only.
                   // Reopen twice: both reach the last good state.
        let mut doc_a = Document::new();
        let mut journal_a = Journal::new();
        let (_, report_a) = Wal::open(&dir, &mut doc_a, &mut journal_a).expect("reopen");
        assert_eq!(report_a.replayed, 127);
        assert_eq!(report_a.truncated_bytes, 0);
        assert_state_eq(&live, &doc_a, &journal_a);
        let mut doc_b = Document::new();
        let mut journal_b = Journal::new();
        let (_, report_b) = Wal::open(&dir, &mut doc_b, &mut journal_b).expect("reopen twice");
        assert_eq!(report_b.revision, live.doc.revision);
        assert_state_eq(&live, &doc_b, &journal_b);
        assert_eq!(doc_a, doc_b);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_killed_process_and_checkpoint_reopen_x2() {
        let dir = scratch("kill-checkpoint");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        let mut parent = ObjectId::NIL;
        for i in 0..30u64 {
            let node = id(i + 1);
            let batch = insert_batch(node, parent, i as f32, doc.revision, 100 + i, None);
            wal_apply(
                &mut wal,
                &mut doc,
                &mut journal,
                &batch,
                &CommandMeta::significant("insert"),
                i,
            )
            .expect("applies");
            parent = node;
        }
        let sealed = wal.checkpoint(&doc, &journal).expect("checkpoints");
        assert_eq!(sealed.revision, 30);
        assert_eq!((sealed.past, sealed.future), (30, 0));
        assert_eq!(wal.end_mark(), HEADER_LEN as u64);
        for i in 30..50u64 {
            let node = id(i + 1);
            let batch = insert_batch(node, parent, i as f32, doc.revision, 100 + i, None);
            wal_apply(
                &mut wal,
                &mut doc,
                &mut journal,
                &batch,
                &CommandMeta::significant("insert"),
                i,
            )
            .expect("applies");
            parent = node;
        }
        for _ in 0..3 {
            wal_undo(&mut wal, &mut doc, &mut journal)
                .expect("undo logs")
                .expect("undoes");
        }
        wal_redo(&mut wal, &mut doc, &mut journal)
            .expect("redo logs")
            .expect("redoes");
        let live = capture(&doc, &journal);
        assert_eq!(live.doc.revision, 54);
        drop(wal); // Kill after the checkpoint plus more edits.
        let mut doc_a = Document::new();
        let mut journal_a = Journal::new();
        let (_, report_a) = Wal::open(&dir, &mut doc_a, &mut journal_a).expect("reopen");
        assert_eq!(report_a.snapshot_revision, 30);
        assert_eq!(report_a.replayed, 24);
        assert_state_eq(&live, &doc_a, &journal_a);
        let mut doc_b = Document::new();
        let mut journal_b = Journal::new();
        let (_, report_b) = Wal::open(&dir, &mut doc_b, &mut journal_b).expect("reopen twice");
        assert_eq!(report_b.revision, 54);
        assert_state_eq(&live, &doc_b, &journal_b);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_corrupt_tail_recovers_prefix_x2() {
        for (name, mangle) in [("flip", 0u8), ("truncate", 1u8)] {
            let dir = scratch(&format!("corrupt-{name}"));
            let mut doc = Document::new();
            let mut journal = Journal::new();
            let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
            let mut parent = ObjectId::NIL;
            let mut good = capture(&doc, &journal);
            for i in 0..20u64 {
                let node = id(i + 1);
                let batch = insert_batch(node, parent, i as f32, doc.revision, 700 + i, None);
                wal_apply(
                    &mut wal,
                    &mut doc,
                    &mut journal,
                    &batch,
                    &CommandMeta::significant("insert"),
                    i,
                )
                .expect("applies");
                parent = node;
                if i == 18 {
                    good = capture(&doc, &journal);
                }
            }
            assert_eq!(doc.revision, 20);
            drop(wal);
            let path = dir.join(WAL_FILE_NAME);
            if mangle == 0 {
                let mut bytes = std::fs::read(&path).expect("log bytes");
                let last = bytes.len() - 3;
                bytes[last] ^= 0xFF;
                std::fs::write(&path, &bytes).expect("mangled log");
            } else {
                let bytes = std::fs::read(&path).expect("log bytes");
                std::fs::write(&path, &bytes[..bytes.len() - 10]).expect("truncated log");
            }
            for round in 0..2 {
                let mut back = Document::new();
                let mut journal_back = Journal::new();
                let (_, report) = Wal::open(&dir, &mut back, &mut journal_back).expect("recovers");
                assert_eq!(back.revision, 19, "round {round} reopens to last good");
                assert_state_eq(&good, &back, &journal_back);
                if round == 0 {
                    assert!(report.truncated_bytes > 0, "tail loss is reported");
                }
            }
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn wal_fresh_and_torn_header_open() {
        let dir = scratch("fresh");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (_, report) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        assert_eq!(
            (report.revision, report.replayed, report.truncated_bytes),
            (0, 0, 0)
        );
        std::fs::write(dir.join(WAL_FILE_NAME), b"abc").expect("torn header");
        let mut back = Document::new();
        let mut journal_back = Journal::new();
        let (_, reopened) =
            Wal::open(&dir, &mut back, &mut journal_back).expect("torn header recovers");
        assert_eq!(reopened.truncated_bytes, 3);
        assert_eq!(back.revision, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_bad_snapshot_and_version_refuse() {
        let dir = scratch("refuse");
        std::fs::write(dir.join(SNAP_FILE_NAME), b"definitely not a snapshot").expect("bad snap");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        assert!(matches!(
            Wal::open(&dir, &mut doc, &mut journal),
            Err(WalError::Corrupt)
        ));
        // A future snapshot version refuses instead of guessing.
        let mut future = Vec::new();
        future.extend_from_slice(&SNAP_MAGIC);
        future.extend_from_slice(&999u32.to_le_bytes());
        std::fs::write(dir.join(SNAP_FILE_NAME), &future).expect("future snap");
        assert!(matches!(
            Wal::open(&dir, &mut doc, &mut journal),
            Err(WalError::UnsupportedVersion { got: 999 })
        ));
        // A future log version refuses as well.
        std::fs::remove_file(dir.join(SNAP_FILE_NAME)).unwrap();
        let mut header = [0u8; HEADER_LEN];
        header[0..4].copy_from_slice(&WAL_MAGIC);
        header[4..8].copy_from_slice(&999u32.to_le_bytes());
        std::fs::write(dir.join(WAL_FILE_NAME), header).expect("future log");
        assert!(matches!(
            Wal::open(&dir, &mut doc, &mut journal),
            Err(WalError::UnsupportedVersion { got: 999 })
        ));
        // A foreign file wearing the log name salvages to empty, never panics.
        std::fs::write(dir.join(WAL_FILE_NAME), vec![7u8; 64]).expect("foreign log");
        let (_, report) = Wal::open(&dir, &mut doc, &mut journal).expect("foreign log salvages");
        assert_eq!(report.revision, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_checkpoint_refuses_open_group() {
        let dir = scratch("open-group");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        wal_begin_group(&mut wal, &mut journal, &CommandMeta::significant("drag"), 2)
            .expect("begin");
        let batch = insert_batch(id(1), ObjectId::NIL, 0.0, 0, 1, None);
        wal_apply(
            &mut wal,
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::insignificant("tick"),
            0,
        )
        .expect("step");
        assert!(matches!(
            wal.checkpoint(&doc, &journal),
            Err(WalError::OpenGroup)
        ));
        wal_end_group(&mut wal, &mut journal)
            .expect("end")
            .expect("merged");
        let sealed = wal.checkpoint(&doc, &journal).expect("seals after close");
        assert_eq!(sealed.revision, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_end_group_failed_append_leaves_journal_untouched() {
        // Fault injection for the log-before-pop contract: the log file
        // handle is swapped for a read-only one, so the END marker write
        // fails and the journal must come back bit-identical.
        let dir = scratch("end-group-atomic");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        wal_begin_group(&mut wal, &mut journal, &CommandMeta::significant("drag"), 2)
            .expect("begin");
        let batch = insert_batch(id(1), ObjectId::NIL, 0.0, 0, 1, None);
        wal_apply(
            &mut wal,
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::insignificant("tick"),
            0,
        )
        .expect("step");
        let before = capture(&doc, &journal);
        assert_eq!(before.depth, 1, "one group open before the End");
        wal.file = std::fs::File::open(dir.join(WAL_FILE_NAME)).expect("read-only handle");
        let failed = wal_end_group(&mut wal, &mut journal);
        assert!(failed.is_err(), "END marker write must fail");
        assert_state_eq(&before, &doc, &journal);
        assert_eq!(open_group_depth(&journal), 1, "group still open");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_refused_apply_truncates_frame_and_reports_batch() {
        // Refused edit (duplicate id) truncates the frame it just wrote:
        // stamps, document and journal come back identical, and the caller
        // sees the typed batch cause.
        let dir = scratch("refused-truncates");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        let first = insert_batch(id(1), ObjectId::NIL, 0.0, 0, 1, None);
        wal_apply(
            &mut wal,
            &mut doc,
            &mut journal,
            &first,
            &CommandMeta::significant("insert"),
            0,
        )
        .expect("first applies");
        let mark = wal.end_mark();
        let stamps = wal.stamps().expect("stamps read");
        assert_eq!(stamps.len(), 1);
        let before = capture(&doc, &journal);
        // Fresh key but a repeated node id: the apply refuses after logging.
        let refused = insert_batch(id(1), ObjectId::NIL, 1.0, doc.revision, 2, None);
        let failed = wal_apply(
            &mut wal,
            &mut doc,
            &mut journal,
            &refused,
            &CommandMeta::significant("insert"),
            1,
        );
        assert!(
            matches!(failed, Err(WalError::Batch(_))),
            "refused edit reports its batch cause, got {failed:?}"
        );
        assert_eq!(wal.end_mark(), mark, "refused frame is truncated");
        assert_eq!(wal.stamps().expect("stamps").len(), stamps.len());
        assert_state_eq(&before, &doc, &journal);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_compensation_failures_are_typed() {
        // The truncate itself surfaces IO instead of swallowing it: a
        // read-only handle makes `forget_last` fail loudly.
        let dir = scratch("truncate-typed");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        wal.file = std::fs::File::open(dir.join(WAL_FILE_NAME)).expect("read-only handle");
        let failed = wal.forget_last(16);
        assert!(
            matches!(failed, Err(WalError::Io(_))),
            "truncate failure is a typed IO error, got {failed:?}"
        );
        // The new variants carry both causes with a source chain, and the
        // orphan-group refusal names the invariant instead of panicking.
        let io = std::io::Error::other("boom");
        let truncate = WalError::Truncate(io);
        assert!(truncate.to_string().contains("truncate"));
        assert!(std::error::Error::source(&truncate).is_some());
        let orphan = WalError::OrphanGroup;
        assert!(orphan.to_string().contains("retry key"));
        assert!(std::error::Error::source(&orphan).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wal_batch_truncate_reports_both_causes() {
        let batch = crate::commands::BatchError::NilId;
        let cleanup = std::io::Error::other("disk gone");
        let error = WalError::BatchTruncate { batch, cleanup };
        let text = error.to_string();
        assert!(text.contains("truncate"), "cleanup named, got {text}");
        assert!(text.contains("nil"), "batch cause named, got {text}");
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn wal_frame_size_bounded() {
        // One single-op apply frame stays tiny: per-edit overhead is O(ops).
        let batch = insert_batch(id(1), ObjectId::NIL, 0.0, 0, 1, None);
        let one = encode_apply(0, &batch, &CommandMeta::significant("insert"), 1).expect("encodes");
        assert!(one.len() < 512, "single-op frame is {} bytes", one.len());
        let mut ops = Vec::new();
        for i in 0..100u64 {
            ops.push(Op::SetLocal {
                id: id(1),
                local: pose(i as f32),
            });
        }
        let big = Batch {
            ops,
            base_revision: 0,
            idempotency_key: 2,
            provenance: None,
        };
        let hundred = encode_apply(0, &big, &CommandMeta::significant("bulk"), 1).expect("encodes");
        assert!(
            hundred.len() < 64 << 10,
            "100-op frame is {} bytes",
            hundred.len()
        );
        assert!(hundred.len() <= MAX_FRAME_BYTES);
        let marginal = (hundred.len() - one.len()) as f64 / 99.0;
        assert!(marginal < 200.0, "marginal {marginal:.1} bytes/op");
        // Past the frame cap the encoder refuses before anything is logged.
        let huge = Batch {
            ops: vec![Op::Remove { id: id(1) }; MAX_OPS + 1],
            base_revision: 0,
            idempotency_key: 3,
            provenance: None,
        };
        assert!(matches!(
            encode_apply(0, &huge, &CommandMeta::significant("x"), 1),
            Err(WalError::TooLarge)
        ));
    }

    #[test]
    fn wal_end_mark_pins_readers() {
        let dir = scratch("end-mark");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc, &mut journal).expect("fresh");
        let mut parent = ObjectId::NIL;
        for i in 0..5u64 {
            let node = id(i + 1);
            let batch = insert_batch(node, parent, i as f32, doc.revision, 300 + i, None);
            wal_apply(
                &mut wal,
                &mut doc,
                &mut journal,
                &batch,
                &CommandMeta::significant("insert"),
                i,
            )
            .expect("applies");
            parent = node;
        }
        assert!(wal.end_mark() > HEADER_LEN as u64);
        let live = capture(&doc, &journal);
        drop(wal);
        // Bytes written past the end mark (a kill mid-append) are ignored.
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(dir.join(WAL_FILE_NAME))
                .expect("append");
            file.write_all(&[0xA5u8; 100]).expect("junk");
            file.flush().expect("flush");
        }
        let mut back = Document::new();
        let mut journal_back = Journal::new();
        let (_, report) = Wal::open(&dir, &mut back, &mut journal_back).expect("recovers");
        assert_eq!(report.replayed, 5);
        assert_eq!(report.truncated_bytes, 100);
        assert_state_eq(&live, &back, &journal_back);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "timing: wall-clock cost gate, run under --perf"]
    fn wal_apply_cost_within_2x() {
        const N: u64 = 5_000;
        // Baseline: bare command-service applies with the same command meta.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut parent = ObjectId::NIL;
        let start = std::time::Instant::now();
        for i in 0..N {
            let node = id(i + 1);
            let batch = insert_batch(node, parent, i as f32, doc.revision, 1000 + i, None);
            crate::commands::apply_named(
                &mut doc,
                &mut journal,
                &batch,
                &CommandMeta::significant("insert"),
            )
            .expect("applies");
            parent = node;
        }
        let plain_ms = start.elapsed().as_secs_f64() * 1000.0;
        // WAL path: same edits through the logged wrappers.
        let dir = scratch("cost");
        let mut doc_w = Document::new();
        let mut journal_w = Journal::new();
        let (mut wal, _) = Wal::open(&dir, &mut doc_w, &mut journal_w).expect("fresh");
        let mut parent_w = ObjectId::NIL;
        let start = std::time::Instant::now();
        for i in 0..N {
            let node = id(i + 1);
            let batch = insert_batch(node, parent_w, i as f32, doc_w.revision, 1000 + i, None);
            wal_apply(
                &mut wal,
                &mut doc_w,
                &mut journal_w,
                &batch,
                &CommandMeta::significant("insert"),
                i,
            )
            .expect("applies");
            parent_w = node;
        }
        let wal_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(doc, doc_w, "logged edits reach the same document");
        assert_eq!(
            journal.past, journal_w.past,
            "logged edits keep the same journal"
        );
        let ratio = wal_ms / plain_ms.max(0.001);
        println!("wal-apply-cost plain={plain_ms:.1}ms wal={wal_ms:.1}ms ratio={ratio:.2}");
        assert!(
            wal_ms <= plain_ms * 2.0 + 5.0,
            "logged apply costs at most 2x: plain={plain_ms:.1}ms wal={wal_ms:.1}ms"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
