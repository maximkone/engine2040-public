use std::sync::Arc;

use engine_core::alloc_probe::measure;
use engine_core::Rng;

use super::*;
use crate::graph::{AnimGraph, ClipBank, NodeKind, Pose};

fn state(name: &str, clip: u32, len: u32, looping: bool) -> StateDef {
    StateDef {
        name: name.into(),
        source: Source::Clip(clip),
        length_ticks: len,
        looping,
    }
}

fn when(param: &str, op: CmpOp, value: i64) -> Cond {
    CondOf::Param {
        param: param.into(),
        op,
        value,
    }
}

const NAMES: [&str; 8] = [
    "idle", "walk", "run", "jump", "attack", "block", "hit", "dead",
];

/// 8 states, 14 transitions, three parameters: the fixture of the row bar.
pub(super) fn fighter() -> StateMachineDef {
    let mut m = StateMachineDef::new("fighter", "idle");
    for (name, min, max) in [("speed", 0, 10), ("guard", 0, 1), ("hurt", 0, 1)] {
        m.params.push(ParamDef {
            name: name.into(),
            default: 0,
            min,
            max,
        });
    }
    for (i, name) in NAMES.iter().enumerate() {
        let looping = !matches!(*name, "jump" | "attack" | "hit" | "dead");
        m.states.push(state(
            name,
            u32::try_from(i).unwrap_or(0),
            20 + 4 * u32::try_from(i).unwrap_or(0),
            looping,
        ));
    }
    let edge = |from: &str, to: &str, cond: Cond, fade: u32, prio: u32| {
        let mut t = TransitionDef::new(from, to);
        t.condition = cond;
        t.fade_ticks = fade;
        t.priority = prio;
        t
    };
    m.transitions = vec![
        edge("idle", "walk", when("speed", CmpOp::Ge, 2), 6, 1),
        edge("walk", "idle", when("speed", CmpOp::Lt, 2), 6, 1),
        edge("walk", "run", when("speed", CmpOp::Ge, 6), 8, 1),
        edge("run", "walk", when("speed", CmpOp::Lt, 6), 8, 1),
        edge("idle", "jump", CondOf::TimeAtLeast(30), 3, 2),
        edge("jump", "idle", CondOf::Always, 4, 0),
        edge("walk", "attack", when("guard", CmpOp::Eq, 0), 2, 3),
        edge("attack", "idle", CondOf::Always, 5, 0),
        edge("idle", "block", when("guard", CmpOp::Eq, 1), 4, 0),
        edge("block", "idle", when("guard", CmpOp::Eq, 0), 4, 0),
        edge("idle", "hit", when("hurt", CmpOp::Eq, 1), 0, 0),
        edge("walk", "hit", when("hurt", CmpOp::Eq, 1), 0, 0),
        edge("hit", "dead", when("hurt", CmpOp::Ge, 1), 0, 1),
        edge("hit", "idle", when("hurt", CmpOp::Eq, 0), 5, 0),
    ];
    m
}

fn compiled(def: &StateMachineDef) -> Arc<CompiledMachine> {
    Arc::new(CompiledMachine::new(def).expect("fixture validates"))
}

#[test]
fn fighter_fixture_has_8_states_14_transitions_and_validates() {
    let m = fighter();
    assert_eq!((m.states.len(), m.transitions.len()), (8, 14));
    m.validate().expect("valid");
}

#[test]
fn digest_moves_with_every_field() {
    let base = fighter();
    let d0 = base.digest();
    let mut a = base.clone();
    a.transitions[0].fade_ticks += 1;
    let mut b = base.clone();
    b.states[2].looping = !b.states[2].looping;
    let mut c = base.clone();
    c.params[0].max += 1;
    let mut e = base.clone();
    e.transitions[3].condition = when("speed", CmpOp::Lt, 5);
    for other in [a, b, c, e] {
        assert_ne!(other.digest(), d0);
    }
}

#[test]
fn validator_names_each_authoring_mistake() {
    let ok = fighter();
    let mut m = ok.clone();
    m.version = 2;
    assert_eq!(m.validate(), Err(StateMachineError::UnsupportedVersion(2)));
    let mut m = ok.clone();
    m.states[1].name = "idle".into();
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::Duplicate { what: "state", .. })
    ));
    let mut m = ok.clone();
    m.transitions[0].to = "ghost".into();
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::UnknownState { transition: 0, .. })
    ));
    let mut m = ok.clone();
    m.transitions[1].condition = when("nope", CmpOp::Eq, 1);
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::UnknownParam { transition: 1, .. })
    ));
    let mut m = ok.clone();
    m.transitions[2].fade_ticks = MAX_FADE_TICKS + 1;
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::FadeTooLong { transition: 2, .. })
    ));
    let mut m = ok.clone();
    m.states[0].length_ticks = 0;
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::BadLength { .. })
    ));
    let mut m = ok.clone();
    m.params[0].default = 99;
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::BadParamRange(_))
    ));
    let mut m = ok.clone();
    m.start = "ghost".into();
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::UnknownStart(_))
    ));
    let mut m = ok;
    let mut deep = CondOf::Always;
    for _ in 0..=MAX_COND_DEPTH {
        deep = CondOf::Not(Box::new(deep));
    }
    m.transitions[0].condition = deep;
    assert!(matches!(
        m.validate(),
        Err(StateMachineError::CondTooBig { transition: 0 })
    ));
}

