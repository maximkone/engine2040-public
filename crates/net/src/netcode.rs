//! netcode: rollback (save/load snapshots, repeat-last predict, resim, hash stream, cap 9). Net never owns transport.
//!
//! History bound (ON40): the per-frame hash log holds at most
//! `HISTORY_CAP + pending` entries (`HISTORY_CAP = MAX_ROLLBACK + 1`).
//! PORT(gschup/ggrs@e97e3d2416cc68af2d2876d41180d950c2939b6e:src/input_queue.rs)
//! MIT, shape only reimplemented std-only, zero donor bytes: the donor
//! `discard_confirmed_frames` tail-advance plus `last_requested_frame` guard
//! become an automatic windowed evict here (drop oldest only, never a frame
//! still in the snapshot window) with post-evict reads refusing loudly
//! (`hash_at` -> `None`, `check_remote` -> `UnknownFrame`, `confirm` ->
//! `TooFar`). Resolves the old UNVERIFIED license note.

/// Remote input ledger with sequential push plus confirmed-prefix tail advance.
///
/// One implementation lives with the rollback tape; this re-export lets the
/// small-window session share it instead of mirroring the type.
pub use crate::rollback::InputQueue;
use crate::Rollback;
use std::collections::{BTreeMap, VecDeque};

/// Max rollback window (9: 150 ms at 60 Hz). Resims beyond this refuse `TooFar`.
pub const MAX_ROLLBACK: usize = 9;

/// Bounded history capacity: one hash per windowed snapshot (9 + 1).
/// The live log holds at most this plus buffered future inputs (`pending`).
pub const HISTORY_CAP: usize = MAX_ROLLBACK + 1;

/// Netcode-tick contract pin (buyer-expectation, ideas only): per-tick
/// snapshot byte budget, static-on-change-only split, input-loss path via
/// the landed counters. Rollback math unchanged; this only asserts the
/// landed window in debug builds. Units: bytes, ticks. Determinism: integer
/// compares only, no allocation.
const SNAPSHOT_BUDGET_BYTES: usize = crate::session::MAX_PACKET_BYTES;

/// True when `bytes` fits the per-tick snapshot budget.
fn within_snapshot_budget(bytes: usize) -> bool {
    bytes <= SNAPSHOT_BUDGET_BYTES
}

/// Debug-only window check: every resim inside cap 9, history bounded,
/// prediction counts expose input loss. No-op in release.
fn debug_assert_tick_window<S: Sim>(session: &Session<S>) {
    debug_assert!(session.max_rollback() <= MAX_ROLLBACK);
    debug_assert!(session.history_len() <= HISTORY_CAP + session.pending_len());
    debug_assert!(session.len() <= MAX_ROLLBACK + 1);
}

/// Loud netcode failures: window / horizon / desync refusals change nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetcodeError {
    /// Confirm targets an evicted frame or a resim deeper than the window.
    TooFar {
        /// Requested frame.
        frame: u32,
        /// Oldest frame still held in the window.
        oldest: u32,
    },
    /// Confirm targets a frame past the near-future buffer horizon.
    Future {
        /// Requested frame.
        frame: u32,
        /// Next frame to author.
        current: u32,
    },
    /// Remote hash probe names an authoritatively unknown frame.
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
}

impl std::fmt::Display for NetcodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::TooFar { frame, oldest } => {
                write!(f, "rollback too far: frame {frame}, oldest {oldest}")
            }
            Self::Future { frame, current } => {
                write!(f, "future frame: frame {frame}, current {current}")
            }
            Self::UnknownFrame(frame) => write!(f, "unknown frame {frame}"),
            Self::Desync { frame, local, peer } => write!(
                f,
                "desync at frame {frame}: local {local:#016x} != peer {peer:#016x}"
            ),
        }
    }
}

impl std::error::Error for NetcodeError {}

/// Deterministic sim contract: snapshots via Clone, fixed inputs yield fixed state.
pub trait Sim: Clone {
    fn step(&mut self, inputs: [u32; 2]);
    fn hash(&self) -> u64;
}

/// Channel hint only. Caller maps to transport; net never sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Reliable,
    Unreliable,
    Sequenced,
}

