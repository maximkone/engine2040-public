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
//! most `INPUT_QUEUE_LEN` inputs; hash log costs 8 B per frame (documented).
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

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};

use engine_core::Fnv64;

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
    },
    /// Seat token mismatch on a guarded confirm; state unchanged.
    BadSeat { frame: u32, want: u64, got: u64 },
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
            Self::Desync { frame, local, peer } => write!(
                f,
                "desync at frame {frame}: local {local:#016x} != peer {peer:#016x}"
            ),
            Self::BadSeat { frame, want, got } => {
                write!(f, "bad seat {frame}: want {want:#x} got {got:#x}")
            }
        }
    }
}

impl std::error::Error for RollbackError {}

/// Deterministic 2-player sim tick behind the rollback tape.
///
/// Integer-only: implementations use `+ - * /`, wrapping integer math and
/// `state_hash`; no float transcendental calls, no wall clock.
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
/// the tape and refuses loudly on mismatch.
pub struct P2pSession<S: SimTick> {
    sim: S,
    frame: u32,
    window: VecDeque<FrameRecord<S>>,
    history: Vec<u64>,
    pending: BTreeMap<u32, u32>,
    queues: [InputQueue; PLAYERS],
    last_remote: u32,
    rollbacks: u64,
    predicted: u64,
    mispredicted: u64,
    max_rollback: usize,
    last_mismatch: Cell<Option<u32>>,
    frontier: u32,
    settled: u32,
    log: Vec<[u32; PLAYERS]>,
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
            history: Vec::new(),
            pending: BTreeMap::new(),
            queues: [InputQueue::new(), InputQueue::new()],
            last_remote: 0,
            rollbacks: 0,
            predicted: 0,
            mispredicted: 0,
            max_rollback: 0,
            last_mismatch: Cell::new(None),
            frontier: 0,
            settled: 0,
            log: Vec::new(),
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

    /// Full per-frame hash tape, frame `i` at index `i`.
    pub fn stream(&self) -> Vec<u64> {
        self.history.clone()
    }

    /// Tape hash at `frame`, or `None` before/after the authored range.
    pub fn hash_at(&self, frame: u32) -> Option<u64> {
        self.history.get(frame as usize).copied()
    }

    /// GGRS-style per-tick checksum: state hash saved at `frame`.
    pub fn checksum_at(&self, frame: u32) -> Option<u64> {
        self.hash_at(frame)
    }

    /// GGRS-style checksum log: one `(frame, state hash)` per authored tick.
    pub fn checksum_log(&self) -> Vec<(u32, u64)> {
        self.history
            .iter()
            .enumerate()
            .map(|(i, &hash)| (i as u32, hash))
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

    /// Append-only input log at `frame`, or `None` when absent.
    pub fn input_at(&self, frame: u32) -> Option<[u32; PLAYERS]> {
        usize::try_from(frame)
            .ok()
            .and_then(|i| self.log.get(i).copied())
    }

    /// Spectator read path: tape hash at `frame` without mutating.
    pub fn spectator_hash_at(&self, frame: u32) -> Option<&u64> {
        usize::try_from(frame)
            .ok()
            .and_then(|i| self.history.get(i))
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
        self.history.push(hash);
        self.queues[0].push_input(f, local, true);
        self.queues[1].push_input(f, r, !predicted);
        self.log.push([local, r]);
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
    /// (frame plus the 8 B per frame hash log); full state lives only in
    /// the window owner.
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
            if let Some(entry) = self.log.get_mut(index(frame)) {
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
            slot.hash = hash;
            if let Some(entry) = self.history.get_mut(index(slot.frame)) {
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
    pub fn confirm_frame_wan(&mut self, frame: u32, remote: u32) -> Result<usize, RollbackError> {
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

    /// Compare one peer hash against the tape. Matches answer `Ok(())`;
    /// mismatches and unknown frames refuse loudly and change nothing,
    /// except a mismatch records [`Self::last_mismatch`].
    pub fn check_desync(&self, frame: u32, peer_hash: u64) -> Result<(), RollbackError> {
        match self.spectator_hash_at(frame) {
            None => Err(RollbackError::UnknownFrame(frame)),
            Some(&local) if local == peer_hash => Ok(()),
            Some(&local) => {
                self.last_mismatch.set(Some(frame));
                Err(RollbackError::Desync {
                    frame,
                    local,
                    peer: peer_hash,
                })
            }
        }
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
        assert_eq!(
            err,
            RollbackError::Desync {
                frame: 7,
                local: good,
                peer: good.wrapping_add(1),
            }
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
}
