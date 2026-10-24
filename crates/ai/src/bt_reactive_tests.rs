//! bt_reactive unit tests: a guard that flips halts the running child.
//! Units: millimetres, permille, ticks.

use crate::bt::{
    BtBlackboard, BtCond, BtNode, BtState, BtStatus, BtTree, ACTION_ATTACK, ACTION_BLOCK,
    BT_VERSION, MAX_NODES,
};
use crate::bt_reactive::BtRun;

fn tick_state(tick: u32) -> BtState {
    BtState::new(1_000, false, 500, tick)
}

/// Three children: a guard on slot 0, a gated worker that starts once per
/// tick and returns `Running` between its periods, and a tail action.
fn sequence_tree() -> BtTree {
    BtTree::new(BtNode::ReactiveSequence(vec![
        BtNode::Cond(BtCond::BbGe {
            slot: 0,
            threshold: 1,
        }),
        BtNode::Sequence(vec![
            BtNode::AddBb { slot: 1, delta: 1 },
            BtNode::TickGate { period: 4 },
        ]),
        BtNode::Action(ACTION_ATTACK),
    ]))
    .expect("the tree fits the caps")
}

/// Mirror image: a guard that can open, the same gated worker, and a tail
/// condition that stays false so the worker is the running child.
fn fallback_tree() -> BtTree {
    BtTree::new(BtNode::ReactiveFallback(vec![
        BtNode::Cond(BtCond::BbGe {
            slot: 0,
            threshold: 1,
        }),
        BtNode::Sequence(vec![
            BtNode::AddBb { slot: 1, delta: 1 },
            BtNode::TickGate { period: 4 },
        ]),
        BtNode::Cond(BtCond::BbGe {
            slot: 2,
            threshold: 1,
        }),
    ]))
    .expect("the tree fits the caps")
}

#[test]
fn reactive_sequence_halts_the_running_child_when_its_guard_fails() {
    let tree = sequence_tree();
    let mut run = BtRun::new();
    let mut bb = BtBlackboard::new();
    let mut out = ACTION_BLOCK;
    assert!(bb.set(0, 1), "guard open");

    assert_eq!(
        tree.tick_run(&tick_state(1), &mut bb, &mut out, &mut run),
        BtStatus::Running
    );
    assert_eq!(bb.get(1), Some(1), "the worker started once");
    assert_eq!(out, ACTION_BLOCK, "the tail did not run");
    assert_eq!(
        run.running_child(0),
        Some(1),
        "child 1 is the running child"
    );
    assert_eq!(run.halts(), 0);

    // The guard flips: child 1 is never visited again, so it is halted.
    assert!(bb.set(0, 0), "guard closed");
    assert_eq!(
        tree.tick_run(&tick_state(2), &mut bb, &mut out, &mut run),
        BtStatus::Failure
    );
    assert_eq!(run.halts(), 1, "the running child was halted");
    assert_eq!(run.running_child(0), None, "no child is remembered");
    assert_eq!(out, ACTION_BLOCK, "the tail still did not run");
    assert_eq!(bb.get(1), Some(1), "the halted worker did not run again");

    // The guard opens again: the sequence restarts at child 0, so the worker
    // counts a second start and its gate succeeds on the period.
    assert!(bb.set(0, 1));
    assert_eq!(
        tree.tick_run(&tick_state(4), &mut bb, &mut out, &mut run),
        BtStatus::Success
    );
    assert_eq!(
        bb.get(1),
        Some(2),
        "the worker restarted from its first node"
    );
    assert_eq!(out, ACTION_ATTACK, "the tail ran");
    assert_eq!(run.halts(), 1);
}

