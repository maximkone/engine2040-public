//! rollback: P2P rollback tape over the sim tick (2 players, 128 slots).
//!
//! Owner: `engine-net`; consumer: P2P sessions (`P2pSession`, also the
//! `crate::netcode::Session` predict/resim pattern). Units: frames (`u32`),
//! compact inputs (`u32`), state hashes (`u64`). Determinism: integer-only
//! tape (`+ - * /`, wrapping adds, FNV); stable frame order; no wall clock,
//! no float, no `HashMap` iteration in hashed state. Errors: `RollbackError`
//! (desync / too-far / unknown refuse loudly, state unchanged).
//!
//! Request discipline mirrors the GGRS-style host requests as an idea only:
//! `save_game_state` / `load_game_state` / `advance_frame`, reimplemented
//! here over `SimTick::advance`. Per-player input queues (`InputQueue`,
//! 128 slots each) mirror the GGRS input-queue school as an idea only:
//! ordered confirmed/predicted inputs per player plus a
//! `discard_confirmed_frames` tail-advance. Resim replays the window in
//! frame order; interest relevancy (`super::interest::Relevancy`) scopes what
//! the session layer replicates, never the tape order. Budget: snapshot
//! window holds at most `MAX_P2P_ROLLBACK + 1` states; per-player queues hold at
//! most `INPUT_QUEUE_LEN` inputs; the replay tape keeps at most [`TAPE_CAP`]
//! frames (hash plus input row) and drops the oldest past it, so its bytes
//! stay flat however long the session runs.
//! Input delay and first-incorrect resim (MP22, `HouraiTeahouse/backroll-rs`
//! ISC, ideas only): `P2pSession::with_frame_delay` applies each local input
//! `frame_delay` frames late so the peer's copy of it travels for free, and
//! `confirm_input` only marks the earliest wrong frame (`first_incorrect_frame`)
//! so `resolve_incorrect` resimulates once from there however many late
//! inputs arrived this tick.
//! License: MIT (workspace). PORT(clean-room: no vendor bytes copied;
//! GGRS-style save/load/advance request shape used as idea-only per
//! CODELAW 9; MIT OR Apache-2.0 compatible).
//! EVIDENCE: | crates/net/src/rollback.rs | clean-room P2P rollback tape,
//! no vendor bytes | MIT OR Apache-2.0 | ADOPT - pattern, ideas
//! reimplemented, no vendor |
//!
//! Link conditioner plus seat input buffer (lightyear `LinkConditionerConfig`
//! and `InputBuffer` ideas only, MIT OR Apache-2.0, reimplemented std-only,
//! no Bevy dependency, zero donor bytes): [`LinkConditionerConfig`] states
//! the one-way latency and jitter each way in ticks and derives the local
//! input delay that hides the server relay ([`LinkConditionerConfig::input_delay`]),
//! and [`InputBuffer`] holds every seat's authored input and applies it
//! `delay` ticks later. [`SeatSession`] runs an N-seat predicting client over
//! them: known inputs win, a missing seat repeats its last confirmed input,
//! and a late true input that contradicts a played frame rolls back once and
//! resimulates the window. The 4-seat headless proof drives a server and four
//! such clients through [`crate::link::LoopbackWire`] under the LAN
//! conditioner.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};

use engine_core::Fnv64;

use crate::desync::DesyncReport;

/// Maximum P2P rollback depth (predict repeat-last, resim on mispredict).
/// 128 slots: the GGRS input-queue school length, reimplemented std-only.
pub const MAX_P2P_ROLLBACK: usize = 128;
/// WAN authority window (9: 150 ms at 60 Hz, `RTT_TICKS_150MS` 9): the
/// netcode session window, one owner for both rollback layers. A 61-tick
/// WAN rollback exceeds this and must refuse (resync, not resim);
/// `MAX_P2P_ROLLBACK` 128 stays the P2P storage window only.
pub const WAN_ROLLBACK_CAP: usize = crate::netcode::MAX_ROLLBACK;
/// Per-player input queue length (GGRS school: 128 slots, std-only).
pub const INPUT_QUEUE_LEN: usize = 128;
/// Player count for the P2P tape (fixed 2-player duel).
pub const PLAYERS: usize = 2;
/// Production local input delay in frames (`session::INPUT_DELAY_TICKS`):
/// pass it to [`P2pSession::with_frame_delay`]; `P2pSession::new` stays at 0.
pub const DEFAULT_FRAME_DELAY: u32 = crate::session::INPUT_DELAY_TICKS;
/// Narrow Lunaris-v2 resim window (8 frames, GGRS default, pattern only).
pub const LUNARIS_WINDOW: usize = 8;
/// Frames kept by the replay tape (one hash plus one input row each). The
/// tape is a ring: the newest `TAPE_CAP` frames stay readable and older ones
/// drop off, so a long session's tape bytes stay flat. 512 covers the
/// 500-tick reference replay with headroom and is a power of two.
pub const TAPE_CAP: usize = MAX_P2P_ROLLBACK * 4;

/// A frame count as a window index.
fn index(frames: u32) -> usize {
    usize::try_from(frames).unwrap_or(usize::MAX)
}

/// One confirmed frame: state hash + compact input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// State hash for this frame.
    pub hash: u64,
    /// Compact input bits.
    pub input: u32,
}

/// Bounded frame log: push newest, evict oldest past cap.
pub struct FrameLog {
    frames: std::collections::VecDeque<Frame>,
    cap: usize,
}

impl FrameLog {
    /// Construct with capacity (clamped to `MAX_P2P_ROLLBACK`).
    pub fn new(cap: usize) -> Self {
        Self {
            frames: std::collections::VecDeque::new(),
            cap: cap.clamp(1, MAX_P2P_ROLLBACK),
        }
    }
    /// Save one frame (advance).
    pub fn save(&mut self, hash: u64, input: u32) {
        self.frames.push_back(Frame { hash, input });
        if self.frames.len() > self.cap {
            self.frames.pop_front();
        }
    }
    /// Load a saved frame for resimulation.
    pub fn load(&self, idx: usize) -> Option<Frame> {
        self.frames.get(idx).copied()
    }
    /// Fold all frames to one hash (FNV-1a over the hash+input stream,
    /// little-endian words in order, through the shared [`Fnv64`] owner).
    pub fn fold(&self) -> u64 {
        let mut fold = Fnv64::new();
        for f in &self.frames {
            fold.write_u64(f.hash);
            fold.write_u64(u64::from(f.input));
        }
        fold.finish()
    }
    /// Frame count.
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// One queued input: frame number plus confirmed flag.
///
/// `confirmed == true` means the true remote/local input for `frame`;
/// `false` means a repeat-last prediction still awaiting confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QueueSlot {
    /// Authored frame number.
    frame: u32,
    /// Compact input bits.
    input: u32,
    /// True once the authoritative input arrived.
    confirmed: bool,
}

/// Per-player input queue: ordered frame inputs, at most 128 slots.
///
/// GGRS input-queue school reimplemented std-only (idea only, no vendor
/// bytes): sequential `push_input` per authored frame, out-of-order late
/// `confirm` updates, and a confirmed-prefix `discard_confirmed_frames`
/// tail-advance. Order-preserving: slots stay sorted by frame, discards pop
/// the front only. Determinism: integer frame order only, no wall clock.
#[derive(Clone, Debug)]
pub struct InputQueue {
    slots: VecDeque<QueueSlot>,
}

impl InputQueue {
    /// Empty queue with capacity for `INPUT_QUEUE_LEN` slots (no alloc yet).
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: VecDeque::new(),
        }
    }

    /// Queued input count (at most `INPUT_QUEUE_LEN`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True when no input is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Oldest queued frame, or `None` when empty.
    #[must_use]
    pub fn first_frame(&self) -> Option<u32> {
        self.slots.front().map(|s| s.frame)
    }

    /// Newest queued frame, or `None` when empty.
    #[must_use]
    pub fn last_frame(&self) -> Option<u32> {
        self.slots.back().map(|s| s.frame)
    }

    /// Input for `frame`: `(input, confirmed)`, or `None` when absent.
    #[must_use]
    pub fn get(&self, frame: u32) -> Option<(u32, bool)> {
        self.slots
            .iter()
            .find(|s| s.frame == frame)
            .map(|s| (s.input, s.confirmed))
    }

    /// True when `frame` is queued and confirmed.
    #[must_use]
    pub fn is_confirmed(&self, frame: u32) -> bool {
        self.get(frame).is_some_and(|(_, c)| c)
    }

    /// Push the authored input for `frame` (sequential steady state).
    /// Out-of-order or duplicate frames insert sorted without duplication;
    /// past-cap pushes evict the oldest slot (order-preserving).
    pub fn push_input(&mut self, frame: u32, input: u32, confirmed: bool) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.frame == frame) {
            slot.input = input;
            slot.confirmed = slot.confirmed || confirmed;
            return;
        }
        let at = self.slots.iter().position(|s| s.frame > frame);
        let slot = QueueSlot {
            frame,
            input,
            confirmed,
        };
        match at {
            Some(i) => self.slots.insert(i, slot),
            None => self.slots.push_back(slot),
        }
        while self.slots.len() > INPUT_QUEUE_LEN {
            self.slots.pop_front();
        }
    }

    /// Mark `frame` confirmed with the true `input`.
    /// Returns `true` when the stored input changed (a mispredict).
    pub fn confirm(&mut self, frame: u32, input: u32) -> bool {
        match self.slots.iter_mut().find(|s| s.frame == frame) {
            None => {
                self.push_input(frame, input, true);
                true
            }
            Some(slot) => {
                let changed = slot.input != input;
                slot.input = input;
                slot.confirmed = true;
                changed
            }
        }
    }

    /// Discard the confirmed prefix below `up_to` (exclusive): pops leading
    /// confirmed slots with `frame < up_to`, stops at the first predicted
    /// or absent frame. Returns the discard count. Order-preserving.
    pub fn discard_confirmed_frames(&mut self, up_to: u32) -> u32 {
        let mut n = 0u32;
        while self
            .slots
            .front()
            .is_some_and(|s| s.frame < up_to && s.confirmed)
        {
            self.slots.pop_front();
            n += 1;
        }
        n
    }
}

impl Default for InputQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// One-way link conditioning in ticks, each way.
///
/// The lightyear `LinkConditionerConfig` shape (incoming and outgoing latency
/// plus jitter), reimplemented std-only over integer ticks: no sleep, no wall
/// clock, no Bevy. [`Self::incoming`] and [`Self::outgoing`] are the profiles
/// a [`crate::link::FaultLink`] draws from, and [`Self::input_delay`] is the
/// local input delay that hides the full client-to-server-to-client relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkConditionerConfig {
    /// One-way latency server-to-client, in ticks.
    pub incoming_latency: u32,
    /// Extra server-to-client latency drawn from `0..=incoming_jitter`, in ticks.
    pub incoming_jitter: u32,
    /// One-way latency client-to-server, in ticks.
    pub outgoing_latency: u32,
    /// Extra client-to-server latency drawn from `0..=outgoing_jitter`, in ticks.
    pub outgoing_jitter: u32,
}

impl LinkConditionerConfig {
    /// State every direction in ticks.
    #[must_use]
    pub const fn new(
        incoming_latency: u32,
        incoming_jitter: u32,
        outgoing_latency: u32,
        outgoing_jitter: u32,
    ) -> Self {
        Self {
            incoming_latency,
            incoming_jitter,
            outgoing_latency,
            outgoing_jitter,
        }
    }

    /// No delay either way (loopback in one tick).
    #[must_use]
    pub const fn clean() -> Self {
        Self::new(0, 0, 0, 0)
    }

    /// LAN: one tick each way, no jitter. A two-frame input delay covers the
    /// full relay, so a client predicts almost nothing.
    #[must_use]
    pub const fn lan() -> Self {
        Self::new(1, 0, 1, 0)
    }

