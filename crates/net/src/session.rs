//! session: LAN session over UDP (server-authoritative arena match).
//!
//! Reference design: CrystalOrb client-server rollback (pattern only): the
//! server runs the authoritative match, clients predict their inputs and
//! resimulate when the server corrects them. Transport is [`std::net`] UDP
//! only; this crate stays dependency-light (arena plus input path deps).
//!
//! Authority per tick: [`Server`] owns the match. It holds one cold
//! [`LiveMatch`](engine_arena::LiveMatch) and steps it exactly once per
//! authored tick with the human inputs it has received (one sim step per
//! tick, byte-identical to the arena prefix runners); every slot without a
//! human input is filled by the arena's own brain fill, so AI fills empty
//! slots server-side with no extra code here. The server never rolls back:
//! an input that arrives after its tick was authored loses and the client
//! resimulates to the server truth. [`Server::steps`] is the cost proof.
//!
//! Prediction per tick: [`Client`] predicts its local input immediately for
//! responsiveness through its own held live match (one sim step per
//! predicted tick; [`Client::steps`] is the cost proof) and assumes the
//! server shares every prediction so far. [`Client::on_server_state`] names
//! the ticks where that assumption was wrong (mispredict ticks);
//! [`Client::resimulate`] then replays the server-true prefix plus the
//! still-unacked local predictions.
//!
//! [`SnapshotDelta`] is a full snapshot for now: version tag, tick, hash and
//! the complete authoritative input prefix. Quantized fighter-state deltas
//! against an acked baseline plus remote interpolation live in
//! [`crate::delta`]; the state wire packet below carries them with the delta
//! bit set, bit clear still means the full input-tape encoding byte-for-byte.
//!
//! Input delay plus redundancy: the sender repeats the last [`REDUNDANCY`]
//! ticks of inputs in every packet, so one dropped datagram loses nothing;
//! the send queue additionally holds each input for [`INPUT_DELAY_TICKS`]
//! ticks (wired through the transport harness, 2 ticks by default) so the
//! server usually has the input before it authors the tick. Late inputs
//! still lose; that is the documented cost of server authority.
//!
//! Input compression (experiment): [`encode_input_compressed`] XOR-chains the
//! sorted inputs from a zero base and RLE-packs the zero runs. Same header
//! and kind byte as [`encode_input`], with [`INPUT_COMPRESSED_BIT`] set on
//! the count byte; [`decode_input`] reads both shapes, so this is one codec,
//! not two. Pattern from backroll-rs `compression` plus `bitfield` (ISC),
//! reimplemented here integer-only with no new dependency.
//!
//! Wire format: every datagram starts with magic plus a version byte.
//! Versions other than [`WIRE_VERSION`] — including future versions — are
//! rejected as [`NetError::VersionMismatch`] and change nothing. Duplicate
//! or out-of-order packets are acknowledged but change nothing.
//!
//! Physics track: the session uses the arena default kinematic track. The
//! arena `sim` feature (Rapier settling) stays off: rollback needs the
//! same-machine reproducibility the default track already gives, without the
//! physics snapshot surface.
//!
//! Units: fixed ticks at the arena tick rate; latency mapping is 3 ticks ==
//! 50 ms at 60 Hz. Determinism scope: same-machine reproducibility — the
//! arena prefix runs are bit-identical given the same config plus tape.
//! Error type: [`NetError`]; invalid wire or simulation input rejects loudly
//! and changes nothing, never panics.
//!
//! Consumer: the packaged game and LAN playtests. Data owner: the
//! server owns authoritative inputs; the client owns predictions.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use engine_arena::{ArenaConfig, Checkpoint, LiveMatch, MatchSpec};
use engine_input::{FighterId, FighterInput};
/// Neutral names for the input contract types: engine code outside the GE2
/// genre ledger imports them from here instead of naming the genre types.
pub use engine_input::{FighterId as BodyId, FighterInput as BodyInput, FighterTape as BodyTape};

pub use crate::delta::{
    decode_delta, encode_delta, Delta, FighterDelta, FighterSnap, FullSnap, InterpResult,
    Interpolator, DEFAULT_INTERP_DELAY_MS, MASK_ALL, MAX_INTERP_SAMPLES,
};
use crate::delta::{
    delta_wire_len, validate_delta_shape, MASK_HP, MASK_STAMINA, MASK_X, MASK_Y, MASK_YAW, MASK_Z,
};
/// Remote presentation of the LAN path: the frozen [`Interpolator`] plus the
/// Hermite/quaternion/guard modules, fed together from one snapshot stream.
pub use crate::interp::{FrameHead, InterpFrame, QueryKind, RemoteView, SnapshotInterpolator};

/// Wire version accepted by [`decode_input`] and [`decode_state`].
/// Anything else — older or future — is rejected outright.
pub const WIRE_VERSION: u8 = 1;
/// Wire magic: `b"N1"` little-endian, first two bytes of every datagram.
pub const WIRE_MAGIC: u16 = 0x314E;
/// Input packet kind byte.
pub const KIND_INPUT: u8 = 0;
/// State packet kind byte.
pub const KIND_STATE: u8 = 1;
/// Full-snapshot packet kind byte (quantized fighter states, the delta
/// resync primitive). Full-only peers answer `BadKind` loudly.
pub const KIND_SNAPSHOT: u8 = 2;
/// Delta bit on the state count byte: set means the payload is a
/// fighter-state [`Delta`] plus its baseline tick; clear means the full
/// input-tape encoding. The version byte stays [`WIRE_VERSION`] either way.
pub const STATE_DELTA_BIT: u8 = 0x80;
/// Full-snapshot header length: magic 2 + version 1 + kind 1 + tick 4 + count 1.
pub const SNAPSHOT_HEADER_LEN: usize = 9;
/// Full-snapshot per-slot length: slot 1 + xyz 12 + yaw 2 + hp 2 + stamina 2.
pub const SNAPSHOT_SLOT_LEN: usize = 19;
/// State-delta header length: the 17-byte state header plus baseline tick 4.
pub const STATE_DELTA_HEADER_LEN: usize = 21;
/// Last-N inputs repeated in every packet (loss redundancy).
pub const REDUNDANCY: usize = 3;
/// Ticks a sent input is held before the server is expected to need it.
pub const INPUT_DELAY_TICKS: u32 = 2;
/// Upper bound on inputs carried by one datagram (flood guard).
pub const MAX_INPUTS_PER_PACKET: usize = 16;
/// Upper bound on an accepted datagram in bytes (flood guard).
pub const MAX_PACKET_BYTES: usize = 1200;
/// Encoded [`FighterInput`] size: tick u32 + fighter u8 + mx i8 + mz i8 +
/// facing i16 + buttons u8.
pub const INPUT_BYTES: usize = 10;
/// High bit on the input count byte: set means the payload is XOR-chained
/// deltas with zero-run RLE (see [`encode_input_compressed`]); clear means
/// the raw full encoding. Same header, same kind byte: one codec, not two.
pub const INPUT_COMPRESSED_BIT: u8 = 0x80;
/// Longest run one RLE tag names (the tag stores length minus one in 7 bits).
const RLE_MAX_RUN: usize = 128;
/// Largest decoded input payload in bytes (the flood-guard shape).
const MAX_DECODED_INPUT_BYTES: usize = MAX_INPUTS_PER_PACKET * INPUT_BYTES;

/// ioq3 netchan header beside the KIND path (pattern only, no version bump).
pub const IOQ3_FRAGMENT_BIT: u32 = 1 << 31;
/// ioq3 header bytes: sequence 4 + qport 2 + start 2 + length 2.
pub const IOQ3_HEADER_LEN: usize = 10;
/// ioq3 fragment payload cap (1400 minus 100, pattern only).
pub const IOQ3_FRAGMENT_SIZE: usize = 1300;
/// ioq3 OOB sequence (-1 as u32), never a data sequence.
pub const IOQ3_OOB_SEQUENCE: u32 = u32::MAX;
/// ioq3 fragment header: one sequence shared by every fragment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ioq3Header {
    /// Sequence with FRAGMENT_BIT set when fragmented.
    pub sequence: u32,
    /// Port slot for remapped ports.
    pub qport: u16,
    /// Byte offset of this fragment.
    pub start: u16,
    /// Payload bytes in this fragment.
    pub length: u16,
}
/// Encode one ioq3 header as 10 little-endian bytes.
pub fn ioq3_encode_header(h: &Ioq3Header) -> [u8; IOQ3_HEADER_LEN] {
    let mut out = [0u8; IOQ3_HEADER_LEN];
    out[0..4].copy_from_slice(&h.sequence.to_le_bytes());
    out[4..6].copy_from_slice(&h.qport.to_le_bytes());
    out[6..8].copy_from_slice(&h.start.to_le_bytes());
    out[8..10].copy_from_slice(&h.length.to_le_bytes());
    out
}
/// Decode one ioq3 header; loud refuse on short, OOB, or length mismatch.
pub fn ioq3_decode_header(bytes: &[u8]) -> Result<Ioq3Header, NetError> {
    if bytes.len() < IOQ3_HEADER_LEN {
        return Err(NetError::Truncated);
    }
    let sequence = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let qport = u16::from_le_bytes([bytes[4], bytes[5]]);
    let start = u16::from_le_bytes([bytes[6], bytes[7]]);
    let length = u16::from_le_bytes([bytes[8], bytes[9]]);
    if sequence == IOQ3_OOB_SEQUENCE {
        return Err(NetError::BadState);
    }
    let start_us = usize::from(start);
    let len_us = usize::from(length);
    if len_us > bytes.len() - IOQ3_HEADER_LEN || start_us.checked_add(len_us).is_none() {
        return Err(NetError::Truncated);
    }
    Ok(Ioq3Header {
        sequence,
        qport,
        start,
        length,
    })
}
/// Split payload into same-sequence fragments (single keeps raw sequence).
pub fn ioq3_fragment_message(sequence: u32, qport: u16, payload: &[u8]) -> Vec<Vec<u8>> {
    if payload.is_empty() {
        return Vec::new();
    }
    let count = payload.chunks(IOQ3_FRAGMENT_SIZE).len();
    let sequence = if count > 1 {
        sequence | IOQ3_FRAGMENT_BIT
    } else {
        sequence & !IOQ3_FRAGMENT_BIT
    };
    let mut out = Vec::with_capacity(count);
    let mut offset = 0usize;
    for chunk in payload.chunks(IOQ3_FRAGMENT_SIZE) {
        let start = u16::try_from(offset).unwrap_or(u16::MAX);
        let length = u16::try_from(chunk.len()).unwrap_or(u16::MAX);
        let header = Ioq3Header {
            sequence,
            qport,
            start,
            length,
        };
        let mut datagram = Vec::with_capacity(IOQ3_HEADER_LEN + chunk.len());
        datagram.extend_from_slice(&ioq3_encode_header(&header));
        datagram.extend_from_slice(chunk);
        out.push(datagram);
        offset += chunk.len();
    }
    out
}
/// Reassemble same-sequence fragments byte-exact; dupes ignored, gaps loud.
pub fn ioq3_reassemble(fragments: &[Vec<u8>]) -> Result<Vec<u8>, NetError> {
    if fragments.is_empty() {
        return Err(NetError::Truncated);
    }
    let first = ioq3_decode_header(&fragments[0])?;
    let mut parts: Vec<(usize, Vec<u8>)> = Vec::with_capacity(fragments.len());
    for datagram in fragments {
        let header = ioq3_decode_header(datagram)?;
        if header.sequence != first.sequence || header.qport != first.qport {
            return Err(NetError::BadState);
        }
        let body = datagram[IOQ3_HEADER_LEN..].to_vec();
        if body.len() != usize::from(header.length) {
            return Err(NetError::Truncated);
        }
        parts.push((usize::from(header.start), body));
    }
    parts.sort_by_key(|(start, _)| *start);
    let mut out = Vec::new();
    let mut next = 0usize;
    for (start, body) in &parts {
        if *start == next {
            out.extend_from_slice(body);
            next += body.len();
        } else if *start < next {
            let end = start.saturating_add(body.len());
            if end > next || out[*start..end] != body[..] {
                return Err(NetError::BadState);
            }
        } else {
            return Err(NetError::Truncated);
        }
    }
    Ok(out)
}

