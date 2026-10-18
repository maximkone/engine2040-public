//! space_events_test: gait-blend cue selection through the real space tick.
//!
//! Every test runs [`SpacePlayer::tick`] then [`collect_space_cues`] once per
//! tick, the order the animation tick uses. One test pins the graph `Emit`
//! path, where each emit node fires its own track.

use super::events::{
    CallbackLog, Callee, EventError, RecordedCall, Router, FOOTSTEP_CODE, HIT_CUE_CODE,
};
use super::graph::{
    AnimGraph, ClipBank, Evaluator, EventBank, FiredEvent, NodeId, NodeKind, Pose, TimedEvent,
};
use super::space::{bake, BlendSpace, SpaceParams, SpacePlayer, SpaceSample, WEIGHT_SCALE};
use super::space_events::{collect_space_cues, EventStrategy, SpaceCues};

const WALK: u32 = 10;
const RUN: u32 = 11;
const SPACE_NODE: NodeId = NodeId(6);
const STEP_TICK: u32 = 12;
const HALF: u32 = WEIGHT_SCALE / 2;
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Midpoint between the walk (0) and run (2 m/s) samples, Q10 speed.
const MID_Q: i32 = 1024;
/// Speed where the walk takes about 70 percent of the quantum.
const SEVENTY_THIRTY_Q: i32 = 810;