    /// The server-to-client loss profile this conditioner implies (no loss).
    #[must_use]
    pub const fn incoming(&self) -> crate::link::LinkProfile {
        crate::link::LinkProfile {
            latency: self.incoming_latency,
            jitter: self.incoming_jitter,
            loss_pct: 0,
            reorder_pct: 0,
        }
    }

    /// The client-to-server loss profile this conditioner implies (no loss).
    #[must_use]
    pub const fn outgoing(&self) -> crate::link::LinkProfile {
        crate::link::LinkProfile {
            latency: self.outgoing_latency,
            jitter: self.outgoing_jitter,
            loss_pct: 0,
            reorder_pct: 0,
        }
    }

    /// Slowest full relay the conditioner can draw, in ticks: out plus back.
    #[must_use]
    pub const fn worst_relay(&self) -> u32 {
        self.outgoing_latency
            .saturating_add(self.outgoing_jitter)
            .saturating_add(self.incoming_latency)
            .saturating_add(self.incoming_jitter)
    }

    /// Local input delay that hides the full relay, clamped to the input-delay
    /// knob ([`crate::delay::MAX_DELAY`]). A client that samples `delay` ticks
    /// before it plays a tick receives every peer input just in time.
    #[must_use]
    pub fn input_delay(&self) -> u32 {
        crate::delay::clamp_delay(self.worst_relay())
    }
}

/// Seat input buffer: every seat's authored input per tick, bounded to
/// [`INPUT_QUEUE_LEN`] ticks (lightyear `InputBuffer` shape, ideas only).
///
/// The local seat presses with [`Self::press`], which files the input
/// `delay` ticks ahead of the tick being played (frame delay); peers file
/// their authored tick with [`Self::record`]. [`Self::resolve`] answers the
/// inputs a played tick uses: a held input wins, a missing seat repeats its
/// last confirmed input. Order-preserving: rows stay keyed by tick in a
/// `BTreeMap`, the prune only drops the oldest. Determinism: integer ticks
/// only, no wall clock.
#[derive(Clone, Debug)]
pub struct InputBuffer<const N: usize> {
    delay: u32,
    rows: BTreeMap<u32, [Option<u32>; N]>,
    last: [u32; N],
    newest: u32,
}

impl<const N: usize> InputBuffer<N> {
    /// Empty buffer whose local presses play `delay` ticks later (clamped to
    /// [`crate::delay::MAX_DELAY`]).
    #[must_use]
    pub fn new(delay: u32) -> Self {
        Self {
            delay: crate::delay::clamp_delay(delay),
            rows: BTreeMap::new(),
            last: [0; N],
            newest: 0,
        }
    }

    /// Local input delay in ticks.
    #[must_use]
    pub fn delay(&self) -> u32 {
        self.delay
    }

    /// File one authored input for `tick`. Returns `true` when the held value
    /// changed (a new row, or a different input); an evicted tick answers
    /// `false` without writing.
    pub fn record(&mut self, tick: u32, seat: usize, input: u32) -> bool {
        if seat >= N {
            return false;
        }
        let cap = u32::try_from(INPUT_QUEUE_LEN).unwrap_or(u32::MAX);
        if tick.saturating_add(cap) < self.newest {
            return false;
        }
        let row = self.rows.entry(tick).or_insert([None; N]);
        let changed = row[seat] != Some(input);
        row[seat] = Some(input);
        self.newest = self.newest.max(tick);
        while let Some((&oldest, _)) = self.rows.first_key_value() {
            if self.newest.saturating_sub(oldest) < cap {
                break;
            }
            self.rows.pop_first();
        }
        changed
    }

    /// File the local press for the tick being played: it plays `delay` ticks
    /// later. Returns [`Self::record`]'s answer for that future tick.
    pub fn press(&mut self, play_tick: u32, seat: usize, input: u32) -> bool {
        self.record(play_tick.saturating_add(self.delay), seat, input)
    }

    /// Remember `inputs` as the last played row (the repeat-last source).
    pub fn note_played(&mut self, inputs: [u32; N]) {
        self.last = inputs;
    }

    /// Held input for `tick` and `seat`, or `None` when absent/evicted.
    #[must_use]
    pub fn get(&self, tick: u32, seat: usize) -> Option<u32> {
        self.rows
            .get(&tick)
            .and_then(|row| row.get(seat).copied().flatten())
    }

    /// Last played input for `seat` (the repeat-last prediction).
    #[must_use]
    pub fn last(&self, seat: usize) -> u32 {
        self.last.get(seat).copied().unwrap_or(0)
    }

    /// True when every seat holds an authored input for `tick`.
    #[must_use]
    pub fn complete(&self, tick: u32) -> bool {
        self.rows
            .get(&tick)
            .is_some_and(|row| row.iter().all(Option::is_some))
    }

    /// Inputs a played `tick` runs: a held input wins, a missing seat repeats
    /// its last confirmed input.
    #[must_use]
    pub fn resolve(&self, tick: u32) -> [u32; N] {
        std::array::from_fn(|seat| self.get(tick, seat).unwrap_or_else(|| self.last(seat)))
    }

    /// Drop every row below `tick`; returns the dropped row count.
    pub fn discard_before(&mut self, tick: u32) -> u32 {
        let mut dropped = 0u32;
        while let Some((&oldest, _)) = self.rows.first_key_value() {
            if oldest >= tick {
                break;
            }
            self.rows.pop_first();
            dropped += 1;
        }
        dropped
    }

    /// Held rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no row is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// N-seat deterministic sim behind [`SeatSession`].
///
/// The N-seat sibling of [`SimTick`] (which is the fixed two-player
/// contract): `+ - * /`, wrapping integer math and a state hash; no float
/// transcendental calls, no wall clock.
pub trait SeatSim<const N: usize>: Clone {
    /// Advance one fixed tick with every seat's input.
    fn step(&mut self, inputs: [u32; N]);
    /// Canonical state hash after the latest tick.
    fn hash(&self) -> u64;
}

/// One played tick of a [`SeatSession`].
struct SeatFrame<const N: usize, S> {
    tick: u32,
    before: S,
    inputs: [u32; N],
}

/// N-seat predicting client: local frame delay plus repeat-last prediction,
/// with one rollback and resim per mispredicted tick.
///
/// Consumer: the 4-seat headless proof below; a seat client of the dedicated
/// server builds the same way over [`crate::link::LoopbackWire`]. The local
/// input is pressed `delay` ticks before it plays ([`InputBuffer::press`]);
/// peer inputs arrive as authored ticks ([`InputBuffer::record`]). A late true
/// input that contradicts a played frame restores that frame's checkpoint and
/// replays the window in tick order, counted as one rollback event.
pub struct SeatSession<const N: usize, S: SeatSim<N>> {
    sim: S,
    tick: u32,
    buffer: InputBuffer<N>,
    window: VecDeque<SeatFrame<N, S>>,
    history: Vec<u64>,
    local: usize,
    rollbacks: u64,
    predicted: u64,
    mispredicted: u64,
    max_rollback: usize,
}

impl<const N: usize, S: SeatSim<N>> SeatSession<N, S> {
    /// Open a session at tick zero for `local`, with `delay` local frames.
    pub fn new(initial: S, local: usize, delay: u32) -> Self {
        Self {
            sim: initial,
            tick: 0,
            buffer: InputBuffer::new(delay),
            window: VecDeque::new(),
            history: Vec::new(),
            local,
            rollbacks: 0,
            predicted: 0,
            mispredicted: 0,
            max_rollback: 0,
        }
    }

    /// Next tick to play.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Local input delay in ticks.
    pub fn delay(&self) -> u32 {
        self.buffer.delay()
    }

    /// Live sim state after the latest played tick.
    pub fn sim(&self) -> &S {
        &self.sim
    }

    /// Rollback events so far (one per resimulated mispredict).
    pub fn rollbacks(&self) -> u64 {
        self.rollbacks
    }

    /// Frames played with at least one seat still predicted.
    pub fn predicted_count(&self) -> u64 {
        self.predicted
    }

    /// Late true inputs that contradicted their prediction.
    pub fn mispredicted_count(&self) -> u64 {
        self.mispredicted
    }

    /// Deepest rollback so far, in ticks.
    pub fn max_rollback(&self) -> usize {
        self.max_rollback
    }

    /// Tape hash at `tick`, or `None` before/after the played range.
    pub fn hash_at(&self, tick: u32) -> Option<u64> {
        usize::try_from(tick)
            .ok()
            .and_then(|i| self.history.get(i).copied())
    }

    /// Play the next tick with this tick's local press. The press plays
    /// `delay` ticks later; every other seat uses its held input or repeats
    /// its last confirmed one. Returns the post-tick hash.
    pub fn advance(&mut self, sample: u32) -> u64 {
        let tick = self.tick;
        self.buffer.press(tick, self.local, sample);
        let complete = self.buffer.complete(tick);
        let inputs = self.buffer.resolve(tick);
        if !complete {
            self.predicted += 1;
        }
        self.buffer.note_played(inputs);
        let before = self.sim.clone();
        self.sim.step(inputs);
        let hash = self.sim.hash();
        self.window.push_back(SeatFrame {
            tick,
            before,
            inputs,
        });
        self.history.push(hash);
        if self.window.len() > MAX_P2P_ROLLBACK + 1 {
            self.window.pop_front();
        }
        self.tick += 1;
        hash
    }

    /// File one authored peer input for `tick`. A future tick buffers
    /// (`Ok(0)`); an already-played tick that disagrees with its prediction
    /// rolls back and resimulates the window (`Ok(distance)`); an evicted
    /// tick refuses with [`RollbackError::TooFar`]; an unknown seat refuses
    /// with [`RollbackError::NoSeat`]. Any refusal changes nothing.
    pub fn confirm(&mut self, tick: u32, seat: usize, input: u32) -> Result<usize, RollbackError> {
        if seat >= N {
            return Err(RollbackError::NoSeat { seat });
        }
        if tick >= self.tick {
            self.buffer.record(tick, seat, input);
            return Ok(0);
        }
        let oldest = self.window.front().map_or(self.tick, |frame| frame.tick);
        let Some(since) = tick.checked_sub(oldest) else {
            return Err(RollbackError::TooFar {
                frame: tick,
                oldest,
            });
        };
        let Some(index) = usize::try_from(since).ok() else {
            return Err(RollbackError::TooFar {
                frame: tick,
                oldest,
            });
        };
        let Some(frame) = self.window.get(index) else {
            return Err(RollbackError::TooFar {
                frame: tick,
                oldest,
            });
        };
        let corrected = frame.inputs;
        self.buffer.record(tick, seat, input);
        if corrected[seat] == input {
            return Ok(0);
        }
        self.mispredicted += 1;
        self.resimulate(index)?;
        Ok(usize::try_from(self.tick - tick).unwrap_or(usize::MAX))
    }

