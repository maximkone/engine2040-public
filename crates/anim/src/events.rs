//! events: frame-exact combat/audio callbacks over the animation graph.
//!
//! Owner: `engine-anim`; consumers: `audio` (footstep cues) plus
//! `combat` (hit cues) via [`Router::with_footstep_hit`]. Units:
//! integer ticks (`u32`), opaque cue codes (`u32`). Determinism: tick
//! stamps only, [`BTreeMap`] routes, fired order preserved, no clock.
//! Errors: [`EventError`] names the tick at fault. Budget: `collect`
//! is `O(fired)` pushes, zero steady-state allocation once reserved.

use super::graph::{FiredEvent, NodeId};
use std::collections::BTreeMap;

/// Footstep cue code; routed to [`Callee::Audio`] by the default wiring.
pub const FOOTSTEP_CODE: u32 = 1;
/// Hit cue code; routed to [`Callee::Combat`] by the default wiring.
pub const HIT_CUE_CODE: u32 = 2;
/// Windup-start clip marker: the tick the attack's windup begins, read by
/// [`super::phase_warp::ClipMarkers::from_cues`]. A marker, not a callback:
/// nothing routes it.
pub const WINDUP_MARKER_CODE: u32 = 3;
/// Impact clip marker: the clip's impact frame, which the phase warp lands on
/// the sim's impact tick. A marker, not a callback: nothing routes it.
pub const IMPACT_MARKER_CODE: u32 = 4;
/// Recover clip marker: the first tick after the impact, where the clip plays
/// 1:1 again. A marker, not a callback: nothing routes it.
pub const RECOVER_MARKER_CODE: u32 = 5;

/// Recorded callback recipient; data only, never a function pointer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Callee {
    /// Audio consumer (footstep cues).
    Audio,
    /// Combat consumer (hit cues).
    Combat,
}

/// Code-to-callee table; authoring data, sorted by code for stable routing.
#[derive(Clone, Debug, Default)]
pub struct Router {
    entries: BTreeMap<u32, Callee>,
}

impl Router {
    /// Empty table; [`Router::route`] before [`CallbackLog::collect`].
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Default wiring: footstep to audio, hit cue to combat.
    pub fn with_footstep_hit() -> Self {
        let mut router = Self::new();
        router.route(FOOTSTEP_CODE, Callee::Audio);
        router.route(HIT_CUE_CODE, Callee::Combat);
        router
    }

    /// Insert or replace the callee for `code`; returns the old callee.
    pub fn route(&mut self, code: u32, callee: Callee) -> Option<Callee> {
        self.entries.insert(code, callee)
    }

    /// Callee for `code`; `None` when the code has no route.
    pub fn callee_for(&self, code: u32) -> Option<Callee> {
        self.entries.get(&code).copied()
    }

    /// Route count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the table holds no routes.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One recorded callback: who to call, from which node, on which tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecordedCall {
    /// Callback recipient; data only, never a live function pointer.
    pub callee: Callee,
    /// Emit node that fired.
    pub node: NodeId,
    /// Tick fired on; always equals the eval tick.
    pub tick: u32,
    /// Cue code from the track.
    pub code: u32,
}

/// Typed event-log failures; every variant names the value at fault.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventError {
    /// A fired cue stamped `got` arrived in a `expected`-tick collect.
    TickMismatch { expected: u32, got: u32 },
    /// Tape ran backwards: `got` arrived after `last`.
    OutOfOrder { last: u32, got: u32 },
    /// A blend-space weight row of `weights` entries for `samples` samples.
    WeightCount { samples: usize, weights: usize },
}

impl std::fmt::Display for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventError::TickMismatch { expected, got } => {
                write!(f, "events: cue tick {got} in a {expected}-tick collect")
            }
            EventError::OutOfOrder { last, got } => {
                write!(f, "events: tick {got} arrived after tick {last}")
            }
            EventError::WeightCount { samples, weights } => {
                write!(f, "events: {weights} blend weights for {samples} samples")
            }
        }
    }
}

impl std::error::Error for EventError {}

/// Monotonic tape of recorded callbacks in firing order.
///
/// `collect` appends one eval tick at a time; ticks must not run
/// backwards, so a replayed tape compares equal call-for-call. The log
/// owns the only allocation; reserved once, `collect` pushes only.
#[derive(Clone, Debug, Default)]
pub struct CallbackLog {
    calls: Vec<RecordedCall>,
    last_tick: Option<u32>,
}

impl CallbackLog {
    /// Empty log.
    pub fn new() -> Self {
        Self {
            calls: Vec::new(),
            last_tick: None,
        }
    }

