//! Instant play and restart: play from the editor camera, snapshot a
//! round to opaque bytes, restart from a snapshot, replay the last 30 s.
//!
//! The consumer is the editor play button and its first integrations (the
//! play-checks, the tuning loop). The data owner is [`InstantPlay`], which
//! owns a round copy of the authoring [`Document`], a deterministic round
//! simulation and a bounded input tape. Only `std` plus the crates this lane
//! already uses (`engine-scene` through [`commands`](crate::commands)) appear:
//! no new dependency and no new cross-crate edge.
//!
//! The round simulation is intentionally small and integer-only (millimetres
//! and mm/tick, `+ - *` only, no floats in state): two fighters on a clamped
//! 1D track. Its encoded form is the opaque snapshot contract — bytes the
//! game produces and consumes while the editor stores and hands back without
//! arena types. That keeps the `engine-arena` edge out: the editor never
//! interprets round internals, it only seals, digests and restores them. The
//! day a real arena snapshot path exists, it can replace the payload behind
//! the same [`RoundSnapshot`] shape; the editor side does not change.
//!
//! Child reuse: [`play_from_camera`](InstantPlay::play_from_camera) launches a
//! [`ChildPlay`](crate::play::ChildPlay) with camera seed args and never
//! shares the live document with the child, so a killed game leaves the
//! authoring document intact. The input tape is the durable-journal
//! equivalent for rounds: a bounded window (30 s at [`TICK_HZ`]) that
//! re-simulates byte-identically on re-run.
//!
//! [`Document`]: crate::commands::Document

use std::collections::VecDeque;
use std::fmt;
use std::io::{Error, ErrorKind};

use engine_core::hash::{FNV64_OFFSET, GOLDEN_GAMMA};
use engine_core::splitmix64;

use crate::audit::{digest_parts, fold_pinned};
use crate::commands::{self, Applied, Batch, BatchError, Document, Journal};
use crate::play::{ChildPlay, PlayEntrypoint};
use crate::save::crc32;
use crate::viewport::Camera;

/// Fixed round rate: inputs are recorded and stepped at 60 Hz.
/// Single owner of the rate is `engine_core::TICK_HZ`.
pub const TICK_HZ: u64 = engine_core::TICK_HZ;
/// Replay window: the last 30 s of inputs at [`TICK_HZ`].
pub const REPLAY_WINDOW_TICKS: usize = 30 * TICK_HZ as usize;
/// Frame magic for round snapshots. Distinct from the editor save magic
/// (`EDOC`) and the durable journal magic (`EEDJ`): these are round states.
pub const SNAPSHOT_MAGIC: [u8; 4] = *b"EPL1";
/// Newest snapshot version this module writes and reads.
pub const SNAPSHOT_VERSION: u8 = 1;
/// Oldest snapshot version this module still opens.
pub const MIN_SNAPSHOT_VERSION: u8 = 1;
/// Largest accepted snapshot: the sealed payload is fixed at 49 bytes, so
/// anything past this cap is not ours.
pub const MAX_SNAPSHOT_BYTES: usize = 128;
/// Snapshot payload bytes: magic (4) + version (1) + revision (8) + tick (8)
/// + seed (8) + four i32 lanes (16) + CRC32 (4).
const SNAPSHOT_BYTES: usize = 49;
/// Per-tick acceleration step in mm/tick^2 per input unit.
const ACCEL_STEP: i32 = 20;
/// Speed clamp in mm/tick (300 mm/tick is 18 m/s at 60 Hz, already generous).
const MAX_SPEED: i32 = 300;
/// Track half-length in mm (±10 m); fighters clamp and stop at the walls.
const TRACK_MM: i32 = 10_000;
/// Input clamp: each lane accepts [-2, 2].
const MAX_INPUT: i8 = 2;

/// One tick of round input: per-fighter acceleration in units of
/// [`ACCEL_STEP`]. Built with [`RoundInput::new`], which clamps each lane to
/// `[-2, 2]` so out-of-range callers cannot push the sim out of its envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundInput {
    /// Fighter 0 acceleration, clamped to `[-2, 2]`.
    pub p0: i8,
    /// Fighter 1 acceleration, clamped to `[-2, 2]`.
    pub p1: i8,
}