#[test]
fn validator_rejects_an_instant_cycle_but_allows_a_gated_one() {
    let mut m = StateMachineDef::new("loop", "a");
    m.states = vec![
        state("a", 0, 10, true),
        state("b", 1, 10, true),
        state("c", 2, 10, true),
    ];
    m.transitions = vec![
        TransitionDef::new("a", "b"),
        TransitionDef::new("b", "c"),
        TransitionDef::new("c", "a"),
    ];
    match m.validate() {
        Err(StateMachineError::InstantCycle(names)) => assert_eq!(names, ["a", "b", "c"]),
        other => panic!("expected an instant cycle, got {other:?}"),
    }
    m.transitions[2].fade_ticks = 1;
    m.validate()
        .expect("a one-tick fade breaks the instant cycle");
    m.transitions[2].fade_ticks = 0;
    m.transitions[2].condition = when_param_machine();
    m.params.push(ParamDef {
        name: "go".into(),
        default: 0,
        min: 0,
        max: 1,
    });
    m.validate().expect("a gated edge breaks it too");
    let mut selfloop = StateMachineDef::new("s", "a");
    selfloop.states = vec![state("a", 0, 10, true)];
    selfloop.transitions = vec![TransitionDef::new("a", "a")];
    assert!(matches!(
        selfloop.validate(),
        Err(StateMachineError::InstantCycle(_))
    ));
}

fn when_param_machine() -> Cond {
    when("go", CmpOp::Eq, 1)
}

#[test]
fn validator_rejects_a_cyclic_graph_source_and_a_missing_clip() {
    let mut graph = AnimGraph::new();
    graph.add(
        1,
        "a",
        NodeKind::Blend {
            a: NodeId(2),
            b: NodeId(2),
            weight: 0.5,
        },
    );
    graph.add(
        2,
        "b",
        NodeKind::Blend {
            a: NodeId(1),
            b: NodeId(1),
            weight: 0.5,
        },
    );
    let mut clips = ClipBank::new();
    clips.insert(7, Pose::zero(2)).expect("clip");
    let mut m = StateMachineDef::new("m", "x");
    m.states = vec![StateDef {
        name: "x".into(),
        source: Source::Node(1),
        length_ticks: 10,
        looping: true,
    }];
    m.validate().expect("structure is fine");
    match m.check_sources(&clips, &graph) {
        Err(StateMachineError::BadSource {
            state,
            error: GraphError::Cycle { nodes },
        }) => {
            assert_eq!(state, "x");
            assert_eq!(nodes, ["a", "b"]);
        }
        other => panic!("expected a cycle refusal, got {other:?}"),
    }
    m.states[0].source = Source::Clip(9);
    assert!(matches!(
        m.check_sources(&clips, &graph),
        Err(StateMachineError::UnknownClip { clip: 9, .. })
    ));
    m.states[0].source = Source::Clip(7);
    m.check_sources(&clips, &graph).expect("known clip");
}

#[test]
fn lints_find_unreachable_states_and_dead_ends() {
    let mut m = fighter();
    m.transitions.retain(|t| t.to != "dead");
    let lints = m.lints();
    assert!(lints.contains(&Lint::Unreachable("dead".into())));
    assert!(lints.contains(&Lint::DeadEnd("dead".into())));
    assert!(fighter().lints().contains(&Lint::DeadEnd("dead".into())));
    assert!(!fighter()
        .lints()
        .iter()
        .any(|l| matches!(l, Lint::Unreachable(_))));
}

