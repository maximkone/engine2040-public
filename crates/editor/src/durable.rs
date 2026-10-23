//! Durable journal: sealed document generations on disk.
//!
//! The consumer is the editor shell's save flow (crash-safe generations,
//! crash-recovery drill) and the host startup path, which calls
//! [`reopen_sealed`] to restore the document plus its idempotency map and
//! undo history after a restart (the endpoint wires that call).
//! The data owner is [`Document`](crate::commands::Document) plus its
//! [`Journal`](crate::commands::Journal).
//! Semantics mirror the `engine-save` atomic generation API (`write_atomic` /
//! `save_generation`: tmp file plus rename, oversize rejected before touching
//! the target, a failed write preserves the prior valid generation),
//! reimplemented here with `std::fs` only so this lane needs no new crate
//! edge. `engine-save` is a read-only reference: it is never edited here.
//!
//! Each generation seals one full [`save_to_bytes`](crate::save::save_to_bytes)
//! snapshot plus the command-service journal (both undo and redo stacks) in a
//! frame holding the magic, a version byte, the generation, the payload
//! length, the payload and a CRC32 over the payload. Version 1 payloads hold
//! the document bytes only and still open (with an empty journal); version 2
//! payloads prefix the document bytes with a length-prefixed journal record
//! section (see [`JOURNAL_RECORD_VERSION`]), so one sealed file always
//! recovers a consistent pair: the document plus the idempotency map and the
//! undo history that produced it. [`persist_generation`] seals the document
//! with empty stacks; [`persist_sealed`] and [`apply_persisted`] seal both.
//! [`reopen_latest`] recovers the document as before, while [`reopen_sealed`]
//! also rebuilds the journal: replaying a stored idempotency key after
//! rehydrate returns the recorded receipt without re-applying (exact-duplicate
//! seals collapse to one entry and report through [`Reopened::deduped`]).
//! [`persist_sealed`]
//! writes the frame to `<name>.tmp` and renames it to `gen-{n:08}.edoc`,
//! which is atomic on one filesystem: a kill mid-write leaves a `.tmp`
//! leftover or a torn file at worst. [`reopen_sealed`] ignores `.tmp` files
//! and skips corrupt generations with one line each in
//! [`Reopened::skipped`], so reopen always recovers the last sealed valid
//! generation. Corrupt files are skipped, never fatal, and never panic.

use std::fmt;
use std::path::Path;

use engine_scene::ids::ObjectId;
use engine_scene::provenance::{Provenance, MAX_PROVENANCE_BYTES};
use engine_scene::transform::Transform;

use crate::commands::{
    apply_named, document_digest, Applied, Batch, BatchError, BondStrength, CommandMeta, Document,
    Journal, JournalEntry, MaterialName, Op, JOURNAL_CAP, MAX_MATERIAL_NAME_LEN,
};
use crate::save::{crc32, load_from_bytes, push_id, push_transform, save_to_bytes, MAX_SAVE_BYTES};

/// Frame magic for durable journal generations. Distinct from both the
/// editor save magic (`EDOC`) and the game-save magic (`E204`).
pub const JOURNAL_MAGIC: [u8; 4] = *b"EEDJ";
/// Newest journal frame version this module writes and reads: document plus
/// journal record section (see [`JOURNAL_RECORD_VERSION`]).
pub const JOURNAL_VERSION: u8 = 2;
/// Oldest journal frame version this module still opens: version 1 frames
/// hold the document bytes only and reopen with an empty journal.
pub const MIN_JOURNAL_VERSION: u8 = 1;
/// Layout version of the journal record section inside version 2 payloads (a
/// `u32` head word before the undo/redo stacks). Bumped when the entry layout
/// below changes; readers accept the older layout and refuse unknown ones
/// instead of guessing. Version 2 appends the three content digests to every
/// entry; version 1 entries reopen with zero digests (unknown, not invented).
const JOURNAL_RECORD_VERSION: u32 = 2;
/// Oldest record layout this module still opens.
const MIN_RECORD_VERSION: u32 = 1;
/// Largest accepted encoded journal section: the capped live stacks (see
/// [`JOURNAL_CAP`]) with provenance, effects and ops each land far below
/// this; bigger seals refuse before touching the filesystem.
pub const MAX_JOURNAL_SECTION_BYTES: usize = 4 << 20;
/// Largest accepted sealed generation: the biggest save plus the biggest
/// journal section plus frame overhead (17 header bytes + 4 section-length
/// bytes + 4 checksum bytes, with headroom).
pub const MAX_GENERATION_BYTES: usize = MAX_SAVE_BYTES + MAX_JOURNAL_SECTION_BYTES + 64;
/// Cap for entries per sealed stack (a corrupt length field past this fails
/// bounded; live stacks never exceed [`JOURNAL_CAP`]).
const MAX_SEALED_STACK: usize = 4_096;
/// Cap for ops in one sealed vec (a corrupt count past this fails bounded).
const MAX_SEALED_OPS: usize = 65_536;
/// Cap for inverse groups per entry (one group per forward op at most).
const MAX_SEALED_GROUPS: usize = 65_536;
/// Cap for effects lines per entry (a corrupt count past this fails bounded).
const MAX_SEALED_EFFECTS: usize = 65_536;
/// Cap for folded keys per entry (merged groups keep their members' keys).
const MAX_SEALED_KEYS: usize = 4_096;
/// Cap for one effects line or entry name in bytes (names seal at 128 by
/// construction; the bound only bites on corrupt length fields).
const MAX_SEALED_TEXT: usize = 4_096;
/// Cap for one sealed component's canonical bytes (schemas stay under 200).
const MAX_SEALED_COMPONENT: usize = 256;
/// Suffix for in-flight frames. A kill between write and rename leaves these
/// behind; reopen ignores them.
pub const TMP_SUFFIX: &str = ".tmp";

/// What rejected a journal frame. Used internally: [`reopen_latest`] skips
/// bad generations rather than failing, so the last sealed valid one wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameError {
    /// Magic mismatch.
    BadMagic,
    /// Version outside the supported range.
    BadVersion,
    /// Short input or a length field that does not match the input.
    Truncated,
    /// CRC32 mismatch over the payload.
    CrcMismatch,
    /// Generation file holds more than the bounded cap.
    TooLarge,
    /// Journal record section with an unsupported layout version.
    BadRecord,
    /// Journal record section that fails to decode (bad tag, text, id, pose,
    /// material or provenance, or trailing bytes past the sealed end).
    BadJournal,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("bad journal frame magic"),
            Self::BadVersion => f.write_str("unsupported journal frame version"),
            Self::Truncated => f.write_str("truncated journal frame"),
            Self::CrcMismatch => f.write_str("journal frame checksum mismatch"),
            Self::TooLarge => f.write_str("journal frame exceeds its size bound"),
            Self::BadRecord => f.write_str("unsupported journal record version"),
            Self::BadJournal => f.write_str("bad journal record section"),
        }
    }
}

/// File name for a sealed generation (`gen-00000007.edoc`).
pub fn generation_file_name(generation: u64) -> String {
    format!("gen-{generation:08}.edoc")
}

/// Seal one document generation and store it atomically in `dir`.
///
/// Seals through [`persist_sealed`] with empty undo/redo stacks: the document
/// recovers exactly as before, and [`reopen_sealed`] reports an empty
/// journal. Prefer [`persist_sealed`] (or [`apply_persisted`]) on paths that
/// must survive a restart with their idempotency map intact.
pub fn persist_generation(dir: &Path, generation: u64, doc: &Document) -> std::io::Result<()> {
    persist_sealed(dir, generation, doc, &Journal::new())
}