impl RoundInput {
    /// Build an input, clamping each lane into `[-2, 2]`.
    pub fn new(p0: i8, p1: i8) -> Self {
        Self {
            p0: p0.clamp(-MAX_INPUT, MAX_INPUT),
            p1: p1.clamp(-MAX_INPUT, MAX_INPUT),
        }
    }

    /// Zero input: both fighters coast.
    pub fn neutral() -> Self {
        Self { p0: 0, p1: 0 }
    }
}

/// Deterministic round state: tick, seed and two integer fighters (position
/// in mm, velocity in mm/tick). Integer-only so re-simulation is bit-exact on
/// every machine; the tick saturates rather than wrapping past `u64::MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RoundState {
    tick: u64,
    seed: u64,
    p0: i32,
    v0: i32,
    p1: i32,
    v1: i32,
}

impl RoundState {
    /// Fresh round: tick zero, fighters spread from the seed (splitmix64
    /// expansion, non-aliasing across seeds), velocities zero.
    fn initial(seed: u64) -> Self {
        let spread = |salt: u64| (splitmix64(seed ^ salt) % 4001) as i32 - 2000;
        Self {
            tick: 0,
            seed,
            p0: spread(GOLDEN_GAMMA),
            v0: 0,
            // Distinct salt from lane 0; any other value re-seeds every round.
            p1: spread(0xBF58_476D_1CE4_E5B9),
            v1: 0,
        }
    }

    /// Step one tick: accelerate, clamp speed, integrate, clamp to the track
    /// (a wall stop zeroes that lane's velocity). Tick saturates at
    /// `u64::MAX`; inputs are already clamped by [`RoundInput::new`].
    fn step(&mut self, input: &RoundInput) {
        (self.v0, self.p0) = lane_next(self.v0, self.p0, input.p0);
        (self.v1, self.p1) = lane_next(self.v1, self.p1, input.p1);
        self.tick = self.tick.saturating_add(1);
    }
}

/// Next `(velocity, position)` for one fighter lane.
fn lane_next(vel: i32, pos: i32, input: i8) -> (i32, i32) {
    let vel = (vel + i32::from(input) * ACCEL_STEP).clamp(-MAX_SPEED, MAX_SPEED);
    let pos = pos + vel;
    if pos < -TRACK_MM {
        (0, -TRACK_MM)
    } else if pos > TRACK_MM {
        (0, TRACK_MM)
    } else {
        (vel, pos)
    }
}

/// What rejected a round snapshot. Every variant is bounded: a bad snapshot
/// fails, never panics and never leaves a partial round behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Magic mismatch: not a round snapshot.
    BadMagic,
    /// Version outside the supported range; carries what was found.
    BadVersion {
        /// Version stamped on the snapshot.
        got: u8,
    },
    /// Short input or a length that does not match the sealed size.
    Truncated,
    /// CRC32 mismatch over the payload.
    CrcMismatch,
    /// Snapshot past the bounded cap.
    TooLarge,
    /// The opaque bytes decode but do not match the snapshot's digest, tick
    /// or revision fields: tampered or mixed-up snapshot.
    DigestMismatch,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a round snapshot (bad magic)"),
            Self::BadVersion { got } => write!(
                f,
                "unsupported round snapshot v{got}; this reader opens v{MIN_SNAPSHOT_VERSION}..=v{SNAPSHOT_VERSION}"
            ),
            Self::Truncated => f.write_str("round snapshot is truncated"),
            Self::CrcMismatch => f.write_str("round snapshot failed its checksum (corrupt)"),
            Self::TooLarge => f.write_str("round snapshot exceeds its size bound"),
            Self::DigestMismatch => f.write_str("round snapshot digest does not match its bytes"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Opaque round snapshot: the tick plus bytes the game produces and the
/// editor stores without interpreting, plus a digest over those bytes. The
/// editor hands `opaque` back to [`InstantPlay::restart`] verbatim; field
/// reads (`tick`, `revision`, `digest`) are the only inspection it performs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundSnapshot {
    /// Round tick sealed in the snapshot.
    pub tick: u64,
    /// Round document revision sealed in the snapshot.
    pub revision: u64,
    /// Opaque game bytes: sealed round state, never interpreted here.
    pub opaque: Vec<u8>,
    /// 32-byte digest over [`opaque`](Self::opaque).
    pub digest: [u8; 32],
}

