//! feed: tick-stamped single-writer command queue from the game thread to
//! the audio callback.
//!
//! A fixed ring of atomic slots with one producer ([`FeedTx`], `&mut self`
//! to write, not `Clone`) and one consumer ([`FeedRx`]). Neither side locks
//! and neither allocates after construction: the producer fills a slot with
//! relaxed stores and publishes it with a release store of the tail; the
//! consumer acquires the tail, reads the slot and releases the head so the
//! producer may reuse it. A full ring refuses the command and counts it
//! ([`FeedStats::dropped_full`]) instead of blocking, so the sim tick never
//! waits on the device. Slots are plain `AtomicU32`s, so the queue needs no
//! `unsafe`. Gems 2 29.2 (single writer, single reader) and Rust Atomics
//! and Locks ch. 5 (channel design, release/acquire hand-off).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::cue::{CueBank, CueId, Group};
use crate::music::Tension;

/// Commands the default ring holds: far more than one tick emits, so a full
/// ring means the callback has stalled, not that the game is busy.
pub const FEED_CAPACITY: usize = 1024;
/// Largest ring [`feed`] will build.
const MAX_FEED_CAPACITY: usize = 1 << 16;
/// `u32` words per slot.
const SLOT_WORDS: usize = 8;

const TAG_PLAY: u32 = 1;
const TAG_PLAY_AT: u32 = 2;
const TAG_LISTENER: u32 = 3;
const TAG_STOP: u32 = 4;
const TAG_TENSION: u32 = 5;

/// One cue play request, tick-stamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioCommand {
    /// Fixed tick the command was emitted on.
    pub tick: u32,
    /// Cue to play.
    pub cue: CueId,
    /// Play level, `0.0..=1.0` (sanitized by the mixer).
    pub gain: f32,
    /// Source position `[east, up, south]` in centimeters; `None` plays the
    /// cue flat.
    pub pos_cm: Option<[i32; 3]>,
}

/// Everything the game thread can tell the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Start a cue.
    Play(AudioCommand),
    /// Move the listener (the camera).
    Listener {
        /// Fixed tick the command was emitted on.
        tick: u32,
        /// Listener position `[east, up, south]` in centimeters.
        pos_cm: [i32; 3],
        /// Facing in degrees clockwise from north.
        yaw_deg: i32,
    },
    /// Stop every voice of a cue (loops).
    Stop {
        /// Fixed tick the command was emitted on.
        tick: u32,
        /// Cue to stop.
        cue: CueId,
    },
    /// Ask the music bed for a tension level; it switches at the next bar line.
    Tension {
        /// Fixed tick the command was emitted on.
        tick: u32,
        /// Level to move to.
        level: Tension,
    },
}

impl Command {
    /// Tick the command was stamped with.
    pub fn tick(&self) -> u32 {
        match *self {
            Self::Play(play) => play.tick,
            Self::Listener { tick, .. } | Self::Stop { tick, .. } | Self::Tension { tick, .. } => {
                tick
            }
        }
    }
}

/// Why a command was not queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedError {
    /// The ring is full; the command was counted as dropped.
    Full,
    /// The audio side is gone (device closed or lost); rebuild the pair.
    Closed,
    /// The cue name is not in the bank; counted in [`FeedStats::unknown_cue`].
    UnknownCue,
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "feed: ring full, command dropped"),
            Self::Closed => write!(f, "feed: audio side closed"),
            Self::UnknownCue => write!(f, "feed: unknown cue name"),
        }
    }
}

impl std::error::Error for FeedError {}

/// Counters both sides publish; read from the game thread, never reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedStats {
    /// Commands queued.
    pub sent: u64,
    /// Commands refused because the ring was full.
    pub dropped_full: u64,
    /// Plays refused because the cue name was unknown.
    pub unknown_cue: u64,
    /// Commands the audio thread applied.
    pub applied: u64,
    /// Tick stamp of the last applied command.
    pub last_applied_tick: u32,
    /// Voices stolen by priority or group limit.
    pub stolen: u64,
    /// Plays the mixer refused (group muted, nothing weaker to steal).
    pub rejected: u64,
    /// Plays skipped beyond their falloff max.
    pub culled: u64,
}