/// 8 states, 14 edges, with an idle -> a -> b -> block chain so the target is near.
fn random_machine(seed: u64) -> StateMachineDef {
    let mut rng = Rng::new(engine_core::splitmix64(seed));
    let mut m = StateMachineDef::new("rand", "idle");
    m.params.push(ParamDef {
        name: "p".into(),
        default: 0,
        min: 0,
        max: 100,
    });
    for (i, n) in NAMES.iter().enumerate() {
        m.states.push(state(
            n,
            0,
            12 + u32::try_from(i).unwrap_or(0) * 3,
            i % 2 == 0,
        ));
    }
    let edge = |rng: &mut Rng, from: usize, to: usize, enabled: bool| {
        let mut t = TransitionDef::new(NAMES[from], NAMES[to]);
        t.condition = when(
            "p",
            CmpOp::Ge,
            i64::try_from(rng.next_range(1, 99)).unwrap_or(1),
        );
        t.advance = if enabled {
            AdvanceMode::Enabled
        } else {
            AdvanceMode::Auto
        };
        t.priority = u32::try_from(rng.next_range(0, 4)).unwrap_or(0);
        t.fade_ticks = u32::try_from(rng.next_range(0, 25)).unwrap_or(0);
        t.switch = [SwitchMode::Immediate, SwitchMode::Sync, SwitchMode::AtEnd]
            [usize::try_from(rng.next_range(0, 3)).unwrap_or(0)];
        t.reset = rng.next_range(0, 2) == 0;
        t
    };
    let mid_a = usize::try_from(rng.next_range(1, 4)).unwrap_or(1);
    let mid_b = usize::try_from(rng.next_range(4, 7)).unwrap_or(4);
    for (a, b) in [(0, mid_a), (mid_a, mid_b), (mid_b, 5)] {
        let t = edge(&mut rng, a, b, true);
        m.transitions.push(t);
    }
    while m.transitions.len() < 14 {
        let a = usize::try_from(rng.next_range(0, 8)).unwrap_or(0);
        let b = usize::try_from(rng.next_range(0, 8)).unwrap_or(0);
        let enabled = rng.next_range(0, 2) == 0;
        let mut t = edge(&mut rng, a, b, enabled);
        if rng.next_range(0, 6) == 0 {
            t.advance = AdvanceMode::Disabled;
        }
        m.transitions.push(t);
    }
    m
}

/// Every simple path over usable edges, best by (edges, priority sum, indices).
fn brute_force(m: &StateMachineDef, from: &str, to: &str) -> Option<Vec<usize>> {
    fn walk(
        m: &StateMachineDef,
        at: &str,
        to: &str,
        seen: &mut Vec<String>,
        path: &mut Vec<usize>,
        best: &mut Option<(usize, u64, Vec<usize>)>,
    ) {
        if at == to {
            let prio: u64 = path
                .iter()
                .map(|&e| u64::from(m.transitions[e].priority))
                .sum();
            let cand = (path.len(), prio, path.clone());
            if best.as_ref().is_none_or(|b| cand < *b) {
                *best = Some(cand);
            }
            return;
        }
        for (i, t) in m.transitions.iter().enumerate() {
            if t.from == at && t.advance != AdvanceMode::Disabled && !seen.contains(&t.to) {
                seen.push(t.to.clone());
                path.push(i);
                walk(m, &t.to, to, seen, path, best);
                path.pop();
                seen.pop();
            }
        }
    }
    let mut best = None;
    walk(
        m,
        from,
        to,
        &mut vec![from.to_string()],
        &mut Vec::new(),
        &mut best,
    );
    best.map(|b| b.2)
}

#[test]
fn travel_to_block_matches_brute_force_on_20_seeded_graphs() {
    let mut exact = 0;
    for seed in 0..20u64 {
        let m = random_machine(0x7a11_0000 + seed);
        m.validate().expect("random machine validates");
        let machine = CompiledMachine::new(&m).expect("compile");
        let idle = m.state_index("idle").expect("idle");
        let block = m.state_index("block").expect("block");
        let path = machine
            .travel_path(idle, block)
            .expect("chain guarantees a path");
        assert!(path.len() <= 3, "seed {seed}: {} edges", path.len());
        assert_eq!(Some(path), brute_force(&m, "idle", "block"), "seed {seed}");
        exact += 1;
    }
    assert_eq!(exact, 20);
}

#[test]
fn travel_has_no_path_through_disabled_edges_and_is_empty_at_home() {
    let mut m = fighter();
    let machine = CompiledMachine::new(&m).expect("compile");
    assert_eq!(machine.travel_path(0, 0), Ok(Vec::new()));
    // dead has no way out.
    let dead = m.state_index("dead").expect("dead");
    assert!(matches!(
        machine.travel_path(dead, 0),
        Err(PlayError::NoPath { .. })
    ));
    for t in &mut m.transitions {
        if t.to == "block" {
            t.advance = AdvanceMode::Disabled;
        }
    }
    let machine = CompiledMachine::new(&m).expect("compile");
    assert!(matches!(
        machine.travel_path(0, m.state_index("block").expect("block")),
        Err(PlayError::NoPath { .. })
    ));
}