/// Seal one document generation plus its command-service journal and store
/// the pair atomically in `dir`.
///
/// Either half is rejected before the filesystem is touched when it exceeds
/// its size bound, so a failed seal (oversize input or IO error) leaves the
/// prior valid generations intact. Same-filesystem rename keeps the
/// publication atomic: readers see the old or the new generation, never a
/// torn one. Both stacks seal (applied and undone batches), so rehydrate
/// replays recorded receipts and refuses retried undone keys exactly like
/// the live journal; open groups and coalescing gestures never seal (they are
/// transient UI state and reopen closed by construction).
pub fn persist_sealed(
    dir: &Path,
    generation: u64,
    doc: &Document,
    journal: &Journal,
) -> std::io::Result<()> {
    let save = save_to_bytes(doc);
    if save.len() > MAX_SAVE_BYTES {
        return Err(invalid("document exceeds MAX_SAVE_BYTES"));
    }
    let section = encode_journal(journal)?;
    if section.len() > MAX_JOURNAL_SECTION_BYTES {
        return Err(invalid("journal exceeds MAX_JOURNAL_SECTION_BYTES"));
    }
    // Both halves are bounded above, so these conversions hold; the explicit
    // form keeps oversize inputs loud instead of wrapping or panicking.
    let section_len = u32::try_from(section.len())
        .map_err(|_| invalid("journal exceeds MAX_JOURNAL_SECTION_BYTES"))?;
    let payload_len = u32::try_from(4 + section.len() + save.len())
        .map_err(|_| invalid("generation exceeds MAX_GENERATION_BYTES"))?;
    let mut payload = Vec::with_capacity(4 + section.len() + save.len());
    payload.extend_from_slice(&section_len.to_le_bytes());
    payload.extend_from_slice(&section);
    payload.extend_from_slice(&save);
    if payload.len() > MAX_GENERATION_BYTES - 21 {
        return Err(invalid("generation exceeds MAX_GENERATION_BYTES"));
    }
    let mut frame = Vec::with_capacity(17 + payload.len() + 4);
    frame.extend_from_slice(&JOURNAL_MAGIC);
    frame.push(JOURNAL_VERSION);
    frame.extend_from_slice(&generation.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload);
    frame.extend_from_slice(&crc32(&payload).to_le_bytes());
    write_atomic(&dir.join(generation_file_name(generation)), &frame)
}

/// Atomic write to any file path: tmp file synced to disk, then rename, then
/// the directory synced (a crash never leaves half a file; the prior valid
/// file survives a failed write). Mirrors `engine-save`'s `write_atomic`
/// (see the module note) for paths outside the generation naming scheme.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| invalid("path has no file name"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(TMP_SUFFIX);
    let tmp = path.with_file_name(tmp_name);
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        // Flush before the rename publishes the bytes. The handle closes at
        // the end of this block, which Windows needs before the rename.
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        sync_dir(dir);
    }
    Ok(())
}

/// Flushes the directory entry that a rename just created. Best effort:
/// Windows refuses to open a directory for sync, and a seal whose generation
/// is already in place must not fail over it, so every error is ignored here.
fn sync_dir(dir: &Path) {
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Oversize or invalid seal input: rejected before touching the filesystem.
fn invalid(reason: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, reason)
}

/// Lowercase hex for one 32-byte digest (fixed 64 chars).
fn digest_hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Read one exact sealed generation, without the highest-valid selection
/// [`reopen_sealed`] makes. Used by the landed read-back: the generation the
/// caller just sealed is the only one that answers for this write.
fn read_generation(dir: &Path, generation: u64) -> std::io::Result<Option<(Document, Journal)>> {
    let path = dir.join(generation_file_name(generation));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if bytes.len() > MAX_GENERATION_BYTES {
        return Err(invalid("sealed generation exceeds MAX_GENERATION_BYTES"));
    }
    let (sealed, doc, journal) = open_frame(&bytes).map_err(|error| invalid(&error.to_string()))?;
    if sealed != generation {
        return Err(invalid(
            "sealed generation number does not match its file name",
        ));
    }
    Ok(Some((doc, journal)))
}

/// Read the sealed generation back and compare its content digest with the
/// staged one.
///
/// `before` is the digest the document had before the write. A generation
/// that still holds it acked without landing the staged bytes (`no_effect`);
/// any other difference is `landed_mismatch` and names both digests.
pub(crate) fn verify_landed(
    dir: &Path,
    generation: u64,
    before: [u8; 32],
    staged: [u8; 32],
) -> Result<[u8; 32], PersistError> {
    let Some((landed_doc, _)) = read_generation(dir, generation).map_err(PersistError::Io)? else {
        return Err(PersistError::Io(invalid(
            "sealed generation is missing after a successful seal",
        )));
    };
    let landed = document_digest(&landed_doc);
    if landed == before {
        return Err(PersistError::NoEffect);
    }
    if landed != staged {
        return Err(PersistError::LandedMismatch { staged, landed });
    }
    Ok(landed)
}

/// Widen a decoded `u32` length to `usize` without a numeric cast, matching
/// the crate's `TryFrom` conversion style: a `u32` always fits `usize` on
/// every supported target, and the saturating fallback fails the caller's
/// bound instead of wrapping.
fn wide_len(len: u32) -> usize {
    usize::try_from(len).unwrap_or(usize::MAX)
}

/// What rejected a persisted apply. Batch failures write nothing anywhere; a
/// failed seal leaves the prior valid generations (and the live in-memory
/// state) untouched.
#[derive(Debug)]
pub enum PersistError {
    /// The batch itself was refused; nothing was sealed, nothing ran.
    Batch(BatchError),
    /// The seal failed after a successful in-memory stage; the live document
    /// and journal are unchanged, prior generations intact.
    Io(std::io::Error),
    /// The staged content equals the document before the write: the store
    /// acked a write that changed nothing. Nothing was sealed or committed.
    NoEffect,
    /// The sealed generation reopened with different content than was staged:
    /// a store that acked without carrying the staged bytes. The live document
    /// and journal are unchanged.
    LandedMismatch {
        /// Digest of the content that was staged.
        staged: [u8; 32],
        /// Digest of the content the generation actually holds.
        landed: [u8; 32],
    },
}

impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Batch(error) => write!(f, "editor journal batch: {error}"),
            Self::Io(error) => write!(f, "editor journal io: {error}"),
            Self::NoEffect => {
                f.write_str("editor journal landed no change: the write acked the before state")
            }
            Self::LandedMismatch { staged, landed } => write!(
                f,
                "editor journal landed mismatch: staged {}, landed {}",
                digest_hex(staged),
                digest_hex(landed)
            ),
        }
    }
}

impl std::error::Error for PersistError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Batch(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::NoEffect | Self::LandedMismatch { .. } => None,
        }
    }
}

impl From<BatchError> for PersistError {
    fn from(error: BatchError) -> Self {
        Self::Batch(error)
    }
}

/// Apply a batch and seal the result as `generation` in `dir`.
///
/// This is the durable form of the command-service apply path
/// ([`apply_named`](crate::commands::apply_named)): validate, then atomic
/// apply, then seal the idempotency key plus recorded result and the applied
/// batch seal into the EEDJ journal. Stages on private clones first, so a
/// refused batch or a failed seal changes neither the live document and
/// journal nor the sealed generations — the host can retry with a fresh
/// generation number. The receipt's `after_digest` is the **read-back**: the
/// generation is reopened after the seal and its content digest compared with
/// the staged one. A batch that stages the before content is refused with
/// [`PersistError::NoEffect`] before sealing; a store that lands other bytes is
/// [`PersistError::LandedMismatch`] and the live state keeps the prior
/// generation. A replayed idempotency key returns the recorded receipt
/// (including its three digests) without sealing anything new.
/// Seal under the resulting revision (`doc.revision + 1`) so generations
/// track revisions one-to-one like the existing seal-every-edit flows; group
/// merges re-seal under the merged revision when they commit.
pub fn apply_persisted(
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
    meta: &CommandMeta,
    dir: &Path,
    generation: u64,
) -> Result<Applied, PersistError> {
    let before_revision = doc.revision;
    let before_digest = document_digest(doc);
    let mut staged_doc = doc.clone();
    let mut staged_journal = journal.clone();
    let mut applied = apply_named(&mut staged_doc, &mut staged_journal, batch, meta)?;
    if applied.revision != before_revision + 1 {
        // Idempotent replay: the recorded receipt, nothing new to seal.
        return Ok(applied);
    }
    let staged_digest = document_digest(&staged_doc);
    debug_assert_eq!(applied.before_digest, before_digest);
    debug_assert_eq!(applied.staged_digest, staged_digest);
    if staged_digest == before_digest {
        // The batch committed a new revision with identical content: the
        // store would ack a write that changed nothing. Refuse before sealing.
        return Err(PersistError::NoEffect);
    }
    persist_sealed(dir, generation, &staged_doc, &staged_journal).map_err(PersistError::Io)?;
    // The read-back is the outcome, not the seal's return: reopen exactly the
    // generation just written and compare its content with what was staged.
    let landed_digest = verify_landed(dir, generation, before_digest, staged_digest)?;
    *doc = staged_doc;
    *journal = staged_journal;
    applied.after_digest = landed_digest;
    Ok(applied)
}

