//! bt_reactive: the reactive composites and their running-child memory (SW30).
//!
//! [`BtNode::ReactiveSequence`] ticks its children from the first on every
//! tick instead of resuming at the child that returned `Running`. That is what
//! lets a guard abort the branch below it: when a guard fails (or, for a
//! fallback, succeeds) the child the composite was waiting on is halted, its
//! record cleared, and the next tick starts again at child 0.
//!
//! The memory is caller-owned ([`BtRun`], one running child per reactive node,
//! keyed by the node's pre-order index), so a tree tick still takes `&self`,
//! allocates nothing and stays a pure function of the tick stream. A plain
//! [`BtTree::tick`] carries an empty memory: the reactive nodes then re-tick
//! from child 0 with nothing to abort, exactly as a tree that is never halted.
//!
//! Donor: BehaviorTree.CPP `ReactiveSequence::tick` and
//! `ReactiveFallback::tick` (children from 0, halt the siblings of a running
//! child, `resetChildren` on every other exit).
//! Units: node indices, action ids, ticks.
//! Errors: none; every path is total and never panics.

use crate::bt::{tick_node, BtNode, BtStatus, BtTree, Scratch, MAX_NODES};
use crate::bt_trace::Sink;

/// Sentinel: the composite holds no running child.
const NO_CHILD: u8 = u8::MAX;

/// Running memory for one tree: the running child of every reactive
/// composite, keyed by the node's pre-order index, plus the halt count.
/// Caller-owned and stack-sized; ticking allocates nothing.
/// Determinism: the same tree plus the same tick stream gives the same
/// memory, so a bout tape replays it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtRun {
    child: [u8; MAX_NODES],
    halts: u32,
}

impl BtRun {
    /// Empty memory: no composite waits on a child. Units: nodes.
    pub const fn new() -> Self {
        Self {
            child: [NO_CHILD; MAX_NODES],
            halts: 0,
        }
    }

    /// Forget every running child and the halt count. Units: nodes.
    pub fn clear(&mut self) {
        self.child = [NO_CHILD; MAX_NODES];
        self.halts = 0;
    }

    /// Children halted since the memory was made. Telemetry only: it never
    /// feeds a decision or a hash. Units: halts.
    pub const fn halts(&self) -> u32 {
        self.halts
    }

    /// The child index the composite at pre-order `index` waits on; `None`
    /// when the index is out of range or nothing runs there. Units: child
    /// index within that composite.
    pub fn running_child(&self, index: usize) -> Option<u8> {
        match self.child.get(index) {
            Some(child) if *child != NO_CHILD => Some(*child),
            _ => None,
        }
    }

    /// True while the composite at pre-order `index` waits on a child.
    pub fn holds_running(&self, index: usize) -> bool {
        self.running_child(index).is_some()
    }

    /// Record `child` as the one the composite at `index` waits on. An
    /// out-of-range index or child leaves the memory untouched.
    fn hold(&mut self, index: u8, child: usize) {
        if let (Some(slot), Ok(child)) =
            (self.child.get_mut(usize::from(index)), u8::try_from(child))
        {
            *slot = child;
        }
    }

    /// Drop the record of the composite at `index`; true when one was there.
    fn release(&mut self, index: u8) -> bool {
        match self.child.get_mut(usize::from(index)) {
            Some(slot) if *slot != NO_CHILD => {
                *slot = NO_CHILD;
                true
            }
            _ => false,
        }
    }

    /// Halt the child the composite at `idx` waits on (the donor's
    /// `resetChildren`): its own record and every record under its children
    /// go, and the halt is counted once. True when there was a child to
    /// halt. Units: nodes.
    fn halt_held(&mut self, kids: &[BtNode], idx: u8) -> bool {
        let held = self.release(idx);
        halt_children(self, kids, idx, None);
        if held {
            self.halts = self.halts.saturating_add(1);
        }
        held
    }
}

impl Default for BtRun {
    fn default() -> Self {
        Self::new()
    }
}