/// One queue entry: `SLOT_WORDS` atomic words.
struct Slot([AtomicU32; SLOT_WORDS]);

/// Keeps the two indices on separate cache lines.
#[repr(align(64))]
struct Padded<T>(T);

/// Shared state: the ring plus the counters.
struct Ring {
    slots: Box<[Slot]>,
    mask: usize,
    /// Next slot the consumer reads (counts up, wraps at `usize::MAX`).
    head: Padded<AtomicUsize>,
    /// Next slot the producer writes.
    tail: Padded<AtomicUsize>,
    closed: AtomicBool,
    sent: AtomicU64,
    dropped_full: AtomicU64,
    unknown_cue: AtomicU64,
    applied: AtomicU64,
    last_applied_tick: AtomicU32,
    stolen: AtomicU64,
    rejected: AtomicU64,
    culled: AtomicU64,
}

fn word_of(v: i32) -> u32 {
    u32::from_ne_bytes(v.to_ne_bytes())
}

fn int_of(w: u32) -> i32 {
    i32::from_ne_bytes(w.to_ne_bytes())
}

impl Slot {
    fn store(&self, cmd: Command) {
        let words: [u32; SLOT_WORDS] = match cmd {
            Command::Play(p) => {
                let (tag, pos) = match p.pos_cm {
                    Some(pos) => (TAG_PLAY_AT, pos),
                    None => (TAG_PLAY, [0; 3]),
                };
                [
                    tag,
                    u32::from(p.cue.0),
                    p.tick,
                    p.gain.to_bits(),
                    word_of(pos[0]),
                    word_of(pos[1]),
                    word_of(pos[2]),
                    0,
                ]
            }
            Command::Listener {
                tick,
                pos_cm,
                yaw_deg,
            } => [
                TAG_LISTENER,
                0,
                tick,
                0,
                word_of(pos_cm[0]),
                word_of(pos_cm[1]),
                word_of(pos_cm[2]),
                word_of(yaw_deg),
            ],
            Command::Stop { tick, cue } => [TAG_STOP, u32::from(cue.0), tick, 0, 0, 0, 0, 0],
            Command::Tension { tick, level } => {
                [TAG_TENSION, u32::from(level.level()), tick, 0, 0, 0, 0, 0]
            }
        };
        for (cell, word) in self.0.iter().zip(words) {
            cell.store(word, Ordering::Relaxed);
        }
    }

    /// Decode the slot; `None` for a tag no writer produces.
    fn load(&self) -> Option<Command> {
        let w = |i: usize| self.0[i].load(Ordering::Relaxed);
        let cue = || u16::try_from(w(1)).ok().map(CueId);
        let pos = [int_of(w(4)), int_of(w(5)), int_of(w(6))];
        match w(0) {
            tag @ (TAG_PLAY | TAG_PLAY_AT) => Some(Command::Play(AudioCommand {
                tick: w(2),
                cue: cue()?,
                gain: f32::from_bits(w(3)),
                pos_cm: (tag == TAG_PLAY_AT).then_some(pos),
            })),
            TAG_LISTENER => Some(Command::Listener {
                tick: w(2),
                pos_cm: pos,
                yaw_deg: int_of(w(7)),
            }),
            TAG_STOP => Some(Command::Stop {
                tick: w(2),
                cue: cue()?,
            }),
            TAG_TENSION => Some(Command::Tension {
                tick: w(2),
                level: Tension::from_level(u8::try_from(w(1)).ok()?)?,
            }),
            _ => None,
        }
    }
}