    /// Restore the checkpoint at window `index` and replay to the tip.
    fn resimulate(&mut self, index: usize) -> Result<(), RollbackError> {
        let oldest = self.window.front().map_or(self.tick, |frame| frame.tick);
        let Some(start) = self.window.get(index) else {
            return Err(RollbackError::TooFar {
                frame: oldest,
                oldest,
            });
        };
        let checkpoint = start.before.clone();
        self.sim = checkpoint;
        let len = self.window.len();
        let mut inputs = [0u32; N];
        for at in index..len {
            let Some(frame) = self.window.get(at) else {
                break;
            };
            let tick = frame.tick;
            let played = frame.inputs;
            for (seat, slot) in inputs.iter_mut().enumerate() {
                *slot = self.buffer.get(tick, seat).unwrap_or(played[seat]);
            }
            self.sim.step(inputs);
            let hash = self.sim.hash();
            if let Some(frame) = self.window.get_mut(at) {
                frame.inputs = inputs;
            }
            if let Some(slot) = usize::try_from(tick)
                .ok()
                .and_then(|i| self.history.get_mut(i))
            {
                *slot = hash;
            }
            if at + 1 < len {
                let next = self.sim.clone();
                if let Some(frame) = self.window.get_mut(at + 1) {
                    frame.before = next;
                }
            }
        }
        self.rollbacks += 1;
        self.max_rollback = self.max_rollback.max(len - index);
        Ok(())
    }
}

/// Loud rollback failures: desync / window / history refusals change nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackError {
    /// Confirm targets an evicted frame or a resim deeper than the window.
    TooFar {
        /// Requested frame.
        frame: u32,
        /// Oldest frame still held in the window.
        oldest: u32,
    },
    /// Confirm targets a frame past the near-future buffer horizon.
    FutureFrame {
        /// Requested frame.
        frame: u32,
        /// Next frame to author.
        current: u32,
    },
    /// Desync / history probe names an authoritatively unknown frame.
    UnknownFrame(u32),
    /// Peer hash disagrees with the local tape at `frame`.
    Desync {
        /// Divergent frame.
        frame: u32,
        /// Local tape hash.
        local: u64,
        /// Peer-claimed hash.
        peer: u64,
        /// What this session held at that tick: per-side digests plus the
        /// one-bit [`DesyncReport::inputs_agree`] that says whether the
        /// wire is exonerated or the sim is named.
        report: DesyncReport,
    },
    /// Seat token mismatch on a guarded confirm; state unchanged.
    BadSeat { frame: u32, want: u64, got: u64 },
    /// Confirm names a seat outside the session's seat count; state unchanged.
    NoSeat {
        /// Requested seat index.
        seat: usize,
    },
}

impl std::fmt::Display for RollbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::TooFar { frame, oldest } => {
                write!(f, "rollback too far: frame {frame}, oldest {oldest}")
            }
            Self::FutureFrame { frame, current } => {
                write!(f, "future frame: frame {frame}, current {current}")
            }
            Self::UnknownFrame(frame) => write!(f, "unknown frame {frame}"),
            Self::Desync {
                frame,
                local,
                peer,
                report,
            } => write!(
                f,
                "desync at frame {frame}: local {local:#016x} != peer {peer:#016x}; {report}"
            ),
            Self::BadSeat { frame, want, got } => {
                write!(f, "bad seat {frame}: want {want:#x} got {got:#x}")
            }
            Self::NoSeat { seat } => write!(f, "no seat {seat}"),
        }
    }
}

impl std::error::Error for RollbackError {}

/// Deterministic 2-player sim tick behind both rollback sessions.
///
/// One contract for [`P2pSession`] and [`crate::netcode::Session`] (which
/// re-exports this trait as `netcode::Sim`). Integer-only: implementations
/// use `+ - * /`, wrapping integer math and `state_hash`; no float
/// transcendental calls, no wall clock.
pub trait SimTick: Clone {
    /// Advance one fixed tick with both players' inputs.
    fn advance(&mut self, inputs: [u32; PLAYERS]);
    /// Canonical state hash after the latest tick.
    fn state_hash(&self) -> u64;
}

struct FrameRecord<S> {
    frame: u32,
    before: S,
    inputs: [u32; PLAYERS],
    predicted: bool,
    hash: u64,
}

/// 2-player rollback session over [`SimTick`].
///
/// GGRS-style discipline, reimplemented: `save_game_state` snapshots the live
/// sim, `load_game_state` restores it, `advance_frame` authors one tick
/// (missing remote repeats the last confirmed remote input), and
/// `confirm_frame` applies the late true remote input with rollback plus
/// resimulation on mispredict. `check_desync` compares one peer hash against
/// the tape and refuses loudly on mismatch, carrying a
/// [`crate::desync::DesyncReport`] that names the guilty side.
pub struct P2pSession<S: SimTick> {
    sim: S,
    frame: u32,
    window: VecDeque<FrameRecord<S>>,
    history: VecDeque<u64>,
    tape_base: u32,
    pending: BTreeMap<u32, u32>,
    queues: [InputQueue; PLAYERS],
    last_remote: u32,
    rollbacks: u64,
    predicted: u64,
    mispredicted: u64,
    max_rollback: usize,
    last_mismatch: Cell<Option<u32>>,
    last_agreed: Cell<Option<u32>>,
    frontier: u32,
    settled: u32,
    log: VecDeque<[u32; PLAYERS]>,
    seat: u64,
    sparse: bool,
    confirmed: u32,
    anchor_frame: u32,
    anchor: Option<S>,
    frame_delay: u32,
    delayed: VecDeque<u32>,
    first_incorrect: Option<u32>,
}

impl<S: SimTick> P2pSession<S> {
    /// Open a session at frame zero with repeat-last seed `0`.
    pub fn new(initial: S) -> Self {
        Self {
            sim: initial,
            frame: 0,
            window: VecDeque::new(),
            history: VecDeque::new(),
            tape_base: 0,
            pending: BTreeMap::new(),
            queues: [InputQueue::new(), InputQueue::new()],
            last_remote: 0,
            rollbacks: 0,
            predicted: 0,
            mispredicted: 0,
            max_rollback: 0,
            last_mismatch: Cell::new(None),
            last_agreed: Cell::new(None),
            frontier: 0,
            settled: 0,
            log: VecDeque::new(),
            seat: 0,
            sparse: false,
            confirmed: 0,
            anchor_frame: 0,
            anchor: None,
            frame_delay: 0,
            delayed: VecDeque::new(),
            first_incorrect: None,
        }
    }

    /// Open a session whose local input takes effect `delay` frames after it
    /// is sampled (clamped to `delay::MAX_DELAY`); the first `delay` frames
    /// run on neutral local input `0`. Use [`DEFAULT_FRAME_DELAY`] for the
    /// production setting.
    pub fn with_frame_delay(initial: S, delay: u32) -> Self {
        let delay = crate::delay::clamp_delay(delay);
        let mut session = Self::new(initial);
        session.frame_delay = delay;
        session.delayed = std::iter::repeat_n(0, index(delay)).collect();
        session
    }

    /// Local input delay in frames (0 for [`Self::new`]).
    pub fn frame_delay(&self) -> u32 {
        self.frame_delay
    }

    /// Earliest frame whose remote input was confirmed different from its
    /// prediction and not yet resimulated (`first_incorrect_frame`).
    pub fn first_incorrect_frame(&self) -> Option<u32> {
        self.first_incorrect
    }

    /// Enable sparse saving (GGRS sparse mode, idea only): the anchor holds
    /// the last confirmed state, resim replays from there. Caller:
    /// `crates/net/src/delay.rs` reaction probe.
    pub fn enable_sparse_saving(&mut self, on: bool) {
        self.sparse = on;
        if on {
            self.anchor_frame = self.confirmed;
            self.anchor = Some(self.sim.clone());
            if let Some(state) = self.snapshot_before(self.confirmed) {
                self.anchor = Some(state);
            }
        }
    }

    /// SyncTest-style full-window check: refuses with `TooFar` when sparse
    /// is on (GGRS incompatibility, idea only). Caller:
    /// `crates/net/src/delay.rs` reaction probe.
    pub fn verify_full_window(&self, check_distance: usize) -> Result<(), RollbackError> {
        if self.sparse {
            return Err(RollbackError::TooFar {
                frame: check_distance as u32,
                oldest: self.frame,
            });
        }
        let have = self.window.len();
        if check_distance > have {
            return Err(RollbackError::TooFar {
                frame: check_distance as u32,
                oldest: self.oldest_frame(),
            });
        }
        Ok(())
    }

    /// Next frame to author.
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// Next frame to author (alias for frame counters).
    pub fn frames_played(&self) -> u32 {
        self.frame
    }

    /// Live sim state (post-tick of the last authored frame).
    pub fn sim(&self) -> &S {
        &self.sim
    }

    /// Snapshot the live sim (`SaveGameState` request).
    pub fn save_game_state(&self) -> S {
        self.sim.clone()
    }

    /// Restore the live sim (`LoadGameState` request). Window and hash
    /// history are untouched; authoritative correction flows through
    /// [`Self::confirm_frame`], never a blind load.
    pub fn load_game_state(&mut self, state: S) {
        self.sim = state;
    }

    /// Window length (at most `MAX_P2P_ROLLBACK + 1`).
    pub fn len(&self) -> usize {
        self.window.len()
    }

    /// True when no frame has been authored yet.
    pub fn is_empty(&self) -> bool {
        self.window.is_empty()
    }

    /// Hash tape over the held frames, oldest first (at most [`TAPE_CAP`]).
    pub fn stream(&self) -> Vec<u64> {
        self.history.iter().copied().collect()
    }

    /// Tape index of `frame` while the ring still holds it.
    fn tape_index(&self, frame: u32) -> Option<usize> {
        let at = frame.checked_sub(self.tape_base)?;
        usize::try_from(at).ok().filter(|&i| i < self.history.len())
    }

    /// Append one authored frame to the ring tape, dropping the oldest frame
    /// past [`TAPE_CAP`] so the tape never grows without bound.
    fn push_tape(&mut self, hash: u64, inputs: [u32; PLAYERS]) {
        if self.history.len() >= TAPE_CAP {
            self.history.pop_front();
            self.log.pop_front();
            self.tape_base = self.tape_base.wrapping_add(1);
        }
        self.history.push_back(hash);
        self.log.push_back(inputs);
    }

    /// Tape hash at `frame`, or `None` once the frame dropped off the tape.
    pub fn hash_at(&self, frame: u32) -> Option<u64> {
        self.tape_index(frame)
            .and_then(|i| self.history.get(i).copied())
    }

    /// GGRS-style per-tick checksum: state hash saved at `frame`.
    pub fn checksum_at(&self, frame: u32) -> Option<u64> {
        self.hash_at(frame)
    }

    /// GGRS-style checksum log: one `(frame, state hash)` per held tape frame.
    pub fn checksum_log(&self) -> Vec<(u32, u64)> {
        self.history
            .iter()
            .enumerate()
            .map(|(i, &hash)| (self.tape_base.wrapping_add(i as u32), hash))
            .collect()
    }

    /// Last frame where a peer hash mismatched, if any.
    pub fn last_mismatch(&self) -> Option<u32> {
        self.last_mismatch.get()
    }

    /// Snapshot held at the start of `frame`, or `None` once evicted.
    pub fn snapshot_before(&self, frame: u32) -> Option<S> {
        self.window
            .iter()
            .find(|r| r.frame == frame)
            .map(|r| r.before.clone())
    }

    /// Whether `frame` was authored with a predicted remote input.
    pub fn was_predicted(&self, frame: u32) -> Option<bool> {
        self.window
            .iter()
            .find(|r| r.frame == frame)
            .map(|r| r.predicted)
    }

    /// Per-player input queue (index `0` local, `1` remote), or `None`
    /// for an out-of-range player.
    pub fn queue(&self, player: usize) -> Option<&InputQueue> {
        self.queues.get(player)
    }

    /// Queued input count for `player` (at most `INPUT_QUEUE_LEN`).
    pub fn queue_len(&self, player: usize) -> Option<usize> {
        self.queues.get(player).map(InputQueue::len)
    }

    /// Oldest frame still held in the window (`frame` when empty).
    pub fn oldest_frame(&self) -> u32 {
        self.window.front().map_or(self.frame, |r| r.frame)
    }

    /// Tip cursor (next frame to author, mirrors `frame`).
    pub fn frontier(&self) -> u32 {
        self.frontier
    }
    /// Settled cursor (confirmed prefix, advances on discard).
    pub fn settled(&self) -> u32 {
        self.settled
    }

    /// Recorded input row at `frame`, or `None` once it dropped off the tape.
    pub fn input_at(&self, frame: u32) -> Option<[u32; PLAYERS]> {
        self.tape_index(frame)
            .and_then(|i| self.log.get(i).copied())
    }