/// Wire-kind to channel-plus-priority table the encoder reads.
/// Consumer: [`Session::packet`] via [`kind_channel`]; session owns the
/// `KIND_*` bytes, netcode owns this hint. Units: kind byte, channel hint,
/// priority class lower-first (combat inputs 0 outrank state 1 outrank
/// snapshot bulk 2). Determinism: integer compares only, no allocation.
/// Other `KIND_*` (courier/lobby/relay) use different magics, out of scope.
pub const KIND_CHANNEL: [(u8, Channel, u8); 3] = [
    (crate::session::KIND_INPUT, Channel::Sequenced, 0),
    (crate::session::KIND_STATE, Channel::Unreliable, 1),
    (crate::session::KIND_SNAPSHOT, Channel::Reliable, 2),
];

/// Encoder lookup into [`KIND_CHANNEL`]; `None` for unknown kinds.
/// Loops the table so table and lookup cannot drift; no allocation.
#[must_use]
pub fn kind_channel(kind: u8) -> Option<(Channel, u8)> {
    for (k, ch, pri) in KIND_CHANNEL {
        if k == kind {
            return Some((ch, pri));
        }
    }
    None
}

/// Wire shape. Transport-owned; net only fills/consumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub frame: u32,
    pub input: u32,
    pub ch: Channel,
}

struct Rec<S> {
    frame: u32,
    before: S,
    local: u32,
    remote: u32,
    predicted: bool,
    hash: u64,
}

/// 2P rollback session. Hash history is bounded (`HISTORY_CAP + pending`);
/// snapshots windowed to MAX_ROLLBACK+1.
pub struct Session<S: Sim> {
    sim: S,
    frame: u32,
    frames: VecDeque<Rec<S>>,
    pending: BTreeMap<u32, u32>,
    history: VecDeque<u64>,
    history_base: u32,
    ring: Rollback,
    last_remote: u32,
    remote_queue: InputQueue,
    max_rollback: usize,
    rollbacks: u64,
    predicted: u64,
}

