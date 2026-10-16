//! behavior: behavior-tree. Stateless actions; mutation flows through ops appended to a caller buffer.

use super::{Brief, Op};

/// Tick status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Success,
    Failure,
    Running,
}

/// BT node. Action holds a stateless fn that appends its ops; composites own
/// ordered children.
#[derive(Clone)]
pub enum Behavior {
    Action(fn(&Brief, &mut Vec<Op>)),
    Sequence(Vec<Behavior>),
    Select(Vec<Behavior>),
    Invert(Box<Behavior>),
    /// Reads the integer brief and answers a status: the only source of
    /// Failure and Running in a stateless tree.
    Check(fn(&Brief) -> Status),
    /// Ticks every child each pass, none skipped: Failure if any child
    /// failed, else Running if any is running, else Success.
    Parallel(Vec<Behavior>),
}

/// Tick a tree: the status, with ops appended to the caller-owned `ops` in
/// visit order. Deterministic, no dedup/filter; a buffer that already has
/// room makes the tick allocation-free.
pub fn tick(b: &Behavior, brief: &Brief, ops: &mut Vec<Op>) -> Status {
    match b {
        Behavior::Action(f) => {
            f(brief, ops);
            Status::Success
        }
        Behavior::Sequence(kids) => {
            for k in kids {
                match tick(k, brief, ops) {
                    Status::Failure => return Status::Failure,
                    Status::Running => return Status::Running,
                    Status::Success => {}
                }
            }
            Status::Success
        }
        Behavior::Select(kids) => {
            for k in kids {
                match tick(k, brief, ops) {
                    Status::Success => return Status::Success,
                    Status::Running => return Status::Running,
                    Status::Failure => {}
                }
            }
            Status::Failure
        }
        Behavior::Invert(kid) => match tick(kid, brief, ops) {
            Status::Success => Status::Failure,
            Status::Failure => Status::Success,
            Status::Running => Status::Running,
        },
        Behavior::Check(f) => f(brief),
        Behavior::Parallel(kids) => {
            let (mut failed, mut running) = (false, false);
            for k in kids {
                match tick(k, brief, ops) {
                    Status::Failure => failed = true,
                    Status::Running => running = true,
                    Status::Success => {}
                }
            }
            if failed {
                Status::Failure
            } else if running {
                Status::Running
            } else {
                Status::Success
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn br() -> Brief {
        Brief {
            tick: 0,
            entities: 0,
            bodies: 0,
            hash: 0,
        }
    }
    fn run(tree: &Behavior) -> (Status, Vec<Op>) {
        let mut ops = Vec::new();
        let status = tick(tree, &br(), &mut ops);
        (status, ops)
    }
    fn ok_empty(_: &Brief, _: &mut Vec<Op>) {}
    fn ok_a(_: &Brief, ops: &mut Vec<Op>) {
        ops.push(Op::Spawn {
            x: 1.0,
            y: 1.0,
            vx: 0.0,
            vy: 0.0,
        });
    }
    fn ok_b(_: &Brief, ops: &mut Vec<Op>) {
        ops.push(Op::Spawn {
            x: 2.0,
            y: 2.0,
            vx: 0.0,
            vy: 0.0,
        });
    }
    fn fail_a(_: &Brief, ops: &mut Vec<Op>) {
        ops.push(Op::Kick {
            id: 999999,
            vx: 0.0,
            vy: 0.0,
        });
    }
    fn fail_tree() -> Behavior {
        Behavior::Invert(Box::new(Behavior::Action(ok_empty)))
    }
    fn running(_: &Brief, _: &mut Vec<Op>) {}
    fn running_tree() -> Behavior {
        // No Running source in stateless v1; test uses a manual tree.
        Behavior::Action(running)
    }

    #[test]
    fn sequence_fails_fast() {
        let t = Behavior::Sequence(vec![
            Behavior::Action(ok_a),
            fail_tree(),
            Behavior::Action(ok_b),
        ]);
        let (st, ops) = run(&t);
        assert_eq!(st, Status::Failure);
        assert_eq!(ops.len(), 1, "{ops:?}"); // ok_a op only; fail leaf has no ops, ok_b never ran
    }

    #[test]
    fn select_picks_first_success() {
        let t = Behavior::Select(vec![
            fail_tree(),
            Behavior::Action(ok_a),
            Behavior::Action(ok_b),
        ]);
        let (st, ops) = run(&t);
        assert_eq!(st, Status::Success);
        assert_eq!(ops.len(), 1, "{ops:?}"); // A only; fail leaf emits none, B absent
    }

    #[test]
    fn invert_flips() {
        let (s1, o1) = run(&Behavior::Invert(Box::new(Behavior::Action(ok_empty))));
        assert_eq!((s1, o1.len()), (Status::Failure, 0));
        let (s2, o2) = run(&Behavior::Invert(Box::new(Behavior::Select(vec![]))));
        assert_eq!((s2, o2.len()), (Status::Success, 0)); // empty Select fails -> invert succeeds
                                                          // Invert of pass-through gives Failure.
        let (s3, _) = run(&Behavior::Invert(Box::new(running_tree())));
        assert_eq!(s3, Status::Failure);
        let _ = fail_a;
    }

    fn early(brief: &Brief) -> Status {
        if brief.tick < 3 {
            Status::Running
        } else {
            Status::Success
        }
    }
    fn at(tick: u64) -> Brief {
        Brief { tick, ..br() }
    }

    #[test]
    fn behavior_tree_check_and_parallel_status() {
        // Check is the Running/Failure source reading the integer brief.
        let gate = Behavior::Check(early);
        let mut ops = Vec::new();
        assert_eq!(tick(&gate, &at(1), &mut ops), Status::Running);
        assert_eq!(tick(&gate, &at(3), &mut ops), Status::Success);
        // Parallel: every child runs even after a failure or a Running.
        let par = Behavior::Parallel(vec![
            Behavior::Action(ok_a),
            Behavior::Check(early),
            Behavior::Action(ok_b),
        ]);
        let mut ops = Vec::new();
        assert_eq!(tick(&par, &at(1), &mut ops), Status::Running);
        assert_eq!(ops.len(), 2, "{ops:?}");
        assert_eq!(tick(&par, &at(5), &mut Vec::new()), Status::Success);
        let failing = Behavior::Parallel(vec![
            Behavior::Check(early),
            fail_tree(),
            Behavior::Action(ok_a),
        ]);
        let mut ops = Vec::new();
        assert_eq!(tick(&failing, &at(1), &mut ops), Status::Failure);
        assert_eq!(ops.len(), 1, "failure never skips the later child");
        let empty = Behavior::Parallel(vec![]);
        assert_eq!(tick(&empty, &br(), &mut ops), Status::Success);
    }

    #[test]
    fn behavior_tree_trace_is_byte_identical_x2() {
        let tree = Behavior::Sequence(vec![
            Behavior::Parallel(vec![Behavior::Action(ok_a), Behavior::Check(early)]),
            Behavior::Select(vec![fail_tree(), Behavior::Action(ok_b)]),
        ]);
        let trace = || {
            (0..6u64)
                .map(|t| {
                    let mut ops = Vec::new();
                    let status = tick(&tree, &at(t), &mut ops);
                    format!("{t}:{status:?}:{ops:?}")
                })
                .collect::<Vec<_>>()
                .join("|")
        };
        assert_eq!(trace(), trace());
        assert!(trace().contains("Running"), "early ticks run");
        assert!(trace().contains("Success"), "later ticks finish");
    }

    #[test]
    fn determinism() {
        let t = Behavior::Sequence(vec![
            Behavior::Action(ok_a),
            Behavior::Select(vec![fail_tree(), Behavior::Action(ok_b)]),
        ]);
        let (a1, o1) = run(&t);
        let (a2, o2) = run(&t);
        assert_eq!((a1, format!("{o1:?}")), (a2, format!("{o2:?}")));
    }

    #[test]
    fn ops_append_in_visit_order_after_existing_ops() {
        let t = Behavior::Sequence(vec![Behavior::Action(ok_a), Behavior::Action(ok_b)]);
        let mut ops = vec![Op::Kick {
            id: 1,
            vx: 0.0,
            vy: 0.0,
        }];
        assert_eq!(tick(&t, &br(), &mut ops), Status::Success);
        assert_eq!(ops.len(), 3, "{ops:?}");
        assert!(matches!(ops[0], Op::Kick { id: 1, .. }), "{ops:?}");
        assert!(matches!(ops[1], Op::Spawn { x, .. } if x == 1.0), "{ops:?}");
        assert!(matches!(ops[2], Op::Spawn { x, .. } if x == 2.0), "{ops:?}");
    }

    #[test]
    fn tick_into_a_reserved_buffer_allocates_nothing() {
        let tree = Behavior::Sequence(vec![
            Behavior::Select(vec![fail_tree(), Behavior::Action(ok_a)]),
            Behavior::Invert(Box::new(Behavior::Invert(Box::new(Behavior::Action(ok_b))))),
        ]);
        let brief = br();
        let mut ops = Vec::with_capacity(8);
        let ((), heap) = measure(|| {
            for _ in 0..32 {
                ops.clear();
                std::hint::black_box(tick(&tree, &brief, &mut ops));
            }
        });
        assert_eq!(ops.len(), 2);
        assert_eq!(heap.calls, 0, "behavior tick must not allocate");
    }
}