/// What [`InstantPlay::restart`] recovered: the tick, revision and digest now
/// live again. Equals the snapshot's fields on success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restored {
    /// Round tick recovered from the snapshot.
    pub tick: u64,
    /// Round document revision recovered from the snapshot.
    pub revision: u64,
    /// Digest of the restored snapshot bytes.
    pub digest: [u8; 32],
}

/// Exact replay evidence: the input-tape window plus the re-simulated end
/// state. Re-running [`InstantPlay::replay_last_30s`] yields byte-identical
/// `end_opaque` and `end_digest` (asserted by test, not by trust).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactReplay {
    /// Tick of the oldest kept input (window start).
    pub from_tick: u64,
    /// Inputs in the window, oldest first.
    pub inputs: Vec<RoundInput>,
    /// Tick after the last replayed input (equals the live tick).
    pub end_tick: u64,
    /// Digest of the re-simulated end state.
    pub end_digest: [u8; 32],
    /// Opaque bytes of the re-simulated end state.
    pub end_opaque: Vec<u8>,
}

impl ExactReplay {
    /// Number of replayed ticks.
    pub fn len(&self) -> usize {
        self.inputs.len()
    }

    /// True when the window holds no inputs.
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }
}

/// Seal a round state plus its document revision into opaque bytes.
fn encode_round(revision: u64, state: &RoundState) -> Vec<u8> {
    let mut out = Vec::with_capacity(SNAPSHOT_BYTES);
    out.extend_from_slice(&SNAPSHOT_MAGIC);
    out.push(SNAPSHOT_VERSION);
    out.extend_from_slice(&revision.to_le_bytes());
    out.extend_from_slice(&state.tick.to_le_bytes());
    out.extend_from_slice(&state.seed.to_le_bytes());
    for lane in [state.p0, state.v0, state.p1, state.v1] {
        out.extend_from_slice(&lane.to_le_bytes());
    }
    let checksum = crc32(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    debug_assert_eq!(out.len(), SNAPSHOT_BYTES);
    out
}

/// Open sealed bytes into the revision plus the round state. Rejects bad
/// magic, unsupported versions, wrong lengths, checksum failures and oversize
/// input; never panics on any input.
fn decode_round(bytes: &[u8]) -> Result<(u64, RoundState), SnapshotError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotError::TooLarge);
    }
    if bytes.len() != SNAPSHOT_BYTES {
        if bytes.len() >= 4 && bytes[0..4] != SNAPSHOT_MAGIC {
            return Err(SnapshotError::BadMagic);
        }
        return Err(SnapshotError::Truncated);
    }
    if bytes[0..4] != SNAPSHOT_MAGIC {
        return Err(SnapshotError::BadMagic);
    }
    if bytes[4] < MIN_SNAPSHOT_VERSION || bytes[4] > SNAPSHOT_VERSION {
        return Err(SnapshotError::BadVersion { got: bytes[4] });
    }
    let body = &bytes[..SNAPSHOT_BYTES - 4];
    let mut crc_bytes = [0u8; 4];
    crc_bytes.copy_from_slice(&bytes[SNAPSHOT_BYTES - 4..]);
    if u32::from_le_bytes(crc_bytes) != crc32(body) {
        return Err(SnapshotError::CrcMismatch);
    }
    let revision = u64::from_le_bytes(bytes[5..13].try_into().expect("sealed offsets are fixed"));
    let tick = u64::from_le_bytes(bytes[13..21].try_into().expect("sealed offsets are fixed"));
    let seed = u64::from_le_bytes(bytes[21..29].try_into().expect("sealed offsets are fixed"));
    let mut lanes = [0i32; 4];
    for (i, slot) in lanes.iter_mut().enumerate() {
        let at = 29 + 4 * i;
        *slot = i32::from_le_bytes(
            bytes[at..at + 4]
                .try_into()
                .expect("sealed offsets are fixed"),
        );
    }
    Ok((
        revision,
        RoundState {
            tick,
            seed,
            p0: lanes[0],
            v0: lanes[1],
            p1: lanes[2],
            v1: lanes[3],
        },
    ))
}