#[test]
fn reactive_fallback_success_halts_the_rest() {
    let tree = fallback_tree();
    let mut run = BtRun::new();
    let mut bb = BtBlackboard::new();
    let mut out = ACTION_BLOCK;

    assert_eq!(
        tree.tick_run(&tick_state(1), &mut bb, &mut out, &mut run),
        BtStatus::Running
    );
    assert_eq!(bb.get(1), Some(1), "the worker started once");
    assert_eq!(
        run.running_child(0),
        Some(1),
        "child 1 is the running child"
    );

    // The guard succeeds: the rest is halted and the fallback succeeds.
    assert!(bb.set(0, 1), "guard open");
    assert_eq!(
        tree.tick_run(&tick_state(2), &mut bb, &mut out, &mut run),
        BtStatus::Success
    );
    assert_eq!(run.halts(), 1, "the running child was halted");
    assert_eq!(run.running_child(0), None, "no child is remembered");
    assert_eq!(bb.get(1), Some(1), "the halted worker did not run again");

    // The guard closes: the worker starts again from its first node.
    assert!(bb.set(0, 0), "guard closed");
    assert_eq!(
        tree.tick_run(&tick_state(3), &mut bb, &mut out, &mut run),
        BtStatus::Running
    );
    assert_eq!(
        bb.get(1),
        Some(2),
        "the worker restarted from its first node"
    );
}

#[test]
fn halt_forgets_every_running_child() {
    let tree = sequence_tree();
    let mut run = BtRun::new();
    let mut bb = BtBlackboard::new();
    let mut out = ACTION_BLOCK;
    assert!(bb.set(0, 1));
    assert_eq!(
        tree.tick_run(&tick_state(1), &mut bb, &mut out, &mut run),
        BtStatus::Running
    );
    assert!(tree.halt(&mut run), "the running child was halted");
    assert_eq!(run.running_child(0), None);
    assert_eq!(run.halts(), 1);
    assert!(!tree.halt(&mut run), "a second halt has nothing to clear");
    assert_eq!(run.halts(), 1);
}

#[test]
fn a_plain_tick_runs_a_reactive_tree_without_memory() {
    let tree = sequence_tree();
    let mut bb = BtBlackboard::new();
    let mut out = ACTION_BLOCK;
    assert!(bb.set(0, 1));
    assert_eq!(
        tree.tick(&tick_state(1), &mut bb, &mut out),
        BtStatus::Running
    );
    assert_eq!(bb.get(1), Some(1));
    // Nothing was remembered, so the flipped guard simply fails the tick.
    assert!(bb.set(0, 0));
    assert_eq!(
        tree.tick(&tick_state(2), &mut bb, &mut out),
        BtStatus::Failure
    );
    assert_eq!(bb.get(1), Some(1));
}

#[test]
fn reactive_nodes_round_trip_and_keep_their_own_tag() {
    for tree in [sequence_tree(), fallback_tree()] {
        let bytes = tree.encode();
        let back = BtTree::decode(&bytes).expect("decode");
        assert_eq!(back, tree);
        assert_eq!(back.encode(), bytes);
        assert_eq!(back.tree_hash(), tree.tree_hash());
        assert!(tree.node_count() <= MAX_NODES);
    }
    let sequence = sequence_tree().encode();
    assert!(
        sequence.contains(&18),
        "reactive sequence tag: {sequence:?}"
    );
    assert!(
        !sequence.contains(&17),
        "no plain selector tag: {sequence:?}"
    );
    let fallback = fallback_tree().encode();
    assert!(
        fallback.contains(&19),
        "reactive fallback tag: {fallback:?}"
    );
    let empty = BtTree::decode(&[BT_VERSION, 19, 0]).expect("empty fallback");
    assert_eq!(
        empty.tick(&tick_state(0), &mut BtBlackboard::new(), &mut 0u8),
        BtStatus::Failure
    );
}

#[test]
fn a_reactive_tick_allocates_nothing() {
    let tree = sequence_tree();
    let mut run = BtRun::new();
    let mut bb = BtBlackboard::new();
    let mut out = ACTION_BLOCK;
    assert!(bb.set(0, 1));
    let ((), heap) = engine_core::alloc_probe::measure(|| {
        for tick in 0..64u32 {
            std::hint::black_box(tree.tick_run(&tick_state(tick), &mut bb, &mut out, &mut run));
        }
    });
    assert_eq!(heap.calls, 0, "a reactive tick must not allocate");
}