/// Producer end: owned by the game thread. Not `Clone`, and writing takes
/// `&mut self`, so there is exactly one writer.
pub struct FeedTx {
    ring: Arc<Ring>,
    bank: Arc<CueBank>,
}

/// Consumer end: owned by the audio engine on the callback thread.
pub struct FeedRx {
    ring: Arc<Ring>,
}

/// Build a feed holding at least `capacity` commands (rounded up to a power
/// of two, between 2 and 65536). `bank` resolves cue names on the producer
/// side so the audio thread only handles ids.
pub fn feed(capacity: usize, bank: Arc<CueBank>) -> (FeedTx, FeedRx) {
    let capacity = capacity.clamp(2, MAX_FEED_CAPACITY).next_power_of_two();
    let slots: Box<[Slot]> = (0..capacity)
        .map(|_| Slot(std::array::from_fn(|_| AtomicU32::new(0))))
        .collect();
    let ring = Arc::new(Ring {
        slots,
        mask: capacity - 1,
        head: Padded(AtomicUsize::new(0)),
        tail: Padded(AtomicUsize::new(0)),
        closed: AtomicBool::new(false),
        sent: AtomicU64::new(0),
        dropped_full: AtomicU64::new(0),
        unknown_cue: AtomicU64::new(0),
        applied: AtomicU64::new(0),
        last_applied_tick: AtomicU32::new(0),
        stolen: AtomicU64::new(0),
        rejected: AtomicU64::new(0),
        culled: AtomicU64::new(0),
    });
    (
        FeedTx {
            ring: Arc::clone(&ring),
            bank,
        },
        FeedRx { ring },
    )
}

impl Ring {
    fn snapshot(&self) -> FeedStats {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        FeedStats {
            sent: get(&self.sent),
            dropped_full: get(&self.dropped_full),
            unknown_cue: get(&self.unknown_cue),
            applied: get(&self.applied),
            last_applied_tick: self.last_applied_tick.load(Ordering::Relaxed),
            stolen: get(&self.stolen),
            rejected: get(&self.rejected),
            culled: get(&self.culled),
        }
    }
}