/// Session failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetError {
    /// Arena config invalid (`ticks`, `fighters` or extent out of range).
    BadConfig,
    /// A fighter input failed range/order/slot validation.
    BadInput,
    /// State packet for an already-applied tick is not an error; this
    /// variant is returned only by strict single-shot helpers in tests.
    /// Core `on_server_state` answers stale packets with `Ok(vec![])`.
    StaleTick(u32),
    /// Input stamped for an already-authored server tick; it loses.
    LateInput(u32),
    /// Wire version byte is not [`WIRE_VERSION`]; culprit carried.
    /// Future versions are rejected, never accepted.
    VersionMismatch {
        /// Version byte found on the wire.
        got: u8,
    },
    /// Magic bytes do not match [`WIRE_MAGIC`].
    BadMagic,
    /// Kind byte is neither [`KIND_INPUT`] nor [`KIND_STATE`].
    BadKind(u8),
    /// Datagram ends mid-field.
    Truncated,
    /// Datagram exceeds [`MAX_PACKET_BYTES`].
    TooLarge,
    /// Input count outside the per-kind bound, count carried.
    TooManyInputs(usize),
    /// Arena rejected a prefix run the net layer believed valid.
    /// Unreachable by construction; loud instead of silent.
    SimRejected,
    /// Checkpoint and delta disagree (tick or hash mismatch).
    BadState,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadConfig => write!(f, "invalid arena config"),
            Self::BadInput => write!(f, "invalid fighter input"),
            Self::StaleTick(t) => write!(f, "stale tick {t}"),
            Self::LateInput(t) => write!(f, "late input for tick {t}"),
            Self::VersionMismatch { got } => {
                write!(f, "wire version mismatch: got {got}, want {WIRE_VERSION}")
            }
            Self::BadMagic => write!(f, "bad wire magic"),
            Self::BadKind(k) => write!(f, "unknown packet kind {k}"),
            Self::Truncated => write!(f, "truncated packet"),
            Self::TooLarge => write!(f, "packet exceeds size bound"),
            Self::TooManyInputs(n) => write!(f, "too many inputs: {n}"),
            Self::SimRejected => write!(f, "arena rejected the prefix run"),
            Self::BadState => write!(f, "checkpoint and delta disagree"),
        }
    }
}

impl std::error::Error for NetError {}

/// Full-snapshot-for-now server state: version tag, authoritative
/// checkpoint and the complete authoritative input prefix. The version tag
/// is the migration seam for replacing the `tape` payload with byte deltas
/// against an acked baseline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SnapshotDelta {
    /// Must equal [`WIRE_VERSION`]; anything else is rejected.
    pub version: u8,
    /// Authoritative tick this snapshot covers.
    pub tick: u32,
    /// Authoritative state hash at `tick` (equals the checkpoint hash).
    pub hash: u64,
    /// Authoritative human inputs for ticks `0..=tick`, `(tick, fighter)`
    /// sorted and unique. Slots without an entry were AI-filled by the
    /// arena's brain fill.
    pub tape: Vec<FighterInput>,
}

/// Server-authoritative arena match: owns the config, the authoritative input
/// tape, the checkpoint history and one incremental live stepper. Data owner
/// for authoritative inputs.
pub struct Server {
    spec: MatchSpec,
    next_tick: u32,
    tape: Vec<FighterInput>,
    checkpoints: Vec<Checkpoint>,
    /// One cold authority sim stepped exactly once per authored tick, the
    /// same shape as the arena's EG1 live match. The tape stays the data
    /// owner above; the stepper owns the running sim state.
    stepper: LiveMatch,
}

impl Server {
    /// Validate `config` and open the default match at tick zero: rules v1
    /// on open sand, [`MatchSpec::v1`].
    pub fn new(config: ArenaConfig) -> Result<Self, NetError> {
        Self::from_spec(MatchSpec::v1(config))
    }

    /// Validate `spec` and open its match at tick zero: the same match the
    /// window plays when it builds from the same spec. A spec whose input
    /// delay is outside [`crate::delay::MIN_DELAY`]`..=`[`crate::delay::MAX_DELAY`]
    /// is refused loudly (the knob the client drives).
    pub fn from_spec(spec: MatchSpec) -> Result<Self, NetError> {
        if spec.config.validate().is_err() || spec.input_delay_ticks > crate::delay::MAX_DELAY {
            return Err(NetError::BadConfig);
        }
        let stepper = spec.live().ok_or(NetError::BadConfig)?;
        Ok(Self {
            spec,
            next_tick: 0,
            tape: Vec::new(),
            checkpoints: Vec::new(),
            stepper,
        })
    }

    /// Next tick to author.
    pub fn tick_now(&self) -> u32 {
        self.next_tick
    }

    /// Incremental sim steps performed since construction: exactly one per
    /// authored tick. A from-zero replay per tick would blow this up; the
    /// finding-4 test pins 600 ticks to 600 steps.
    pub fn steps(&self) -> u64 {
        self.stepper.steps()
    }

    /// Authoritative checkpoints so far, one per authored tick.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Match config.
    pub fn config(&self) -> ArenaConfig {
        self.spec.config
    }

    /// The match this server runs: config, rules and world.
    pub fn spec(&self) -> &MatchSpec {
        &self.spec
    }

    /// Author one fixed tick with this tick's human inputs, returning the
    /// authoritative checkpoint plus the full [`SnapshotDelta`].
    ///
    /// Every input must be stamped for [`Self::tick_now`], name a fighter
    /// slot below the config count, and pass [`FighterInput::validate`];
    /// duplicates within the call fail. Slots without an input are AI-filled
    /// by the arena. Any failure rejects the whole tick and changes nothing.
    /// The held live match steps exactly once per call: the checkpoint equals
    /// the last row of the arena prefix run over the whole tape (EG1 parity).
    pub fn tick(
        &mut self,
        inputs: &[FighterInput],
    ) -> Result<(Checkpoint, SnapshotDelta), NetError> {
        let fighters = self.spec.config.fighters;
        let mut sorted: Vec<FighterInput> = inputs.to_vec();
        sorted.sort_by_key(|input| (input.tick, input.fighter.0));
        let mut prev: Option<(u32, u8)> = None;
        for input in &sorted {
            if input.validate().is_err() || input.fighter.0 >= fighters {
                return Err(NetError::BadInput);
            }
            if input.tick != self.next_tick {
                return Err(NetError::LateInput(input.tick));
            }
            let key = (input.tick, input.fighter.0);
            if prev == Some(key) {
                return Err(NetError::BadInput);
            }
            prev = Some(key);
        }
        let checkpoint = match self.stepper.step(&sorted) {
            Some(stepped) => stepped.checkpoint,
            None => return Err(NetError::SimRejected),
        };
        debug_assert_eq!(checkpoint.tick, self.next_tick);
        self.tape.extend_from_slice(&sorted);
        self.checkpoints.push(checkpoint);
        self.next_tick += 1;
        Ok((
            checkpoint,
            SnapshotDelta {
                version: WIRE_VERSION,
                tick: checkpoint.tick,
                hash: checkpoint.hash,
                tape: self.tape.clone(),
            },
        ))
    }
}

/// Predicting arena client: owns local predictions plus the server truth
/// learned so far. Never imports game or editor types.
pub struct Client {
    spec: MatchSpec,
    local: FighterId,
    predicted: Vec<FighterInput>,
    server_tape: Vec<FighterInput>,
    server_tick: Option<u32>,
    server_hash: Option<u64>,
    checkpoints: Vec<Checkpoint>,
    total_mispredicts: u64,
    last_mispredicts: Vec<u32>,
    cells: crate::cells::CellMirror,
    /// Prediction sim: the predicted tape, one step per predicted tick (the
    /// same tape the old prefix run rebuilt per call, now held and stepped).
    stepper: LiveMatch,
    /// Authority sim: the merged server tape, one step per received tick.
    /// Never rewinds and never replays: the merged tape only grows.
    truth: LiveMatch,
    /// Authoritative checkpoints, one per truth step (index == tick): the
    /// prefix the old per-state `run_prefix` rebuilt, held instead.
    truth_cps: Vec<Checkpoint>,
    /// Leading `checkpoints` entries known equal to `truth_cps`: the
    /// mispredict compare walks only the ticks truth newly covered, so a
    /// state costs O(1) amortized instead of a from-zero replay.
    agreed: usize,
    /// Sim steps the held steppers performed (the finding-4 cost proof).
    steps: u64,
}

impl Client {
    /// Validate `config` and bind the local fighter slot, on the default
    /// match: rules v1 on open sand, [`MatchSpec::v1`].
    pub fn new(config: ArenaConfig, local: FighterId) -> Result<Self, NetError> {
        Self::from_spec(MatchSpec::v1(config), local)
    }

    /// Validate `spec` and bind the local fighter slot. Prediction and
    /// authority sims both build from `spec`, so a client on the server's
    /// spec predicts the match the server authors. A spec whose input delay
    /// is outside [`crate::delay::MIN_DELAY`]`..=`[`crate::delay::MAX_DELAY`]
    /// is refused loudly.
    pub fn from_spec(spec: MatchSpec, local: FighterId) -> Result<Self, NetError> {
        if spec.config.validate().is_err()
            || spec.input_delay_ticks > crate::delay::MAX_DELAY
            || local.0 >= spec.config.fighters
        {
            return Err(NetError::BadConfig);
        }
        let stepper = spec.live().ok_or(NetError::BadConfig)?;
        let truth = spec.live().ok_or(NetError::BadConfig)?;
        Ok(Self {
            spec,
            local,
            predicted: Vec::new(),
            server_tape: Vec::new(),
            server_tick: None,
            server_hash: None,
            checkpoints: Vec::new(),
            total_mispredicts: 0,
            last_mispredicts: Vec::new(),
            cells: crate::cells::CellMirror::new(),
            stepper,
            truth,
            truth_cps: Vec::new(),
            agreed: 0,
            steps: 0,
        })
    }

    /// The match this client predicts: config, rules and world.
    pub fn spec(&self) -> &MatchSpec {
        &self.spec
    }

    /// Local fighter slot.
    pub fn local(&self) -> FighterId {
        self.local
    }

    /// Next local tick to predict (equals predicted checkpoint count).
    pub fn tick_now(&self) -> u32 {
        self.checkpoints.len() as u32
    }

    /// Predicted checkpoints, one per predicted tick.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Last applied server tick, if any state arrived.
    pub fn server_tick(&self) -> Option<u32> {
        self.server_tick
    }

    /// Mispredict ticks named by the most recent server state.
    pub fn last_mispredicts(&self) -> &[u32] {
        &self.last_mispredicts
    }

    /// Total mispredict ticks named over the session.
    pub fn total_mispredicts(&self) -> u64 {
        self.total_mispredicts
    }

    /// Decode one cell packet on the client path and hold its cell.
    /// Returns the cell hash; bad wire or bad ids refuse loudly.
    pub fn ingest_cell_bytes(&mut self, bytes: &[u8]) -> Result<u64, NetError> {
        let p = crate::cells::decode_packet(bytes).map_err(|_| NetError::BadInput)?;
        let receipt = self.cells.apply(&p).map_err(|_| NetError::BadInput)?;
        Ok(receipt.hash)
    }

    /// Held cell count for the streamed fixture.
    pub fn cell_count(&self) -> u32 {
        self.cells.count()
    }

    /// Canonical digest over held cells in ascending id order.
    pub fn cell_digest(&self) -> u64 {
        self.cells.digest()
    }