#[test]
fn travel_prefers_fewer_edges_then_lower_priority() {
    let mut m = StateMachineDef::new("t", "a");
    m.states = ["a", "b", "c", "d"]
        .iter()
        .map(|n| state(n, 0, 10, true))
        .collect();
    let e = |f: &str, t: &str, p: u32| {
        let mut x = TransitionDef::new(f, t);
        x.advance = AdvanceMode::Enabled;
        x.priority = p;
        x
    };
    m.transitions = vec![
        e("a", "b", 5),
        e("b", "d", 5),
        e("a", "c", 0),
        e("c", "d", 0),
        e("a", "d", 9),
    ];
    let machine = CompiledMachine::new(&m).expect("compile");
    assert_eq!(
        machine.travel_path(0, 3),
        Ok(vec![4]),
        "one edge beats two cheaper edges"
    );
    m.transitions.pop();
    let machine = CompiledMachine::new(&m).expect("compile");
    assert_eq!(
        machine.travel_path(0, 3),
        Ok(vec![2, 3]),
        "equal edges: lower priority sum"
    );
    m.transitions[2].priority = 5;
    m.transitions[3].priority = 5;
    let machine = CompiledMachine::new(&m).expect("compile");
    assert_eq!(
        machine.travel_path(0, 3),
        Ok(vec![0, 1]),
        "full tie: lower indices"
    );
}

#[test]
fn edits_undo_and_redo_exactly() {
    let base = fighter();
    let d0 = base.digest();
    let mut doc = Document::new(base).expect("doc");
    let mut digests = vec![d0];
    let edits = vec![
        Edit::InsertState {
            index: 8,
            def: state("crouch", 9, 18, true),
        },
        Edit::InsertTransition {
            index: 14,
            def: TransitionDef::new("crouch", "idle"),
        },
        Edit::InsertTransition {
            index: 0,
            def: {
                let mut t = TransitionDef::new("idle", "crouch");
                t.condition = when("guard", CmpOp::Eq, 1);
                t
            },
        },
        Edit::ReplaceTransition {
            index: 3,
            def: {
                let mut t = TransitionDef::new("run", "walk");
                t.fade_ticks = 11;
                t.condition = when("speed", CmpOp::Lt, 5);
                t
            },
        },
        Edit::RenameState {
            index: 1,
            to: "stroll".into(),
        },
        Edit::ReplaceState {
            index: 2,
            def: state("run", 2, 33, false),
        },
        Edit::InsertParam {
            index: 3,
            def: ParamDef {
                name: "stamina".into(),
                default: 5,
                min: 0,
                max: 9,
            },
        },
        Edit::ReplaceParam {
            index: 3,
            def: ParamDef {
                name: "stamina".into(),
                default: 6,
                min: 0,
                max: 9,
            },
        },
        Edit::SetStart {
            name: "block".into(),
        },
        Edit::RemoveTransition { index: 5 },
        Edit::RemoveParam { index: 3 },
    ];
    for edit in edits {
        doc.apply(edit).expect("edit applies");
        digests.push(doc.def().digest());
    }
    assert_eq!(doc.undo_depth(), 11);
    assert!(
        digests.windows(2).all(|w| w[0] != w[1]),
        "each edit changes the digest"
    );
    for i in (0..digests.len() - 1).rev() {
        doc.undo().expect("undo");
        assert_eq!(doc.def().digest(), digests[i], "undo to step {i}");
    }
    assert_eq!(doc.def().digest(), d0);
    assert_eq!(doc.undo(), Err(StateMachineError::NothingToUndo));
    for d in &digests[1..] {
        doc.redo().expect("redo");
        assert_eq!(doc.def().digest(), *d);
    }
}

#[test]
fn a_refused_edit_changes_nothing_and_a_new_edit_clears_redo() {
    let mut doc = Document::new(fighter()).expect("doc");
    let d0 = doc.def().digest();
    assert!(matches!(
        doc.apply(Edit::RemoveState { index: 0 }),
        Err(StateMachineError::InUse { what: "state", .. })
    ));
    assert!(matches!(
        doc.apply(Edit::RemoveParam { index: 0 }),
        Err(StateMachineError::InUse {
            what: "parameter",
            ..
        })
    ));
    assert!(matches!(
        doc.apply(Edit::ReplaceState {
            index: 0,
            def: state("other", 0, 5, true)
        }),
        Err(StateMachineError::RenameViaReplace(_))
    ));
    assert!(matches!(
        doc.apply(Edit::InsertTransition {
            index: 99,
            def: TransitionDef::new("idle", "walk")
        }),
        Err(StateMachineError::IndexOutOfRange {
            what: "transition",
            ..
        })
    ));
    assert!(doc
        .apply(Edit::InsertTransition {
            index: 0,
            def: TransitionDef::new("idle", "ghost")
        })
        .is_err());
    assert_eq!(doc.def().digest(), d0);
    assert_eq!(doc.undo_depth(), 0);
    doc.apply(Edit::SetStart {
        name: "walk".into(),
    })
    .expect("ok");
    doc.undo().expect("undo");
    doc.apply(Edit::SetStart { name: "run".into() })
        .expect("ok");
    assert_eq!(doc.redo(), Err(StateMachineError::NothingToUndo));
}