impl FeedTx {
    /// Queue `cmd`. Never blocks and never allocates.
    ///
    /// # Errors
    ///
    /// [`FeedError::Full`] when the ring has no free slot (counted),
    /// [`FeedError::Closed`] when the audio side was dropped.
    pub fn send(&mut self, cmd: Command) -> Result<(), FeedError> {
        let ring = &*self.ring;
        if ring.closed.load(Ordering::Acquire) {
            return Err(FeedError::Closed);
        }
        let tail = ring.tail.0.load(Ordering::Relaxed);
        let head = ring.head.0.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= ring.slots.len() {
            ring.dropped_full.fetch_add(1, Ordering::Relaxed);
            return Err(FeedError::Full);
        }
        ring.slots[tail & ring.mask].store(cmd);
        ring.tail.0.store(tail.wrapping_add(1), Ordering::Release);
        ring.sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Queue a play of the cue called `cue`. A positional cue given
    /// `pos_cm` (`[east, up, south]` cm) is placed against the listener;
    /// `None` plays it flat.
    ///
    /// # Errors
    ///
    /// [`FeedError::UnknownCue`] (counted) for a name the bank lacks, else
    /// as [`Self::send`].
    pub fn play(
        &mut self,
        tick: u32,
        cue: &str,
        gain: f32,
        pos_cm: Option<[i32; 3]>,
    ) -> Result<(), FeedError> {
        let id = self.resolve(cue)?;
        self.send(Command::Play(AudioCommand {
            tick,
            cue: id,
            gain,
            pos_cm,
        }))
    }

    /// Queue a listener move: call once per frame with the camera.
    ///
    /// # Errors
    ///
    /// As [`Self::send`].
    pub fn listener(&mut self, tick: u32, pos_cm: [i32; 3], yaw_deg: i32) -> Result<(), FeedError> {
        self.send(Command::Listener {
            tick,
            pos_cm,
            yaw_deg,
        })
    }

    /// Queue a stop of every voice of the cue called `cue`.
    ///
    /// # Errors
    ///
    /// As [`Self::play`].
    pub fn stop(&mut self, tick: u32, cue: &str) -> Result<(), FeedError> {
        let id = self.resolve(cue)?;
        self.send(Command::Stop { tick, cue: id })
    }

    /// Queue a tension level for the music bed: call when the director's
    /// level changes. The bed switches at its next bar line.
    ///
    /// # Errors
    ///
    /// As [`Self::send`].
    pub fn tension(&mut self, tick: u32, level: Tension) -> Result<(), FeedError> {
        self.send(Command::Tension { tick, level })
    }

    /// Queue a one-shot stinger (a cue in [`Group::Stinger`]) that ducks the
    /// music bed while it sounds.
    ///
    /// # Errors
    ///
    /// [`FeedError::UnknownCue`] (counted) when `name` is not a stinger cue,
    /// else as [`Self::send`].
    pub fn stinger(&mut self, tick: u32, name: &str) -> Result<(), FeedError> {
        let id = self.resolve(name)?;
        if self
            .bank
            .cue(id)
            .is_none_or(|cue| cue.group != Group::Stinger)
        {
            self.ring.unknown_cue.fetch_add(1, Ordering::Relaxed);
            return Err(FeedError::UnknownCue);
        }
        self.send(Command::Play(AudioCommand {
            tick,
            cue: id,
            gain: 1.0,
            pos_cm: None,
        }))
    }

    /// Name to id, counting a miss so a typo does not fail silently.
    fn resolve(&self, cue: &str) -> Result<CueId, FeedError> {
        self.bank.id(cue).ok_or_else(|| {
            self.ring.unknown_cue.fetch_add(1, Ordering::Relaxed);
            FeedError::UnknownCue
        })
    }

    /// The bank names resolve against.
    pub fn bank(&self) -> &Arc<CueBank> {
        &self.bank
    }

    /// Counters from both sides.
    pub fn stats(&self) -> FeedStats {
        self.ring.snapshot()
    }

    /// True once the audio side has been dropped.
    pub fn is_closed(&self) -> bool {
        self.ring.closed.load(Ordering::Acquire)
    }
}

impl FeedRx {
    /// Next queued command in send order, `None` when empty. Never blocks
    /// and never allocates.
    pub fn pop(&mut self) -> Option<Command> {
        let ring = &*self.ring;
        loop {
            let head = ring.head.0.load(Ordering::Relaxed);
            let tail = ring.tail.0.load(Ordering::Acquire);
            if head == tail {
                return None;
            }
            let cmd = ring.slots[head & ring.mask].load();
            ring.head.0.store(head.wrapping_add(1), Ordering::Release);
            if cmd.is_some() {
                return cmd;
            }
        }
    }

    /// Publish what the audio side did, for the game thread's [`FeedStats`].
    pub(crate) fn publish(&self, applied: u64, last_tick: u32, mixer: MixCounts) {
        let ring = &*self.ring;
        ring.applied.store(applied, Ordering::Relaxed);
        ring.last_applied_tick.store(last_tick, Ordering::Relaxed);
        ring.stolen.store(mixer.stolen, Ordering::Relaxed);
        ring.rejected.store(mixer.rejected, Ordering::Relaxed);
        ring.culled.store(mixer.culled, Ordering::Relaxed);
    }
}

/// Mixer counters the engine hands to [`FeedRx::publish`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MixCounts {
    pub(crate) stolen: u64,
    pub(crate) rejected: u64,
    pub(crate) culled: u64,
}