    /// Spectator read path: tape hash at `frame` without mutating.
    pub fn spectator_hash_at(&self, frame: u32) -> Option<&u64> {
        self.tape_index(frame).and_then(|i| self.history.get(i))
    }
    /// Narrow to the Lunaris-v2 8-frame window and bind `seat`.
    pub fn enable_lunaris_v2(&mut self, seat: u64) {
        self.seat = seat;
    }
    /// Lunaris-v2 confirm: seat guard plus 8-frame window, then resim.
    pub fn confirm_frame_lunaris(
        &mut self,
        frame: u32,
        remote: u32,
        token: u64,
    ) -> Result<usize, RollbackError> {
        if self.seat != 0 && token != self.seat {
            return Err(RollbackError::BadSeat {
                frame,
                want: self.seat,
                got: token,
            });
        }
        if self.seat != 0 {
            let gap = self.frontier().saturating_sub(frame);
            let cap = u32::try_from(LUNARIS_WINDOW).unwrap_or(u32::MAX);
            if gap > cap {
                let oldest = self.frontier().saturating_sub(cap);
                return Err(RollbackError::TooFar { frame, oldest });
            }
        }
        self.confirm_frame(frame, remote)
    }

    /// Rollbacks performed so far.
    pub fn rollback_count(&self) -> u64 {
        self.rollbacks
    }

    /// Frames authored with a predicted remote input.
    pub fn predicted_count(&self) -> u64 {
        self.predicted
    }

    /// Predicted remote inputs the true input later contradicted.
    pub fn mispredicted_count(&self) -> u64 {
        self.mispredicted
    }

    /// Deepest rollback distance so far (frames).
    pub fn max_rollback(&self) -> usize {
        self.max_rollback
    }

    /// Author one frame (`AdvanceFrame` request). A buffered late input for
    /// this frame wins; otherwise the explicit `remote` wins; otherwise the
    /// last confirmed remote input repeats (prediction). Returns the post
    /// hash. Steady state is one window push plus one amortized hash push.
    /// Both per-player queues record the authored inputs (remote flagged
    /// predicted when repeated); past-cap slots evict the oldest
    /// order-preserving. With a [`Self::with_frame_delay`] session `local` is
    /// the input sampled this tick and the frame runs the one sampled
    /// `frame_delay` ticks ago.
    pub fn advance_frame(&mut self, local: u32, remote: Option<u32>) -> u64 {
        let f = self.frame;
        let local = if self.frame_delay == 0 {
            local
        } else {
            self.delayed.push_back(local);
            self.delayed.pop_front().unwrap_or(0)
        };
        let (r, predicted) = match self.pending.remove(&f).or(remote) {
            Some(v) => {
                self.last_remote = v;
                (v, false)
            }
            None => (self.last_remote, true),
        };
        if predicted {
            self.predicted += 1;
        }
        let before = self.sim.clone();
        self.sim.advance([local, r]);
        let hash = self.sim.state_hash();
        self.window.push_back(FrameRecord {
            frame: f,
            before,
            inputs: [local, r],
            predicted,
            hash,
        });
        self.push_tape(hash, [local, r]);
        self.queues[0].push_input(f, local, true);
        self.queues[1].push_input(f, r, !predicted);
        if self.window.len() > MAX_P2P_ROLLBACK + 1 {
            self.window.pop_front();
        }
        self.frame += 1;
        self.frontier = self.frame;
        hash
    }

    /// Apply the late true remote input for `frame`.
    ///
    /// Near-future frames buffer (`Ok(0)`); far-future frames refuse with
    /// [`RollbackError::FutureFrame`]. Authored frames inside the window
    /// confirm (`Ok(0)`) or roll back plus resimulate on mispredict
    /// (`Ok(distance)`). Evicted frames refuse with
    /// [`RollbackError::TooFar`]. Any error changes nothing.
    ///
    /// Cheap per-tick checkpoint (the Lunaris `Sim::checkpoint` idea only,
    /// no vendor bytes): [`Self::advance_frame`] clones the pre-tick sim
    /// into the window (`save_game_state` shape), so a mispredict restores
    /// the `before` checkpoint for `frame` and replays the window in frame
    /// order instead of rebuilding from tick zero. Handles stay small
    /// (frame plus the capped hash tape); full state lives only in the
    /// window owner.
    pub fn confirm_frame(&mut self, frame: u32, remote: u32) -> Result<usize, RollbackError> {
        self.confirm_input(frame, remote)?;
        self.resolve_incorrect()
    }

    /// Record the late true remote input for `frame` without resimulating:
    /// same buffering and refusals as [`Self::confirm_frame`], but a wrong
    /// prediction only lowers [`Self::first_incorrect_frame`]. Call
    /// [`Self::resolve_incorrect`] once after a batch of arrivals.
    pub fn confirm_input(&mut self, frame: u32, remote: u32) -> Result<(), RollbackError> {
        if frame >= self.frame {
            if u32::try_from(MAX_P2P_ROLLBACK).is_ok_and(|cap| frame - self.frame > cap) {
                return Err(RollbackError::FutureFrame {
                    frame,
                    current: self.frame,
                });
            }
            self.pending.insert(frame, remote);
            return Ok(());
        }
        let oldest = self.window.front().map_or(self.frame, |r| r.frame);
        if frame < oldest || index(self.frame - frame) > MAX_P2P_ROLLBACK {
            return Err(RollbackError::TooFar { frame, oldest });
        }
        let Some(record) = self.window.get_mut(index(frame - oldest)) else {
            return Err(RollbackError::TooFar { frame, oldest });
        };
        let wrong = record.inputs[1] != remote;
        record.inputs[1] = remote;
        record.predicted = false;
        self.queues[1].confirm(frame, remote);
        if wrong {
            self.mispredicted += 1;
            if let Some(entry) = self.tape_index(frame).and_then(|i| self.log.get_mut(i)) {
                entry[1] = remote;
            }
            self.first_incorrect = Some(self.first_incorrect.map_or(frame, |f| f.min(frame)));
            // The anchor refresh waits for the resim: its states are stale.
            self.confirmed = self.confirmed.max(frame.saturating_add(1));
        } else {
            self.note_confirmed(frame);
        }
        Ok(())
    }

    /// Restore the checkpoint at [`Self::first_incorrect_frame`] and replay
    /// the window to the tip once, however many late inputs were recorded.
    /// Returns the replayed frame count (`0` when nothing was wrong). The
    /// cursor clears even on `TooFar` (the frame left the window): that is a
    /// resync, not a resim.
    pub fn resolve_incorrect(&mut self) -> Result<usize, RollbackError> {
        let Some(frame) = self.first_incorrect.take() else {
            return Ok(0);
        };
        let oldest = self.oldest_frame();
        if frame < oldest {
            return Err(RollbackError::TooFar { frame, oldest });
        }
        let idx = index(frame - oldest);
        let distance = index(self.frame - frame);
        let Some(record) = self.window.get(idx) else {
            return Err(RollbackError::TooFar { frame, oldest });
        };
        let (start_idx, start_state, resim_len) = match self.anchor.clone() {
            Some(anchor)
                if self.sparse && self.anchor_frame <= frame && self.anchor_frame >= oldest =>
            {
                (
                    index(self.anchor_frame - oldest),
                    anchor,
                    index(self.frame - self.anchor_frame),
                )
            }
            _ => (idx, record.before.clone(), distance),
        };
        self.sim = start_state;
        let len = self.window.len();
        for i in start_idx..len {
            let Some(slot) = self.window.get(i) else {
                break;
            };
            let inputs = slot.inputs;
            self.sim.advance(inputs);
            let hash = self.sim.state_hash();
            let Some(slot) = self.window.get_mut(i) else {
                break;
            };
            let frame = slot.frame;
            slot.hash = hash;
            if let Some(entry) = self.tape_index(frame).and_then(|i| self.history.get_mut(i)) {
                *entry = hash;
            }
            if i + 1 < len {
                let next_before = self.sim.clone();
                if let Some(next) = self.window.get_mut(i + 1) {
                    next.before = next_before;
                }
            }
        }
        self.rollbacks += 1;
        self.max_rollback = self.max_rollback.max(resim_len.max(distance));
        self.refresh_anchor();
        Ok(distance)
    }

    fn note_confirmed(&mut self, frame: u32) {
        if frame.wrapping_add(1) > self.confirmed {
            self.confirmed = frame.wrapping_add(1);
            self.refresh_anchor();
        }
    }

    fn refresh_anchor(&mut self) {
        if self.sparse {
            self.anchor_frame = self.confirmed;
            if let Some(state) = self.snapshot_before(self.confirmed) {
                self.anchor = Some(state);
            } else if self.confirmed == self.frame {
                self.anchor = Some(self.sim.clone());
            }
        }
    }

    /// WAN-checked confirm over [`WAN_ROLLBACK_CAP`] (9: 150 ms at 60 Hz).
    /// Distances beyond 9 refuse with [`RollbackError::TooFar`] naming the
    /// WAN oldest (`frame - 9`), so a 61-tick-old correction takes the resync
    /// path, never a 61-tick resim. Storage window (`MAX_P2P_ROLLBACK` 128)
    /// still holds the frames; this is the authority rule on top of it.
    /// No live WAN session confirms through this guard yet, so it stays
    /// crate-private and test-only (S28 hygiene: a pub item with no
    /// production caller is the finding this closes).
    #[cfg(test)]
    fn confirm_frame_wan(&mut self, frame: u32, remote: u32) -> Result<usize, RollbackError> {
        let cap = u32::try_from(WAN_ROLLBACK_CAP).unwrap_or(u32::MAX);
        let gap = self.frame.saturating_sub(frame);
        if gap > cap {
            let oldest = self.frame.saturating_sub(cap);
            return Err(RollbackError::TooFar { frame, oldest });
        }
        self.confirm_frame(frame, remote)
    }

    /// Discard confirmed frames below `frame` (GGRS tail-advance, idea only).
    ///
    /// Discards window records and both per-player queue slots with
    /// `record < min(frame, self.frame)` while the front is confirmed
    /// (non-predicted); stops at the first predicted frame so unconfirmed
    /// history is never shed. Returns the discard count. Order-preserving:
    /// survivors keep frame order and the hash tape (`history`) is untouched,
    /// so replays and `check_desync` probes stay hash-identical.
    pub fn discard_confirmed_frames(&mut self, frame: u32) -> u32 {
        let target = frame
            .min(self.frame)
            .min(self.first_incorrect.unwrap_or(u32::MAX));
        let mut n = 0u32;
        while self
            .window
            .front()
            .is_some_and(|r| r.frame < target && !r.predicted)
        {
            let Some(record) = self.window.pop_front() else {
                break;
            };
            for q in &mut self.queues {
                q.discard_confirmed_frames(record.frame.saturating_add(1));
            }
            n += 1;
        }
        self.settled = self.oldest_frame();
        n
    }

    /// Compare one peer hash against the tape. Matches answer `Ok(())` and
    /// move the agreed watermark; mismatches and unknown frames refuse
    /// loudly and change nothing, except a mismatch records
    /// [`Self::last_mismatch`] and carries a [`DesyncReport`] built from
    /// the digests this session already holds (no wire change).
    pub fn check_desync(&self, frame: u32, peer_hash: u64) -> Result<(), RollbackError> {
        match self.spectator_hash_at(frame) {
            None => Err(RollbackError::UnknownFrame(frame)),
            Some(&local) if local == peer_hash => {
                self.last_agreed.set(Some(frame));
                Ok(())
            }
            Some(&local) => {
                self.last_mismatch.set(Some(frame));
                Err(RollbackError::Desync {
                    frame,
                    local,
                    peer: peer_hash,
                    report: self.desync_report(frame, local, peer_hash),
                })
            }
        }
    }

    /// One desync report from data this session already holds: the state
    /// digests both sides claimed, this side's input digest for the frame,
    /// the remote input digest only when its true input is settled
    /// (confirmed, no repeat-last prediction standing), and the last probe
    /// that agreed strictly below `frame` (`None` otherwise: a later
    /// agreement is not a watermark below the divergence).
    fn desync_report(&self, frame: u32, local: u64, peer: u64) -> DesyncReport {
        // The hash ring and the input ring push and evict together, so a
        // frame with a tape hash always has its input row.
        let inputs = self
            .input_at(frame)
            .map(|row| DesyncReport::digest_inputs(&row));
        let settled = match self.was_predicted(frame) {
            Some(predicted) => !predicted,
            None => self.queues[1]
                .get(frame)
                .is_some_and(|(_, confirmed)| confirmed),
        };
        DesyncReport::from_probe(
            frame,
            local,
            peer,
            inputs,
            if settled { inputs } else { None },
            self.last_agreed.get().filter(|&agreed| agreed < frame),
        )
    }
}