/// Deterministic camera seed for [`InstantPlay::play_from_camera`]: the
/// pinned byte fold over the little-endian camera bytes. None for a degenerate camera (bad
/// field of view, non-finite fields, coincident eye/target or parallel up),
/// which must never seed a round.
pub fn camera_seed(camera: &Camera) -> Option<u64> {
    if !camera.is_well_formed() || camera.view_matrix_rows().is_none() {
        return None;
    }
    let mut bytes = Vec::with_capacity(40);
    for lane in camera
        .position
        .iter()
        .chain(camera.target.iter())
        .chain(camera.up.iter())
    {
        bytes.extend_from_slice(&lane.to_bits().to_le_bytes());
    }
    bytes.extend_from_slice(&camera.fov_y_deg.to_bits().to_le_bytes());
    Some(fold_pinned(FNV64_OFFSET, &bytes))
}

/// Default round seed from the document: the pinned byte fold over its
/// canonical save bytes, so distinct documents seed distinct rounds
/// deterministically.
fn seed_from_doc(doc: &Document) -> u64 {
    fold_pinned(FNV64_OFFSET, &crate::save::save_to_bytes(doc))
}

/// Instant play session over a round copy of an authoring document.
///
/// Owns the round [`Document`] (the live authoring document is never shared
/// with the child), the deterministic round simulation and the bounded input
/// tape. Ticks never touch the hierarchy: the revision moves only through
/// [`apply_batch`](Self::apply_batch), which routes through the command
/// service like every other editor edit.
pub struct InstantPlay {
    doc: Document,
    entry: PlayEntrypoint,
    /// State at the oldest kept input; the replay start when the tape is full.
    base: RoundState,
    /// Live round state: `base` stepped through every kept input.
    current: RoundState,
    /// Bounded input window, oldest first; capped at [`REPLAY_WINDOW_TICKS`].
    inputs: VecDeque<RoundInput>,
}

impl InstantPlay {
    /// Open a session on a round copy of `doc` with `entry` as the game
    /// entrypoint. The round is seeded from the document bytes; the very
    /// first [`play_from_camera`](Self::play_from_camera) reseeds from the
    /// editor camera.
    pub fn new(doc: Document, entry: PlayEntrypoint) -> Self {
        let seed = seed_from_doc(&doc);
        let initial = RoundState::initial(seed);
        Self {
            doc,
            entry,
            base: initial,
            current: initial,
            inputs: VecDeque::new(),
        }
    }

    /// The round document (read-only: edits go through [`apply_batch`](Self::apply_batch)).
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Round document revision.
    pub fn revision(&self) -> u64 {
        self.doc.revision
    }

    /// Live round tick.
    pub fn tick(&self) -> u64 {
        self.current.tick
    }

    /// Live round seed (camera seed after [`play_from_camera`](Self::play_from_camera)).
    pub fn seed(&self) -> u64 {
        self.current.seed
    }

    /// Ticks kept in the replay window.
    pub fn tape_len(&self) -> usize {
        self.inputs.len()
    }

    /// True when no input is recorded yet.
    pub fn tape_is_empty(&self) -> bool {
        self.inputs.is_empty()
    }

    /// Tick of the oldest kept input (the replay start).
    pub fn window_start(&self) -> u64 {
        self.base.tick
    }

    /// Apply an authoring batch to the round document through the command
    /// service. Failures apply nothing, like every other editor edit.
    pub fn apply_batch(
        &mut self,
        journal: &mut Journal,
        batch: &Batch,
    ) -> Result<Applied, BatchError> {
        commands::apply(&mut self.doc, journal, batch)
    }