fn run_script(m: &StateMachineDef, seed: u64, ticks: u32) -> Vec<u64> {
    let mut play = Playback::new(compiled(m));
    let mut rng = Rng::new(engine_core::splitmix64(seed));
    let mut out = Vec::new();
    for _ in 0..ticks {
        if rng.next_range(0, 7) == 0 {
            let name =
                ["speed", "guard", "hurt"][usize::try_from(rng.next_range(0, 3)).unwrap_or(0)];
            let max = if name == "speed" { 11 } else { 2 };
            play.set_param(name, i64::try_from(rng.next_range(0, max)).unwrap_or(0))
                .expect("in range");
        }
        if rng.next_range(0, 40) == 0 {
            let _ = play.travel(NAMES[usize::try_from(rng.next_range(0, 8)).unwrap_or(0)]);
        }
        play.tick();
        out.push(play.digest());
    }
    out
}

#[test]
fn three_state_fixture_replays_twice_equal() {
    let mut m = StateMachineDef::new("three", "idle");
    m.params.push(ParamDef {
        name: "speed".into(),
        default: 0,
        min: 0,
        max: 10,
    });
    m.states = vec![
        state("idle", 0, 30, true),
        state("walk", 1, 24, true),
        state("run", 2, 18, true),
    ];
    let edge = |f: &str, t: &str, c: Cond, fade: u32| {
        let mut x = TransitionDef::new(f, t);
        x.condition = c;
        x.fade_ticks = fade;
        x
    };
    m.transitions = vec![
        edge("idle", "walk", when("speed", CmpOp::Ge, 2), 5),
        edge("walk", "run", when("speed", CmpOp::Ge, 7), 7),
        edge("run", "walk", when("speed", CmpOp::Lt, 7), 7),
        edge("walk", "idle", when("speed", CmpOp::Lt, 2), 5),
    ];
    m.validate().expect("valid");
    let script = |m: &StateMachineDef| {
        let mut p = Playback::new(compiled(m));
        let mut trace = Vec::new();
        for tick in 0..600u32 {
            let speed = match tick / 100 {
                0 => 0,
                1 => 4,
                2 => 9,
                3 => 4,
                4 => 0,
                _ => 9,
            };
            p.set_param("speed", speed).expect("range");
            p.tick();
            trace.push((p.state(), p.sample().weight, p.digest()));
        }
        trace
    };
    assert_eq!(script(&m), script(&m));
    let trace = script(&m);
    assert_eq!(trace.last().map(|t| t.0), Some(2), "ends running");
    assert!(trace.iter().any(|t| t.0 == 1), "walked on the way");
}

#[test]
fn fighter_replays_x2_and_a_clone_is_a_snapshot() {
    let m = fighter();
    let a = run_script(&m, 40, 1500);
    assert_eq!(a, run_script(&m, 40, 1500));
    assert_ne!(a, run_script(&m, 41, 1500));
    let mut play = Playback::new(compiled(&m));
    for _ in 0..300 {
        play.tick();
    }
    let snap = play.clone();
    for _ in 0..200 {
        play.tick();
    }
    let mut again = snap;
    for _ in 0..200 {
        again.tick();
    }
    assert_eq!(play.digest(), again.digest());
}

#[test]
fn crossfade_length_equals_the_configured_ticks_on_500_sampled_transitions() {
    let mut sampled = 0;
    for seed in 0..100u64 {
        let m = random_machine(0x500 + seed);
        let machine = compiled(&m);
        for (e, t) in m.transitions.iter().enumerate() {
            if t.advance == AdvanceMode::Disabled || t.from == t.to {
                continue;
            }
            let from = m.state_index(&t.from).expect("from");
            let to = m.state_index(&t.to).expect("to");
            if machine.travel_path(from, to) != Ok(vec![e]) {
                continue;
            }
            let mut p = Playback::new(Arc::clone(&machine));
            p.start(&t.from).expect("start");
            p.travel(&t.to).expect("travel");
            let mut guard = 0;
            let report = loop {
                let r = p.tick();
                if r.taken.is_some() || r.fade_done {
                    break r;
                }
                guard += 1;
                assert!(guard < 200, "transition never started");
            };
            assert_eq!(report.taken, Some(e));
            let mut ticks_with_fade = 0u32;
            let mut last_weight = 0;
            if t.fade_ticks > 0 {
                let s = p.sample();
                assert!(s.from.is_some());
                assert_eq!(s.weight, 0, "a fade starts at zero");
                while p.sample().from.is_some() {
                    p.tick();
                    ticks_with_fade += 1;
                    let w = p.sample().weight;
                    assert!(
                        w >= last_weight && w <= WEIGHT_ONE,
                        "weight is monotone and bounded"
                    );
                    last_weight = w;
                    assert!(ticks_with_fade <= t.fade_ticks);
                }
                assert_eq!(last_weight, WEIGHT_ONE);
            } else {
                assert!(p.sample().from.is_none(), "zero fade is a cut");
            }
            assert_eq!(ticks_with_fade, t.fade_ticks, "seed {seed} edge {e}");
            sampled += 1;
        }
    }
    assert!(sampled >= 500, "only {sampled} transitions sampled");
}