    /// Predict one fixed tick with the local input, returning the predicted
    /// checkpoint. The input must be stamped for [`Self::tick_now`], name
    /// the local slot and validate; remote slots stay empty so the arena
    /// AI-fills them exactly like the server does when it lacks their input.
    /// The held live match steps once per covered tick (a server-ahead
    /// catch-up leaves unpredicted ticks, which step AI-filled, exactly like
    /// the prefix run over the predicted tape filled them).
    pub fn predict(&mut self, input: FighterInput) -> Result<Checkpoint, NetError> {
        if input.validate().is_err() || input.fighter != self.local {
            return Err(NetError::BadInput);
        }
        if input.tick != self.tick_now() {
            return Err(NetError::BadInput);
        }
        let mut checkpoint = None;
        while self.stepper.tick() <= input.tick {
            let slice: &[FighterInput] = if self.stepper.tick() == input.tick {
                std::slice::from_ref(&input)
            } else {
                &[]
            };
            let Some(stepped) = self.stepper.step(slice) else {
                return Err(NetError::SimRejected);
            };
            checkpoint = Some(stepped.checkpoint);
            self.steps += 1;
        }
        let Some(checkpoint) = checkpoint else {
            return Err(NetError::SimRejected);
        };
        self.predicted.push(input);
        self.checkpoints.push(checkpoint);
        Ok(checkpoint)
    }

    /// Sim steps the held steppers performed since construction: one per
    /// predicted tick (plus AI-filled catch-up ticks), one per authoritative
    /// tick a state newly covers, and one per re-based suffix tick. A
    /// from-zero replay anywhere would blow this up; the finding-4 test pins
    /// 600 predicted ticks to 600 steps and the lockstep state pass to 2x.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Apply one authoritative server state, naming the ticks where the
    /// local prediction diverged from server truth.
    ///
    /// Version, tick/hash agreement and input validity are checked first;
    /// any failure changes nothing. A duplicate or out-of-order state
    /// (tick at or below the last applied server tick) answers `Ok(vec![])`
    /// and changes nothing. Otherwise the delta tape merges into the server
    /// truth and the authority sim steps forward one tick per newly received
    /// tick — never a from-zero replay. The mispredict list stays byte-equal
    /// to the old full-prefix compare: every overlapping tick is compared,
    /// but entries already known equal are skipped, so a state costs O(1)
    /// amortized and a later mismatch still names every divergent tick.
    /// Call [`Self::resimulate`] afterwards to converge.
    pub fn on_server_state(
        &mut self,
        checkpoint: Checkpoint,
        delta: &SnapshotDelta,
    ) -> Result<Vec<u32>, NetError> {
        if delta.version != WIRE_VERSION {
            return Err(NetError::VersionMismatch { got: delta.version });
        }
        if delta.tick != checkpoint.tick || delta.hash != checkpoint.hash {
            return Err(NetError::BadState);
        }
        validate_tape(&delta.tape, self.spec.config.fighters, Some(delta.tick))?;
        if self.server_tick.is_some_and(|done| checkpoint.tick <= done) {
            return Ok(Vec::new());
        }
        let merged = merge_tapes(&self.server_tape, &delta.tape);
        // The merged tape only grows and a known tick never changes (merge
        // keeps the first entry per key), so the authority sim can step
        // forward one tick per newly received tick. A delta that adds entries
        // for already-applied ticks rewrites the authority prefix (a heal
        // snapshot can legitimately add one the windows missed); the held sim
        // cannot un-step, so rebuild it from zero over the merged tape and
        // compare the whole overlap again, exactly like the old prefix run.
        let old_known = self
            .server_tape
            .partition_point(|input| Some(input.tick) <= self.server_tick);
        let new_known = merged.partition_point(|input| Some(input.tick) <= self.server_tick);
        if new_known > old_known {
            self.truth = self.spec.live().ok_or(NetError::SimRejected)?;
            self.truth_cps.clear();
            self.agreed = 0;
        }
        while self.truth.tick() <= checkpoint.tick {
            let tick = self.truth.tick();
            let slice = tick_slice(&merged, tick);
            let Some(stepped) = self.truth.step(slice) else {
                return Err(NetError::SimRejected);
            };
            self.truth_cps.push(stepped.checkpoint);
            self.steps += 1;
        }
        let compare_to = self.checkpoints.len().min((checkpoint.tick + 1) as usize);
        let mut mispredicts = Vec::new();
        for index in self.agreed.min(compare_to)..compare_to {
            if self.checkpoints[index] != self.truth_cps[index] {
                mispredicts.push(index as u32);
            }
        }
        self.agreed = match mispredicts.first() {
            Some(&first) => first as usize,
            None => compare_to,
        };
        self.server_tape = merged;
        self.server_tick = Some(checkpoint.tick);
        self.server_hash = Some(checkpoint.hash);
        self.total_mispredicts += mispredicts.len() as u64;
        self.last_mispredicts.clone_from(&mispredicts);
        Ok(mispredicts)
    }

    /// Resimulate from the server truth: ticks at or below the last server
    /// tick are the authoritative checkpoints the held truth sim already
    /// produced, later ticks re-step the still-unacked local predictions
    /// from a snapshot at the truth tip (one step per unacked tick, never a
    /// from-zero replay). Returns the converged checkpoint history.
    pub fn resimulate(&mut self) -> Vec<Checkpoint> {
        let Some(done) = self.server_tick.map(|tick| tick + 1) else {
            // No authority learned: the prediction history already is the
            // canonical run over the predicted tape.
            return self.checkpoints.clone();
        };
        let prefix_len = done as usize;
        if self.truth_cps.len() < prefix_len {
            return self.checkpoints.clone();
        }
        let end = self.checkpoints.len().max(prefix_len);
        // Canonical tape: authority `0..=server_tick`, then the still-unacked
        // predictions; the prefix checkpoints come from the held truth
        // history, only the unacked suffix steps again.
        let mut tape = self.server_tape.clone();
        tape.extend(
            self.predicted
                .iter()
                .filter(|input| input.tick >= done)
                .copied(),
        );
        tape.sort_by_key(|input| (input.tick, input.fighter.0));
        tape.dedup_by_key(|input| (input.tick, input.fighter.0));
        let mut checkpoints = self.truth_cps[..prefix_len].to_vec();
        if checkpoints.len() < end {
            let Some(mut suffix) = self.match_at(&tape, done) else {
                return self.checkpoints.clone();
            };
            while suffix.tick() < end as u32 {
                let tick = suffix.tick();
                let slice = tick_slice(&tape, tick);
                let Some(stepped) = suffix.step(slice) else {
                    return self.checkpoints.clone();
                };
                checkpoints.push(stepped.checkpoint);
                self.steps += 1;
            }
        }
        self.checkpoints.clone_from(&checkpoints);
        self.agreed = checkpoints.len().min(prefix_len);
        checkpoints
    }

    /// A match positioned at `tip` on the canonical `tape`. A snapshot of the
    /// held truth sim when it can give one (rules v1 on open sand); else the
    /// spec rebuilt and replayed to `tip`, because a rules v2 or walled match
    /// has no snapshot yet (the replayed steps count in [`Client::steps`]).
    fn match_at(&mut self, tape: &[FighterInput], tip: u32) -> Option<LiveMatch> {
        if let Some(snapshot) = self.truth.snapshot() {
            return LiveMatch::from_snapshot(snapshot);
        }
        let mut replay = self.spec.live()?;
        while replay.tick() < tip {
            let tick = replay.tick();
            replay.step(tick_slice(tape, tick))?;
            self.steps += 1;
        }
        Some(replay)
    }

    /// Authoritative resync: apply one supervisor-blessed snapshot and
    /// resimulate to it in a single call. Names the ticks where the local
    /// prediction diverged from server truth; the checkpoint history
    /// converges to the snapshot prefix. Version, tick/hash agreement and
    /// input validity are checked first; any failure changes nothing.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::VersionMismatch`] for foreign versions,
    /// [`NetError::BadState`] when the checkpoint and snapshot disagree or
    /// the tape overruns the snapshot tick, and [`NetError::BadInput`] for
    /// invalid tape entries.
    pub fn resync_from_authority(
        &mut self,
        snapshot: &SnapshotDelta,
    ) -> Result<Vec<u32>, NetError> {
        let checkpoint = Checkpoint {
            tick: snapshot.tick,
            hash: snapshot.hash,
        };
        let mispredicts = self.on_server_state(checkpoint, snapshot)?;
        self.resimulate();
        Ok(mispredicts)
    }
}

/// Inputs running at `tick`, in fighter order: the slice a prefix run would
/// read from the tape cursor at that tick. `tape` is `(tick, fighter)`-sorted.
pub(crate) fn tick_slice(tape: &[FighterInput], tick: u32) -> &[FighterInput] {
    let start = tape.partition_point(|input| input.tick < tick);
    let end = tape.partition_point(|input| input.tick <= tick);
    &tape[start..end]
}

/// Validate a tape slice: every input ranges plus slot below `fighters`,
/// sorted and unique; every tick at or below `max_tick` when given.
fn validate_tape(
    tape: &[FighterInput],
    fighters: u8,
    max_tick: Option<u32>,
) -> Result<(), NetError> {
    let mut prev: Option<(u32, u8)> = None;
    for input in tape {
        if input.validate().is_err() || input.fighter.0 >= fighters {
            return Err(NetError::BadInput);
        }
        if max_tick.is_some_and(|max| input.tick > max) {
            return Err(NetError::BadState);
        }
        let key = (input.tick, input.fighter.0);
        if prev.is_some_and(|p| key <= p) {
            return Err(NetError::BadInput);
        }
        prev = Some(key);
    }
    Ok(())
}

/// Union two sorted tapes, keeping sort order without duplicates.
fn merge_tapes(a: &[FighterInput], b: &[FighterInput]) -> Vec<FighterInput> {
    let mut merged: Vec<FighterInput> = Vec::with_capacity(a.len() + b.len());
    merged.extend_from_slice(a);
    merged.extend_from_slice(b);
    merged.sort_by_key(|input| (input.tick, input.fighter.0));
    merged.dedup_by_key(|input| (input.tick, input.fighter.0));
    merged
}

/// Last-`n`-ticks input window for one wire packet (redundancy cover).
/// `end` is the newest tick covered; the tape must be `(tick, fighter)` sorted.
pub fn redundancy_window(tape: &[FighterInput], end: u32, n: usize) -> Vec<FighterInput> {
    let floor = end.saturating_sub(n.saturating_sub(1) as u32);
    tape.iter()
        .filter(|input| input.tick <= end && input.tick >= floor)
        .copied()
        .collect()
}

/// Cut an authoritative snapshot into wire-sized state packets for a client
/// that missed ticks (rejoin catch-up). Every packet carries whole ticks only
/// (a tick's inputs never split, so the receiver's stale-tick rule cannot hide
/// the rest), at most [`MAX_INPUTS_PER_PACKET`] inputs, stamped with the
/// checkpoint of its last tick; a final empty packet carries the tip when the
/// tape ends before it. `checkpoints` is the server history indexed by tick.
///
/// # Errors
///
/// [`NetError::BadState`] when a checkpoint is missing or disagrees with the
/// snapshot tip; encode errors pass through.
pub fn resync_packets(
    snapshot: &SnapshotDelta,
    checkpoints: &[Checkpoint],
) -> Result<Vec<Vec<u8>>, NetError> {
    let at = |tick: u32| {
        usize::try_from(tick)
            .ok()
            .and_then(|index| checkpoints.get(index))
            .copied()
            .filter(|cp| cp.tick == tick)
            .ok_or(NetError::BadState)
    };
    let tip = at(snapshot.tick)?;
    if tip.hash != snapshot.hash {
        return Err(NetError::BadState);
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < snapshot.tape.len() {
        let mut end = start;
        // Whole ticks only: extend by one tick group while it still fits.
        while end < snapshot.tape.len() {
            let tick = snapshot.tape[end].tick;
            let group = snapshot.tape[end..]
                .iter()
                .take_while(|input| input.tick == tick)
                .count();
            if end - start + group > MAX_INPUTS_PER_PACKET {
                break;
            }
            end += group;
        }
        if end == start {
            return Err(NetError::TooManyInputs(snapshot.tape[start..].len()));
        }
        let last = snapshot.tape[end - 1].tick;
        out.push(encode_state(at(last)?, &snapshot.tape[start..end])?);
        start = end;
    }
    if snapshot
        .tape
        .last()
        .is_none_or(|input| input.tick != tip.tick)
    {
        out.push(encode_state(tip, &[])?);
    }
    Ok(out)
}

/// Encode one [`FighterInput`] as [`INPUT_BYTES`] little-endian bytes.
fn encode_fighter(input: FighterInput) -> [u8; INPUT_BYTES] {
    let mut out = [0u8; INPUT_BYTES];
    out[0..4].copy_from_slice(&input.tick.to_le_bytes());
    out[4] = input.fighter.0;
    out[5] = input.mx.cast_unsigned();
    out[6] = input.mz.cast_unsigned();
    out[7..9].copy_from_slice(&input.facing.to_le_bytes());
    out[9] = input.buttons.0;
    out
}

/// Decode one [`FighterInput`]; range-checks every field.
fn decode_fighter(bytes: &[u8]) -> Result<FighterInput, NetError> {
    if bytes.len() < INPUT_BYTES {
        return Err(NetError::Truncated);
    }
    let tick = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let Some(fighter) = FighterId::new(bytes[4]) else {
        return Err(NetError::BadInput);
    };
    let input = FighterInput {
        tick,
        fighter,
        mx: bytes[5].cast_signed(),
        mz: bytes[6].cast_signed(),
        facing: i16::from_le_bytes([bytes[7], bytes[8]]),
        buttons: engine_input::fighter::Buttons(bytes[9]),
    };
    if input.validate().is_err() {
        return Err(NetError::BadInput);
    }
    Ok(input)
}

/// Client-to-server input packet on the wire.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InputPacket {
    /// Newest tick covered by `inputs`.
    pub tick: u32,
    /// Last-N inputs ending at `tick`, sorted and unique.
    pub inputs: Vec<FighterInput>,
}