/// Which reactive composite is ticking. The two differ only in the status
/// that stops them and the status they then report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reactive {
    /// Stops on the first `Failure` and reports it.
    Sequence,
    /// Stops on the first `Success` and reports it.
    Fallback,
}

impl<S: Sink> Scratch<'_, S> {
    /// Tick one reactive composite at pre-order `idx`. Children run from the
    /// first every tick. `Running` halts the siblings and is remembered; the
    /// stop status [`Reactive::Sequence`] is `Failure` and
    /// [`Reactive::Fallback`] is `Success`, and it halts the remembered child
    /// so the next tick restarts at child 0. `depth` is this node's nesting.
    /// Units: ticks.
    pub(crate) fn reactive(
        &mut self,
        kind: Reactive,
        kids: &[BtNode],
        idx: u8,
        depth: u8,
    ) -> BtStatus {
        let stop = match kind {
            Reactive::Sequence => BtStatus::Failure,
            Reactive::Fallback => BtStatus::Success,
        };
        let mut next = idx.saturating_add(1);
        for (child, kid) in kids.iter().enumerate() {
            let got = tick_node(kid, self, next, depth.saturating_add(1));
            if got == BtStatus::Running {
                halt_children(self.run, kids, idx, Some(child));
                self.run.hold(idx, child);
                return BtStatus::Running;
            }
            if got == stop {
                self.run.halt_held(kids, idx);
                return stop;
            }
            next = next.saturating_add(subtree_len(kid));
        }
        self.run.halt_held(kids, idx);
        match kind {
            Reactive::Sequence => BtStatus::Success,
            Reactive::Fallback => BtStatus::Failure,
        }
    }
}

impl BtTree {
    /// Halt the whole tree: every running child under the root is forgotten,
    /// so the next [`tick_run`](BtTree::tick_run) starts cold. The blackboard
    /// is left alone. True when a child was halted. Units: nodes.
    pub fn halt(&self, run: &mut BtRun) -> bool {
        let halted = halt_node(run, self.root(), 0);
        if halted {
            run.halts = run.halts.saturating_add(1);
        }
        halted
    }
}

/// Node count of a subtree as a `u8` (trees hold at most [`MAX_NODES`]).
/// Units: nodes.
fn subtree_len(node: &BtNode) -> u8 {
    u8::try_from(node.node_count()).unwrap_or(u8::MAX)
}

/// Halt one subtree: forget every running child under `node`, which sits at
/// pre-order `idx`. True when a record was there to clear. Units: nodes.
fn halt_node(run: &mut BtRun, node: &BtNode, idx: u8) -> bool {
    let mut halted = run.release(idx);
    match node {
        BtNode::Sequence(kids)
        | BtNode::Selector(kids)
        | BtNode::ReactiveSequence(kids)
        | BtNode::ReactiveFallback(kids) => {
            let mut next = idx.saturating_add(1);
            for kid in kids {
                halted |= halt_node(run, kid, next);
                next = next.saturating_add(subtree_len(kid));
            }
        }
        BtNode::Action(_)
        | BtNode::Cond(_)
        | BtNode::SetBb { .. }
        | BtNode::AddBb { .. }
        | BtNode::TickGate { .. } => {}
    }
    halted
}

/// Halt every child of the composite at `idx` except the child index `keep`
/// (the donor halts the siblings of the running child). A sibling that
/// already finished this tick has no running child left, so the walk clears
/// nothing and counts nothing. Units: nodes.
fn halt_children(run: &mut BtRun, kids: &[BtNode], idx: u8, keep: Option<usize>) {
    let mut next = idx.saturating_add(1);
    for (child, kid) in kids.iter().enumerate() {
        if keep != Some(child) {
            halt_node(run, kid, next);
        }
        next = next.saturating_add(subtree_len(kid));
    }
}

#[cfg(test)]
#[path = "bt_reactive_tests.rs"]
mod tests;