#[test]
fn curves_are_integer_monotone_and_hit_the_ends() {
    for curve in [
        Curve::Linear,
        Curve::EaseIn,
        Curve::EaseOut,
        Curve::EaseInOut,
    ] {
        let mut last = 0;
        for e in 0..=37u32 {
            let w = playback::curve_weight(curve, e, 37);
            assert!(w >= last && w <= WEIGHT_ONE, "{curve:?} at {e}");
            last = w;
        }
        assert_eq!(playback::curve_weight(curve, 0, 37), 0);
        assert_eq!(playback::curve_weight(curve, 37, 37), WEIGHT_ONE);
    }
    assert_eq!(
        playback::curve_weight(Curve::EaseInOut, 5, 10),
        WEIGHT_ONE / 2
    );
}

fn two_states(switch: SwitchMode, fade: u32, len_a: u32, looping_a: bool) -> StateMachineDef {
    let mut m = StateMachineDef::new("two", "a");
    m.states = vec![state("a", 0, len_a, looping_a), state("b", 1, 40, true)];
    let mut t = TransitionDef::new("a", "b");
    t.switch = switch;
    t.fade_ticks = fade;
    m.transitions = vec![t];
    m
}

#[test]
fn at_end_waits_for_the_loop_end_and_the_fade_finishes_with_it() {
    for (looping, fade) in [(true, 0), (true, 6), (false, 0), (false, 6)] {
        let m = two_states(SwitchMode::AtEnd, fade, 30, looping);
        let mut p = Playback::new(compiled(&m));
        let mut start = None;
        let mut done = None;
        for tick in 1..=80u32 {
            let r = p.tick();
            if r.taken.is_some() {
                start = Some(tick);
            }
            if r.fade_done || (fade == 0 && r.taken.is_some()) {
                done = Some(tick);
                break;
            }
        }
        assert_eq!(
            done,
            Some(30),
            "looping {looping} fade {fade}: crossfade ends at the clip end"
        );
        let expect_start = if fade > 0 { 30 - fade } else { 30 };
        assert_eq!(start, Some(expect_start), "looping {looping} fade {fade}");
    }
}

#[test]
fn sync_starts_the_target_at_the_same_relative_position() {
    let mut m = two_states(SwitchMode::Sync, 4, 20, true);
    m.transitions[0].condition = CondOf::TimeAtLeast(10);
    let mut p = Playback::new(compiled(&m));
    for _ in 0..10 {
        p.tick();
    }
    let s = p.sample();
    assert_eq!(s.state, 1);
    assert_eq!(s.phase, 20, "10 of 20 ticks in a becomes 20 of 40 in b");
    assert_eq!(s.from.map(|f| f.phase), Some(10));
}

#[test]
fn auto_advance_picks_the_lowest_priority_then_the_earliest_edge() {
    let mut m = StateMachineDef::new("p", "a");
    m.states = vec![
        state("a", 0, 30, true),
        state("b", 1, 30, true),
        state("c", 2, 30, true),
    ];
    let mut to_b = TransitionDef::new("a", "b");
    to_b.condition = CondOf::TimeAtLeast(2);
    to_b.priority = 3;
    let mut to_c = TransitionDef::new("a", "c");
    to_c.condition = CondOf::TimeAtLeast(2);
    to_c.priority = 1;
    m.transitions = vec![to_b, to_c];
    let mut p = Playback::new(compiled(&m));
    assert_eq!(p.tick().taken, None);
    assert_eq!(p.tick().taken, Some(1));
    assert_eq!(p.state_name(), "c");
    let mut disabled = m.clone();
    disabled.transitions[1].advance = AdvanceMode::Enabled;
    let mut p = Playback::new(compiled(&disabled));
    p.tick();
    assert_eq!(
        p.tick().taken,
        Some(0),
        "an enabled-only edge is not taken automatically"
    );
}