    /// Empty log with `capacity` calls reserved up front.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            calls: Vec::with_capacity(capacity),
            last_tick: None,
        }
    }

    /// Record one eval tick: route `fired` through `router` in slice
    /// order (eval order, then code order) and push the routed calls.
    ///
    /// Every cue must be stamped `tick`; unrouted codes are skipped.
    /// Returns the pushed count.
    pub fn collect(
        &mut self,
        tick: u32,
        fired: &[FiredEvent],
        router: &Router,
    ) -> Result<usize, EventError> {
        if let Some(last) = self.last_tick {
            if tick < last {
                return Err(EventError::OutOfOrder { last, got: tick });
            }
        }
        for cue in fired {
            if cue.tick != tick {
                return Err(EventError::TickMismatch {
                    expected: tick,
                    got: cue.tick,
                });
            }
        }
        let mut pushed = 0;
        for cue in fired {
            if let Some(callee) = router.callee_for(cue.code) {
                self.calls.push(RecordedCall {
                    callee,
                    node: cue.node,
                    tick,
                    code: cue.code,
                });
                pushed += 1;
            }
        }
        self.last_tick = Some(tick);
        Ok(pushed)
    }

    /// All recorded calls in firing order.
    pub fn calls(&self) -> &[RecordedCall] {
        &self.calls
    }

    /// Recorded call count.
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// True when no call is recorded.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Reserved capacity; steady-state ticks must not move it.
    pub fn capacity(&self) -> usize {
        self.calls.capacity()
    }

    /// Drop every call but keep the reservation; tapes restart at any tick.
    pub fn clear(&mut self) {
        self.calls.clear();
        self.last_tick = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::graph::{
        AnimGraph, ClipBank, CompiledGraph, Evaluator, EventBank, FiredEvent, NodeId, NodeKind,
        Pose, TimedEvent,
    };
    use super::*;
    use engine_core::alloc_probe::measure;
    use engine_core::Fnv64;

    const TRACK: u32 = 7;
    const CLIP: u32 = 10;
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn clip_bank() -> ClipBank {
        let mut bank = ClipBank::new();
        bank.insert(
            CLIP,
            Pose {
                positions: vec![[0.0; 3]],
                rotations: vec![IDENTITY],
            },
        )
        .unwrap();
        bank
    }

    fn cue_bank() -> EventBank {
        let mut bank = EventBank::new();
        bank.insert(
            TRACK,
            vec![
                TimedEvent {
                    tick: 12,
                    code: FOOTSTEP_CODE,
                },
                TimedEvent {
                    tick: 20,
                    code: HIT_CUE_CODE,
                },
            ],
        );
        bank
    }

    /// One clip sampled through one emit node rooted at the emit.
    fn rig() -> (CompiledGraph, AnimGraph, Evaluator) {
        let mut graph = AnimGraph::new();
        graph.add(1, "idle", NodeKind::Clip { clip: CLIP });
        graph.add(
            2,
            "cue",
            NodeKind::Emit {
                source: NodeId(1),
                track: TRACK,
            },
        );
        let compiled = graph.compile(NodeId(2)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 1).unwrap();
        (compiled, graph, eval)
    }

    /// Drive `eval` over `ticks`, collecting each tick into a fresh log.
    fn tape(
        eval: &mut Evaluator,
        clips: &ClipBank,
        events: &EventBank,
        router: &Router,
        ticks: std::ops::Range<u32>,
    ) -> CallbackLog {
        let mut log = CallbackLog::new();
        let mut out = Pose::zero(1);
        let mut fired = Vec::new();
        for tick in ticks {
            eval.eval(clips, events, tick, 16, &mut out, &mut fired)
                .unwrap();
            log.collect(tick, &fired, router).unwrap();
        }
        log
    }

    #[test]
    fn footstep_and_hit_fire_on_the_exact_frame() {
        let (_compiled, _graph, mut eval) = rig();
        let (clips, events) = (clip_bank(), cue_bank());
        let router = Router::with_footstep_hit();
        let log = tape(&mut eval, &clips, &events, &router, 0..32);
        assert_eq!(
            log.calls(),
            &[
                RecordedCall {
                    callee: Callee::Audio,
                    node: NodeId(2),
                    tick: 12,
                    code: FOOTSTEP_CODE,
                },
                RecordedCall {
                    callee: Callee::Combat,
                    node: NodeId(2),
                    tick: 20,
                    code: HIT_CUE_CODE,
                },
            ]
        );
        // Off-by-one ticks stay silent for both callees.
        assert!(log
            .calls()
            .iter()
            .all(|call| call.tick == 12 || call.tick == 20));
        assert_eq!(log.calls().len(), 2);
    }

    #[test]
    fn cue_tape_replays_identical_twice() {
        let (clips, events) = (clip_bank(), cue_bank());
        let router = Router::with_footstep_hit();
        let (_c, _g, mut first_eval) = rig();
        let (_c, _g, mut second_eval) = rig();
        let first = tape(&mut first_eval, &clips, &events, &router, 0..32);
        let second = tape(&mut second_eval, &clips, &events, &router, 0..32);
        // Replay-exact-x2: same tape, same ticks, same recorded calls.
        assert_eq!(first.calls(), second.calls());
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn calls_keep_eval_order_then_code_order() {
        let mut events = EventBank::new();
        events.insert(
            TRACK,
            vec![
                TimedEvent {
                    tick: 6,
                    code: HIT_CUE_CODE,
                },
                TimedEvent {
                    tick: 6,
                    code: FOOTSTEP_CODE,
                },
            ],
        );
        let clips = clip_bank();
        let mut graph = AnimGraph::new();
        graph.add(1, "idle", NodeKind::Clip { clip: CLIP });
        graph.add(
            5,
            "first",
            NodeKind::Emit {
                source: NodeId(1),
                track: TRACK,
            },
        );
        graph.add(
            6,
            "second",
            NodeKind::Emit {
                source: NodeId(1),
                track: TRACK,
            },
        );
        graph.add(
            7,
            "root",
            NodeKind::Blend {
                a: NodeId(5),
                b: NodeId(6),
                weight: 1.0,
            },
        );
        let compiled = graph.compile(NodeId(7)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 1).unwrap();
        let router = Router::with_footstep_hit();
        let log = tape(&mut eval, &clips, &events, &router, 6..7);
        // Node 5 fires before node 6; footstep sorts before hit cue.
        assert_eq!(
            log.calls(),
            &[
                RecordedCall {
                    callee: Callee::Audio,
                    node: NodeId(5),
                    tick: 6,
                    code: FOOTSTEP_CODE,
                },
                RecordedCall {
                    callee: Callee::Combat,
                    node: NodeId(5),
                    tick: 6,
                    code: HIT_CUE_CODE,
                },
                RecordedCall {
                    callee: Callee::Audio,
                    node: NodeId(6),
                    tick: 6,
                    code: FOOTSTEP_CODE,
                },
                RecordedCall {
                    callee: Callee::Combat,
                    node: NodeId(6),
                    tick: 6,
                    code: HIT_CUE_CODE,
                },
            ]
        );
        // Same order on replay.
        let mut eval2 = Evaluator::new();
        eval2.prepare(&graph, &compiled, 1).unwrap();
        let replay = tape(&mut eval2, &clips, &events, &router, 6..7);
        assert_eq!(log.calls(), replay.calls());
    }

    #[test]
    fn unrouted_codes_are_skipped() {
        let mut events = EventBank::new();
        events.insert(
            TRACK,
            vec![
                TimedEvent { tick: 4, code: 999 },
                TimedEvent {
                    tick: 4,
                    code: FOOTSTEP_CODE,
                },
            ],
        );
        let (_c, _g, mut eval) = rig();
        let mut out = Pose::zero(1);
        let mut fired = Vec::new();
        eval.eval(&clip_bank(), &events, 4, 16, &mut out, &mut fired)
            .unwrap();
        assert_eq!(fired.len(), 2);
        let mut log = CallbackLog::new();
        let pushed = log
            .collect(4, &fired, &Router::with_footstep_hit())
            .unwrap();
        // Unknown code 999 records nothing; the footstep still lands.
        assert_eq!(pushed, 1);
        assert_eq!(log.len(), 1);
        assert_eq!(log.calls()[0].code, FOOTSTEP_CODE);
    }

    #[test]
    fn backwards_and_mismatched_ticks_are_refused() {
        let (_c, _g, mut eval) = rig();
        let (clips, events) = (clip_bank(), cue_bank());
        let router = Router::with_footstep_hit();
        let mut out = Pose::zero(1);
        let mut fired = Vec::new();
        eval.eval(&clips, &events, 12, 16, &mut out, &mut fired)
            .unwrap();
        let mut log = CallbackLog::new();
        log.collect(12, &fired, &router).unwrap();
        // Time travel is loud, not a silent reorder.
        assert_eq!(
            log.collect(3, &[], &router).unwrap_err(),
            EventError::OutOfOrder { last: 12, got: 3 }
        );
        let err = EventError::OutOfOrder { last: 12, got: 3 };
        assert!(err.to_string().contains('3'), "{err}");
        // A cue stamped for another tick never lands in this tick.
        let stray = vec![FiredEvent {
            node: NodeId(2),
            tick: 20,
            code: HIT_CUE_CODE,
        }];
        assert_eq!(
            log.collect(12, &stray, &router).unwrap_err(),
            EventError::TickMismatch {
                expected: 12,
                got: 20
            }
        );
        assert!(log.calls().iter().all(|call| call.tick == 12));
    }

    #[test]
    fn router_replace_names_the_consumer() {
        let mut router = Router::new();
        assert!(router.is_empty());
        assert_eq!(router.len(), 0);
        assert_eq!(router.callee_for(FOOTSTEP_CODE), None);
        assert_eq!(router.route(FOOTSTEP_CODE, Callee::Combat), None);
        assert_eq!(router.callee_for(FOOTSTEP_CODE), Some(Callee::Combat));
        // Replace reports the old callee; the new one wins.
        assert_eq!(
            router.route(FOOTSTEP_CODE, Callee::Audio),
            Some(Callee::Combat)
        );
        assert_eq!(router.callee_for(FOOTSTEP_CODE), Some(Callee::Audio));
        assert_eq!(router.len(), 1);
        // Default wiring names both consumers.
        let wired = Router::with_footstep_hit();
        assert_eq!(wired.callee_for(FOOTSTEP_CODE), Some(Callee::Audio));
        assert_eq!(wired.callee_for(HIT_CUE_CODE), Some(Callee::Combat));
        assert_eq!(wired.len(), 2);
    }

    #[test]
    fn steady_state_collect_reuses_reservation() {
        let (_c, _g, mut eval) = rig();
        let (clips, events) = (clip_bank(), cue_bank());
        let router = Router::with_footstep_hit();
        let mut log = CallbackLog::with_capacity(256);
        let mut out = Pose::zero(1);
        let mut fired = Vec::with_capacity(8);
        for tick in 0..32 {
            eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                .unwrap();
            log.collect(tick, &fired, &router).unwrap();
        }
        assert_eq!(log.capacity(), 256);
        assert_eq!(log.len(), 2);
        // Clearing keeps the reservation and restarts the tape.
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.capacity(), 256);
        let replay = tape(&mut eval, &clips, &events, &router, 0..32);
        assert_eq!(replay.len(), 2);
    }

    /// One digest over the routed tape: call count plus every tick,
    /// code and node lane. Pure integer work; allocates nothing.
    fn am16_digest(log: &CallbackLog) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_len(log.len());
        for call in log.calls() {
            hash.write(&call.tick.to_le_bytes());
            hash.write(&call.code.to_le_bytes());
            hash.write(&call.node.0.to_le_bytes());
        }
        hash.finish()
    }

    #[test]
    fn am16_alloc_probe_sees_a_real_allocation() {
        // Calibration: a blind counter must not fake the gate below.
        let ((), heap) = measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(heap.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn am16_events_replay_x2_and_restore_equal() {
        let (clips, events) = (clip_bank(), cue_bank());
        let router = Router::with_footstep_hit();
        let run = || {
            let (_c, _g, mut eval) = rig();
            let mut log = CallbackLog::with_capacity(256);
            let mut out = Pose::zero(1);
            let mut fired = Vec::with_capacity(8);
            let mut digests = Vec::with_capacity(32);
            for tick in 0..32u32 {
                eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                    .unwrap();
                log.collect(tick, &fired, &router).unwrap();
                digests.push(am16_digest(&log));
            }
            digests
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "cue tape must replay x2");
        assert_ne!(first[0], first[13], "footstep tick must move the tape");
        // Snapshot mid-tape, run out, restore, run out again.
        let (_c, _g, mut eval) = rig();
        let mut log = CallbackLog::with_capacity(256);
        let mut out = Pose::zero(1);
        let mut fired = Vec::with_capacity(8);
        for tick in 0..12u32 {
            eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                .unwrap();
            log.collect(tick, &fired, &router).unwrap();
        }
        let snapshot_log = log.clone();
        let snapshot_eval = eval.clone();
        let mut reference = Vec::with_capacity(20);
        for tick in 12..32u32 {
            eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                .unwrap();
            log.collect(tick, &fired, &router).unwrap();
            reference.push(am16_digest(&log));
        }
        log.clone_from(&snapshot_log);
        eval.clone_from(&snapshot_eval);
        let mut replay = Vec::with_capacity(20);
        for tick in 12..32u32 {
            eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                .unwrap();
            log.collect(tick, &fired, &router).unwrap();
            replay.push(am16_digest(&log));
        }
        assert_eq!(reference, replay, "restore-then-collect must equal");
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn am16_events_allocate_nothing_per_tick() {
        let (_c, _g, mut eval) = rig();
        let (clips, events) = (clip_bank(), cue_bank());
        let router = Router::with_footstep_hit();
        let mut log = CallbackLog::with_capacity(256);
        let mut out = Pose::zero(1);
        let mut fired = Vec::with_capacity(8);
        for tick in 0..8u32 {
            eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                .unwrap();
            log.collect(tick, &fired, &router).unwrap();
        }
        let ((), heap) = measure(|| {
            for tick in 8..72u32 {
                eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                    .unwrap();
                log.collect(tick, &fired, &router).unwrap();
                std::hint::black_box(am16_digest(&log));
            }
        });
        assert_eq!(
            heap.calls, 0,
            "collect must not allocate per tick after warmup"
        );
    }
}