/// Reopen the last sealed valid generation in `dir`.
///
/// Recovers the document through [`reopen_sealed`], dropping the journal and
/// the skip report: same selection as before (highest generation whose frame
/// and document both validate), same `Ok(None)` on empty or missing
/// directories. Hosts that must keep idempotency across restarts call
/// [`reopen_sealed`] instead.
pub fn reopen_latest(dir: &Path) -> std::io::Result<Option<(u64, Document)>> {
    Ok(reopen_sealed(dir)?.map(|open| (open.generation, open.doc)))
}

/// A sealed generation reopened with its journal: the host restores all three
/// into its command-service state at startup (endpoint wires that call in a
/// later packet; this type is the callable API for it).
#[derive(Debug)]
pub struct Reopened {
    /// Highest generation whose frame, journal record and document validate.
    pub generation: u64,
    /// Document at that generation.
    pub doc: Document,
    /// Undo/redo stacks at that generation: replays recorded receipts,
    /// conflicts on reused keys and refuses undone keys like the live journal.
    pub journal: Journal,
    /// One line per skipped generation file, sorted by file name
    /// (`editor journal: skipped <name> (<reason>)`); torn tails land here
    /// and never fail the reopen.
    pub skipped: Vec<String>,
    /// Exact-duplicate past entries collapsed on load (same key, same forward
    /// ops, same base revision); the oldest copy answers replays.
    pub deduped: usize,
}

/// Reopen the last sealed valid generation plus its journal in `dir`.
///
/// Ignores in-flight `.tmp` leftovers, skips corrupt or unreadable
/// generation files with one line each in [`Reopened::skipped`], and returns
/// the highest generation whose frame, journal record and document all
/// validate. `Ok(None)` when no sealed generation exists (including a missing
/// directory). Never panics on any directory content.
pub fn reopen_sealed(dir: &Path) -> std::io::Result<Option<Reopened>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut best: Option<(u64, Document, Journal, usize)> = None;
    let mut skipped: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(TMP_SUFFIX) {
            continue;
        }
        let generation = match parse_generation(&name) {
            Some(generation) => generation,
            None => continue,
        };
        let bytes = match std::fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(_) => {
                skipped.push((name, "unreadable generation file".to_owned()));
                continue;
            }
        };
        let (sealed, doc, journal) = match open_frame(&bytes) {
            Ok(opened) => opened,
            Err(error) => {
                skipped.push((name, error.to_string()));
                continue;
            }
        };
        if sealed != generation {
            skipped.push((name, "generation number mismatch".to_owned()));
            continue;
        }
        // Exact-duplicate seals (a crash between apply and seal can seal one
        // batch twice) collapse to the oldest copy, and stacks re-cap at the
        // live bound with the newest kept, mirroring live eviction.
        let (past, deduped) = dedup_past(journal.past);
        let mut rehydrated = Journal::new();
        rehydrated.past = cap_stack(past);
        rehydrated.future = cap_stack(journal.future);
        if best
            .as_ref()
            .is_none_or(|(held, _, _, _)| generation > *held)
        {
            best = Some((generation, doc, rehydrated, deduped));
        }
    }
    // Sort by file name so the report reads the same on every platform and
    // run: directory order is unspecified.
    skipped.sort();
    let skipped = skipped
        .into_iter()
        .map(|(name, reason)| format!("editor journal: skipped {name} ({reason})"))
        .collect();
    Ok(best.map(|(generation, doc, journal, deduped)| Reopened {
        generation,
        doc,
        journal,
        skipped,
        deduped,
    }))
}

/// Install a reopened generation into live memory (no IO).
///
/// The caller reads the sealed pair with [`reopen_sealed`], then hands it
/// here; the document plus both journal stacks land in the command service
/// so replayed keys answer recorded receipts and undo keeps working.
pub fn rehydrate(doc: &mut Document, journal: &mut Journal, opened: Reopened) -> u64 {
    let generation = opened.generation;
    let Reopened {
        doc: sealed,
        journal: sealed_journal,
        ..
    } = opened;
    journal.rehydrate(sealed_journal.past, sealed_journal.future);
    *doc = sealed;
    generation
}

/// Drop exact-duplicate entries from `past` (same key, same forward ops, same
/// base revision), keeping the oldest: the survivor still answers its key
/// with the recorded receipt, and undo steps over it once. Returns the kept
/// stack plus how many entries collapsed.
fn dedup_past(past: Vec<JournalEntry>) -> (Vec<JournalEntry>, usize) {
    let mut kept: Vec<JournalEntry> = Vec::new();
    let mut dropped = 0usize;
    for entry in past {
        // Keys compare first: duplicates are rare, so the full op-vec
        // compare almost never runs.
        let known = kept.iter().any(|seen: &JournalEntry| {
            seen.key == entry.key
                && seen.base_revision == entry.base_revision
                && seen.forward == entry.forward
        });
        if known {
            dropped += 1;
        } else {
            kept.push(entry);
        }
    }
    (kept, dropped)
}

/// Cap a rehydrated stack at the live bound, forgetting the oldest first —
/// the same eviction the command service applies on every apply.
fn cap_stack(mut stack: Vec<JournalEntry>) -> Vec<JournalEntry> {
    if stack.len() > JOURNAL_CAP {
        stack.drain(..stack.len() - JOURNAL_CAP);
    }
    stack
}

/// Parse `gen-{n}.edoc` into its generation number. Anything else is skipped.
fn parse_generation(name: &str) -> Option<u64> {
    let stem = name.strip_prefix("gen-")?.strip_suffix(".edoc")?;
    if stem.is_empty() || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    stem.parse::<u64>().ok()
}

/// Open and verify one sealed frame into its generation, document and
/// journal. Version 1 frames hold the document bytes only and reopen with an
/// empty journal; version 2 frames prefix them with the journal record
/// section. A corrupt journal section fails the whole generation (never a
/// partial pair): the previous sealed generation wins instead.
fn open_frame(bytes: &[u8]) -> Result<(u64, Document, Journal), FrameError> {
    if bytes.len() > MAX_GENERATION_BYTES {
        return Err(FrameError::TooLarge);
    }
    if bytes.len() < 21 {
        return Err(FrameError::Truncated);
    }
    if bytes[0..4] != JOURNAL_MAGIC {
        return Err(FrameError::BadMagic);
    }
    let version = bytes[4];
    if !(MIN_JOURNAL_VERSION..=JOURNAL_VERSION).contains(&version) {
        return Err(FrameError::BadVersion);
    }
    let mut generation_bytes = [0u8; 8];
    generation_bytes.copy_from_slice(&bytes[5..13]);
    let generation = u64::from_le_bytes(generation_bytes);
    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&bytes[13..17]);
    let len = wide_len(u32::from_le_bytes(len_bytes));
    if len > MAX_SAVE_BYTES + MAX_JOURNAL_SECTION_BYTES + 4 || bytes.len() != 17 + len + 4 {
        return Err(FrameError::Truncated);
    }
    let payload = &bytes[17..17 + len];
    let mut crc_bytes = [0u8; 4];
    crc_bytes.copy_from_slice(&bytes[17 + len..21 + len]);
    if u32::from_le_bytes(crc_bytes) != crc32(payload) {
        return Err(FrameError::CrcMismatch);
    }
    if version == 1 {
        let doc = load_from_bytes(payload).map_err(|_| FrameError::Truncated)?;
        return Ok((generation, doc, Journal::new()));
    }
    if payload.len() < 4 {
        return Err(FrameError::Truncated);
    }
    let mut section_len_bytes = [0u8; 4];
    section_len_bytes.copy_from_slice(&payload[0..4]);
    let section_len = wide_len(u32::from_le_bytes(section_len_bytes));
    if section_len > MAX_JOURNAL_SECTION_BYTES || payload.len() < 4 + section_len {
        return Err(FrameError::Truncated);
    }
    let journal = decode_journal(&payload[4..4 + section_len])?;
    let doc = load_from_bytes(&payload[4 + section_len..]).map_err(|_| FrameError::Truncated)?;
    Ok((generation, doc, journal))
}