/// Seeded two-endpoint loopback shared by the input-delay tests (MP22, QN2).
#[cfg(test)]
pub(crate) mod loopback {
    use super::*;
    use crate::link::{FaultLink, Frame as LinkFrame, LinkProfile};
    use engine_core::mix64;

    /// Input copies per datagram (the session's `REDUNDANCY` plus one).
    const COPIES: u32 = 4;
    /// Ticks a scripted button is held, human-shaped.
    const HOLD: u32 = 4;

    /// History-sensitive integer sim: any wrong input changes every later hash.
    #[derive(Clone, Default)]
    pub(crate) struct LoopSim {
        acc: u64,
    }

    impl SimTick for LoopSim {
        fn advance(&mut self, inputs: [u32; PLAYERS]) {
            let mix = (u64::from(inputs[0]) << 8) | u64::from(inputs[1]);
            self.acc = (self.acc ^ mix).wrapping_mul(0x0000_0100_0000_01B3);
        }

        fn state_hash(&self) -> u64 {
            self.acc
        }
    }

    /// What one loopback run measured.
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Receipt {
        pub frames: u32,
        pub predicted: u64,
        pub mispredicted: u64,
        pub rollbacks: u64,
        pub max_rollback: usize,
        pub tip_matches_truth: bool,
    }

    fn script(salt: u64, tick: u32) -> u32 {
        (mix64(u64::from(tick / HOLD) ^ salt) % 5) as u32
    }

    /// Hand every datagram due this tick to the session, then resim once.
    fn deliver(session: &mut P2pSession<LoopSim>, link: &mut FaultLink) {
        for frame in link.due() {
            let LinkFrame::Datagram(bytes) = frame else {
                continue;
            };
            for pair in bytes.chunks_exact(8) {
                let word = |at: usize| {
                    u32::from_le_bytes([pair[at], pair[at + 1], pair[at + 2], pair[at + 3]])
                };
                session
                    .confirm_input(word(0), word(4))
                    .expect("arrival inside the window");
            }
        }
        session
            .resolve_incorrect()
            .expect("resim inside the window");
    }