impl Drop for FeedRx {
    fn drop(&mut self) {
        self.ring.closed.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::{Clip, Cue, Group, Spatial};
    use crate::dsp::Bus;
    use crate::Pcm;

    fn bank() -> Arc<CueBank> {
        let cue = |name: &str| Cue {
            name: name.to_string(),
            variants: vec![Clip::from_pcm(&Pcm::new(8_000, 1, vec![0.5; 8]))],
            spatial: Spatial::Flat,
            group: Group::Impact,
            looped: false,
            priority: 1,
            bus: Bus::Sfx,
            gain: 1.0,
            ducks_music: false,
        };
        let mut sting = cue("s");
        sting.group = Group::Stinger;
        sting.ducks_music = true;
        Arc::new(CueBank::new(vec![cue("a"), cue("b"), sting]).expect("three valid cues"))
    }

    fn play(tick: u32) -> Command {
        Command::Play(AudioCommand {
            tick,
            cue: CueId(1),
            gain: 0.5,
            pos_cm: None,
        })
    }

    #[test]
    fn every_command_shape_round_trips_bit_exact() {
        let (mut tx, mut rx) = feed(8, bank());
        let cmds = [
            play(0),
            Command::Play(AudioCommand {
                tick: u32::MAX,
                cue: CueId(u16::MAX),
                gain: f32::NAN,
                pos_cm: Some([i32::MIN, -7, i32::MAX]),
            }),
            Command::Listener {
                tick: 9,
                pos_cm: [-1, 2, -3],
                yaw_deg: -450,
            },
            Command::Stop {
                tick: 4,
                cue: CueId(0),
            },
        ];
        for cmd in cmds {
            tx.send(cmd).expect("room");
        }
        for want in cmds {
            let got = rx.pop().expect("queued");
            match (got, want) {
                // NaN gain: compare by bits, everything else by value.
                (Command::Play(g), Command::Play(w)) => {
                    assert_eq!(g.gain.to_bits(), w.gain.to_bits());
                    assert_eq!((g.tick, g.cue, g.pos_cm), (w.tick, w.cue, w.pos_cm));
                }
                _ => assert_eq!(got, want),
            }
            assert_eq!(got.tick(), want.tick());
        }
        assert!(rx.pop().is_none());
    }

    #[test]
    fn full_ring_refuses_counts_and_recovers_in_order() {
        let (mut tx, mut rx) = feed(4, bank());
        for t in 0..4 {
            tx.send(play(t)).expect("room");
        }
        assert_eq!(tx.send(play(99)), Err(FeedError::Full));
        assert_eq!(tx.send(play(100)), Err(FeedError::Full));
        assert_eq!(tx.stats().dropped_full, 2, "drops are counted, not silent");
        assert_eq!(rx.pop().map(|c| c.tick()), Some(0));
        tx.send(play(4)).expect("one slot freed");
        let rest: Vec<u32> = std::iter::from_fn(|| rx.pop()).map(|c| c.tick()).collect();
        assert_eq!(rest, vec![1, 2, 3, 4], "order kept across the wrap");
        assert_eq!(tx.stats().sent, 5);
    }

    #[test]
    fn indices_wrap_many_times_without_losing_order() {
        let (mut tx, mut rx) = feed(4, bank());
        let mut next = 0;
        for t in 0..1_000 {
            tx.send(play(t)).expect("room");
            if t % 3 == 2 {
                while let Some(cmd) = rx.pop() {
                    assert_eq!(cmd.tick(), next);
                    next += 1;
                }
            }
        }
        while let Some(cmd) = rx.pop() {
            assert_eq!(cmd.tick(), next);
            next += 1;
        }
        assert_eq!(next, 1_000);
    }

    #[test]
    fn capacity_rounds_to_a_power_of_two_within_bounds() {
        for (ask, want) in [
            (0, 2),
            (1, 2),
            (3, 4),
            (5, 8),
            (1024, 1024),
            (usize::MAX, 65_536),
        ] {
            let (mut tx, _rx) = feed(ask, bank());
            let mut held = 0;
            while tx.send(play(0)).is_ok() {
                held += 1;
            }
            assert_eq!(held, want, "capacity {ask}");
        }
    }

    #[test]
    fn names_resolve_and_unknown_names_are_counted() {
        let (mut tx, mut rx) = feed(8, bank());
        tx.play(3, "b", 0.25, Some([1, 2, 3])).expect("known cue");
        assert_eq!(tx.play(3, "nope", 1.0, None), Err(FeedError::UnknownCue));
        assert_eq!(tx.stop(3, "nope"), Err(FeedError::UnknownCue));
        assert_eq!(tx.stats().unknown_cue, 2);
        assert_eq!(
            rx.pop(),
            Some(Command::Play(AudioCommand {
                tick: 3,
                cue: CueId(1),
                gain: 0.25,
                pos_cm: Some([1, 2, 3]),
            }))
        );
        assert!(rx.pop().is_none(), "unknown names queue nothing");
        tx.listener(4, [5, 6, 7], 90).expect("room");
        tx.stop(4, "a").expect("known cue");
        assert_eq!(rx.pop().map(|c| c.tick()), Some(4));
        assert_eq!(
            rx.pop(),
            Some(Command::Stop {
                tick: 4,
                cue: CueId(0)
            })
        );
    }

    #[test]
    fn dropping_the_consumer_closes_the_producer() {
        let (mut tx, rx) = feed(4, bank());
        assert!(!tx.is_closed());
        drop(rx);
        assert!(tx.is_closed());
        assert_eq!(tx.send(play(0)), Err(FeedError::Closed));
        assert!(!FeedError::Closed.to_string().is_empty());
    }

    #[test]
    fn ring_use_after_warmup_allocates_nothing() {
        let (mut tx, mut rx) = feed(64, bank());
        tx.send(play(0)).expect("room");
        rx.pop();
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for t in 0..200 {
                tx.send(play(t)).expect("room");
                tx.listener(t, [1, 2, 3], 4).expect("room");
                assert!(rx.pop().is_some());
                assert!(rx.pop().is_some());
            }
        });
        assert_eq!(heap.calls, 0, "send and pop never touch the heap");
    }

    #[test]
    fn tension_and_stinger_commands_round_trip_and_validate() {
        let (mut tx, mut rx) = feed(8, bank());
        for (tick, level) in Tension::ALL.into_iter().enumerate() {
            tx.tension(u32::try_from(tick).unwrap_or(0), level)
                .expect("room");
        }
        for (tick, level) in Tension::ALL.into_iter().enumerate() {
            let want = Command::Tension {
                tick: u32::try_from(tick).unwrap_or(0),
                level,
            };
            assert_eq!(rx.pop(), Some(want));
            assert_eq!(want.tick(), u32::try_from(tick).unwrap_or(0));
        }
        tx.stinger(9, "s").expect("a stinger cue");
        assert_eq!(
            rx.pop(),
            Some(Command::Play(AudioCommand {
                tick: 9,
                cue: CueId(2),
                gain: 1.0,
                pos_cm: None,
            }))
        );
        assert_eq!(
            tx.stinger(9, "a"),
            Err(FeedError::UnknownCue),
            "a is no stinger"
        );
        assert_eq!(tx.stinger(9, "zzz"), Err(FeedError::UnknownCue));
        assert_eq!(tx.stats().unknown_cue, 2);
        assert!(rx.pop().is_none(), "refused stingers queue nothing");
    }

    #[test]
    fn a_corrupt_tension_level_is_skipped_not_applied() {
        let (mut tx, mut rx) = feed(4, bank());
        tx.ring.slots[0].0[0].store(TAG_TENSION, Ordering::Relaxed);
        tx.ring.slots[0].0[1].store(9, Ordering::Relaxed);
        tx.ring.tail.0.store(1, Ordering::Release);
        tx.send(play(7)).expect("room");
        assert_eq!(
            rx.pop().map(|c| c.tick()),
            Some(7),
            "the bad slot is dropped"
        );
    }
}