/// Encode both journal stacks into a record section: a record version head
/// word, then the past stack, then the future stack, each oldest entry first.
/// Field order here and in [`Cursor::entry`] must match exactly.
fn encode_journal(journal: &Journal) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&JOURNAL_RECORD_VERSION.to_le_bytes());
    push_stack(&mut out, &journal.past, true)?;
    push_stack(&mut out, &journal.future, true)?;
    Ok(out)
}

/// Decode a record section, checking the layout version, every bound and
/// exact consumption: trailing bytes are torn writes, never ours.
fn decode_journal(bytes: &[u8]) -> Result<Journal, FrameError> {
    let mut cursor = Cursor { bytes, pos: 0 };
    let record_version = u32::from_le_bytes(cursor.num::<4>()?);
    if !(MIN_RECORD_VERSION..=JOURNAL_RECORD_VERSION).contains(&record_version) {
        return Err(FrameError::BadRecord);
    }
    let with_digests = record_version >= 2;
    let past = cursor.stack(MAX_SEALED_STACK, with_digests)?;
    let future = cursor.stack(MAX_SEALED_STACK, with_digests)?;
    if cursor.pos != bytes.len() {
        return Err(FrameError::BadJournal);
    }
    let mut journal = Journal::new();
    journal.past = past;
    journal.future = future;
    Ok(journal)
}

/// Push one sealed stack: count plus entries in journal order (oldest first).
fn push_stack(
    out: &mut Vec<u8>,
    stack: &[JournalEntry],
    with_digests: bool,
) -> std::io::Result<()> {
    // A `u32` count needs 4B entries, far past any sealed stack: saturate to
    // a count the loader rejects instead of truncating silently.
    let count =
        u32::try_from(stack.len()).map_err(|_| invalid("journal stack exceeds its bound"))?;
    out.extend_from_slice(&count.to_le_bytes());
    for entry in stack {
        push_entry(out, entry, with_digests)?;
    }
    Ok(())
}

/// Push one journal entry: retry identity and revisions, the display name and
/// flag, the forward and inverse ops, the per-op inverse groups, the recorded
/// effects lines, every folded idempotency key, and the provenance record
/// through its own codec roundtrip. Version 2 entries append the three content
/// digests after the provenance record, so version 1 bytes still parse.
fn push_entry(out: &mut Vec<u8>, entry: &JournalEntry, with_digests: bool) -> std::io::Result<()> {
    out.extend_from_slice(&entry.key.to_le_bytes());
    out.extend_from_slice(&entry.base_revision.to_le_bytes());
    out.extend_from_slice(&entry.result_revision.to_le_bytes());
    out.extend_from_slice(&entry.group.to_le_bytes());
    out.push(u8::from(entry.significant));
    push_text(out, entry.name.as_bytes());
    push_ops(out, &entry.forward)?;
    push_ops(out, &entry.inverse)?;
    let groups = u32::try_from(entry.inverse_groups.len())
        .map_err(|_| invalid("journal entry exceeds its bound"))?;
    out.extend_from_slice(&groups.to_le_bytes());
    for group in &entry.inverse_groups {
        push_ops(out, group)?;
    }
    let effects = u32::try_from(entry.effects.len())
        .map_err(|_| invalid("journal entry exceeds its bound"))?;
    out.extend_from_slice(&effects.to_le_bytes());
    for line in &entry.effects {
        push_text(out, line.as_bytes());
    }
    let keys =
        u32::try_from(entry.keys.len()).map_err(|_| invalid("journal entry exceeds its bound"))?;
    out.extend_from_slice(&keys.to_le_bytes());
    for key in &entry.keys {
        out.extend_from_slice(&key.to_le_bytes());
    }
    let record = entry
        .provenance
        .encode()
        .map_err(|_| invalid("journal provenance record exceeds its bound"))?;
    let record_len = u32::try_from(record.len())
        .map_err(|_| invalid("journal provenance record exceeds its bound"))?;
    out.extend_from_slice(&record_len.to_le_bytes());
    out.extend_from_slice(&record);
    if with_digests {
        out.extend_from_slice(&entry.before_digest);
        out.extend_from_slice(&entry.staged_digest);
        out.extend_from_slice(&entry.after_digest);
    }
    Ok(())
}

/// Push one op vec: count plus tagged ops.
fn push_ops(out: &mut Vec<u8>, ops: &[Op]) -> std::io::Result<()> {
    let count =
        u32::try_from(ops.len()).map_err(|_| invalid("journal op vec exceeds its bound"))?;
    out.extend_from_slice(&count.to_le_bytes());
    for op in ops {
        push_op(out, op);
    }
    Ok(())
}

/// Push one op with the same tags as the command-service artifact digest (0
/// insert, 1 remove, 2 set-local, 3 reparent, 4 set-material, 5 set-component,
/// 6 remove-component, 7 put-material, 8 set-material-param, 9
/// remove-last-material, 10 set-bond): ids ride hyphenated (36 bytes, like the
/// save codec, so no new id conversion), poses ride as ten little-endian f32
/// values and material record texts as length-prefixed bytes.
fn push_op(out: &mut Vec<u8>, op: &Op) {
    match op {
        Op::Insert { id, parent, local } => {
            out.push(0);
            push_id(out, *id);
            push_id(out, *parent);
            push_transform(out, *local);
        }
        Op::Remove { id } => {
            out.push(1);
            push_id(out, *id);
        }
        Op::SetLocal { id, local } => {
            out.push(2);
            push_id(out, *id);
            push_transform(out, *local);
        }
        Op::Reparent { id, new_parent } => {
            out.push(3);
            push_id(out, *id);
            push_id(out, *new_parent);
        }
        Op::SetMaterial { id, material } => {
            out.push(4);
            push_id(out, *id);
            match material {
                Some(name) => {
                    out.push(1);
                    push_text(out, name.as_str().as_bytes());
                }
                None => out.push(0),
            }
        }
        Op::SetComponent { id, component } => {
            out.push(5);
            push_id(out, *id);
            push_text(out, &component.to_bytes());
        }
        Op::RemoveComponent { id, kind } => {
            out.push(6);
            push_id(out, *id);
            out.push(kind.tag());
        }
        Op::PutMaterial { text } => {
            out.push(7);
            push_text(out, text.as_bytes());
        }
        Op::SetMaterialParam {
            material,
            path,
            value,
        } => {
            out.push(8);
            push_text(out, material.as_bytes());
            push_text(out, path.as_bytes());
            push_text(out, value.as_bytes());
        }
        Op::RemoveLastMaterial => out.push(9),
        Op::SetBond { id, a, b, strength } => {
            out.push(10);
            push_id(out, *id);
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
            match strength {
                Some(value) => {
                    out.push(1);
                    out.extend_from_slice(&u32::from(value.permille()).to_le_bytes());
                }
                None => out.push(0),
            }
        }
    }
}