fn clip_bank() -> ClipBank {
    let mut bank = ClipBank::new();
    bank.insert(
        WALK,
        Pose {
            positions: vec![[0.0; 3], [0.0; 3]],
            rotations: vec![IDENTITY; 2],
        },
    )
    .unwrap();
    bank.insert(
        RUN,
        Pose {
            positions: vec![[4.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            rotations: vec![IDENTITY; 2],
        },
    )
    .unwrap();
    bank
}

fn gait_space() -> BlendSpace {
    bake(vec![
        SpaceSample::new(0, 0, WALK).unwrap(),
        SpaceSample::new(2048, 0, RUN).unwrap(),
    ])
    .unwrap()
}

/// Both clips step on the same tick with the same code: the gait case.
fn same_step_events() -> EventBank {
    let mut events = EventBank::new();
    for clip in [WALK, RUN] {
        events.insert(
            clip,
            vec![TimedEvent {
                tick: STEP_TICK,
                code: FOOTSTEP_CODE,
            }],
        );
    }
    events
}

/// Walk steps, run hits: distinct codes show which clip spoke in the calls.
fn distinct_events() -> EventBank {
    let mut events = EventBank::new();
    events.insert(
        WALK,
        vec![TimedEvent {
            tick: STEP_TICK,
            code: FOOTSTEP_CODE,
        }],
    );
    events.insert(
        RUN,
        vec![TimedEvent {
            tick: STEP_TICK,
            code: HIT_CUE_CODE,
        }],
    );
    events
}

/// Gait space, clips, cue tracks and router, driven one animation tick at a time.
struct Rig {
    space: BlendSpace,
    clips: ClipBank,
    events: EventBank,
    router: Router,
    player: SpacePlayer,
    log: CallbackLog,
    out: Pose,
    scratch: Vec<FiredEvent>,
}

impl Rig {
    fn new(events: EventBank) -> Self {
        let mut player = SpacePlayer::new();
        player.prepare(2, 2);
        Self {
            space: gait_space(),
            clips: clip_bank(),
            events,
            router: Router::with_footstep_hit(),
            player,
            log: CallbackLog::with_capacity(64),
            out: Pose::zero(2),
            scratch: Vec::with_capacity(8),
        }
    }

    /// One animation tick at `speed_q`: space tick, then the cue collect.
    fn tick(&mut self, speed_q: i32, tick: u32, strategy: EventStrategy) -> usize {
        let params = SpaceParams::from_fixed(speed_q, 0).unwrap();
        self.player
            .tick(&self.space, &self.clips, params, false, &mut self.out)
            .unwrap();
        let cues = SpaceCues::new(SPACE_NODE, &self.events, &self.router, strategy);
        collect_space_cues(
            &mut self.log,
            tick,
            &cues,
            &self.space,
            self.player.weights(),
            &mut self.scratch,
        )
        .unwrap()
    }

    /// Run ticks `0..=STEP_TICK` at one speed under one strategy.
    fn run_to_step(&mut self, speed_q: i32, strategy: EventStrategy) {
        for tick in 0..=STEP_TICK {
            self.tick(speed_q, tick, strategy);
        }
    }
}

fn step_call(callee: Callee, code: u32) -> RecordedCall {
    RecordedCall {
        callee,
        node: SPACE_NODE,
        tick: STEP_TICK,
        code,
    }
}

#[test]
fn gait_50_50_footstep_fires_once_not_per_clip() {
    // Today's per-clip answer: All fires the shared footstep for each clip.
    let mut per_clip = Rig::new(same_step_events());
    per_clip.run_to_step(MID_Q, EventStrategy::All);
    assert_eq!(per_clip.player.weights().to_vec(), vec![HALF, HALF]);
    assert_eq!(per_clip.log.len(), 2, "All fires once per blended clip");
    // The footstep wiring: one dominant pose speaks, so the shared step fires once.
    let mut wired = Rig::new(same_step_events());
    wired.run_to_step(MID_Q, EventStrategy::MaxWeight);
    assert_eq!(wired.player.weights().to_vec(), vec![HALF, HALF]);
    assert_eq!(
        wired.log.calls(),
        &[step_call(Callee::Audio, FOOTSTEP_CODE)]
    );
}

#[test]
fn seventy_thirty_max_weight_fires_the_dominant_clip_only() {
    let mut rig = Rig::new(distinct_events());
    rig.run_to_step(SEVENTY_THIRTY_Q, EventStrategy::MaxWeight);
    let weights = rig.player.weights().to_vec();
    assert!(weights[0] > weights[1], "walk must dominate: {weights:?}");
    assert!(
        (45_000..=46_500).contains(&weights[0]),
        "walk should take about 70 percent: {weights:?}"
    );
    assert_eq!(
        rig.log.calls(),
        &[step_call(Callee::Audio, FOOTSTEP_CODE)],
        "only the walk's step speaks; the run's hit stays silent"
    );
}

#[test]
fn all_fires_every_blended_clip_in_sample_order() {
    let mut rig = Rig::new(distinct_events());
    rig.run_to_step(SEVENTY_THIRTY_Q, EventStrategy::All);
    assert_eq!(
        rig.log.calls(),
        &[
            step_call(Callee::Audio, FOOTSTEP_CODE),
            step_call(Callee::Combat, HIT_CUE_CODE),
        ]
    );
}

#[test]
fn min_weight_fires_the_lightest_clip_only() {
    let mut rig = Rig::new(distinct_events());
    rig.run_to_step(SEVENTY_THIRTY_Q, EventStrategy::MinWeight);
    assert_eq!(
        rig.log.calls(),
        &[step_call(Callee::Combat, HIT_CUE_CODE)],
        "the 30 percent run speaks alone"
    );
}

#[test]
fn equal_weights_pick_the_lowest_sample_index_twice() {
    let run = |strategy: EventStrategy| {
        let mut rig = Rig::new(distinct_events());
        rig.run_to_step(MID_Q, strategy);
        assert_eq!(rig.player.weights().to_vec(), vec![HALF, HALF]);
        rig.log.calls().to_vec()
    };
    for strategy in [EventStrategy::MaxWeight, EventStrategy::MinWeight] {
        let first = run(strategy);
        let second = run(strategy);
        assert_eq!(first, second, "{strategy:?} tie must replay x2");
        assert_eq!(first, vec![step_call(Callee::Audio, FOOTSTEP_CODE)]);
    }
}

#[test]
fn weights_stay_on_the_quantum_under_every_strategy() {
    for speed in [0, 1500, MID_Q, SEVENTY_THIRTY_Q, 2048] {
        for strategy in [
            EventStrategy::All,
            EventStrategy::MaxWeight,
            EventStrategy::MinWeight,
        ] {
            let mut rig = Rig::new(same_step_events());
            rig.run_to_step(speed, strategy);
            let sum: u32 = rig.player.weights().iter().sum();
            assert_eq!(sum, WEIGHT_SCALE, "speed {speed} under {strategy:?}");
        }
    }
}

#[test]
fn footstep_hit_wiring_is_max_weight_and_the_default_is_all() {
    assert_eq!(EventStrategy::default(), EventStrategy::All);
    let events = EventBank::new();
    let router = Router::with_footstep_hit();
    let cues = SpaceCues::footstep_hit(SPACE_NODE, &events, &router);
    assert_eq!(cues.strategy(), EventStrategy::MaxWeight);
}

#[test]
fn a_weight_row_of_the_wrong_length_is_refused() {
    let events = same_step_events();
    let router = Router::with_footstep_hit();
    let cues = SpaceCues::footstep_hit(SPACE_NODE, &events, &router);
    let mut log = CallbackLog::new();
    let mut scratch = Vec::new();
    let err = collect_space_cues(
        &mut log,
        0,
        &cues,
        &gait_space(),
        &[WEIGHT_SCALE],
        &mut scratch,
    )
    .unwrap_err();
    assert_eq!(
        err,
        EventError::WeightCount {
            samples: 2,
            weights: 1
        }
    );
    assert!(log.is_empty());
}

/// Pins the graph path as it is today: each `Emit` node fires its own track,
/// so a 50/50 `Blend` over two per-clip emits fires the shared footstep twice.
/// Author one `Emit` above the `Blend` to get one footfall.
#[test]
fn graph_blend_of_per_clip_emits_fires_once_per_emit_node() {
    let mut graph = AnimGraph::new();
    graph.add(1, "walk", NodeKind::Clip { clip: WALK });
    graph.add(2, "run", NodeKind::Clip { clip: RUN });
    graph.add(
        3,
        "walk-cue",
        NodeKind::Emit {
            source: NodeId(1),
            track: WALK,
        },
    );
    graph.add(
        4,
        "run-cue",
        NodeKind::Emit {
            source: NodeId(2),
            track: RUN,
        },
    );
    graph.add(
        5,
        "gait",
        NodeKind::Blend {
            a: NodeId(3),
            b: NodeId(4),
            weight: 0.5,
        },
    );
    let compiled = graph.compile(NodeId(5)).unwrap();
    let mut eval = Evaluator::new();
    eval.prepare(&graph, &compiled, 2).unwrap();
    let mut out = Pose::zero(2);
    let mut fired = Vec::new();
    eval.eval(
        &clip_bank(),
        &same_step_events(),
        STEP_TICK,
        16,
        &mut out,
        &mut fired,
    )
    .unwrap();
    let seen: Vec<(NodeId, u32)> = fired.iter().map(|cue| (cue.node, cue.code)).collect();
    assert_eq!(
        seen,
        vec![(NodeId(3), FOOTSTEP_CODE), (NodeId(4), FOOTSTEP_CODE)]
    );
}