#[test]
fn travel_walks_the_path_and_holds_back_automatic_edges() {
    let m = fighter();
    let mut p = Playback::new(compiled(&m));
    p.set_param("hurt", 1).expect("range");
    p.travel("run").expect("path");
    assert!(p.travelling());
    // Travel ignores conditions: idle -> walk -> run despite speed 0, and the
    // automatic idle -> hit edge (hurt = 1) never fires on the way.
    let mut seen = vec![p.state_name().to_string()];
    for _ in 0..80 {
        p.tick();
        if seen.last().map(String::as_str) != Some(p.state_name()) {
            seen.push(p.state_name().to_string());
        }
    }
    assert_eq!(&seen[..3], ["idle", "walk", "run"]);
    assert!(!p.travelling());
    assert!(matches!(
        p.travel("nowhere"),
        Err(PlayError::UnknownState(_))
    ));
    assert!(matches!(
        p.set_param("speed", 99),
        Err(PlayError::ParamOutOfRange { .. })
    ));
    assert!(matches!(
        p.set_param("ghost", 1),
        Err(PlayError::UnknownParam(_))
    ));
}

#[test]
fn walk_run_boundary_tape_oscillates_and_replays() {
    // Adjacent thresholds on the fighter fixture: walk -> run is
    // speed >= 6 while run -> walk is speed < 6, so 5/6 straddles the
    // boundary. guard = 1 holds back walk -> attack so the pair is
    // isolated; hurt stays 0. Each 10-tick half outlasts the 8-tick
    // fade, so every half must enter the other state.
    fn run_tape() -> (Vec<String>, Vec<u64>, usize) {
        let m = fighter();
        let mut p = Playback::new(compiled(&m));
        p.set_param("guard", 1).expect("range");
        p.set_param("hurt", 0).expect("range");
        p.start("walk").expect("start");
        let mut states = Vec::new();
        let mut digests = Vec::new();
        let mut flips = 0usize;
        let mut prev = p.state_name().to_string();
        for tick in 0..60u32 {
            let speed = if (tick / 10) % 2 == 0 { 5 } else { 6 };
            p.set_param("speed", speed).expect("range");
            p.tick();
            let cur = p.state_name().to_string();
            if cur != prev {
                flips += 1;
            }
            prev = cur.clone();
            states.push(cur);
            digests.push(p.digest());
        }
        (states, digests, flips)
    }
    let (states_a, digests_a, flips_a) = run_tape();
    let (states_b, digests_b, flips_b) = run_tape();
    assert_eq!(states_a, states_b, "states replay x2 identical");
    assert_eq!(digests_a, digests_b, "digests replay x2 identical");
    assert_eq!(flips_a, flips_b, "flip count replays");
    assert!(
        (4..=6).contains(&flips_a),
        "boundary tape flips={flips_a} in 60 ticks, want 4..=6 (at least 4 flips)"
    );
    assert!(
        states_a.contains(&"walk".to_string()) && states_a.contains(&"run".to_string()),
        "tape visits both walk and run, got {states_a:?}"
    );
}

#[test]
fn rebind_keeps_the_state_by_name_and_clamps_parameters() {
    let m = fighter();
    let mut p = Playback::new(compiled(&m));
    p.set_param("speed", 8).expect("range");
    p.start("run").expect("start");
    for _ in 0..5 {
        p.tick();
    }
    let mut next = m.clone();
    next.params[0].max = 5;
    p.rebind(compiled(&next));
    assert_eq!(p.state_name(), "run");
    assert_eq!(p.param("speed"), Some(5));
    let mut gone = m;
    gone.transitions
        .retain(|t| t.from != "run" && t.to != "run");
    gone.states.retain(|s| s.name != "run");
    p.rebind(compiled(&gone));
    assert_eq!(p.state_name(), "idle");
}

#[test]
fn tick_allocates_nothing() {
    let m = fighter();
    let mut p = Playback::new(compiled(&m));
    p.set_param("speed", 7).expect("range");
    p.travel("block").expect("path");
    for _ in 0..50 {
        p.tick();
    }
    let ((), heap) = measure(|| {
        for i in 0..2000u32 {
            p.set_param("speed", i64::from(i % 11)).expect("range");
            std::hint::black_box(p.tick());
            std::hint::black_box(p.sample());
        }
    });
    assert_eq!(heap.calls, 0, "tick allocated");
}

fn pose_of(v: f32) -> Pose {
    let mut p = Pose::zero(2);
    p.positions[0] = [v, 0.0, 0.0];
    p.positions[1] = [0.0, v * 2.0, 0.0];
    p
}