    /// Play `frames` ticks of one endpoint against a peer whose inputs cross
    /// a seeded [`FaultLink`]. Both sides apply input `delay` frames late;
    /// the peer repeats its last [`COPIES`] inputs in every datagram. The
    /// counters are read once the datagrams still in flight have landed; a
    /// frame whose every copy was lost is then caught up from the script (the
    /// reliable lane) so the tip can be checked against the truth.
    pub(crate) fn run(delay: u32, profile: LinkProfile, seed: u64, frames: u32) -> Receipt {
        let (local_salt, remote_salt) = (seed ^ 0xA5A5, seed ^ 0x5A5A);
        let mut session = P2pSession::with_frame_delay(LoopSim::default(), delay);
        let mut link = FaultLink::new(profile, seed);
        for t in 0..frames {
            let mut datagram = Vec::new();
            for k in (0..COPIES.min(t + 1)).rev() {
                datagram.extend_from_slice(&(t - k + delay).to_le_bytes());
                datagram.extend_from_slice(&script(remote_salt, t - k).to_le_bytes());
            }
            link.send_datagram(&datagram);
            deliver(&mut session, &mut link);
            link.tick();
            let neutral = (t < delay).then_some(0);
            session.advance_frame(script(local_salt, t), neutral);
        }
        for _ in 0..profile.worst_delay() {
            deliver(&mut session, &mut link);
            link.tick();
        }
        let receipt = (
            session.predicted_count(),
            session.mispredicted_count(),
            session.rollback_count(),
            session.max_rollback(),
        );
        let mut truth = LoopSim::default();
        for f in 0..frames {
            let at = |salt| f.checked_sub(delay).map_or(0, |s| script(salt, s));
            let remote = at(remote_salt);
            truth.advance([at(local_salt), remote]);
            if session.was_predicted(f) == Some(true) {
                session.confirm_frame(f, remote).expect("catch-up");
            }
        }
        Receipt {
            frames,
            predicted: receipt.0,
            mispredicted: receipt.1,
            rollbacks: receipt.2,
            max_rollback: receipt.3,
            tip_matches_truth: session.sim().state_hash() == truth.state_hash()
                && session.stream().last() == Some(&truth.state_hash()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::LinkProfile;
    use crate::link::{Frame as LinkFrame, LoopbackWire, Peer, Wire};

    #[test]
    fn cap_respected_at_128() {
        let mut log = FrameLog::new(INPUT_QUEUE_LEN);
        for i in 0..140u64 {
            log.save(i, i as u32);
        }
        assert_eq!(log.len(), 128);
        assert_eq!(
            log.load(0),
            Some(Frame {
                hash: 12,
                input: 12
            })
        );
        assert_eq!(log.load(128), None);
    }

    #[test]
    fn fold_deterministic_and_sensitive() {
        let (mut a, mut b) = (FrameLog::new(8), FrameLog::new(8));
        for i in 0..4u64 {
            a.save(i * 10, i as u32);
            b.save(i * 10, i as u32);
        }
        assert_eq!(a.fold(), b.fold());
        b.save(99, 9);
        assert_ne!(a.fold(), b.fold());
    }

    /// Integer-only toy duel: two wrapping counters, FNV hash. No float.
    #[derive(Clone, Default, PartialEq, Eq)]
    struct ToyDuel {
        a: i32,
        b: i32,
    }

    impl SimTick for ToyDuel {
        fn advance(&mut self, inputs: [u32; PLAYERS]) {
            self.a = self.a.wrapping_add(inputs[0] as i32 - 1);
            self.b = self.b.wrapping_add(inputs[1] as i32 - 1);
        }

        fn state_hash(&self) -> u64 {
            let mut fold = Fnv64::new();
            fold.write_i32(self.a);
            fold.write_i32(self.b);
            fold.finish()
        }
    }

    fn duel_inputs(n: usize) -> (Vec<u32>, Vec<u32>) {
        let mut local = Vec::with_capacity(n);
        let mut remote = Vec::with_capacity(n);
        for f in 0..n {
            local.push((f as u32 * 7 + 1) % 5);
            remote.push((f as u32 * 11 + 2) % 5);
        }
        (local, remote)
    }

    fn run_tape(local: &[u32], remote: &[u32], loss_every: u32) -> P2pSession<ToyDuel> {
        let mut session = P2pSession::new(ToyDuel::default());
        for (f, (&l, &r)) in local.iter().zip(remote.iter()).enumerate() {
            let avail = if loss_every != 0 && (f as u32) % loss_every == loss_every - 1 {
                None
            } else {
                Some(r)
            };
            session.advance_frame(l, avail);
            if f >= 2
                && loss_every != 0
                && (f as u32) >= 2
                && (f as u32 - 2) % loss_every == loss_every - 1
            {
                let late = (f - 2) as u32;
                session
                    .confirm_frame(late, remote[f - 2])
                    .expect("late input inside window must confirm");
            }
        }
        // Drain any still-predicted in-window frames back to truth.
        for (f, &r) in remote.iter().enumerate() {
            if session.was_predicted(f as u32) == Some(true) {
                match session.confirm_frame(f as u32, r) {
                    Ok(_) => {}
                    Err(RollbackError::TooFar { .. }) => {}
                    Err(other) => panic!("unexpected confirm error: {other}"),
                }
            }
        }
        session
    }

    #[test]
    fn tape_identical_x2() {
        let (local, remote) = duel_inputs(48);
        let first = run_tape(&local, &remote, 5);
        let second = run_tape(&local, &remote, 5);
        assert_eq!(first.frames_played(), 48);
        assert_eq!(first.stream(), second.stream(), "same tape must hash twice");
        assert!(first.rollback_count() > 0, "loss must force resim");
        assert!(first.max_rollback() <= MAX_P2P_ROLLBACK);
        // A different remote tape must diverge: guards a vacuous equality.
        let (_, other_remote) = duel_inputs(48);
        let mut altered = other_remote.clone();
        altered[17] = altered[17].wrapping_add(1);
        let divergent = run_tape(&local, &altered, 5);
        assert_ne!(first.stream(), divergent.stream());
    }

    #[test]
    fn desync_refuses_loudly() {
        let (local, remote) = duel_inputs(16);
        let session = run_tape(&local, &remote, 0);
        let good = session.hash_at(7).expect("frame 7 authored");
        assert!(session.check_desync(7, good).is_ok());
        let before = session.stream();
        let err = session
            .check_desync(7, good.wrapping_add(1))
            .expect_err("mismatched peer hash must refuse");
        let RollbackError::Desync {
            frame,
            local,
            peer,
            report,
        } = err
        else {
            panic!("a mismatched peer hash must refuse with Desync");
        };
        assert_eq!((frame, local, peer), (7, good, good.wrapping_add(1)));
        assert_eq!(report.tick, 7, "the report names the divergent tick");
        assert!(
            report.inputs_agree(),
            "run_tape drained every prediction: the wire is exonerated"
        );
        assert_eq!(session.stream(), before, "failed check must change nothing");
        assert_eq!(
            session.check_desync(999, 0),
            Err(RollbackError::UnknownFrame(999))
        );
        assert_eq!(session.frames_played(), 16);
    }

    #[test]
    fn rollback_converges_after_mispredict() {
        // Eight frames fit well inside the 129-slot window, so every late
        // input is still confirmable; a run past the window would evict
        // frame 0 first (correct TooFar) and must not converge.
        let (local, remote) = duel_inputs(8);
        let mut reference = ToyDuel::default();
        for (&l, &r) in local.iter().zip(remote.iter()) {
            reference.advance([l, r]);
        }
        let mut session = P2pSession::new(ToyDuel::default());
        for &l in &local {
            // Drop every remote input: pure repeat-last prediction.
            session.advance_frame(l, None);
        }
        assert_eq!(session.predicted_count(), 8);
        for (f, &r) in remote.iter().enumerate() {
            session
                .confirm_frame(f as u32, r)
                .expect("in-window late input must confirm");
        }
        assert!(session.rollback_count() > 0, "mispredicts must resim");
        assert_eq!(session.sim().state_hash(), reference.state_hash());
        assert_eq!(
            session.hash_at(7),
            Some(reference.state_hash()),
            "tape tip must equal the true sim"
        );
    }

    #[test]
    fn rollback_cost() {
        // MP14 spike: the 8-tick mispredict rewind restores the frame-0
        // checkpoint and replays the window, converging to the true-input
        // hash inside the 60 Hz frame budget (16.6 ms) with headroom.
        let (local, remote) = duel_inputs(8);
        let mut reference = ToyDuel::default();
        for (&l, &r) in local.iter().zip(remote.iter()) {
            reference.advance([l, r]);
        }
        let mut session = P2pSession::new(ToyDuel::default());
        for &l in &local {
            session.advance_frame(l, None);
        }
        assert!(
            session.snapshot_before(0).is_some(),
            "the rewind restores the frame-0 checkpoint"
        );
        #[expect(
            clippy::disallowed_types,
            reason = "wall-clock resim sample, never hashed."
        )]
        #[expect(
            clippy::disallowed_methods,
            reason = "wall-clock resim sample, never hashed."
        )]
        let started = std::time::Instant::now(); // det-ok: wall-clock resim sample, never hashed.
        let distance = session
            .confirm_frame(0, remote[0])
            .expect("in-window late input must confirm");
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(
            distance, LUNARIS_WINDOW,
            "the 8-tick rewind resimulates one window"
        );
        assert!(ms < 2.0, "8-tick rewind {ms:.3}ms must stay under 2 ms");
        for (f, &r) in remote.iter().enumerate().skip(1) {
            session
                .confirm_frame(f as u32, r)
                .expect("in-window late input must confirm");
        }
        assert_eq!(session.sim().state_hash(), reference.state_hash());
        assert_eq!(
            session.hash_at(7),
            Some(reference.state_hash()),
            "tape tip must equal the true sim"
        );
        println!("rollback-cost: 8-tick rewind wall={ms:.3}ms budget=2ms frame=16.6ms");
    }

    #[test]
    fn rollback_too_far_refuses() {
        let (local, remote) = duel_inputs(MAX_P2P_ROLLBACK + 2);
        let mut session = P2pSession::new(ToyDuel::default());
        for (&l, &r) in local.iter().zip(remote.iter()) {
            session.advance_frame(l, Some(r));
        }
        let oldest = session.frame() - session.len() as u32;
        let before = session.stream();
        let err = session
            .confirm_frame(0, remote[0].wrapping_add(1))
            .expect_err("evicted frame must refuse");
        assert_eq!(err, RollbackError::TooFar { frame: 0, oldest });
        assert_eq!(session.stream(), before, "refused confirm changes nothing");
        assert_eq!(
            session.confirm_frame(session.frame() + MAX_P2P_ROLLBACK as u32 + 1, 1),
            Err(RollbackError::FutureFrame {
                frame: session.frame() + MAX_P2P_ROLLBACK as u32 + 1,
                current: session.frame(),
            })
        );
    }

    #[test]
    fn save_load_roundtrip_over_tick() {
        let mut session = P2pSession::new(ToyDuel::default());
        session.advance_frame(2, Some(3));
        let (snap, hash) = (session.save_game_state(), session.sim().state_hash());
        session.advance_frame(4, Some(1));
        assert_ne!(session.sim().state_hash(), hash);
        session.load_game_state(snap);
        assert_eq!(session.sim().state_hash(), hash);
        assert!(session.snapshot_before(0).is_some());
        assert_eq!(session.hash_at(0), Some(hash));
        assert_eq!(session.was_predicted(0), Some(false));
    }

    #[test]
    fn queues_hold_128_and_discard_advances_tail() {
        let (local, remote) = duel_inputs(INPUT_QUEUE_LEN + 2);
        let mut session = P2pSession::new(ToyDuel::default());
        for (&l, &r) in local.iter().zip(remote.iter()) {
            session.advance_frame(l, Some(r));
        }
        assert_eq!(session.len(), MAX_P2P_ROLLBACK + 1);
        assert_eq!(session.queue_len(0), Some(INPUT_QUEUE_LEN));
        assert_eq!(session.queue_len(1), Some(INPUT_QUEUE_LEN));
        assert_eq!(session.oldest_frame(), 1);
        let stream_before = session.stream();
        let dropped = session.discard_confirmed_frames(32);
        assert_eq!(dropped, 31);
        assert_eq!(session.oldest_frame(), 32);
        assert_eq!(session.stream(), stream_before, "discard keeps tape");
        assert_eq!(
            session.queue(1).and_then(|q| q.first_frame()),
            Some(32),
            "queue tail advances with the window"
        );
        let oldest = session.oldest_frame();
        assert_eq!(
            session.confirm_frame(0, remote[0]),
            Err(RollbackError::TooFar { frame: 0, oldest })
        );
    }

    #[test]
    fn discard_stops_at_predicted_and_clamps_to_frame() {
        let (local, remote) = duel_inputs(16);
        let mut session = P2pSession::new(ToyDuel::default());
        for (f, &l) in local.iter().enumerate() {
            let avail = if f < 4 { None } else { Some(remote[f]) };
            session.advance_frame(l, avail);
        }
        // Frames 0..4 predicted; discard must stop at frame 0 (nothing shed).
        assert_eq!(session.discard_confirmed_frames(16), 0);
        assert_eq!(session.oldest_frame(), 0);
        // Confirm frame 0, then one slot sheds; far-future requests clamp.
        session.confirm_frame(0, remote[0]).expect("confirm 0");
        assert_eq!(session.discard_confirmed_frames(16), 1);
        assert_eq!(session.oldest_frame(), 1);
        let at = session.frame();
        let shed = session.discard_confirmed_frames(u32::MAX);
        assert!(session.oldest_frame() >= 1);
        assert!(shed <= at);
    }

    #[test]
    fn loopback_seeded_loss_converges_replay_empty() {
        // 2-player loopback, same orientation on both ends (`[p0, p1]`):
        // two endpoints take turns as the p0 owner against the same p1
        // script with independent seeded loss, then both must converge to
        // the true-input replay with an empty diff and green peer hashes.
        let n = 96;
        let (a_inputs, b_inputs) = duel_inputs(n);
        let mut a = P2pSession::new(ToyDuel::default());
        let mut b = P2pSession::new(ToyDuel::default());
        for f in 0..n {
            let la = a_inputs[f];
            let rb = b_inputs[f];
            // Seeded loss: endpoint A drops every 7th==5, B every 7th==3.
            if f % 7 == 5 {
                a.advance_frame(la, None);
            } else {
                a.advance_frame(la, Some(rb));
            }
            if f % 7 == 3 {
                b.advance_frame(la, None);
            } else {
                b.advance_frame(la, Some(rb));
            }
            if f >= 2 {
                if (f - 2) % 7 == 5 {
                    a.confirm_frame((f - 2) as u32, b_inputs[f - 2])
                        .expect("loopback late input in window");
                }
                if (f - 2) % 7 == 3 {
                    b.confirm_frame((f - 2) as u32, b_inputs[f - 2])
                        .expect("loopback late input in window");
                }
            }
        }
        for f in 0..n as u32 {
            if a.was_predicted(f) == Some(true) {
                let _ = a.confirm_frame(f, b_inputs[f as usize]);
            }
            if b.was_predicted(f) == Some(true) {
                let _ = b.confirm_frame(f, b_inputs[f as usize]);
            }
        }
        // Cross-check: a's tape at every frame equals b's mirrored tape only
        // when both saw truth; instead each must equal its own true-input
        // replay with an empty diff, and every peer hash verifies green.
        for side in [&a, &b] {
            for f in 0..n as u32 {
                let h = side.hash_at(f).expect("authored frame hashed");
                side.check_desync(f, h).expect("peer hash green");
            }
        }
        let mut truth = ToyDuel::default();
        for (&l, &r) in a_inputs.iter().zip(b_inputs.iter()) {
            truth.advance([l, r]);
        }
        assert_eq!(a.sim().state_hash(), truth.state_hash());
        assert_eq!(a.stream(), b.stream(), "loopback tapes agree");
        let replay = run_tape(&a_inputs, &b_inputs, 0).stream();
        assert_eq!(a.stream(), replay, "replay diff empty");
        assert!(a.rollback_count() > 0, "seeded loss must resim");
    }

    #[test]
    fn checksum_log_saves_per_tick_and_tracks_mismatch() {
        let (local, remote) = duel_inputs(8);
        let session = run_tape(&local, &remote, 0);
        assert_eq!(session.last_mismatch(), None);
        let log = session.checksum_log();
        assert_eq!(log.len(), 8);
        for (frame, hash) in log {
            assert_eq!(session.checksum_at(frame), Some(hash));
            assert_eq!(session.hash_at(frame), Some(hash));
        }
        let good = session.checksum_at(5).expect("frame 5 hashed");
        assert!(session.check_desync(5, good).is_ok());
        assert_eq!(session.last_mismatch(), None);
        session
            .check_desync(5, good.wrapping_add(1))
            .expect_err("bad peer hash must refuse");
        assert_eq!(session.last_mismatch(), Some(5));
    }

    #[test]
    fn wan_61_tick_rollback_refused_against_9_tick_cap() {
        // 150 ms at 60 Hz is 9 ticks (anim `RTT_TICKS_150MS`, wan_soak
        // `RTT_TICKS`); a 61-tick-old WAN correction exceeds the 9-tick
        // authority window and must refuse `TooFar` (resync path), never a
        // 61-tick resim. Storage window 128 still holds the frame: the old
        // code confirmed it (false PASS); the WAN guard refuses it.
        assert_eq!(super::WAN_ROLLBACK_CAP, 9, "150 ms at 60 Hz is 9 ticks");
        let (local, remote) = duel_inputs(70);
        let mut session = P2pSession::new(ToyDuel::default());
        for (&l, &r) in local.iter().zip(remote.iter()) {
            session.advance_frame(l, Some(r));
        }
        assert_eq!(session.frame(), 70);
        // Frame 9 at tip 70 is 61 ticks old (70 - 9 = 61).
        let before = session.stream();
        let err = session
            .confirm_frame_wan(9, remote[9].wrapping_add(1))
            .expect_err("61-tick WAN rollback must refuse");
        assert_eq!(
            err,
            RollbackError::TooFar {
                frame: 9,
                oldest: 61
            }
        );
        assert_eq!(session.stream(), before, "refused confirm changes nothing");
        // In-window WAN corrections still confirm (9-tick distance ok).
        session
            .confirm_frame_wan(61, remote[61])
            .expect("9-tick WAN correction confirms");
    }

    #[test]
    fn wan_cap_is_the_netcode_window_inside_the_storage_window() {
        // One owner: the WAN guard follows the netcode session window and
        // sits inside the P2P storage window, so it never widens by itself.
        assert_eq!(super::WAN_ROLLBACK_CAP, crate::netcode::MAX_ROLLBACK);
        const { assert!(super::WAN_ROLLBACK_CAP < MAX_P2P_ROLLBACK) };
    }

    #[test]
    fn p2p_window_has_its_own_name_and_holds_128() {
        // S28 hygiene: `MAX_ROLLBACK` meant 9 in netcode and 128 here.
        // The 128-slot P2P storage window owns the distinct name
        // `MAX_P2P_ROLLBACK` so the two windows never confuse.
        assert_eq!(super::MAX_P2P_ROLLBACK, 128);
        assert_eq!(super::WAN_ROLLBACK_CAP, crate::netcode::MAX_ROLLBACK);
        const { assert!(super::WAN_ROLLBACK_CAP < super::MAX_P2P_ROLLBACK) };
    }

    #[test]
    fn lunaris_window8_seat_and_cursors() {
        let (local, remote) = duel_inputs(10);
        let mut s = P2pSession::new(ToyDuel::default());
        s.enable_lunaris_v2(7);
        for (f, (&l, &r)) in local.iter().zip(remote.iter()).enumerate() {
            s.advance_frame(l, Some(r));
            assert_eq!(s.frontier(), s.frame());
            assert_eq!(s.input_at(f as u32), Some([l, r]));
        }
        assert_eq!(s.spectator_hash_at(9).copied(), s.hash_at(9));
        assert_eq!(s.discard_confirmed_frames(5), 5);
        assert_eq!(s.settled(), 5);
        let before = s.stream();
        let bad = s.confirm_frame_lunaris(9, remote[9], 8);
        assert!(matches!(bad, Err(RollbackError::BadSeat { .. })));
        assert!(matches!(
            s.confirm_frame_lunaris(0, remote[0], 7),
            Err(RollbackError::TooFar { .. })
        ));
        assert_eq!(s.stream(), before);
    }

    #[test]
    fn sparse_save_rollback_converges() {
        let (local, remote) = duel_inputs(48);
        let dense = run_tape(&local, &remote, 5);
        let mut sparse = P2pSession::new(ToyDuel::default());
        sparse.enable_sparse_saving(true);
        for (f, (&l, &r)) in local.iter().zip(remote.iter()).enumerate() {
            let avail = if (f as u32) % 5 == 4 { None } else { Some(r) };
            sparse.advance_frame(l, avail);
            if f >= 2 && (f as u32 - 2) % 5 == 4 {
                let late = (f - 2) as u32;
                sparse
                    .confirm_frame(late, remote[f - 2])
                    .expect("late input inside window must confirm");
            }
        }
        for (f, &r) in remote.iter().enumerate() {
            if sparse.was_predicted(f as u32) == Some(true) {
                match sparse.confirm_frame(f as u32, r) {
                    Ok(_) => {}
                    Err(RollbackError::TooFar { .. }) => {}
                    Err(other) => panic!("unexpected confirm error: {other}"),
                }
            }
        }
        assert!(sparse.rollback_count() > 0, "loss must force resim");
        assert!(sparse.max_rollback() <= MAX_P2P_ROLLBACK);
        assert_eq!(
            sparse.stream(),
            dense.stream(),
            "sparse tip must equal dense tip"
        );
        assert_eq!(sparse.sim().state_hash(), dense.sim().state_hash());
    }

    #[test]
    fn sparse_refuses_full_window_check() {
        let (local, remote) = duel_inputs(8);
        let mut dense = P2pSession::new(ToyDuel::default());
        for (&l, &r) in local.iter().zip(remote.iter()) {
            dense.advance_frame(l, Some(r));
        }
        assert!(dense.verify_full_window(8).is_ok());
        let mut sparse = P2pSession::new(ToyDuel::default());
        sparse.enable_sparse_saving(true);
        for (&l, &r) in local.iter().zip(remote.iter()) {
            sparse.advance_frame(l, Some(r));
        }
        assert!(matches!(
            sparse.verify_full_window(8),
            Err(RollbackError::TooFar { .. })
        ));
    }

    #[test]
    fn first_incorrect_batches_one_resim() {
        // Frames 0..8 predicted; the true inputs arrive 6, 4, 5 in one tick.
        // Confirming one by one resims three times; marking the first
        // incorrect frame resims once, from frame 4, to the same tip.
        let (local, remote) = duel_inputs(8);
        let play = || {
            let mut s = P2pSession::new(ToyDuel::default());
            for &l in &local {
                s.advance_frame(l, None);
            }
            s
        };
        let mut batched = play();
        for f in [6u32, 4, 5] {
            batched
                .confirm_input(f, remote[f as usize])
                .expect("in window");
        }
        assert_eq!(batched.first_incorrect_frame(), Some(4));
        assert_eq!(batched.rollback_count(), 0, "marking never resims");
        assert_eq!(batched.resolve_incorrect(), Ok(4));
        assert_eq!(batched.first_incorrect_frame(), None);
        assert_eq!(batched.rollback_count(), 1);
        assert_eq!(batched.resolve_incorrect(), Ok(0), "nothing left to resim");
        let mut single = play();
        for f in [6u32, 4, 5] {
            single
                .confirm_frame(f, remote[f as usize])
                .expect("in window");
        }
        assert_eq!(single.rollback_count(), 3);
        assert_eq!(batched.stream(), single.stream());
        assert_eq!(batched.sim().state_hash(), single.sim().state_hash());
    }

    #[test]
    fn discard_keeps_the_unresolved_incorrect_frame() {
        let (local, remote) = duel_inputs(8);
        let mut s = P2pSession::new(ToyDuel::default());
        for &l in &local {
            s.advance_frame(l, None);
        }
        s.confirm_input(2, remote[2].wrapping_add(1))
            .expect("in window");
        assert_eq!(s.discard_confirmed_frames(8), 0, "frame 0 is predicted");
        let predicted = s.input_at(0).map_or(0, |i| i[1]);
        s.confirm_input(0, predicted).expect("in window");
        s.confirm_input(1, s.input_at(1).map_or(0, |i| i[1]))
            .expect("in window");
        assert_eq!(
            s.discard_confirmed_frames(8),
            2,
            "stops at the incorrect frame"
        );
        assert_eq!(s.oldest_frame(), 2);
        assert!(s.resolve_incorrect().is_ok());
    }

    #[test]
    fn frame_delay_applies_local_input_late() {
        let mut s = P2pSession::with_frame_delay(ToyDuel::default(), DEFAULT_FRAME_DELAY);
        assert_eq!(DEFAULT_FRAME_DELAY, 2);
        assert_eq!(s.frame_delay(), 2);
        for f in 0..5u32 {
            s.advance_frame(10 + f, Some(1));
        }
        let applied: Vec<u32> = (0..5).filter_map(|f| s.input_at(f)).map(|i| i[0]).collect();
        assert_eq!(applied, [0, 0, 10, 11, 12], "frame f runs the input of f-2");
        assert_eq!(P2pSession::new(ToyDuel::default()).frame_delay(), 0);
    }

    #[test]
    fn delay_cuts_predictions_and_rollbacks_on_a_lossy_link() {
        // MP22 receipt: same seeded link (1-3 ticks one way, 2% loss), delay
        // 0 vs the production 2. The peer's input travels for free during
        // the delay, so fewer frames are predicted and fewer resims run.
        let profile = LinkProfile {
            latency: 1,
            jitter: 2,
            loss_pct: 2,
            reorder_pct: 0,
        };
        let off = loopback::run(0, profile, 0xD31A, 300);
        let on = loopback::run(DEFAULT_FRAME_DELAY, profile, 0xD31A, 300);
        println!("delay-off {off:?}\ndelay-on  {on:?}");
        for r in [off, on] {
            assert!(r.tip_matches_truth, "tip must equal the truth hash: {r:?}");
            assert!(r.max_rollback <= LUNARIS_WINDOW, "resim over 8: {r:?}");
        }
        assert!(on.predicted < off.predicted, "{on:?} vs {off:?}");
        assert!(on.rollbacks < off.rollbacks, "{on:?} vs {off:?}");
        let again = loopback::run(DEFAULT_FRAME_DELAY, profile, 0xD31A, 300);
        assert_eq!(
            (again.predicted, again.rollbacks, again.max_rollback),
            (on.predicted, on.rollbacks, on.max_rollback),
            "same seed, same receipt"
        );
    }

    // ---- Link conditioner + 4-seat predicting client proof (ON21) ----

    /// Integer-only toy squad: four seat inputs mixed into one counter.
    #[derive(Clone, Default)]
    struct QuadSim {
        acc: u64,
    }

    impl SeatSim<4> for QuadSim {
        fn step(&mut self, inputs: [u32; 4]) {
            let mut mix = 0u64;
            for (seat, input) in inputs.iter().enumerate() {
                let shift = u32::try_from(seat).unwrap_or(0) * 8;
                mix ^= u64::from(*input) << shift;
            }
            self.acc = engine_core::mix64(self.acc ^ mix);
        }

        fn hash(&self) -> u64 {
            self.acc
        }
    }

    /// Four seats, one authored tick every [`QUAD_HOLD`] ticks.
    const QUAD_SEATS: usize = 4;
    /// Ticks a scripted button is held.
    const QUAD_HOLD: u32 = 3;

    fn quad_script(seat: usize, tick: u32) -> u32 {
        let lane = 0x51u64 ^ u64::try_from(seat).unwrap_or(0);
        u32::try_from(engine_core::mix64(u64::from(tick / QUAD_HOLD) ^ lane) % 4).unwrap_or(0)
    }

    /// One authored input datagram: tick, seat, input.
    fn quad_datagram(tick: u32, seat: usize, input: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&tick.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(seat).unwrap_or(u32::MAX).to_le_bytes());
        bytes.extend_from_slice(&input.to_le_bytes());
        bytes
    }

    fn quad_read(bytes: &[u8]) -> Option<(u32, usize, u32)> {
        if bytes.len() < 12 {
            return None;
        }
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        Some((
            word(0),
            usize::try_from(word(4)).unwrap_or(usize::MAX),
            word(8),
        ))
    }

    /// What one 4-seat headless run measured.
    #[derive(Clone, Copy, Debug)]
    struct QuadReceipt {
        rollbacks: u64,
        predicted: u64,
        max_rollback: usize,
        tips: [u64; QUAD_SEATS],
        truth: u64,
    }

    /// Headless 4-seat bout: one authoritative sim plus four predicting
    /// clients, every authored input crossing [`LoopbackWire`]. The server
    /// steps its truth and relays; a client presses `delay` ticks ahead, so a
    /// relay of `out + in` ticks is fully hidden when `delay` covers it.
    fn run_quad(delay: u32, conditioner: LinkConditionerConfig, ticks: u32) -> QuadReceipt {
        let seats = u8::try_from(QUAD_SEATS).expect("four seats fit a byte");
        let mut wire = LoopbackWire::new(
            seats,
            conditioner.outgoing(),
            conditioner.incoming(),
            0x215E_u64,
        );
        let mut clients: [SeatSession<QUAD_SEATS, QuadSim>; QUAD_SEATS] =
            std::array::from_fn(|seat| SeatSession::new(QuadSim::default(), seat, delay));
        // Countdown: the inputs authored before the first play are already
        // exchanged, so ticks `0..delay` start complete.
        for tick in 0..delay {
            for seat in 0..QUAD_SEATS {
                let sample = quad_script(seat, tick);
                for client in clients.iter_mut() {
                    client
                        .confirm(tick, seat, sample)
                        .expect("countdown row inside the window");
                }
            }
        }
        for w in 0..ticks {
            // Clients take in the peer inputs the server relayed.
            for (seat, client) in clients.iter_mut().enumerate() {
                let at = Peer::Client(u8::try_from(seat).unwrap_or(u8::MAX));
                for (_, frame) in wire.recv(at) {
                    let LinkFrame::Datagram(bytes) = frame else {
                        continue;
                    };
                    if let Some((tick, from, input)) = quad_read(&bytes) {
                        client
                            .confirm(tick, from, input)
                            .expect("inside the window");
                    }
                }
            }
            // The server relays every authored input it holds to every client.
            for (_, frame) in wire.recv(Peer::Server) {
                let LinkFrame::Datagram(bytes) = frame else {
                    continue;
                };
                for to in 0..QUAD_SEATS {
                    wire.datagram(
                        Peer::Server,
                        Peer::Client(u8::try_from(to).unwrap_or(u8::MAX)),
                        &bytes,
                    );
                }
            }
            // Each client authors tick `w + delay` and plays tick `w`.
            for (seat, client) in clients.iter_mut().enumerate() {
                let authored = w + delay;
                let sample = quad_script(seat, authored);
                client.advance(sample);
                wire.datagram(
                    Peer::Client(u8::try_from(seat).unwrap_or(u8::MAX)),
                    Peer::Server,
                    &quad_datagram(authored, seat, sample),
                );
            }
            wire.tick();
        }
        // Drain: let every datagram still in flight land, so the tail ticks are
        // corrected before the tip is read. No tick is played.
        for _ in 0..delay + 2 {
            for (seat, client) in clients.iter_mut().enumerate() {
                let at = Peer::Client(u8::try_from(seat).unwrap_or(u8::MAX));
                for (_, frame) in wire.recv(at) {
                    let LinkFrame::Datagram(bytes) = frame else {
                        continue;
                    };
                    if let Some((tick, from, input)) = quad_read(&bytes) {
                        client
                            .confirm(tick, from, input)
                            .expect("inside the window");
                    }
                }
            }
            for (_, frame) in wire.recv(Peer::Server) {
                let LinkFrame::Datagram(bytes) = frame else {
                    continue;
                };
                for to in 0..QUAD_SEATS {
                    wire.datagram(
                        Peer::Server,
                        Peer::Client(u8::try_from(to).unwrap_or(u8::MAX)),
                        &bytes,
                    );
                }
            }
            wire.tick();
        }
        // Ground truth: the same authored inputs, applied on time.
        let mut truth = QuadSim::default();
        let mut truth_tape = Vec::with_capacity(usize::try_from(ticks).unwrap_or(0));
        for tick in 0..ticks {
            let mut inputs = [0u32; QUAD_SEATS];
            for (seat, slot) in inputs.iter_mut().enumerate() {
                *slot = quad_script(seat, tick);
            }
            truth.step(inputs);
            truth_tape.push(truth.hash());
        }
        let mut rollbacks = 0u64;
        let mut predicted = 0u64;
        let mut max_rollback = 0usize;
        let mut tips = [0u64; QUAD_SEATS];
        for (seat, client) in clients.iter().enumerate() {
            assert_eq!(client.delay(), delay, "each seat runs the derived delay");
            rollbacks += client.rollbacks();
            predicted += client.predicted_count();
            max_rollback = max_rollback.max(client.max_rollback());
            tips[seat] = client.sim().hash();
            for tick in 0..ticks {
                assert_eq!(
                    client.hash_at(tick),
                    truth_tape.get(usize::try_from(tick).unwrap_or(0)).copied(),
                    "seat {seat} diverged from the truth at tick {tick}"
                );
            }
        }
        QuadReceipt {
            rollbacks,
            predicted,
            max_rollback,
            tips,
            truth: truth.hash(),
        }
    }

    #[test]
    fn conditioner_derives_the_delay_that_hides_the_relay() {
        assert_eq!(LinkConditionerConfig::clean().worst_relay(), 0);
        assert_eq!(LinkConditionerConfig::clean().input_delay(), 0);
        let lan = LinkConditionerConfig::lan();
        assert_eq!(lan.worst_relay(), 2, "LAN is one tick each way");
        assert_eq!(lan.input_delay(), 2, "a two-frame delay covers the relay");
        assert_eq!(
            lan.outgoing(),
            LinkProfile {
                latency: 1,
                jitter: 0,
                loss_pct: 0,
                reorder_pct: 0
            }
        );
        assert_eq!(
            lan.incoming(),
            LinkProfile {
                latency: 1,
                jitter: 0,
                loss_pct: 0,
                reorder_pct: 0
            }
        );
        let wan = LinkConditionerConfig::new(4, 2, 4, 2);
        assert_eq!(wan.worst_relay(), 12);
        assert_eq!(
            wan.input_delay(),
            crate::delay::MAX_DELAY,
            "clamped to the knob"
        );
    }

    #[test]
    fn input_buffer_presses_delay_ticks_ahead_and_repeats_last() {
        let mut buffer = InputBuffer::<QUAD_SEATS>::new(2);
        assert_eq!(buffer.delay(), 2);
        assert!(buffer.is_empty());
        assert!(buffer.press(0, 0, 7), "a new row is a change");
        assert_eq!(buffer.get(2, 0), Some(7), "play_tick 0 lands at 2");
        assert_eq!(buffer.get(0, 0), None);
        assert!(!buffer.press(0, 0, 7), "the same value is not a change");
        assert!(buffer.press(0, 0, 8), "a different value is a change");
        assert!(!buffer.complete(2), "the other seats are missing");
        assert_eq!(buffer.resolve(2), [8, 0, 0, 0], "absent seats repeat last");
        buffer.note_played([1, 2, 3, 4]);
        assert_eq!(buffer.resolve(9), [1, 2, 3, 4], "repeat-last source");
        assert!(buffer.record(9, 0, 5));
        assert!(buffer.record(9, 1, 6));
        assert!(buffer.record(9, 2, 7));
        assert!(buffer.record(9, 3, 8));
        assert!(buffer.complete(9), "every seat authored tick 9");
        assert_eq!(buffer.resolve(9), [5, 6, 7, 8]);
        assert!(!buffer.record(9, QUAD_SEATS, 1), "no such seat");
    }

    #[test]
    fn input_buffer_evicts_beyond_its_capacity() {
        let mut buffer = InputBuffer::<1>::new(0);
        let cap = u32::try_from(INPUT_QUEUE_LEN).unwrap_or(u32::MAX);
        for tick in 0..cap + 16 {
            buffer.record(tick, 0, tick);
        }
        assert_eq!(buffer.len(), INPUT_QUEUE_LEN, "held at the queue length");
        assert_eq!(buffer.get(0, 0), None, "the oldest rows are evicted");
        assert_eq!(buffer.get(cap + 15, 0), Some(cap + 15));
        assert!(!buffer.record(0, 0, 9), "an evicted tick refuses");
        let dropped = buffer.discard_before(cap + 15);
        assert_eq!(dropped, u32::try_from(INPUT_QUEUE_LEN - 1).unwrap_or(0));
        assert_eq!(buffer.len(), 1, "only the newest row survives");
        assert!(!buffer.is_empty());
    }

    #[test]
    fn seat_session_resims_one_late_mispredict_to_the_same_tip() {
        let mut client = SeatSession::<QUAD_SEATS, QuadSim>::new(QuadSim::default(), 0, 0);
        for tick in 0..4u32 {
            client.advance(quad_script(0, tick));
        }
        assert_eq!(client.tick(), 4);
        assert_eq!(client.predicted_count(), 4, "no peer input ever arrived");
        assert_eq!(client.rollbacks(), 0);
        let distance = client.confirm(1, 1, 3).expect("inside the window");
        assert_eq!(distance, 3, "three frames replay past tick 1");
        assert_eq!(client.rollbacks(), 1);
        assert_eq!(client.mispredicted_count(), 1);
        assert_eq!(client.max_rollback(), 3);
        assert_eq!(client.confirm(1, 1, 3), Ok(0), "already true, no resim");
        assert_eq!(client.rollbacks(), 1);
        assert!(matches!(
            client.confirm(1, QUAD_SEATS, 0),
            Err(RollbackError::NoSeat { seat }) if seat == QUAD_SEATS
        ));
        let mut truth = QuadSim::default();
        for tick in 0..4u32 {
            let mut inputs = [0u32; QUAD_SEATS];
            inputs[0] = quad_script(0, tick);
            if tick == 1 {
                inputs[1] = 3;
            }
            truth.step(inputs);
        }
        assert_eq!(client.sim().hash(), truth.hash(), "the corrected tip");
    }

    #[test]
    fn seat_session_refuses_a_tick_evicted_from_its_window() {
        let mut client = SeatSession::<QUAD_SEATS, QuadSim>::new(QuadSim::default(), 0, 0);
        for tick in 0..u32::try_from(MAX_P2P_ROLLBACK + 8).unwrap_or(0) {
            client.advance(quad_script(0, tick));
        }
        assert!(matches!(
            client.confirm(0, 1, 1),
            Err(RollbackError::TooFar { frame: 0, .. })
        ));
        assert_eq!(client.rollbacks(), 0, "a refusal changes nothing");
    }

    #[test]
    fn four_seat_server_two_frame_delay_holds_rollbacks_under_three() {
        // ON21 accept: 4-client headless server, 2-frame delay, fewer than 3
        // rollback events per 1000 ticks, and every client tip is the truth.
        let conditioner = LinkConditionerConfig::lan();
        let delay = conditioner.input_delay();
        assert_eq!(delay, 2);
        let ticks = 1000;
        let on = run_quad(delay, conditioner, ticks);
        assert!(
            on.rollbacks < 3,
            "delay {delay} over a two-tick relay must hide the lag: {on:?}"
        );
        assert_eq!(on.predicted, 0, "the delay covers every authored tick");
        for tip in on.tips {
            assert_eq!(tip, on.truth, "each client tip is the server truth: {on:?}");
        }
        // Hash x2: the same run repeats exactly.
        let again = run_quad(delay, conditioner, ticks);
        assert_eq!(
            (again.rollbacks, again.max_rollback, again.tips, again.truth),
            (on.rollbacks, on.max_rollback, on.tips, on.truth),
            "same seed, same receipt"
        );
        // Non-vacuous contrast: without the delay every late tick resims.
        let bare = run_quad(0, conditioner, ticks);
        assert!(
            bare.rollbacks > u64::from(ticks) / 2,
            "delay 0 must resim every late tick: {bare:?}"
        );
        println!("quad on {on:?}\nquad bare {bare:?}");
    }

    #[test]
    fn e15_tape_flat_over_10k_frames() {
        // E15 accept: a 10k-frame run keeps the rollback tape flat (bytes
        // printed at 1k and 10k), the tape stops at `TAPE_CAP`, and the
        // replay digest is identical x2.
        fn run(frames: u32) -> (u64, usize, usize) {
            let remote = |f: u32| f.wrapping_mul(11).wrapping_add(2) % 5;
            let mut session = P2pSession::new(ToyDuel::default());
            for f in 0..frames {
                let avail = (f % 5 != 4).then(|| remote(f));
                session.advance_frame(f.wrapping_mul(7).wrapping_add(1) % 5, avail);
                if f >= 2 && (f - 2) % 5 == 4 {
                    session
                        .confirm_frame(f - 2, remote(f - 2))
                        .expect("late input inside window");
                }
            }
            let bytes = session.history.capacity() * std::mem::size_of::<u64>()
                + session.log.capacity() * std::mem::size_of::<[u32; PLAYERS]>()
                + session.window.capacity() * std::mem::size_of::<FrameRecord<ToyDuel>>();
            let mut fold = Fnv64::new();
            for hash in session.stream() {
                fold.write_u64(hash);
            }
            fold.write_u64(session.sim().state_hash());
            fold.write_u64(session.rollback_count());
            (fold.finish(), bytes, session.stream().len())
        }
        let (_, bytes_1k, len_1k) = run(1_000);
        let (digest, bytes_10k, len_10k) = run(10_000);
        let (digest_again, _, _) = run(10_000);
        eprintln!("e15 tape buffers: 1k frames {bytes_1k} B, 10k frames {bytes_10k} B");
        assert_eq!(
            bytes_10k, bytes_1k,
            "tape buffers flat from 1k to 10k frames"
        );
        assert!(len_10k <= TAPE_CAP, "tape {len_10k} over its cap");
        assert_eq!(len_1k, len_10k, "both runs hold the saturated tape");
        assert_eq!(digest, digest_again, "replay digest x2");
    }

    /// Lines naming the retired hash-only `lib.rs` ring: the whole word, in
    /// code (not a comment), inside `crates/net` or beside `engine_net`.
    fn retired_ring_callers(root: &std::path::Path, net_dir: &std::path::Path) -> Vec<String> {
        let name = ["Roll", "back"].concat();
        let edge = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let mut dirs = vec![root.to_path_buf()];
        let mut hits = Vec::new();
        while let Some(dir) = dirs.pop() {
            for path in std::fs::read_dir(&dir)
                .expect("readable dir")
                .flatten()
                .map(|entry| entry.path())
            {
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    for (n, line) in text.lines().enumerate() {
                        let code = line.split("//").next().unwrap_or_default();
                        let word = code.match_indices(&name).any(|(i, _)| {
                            edge(code[..i].chars().next_back())
                                && edge(code[i + name.len()..].chars().next())
                        });
                        if word && (path.starts_with(net_dir) || code.contains("engine_net")) {
                            hits.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        hits.sort();
        hits
    }

    #[test]
    fn retired_ring_gate_flags_a_caller_in_net_or_through_engine_net() {
        let root = std::env::temp_dir().join(format!("engine_net_qn1_gate_{}", std::process::id()));
        let (net, other) = (root.join("net"), root.join("other"));
        std::fs::create_dir_all(&net).expect("net dir");
        std::fs::create_dir_all(&other).expect("other dir");
        let retired = ["Roll", "back"].concat();
        let files = [
            (net.join("a.rs"), format!("use crate::{retired};\n")),
            (
                other.join("b.rs"),
                format!("let r = engine_net::{retired}::new(3);\n"),
            ),
            (
                other.join("c.rs"),
                format!(
                    "// {retired} in prose\nlet e = {retired}Error::TooFar;\nlet x = {retired}::new(1);\n"
                ),
            ),
        ];
        for (path, text) in &files {
            std::fs::write(path, text).expect("write fixture");
        }
        let hits = retired_ring_callers(&root, &net);
        std::fs::remove_dir_all(&root).expect("cleanup");
        let hits: Vec<String> = hits.iter().map(|h| h.replace('\\', "/")).collect();
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits[0].ends_with("net/a.rs:1"), "{hits:?}");
        assert!(hits[1].ends_with("other/b.rs:1"), "{hits:?}");
    }

    #[test]
    fn one_rollback_path_no_source_names_the_retired_ring() {
        // QN1: `P2pSession` and `netcode::Session` are the rollback paths;
        // the hash-only ring that sat in `lib.rs` is deleted and stays so.
        let net = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let crates = net.parent().expect("crates dir");
        let hits = retired_ring_callers(crates, net);
        assert!(hits.is_empty(), "retired ring named again: {hits:?}");
    }
}