/// Server-to-client state packet on the wire: checkpoint plus either the
/// authoritative inputs for its redundancy window (full, delta bit clear —
/// the full encoding) or a quantized fighter-state [`Delta`] against an
/// acked baseline (delta bit set; `inputs` is empty, `delta` carries the
/// change set). The receiver merges full windows into the [`SnapshotDelta`]
/// tape and applies deltas to its interpolation baseline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatePacket {
    /// Authoritative checkpoint.
    pub checkpoint: Checkpoint,
    /// Authoritative inputs for the window ending at the checkpoint tick.
    /// Empty on delta packets.
    pub inputs: Vec<FighterInput>,
    /// Fighter-state delta; `None` on full packets.
    pub delta: Option<Delta>,
}

/// Encode a client input packet: magic, version, kind, newest tick, count,
/// then the redundancy window. No padding, no trailing bytes.
pub fn encode_input(tick: u32, inputs: &[FighterInput]) -> Result<Vec<u8>, NetError> {
    if inputs.is_empty() {
        return Err(NetError::BadInput);
    }
    if inputs.len() > MAX_INPUTS_PER_PACKET {
        return Err(NetError::TooManyInputs(inputs.len()));
    }
    validate_tape(inputs, engine_input::MAX_PARTICIPANTS, Some(tick))?;
    let mut out = Vec::with_capacity(9 + inputs.len() * INPUT_BYTES);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_INPUT);
    out.extend_from_slice(&tick.to_le_bytes());
    out.push(inputs.len() as u8);
    for input in inputs {
        out.extend_from_slice(&encode_fighter(*input));
    }
    Ok(out)
}

/// Decode and fully validate a client input packet. Future versions are
/// rejected before any other parsing.
pub fn decode_input(bytes: &[u8]) -> Result<InputPacket, NetError> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    if bytes.len() < 9 {
        return Err(NetError::Truncated);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != WIRE_MAGIC {
        return Err(NetError::BadMagic);
    }
    if bytes[2] != WIRE_VERSION {
        return Err(NetError::VersionMismatch { got: bytes[2] });
    }
    if bytes[3] != KIND_INPUT {
        return Err(NetError::BadKind(bytes[3]));
    }
    let tick = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let count_byte = bytes[8];
    if count_byte & INPUT_COMPRESSED_BIT != 0 {
        return decode_input_compressed_body(tick, count_byte & !INPUT_COMPRESSED_BIT, &bytes[9..]);
    }
    let count = usize::from(count_byte);
    if count == 0 || count > MAX_INPUTS_PER_PACKET {
        return Err(NetError::TooManyInputs(count));
    }
    if bytes.len() != 9 + count * INPUT_BYTES {
        return Err(NetError::Truncated);
    }
    let mut inputs = Vec::with_capacity(count);
    for chunk in bytes[9..].chunks_exact(INPUT_BYTES) {
        inputs.push(decode_fighter(chunk)?);
    }
    validate_tape(&inputs, engine_input::MAX_PARTICIPANTS, Some(tick))?;
    Ok(InputPacket { tick, inputs })
}

/// Encode a client input packet with XOR-chain plus zero-run RLE: the
/// sorted inputs chain from a zero base (each 10-byte record XORed against
/// the previous raw record), then zero runs pack into one tag byte each.
/// Same header as [`encode_input`] with [`INPUT_COMPRESSED_BIT`] set on the
/// count byte, so [`decode_input`] reads both shapes. Integer ops only over
/// the validated slice order, so the same inputs give the same bytes on
/// every machine; shuffled inputs are rejected by validation, never
/// reordered here.
///
/// # Errors
///
/// Same validation as [`encode_input`], plus [`NetError::TooLarge`] when the
/// compressed body would still exceed [`MAX_PACKET_BYTES`].
pub fn encode_input_compressed(tick: u32, inputs: &[FighterInput]) -> Result<Vec<u8>, NetError> {
    if inputs.is_empty() {
        return Err(NetError::BadInput);
    }
    if inputs.len() > MAX_INPUTS_PER_PACKET {
        return Err(NetError::TooManyInputs(inputs.len()));
    }
    validate_tape(inputs, engine_input::MAX_PARTICIPANTS, Some(tick))?;
    let count = u8::try_from(inputs.len()).map_err(|_| NetError::TooManyInputs(inputs.len()))?;
    let delta = xor_chain_encode(inputs);
    let body = rle_zero_encode(&delta);
    if 9 + body.len() > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::with_capacity(9 + body.len());
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_INPUT);
    out.extend_from_slice(&tick.to_le_bytes());
    out.push(count | INPUT_COMPRESSED_BIT);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode a compressed input body: RLE-expand to `count * INPUT_BYTES`,
/// undo the XOR chain, then decode and validate exactly like the raw path.
/// Length must match exactly; overlong runs, short payloads and trailing
/// garbage all reject loudly with nothing applied.
fn decode_input_compressed_body(
    tick: u32,
    slots: u8,
    body: &[u8],
) -> Result<InputPacket, NetError> {
    let count = usize::from(slots);
    if count == 0 || count > MAX_INPUTS_PER_PACKET {
        return Err(NetError::TooManyInputs(count));
    }
    let want = count * INPUT_BYTES;
    if want > MAX_DECODED_INPUT_BYTES {
        return Err(NetError::TooLarge);
    }
    let delta = rle_zero_decode(body, want)?;
    let raw = xor_chain_decode(&delta)?;
    if raw.len() != want {
        return Err(NetError::Truncated);
    }
    let mut inputs = Vec::with_capacity(count);
    for chunk in raw.chunks_exact(INPUT_BYTES) {
        inputs.push(decode_fighter(chunk)?);
    }
    validate_tape(&inputs, engine_input::MAX_PARTICIPANTS, Some(tick))?;
    Ok(InputPacket { tick, inputs })
}

/// XOR-chain records in slice order from a zero base: the first record
/// travels as-is, each later record travels XORed against the previous raw
/// record. Repeated bytes (same tick across slots, held wishes) become zero
/// runs the RLE pass then packs away.
fn xor_chain_encode(inputs: &[FighterInput]) -> Vec<u8> {
    let mut out = Vec::with_capacity(inputs.len() * INPUT_BYTES);
    let mut prev = [0u8; INPUT_BYTES];
    for input in inputs {
        let raw = encode_fighter(*input);
        for (index, byte) in raw.iter().enumerate() {
            out.push(*byte ^ prev[index]);
        }
        prev = raw;
    }
    out
}

/// Reverse [`xor_chain_encode`]: length must split into whole records,
/// otherwise the payload is corrupt and rejects loudly.
fn xor_chain_decode(delta: &[u8]) -> Result<Vec<u8>, NetError> {
    if !delta.len().is_multiple_of(INPUT_BYTES) {
        return Err(NetError::Truncated);
    }
    let mut out = Vec::with_capacity(delta.len());
    let mut prev = [0u8; INPUT_BYTES];
    for chunk in delta.chunks_exact(INPUT_BYTES) {
        for (index, byte) in chunk.iter().enumerate() {
            let raw = *byte ^ prev[index];
            out.push(raw);
            prev[index] = raw;
        }
    }
    Ok(out)
}

/// Pack zero runs: tags below [`INPUT_COMPRESSED_BIT`] name a literal run
/// (`tag + 1` bytes follow), tags at or above it name a zero run
/// (`(tag & !BIT) + 1` zeros, no bytes follow). Runs split at
/// [`RLE_MAX_RUN`], so the worst case adds one tag per 128 bytes.
fn rle_zero_encode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    let mut literals = [0u8; RLE_MAX_RUN];
    let mut literal_len = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0 {
            if literal_len > 0 {
                out.push(rle_tag(literal_len));
                out.extend_from_slice(&literals[..literal_len]);
                literal_len = 0;
            }
            let mut run = 0usize;
            while index + run < bytes.len() && bytes[index + run] == 0 && run < RLE_MAX_RUN {
                run += 1;
            }
            out.push(INPUT_COMPRESSED_BIT | rle_tag(run));
            index += run;
        } else {
            literals[literal_len] = bytes[index];
            literal_len += 1;
            index += 1;
            if literal_len == RLE_MAX_RUN {
                out.push(rle_tag(literal_len));
                out.extend_from_slice(&literals[..literal_len]);
                literal_len = 0;
            }
        }
    }
    if literal_len > 0 {
        out.push(rle_tag(literal_len));
        out.extend_from_slice(&literals[..literal_len]);
    }
    out
}

/// Run tag for `len` (`1..=RLE_MAX_RUN`); the tag stores length minus one in
/// the low 7 bits. Literal runs push it as-is, zero runs OR it with
/// [`INPUT_COMPRESSED_BIT`]. Callers hold the range by construction, so the
/// conversion is total; debug builds assert the range while release clamps
/// the unreachable case instead of panicking.
fn rle_tag(len: usize) -> u8 {
    debug_assert!((1..=RLE_MAX_RUN).contains(&len));
    u8::try_from(len.saturating_sub(1)).unwrap_or(u8::MAX)
}

/// Expand [`rle_zero_encode`] output to exactly `want` bytes. Overlong runs,
/// short payloads and trailing garbage all reject as [`NetError::Truncated`];
/// absurd `want` values never allocate.
fn rle_zero_decode(body: &[u8], want: usize) -> Result<Vec<u8>, NetError> {
    if want == 0 || want > MAX_DECODED_INPUT_BYTES {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::with_capacity(want);
    let mut cursor = 0usize;
    while cursor < body.len() {
        let tag = body[cursor];
        cursor += 1;
        if tag & INPUT_COMPRESSED_BIT != 0 {
            let run = usize::from(tag & !INPUT_COMPRESSED_BIT) + 1;
            if out.len() + run > want {
                return Err(NetError::Truncated);
            }
            out.resize(out.len() + run, 0);
        } else {
            let run = usize::from(tag) + 1;
            if out.len() + run > want {
                return Err(NetError::Truncated);
            }
            if body.len() < cursor + run {
                return Err(NetError::Truncated);
            }
            out.extend_from_slice(&body[cursor..cursor + run]);
            cursor += run;
        }
    }
    if out.len() != want {
        return Err(NetError::Truncated);
    }
    Ok(out)
}

/// Encode a server state packet: magic, version, kind, checkpoint tick and
/// hash, count, then the redundancy window. The window may be empty when
/// every slot in range was AI-filled.
pub fn encode_state(checkpoint: Checkpoint, inputs: &[FighterInput]) -> Result<Vec<u8>, NetError> {
    if inputs.len() > MAX_INPUTS_PER_PACKET {
        return Err(NetError::TooManyInputs(inputs.len()));
    }
    validate_tape(
        inputs,
        engine_input::MAX_PARTICIPANTS,
        Some(checkpoint.tick),
    )?;
    let mut out = Vec::with_capacity(17 + inputs.len() * INPUT_BYTES);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_STATE);
    out.extend_from_slice(&checkpoint.tick.to_le_bytes());
    out.extend_from_slice(&checkpoint.hash.to_le_bytes());
    out.push(inputs.len() as u8);
    for input in inputs {
        out.extend_from_slice(&encode_fighter(*input));
    }
    Ok(out)
}