/// Push length-prefixed bytes (`u32` length first).
fn push_text(out: &mut Vec<u8>, text: &[u8]) {
    // Sealed texts are bounded in memory (names, effects, provenances); the
    // saturating form keeps the saver total.
    let len = u32::try_from(text.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(text);
}

/// Bounded cursor over a record section: short reads fail as truncated, bad
/// content fails as a bad record, never as panics.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Take `count` bytes, advancing the cursor. Short reads fail bounded.
    fn take(&mut self, count: usize) -> Result<&'a [u8], FrameError> {
        let end = self.pos.saturating_add(count);
        if end > self.bytes.len() {
            return Err(FrameError::Truncated);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take(1)?[0])
    }

    /// Exactly `N` bytes for one little-endian integer (`u32` reads use
    /// `N = 4`, `u64` reads `N = 8`). One shape for every width keeps the
    /// reader free of near-identical per-width helpers.
    fn num<const N: usize>(&mut self) -> Result<[u8; N], FrameError> {
        let chunk = self.take(N)?;
        // `take(N)` guarantees the length, so the copy below cannot fail.
        let mut out = [0u8; N];
        out.copy_from_slice(chunk);
        Ok(out)
    }

    /// One hyphenated id (36 bytes, like the save codec). Unparseable text
    /// fails bounded.
    fn id(&mut self) -> Result<ObjectId, FrameError> {
        let chunk = self.take(36)?;
        let text = core::str::from_utf8(chunk).map_err(|_| FrameError::BadJournal)?;
        ObjectId::parse(text).map_err(|_| FrameError::BadJournal)
    }

    /// Ten little-endian f32 values; non-finite poses are corrupt input.
    fn transform(&mut self) -> Result<Transform, FrameError> {
        let mut pose = [0f32; 10];
        for slot in pose.iter_mut() {
            let chunk = self.take(4)?;
            *slot = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        let local = Transform::new(
            [pose[0], pose[1], pose[2]],
            [pose[3], pose[4], pose[5], pose[6]],
            [pose[7], pose[8], pose[9]],
        );
        if !local.is_finite() {
            return Err(FrameError::BadJournal);
        }
        Ok(local)
    }

    /// Length-prefixed bytes capped at `cap`; over-cap lengths fail bounded.
    fn text(&mut self, cap: usize) -> Result<&'a [u8], FrameError> {
        let len = wide_len(u32::from_le_bytes(self.num::<4>()?));
        if len > cap {
            return Err(FrameError::TooLarge);
        }
        self.take(len)
    }

    /// One tagged op; unknown tags fail bounded.
    fn op(&mut self) -> Result<Op, FrameError> {
        match self.u8()? {
            0 => Ok(Op::Insert {
                id: self.id()?,
                parent: self.id()?,
                local: self.transform()?,
            }),
            1 => Ok(Op::Remove { id: self.id()? }),
            2 => Ok(Op::SetLocal {
                id: self.id()?,
                local: self.transform()?,
            }),
            3 => Ok(Op::Reparent {
                id: self.id()?,
                new_parent: self.id()?,
            }),
            4 => {
                let id = self.id()?;
                match self.u8()? {
                    0 => Ok(Op::SetMaterial { id, material: None }),
                    1 => {
                        let raw = self.text(MAX_MATERIAL_NAME_LEN)?;
                        if raw.is_empty() {
                            return Err(FrameError::BadJournal);
                        }
                        let name_text =
                            core::str::from_utf8(raw).map_err(|_| FrameError::BadJournal)?;
                        let material = MaterialName::try_from(name_text)
                            .map_err(|_| FrameError::BadJournal)?;
                        Ok(Op::SetMaterial {
                            id,
                            material: Some(material),
                        })
                    }
                    _ => Err(FrameError::BadJournal),
                }
            }
            5 => {
                let id = self.id()?;
                let raw = self.text(MAX_SEALED_COMPONENT)?;
                let component = engine_scene::component::Component::from_bytes(raw)
                    .map_err(|_| FrameError::BadJournal)?;
                Ok(Op::SetComponent { id, component })
            }
            6 => {
                let id = self.id()?;
                let kind = engine_scene::component::ComponentKind::from_tag(self.u8()?)
                    .ok_or(FrameError::BadJournal)?;
                Ok(Op::RemoveComponent { id, kind })
            }
            7 => Ok(Op::PutMaterial {
                text: self.op_text(MAX_SEALED_TEXT)?,
            }),
            8 => Ok(Op::SetMaterialParam {
                material: self.op_text(MAX_SEALED_TEXT)?,
                path: self.op_text(MAX_SEALED_TEXT)?,
                value: self.op_text(MAX_SEALED_TEXT)?,
            }),
            9 => Ok(Op::RemoveLastMaterial),
            10 => {
                let id = self.id()?;
                let a = u32::from_le_bytes(self.num::<4>()?);
                let b = u32::from_le_bytes(self.num::<4>()?);
                let strength = match self.u8()? {
                    0 => None,
                    1 => {
                        let raw = u32::from_le_bytes(self.num::<4>()?);
                        Some(BondStrength::new(raw).map_err(|_| FrameError::BadJournal)?)
                    }
                    _ => return Err(FrameError::BadJournal),
                };
                Ok(Op::SetBond { id, a, b, strength })
            }
            _ => Err(FrameError::BadJournal),
        }
    }

    /// Length-prefixed UTF-8 text capped at `cap`, owned for op fields. Bad
    /// bytes or an over-cap length fail bounded.
    fn op_text(&mut self, cap: usize) -> Result<String, FrameError> {
        let raw = self.text(cap)?;
        core::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| FrameError::BadJournal)
    }

    /// One op vec; over-cap counts fail bounded without pre-allocating.
    fn op_vec(&mut self) -> Result<Vec<Op>, FrameError> {
        let count = wide_len(u32::from_le_bytes(self.num::<4>()?));
        if count > MAX_SEALED_OPS {
            return Err(FrameError::TooLarge);
        }
        let mut ops = Vec::new();
        for _ in 0..count {
            ops.push(self.op()?);
        }
        Ok(ops)
    }

    /// One journal entry, in the exact field order [`push_entry`] writes.
    /// Version 2 records carry the three content digests after the provenance
    /// record; version 1 records stop there and leave them zero (unknown).
    fn entry(&mut self, with_digests: bool) -> Result<JournalEntry, FrameError> {
        let key = u64::from_le_bytes(self.num::<8>()?);
        let base_revision = u64::from_le_bytes(self.num::<8>()?);
        let result_revision = u64::from_le_bytes(self.num::<8>()?);
        let group = u64::from_le_bytes(self.num::<8>()?);
        let significant = match self.u8()? {
            0 => false,
            1 => true,
            _ => return Err(FrameError::BadJournal),
        };
        let name_raw = self.text(MAX_SEALED_TEXT)?;
        let name = core::str::from_utf8(name_raw)
            .map_err(|_| FrameError::BadJournal)?
            .to_owned();
        let forward = self.op_vec()?;
        let inverse = self.op_vec()?;
        let groups = wide_len(u32::from_le_bytes(self.num::<4>()?));
        if groups > MAX_SEALED_GROUPS {
            return Err(FrameError::TooLarge);
        }
        let mut inverse_groups = Vec::new();
        for _ in 0..groups {
            inverse_groups.push(self.op_vec()?);
        }
        let effects_count = wide_len(u32::from_le_bytes(self.num::<4>()?));
        if effects_count > MAX_SEALED_EFFECTS {
            return Err(FrameError::TooLarge);
        }
        let mut effects = Vec::new();
        for _ in 0..effects_count {
            let raw = self.text(MAX_SEALED_TEXT)?;
            effects.push(
                core::str::from_utf8(raw)
                    .map_err(|_| FrameError::BadJournal)?
                    .to_owned(),
            );
        }
        let keys_count = wide_len(u32::from_le_bytes(self.num::<4>()?));
        if keys_count > MAX_SEALED_KEYS {
            return Err(FrameError::TooLarge);
        }
        let mut keys = Vec::new();
        for _ in 0..keys_count {
            keys.push(u64::from_le_bytes(self.num::<8>()?));
        }
        let record_raw = self.text(MAX_PROVENANCE_BYTES)?;
        let provenance = Provenance::decode(record_raw).map_err(|_| FrameError::BadJournal)?;
        let (before_digest, staged_digest, after_digest) = if with_digests {
            (self.num::<32>()?, self.num::<32>()?, self.num::<32>()?)
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

    /// One sealed stack; over-cap counts fail bounded without pre-allocating.
    fn stack(&mut self, cap: usize, with_digests: bool) -> Result<Vec<JournalEntry>, FrameError> {
        let count = wide_len(u32::from_le_bytes(self.num::<4>()?));
        if count > cap {
            return Err(FrameError::TooLarge);
        }
        let mut stack = Vec::new();
        for _ in 0..count {
            stack.push(self.entry(with_digests)?);
        }
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{apply, redo, undo, Batch, BatchError, CommandMeta, Journal, Op};
    use crate::save::{load_from_bytes, save_to_bytes};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;
    use std::path::PathBuf;

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    /// Fresh scratch directory per test (parallel-safe: unique names).
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-editor-e3-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Document with `count` chained nodes over `count` applied batches, so
    /// the revision equals `count` and every generation differs.
    fn chained_doc(count: u64, key_base: u64) -> (Document, Journal) {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut parent = ObjectId::NIL;
        for i in 0..count {
            let id = ObjectId::new();
            let rev = doc.revision;
            apply(
                &mut doc,
                &mut journal,
                &Batch {
                    ops: vec![Op::Insert {
                        id,
                        parent,
                        local: pose_at(i as f32),
                    }],
                    base_revision: rev,
                    idempotency_key: key_base + i,
                    provenance: None,
                },
            )
            .unwrap();
            parent = id;
        }
        (doc, journal)
    }

    #[test]
    fn landed_digest_persisted_apply_reads_back_the_sealed_content() {
        let dir = scratch("landed-digest");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: pose_at(1.0),
            }],
            base_revision: 0,
            idempotency_key: 501,
            provenance: None,
        };
        let before = document_digest(&doc);
        let applied = apply_persisted(
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::default(),
            &dir,
            1,
        )
        .expect("persisted apply lands");
        assert_eq!(applied.before_digest, before);
        assert_ne!(
            applied.staged_digest, applied.before_digest,
            "an insert changes the content digest"
        );
        assert_eq!(
            applied.staged_digest, applied.after_digest,
            "the read-back equals what was staged"
        );

        let (sealed_doc, sealed_journal) = read_generation(&dir, 1)
            .expect("generation reads")
            .expect("generation exists");
        assert_eq!(document_digest(&sealed_doc), applied.after_digest);
        assert_eq!(
            sealed_journal
                .past
                .last()
                .expect("entry landed")
                .after_digest,
            applied.after_digest,
            "the sealed journal carries the same after digest"
        );

        // A replayed key answers from the journal entry, not a fresh digest.
        let replay = apply_persisted(
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::default(),
            &dir,
            1,
        )
        .expect("same key and payload replays");
        assert_eq!(replay.revision, applied.revision);
        assert_eq!(replay.before_digest, applied.before_digest);
        assert_eq!(replay.staged_digest, applied.staged_digest);
        assert_eq!(replay.after_digest, applied.after_digest);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn landed_digest_no_effect_batch_writes_no_generation() {
        let dir = scratch("landed-no-effect");
        let (mut doc, mut journal) = chained_doc(1, 6000);
        let Some(id) = doc.hierarchy.propagation_order().first().copied() else {
            panic!("chained doc holds one node");
        };
        let local = doc.hierarchy.local(id).expect("node has a local pose");
        let batch = Batch {
            ops: vec![Op::SetLocal { id, local }],
            base_revision: doc.revision,
            idempotency_key: 601,
            provenance: None,
        };
        let error = apply_persisted(
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::default(),
            &dir,
            1,
        )
        .expect_err("a write that changes nothing is refused");
        assert!(
            matches!(error, PersistError::NoEffect),
            "expected no_effect, got {error:?}"
        );
        assert_eq!(doc.revision, 1, "the live document did not move");
        assert!(
            !dir.join(generation_file_name(1)).exists(),
            "a refused no-effect write seals nothing"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn landed_digest_store_that_acks_without_the_staged_state_refuses() {
        let dir = scratch("landed-fake-store");
        let (doc, _) = chained_doc(1, 7000);
        let before = document_digest(&doc);
        let (changed, changed_journal) = chained_doc(2, 7100);
        let staged = document_digest(&changed);
        assert_ne!(before, staged);

        // A store that acks but keeps the before state: no_effect, not a pass.
        persist_sealed(&dir, 1, &doc, &Journal::new()).unwrap();
        let error = verify_landed(&dir, 1, before, staged).expect_err("before state is no effect");
        assert!(
            matches!(error, PersistError::NoEffect),
            "expected no_effect, got {error:?}"
        );

        // A store that lands some third state: landed_mismatch names both.
        persist_sealed(&dir, 2, &changed, &changed_journal).unwrap();
        let (other, other_journal) = chained_doc(3, 7200);
        persist_sealed(&dir, 3, &other, &other_journal).unwrap();
        let error = verify_landed(&dir, 3, before, staged).expect_err("other state is a mismatch");
        match error {
            PersistError::LandedMismatch {
                staged: got_staged,
                landed,
            } => {
                assert_eq!(got_staged, staged);
                assert_ne!(landed, staged);
                assert_eq!(landed, document_digest(&other));
            }
            other => panic!("expected landed_mismatch, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn landed_digest_save_reopen_diff_catches_one_nulled_ref() {
        // Every reference that a save carries: nodes, material bindings and
        // components. Two saves must keep all of them; one nulled binding must
        // make the diff say so.
        let (mut doc, mut journal) = chained_doc(2, 8000);
        let id = doc.hierarchy.propagation_order()[1];
        let record = doc
            .material_set
            .records()
            .first()
            .expect("the siege kit has records");
        let material = MaterialName::try_from(record.name.as_str()).expect("valid material name");
        let component = engine_scene::component::Component::Light(engine_scene::component::Light {
            kind: engine_scene::component::LightKind::Point,
            color: [1.0, 0.5, 0.0],
            intensity: 2.0,
            range: 5.0,
        });
        let batch = Batch {
            ops: vec![
                Op::SetMaterial {
                    id,
                    material: Some(material),
                },
                Op::SetComponent { id, component },
            ],
            base_revision: doc.revision,
            idempotency_key: 801,
            provenance: None,
        };
        apply(&mut doc, &mut journal, &batch).expect("binding and component land");

        let first = load_from_bytes(&save_to_bytes(&doc)).expect("first save reopens");
        let second = load_from_bytes(&save_to_bytes(&first)).expect("second save reopens");
        assert_eq!(document_digest(&doc), document_digest(&first));
        assert_eq!(
            document_digest(&doc),
            document_digest(&second),
            "a second save must not drop a reference"
        );

        let mut nulled = second.clone();
        nulled.materials.clear();
        assert_ne!(
            document_digest(&second),
            document_digest(&nulled),
            "one nulled binding must change the diff"
        );
    }

    #[test]
    fn landed_digest_v1_journal_record_opens_with_zero_digests() {
        let (_, journal) = chained_doc(1, 9000);
        let entry = journal.past.last().expect("one entry").clone();
        let mut section = Vec::new();
        section.extend_from_slice(&1u32.to_le_bytes());
        push_stack(&mut section, &journal.past, false).expect("v1 entries encode");
        push_stack(&mut section, &[], false).expect("empty future encodes");
        let decoded = decode_journal(&section).expect("v1 record opens");
        let seen = decoded.past.last().expect("entry survives");
        assert_eq!(seen.key, entry.key);
        assert_eq!(seen.result_revision, entry.result_revision);
        assert_eq!(seen.before_digest, [0u8; 32]);
        assert_eq!(seen.staged_digest, [0u8; 32]);
        assert_eq!(seen.after_digest, [0u8; 32]);
    }

    #[test]
    fn crash_point_reopen_recovers_last_sealed_generation() {
        let dir = scratch("crash-point");

        let (doc3, _) = chained_doc(3, 1000);
        persist_generation(&dir, 1, &chained_doc(1, 1000).0).unwrap();
        persist_generation(&dir, 2, &chained_doc(2, 1000).0).unwrap();
        persist_generation(&dir, 3, &doc3).unwrap();
        assert_eq!(reopen_latest(&dir).unwrap(), Some((3, doc3.clone())));

        // Crash point 1: kill between write and rename. The `.tmp` leftover
        // is ignored and reopen still recovers generation 3.
        let (doc4, _) = chained_doc(4, 1000);
        let mut torn = Vec::new();
        torn.extend_from_slice(&JOURNAL_MAGIC);
        torn.push(JOURNAL_VERSION);
        torn.extend_from_slice(&4u64.to_le_bytes());
        let payload = save_to_bytes(&doc4);
        torn.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        torn.extend_from_slice(&payload[..payload.len() / 2]);
        std::fs::write(dir.join("gen-00000004.edoc.tmp"), &torn).unwrap();
        assert_eq!(reopen_latest(&dir).unwrap(), Some((3, doc3.clone())));

        // Crash point 2: a torn sealed file (renamed but never flushed).
        // It fails validation, so reopen skips it and recovers generation 3.
        std::fs::write(dir.join(generation_file_name(4)), &torn).unwrap();
        assert_eq!(reopen_latest(&dir).unwrap(), Some((3, doc3.clone())));

        // The next clean seal publishes generation 4 over the torn file.
        persist_generation(&dir, 4, &doc4).unwrap();
        assert_eq!(reopen_latest(&dir).unwrap(), Some((4, doc4)));

        // Stray files (notes, mismatched generations, bad magic) never block
        // recovery of the last sealed generation.
        std::fs::write(dir.join("notes.txt"), b"not a generation").unwrap();
        std::fs::write(dir.join("gen-00000009.edoc"), b"bad magic here!!!").unwrap();
        let (held_generation, _) = reopen_latest(&dir).unwrap().unwrap();
        assert_eq!(held_generation, 4);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hundred_edit_journal_replays() {
        let dir = scratch("hundred-edits");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut parent = ObjectId::NIL;
        for i in 0..100u64 {
            let id = ObjectId::new();
            let rev = doc.revision;
            apply(
                &mut doc,
                &mut journal,
                &Batch {
                    ops: vec![Op::Insert {
                        id,
                        parent,
                        local: pose_at(i as f32),
                    }],
                    base_revision: rev,
                    idempotency_key: 5000 + i,
                    provenance: None,
                },
            )
            .unwrap();
            // Seal every edit: generation numbers track revisions 1..=100.
            persist_generation(&dir, doc.revision, &doc).unwrap();
            parent = id;
        }
        assert_eq!(doc.revision, 100);

        // All 100 generations sealed; reopen replays to the live document.
        let sealed = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|entry| entry.file_name().to_string_lossy().ends_with(".edoc"))
            })
            .count();
        assert_eq!(sealed, 100);
        let (generation, back) = reopen_latest(&dir).unwrap().unwrap();
        assert_eq!(generation, 100);
        assert_eq!(back, doc);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_write_preserves_prior_valid_generation() {
        let dir = scratch("failed-write");
        let (doc1, _) = chained_doc(1, 2000);
        persist_generation(&dir, 1, &doc1).unwrap();

        // IO failure (missing directory): the prior valid generation is
        // untouched and still reopens.
        let missing = dir.join("no-such-dir");
        let err = persist_generation(&missing, 2, &doc1).unwrap_err();
        assert_ne!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(reopen_latest(&dir).unwrap(), Some((1, doc1)));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_and_missing_directories_reopen_to_none() {
        let dir = scratch("empty");
        assert_eq!(reopen_latest(&dir).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(reopen_latest(&dir).unwrap(), None);
    }

    /// Two persisted applies, a restart, then the same keys replay their
    /// recorded receipts without new revisions; a reused key with another
    /// payload still conflicts.
    #[test]
    fn persisted_apply_rehydrates_and_replays_recorded_result() {
        let dir = scratch("persist-replay");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let meta = CommandMeta::default();
        let first_id = ObjectId::new();
        let first = Batch {
            ops: vec![Op::Insert {
                id: first_id,
                parent: ObjectId::NIL,
                local: pose_at(1.0),
            }],
            base_revision: 0,
            idempotency_key: 11,
            provenance: None,
        };
        let receipt = apply_persisted(&mut doc, &mut journal, &first, &meta, &dir, 1)
            .expect("first persisted apply seals generation 1");
        assert_eq!(doc.revision, 1);
        let second_id = ObjectId::new();
        let second = Batch {
            ops: vec![Op::Insert {
                id: second_id,
                parent: first_id,
                local: pose_at(2.0),
            }],
            base_revision: 1,
            idempotency_key: 12,
            provenance: None,
        };
        apply_persisted(&mut doc, &mut journal, &second, &meta, &dir, 2)
            .expect("second persisted apply seals generation 2");
        assert_eq!(doc.revision, 2);
        let live = doc.clone();

        // Restart: fresh memory rehydrates the pair from the sealed files.
        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("two sealed generations reopen");
        assert_eq!(open.generation, 2);
        assert!(open.skipped.is_empty());
        assert_eq!(open.deduped, 0);
        assert_eq!(open.doc, live);
        assert_eq!(open.journal.past_len(), 2);
        let mut doc = open.doc;
        let mut journal = open.journal;

        // Same key plus same payload replays the recorded receipt: no new
        // revision, no new journal entry, document untouched.
        let replayed =
            apply(&mut doc, &mut journal, &first).expect("stored key replays its recorded receipt");
        assert_eq!(replayed, receipt);
        assert_eq!(doc, live);
        assert_eq!(journal.past_len(), 2);

        // Same key with another payload is still a conflict after rehydrate.
        let conflict = Batch {
            ops: vec![Op::SetLocal {
                id: first_id,
                local: pose_at(8.0),
            }],
            base_revision: doc.revision,
            idempotency_key: 11,
            provenance: None,
        };
        assert_eq!(
            apply(&mut doc, &mut journal, &conflict),
            Err(BatchError::DuplicateKey { key: 11 })
        );
        assert_eq!(doc, live);

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
    }

    /// Undo and redo keep working after a restart: the rehydrated journal
    /// carries the inverses, and revisions keep counting.
    #[test]
    fn undo_redo_after_rehydrate_work() {
        let dir = scratch("persist-undo");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let meta = CommandMeta::default();
        let root = ObjectId::new();
        let leaf = ObjectId::new();
        for (i, (id, parent)) in [(root, ObjectId::NIL), (leaf, root)]
            .into_iter()
            .enumerate()
        {
            // Two loop turns: exact widening, no precision loss.
            let wide = u16::try_from(i).expect("test loop seals two batches");
            let batch = Batch {
                ops: vec![Op::Insert {
                    id,
                    parent,
                    local: pose_at(f32::from(wide)),
                }],
                base_revision: doc.revision,
                idempotency_key: 20 + u64::from(wide),
                provenance: None,
            };
            let generation = doc.revision + 1;
            apply_persisted(&mut doc, &mut journal, &batch, &meta, &dir, generation)
                .expect("persisted loop apply seals its generation");
        }
        let sealed = doc.clone();

        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("sealed pair reopens");
        let mut doc = open.doc;
        let mut journal = open.journal;
        assert_eq!(doc, sealed);

        undo(&mut doc, &mut journal).expect("rehydrated undo pops the leaf");
        assert_eq!(doc.revision, sealed.revision + 1);
        assert!(doc.hierarchy.contains(root));
        assert!(!doc.hierarchy.contains(leaf));
        redo(&mut doc, &mut journal).expect("rehydrated redo restores the leaf");
        assert_eq!(doc.hierarchy, sealed.hierarchy);
        assert_eq!(doc.revision, sealed.revision + 2);

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
    }

    /// A torn tail file (plus a bad-magic stray) never fails startup: the
    /// previous sealed generation wins and each skip reports one line.
    #[test]
    fn torn_tail_skips_with_one_line_reasons() {
        let dir = scratch("persist-torn");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let meta = CommandMeta::default();
        for wide in 0..2u16 {
            let batch = Batch {
                ops: vec![Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: pose_at(f32::from(wide)),
                }],
                base_revision: doc.revision,
                idempotency_key: 30 + u64::from(wide),
                provenance: None,
            };
            let generation = doc.revision + 1;
            apply_persisted(&mut doc, &mut journal, &batch, &meta, &dir, generation)
                .expect("persisted loop apply seals its generation");
        }
        let sealed = doc.clone();

        // Torn tail: half a frame under the next generation name, a bad-magic
        // stray past it, and an ignored in-flight leftover.
        let mut torn = Vec::new();
        torn.extend_from_slice(&JOURNAL_MAGIC);
        torn.push(JOURNAL_VERSION);
        torn.extend_from_slice(&3u64.to_le_bytes());
        let half = save_to_bytes(&doc);
        let half_len = u32::try_from(half.len()).expect("test save fits in a frame length");
        torn.extend_from_slice(&half_len.to_le_bytes());
        torn.extend_from_slice(&half[..half.len() / 2]);
        std::fs::write(dir.join(generation_file_name(3)), &torn).expect("test torn tail writes");
        std::fs::write(dir.join(generation_file_name(4)), b"bad magic here!!!")
            .expect("test bad-magic stray writes");
        std::fs::write(dir.join("gen-00000005.edoc.tmp"), &torn).expect("test tmp leftover writes");

        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("torn tail falls back");
        assert_eq!(open.generation, 2);
        assert_eq!(open.doc, sealed);
        // Sorted by file name; the `.tmp` leftover reports nothing.
        assert_eq!(open.skipped.len(), 2);
        assert!(open.skipped[0].contains("gen-00000003.edoc"));
        assert!(open.skipped[1].contains("gen-00000004.edoc"));
        for line in &open.skipped {
            assert!(line.starts_with("editor journal: skipped "));
            assert!(!line.contains('\n'));
        }

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
    }

    /// A batch sealed twice (crash between apply and seal, then a retry that
    /// re-sealed) collapses to one entry on load; its key still replays.
    #[test]
    fn duplicate_seals_dedup_on_load() {
        let dir = scratch("persist-dedup");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let meta = CommandMeta::default();
        let node = ObjectId::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: node,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            base_revision: 0,
            idempotency_key: 41,
            provenance: None,
        };
        let receipt = apply_persisted(&mut doc, &mut journal, &batch, &meta, &dir, 1)
            .expect("first persisted apply seals generation 1");
        // The retry sealed the same entry a second time under a fresh name.
        let mut doubled = journal.clone();
        let entry = doubled.past[0].clone();
        doubled.past.push(entry);
        persist_sealed(&dir, 2, &doc, &doubled).expect("doubled seal stores over generation 1");

        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("doubled seal reopens");
        assert_eq!(open.generation, 2);
        assert_eq!(open.deduped, 1);
        assert_eq!(open.journal.past_len(), 1);
        let mut doc = open.doc;
        let mut journal = open.journal;
        assert_eq!(
            apply(&mut doc, &mut journal, &batch).expect("stored key replays its recorded receipt"),
            receipt
        );
        // One undo clears the single surviving entry.
        undo(&mut doc, &mut journal).expect("deduped entry undoes once");
        assert!(journal.past.is_empty());

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
    }

    /// Material bindings and pose edits round-trip through the record
    /// section: the rehydrated document matches bit-for-bit, the stored key
    /// replays, and undo restores the binding.
    #[test]
    fn journal_roundtrips_material_and_pose_edits() {
        let dir = scratch("persist-material");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let meta = CommandMeta::default();
        let node = ObjectId::new();
        let hero = MaterialName::try_from("hero-mat").expect("test material name parses");
        let seed = Batch {
            ops: vec![Op::Insert {
                id: node,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            base_revision: 0,
            idempotency_key: 61,
            provenance: None,
        };
        apply_persisted(&mut doc, &mut journal, &seed, &meta, &dir, 1)
            .expect("seed apply seals generation 1");
        let bind = Batch {
            ops: vec![Op::SetMaterial {
                id: node,
                material: Some(hero),
            }],
            base_revision: 1,
            idempotency_key: 62,
            provenance: None,
        };
        let receipt = apply_persisted(&mut doc, &mut journal, &bind, &meta, &dir, 2)
            .expect("bind apply seals generation 2");
        let pose = Batch {
            ops: vec![Op::SetLocal {
                id: node,
                local: pose_at(4.0),
            }],
            base_revision: 2,
            idempotency_key: 63,
            provenance: None,
        };
        apply_persisted(&mut doc, &mut journal, &pose, &meta, &dir, 3)
            .expect("pose apply seals generation 3");
        let live = doc.clone();
        assert_eq!(live.materials.get(&node.to_string()), Some(&hero));

        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("material seals reopen");
        assert_eq!(open.generation, 3);
        assert!(open.skipped.is_empty());
        assert_eq!(open.doc, live);
        let mut doc = open.doc;
        let mut journal = open.journal;

        // The bind key replays its recorded receipt without re-applying.
        assert_eq!(
            apply(&mut doc, &mut journal, &bind)
                .expect("stored bind key replays its recorded receipt"),
            receipt
        );
        assert_eq!(doc, live);
        // Undo the pose, then the bind: the binding clears again.
        undo(&mut doc, &mut journal).expect("pose undoes");
        assert_eq!(doc.materials.get(&node.to_string()), Some(&hero));
        undo(&mut doc, &mut journal).expect("bind undoes");
        assert!(!doc.materials.contains_key(&node.to_string()));

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
    }

    /// Version 1 frames (document bytes only) still open, with an empty
    /// journal and no skips.
    #[test]
    fn v1_frames_still_open_with_empty_journal() {
        let dir = scratch("persist-v1");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let node = ObjectId::new();
        apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id: node,
                    parent: ObjectId::NIL,
                    local: pose_at(3.0),
                }],
                base_revision: 0,
                idempotency_key: 51,
                provenance: None,
            },
        )
        .expect("seed apply stages the v1 document");
        // Hand-seal the old layout: magic, version 1, generation, length,
        // document bytes, checksum.
        let payload = save_to_bytes(&doc);
        let mut frame = Vec::new();
        frame.extend_from_slice(&JOURNAL_MAGIC);
        frame.push(1);
        frame.extend_from_slice(&7u64.to_le_bytes());
        let payload_len = u32::try_from(payload.len()).expect("test save fits in a frame length");
        frame.extend_from_slice(&payload_len.to_le_bytes());
        frame.extend_from_slice(&payload);
        frame.extend_from_slice(&crc32(&payload).to_le_bytes());
        std::fs::write(dir.join(generation_file_name(7)), &frame).expect("test v1 frame writes");

        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("v1 frame opens");
        assert_eq!(open.generation, 7);
        assert_eq!(open.doc, doc);
        assert!(open.journal.is_empty());
        assert!(open.skipped.is_empty());
        assert_eq!(open.deduped, 0);

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
    }

    /// A sealed generation written twice produces identical file bytes, and
    /// reopening it recovers the pair with no tmp leftover on disk.
    #[test]
    fn sealed_generation_reloads_identical_bytes() {
        let dir = scratch("persist-bytes");
        let again = scratch("persist-bytes-again");
        let (doc, journal) = chained_doc(2, 3000);
        persist_sealed(&dir, 1, &doc, &journal).expect("first seal writes");
        persist_sealed(&again, 1, &doc, &journal).expect("second seal writes");
        let first = std::fs::read(dir.join(generation_file_name(1))).expect("sealed file reads");
        let second = std::fs::read(again.join(generation_file_name(1))).expect("sealed file reads");
        assert_eq!(first, second, "same pair seals to the same bytes");

        let open = reopen_sealed(&dir)
            .expect("test workspace reads back")
            .expect("sealed generation reopens");
        assert_eq!(open.generation, 1);
        assert_eq!(open.doc, doc);
        assert_eq!(open.journal.past_len(), journal.past_len());
        assert!(open.skipped.is_empty());
        assert!(!dir
            .join(format!("{}{TMP_SUFFIX}", generation_file_name(1)))
            .exists());

        std::fs::remove_dir_all(&dir).expect("test scratch dir removes");
        std::fs::remove_dir_all(&again).expect("test scratch dir removes");
    }
}