impl<S: Sim> Session<S> {
    /// New session at frame 0 over `initial`, with an empty history and a
    /// rollback window of `MAX_ROLLBACK` frames.
    pub fn new(initial: S) -> Self {
        Self {
            sim: initial,
            frame: 0,
            frames: VecDeque::new(),
            pending: BTreeMap::new(),
            history: VecDeque::new(),
            history_base: 0,
            ring: Rollback::new(MAX_ROLLBACK),
            last_remote: 0,
            remote_queue: InputQueue::new(),
            max_rollback: 0,
            rollbacks: 0,
            predicted: 0,
        }
    }
    /// Next frame to play.
    pub fn frame(&self) -> u32 {
        self.frame
    }
    /// Frames played so far (same count as [`Self::frame`]).
    pub fn frames_played(&self) -> u32 {
        self.frame
    }
    /// The live simulation state.
    pub fn sim(&self) -> &S {
        &self.sim
    }
    /// Deepest rollback taken so far, in frames.
    pub fn max_rollback(&self) -> usize {
        self.max_rollback
    }
    /// Rollbacks taken so far.
    pub fn rollback_count(&self) -> u64 {
        self.rollbacks
    }
    /// Frames played on a predicted remote input.
    pub fn predicted_count(&self) -> u64 {
        self.predicted
    }
    /// Frames still held in the snapshot window.
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    /// True when the snapshot window is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    /// Hashes still held in the bounded history log.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }
    /// Oldest frame still held in the history log.
    pub fn history_base(&self) -> u32 {
        self.history_base
    }
    /// Buffered future inputs (horizon allowance above `HISTORY_CAP`).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
    /// Remote input ledger for this session (at most 128 slots).
    pub fn remote_queue(&self) -> &InputQueue {
        &self.remote_queue
    }
    /// Mutable remote input ledger (tail advance via `discard_confirmed_frames`).
    pub fn remote_queue_mut(&mut self) -> &mut InputQueue {
        &mut self.remote_queue
    }
    /// Current snapshot.
    pub fn save(&self) -> S {
        self.sim.clone()
    }
    /// Restore current snapshot (caller-owned bytes/state).
    pub fn load(&mut self, snap: S) {
        self.sim = snap;
    }
    /// Snapshot at start of a windowed frame (None once evicted).
    pub fn snapshot(&self, frame: u32) -> Option<S> {
        self.frames
            .iter()
            .find(|r| r.frame == frame)
            .map(|r| r.before.clone())
    }
    /// State hash after `frame`, if that frame is still held in the bounded
    /// history (`None` once evicted past `HISTORY_CAP + pending`).
    pub fn hash_at(&self, frame: u32) -> Option<u64> {
        frame
            .checked_sub(self.history_base)
            .and_then(|i| self.history.get(i as usize).copied())
    }
    /// Bounded per-frame hash stream (desync/peer-compare source over the
    /// held window, oldest first).
    pub fn stream(&self) -> Vec<u64> {
        self.history.iter().copied().collect()
    }
    /// lightyear AssertRemote shape: compare peer hash for a confirmed frame.
    /// Matches answer `Ok(())`; mismatches refuse as Desync; predicted,
    /// evicted or unknown frames refuse as UnknownFrame (ggrs compares only
    /// frames below last-confirmed, so a predicted hash never false-fires).
    pub fn check_remote(&self, frame: u32, peer: u64) -> Result<(), NetcodeError> {
        // Confirmed-only: a frame still on a predicted input is not
        // authoritative yet; comparing it would false-fire Desync on every
        // mispredict. Evicted frames (None) refuse the same way.
        if self.was_predicted(frame) != Some(false) {
            return Err(NetcodeError::UnknownFrame(frame));
        }
        match self.hash_at(frame) {
            None => Err(NetcodeError::UnknownFrame(frame)),
            Some(local) if local == peer => Ok(()),
            Some(local) => Err(NetcodeError::Desync { frame, local, peer }),
        }
    }
    /// Whether `frame` was played on a predicted remote input; `None` once it
    /// left the snapshot window.
    pub fn was_predicted(&self, frame: u32) -> Option<bool> {
        self.frames
            .iter()
            .find(|r| r.frame == frame)
            .map(|r| r.predicted)
    }
    /// Ring passthrough (reused crate::Rollback).
    pub fn resim_from(&self, i: usize) -> Option<u64> {
        self.ring.resim_from(i)
    }
    /// Build an outbound packet shape; the caller transports it.
    pub fn packet(frame: u32, input: u32, ch: Channel) -> Packet {
        // Pin the hint to the table so route and shed order cannot split.
        // Reads via `kind_channel`, so the encoder reads the table.
        debug_assert!(
            [
                crate::session::KIND_INPUT,
                crate::session::KIND_STATE,
                crate::session::KIND_SNAPSHOT,
            ]
            .iter()
            .any(|k| kind_channel(*k).is_some_and(|(c, _)| c == ch)),
            "packet channel must appear in KIND_CHANNEL"
        );
        Packet { frame, input, ch }
    }
    /// Play one frame. Missing remote predicts repeat-last; early pending wins. Returns post hash.
    pub fn advance(&mut self, local: u32, remote: Option<u32>) -> u64 {
        let f = self.frame;
        let (r, pred) = match self.pending.remove(&f).or(remote) {
            Some(v) => {
                self.last_remote = v;
                (v, false)
            }
            None => (self.last_remote, true),
        };
        if pred {
            self.predicted += 1;
        }
        let before = self.sim.clone();
        self.sim.step([local, r]);
        let h = self.sim.hash();
        self.frames.push_back(Rec {
            frame: f,
            before,
            local,
            remote: r,
            predicted: pred,
            hash: h,
        });
        self.history.push_back(h);
        self.ring.advance(h, local ^ r);
        self.remote_queue.push_input(f, r, !pred);
        if self.frames.len() > MAX_ROLLBACK + 1 {
            self.frames.pop_front();
        }
        // GGRS tail-advance, automatic: shed the confirmed oldest first, but
        // never a frame the snapshot window still needs. Steady state pops
        // one history entry per advance, so the log mirrors the window plus
        // any buffered future inputs.
        let oldest_needed = self
            .frames
            .front()
            .map(|r| r.frame)
            .unwrap_or(self.frame + 1);
        while self.history.len() > HISTORY_CAP + self.pending.len()
            && self.history_base < oldest_needed
        {
            self.history.pop_front();
            self.history_base += 1;
        }
        self.frame += 1;
        debug_assert!(within_snapshot_budget(crate::scale::full_wire_len(2)));
        debug_assert_tick_window(self);
        h
    }
    /// Late true input. Confirm, or rollback + resim on mispredict.
    /// Near-future frames buffer (`Ok(0)`); far-future frames refuse with
    /// [`NetcodeError::Future`]; evicted frames refuse with
    /// [`NetcodeError::TooFar`]. Any error changes nothing.
    pub fn confirm(&mut self, frame: u32, input: u32) -> Result<usize, NetcodeError> {
        if frame >= self.frame {
            // Horizon is small (8); on a platform where it cannot fit `u32`
            // the saturating fallback refuses nothing new.
            let horizon = u32::try_from(MAX_ROLLBACK).unwrap_or(u32::MAX);
            if frame - self.frame > horizon {
                return Err(NetcodeError::Future {
                    frame,
                    current: self.frame,
                });
            }
            self.pending.insert(frame, input);
            return Ok(0);
        }
        let oldest = self.frames.front().map(|r| r.frame).unwrap_or(self.frame);
        if frame < oldest {
            return Err(NetcodeError::TooFar { frame, oldest });
        }
        let dist = (self.frame - frame) as usize;
        if dist > MAX_ROLLBACK {
            return Err(NetcodeError::TooFar { frame, oldest });
        }
        let idx = (frame - oldest) as usize;
        if self.frames[idx].remote == input {
            self.frames[idx].predicted = false;
            self.remote_queue.confirm(frame, input);
            return Ok(0);
        }
        self.frames[idx].remote = input;
        self.frames[idx].predicted = false;
        self.remote_queue.confirm(frame, input);
        self.sim = self.frames[idx].before.clone();
        let len = self.frames.len();
        for i in idx..len {
            let (l, r) = (self.frames[i].local, self.frames[i].remote);
            self.sim.step([l, r]);
            let h = self.sim.hash();
            self.frames[i].hash = h;
            let f = self.frames[i].frame;
            if let Some(slot) = f
                .checked_sub(self.history_base)
                .and_then(|o| self.history.get_mut(o as usize))
            {
                *slot = h;
            }
            if i + 1 < len {
                let cur = self.sim.clone();
                self.frames[i + 1].before = cur;
            }
        }
        self.rebuild_ring();
        self.rollbacks += 1;
        self.max_rollback = self.max_rollback.max(dist);
        Ok(dist)
    }
    fn rebuild_ring(&mut self) {
        let mut ring = Rollback::new(MAX_ROLLBACK);
        let n = self.frames.len().min(MAX_ROLLBACK);
        for r in self.frames.iter().skip(self.frames.len() - n) {
            ring.advance(r.hash, r.local ^ r.remote);
        }
        self.ring = ring;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fnv64;

    #[derive(Clone, Default, PartialEq, Eq)]
    struct Toy {
        a: i32,
        b: i32,
    }
    impl Sim for Toy {
        fn step(&mut self, inputs: [u32; 2]) {
            self.a = self.a.wrapping_add(inputs[0] as i32 - 1);
            self.b = self.b.wrapping_add(inputs[1] as i32 - 1);
        }
        fn hash(&self) -> u64 {
            let mut fold = Fnv64::new();
            fold.write_i32(self.a);
            fold.write_i32(self.b);
            fold.finish()
        }
    }
    fn seq(n: usize, k: u32) -> Vec<u32> {
        (0..n).map(|f| (f as u32 * k + 1) % 4).collect()
    }

    #[test]
    fn loss_10pct_converges() {
        let n = 64;
        let (local, remote) = (seq(n, 1), seq(n, 3));
        let mut refs = Toy::default();
        for f in 0..n {
            refs.step([local[f], remote[f]]);
        }
        let mut s = Session::new(Toy::default());
        for f in 0..n {
            let avail = if f % 10 == 9 { None } else { Some(remote[f]) };
            s.advance(local[f], avail);
            if f >= 3 && (f - 3) % 10 == 9 {
                let d = s
                    .confirm((f - 3) as u32, remote[f - 3])
                    .expect("within window");
                assert!(d <= MAX_ROLLBACK, "rollback {d} > cap");
            }
        }
        for (f, &r) in remote.iter().enumerate() {
            if f % 10 == 9 {
                // Re-ack in-window frames; evicted frames refuse (idempotent).
                match s.confirm(f as u32, r) {
                    Ok(d) => assert!(d <= MAX_ROLLBACK, "rollback {d} > cap"),
                    Err(NetcodeError::TooFar { .. }) => {}
                    Err(e) => panic!("unexpected confirm err: {e}"),
                }
            }
        }
        assert_eq!(s.sim.hash(), refs.hash(), "10% loss must converge");
        assert_eq!(s.frames_played() as usize, n);
        // ON40: bounded history, not full-length.
        assert!(
            s.history_len() <= HISTORY_CAP + s.pending_len(),
            "history {} > CAP+pending {}",
            s.history_len(),
            HISTORY_CAP + s.pending_len()
        );
        assert_eq!(s.stream().len(), s.history_len());
        assert_eq!(
            *s.stream().last().unwrap(),
            refs.hash(),
            "tip must converge"
        );
        assert!(
            s.rollback_count() > 0 && s.max_rollback() > 0,
            "must have rolled back"
        );
        assert!(s.max_rollback() <= MAX_ROLLBACK);
        let tip = n as u32 - 1;
        assert!(s.check_remote(tip, s.hash_at(tip).unwrap()).is_ok());
        // Evicted frame 0 refuses loudly (requested-frame guard).
        assert_eq!(s.hash_at(0), None);
        assert_eq!(s.check_remote(0, 0), Err(NetcodeError::UnknownFrame(0)));
    }

    #[test]
    fn rollback_bounded_9() {
        let mut ok = Session::new(Toy::default());
        for f in 0..9u32 {
            ok.advance(f % 4, Some((f * 3 + 1) % 4));
        }
        assert_eq!(ok.confirm(0, 99).unwrap(), 9);
        let mut far = Session::new(Toy::default());
        for f in 0..10u32 {
            far.advance(f % 4, Some((f * 3 + 1) % 4));
        }
        let before = far.stream();
        assert_eq!(
            far.confirm(0, 99),
            Err(NetcodeError::TooFar {
                frame: 0,
                oldest: 0
            })
        );
        assert_eq!(far.stream(), before, "refused confirm changes nothing");
        let horizon = u32::try_from(MAX_ROLLBACK).expect("window fits u32");
        assert_eq!(
            far.confirm(far.frame() + horizon + 1, 1),
            Err(NetcodeError::Future {
                frame: far.frame() + horizon + 1,
                current: far.frame(),
            })
        );
        assert!(ok.max_rollback() <= MAX_ROLLBACK && far.max_rollback() <= MAX_ROLLBACK);
    }

    #[test]
    fn wan_61_tick_rollback_refused_against_9_tick_cap() {
        // 150 ms at 60 Hz is 9 ticks (anim `RTT_TICKS_150MS`, wan_soak
        // `RTT_TICKS`); a 61-tick-old correction exceeds the 9-tick authority
        // window and must refuse `TooFar` (resync path), never resim.
        assert_eq!(MAX_ROLLBACK, 9, "150 ms at 60 Hz is 9 ticks");
        let mut s = Session::new(Toy::default());
        for f in 0..70u32 {
            s.advance(f % 4, Some((f * 3 + 1) % 4));
        }
        assert_eq!(s.frame(), 70);
        // Frame 9 at tip 70 is 61 ticks old (70 - 9 = 61): far past the
        // 9-tick window, so the WAN rollback refuses loudly.
        let before = s.stream();
        let err = s
            .confirm(9, 99)
            .expect_err("61-tick WAN rollback must refuse");
        assert!(
            matches!(err, NetcodeError::TooFar { .. }),
            "61-tick rollback must be TooFar, got {err}"
        );
        assert_eq!(s.stream(), before, "refused confirm changes nothing");
        assert!(s.max_rollback() <= MAX_ROLLBACK);
    }

    #[test]
    fn predict_repeat_last() {
        let mut s = Session::new(Toy::default());
        s.advance(2, Some(3));
        s.advance(2, None);
        let mut t = Toy::default();
        t.step([2, 3]);
        t.step([2, 3]);
        assert_eq!(s.sim.hash(), t.hash());
        assert_eq!(s.predicted_count(), 1);
        assert_eq!(s.was_predicted(1), Some(true));
        assert_eq!(s.confirm(1, 3), Ok(0));
        assert_eq!(s.rollback_count(), 0);
        assert_eq!(s.confirm(s.frame() + 1, 1), Ok(0));
        assert_eq!(
            s.confirm(99, 1),
            Err(NetcodeError::Future {
                frame: 99,
                current: s.frame(),
            })
        );
        let h = s.advance(0, None);
        assert_eq!(s.was_predicted(2), Some(true)); // missing remote => repeat-last predict
        assert_eq!(h, s.hash_at(2).unwrap());
    }

    #[test]
    fn hash_stream_deterministic() {
        let run = || {
            let mut s = Session::new(Toy::default());
            for f in 0..16u32 {
                s.advance(f % 4, Some((f + 1) % 4));
            }
            s.stream()
        };
        assert_eq!(run(), run());
        let mut other = Session::new(Toy::default());
        for f in 0..16u32 {
            other.advance(f % 4, Some((f + 2) % 4));
        }
        assert_ne!(run(), other.stream());
        // Tip frame is held; frame 0 is evicted past the bound (guard).
        let good = other.hash_at(15).expect("tip frame held");
        assert!(other.check_remote(15, good).is_ok());
        let before = other.stream();
        assert_eq!(
            other.check_remote(15, 0xdead),
            Err(NetcodeError::Desync {
                frame: 15,
                local: good,
                peer: 0xdead,
            })
        );
        assert_eq!(other.stream(), before, "failed check must change nothing");
        assert_eq!(other.hash_at(0), None, "evicted frame reads None");
        assert_eq!(other.check_remote(0, 0), Err(NetcodeError::UnknownFrame(0)));
        assert_eq!(
            other.check_remote(999, 0),
            Err(NetcodeError::UnknownFrame(999))
        );
    }

    #[test]
    fn on40_history_bounded_cap_plus_pending_bursty_loss() {
        // ON40 accept: 64-frame bursty-loss tape (every 5th remote dropped,
        // late confirm +2f), history <= CAP+pending, fold/tip/digest
        // identical x2, max_rollback <= 8.
        fn run_once() -> (Vec<u64>, u64, u64, u64, usize, usize) {
            let n = 64;
            let (local, remote) = (seq(n, 1), seq(n, 3));
            let mut s = Session::new(Toy::default());
            for f in 0..n {
                let avail = if f % 5 == 4 { None } else { Some(remote[f]) };
                s.advance(local[f], avail);
                if f >= 2 && (f - 2) % 5 == 4 {
                    s.confirm((f - 2) as u32, remote[f - 2])
                        .expect("late input inside window");
                }
            }
            for (f, &r) in remote.iter().enumerate() {
                if s.was_predicted(f as u32) == Some(true) {
                    match s.confirm(f as u32, r) {
                        Ok(_) => {}
                        Err(NetcodeError::TooFar { .. }) => {}
                        Err(e) => panic!("unexpected confirm err: {e}"),
                    }
                }
            }
            assert!(
                s.history_len() <= HISTORY_CAP + s.pending_len(),
                "history {} > CAP+pending {}",
                s.history_len(),
                HISTORY_CAP + s.pending_len()
            );
            assert!(s.max_rollback() <= MAX_ROLLBACK);
            let stream = s.stream();
            let tip = *stream.last().unwrap();
            let mut f = Fnv64::new();
            for h in &stream {
                f.write_u64(*h);
            }
            let fold = f.finish();
            let mut d = Fnv64::new();
            d.write_u64(fold);
            d.write_u64(tip);
            d.write_u64(s.rollback_count());
            let digest = d.finish();
            (stream, fold, tip, digest, s.max_rollback(), s.history_len())
        }
        let (s1, fold1, tip1, dig1, rb1, len1) = run_once();
        let (s2, fold2, tip2, dig2, rb2, len2) = run_once();
        assert_eq!(s1, s2, "tape must replay identically x2");
        assert_eq!(
            (fold1, tip1, dig1),
            (fold2, tip2, dig2),
            "fold/tip/digest x2"
        );
        assert_eq!((rb1, len1), (rb2, len2));
        assert!(
            len1 <= HISTORY_CAP + 8,
            "64-frame tape holds <= CAP+pending"
        );
        // True-input reference still converges at the tip.
        let n = 64;
        let (local, remote) = (seq(n, 1), seq(n, 3));
        let mut refs = Toy::default();
        for f in 0..n {
            refs.step([local[f], remote[f]]);
        }
        assert_eq!(tip1, refs.hash(), "tip equals true sim");
    }

    #[test]
    fn on40_history_flat_over_10k_frames() {
        // ON40 accept: a 10k-frame bursty-loss run keeps its buffers flat
        // (bytes at 1k and 10k frames printed), history <= CAP+pending, and
        // the replay digest is identical x2.
        fn run(frames: u32) -> (u64, usize, usize) {
            let remote = |f: u32| (f * 3 + 1) % 4;
            let mut s = Session::new(Toy::default());
            for f in 0..frames {
                let avail = if f % 5 == 4 { None } else { Some(remote(f)) };
                s.advance((f * 7 + 1) % 4, avail);
                if f >= 2 && (f - 2) % 5 == 4 {
                    s.confirm(f - 2, remote(f - 2))
                        .expect("late input inside window");
                }
            }
            let bytes = s.history.capacity() * std::mem::size_of::<u64>()
                + s.frames.capacity() * std::mem::size_of::<Rec<Toy>>();
            let mut fold = Fnv64::new();
            for h in s.stream() {
                fold.write_u64(h);
            }
            fold.write_u64(s.rollback_count());
            (fold.finish(), bytes, s.history_len())
        }
        let (_, bytes_1k, _) = run(1_000);
        let (digest, bytes_10k, len_10k) = run(10_000);
        let (digest_again, _, _) = run(10_000);
        eprintln!("on40 history buffers: 1k frames {bytes_1k} B, 10k frames {bytes_10k} B");
        assert_eq!(bytes_10k, bytes_1k, "buffers flat from 1k to 10k frames");
        assert!(
            len_10k <= HISTORY_CAP + 8,
            "history {len_10k} > CAP+pending bound"
        );
        assert_eq!(digest, digest_again, "replay digest x2");
    }

    #[test]
    fn discard_confirmed_tail_advances_past_confirmed_prefix() {
        let mut s = Session::new(Toy::default());
        for f in 0..4u32 {
            s.advance(f % 4, Some((f * 3 + 1) % 4));
        }
        assert_eq!(s.remote_queue().len(), 4);
        assert_eq!(s.remote_queue_mut().discard_confirmed_frames(4), 4);
        assert!(s.remote_queue().is_empty());
        assert_eq!(s.remote_queue().first_frame(), None);
    }

    #[test]
    fn discard_confirmed_stops_at_predicted_slot() {
        let mut s = Session::new(Toy::default());
        s.advance(1, Some(2));
        s.advance(1, None);
        assert_eq!(s.remote_queue().len(), 2);
        assert!(!s.remote_queue().is_confirmed(1));
        assert_eq!(s.remote_queue_mut().discard_confirmed_frames(2), 1);
        assert_eq!(s.remote_queue().first_frame(), Some(1));
        s.confirm(1, 2).expect("confirm inside window");
        assert_eq!(s.remote_queue_mut().discard_confirmed_frames(2), 1);
        assert!(s.remote_queue().is_empty());
    }

    #[test]
    fn save_load_roundtrip() {
        let mut s = Session::new(Toy::default());
        s.advance(2, Some(2));
        let (snap, h0) = (s.save(), s.sim.hash());
        s.advance(3, Some(0));
        assert_ne!(s.sim.hash(), h0);
        s.load(snap);
        assert_eq!(s.sim.hash(), h0);
        assert!(s.snapshot(0).is_some());
        let p = Session::<Toy>::packet(7, 2, Channel::Reliable);
        assert_eq!((p.frame, p.input, p.ch), (7, 2, Channel::Reliable));
    }

    #[test]
    fn kind_table_reads_every_kind() {
        // Every session-wire KIND_* resolves; unknown is None; combat inputs
        // outrank state outranks snapshot bulk (lower-first).
        let (ich, ip) = kind_channel(crate::session::KIND_INPUT).expect("input maps");
        let (sch, sp) = kind_channel(crate::session::KIND_STATE).expect("state maps");
        let (nch, np) = kind_channel(crate::session::KIND_SNAPSHOT).expect("snapshot maps");
        assert_eq!((ich, ip), (Channel::Sequenced, 0));
        assert_eq!((sch, sp), (Channel::Unreliable, 1));
        assert_eq!((nch, np), (Channel::Reliable, 2));
        assert!(ip < sp && sp < np, "inputs outrank state outranks bulk");
        assert_eq!(kind_channel(0xFF), None);
        // Encoder reads: packet pins every table channel; twice identical.
        assert_eq!(kind_channel(crate::session::KIND_INPUT), Some((ich, ip)));
        let p = Session::<Toy>::packet(7, 2, ich);
        assert_eq!(p.ch, ich);
    }

    #[test]
    fn det1a_snapshot_restore_readvance_checksums_equal() {
        // DET1.a SyncTest (ggrs synctest shape: roll back and re-run the
        // checked frames every frame and compare checksums, with no
        // network): snapshot at T, advance N, restore the snapshot,
        // re-advance the same N inputs, compare checksums. Restore plus
        // re-advance must reproduce the first pass exactly, or rollback
        // would resimulate from wrong states (replay and rollback must
        // re-run identical ticks).
        const T: usize = 16;
        const N: usize = 8;
        let (local, remote) = (seq(T + N, 1), seq(T + N, 3));
        let mut s = Session::new(Toy::default());
        for f in 0..T {
            s.advance(local[f], Some(remote[f]));
        }
        // Snapshot at T: the sim state plus its checksum.
        let snap = s.save();
        let hash_at_t = s.sim.hash();
        // Advance N past the snapshot, keeping the first-pass checksums.
        let mut first = Vec::with_capacity(N);
        for f in T..T + N {
            first.push(s.advance(local[f], Some(remote[f])));
        }
        let first_tip = *first.last().expect("N checksums");
        // Restore the snapshot and re-advance the same N inputs.
        s.load(snap);
        assert_eq!(s.sim.hash(), hash_at_t, "restore must return the T state");
        let mut second = Vec::with_capacity(N);
        for f in T..T + N {
            second.push(s.advance(local[f], Some(remote[f])));
        }
        assert_eq!(second, first, "re-advance must reproduce every checksum");
        assert_eq!(s.sim.hash(), first_tip, "tip must match the first pass");
    }

    #[test]
    fn det1b_confirmed_only_compare_waits_for_confirm() {
        // DET1.b companion (ggrs confirmed-frame rule: only frames below
        // last-confirmed compare; predicted frames never report Desync).
        // A predicted frame must refuse as UnknownFrame even when the peer
        // hash matches the predicted hash, or a mispredict would false-fire.
        let mut s = Session::new(Toy::default());
        // Frame 0 confirmed (true remote), frame 1 predicted (missing remote).
        s.advance(1, Some(2));
        s.advance(1, None);
        assert_eq!(s.was_predicted(0), Some(false));
        assert_eq!(s.was_predicted(1), Some(true));
        let confirmed_hash = s.hash_at(0).expect("confirmed held");
        assert!(s.check_remote(0, confirmed_hash).is_ok());
        assert_eq!(
            s.check_remote(0, confirmed_hash.wrapping_add(1)),
            Err(NetcodeError::Desync {
                frame: 0,
                local: confirmed_hash,
                peer: confirmed_hash.wrapping_add(1),
            })
        );
        // Predicted frame 1 refuses either peer value as UnknownFrame.
        let predicted_hash = s.hash_at(1).expect("predicted held");
        assert_eq!(
            s.check_remote(1, predicted_hash),
            Err(NetcodeError::UnknownFrame(1))
        );
        assert_eq!(
            s.check_remote(1, predicted_hash.wrapping_add(1)),
            Err(NetcodeError::UnknownFrame(1))
        );
        // The true input matches the repeat-last prediction, so confirm
        // marks the frame confirmed with no resim.
        s.confirm(1, 2).expect("confirm inside window");
        assert_eq!(s.was_predicted(1), Some(false));
        let now = s.hash_at(1).expect("confirmed after");
        assert!(s.check_remote(1, now).is_ok());
    }

    #[test]
    fn det1b_two_clients_same_seed_same_tape_same_bytes() {
        // DET1.b loopback: same seed plus same tape gives the same bytes at
        // both ends. Two sessions from the same initial, fed the same tapes,
        // hold identical hash streams and identical folds.
        const N: usize = 32;
        let (local, remote) = (seq(N, 1), seq(N, 3));
        fn run_once(local: &[u32], remote: &[u32]) -> (Vec<u64>, u64) {
            let mut s = Session::new(Toy::default());
            for f in 0..local.len() {
                s.advance(local[f], Some(remote[f]));
            }
            let stream = s.stream();
            let mut fold = Fnv64::new();
            for h in &stream {
                fold.write_u64(*h);
            }
            (stream, fold.finish())
        }
        let (a_stream, a_fold) = run_once(&local, &remote);
        let (b_stream, b_fold) = run_once(&local, &remote);
        assert_eq!(
            a_stream, b_stream,
            "same tape must give same bytes at both ends"
        );
        assert_eq!(a_fold, b_fold, "fold x2");
    }
}