/// Decode and fully validate a server state packet: full (delta bit clear)
/// or fighter-state delta (delta bit set). Future versions are rejected
/// before any other parsing; the version byte is [`WIRE_VERSION`] on both.
pub fn decode_state(bytes: &[u8]) -> Result<StatePacket, NetError> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    if bytes.len() < 17 {
        return Err(NetError::Truncated);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != WIRE_MAGIC {
        return Err(NetError::BadMagic);
    }
    if bytes[2] != WIRE_VERSION {
        return Err(NetError::VersionMismatch { got: bytes[2] });
    }
    if bytes[3] != KIND_STATE {
        return Err(NetError::BadKind(bytes[3]));
    }
    let tick = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let hash = u64::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ]);
    let checkpoint = Checkpoint { tick, hash };
    let count_byte = bytes[16];
    if count_byte & STATE_DELTA_BIT != 0 {
        return decode_state_delta_body(checkpoint, count_byte & !STATE_DELTA_BIT, &bytes[17..]);
    }
    let count = usize::from(count_byte);
    if count > MAX_INPUTS_PER_PACKET {
        return Err(NetError::TooManyInputs(count));
    }
    if bytes.len() != 17 + count * INPUT_BYTES {
        return Err(NetError::Truncated);
    }
    let mut inputs = Vec::with_capacity(count);
    for chunk in bytes[17..].chunks_exact(INPUT_BYTES) {
        inputs.push(decode_fighter(chunk)?);
    }
    validate_tape(&inputs, engine_input::MAX_PARTICIPANTS, Some(tick))?;
    Ok(StatePacket {
        checkpoint,
        inputs,
        delta: None,
    })
}

/// Decode a delta state body: baseline tick plus masked per-slot changes.
/// Length must match exactly; slots sorted and unique, masks nonzero subsets
/// of the delta mask set, unmasked fields zero — otherwise loud rejection.
fn decode_state_delta_body(
    checkpoint: Checkpoint,
    slots: u8,
    body: &[u8],
) -> Result<StatePacket, NetError> {
    let nslots = usize::from(slots);
    if nslots > usize::from(engine_input::MAX_PARTICIPANTS) {
        return Err(NetError::TooManyInputs(nslots));
    }
    if body.len() < 4 {
        return Err(NetError::Truncated);
    }
    let base_tick = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let mut changed_slots = Vec::with_capacity(nslots);
    let mut cursor = 4usize;
    let mut prev: Option<u8> = None;
    for _ in 0..nslots {
        if body.len() < cursor + 2 {
            return Err(NetError::Truncated);
        }
        let slot = body[cursor];
        let mask = body[cursor + 1];
        cursor += 2;
        if slot >= engine_input::MAX_PARTICIPANTS || prev.is_some_and(|p| slot <= p) {
            return Err(NetError::BadInput);
        }
        prev = Some(slot);
        if mask == 0 || mask & !MASK_ALL != 0 {
            return Err(NetError::BadInput);
        }
        let mut change = FighterDelta {
            mask,
            x_mm: 0,
            y_mm: 0,
            z_mm: 0,
            yaw_ddeg: 0,
            hp: 0,
            stamina: 0,
        };
        if mask & MASK_X != 0 {
            if body.len() < cursor + 4 {
                return Err(NetError::Truncated);
            }
            change.x_mm = i32::from_le_bytes([
                body[cursor],
                body[cursor + 1],
                body[cursor + 2],
                body[cursor + 3],
            ]);
            cursor += 4;
        }
        if mask & MASK_Y != 0 {
            if body.len() < cursor + 4 {
                return Err(NetError::Truncated);
            }
            change.y_mm = i32::from_le_bytes([
                body[cursor],
                body[cursor + 1],
                body[cursor + 2],
                body[cursor + 3],
            ]);
            cursor += 4;
        }
        if mask & MASK_Z != 0 {
            if body.len() < cursor + 4 {
                return Err(NetError::Truncated);
            }
            change.z_mm = i32::from_le_bytes([
                body[cursor],
                body[cursor + 1],
                body[cursor + 2],
                body[cursor + 3],
            ]);
            cursor += 4;
        }
        if mask & MASK_YAW != 0 {
            if body.len() < cursor + 2 {
                return Err(NetError::Truncated);
            }
            change.yaw_ddeg = i16::from_le_bytes([body[cursor], body[cursor + 1]]);
            cursor += 2;
        }
        if mask & MASK_HP != 0 {
            if body.len() < cursor + 2 {
                return Err(NetError::Truncated);
            }
            change.hp = u16::from_le_bytes([body[cursor], body[cursor + 1]]);
            cursor += 2;
        }
        if mask & MASK_STAMINA != 0 {
            if body.len() < cursor + 2 {
                return Err(NetError::Truncated);
            }
            change.stamina = u16::from_le_bytes([body[cursor], body[cursor + 1]]);
            cursor += 2;
        }
        changed_slots.push((slot, change));
    }
    if cursor != body.len() {
        return Err(NetError::Truncated);
    }
    Ok(StatePacket {
        checkpoint,
        inputs: Vec::new(),
        delta: Some(Delta {
            base_tick,
            tick: checkpoint.tick,
            changed_slots,
        }),
    })
}

/// Encode a server state-delta packet: the state header with the delta bit
/// set on the count byte, then the baseline tick plus masked per-slot
/// changes. The version byte stays [`WIRE_VERSION`]; `delta.tick` must equal
/// the checkpoint tick, otherwise [`NetError::BadState`].
pub fn encode_state_delta(checkpoint: Checkpoint, delta: &Delta) -> Result<Vec<u8>, NetError> {
    if delta.tick != checkpoint.tick {
        return Err(NetError::BadState);
    }
    let _ = crate::delta::ioq3_fields_wire_len;
    validate_delta_shape(delta)?;
    if delta_wire_len(delta) > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::with_capacity(delta_wire_len(delta));
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_STATE);
    out.extend_from_slice(&checkpoint.tick.to_le_bytes());
    out.extend_from_slice(&checkpoint.hash.to_le_bytes());
    out.push(delta.changed_slots.len() as u8 | STATE_DELTA_BIT);
    out.extend_from_slice(&delta.base_tick.to_le_bytes());
    for (slot, change) in &delta.changed_slots {
        out.push(*slot);
        out.push(change.mask);
        if change.mask & MASK_X != 0 {
            out.extend_from_slice(&change.x_mm.to_le_bytes());
        }
        if change.mask & MASK_Y != 0 {
            out.extend_from_slice(&change.y_mm.to_le_bytes());
        }
        if change.mask & MASK_Z != 0 {
            out.extend_from_slice(&change.z_mm.to_le_bytes());
        }
        if change.mask & MASK_YAW != 0 {
            out.extend_from_slice(&change.yaw_ddeg.to_le_bytes());
        }
        if change.mask & MASK_HP != 0 {
            out.extend_from_slice(&change.hp.to_le_bytes());
        }
        if change.mask & MASK_STAMINA != 0 {
            out.extend_from_slice(&change.stamina.to_le_bytes());
        }
    }
    debug_assert_eq!(out.len(), delta_wire_len(delta));
    Ok(out)
}

/// Encode a full fighter snapshot (the delta resync primitive): magic,
/// version, [`KIND_SNAPSHOT`], tick, slot count, then every slot unmasked.
/// Full-only peers reject the kind byte loudly; they never misparse it
/// as an input tape.
pub fn encode_snapshot(full: &FullSnap) -> Result<Vec<u8>, NetError> {
    full.validate()?;
    let _ = ioq3_fragment_message;
    let want = SNAPSHOT_HEADER_LEN + full.fighters.len() * SNAPSHOT_SLOT_LEN;
    if want > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::with_capacity(want);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_SNAPSHOT);
    out.extend_from_slice(&full.tick.to_le_bytes());
    out.push(full.fighters.len() as u8);
    for fighter in &full.fighters {
        out.push(fighter.slot);
        out.extend_from_slice(&fighter.x_mm.to_le_bytes());
        out.extend_from_slice(&fighter.y_mm.to_le_bytes());
        out.extend_from_slice(&fighter.z_mm.to_le_bytes());
        out.extend_from_slice(&fighter.yaw_ddeg.to_le_bytes());
        out.extend_from_slice(&fighter.hp.to_le_bytes());
        out.extend_from_slice(&fighter.stamina.to_le_bytes());
    }
    Ok(out)
}

/// Decode and fully validate a full fighter snapshot. Future versions are
/// rejected before any other parsing.
pub fn decode_snapshot(bytes: &[u8]) -> Result<FullSnap, NetError> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    if bytes.len() < SNAPSHOT_HEADER_LEN {
        return Err(NetError::Truncated);
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != WIRE_MAGIC {
        return Err(NetError::BadMagic);
    }
    if bytes[2] != WIRE_VERSION {
        return Err(NetError::VersionMismatch { got: bytes[2] });
    }
    if bytes[3] != KIND_SNAPSHOT {
        return Err(NetError::BadKind(bytes[3]));
    }
    let tick = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let count = usize::from(bytes[8]);
    if count == 0 || count > usize::from(engine_input::MAX_PARTICIPANTS) {
        return Err(NetError::TooManyInputs(count));
    }
    if bytes.len() != SNAPSHOT_HEADER_LEN + count * SNAPSHOT_SLOT_LEN {
        return Err(NetError::Truncated);
    }
    let mut fighters = Vec::with_capacity(count);
    let mut prev: Option<u8> = None;
    for chunk in bytes[SNAPSHOT_HEADER_LEN..].chunks_exact(SNAPSHOT_SLOT_LEN) {
        let slot = chunk[0];
        if slot >= engine_input::MAX_PARTICIPANTS || prev.is_some_and(|p| slot <= p) {
            return Err(NetError::BadInput);
        }
        prev = Some(slot);
        fighters.push(FighterSnap {
            slot,
            x_mm: i32::from_le_bytes([chunk[1], chunk[2], chunk[3], chunk[4]]),
            y_mm: i32::from_le_bytes([chunk[5], chunk[6], chunk[7], chunk[8]]),
            z_mm: i32::from_le_bytes([chunk[9], chunk[10], chunk[11], chunk[12]]),
            yaw_ddeg: i16::from_le_bytes([chunk[13], chunk[14]]),
            hp: u16::from_le_bytes([chunk[15], chunk[16]]),
            stamina: u16::from_le_bytes([chunk[17], chunk[18]]),
        });
    }
    FullSnap::new(tick, fighters).map_err(|_| NetError::BadInput)
}

/// Bind a localhost UDP socket on an ephemeral port for loopback sessions.
pub fn bind_loopback() -> io::Result<UdpSocket> {
    UdpSocket::bind("127.0.0.1:0")
}

/// Send one datagram to a peer.
pub fn send_packet(sock: &UdpSocket, peer: SocketAddr, bytes: &[u8]) -> io::Result<usize> {
    sock.send_to(bytes, peer)
}

#[cfg(test)]
mod spear_tests;