    /// Play from the editor camera: reseed a fresh round from the camera,
    /// clear the tape, and launch the child with camera seed args.
    ///
    /// The document is never shared with the child — the entrypoint carries
    /// only the program, its configured args plus `--camera-seed <hex>` — so
    /// a killed game leaves the round document intact. Err on a degenerate
    /// camera (nothing launches) or when the program cannot start.
    pub fn play_from_camera(&mut self, camera: &Camera) -> std::io::Result<ChildPlay> {
        let seed = camera_seed(camera)
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "editor camera cannot seed play"))?;
        let initial = RoundState::initial(seed);
        self.base = initial;
        self.current = initial;
        self.inputs.clear();
        let mut entry = self.entry.clone();
        entry.args.push("--camera-seed".to_string());
        entry.args.push(format!("{seed:016x}"));
        ChildPlay::spawn(&entry)
    }

    /// Record one input tick: step the live state and append to the bounded
    /// tape. Past the window cap the oldest input folds into the replay
    /// base, so the window always holds the newest inputs and the replay
    /// start stays exact.
    pub fn push_input(&mut self, input: RoundInput) {
        self.current.step(&input);
        self.inputs.push_back(input);
        if self.inputs.len() > REPLAY_WINDOW_TICKS {
            if let Some(oldest) = self.inputs.pop_front() {
                self.base.step(&oldest);
            }
        }
    }

    /// Snapshot the round: seal revision plus state into opaque bytes with
    /// their digest. The caller stores `opaque` verbatim and hands it back
    /// to [`restart`](Self::restart) unopened.
    pub fn snapshot_round(&self) -> RoundSnapshot {
        let opaque = encode_round(self.doc.revision, &self.current);
        let digest = digest_parts(&[&opaque]);
        RoundSnapshot {
            tick: self.current.tick,
            revision: self.doc.revision,
            opaque,
            digest,
        }
    }

    /// Restart the round from a snapshot: verify its digest and fields, then
    /// restore the revision and the state. The tape clears — the restarted
    /// round replays from here. Sub-second on the small fixture by a wide
    /// margin (fixed 49-byte decode, no IO); the timing test pins it.
    pub fn restart(&mut self, snapshot: &RoundSnapshot) -> Result<Restored, SnapshotError> {
        let (revision, state) = decode_round(&snapshot.opaque)?;
        if digest_parts(&[&snapshot.opaque]) != snapshot.digest
            || state.tick != snapshot.tick
            || revision != snapshot.revision
        {
            return Err(SnapshotError::DigestMismatch);
        }
        self.doc.revision = revision;
        self.base = state;
        self.current = state;
        self.inputs.clear();
        Ok(Restored {
            tick: state.tick,
            revision,
            digest: snapshot.digest,
        })
    }

    /// Replay the last 30 s: re-simulate the bounded input window from its
    /// base and return the evidence. Byte-identical across re-runs by
    /// construction (integer sim, fixed input order); the test re-runs and
    /// compares bytes rather than trusting this comment.
    pub fn replay_last_30s(&self) -> ExactReplay {
        let mut rerun = self.base;
        for input in &self.inputs {
            rerun.step(input);
        }
        let end_opaque = encode_round(self.doc.revision, &rerun);
        let end_digest = digest_parts(&[&end_opaque]);
        ExactReplay {
            from_tick: self.base.tick,
            inputs: self.inputs.iter().copied().collect(),
            end_tick: rerun.tick,
            end_digest,
            end_opaque,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Batch, Journal, Op};
    use crate::viewport::Viewport;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    /// A stub game: sleeps ~30 s, long enough to watch and kill. It must
    /// ignore the appended `--camera-seed` args like the real game accepts
    /// them: `rem` swallows them on Windows, `$1..` does on unix.
    #[cfg(windows)]
    fn stub_entrypoint() -> PlayEntrypoint {
        PlayEntrypoint::new("cmd", ["/C", "ping", "-n", "30", "127.0.0.1", "&", "rem"])
    }

    /// A stub game: sleeps ~30 s, long enough to watch and kill.
    #[cfg(not(windows))]
    fn stub_entrypoint() -> PlayEntrypoint {
        PlayEntrypoint::new("sh", ["-c", "exec sleep 30", "stub"])
    }

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    /// Round document with `count` chained nodes: revision equals `count`.
    fn chained_doc(count: u64, key_base: u64) -> (Document, Journal) {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut parent = ObjectId::NIL;
        for i in 0..count {
            let id = ObjectId::new();
            let rev = doc.revision;
            commands::apply(
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

    /// Deterministic input stream: full 30 s window plus overflow, so the
    /// tape cap and the replay base both get exercised.
    fn drive(play: &mut InstantPlay, ticks: usize) {
        for i in 0..ticks {
            let lane = (i % 5) as i8 - 2;
            play.push_input(RoundInput::new(lane, 2 - lane));
        }
    }

    #[test]
    fn snapshot_restart_restores_revision_and_digest() {
        let (doc, _) = chained_doc(3, 1000);
        let mut play = InstantPlay::new(doc, stub_entrypoint());
        let mut journal = Journal::new();
        drive(&mut play, 41);
        let snapshot = play.snapshot_round();
        assert_eq!(snapshot.tick, 41);
        assert_eq!(snapshot.revision, 3);
        assert_eq!(snapshot.opaque.len(), SNAPSHOT_BYTES);

        // Move on: a new authoring batch plus more ticks.
        let id = ObjectId::new();
        play.apply_batch(
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: pose_at(9.0),
                }],
                base_revision: play.revision(),
                idempotency_key: 4242,
                provenance: None,
            },
        )
        .unwrap();
        drive(&mut play, 7);
        assert_eq!(play.revision(), 4);
        assert_ne!(play.snapshot_round(), snapshot);

        // Restart recovers the snapshot exactly.
        let restored = play.restart(&snapshot).unwrap();
        assert_eq!(restored.tick, 41);
        assert_eq!(restored.revision, 3);
        assert_eq!(restored.digest, snapshot.digest);
        assert_eq!(play.revision(), 3);
        assert_eq!(play.tick(), 41);
        assert_eq!(play.snapshot_round(), snapshot);
    }

    #[test]
    fn thirty_second_replay_is_byte_identical_on_rerun() {
        let (doc, _) = chained_doc(2, 2000);
        let mut play = InstantPlay::new(doc, stub_entrypoint());
        // Full 30 s window plus overflow: the tape holds the newest 1800.
        drive(&mut play, REPLAY_WINDOW_TICKS + 100);
        assert_eq!(play.tape_len(), REPLAY_WINDOW_TICKS);
        assert_eq!(play.window_start(), 100);
        assert_eq!(play.tick(), (REPLAY_WINDOW_TICKS + 100) as u64);

        let first = play.replay_last_30s();
        let second = play.replay_last_30s();
        assert_eq!(first.inputs.len(), REPLAY_WINDOW_TICKS);
        assert_eq!(first, second, "replay must be byte-identical on re-run");
        assert_eq!(first.end_tick, play.tick());
        // The replayed end state matches the live round byte for byte.
        let live = play.snapshot_round();
        assert_eq!(first.end_opaque, live.opaque);
        assert_eq!(first.end_digest, live.digest);
        assert!(!first.is_empty());
        assert_eq!(first.len(), REPLAY_WINDOW_TICKS);
    }

    #[test]
    fn empty_replay_matches_a_fresh_snapshot() {
        let (doc, _) = chained_doc(1, 3000);
        let play = InstantPlay::new(doc, stub_entrypoint());
        assert!(play.tape_is_empty());
        let replay = play.replay_last_30s();
        assert!(replay.is_empty());
        assert_eq!(replay.len(), 0);
        let live = play.snapshot_round();
        assert_eq!(replay.end_opaque, live.opaque);
        assert_eq!(replay.end_digest, live.digest);
    }

    #[test]
    fn killed_play_preserves_the_document() {
        // Authoring document with applied batches; the session owns a copy.
        let (doc, _) = chained_doc(2, 4000);
        let kept = doc.clone();
        let mut play = InstantPlay::new(doc, stub_entrypoint());
        let camera = Viewport::default_panel().camera;

        // Play from the editor camera: the child carries camera seed args and
        // the document is never shared with it.
        let mut child = play.play_from_camera(&camera).expect("stub process starts");
        assert!(child.command_line().contains("--camera-seed"));
        assert!(child.is_alive());
        child.kill().expect("kill stops the stub");
        assert!(!child.is_alive());

        // Child isolation asserted here: the killed game left the
        // round document (hierarchy plus revision) exactly intact.
        assert_eq!(play.document(), &kept);
        assert_eq!(play.revision(), kept.revision);
        assert_eq!(play.tick(), 0, "fresh camera play starts at tick zero");
    }

    #[test]
    fn play_seeds_differ_per_camera_and_reject_degenerate_cameras() {
        let (doc, _) = chained_doc(1, 5000);
        let mut play = InstantPlay::new(doc, stub_entrypoint());
        let near = Viewport::default_panel().camera;
        let mut far = near;
        far.position = [3.0, 1.5, 5.0];
        let seed_near = camera_seed(&near).expect("default camera seeds play");
        let seed_far = camera_seed(&far).expect("moved camera seeds play");
        assert_ne!(seed_near, seed_far);

        // Degenerate cameras seed nothing and launch nothing.
        let mut bad = near;
        bad.position = bad.target;
        assert_eq!(camera_seed(&bad), None);
        let err = match play.play_from_camera(&bad) {
            Ok(_) => panic!("degenerate camera must not launch"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let mut tilted = near;
        tilted.up = [0.0, 0.0, 0.0];
        assert_eq!(camera_seed(&tilted), None);
    }

    #[test]
    fn corrupt_snapshots_fail_bounded_and_change_nothing() {
        let (doc, _) = chained_doc(2, 6000);
        let mut play = InstantPlay::new(doc, stub_entrypoint());
        drive(&mut play, 9);
        let good = play.snapshot_round();
        let before = play.snapshot_round();

        // Each corruption class fails with its own error.
        let mut bad_magic = good.opaque.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            play.restart(&RoundSnapshot {
                opaque: bad_magic,
                ..good.clone()
            }),
            Err(SnapshotError::BadMagic)
        );

        assert_eq!(
            play.restart(&RoundSnapshot {
                opaque: good.opaque[..10].to_vec(),
                ..good.clone()
            }),
            Err(SnapshotError::Truncated)
        );

        let mut bad_version = good.opaque.clone();
        bad_version[4] = SNAPSHOT_VERSION + 1;
        let checksum = crc32(&bad_version[..SNAPSHOT_BYTES - 4]);
        bad_version[SNAPSHOT_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(
            play.restart(&RoundSnapshot {
                opaque: bad_version,
                ..good.clone()
            }),
            Err(SnapshotError::BadVersion {
                got: SNAPSHOT_VERSION + 1
            })
        );

        let mut tampered = good.opaque.clone();
        let last_payload = tampered.len() - 5;
        tampered[last_payload] ^= 0xFF;
        assert_eq!(
            play.restart(&RoundSnapshot {
                opaque: tampered,
                ..good.clone()
            }),
            Err(SnapshotError::CrcMismatch)
        );

        assert_eq!(
            play.restart(&RoundSnapshot {
                opaque: vec![0u8; MAX_SNAPSHOT_BYTES + 1],
                ..good.clone()
            }),
            Err(SnapshotError::TooLarge)
        );

        // Valid bytes under a foreign digest or foreign fields are refused.
        let mut foreign = good.clone();
        foreign.digest = [0xABu8; 32];
        assert_eq!(play.restart(&foreign), Err(SnapshotError::DigestMismatch));
        let mut relabeled = good.clone();
        relabeled.tick += 1;
        assert_eq!(play.restart(&relabeled), Err(SnapshotError::DigestMismatch));

        // Every failure left the live round untouched.
        assert_eq!(play.snapshot_round(), before);
        let report = SnapshotError::BadVersion { got: 9 }.to_string();
        assert!(report.contains('9'));
    }

    #[test]
    #[ignore = "timing: restart budget needs a quiet machine"]
    fn restart_recovers_in_under_a_second() {
        let (doc, _) = chained_doc(3, 7000);
        let mut play = InstantPlay::new(doc, stub_entrypoint());
        drive(&mut play, 120);
        let snapshot = play.snapshot_round();
        drive(&mut play, 60);

        let start = std::time::Instant::now();
        let restored = play.restart(&snapshot).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(restored.tick, 120);
        assert_eq!(play.snapshot_round(), snapshot);
        assert!(
            elapsed.as_millis() < 1000,
            "restart took {elapsed:?}, budget is under a second"
        );
    }
}