#[test]
fn a_fade_blends_poses_exactly_like_a_blend_node() {
    let mut clips = ClipBank::new();
    clips.insert(0, pose_of(0.0)).expect("clip");
    clips.insert(1, pose_of(8.0)).expect("clip");
    let m = two_states(SwitchMode::Immediate, 4, 30, true);
    let mut p = Playback::new(compiled(&m));
    p.tick();
    let mut scratch = [Pose::zero(2), Pose::zero(2)];
    let mut out = Pose::zero(2);
    let mut eval = |src: Source, _phase: u32, pose: &mut Pose| {
        let Source::Clip(k) = src else {
            unreachable!("clip states only")
        };
        *pose = clips.get(k).expect("banked").clone();
        Ok(())
    };
    p.pose(&mut eval, &mut scratch, &mut out).expect("pose");
    assert_eq!(out, pose_of(0.0), "weight 0 shows the state being left");
    p.tick();
    p.tick();
    assert_eq!(p.sample().weight, WEIGHT_ONE / 2);
    p.pose(&mut eval, &mut scratch, &mut out).expect("pose");
    assert_eq!(out, pose_of(4.0));
    p.tick();
    p.tick();
    p.pose(&mut eval, &mut scratch, &mut out).expect("pose");
    assert_eq!(out, pose_of(8.0));
}

#[test]
fn crossfade_midpoint_pose_is_half_at_9_ticks() {
    assert_eq!(playback::FIGHTER_CROSSFADE_TICKS, 9);
    assert_eq!(playback::fighter_crossfade_ticks(), 9);
    let mut clips = ClipBank::new();
    clips.insert(0, pose_of(0.0)).expect("clip");
    clips.insert(1, pose_of(8.0)).expect("clip");
    let m = two_states(
        SwitchMode::Immediate,
        playback::FIGHTER_CROSSFADE_TICKS,
        30,
        true,
    );
    let mut p = Playback::new(compiled(&m));
    p.tick();
    let mut scratch = [Pose::zero(2), Pose::zero(2)];
    let mut out = Pose::zero(2);
    let mut eval = |src: Source, _phase: u32, pose: &mut Pose| {
        let Source::Clip(k) = src else {
            unreachable!("clip states only")
        };
        *pose = clips.get(k).expect("banked").clone();
        Ok(())
    };
    for _ in 0..4 {
        p.tick();
    }
    // 9 is odd so Linear 4/9 is 1820, not 2048: no integer tick is
    // exactly half. The midpoint still blends exactly like a blend node.
    let w = p.sample().weight;
    assert_eq!(w, playback::curve_weight(Curve::Linear, 4, 9));
    assert_eq!(w, 1820);
    p.pose(&mut eval, &mut scratch, &mut out).expect("pose");
    let mut expect = Pose::zero(2);
    crate::graph::blend_poses(
        &mut expect,
        &pose_of(0.0),
        &pose_of(8.0),
        f32::from(u16::try_from(w).unwrap_or(u16::MAX)) / 4096.0,
    )
    .expect("blend");
    assert_eq!(out, expect);
    assert_eq!(out, pose_of(3.5546875));
    for _ in 0..5 {
        p.tick();
    }
    assert!(p.sample().from.is_none(), "fade is gone at the end");
    p.pose(&mut eval, &mut scratch, &mut out).expect("pose");
    assert_eq!(out, pose_of(8.0));
}

#[test]
fn crossfade_midpoint_replays_x2() {
    fn run() -> Vec<(u64, Pose)> {
        let mut clips = ClipBank::new();
        clips.insert(0, pose_of(0.0)).expect("clip");
        clips.insert(1, pose_of(8.0)).expect("clip");
        let m = two_states(
            SwitchMode::Immediate,
            playback::fighter_crossfade_ticks(),
            30,
            true,
        );
        let mut p = Playback::new(compiled(&m));
        let mut scratch = [Pose::zero(2), Pose::zero(2)];
        let mut out = Pose::zero(2);
        let mut trace = Vec::new();
        for _ in 0..10 {
            p.tick();
            let mut eval = |src: Source, _phase: u32, pose: &mut Pose| {
                let Source::Clip(k) = src else {
                    unreachable!("clip states only")
                };
                *pose = clips.get(k).expect("banked").clone();
                Ok(())
            };
            p.pose(&mut eval, &mut scratch, &mut out).expect("pose");
            trace.push((p.digest(), out.clone()));
        }
        trace
    }
    assert_eq!(run(), run());
}

#[test]
fn replace_keeps_the_last_good_machine_and_undoes() {
    let mut doc = Document::new(fighter()).expect("doc");
    let d0 = doc.def().digest();
    let mut next = fighter();
    next.transitions[0].fade_ticks = 9;
    doc.replace(next.clone()).expect("replace");
    assert_eq!(doc.def().digest(), next.digest());
    let mut bad = fighter();
    bad.transitions[0].to = "ghost".into();
    assert!(matches!(
        doc.replace(bad),
        Err(StateMachineError::UnknownState { transition: 0, .. })
    ));
    assert_eq!(doc.def().digest(), next.digest(), "last good kept");
    doc.undo().expect("undo");
    assert_eq!(doc.def().digest(), d0);
}