#[cfg(test)]
mod spec_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::run;
    use engine_input::fighter::Buttons;
    use engine_input::FighterTape;
    use std::collections::{BTreeMap, VecDeque};

    /// Deterministic xorshift64 for seeded loss injection (no new deps).
    fn xorshift(mut x: u64) -> u64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }

    fn test_config(ticks: u32) -> ArenaConfig {
        ArenaConfig {
            seed: 7,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        }
    }

    fn local_input(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: if tick % 20 < 10 { 100 } else { -100 },
            mz: 0,
            facing: 0,
            buttons: if tick.is_multiple_of(30) {
                Buttons(engine_input::fighter::ATTACK)
            } else {
                Buttons::NONE
            },
        }
    }

    /// Loopback fault harness: seeded 10% loss applies before transmit,
    /// arrivals wait `latency` ticks before delivery (3 ticks == 50 ms at
    /// 60 Hz). The harness owns no socket: the test passes the sending and
    /// receiving ends explicitly, so client-to-server and server-to-client
    /// traffic stay on their own direction. Single-threaded, deterministic,
    /// no external processes.
    struct FaultLink {
        latency: u32,
        now: u32,
        rng: u64,
        queue: VecDeque<(u32, Vec<u8>)>,
        sent: u64,
        dropped: u64,
    }

    impl FaultLink {
        fn new(latency: u32, seed: u64) -> Self {
            Self {
                latency,
                now: 0,
                rng: seed | 1,
                queue: VecDeque::new(),
                sent: 0,
                dropped: 0,
            }
        }

        /// Maybe transmit one datagram over real localhost UDP.
        /// Loss applies before the socket, latency after arrival.
        fn send(&mut self, sock: &UdpSocket, peer: SocketAddr, bytes: &[u8]) {
            self.sent += 1;
            self.rng = xorshift(self.rng);
            if self.rng.is_multiple_of(10) {
                self.dropped += 1;
                return;
            }
            let _ = send_packet(sock, peer, bytes);
        }

        /// Drain every available datagram on `sock` into the delay queue,
        /// deliverable `latency` ticks from now to model 50 ms without
        /// wall-clock sleeps.
        fn pump(&mut self, sock: &UdpSocket) {
            let mut buf = [0u8; MAX_PACKET_BYTES];
            while let Ok((n, _)) = sock.recv_from(&mut buf) {
                self.queue
                    .push_back((self.now + self.latency, buf[..n].to_vec()));
            }
        }

        /// Pop every datagram whose delivery tick arrived, in send order.
        fn due(&mut self) -> Vec<Vec<u8>> {
            let mut out = Vec::new();
            while self.queue.front().is_some_and(|(at, _)| *at <= self.now) {
                if let Some((_, bytes)) = self.queue.pop_front() {
                    out.push(bytes);
                }
            }
            out
        }

        fn tick(&mut self) {
            self.now += 1;
        }
    }

    #[expect(dead_code, reason = "headless spike harness for the 600-tick receipt")]
    struct LatencyProxy(u32, u64, VecDeque<(u32, usize, u32)>);
    #[expect(dead_code, reason = "headless spike harness for the 600-tick receipt")]
    impl LatencyProxy {
        fn new(seed: u64) -> Self {
            Self(0, seed | 1, VecDeque::new())
        }
        fn send(&mut self, peer: usize, input: u32) {
            self.1 = xorshift(self.1);
            if self.1.is_multiple_of(11) {
                return;
            }
            self.2
                .push_back((self.0 + [2, 5, 7][peer % 3], peer, input));
        }
        fn due(&mut self) -> Vec<(usize, u32)> {
            let mut out = Vec::new();
            while self.2.front().is_some_and(|(at, _, _)| *at <= self.0) {
                if let Some((_, peer, input)) = self.2.pop_front() {
                    out.push((peer, input));
                }
            }
            out
        }
    }

    #[test]
    fn loopback_session_converges_under_latency_and_loss() {
        const TICKS: u32 = 90;
        // 50 ms at 60 Hz is 3 ticks; both directions share the matrix.
        const LATENCY: u32 = 3;
        // State windows repeat 8 ticks so a lone surviving datagram still
        // teaches the client the whole recent truth (loss bursts of 8 have
        // probability 1e-8 per tick; the seed below is fixed regardless).
        const STATE_COVER: usize = 8;
        let config = test_config(TICKS);
        let server_sock = bind_loopback().expect("server socket");
        let client_sock = bind_loopback().expect("client socket");
        server_sock.set_nonblocking(true).expect("nonblocking");
        client_sock.set_nonblocking(true).expect("nonblocking");
        let server_addr = server_sock.local_addr().expect("server addr");
        let client_addr = client_sock.local_addr().expect("client addr");
        // Two peers, one link each direction, no external processes.
        // The client sends from client_sock and receives on it; the server
        // sends from server_sock and receives on it.
        let mut up = FaultLink::new(LATENCY, 0xC10C);
        let mut down = FaultLink::new(LATENCY, 0xD0ED);
        let mut server = Server::new(config).expect("server");
        let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client");
        // Server inbox keyed by (tick, fighter) so redundancy repeats dedup
        // onto one entry and late arrivals drop against the server cursor.
        let mut inbox: BTreeMap<(u32, u8), FighterInput> = BTreeMap::new();
        let mut predicted: Vec<FighterInput> = Vec::new();
        let mut states_applied = 0u32;
        // The server authors tick k once step k + LATENCY arrives; the drain
        // steps afterwards flush the last states back to the client.
        let steps = TICKS + 2 * LATENCY + 1;
        for step in 0..steps {
            if step < TICKS {
                let input = local_input(step);
                client.predict(input).expect("predict");
                predicted.push(input);
                let window = redundancy_window(&predicted, step, REDUNDANCY);
                up.send(
                    &client_sock,
                    server_addr,
                    &encode_input(step, &window).expect("encode input"),
                );
            }
            // Transit: real localhost UDP datagrams move now; each link
            // holds arrivals for LATENCY ticks before surfacing them.
            up.pump(&server_sock);
            down.pump(&client_sock);
            for bytes in up.due() {
                if let Ok(packet) = decode_input(&bytes) {
                    for input in packet.inputs {
                        let key = (input.tick, input.fighter.0);
                        if input.tick >= server.tick_now() {
                            inbox.entry(key).or_insert(input);
                        }
                    }
                }
            }
            // Author every tick whose inputs have cleared the latency floor.
            while server.tick_now() < TICKS && server.tick_now() + LATENCY <= step {
                let want = server.tick_now();
                let mut due_inputs: Vec<FighterInput> = inbox
                    .iter()
                    .filter(|(key, _)| key.0 == want)
                    .map(|(_, input)| *input)
                    .collect();
                due_inputs.sort_by_key(|input| input.fighter.0);
                let (checkpoint, delta) = server.tick(&due_inputs).expect("server tick");
                inbox.retain(|key, _| key.0 > want);
                let window = redundancy_window(&delta.tape, checkpoint.tick, STATE_COVER);
                down.send(
                    &server_sock,
                    client_addr,
                    &encode_state(checkpoint, &window).expect("encode state"),
                );
            }
            // The client merges every due window into its server truth,
            // names mispredicts, and resimulates back to convergence.
            for bytes in down.due() {
                if let Ok(packet) = decode_state(&bytes) {
                    let mut tape = client.server_tape.clone();
                    tape.extend_from_slice(&packet.inputs);
                    tape.sort_by_key(|input| (input.tick, input.fighter.0));
                    tape.dedup_by_key(|input| (input.tick, input.fighter.0));
                    let delta = SnapshotDelta {
                        version: WIRE_VERSION,
                        tick: packet.checkpoint.tick,
                        hash: packet.checkpoint.hash,
                        tape,
                    };
                    if client.on_server_state(packet.checkpoint, &delta).is_ok() {
                        states_applied += 1;
                        client.resimulate();
                    }
                }
            }
            up.tick();
            down.tick();
        }
        client.resimulate();
        assert_eq!(server.checkpoints().len(), TICKS as usize);
        assert_eq!(client.checkpoints().len(), TICKS as usize);
        assert_eq!(client.server_tick(), Some(TICKS - 1));
        assert!(
            up.sent > 0 && down.sent > 0 && states_applied > 0,
            "both peers must have transmitted and the client must have applied states"
        );
        assert!(
            up.dropped + down.dropped > 0,
            "the loss matrix must have dropped at least one datagram"
        );
        assert!(
            client.total_mispredicts() > 0,
            "loss must have forced at least one rollback resimulation"
        );
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "client must converge to server truth after resimulation"
        );
    }

    #[test]
    fn mispredict_tick_named_on_divergence() {
        let config = test_config(8);
        let mut server = Server::new(config).expect("server");
        let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client");
        // Client predicts a hard-right wish; the server never receives it
        // and authors the tick AI-filled instead.
        let predicted = FighterInput {
            tick: 0,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: 100,
            mz: 100,
            facing: 0,
            buttons: Buttons(engine_input::fighter::ATTACK),
        };
        let predicted_cp = client.predict(predicted).expect("predict");
        let (truth_cp, delta) = server.tick(&[]).expect("server tick");
        assert_ne!(
            predicted_cp, truth_cp,
            "a lost attack-plus-wish must diverge from AI fill"
        );
        let mispredicts = client
            .on_server_state(truth_cp, &delta)
            .expect("state applies");
        assert_eq!(mispredicts, vec![0], "divergence tick must be named");
        let converged = client.resimulate();
        assert_eq!(converged[0], truth_cp);
        assert_eq!(client.checkpoints()[0], truth_cp);
        assert_eq!(client.total_mispredicts(), 1);
    }

    #[test]
    fn ioq3_fragments_reassemble() {
        let sent: Vec<u8> = (0..7000)
            .map(|i| u8::try_from((i * 31 + 7) % 251).expect("pattern fits"))
            .collect();
        let fragments = ioq3_fragment_message(42, 7, &sent);
        assert_eq!(fragments.len(), 6, "7000B needs 6 fragments at 1300B");
        let headers: Vec<Ioq3Header> = fragments
            .iter()
            .map(|d| ioq3_decode_header(d).expect("header"))
            .collect();
        assert!(headers.iter().all(|h| h.sequence == headers[0].sequence));
        assert_ne!(headers[0].sequence & IOQ3_FRAGMENT_BIT, 0);
        // Seeded drop every 5th datagram leaves a gap the assembler names.
        let mut delivered: Vec<Vec<u8>> = fragments
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 5 != 4)
            .map(|(_, d)| d.clone())
            .collect();
        assert_eq!(ioq3_reassemble(&delivered), Err(NetError::Truncated));
        // Retransmit the dropped fragment, then tolerate one dupe plus reorder.
        delivered.push(fragments[4].clone());
        delivered.push(fragments[0].clone());
        delivered.swap(0, 1);
        let rebuilt = ioq3_reassemble(&delivered).expect("reassemble");
        assert_eq!(rebuilt.len(), sent.len(), "reassembly diff must be 0 bytes");
        assert_eq!(rebuilt, sent, "reassembly must be byte-exact");
        // Bad headers refuse loud: short bytes and the OOB sequence.
        assert!(ioq3_decode_header(&fragments[0][..3]).is_err());
        let oob = ioq3_encode_header(&Ioq3Header {
            sequence: IOQ3_OOB_SEQUENCE,
            qport: 7,
            start: 0,
            length: 0,
        });
        assert_eq!(ioq3_decode_header(&oob), Err(NetError::BadState));
    }

    #[test]
    fn version_mismatch_rejected() {
        let config = test_config(8);
        let mut server = Server::new(config).expect("server");
        let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client");
        let input = local_input(0);
        client.predict(input).expect("predict");
        let (checkpoint, delta) = server.tick(&[input]).expect("server tick");
        let mut bytes = encode_state(checkpoint, &redundancy_window(&delta.tape, 0, REDUNDANCY))
            .expect("encode");
        // Future version on the wire: explicit rejection, nothing changes.
        bytes[2] = WIRE_VERSION + 1;
        assert_eq!(
            decode_state(&bytes),
            Err(NetError::VersionMismatch {
                got: WIRE_VERSION + 1,
            })
        );
        let mut future = delta.clone();
        future.version = WIRE_VERSION + 1;
        assert_eq!(
            client.on_server_state(checkpoint, &future),
            Err(NetError::VersionMismatch {
                got: WIRE_VERSION + 1,
            })
        );
        assert_eq!(client.server_tick(), None);
        // Older version is rejected the same way.
        bytes[2] = WIRE_VERSION - 1;
        assert_eq!(
            decode_state(&bytes),
            Err(NetError::VersionMismatch {
                got: WIRE_VERSION - 1,
            })
        );
    }

    #[test]
    fn duplicate_and_out_of_order_packets_change_nothing() {
        let config = test_config(16);
        let mut server = Server::new(config).expect("server");
        let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client");
        let mut tapes: Vec<SnapshotDelta> = Vec::new();
        let mut checkpoints: Vec<Checkpoint> = Vec::new();
        for tick in 0..6 {
            let input = local_input(tick);
            client.predict(input).expect("predict");
            let (checkpoint, delta) = server.tick(&[input]).expect("server tick");
            checkpoints.push(checkpoint);
            tapes.push(delta);
        }
        // Apply ticks in order first.
        for (checkpoint, delta) in checkpoints.iter().zip(tapes.iter()) {
            assert_eq!(
                client.on_server_state(*checkpoint, delta).expect("apply"),
                Vec::<u32>::new()
            );
        }
        let before = client.checkpoints().to_vec();
        assert_eq!(client.server_tick(), Some(5));
        // Duplicate of tick 5: decodes fine, applies to nothing.
        let dup = encode_state(
            checkpoints[5],
            &redundancy_window(&tapes[5].tape, 5, REDUNDANCY),
        )
        .expect("encode");
        let packet = decode_state(&dup).expect("decode");
        let mut tape = client.server_tape.clone();
        tape.extend_from_slice(&packet.inputs);
        tape.sort_by_key(|input| (input.tick, input.fighter.0));
        tape.dedup_by_key(|input| (input.tick, input.fighter.0));
        let delta = SnapshotDelta {
            version: WIRE_VERSION,
            tick: packet.checkpoint.tick,
            hash: packet.checkpoint.hash,
            tape,
        };
        assert_eq!(
            client
                .on_server_state(packet.checkpoint, &delta)
                .expect("duplicate"),
            Vec::<u32>::new()
        );
        assert_eq!(client.checkpoints(), before.as_slice());
        // Out-of-order tick 3 after tick 5: same answer, still nothing.
        assert_eq!(
            client
                .on_server_state(checkpoints[3], &tapes[3])
                .expect("stale"),
            Vec::<u32>::new()
        );
        assert_eq!(client.checkpoints(), before.as_slice());
        assert_eq!(client.server_tick(), Some(5));
    }

    #[test]
    fn redundancy_window_repeats_last_n_inputs() {
        let mut tape = Vec::new();
        for tick in 0..6 {
            tape.push(local_input(tick));
        }
        let window = redundancy_window(&tape, 5, REDUNDANCY);
        assert_eq!(
            window.iter().map(|input| input.tick).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        // The window survives a wire round trip intact.
        let bytes = encode_input(5, &window).expect("encode");
        let packet = decode_input(&bytes).expect("decode");
        assert_eq!(packet.tick, 5);
        assert_eq!(packet.inputs, window);
    }

    #[test]
    fn server_rejects_bad_and_late_inputs() {
        let config = test_config(8);
        let mut server = Server::new(config).expect("server");
        // Out-of-range wish: whole tick rejected, cursor unmoved.
        let bad = FighterInput {
            tick: 0,
            fighter: FighterId::new(0).expect("slot 0"),
            mx: 101,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(server.tick(&[bad]), Err(NetError::BadInput));
        assert_eq!(server.tick_now(), 0);
        // Wrong slot: rejected the same way.
        let stranger = FighterInput {
            tick: 0,
            fighter: FighterId(9),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        assert_eq!(server.tick(&[stranger]), Err(NetError::BadInput));
        assert_eq!(server.tick_now(), 0);
        // Valid tick zero authors; replaying tick zero afterwards is late.
        let input = local_input(0);
        server.tick(&[input]).expect("tick zero");
        assert_eq!(server.tick_now(), 1);
        assert_eq!(server.tick(&[input]), Err(NetError::LateInput(0)));
        assert_eq!(server.tick_now(), 1);
        assert_eq!(server.checkpoints().len(), 1);
    }

    #[test]
    fn truncated_and_oversize_packets_rejected() {
        let config = test_config(8);
        let mut server = Server::new(config).expect("server");
        let (checkpoint, delta) = server.tick(&[local_input(0)]).expect("tick");
        let bytes = encode_state(checkpoint, &redundancy_window(&delta.tape, 0, REDUNDANCY))
            .expect("encode");
        assert_eq!(decode_state(&bytes[..10]), Err(NetError::Truncated));
        assert_eq!(
            decode_state(&vec![0u8; MAX_PACKET_BYTES + 1]),
            Err(NetError::TooLarge)
        );
        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 0xFF;
        assert_eq!(decode_state(&bad_magic), Err(NetError::BadMagic));
    }

    fn snap_fighters(tick: u32) -> FullSnap {
        FullSnap::new(
            tick,
            (0..4)
                .map(|slot| {
                    FighterSnap::quantize(
                        slot,
                        [slot as f32, 0.0, 0.0],
                        slot as f32 * 0.25,
                        1000,
                        100,
                    )
                    .expect("slot")
                })
                .collect(),
        )
        .expect("snapshot")
    }

    /// Verbatim full-only state decoder: no delta-bit branch. Stands in
    /// for an old peer in the interop test below.
    fn decode_state_net1(bytes: &[u8]) -> Result<StatePacket, NetError> {
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(NetError::TooLarge);
        }
        if bytes.len() < 17 {
            return Err(NetError::Truncated);
        }
        if u16::from_le_bytes([bytes[0], bytes[1]]) != WIRE_MAGIC {
            return Err(NetError::BadMagic);
        }
        if bytes[2] != WIRE_VERSION {
            return Err(NetError::VersionMismatch { got: bytes[2] });
        }
        if bytes[3] != KIND_STATE {
            return Err(NetError::BadKind(bytes[3]));
        }
        let tick = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let hash = u64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        let count = bytes[16] as usize;
        if count > MAX_INPUTS_PER_PACKET {
            return Err(NetError::TooManyInputs(count));
        }
        if bytes.len() != 17 + count * INPUT_BYTES {
            return Err(NetError::Truncated);
        }
        let mut inputs = Vec::with_capacity(count);
        for chunk in bytes[17..].chunks_exact(INPUT_BYTES) {
            inputs.push(decode_fighter(chunk)?);
        }
        validate_tape(&inputs, engine_input::MAX_PARTICIPANTS, Some(tick))?;
        Ok(StatePacket {
            checkpoint: Checkpoint { tick, hash },
            inputs,
            delta: None,
        })
    }

    #[test]
    fn delta_state_wire_roundtrip_beats_full_snapshot_size() {
        let baseline = snap_fighters(50);
        let mut full = snap_fighters(51);
        full.fighters[1].x_mm += 250;
        full.fighters[1].yaw_ddeg += 10;
        full.fighters[3].hp -= 120;
        let delta = encode_delta(&full, &baseline).expect("encode delta");
        assert_eq!(delta.changed_slots.len(), 2);
        let checkpoint = Checkpoint {
            tick: 51,
            hash: 0xABCD,
        };
        let bytes = encode_state_delta(checkpoint, &delta).expect("encode wire");
        // Wire version stays 1; the delta bit rides the count byte.
        assert_eq!(bytes[2], WIRE_VERSION);
        assert_eq!(bytes[3], KIND_STATE);
        assert_ne!(bytes[16] & STATE_DELTA_BIT, 0);
        assert_eq!(bytes.len(), crate::delta::delta_wire_len(&delta));
        let packet = decode_state(&bytes).expect("decode");
        assert_eq!(packet.checkpoint, checkpoint);
        assert!(packet.inputs.is_empty());
        assert_eq!(packet.delta.as_ref(), Some(&delta));
        // Two changed slots travel in far fewer bytes than four full slots.
        let full_bytes = encode_snapshot(&full).expect("encode snapshot");
        assert!(
            bytes.len() < full_bytes.len(),
            "delta {} must beat full {}",
            bytes.len(),
            full_bytes.len()
        );
        // And the delta still rebuilds the full snapshot exactly.
        assert_eq!(
            decode_delta(packet.delta.as_ref().expect("delta"), &baseline).expect("rebuild"),
            full
        );
    }

    #[test]
    fn snapshot_wire_roundtrip() {
        let full = snap_fighters(60);
        let bytes = encode_snapshot(&full).expect("encode");
        assert_eq!(bytes[2], WIRE_VERSION);
        assert_eq!(bytes[3], KIND_SNAPSHOT);
        assert_eq!(bytes.len(), full.full_wire_len());
        assert_eq!(decode_snapshot(&bytes).expect("decode"), full);
        // Full-only peers reject the new kind loudly, never misparse it.
        assert_eq!(
            decode_state_net1(&bytes),
            Err(NetError::BadKind(KIND_SNAPSHOT))
        );
        assert_eq!(decode_snapshot(&bytes[..8]), Err(NetError::Truncated));
    }

    #[test]
    fn version_interop_with_full_only_peers() {
        let config = test_config(8);
        let mut server = Server::new(config).expect("server");
        let (checkpoint, tape_delta) = server.tick(&[local_input(0)]).expect("tick");
        // Flag clear means full: new bytes decode on the old path identically.
        let full_bytes = encode_state(
            checkpoint,
            &redundancy_window(&tape_delta.tape, 0, REDUNDANCY),
        )
        .expect("encode");
        assert_eq!(full_bytes[2], WIRE_VERSION);
        assert_eq!(full_bytes[16] & STATE_DELTA_BIT, 0);
        let old = decode_state_net1(&full_bytes).expect("old decodes new full");
        let new = decode_state(&full_bytes).expect("new decodes new full");
        assert_eq!(old.checkpoint, new.checkpoint);
        assert_eq!(old.inputs, new.inputs);
        assert_eq!(new.delta, None);
        // Flag set means delta: the old path rejects loudly, the new path reads.
        let baseline = snap_fighters(checkpoint.tick);
        let mut full = baseline.clone();
        full.tick = checkpoint.tick;
        full.fighters[0].stamina -= 25;
        let delta = encode_delta(&full, &baseline).expect("delta");
        let delta_bytes = encode_state_delta(checkpoint, &delta).expect("encode delta");
        assert_eq!(delta_bytes[2], WIRE_VERSION);
        let count = delta_bytes[16] & !STATE_DELTA_BIT;
        assert_eq!(
            decode_state_net1(&delta_bytes),
            Err(NetError::TooManyInputs((count | STATE_DELTA_BIT) as usize))
        );
        let packet = decode_state(&delta_bytes).expect("new decodes delta");
        assert_eq!(packet.delta.as_ref(), Some(&delta));
    }

    #[test]
    fn delta_wire_rejects_malformed() {
        let baseline = snap_fighters(70);
        let mut full = snap_fighters(71);
        full.fighters[0].x_mm += 1;
        let delta = encode_delta(&full, &baseline).expect("delta");
        let checkpoint = Checkpoint { tick: 71, hash: 7 };
        // Delta bound to another tick never encodes.
        let mut wrong_tick = delta.clone();
        wrong_tick.tick = 72;
        assert_eq!(
            encode_state_delta(checkpoint, &wrong_tick),
            Err(NetError::BadState)
        );
        let bytes = encode_state_delta(checkpoint, &delta).expect("encode");
        assert_eq!(
            decode_state(&bytes[..bytes.len() - 1]),
            Err(NetError::Truncated)
        );
        // Unknown mask bit.
        let mut bad_mask = bytes.clone();
        let cursor = STATE_DELTA_HEADER_LEN + 1;
        bad_mask[cursor] |= 0x40;
        assert_eq!(decode_state(&bad_mask), Err(NetError::BadInput));
        // Slot out of range.
        let mut bad_slot = bytes.clone();
        bad_slot[STATE_DELTA_HEADER_LEN] = engine_input::MAX_PARTICIPANTS;
        assert_eq!(decode_state(&bad_slot), Err(NetError::BadInput));
        // Trailing garbage.
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(decode_state(&trailing), Err(NetError::Truncated));
    }

    fn slot_input(tick: u32, slot: u8, mx: i8, mz: i8, facing: i16, buttons: u8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(slot).expect("slot"),
            mx,
            mz,
            facing,
            buttons: Buttons(buttons),
        }
    }

    fn sixteen_fighter_tick(tick: u32) -> Vec<FighterInput> {
        (0u8..16u8)
            .map(|slot| slot_input(tick, slot, 100, 0, 0, 0))
            .collect()
    }

    #[test]
    fn input_compression_roundtrips_adversarial_vectors() {
        // Held wish: only the tick counter moves, the classic redundancy case.
        let held: Vec<FighterInput> = (0..8).map(|tick| slot_input(tick, 0, 0, 0, 0, 0)).collect();
        // Wild: every record flips wish, facing and buttons.
        let wild: Vec<FighterInput> = (0..8)
            .map(|tick| {
                let odd = tick % 2 == 1;
                slot_input(
                    tick,
                    0,
                    if odd { 100 } else { -100 },
                    if odd { -100 } else { 100 },
                    if odd { 9000 } else { -9000 },
                    if odd {
                        engine_input::fighter::ATTACK
                    } else {
                        0
                    },
                )
            })
            .collect();
        // Flip: two wishes take turns on one slot.
        let flip: Vec<FighterInput> = (0..8)
            .map(|tick| slot_input(tick, 1, if tick % 2 == 0 { 100 } else { -100 }, 0, 0, 0))
            .collect();
        for (name, tape) in [("held", held), ("wild", wild), ("flip", flip)] {
            let end = tape.last().map(|input| input.tick).expect("tape");
            let raw = encode_input(end, &tape).expect("raw encode");
            let packed = encode_input_compressed(end, &tape).expect("packed encode");
            assert_ne!(
                packed[8] & INPUT_COMPRESSED_BIT,
                0,
                "{name}: the flag must ride the count byte"
            );
            assert_eq!(packed[2], WIRE_VERSION);
            assert_eq!(packed[3], KIND_INPUT);
            let back = decode_input(&packed).expect("packed decode");
            assert_eq!(back.tick, end);
            assert_eq!(back.inputs, tape, "{name}: byte-exact roundtrip");
            let plain = decode_input(&raw).expect("raw decode");
            assert_eq!(plain.inputs, back.inputs, "{name}: both shapes agree");
            let again = encode_input_compressed(end, &tape).expect("repack");
            assert_eq!(again, packed, "{name}: same inputs give same bytes");
        }
    }

    #[test]
    fn compressed_16_fighter_saves_kbps_with_identical_hashes() {
        const TICK: u32 = 42;
        let tape = sixteen_fighter_tick(TICK);
        let raw = encode_input(TICK, &tape).expect("raw encode");
        let packed = encode_input_compressed(TICK, &tape).expect("packed encode");
        let saving_pct = (raw.len() - packed.len()) * 100 / raw.len();
        println!(
            "MP9 raw={} packed={} saving={}% at 16 fighters",
            raw.len(),
            packed.len(),
            saving_pct
        );
        assert!(
            packed.len() * 5 <= raw.len() * 4,
            "need >=20% saving: raw {} packed {}",
            raw.len(),
            packed.len()
        );
        // Identical hashes: both shapes decode to the same tape and the arena
        // authors the same checkpoints from it (same send rate, fewer bytes,
        // so kbps falls by the same share).
        let config = ArenaConfig {
            seed: 7,
            ticks: TICK + 1,
            fighters: 16,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        };
        let back_raw = decode_input(&raw).expect("raw decode").inputs;
        let back_packed = decode_input(&packed).expect("packed decode").inputs;
        assert_eq!(back_raw, back_packed);
        let run_raw = run(&config, &FighterTape(back_raw));
        let run_packed = run(&config, &FighterTape(back_packed));
        assert_eq!(
            run_raw, run_packed,
            "same tape must author the same checkpoints"
        );
        assert!(!run_raw.is_empty(), "the sim must actually run");
        println!(
            "MP9 hash={:016x} checkpoints={}",
            run_raw.last().map(|cp| cp.hash).unwrap_or(0),
            run_raw.len()
        );
    }

    #[test]
    fn compressed_wire_rejects_malformed_loudly() {
        let tape: Vec<FighterInput> = (0..8).map(|tick| slot_input(tick, 0, 0, 0, 0, 0)).collect();
        let packed = encode_input_compressed(7, &tape).expect("packed encode");
        // Cut payload: short RLE rejects.
        assert_eq!(
            decode_input(&packed[..packed.len() - 1]),
            Err(NetError::Truncated)
        );
        // Poisoned first tag claims 128 zeros on an 80-byte want: overlong.
        let mut wild_tag = packed.clone();
        wild_tag[9] = INPUT_COMPRESSED_BIT | 0x7F;
        assert_eq!(decode_input(&wild_tag), Err(NetError::Truncated));
        // Trailing garbage: the extra tag overruns the want.
        let mut trailing = packed.clone();
        trailing.push(0);
        assert_eq!(decode_input(&trailing), Err(NetError::Truncated));
        // Flag cleared: the packed body never parses as a raw window.
        let mut unflagged = packed.clone();
        unflagged[8] &= !INPUT_COMPRESSED_BIT;
        assert_eq!(decode_input(&unflagged), Err(NetError::Truncated));
        // Flag set with an empty body: nothing to expand.
        let mut header = packed.clone();
        header.truncate(9);
        assert_eq!(decode_input(&header), Err(NetError::Truncated));
        // Foreign version still rejected before any payload parsing.
        let mut future = packed.clone();
        future[2] = WIRE_VERSION + 1;
        assert_eq!(
            decode_input(&future),
            Err(NetError::VersionMismatch {
                got: WIRE_VERSION + 1,
            })
        );
        // Empty and oversize inputs never encode.
        assert_eq!(encode_input_compressed(7, &[]), Err(NetError::BadInput));
        let many: Vec<FighterInput> = (0..17)
            .map(|tick| slot_input(tick, 0, 0, 0, 0, 0))
            .collect();
        assert_eq!(
            encode_input_compressed(16, &many),
            Err(NetError::TooManyInputs(17))
        );
    }

    #[test]
    fn loopback_converges_with_compressed_inputs() {
        const TICKS: u32 = 8;
        let config = test_config(TICKS);
        let mut server = Server::new(config).expect("server");
        let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client");
        for tick in 0..TICKS {
            let input = local_input(tick);
            client.predict(input).expect("predict");
            // Client-to-server leg travels compressed; the server authors
            // from the decoded input.
            let wire = encode_input_compressed(tick, &[input]).expect("compress");
            let packet = decode_input(&wire).expect("decode");
            assert_eq!(packet.inputs, [input]);
            let (checkpoint, delta) = server.tick(&packet.inputs).expect("server tick");
            // Server-to-client leg is the unchanged full state; the client
            // converges onto it.
            let state = encode_state(
                checkpoint,
                &redundancy_window(&delta.tape, tick, REDUNDANCY),
            )
            .expect("state");
            let seen = decode_state(&state).expect("decode state");
            let mut tape = client.server_tape.clone();
            tape.extend_from_slice(&seen.inputs);
            tape.sort_by_key(|entry| (entry.tick, entry.fighter.0));
            tape.dedup_by_key(|entry| (entry.tick, entry.fighter.0));
            let full = SnapshotDelta {
                version: WIRE_VERSION,
                tick: seen.checkpoint.tick,
                hash: seen.checkpoint.hash,
                tape,
            };
            client.on_server_state(checkpoint, &full).expect("apply");
            client.resimulate();
        }
        client.resimulate();
        assert_eq!(client.server_tick(), Some(TICKS - 1));
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "the compressed wire must land the same hash on both ends"
        );
    }

    /// Two humans on slots 0 and 1 for `ticks` ticks, authored by a server.
    fn two_human_server(ticks: u32) -> Server {
        let config = ArenaConfig {
            fighters: 4,
            ..test_config(ticks)
        };
        let mut server = Server::new(config).expect("server");
        for tick in 0..ticks {
            let mut inputs = vec![local_input(tick)];
            if tick % 3 != 1 {
                let mut other = local_input(tick);
                other.fighter = FighterId::new(1).expect("slot 1");
                other.mx = -other.mx;
                inputs.push(other);
            }
            server.tick(&inputs).expect("server tick");
        }
        server
    }

    #[test]
    fn resync_packets_carry_whole_ticks_and_converge_a_cold_client() {
        const TICKS: u32 = 40;
        let server = two_human_server(TICKS);
        let tape: Vec<FighterInput> = {
            let mut all = Vec::new();
            for tick in 0..TICKS {
                all.push(local_input(tick));
                if tick % 3 != 1 {
                    let mut other = local_input(tick);
                    other.fighter = FighterId::new(1).expect("slot 1");
                    other.mx = -other.mx;
                    all.push(other);
                }
            }
            all
        };
        let tip = *server.checkpoints().last().expect("tip");
        let snapshot = SnapshotDelta {
            version: WIRE_VERSION,
            tick: tip.tick,
            hash: tip.hash,
            tape: tape.clone(),
        };
        let packets = resync_packets(&snapshot, server.checkpoints()).expect("packets");
        assert!(packets.len() > 1, "a long tape needs several packets");
        let mut seen: Vec<FighterInput> = Vec::new();
        let mut last_tick = None;
        let mut client =
            Client::new(server.config(), FighterId::new(0).expect("slot 0")).expect("client");
        for bytes in &packets {
            assert!(bytes.len() <= MAX_PACKET_BYTES);
            let packet = decode_state(bytes).expect("packet decodes");
            assert!(packet.inputs.len() <= MAX_INPUTS_PER_PACKET);
            // Whole ticks: no tick shows up in two packets.
            for input in &packet.inputs {
                assert!(last_tick.is_none_or(|last| input.tick >= last));
                assert!(input.tick <= packet.checkpoint.tick);
            }
            if let (Some(first), Some(prev)) = (packet.inputs.first(), seen.last()) {
                assert!(first.tick > prev.tick, "a tick never splits across packets");
            }
            last_tick = packet.inputs.last().map(|input| input.tick).or(last_tick);
            seen.extend_from_slice(&packet.inputs);
            assert_eq!(
                packet.checkpoint,
                server.checkpoints()[packet.checkpoint.tick as usize]
            );
        }
        assert_eq!(seen, tape, "the stream carries the whole tape once");
        assert_eq!(
            decode_state(packets.last().expect("last"))
                .expect("tip")
                .checkpoint,
            tip
        );
        // A cold client that never predicted anything catches up from the
        // stream alone and lands on the server's checkpoints.
        let mut known: Vec<FighterInput> = Vec::new();
        for bytes in &packets {
            let packet = decode_state(bytes).expect("decode");
            known.extend_from_slice(&packet.inputs);
            let delta = SnapshotDelta {
                version: WIRE_VERSION,
                tick: packet.checkpoint.tick,
                hash: packet.checkpoint.hash,
                tape: known.clone(),
            };
            client
                .on_server_state(packet.checkpoint, &delta)
                .expect("apply");
        }
        client.resimulate();
        assert_eq!(client.server_tick(), Some(TICKS - 1));
        assert_eq!(client.checkpoints(), server.checkpoints());
    }

    #[test]
    fn resync_packets_refuse_a_history_that_disagrees() {
        let server = two_human_server(12);
        let tip = *server.checkpoints().last().expect("tip");
        let snapshot = SnapshotDelta {
            version: WIRE_VERSION,
            tick: tip.tick,
            hash: tip.hash,
            tape: vec![local_input(0)],
        };
        // The tip hash must match the history the packets will cite.
        let mut forged = snapshot.clone();
        forged.hash ^= 1;
        assert_eq!(
            resync_packets(&forged, server.checkpoints()),
            Err(NetError::BadState)
        );
        // A history shorter than the snapshot cannot cite its tip.
        assert_eq!(
            resync_packets(&snapshot, &server.checkpoints()[..6]),
            Err(NetError::BadState)
        );
        // An empty tape still carries the tip so a client learns the tick.
        let bare = SnapshotDelta {
            tape: Vec::new(),
            ..snapshot
        };
        let packets = resync_packets(&bare, server.checkpoints()).expect("tip only");
        assert_eq!(packets.len(), 1);
        assert_eq!(decode_state(&packets[0]).expect("tip").checkpoint, tip);
    }
}
