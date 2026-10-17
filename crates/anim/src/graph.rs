//! graph: animation graph as data — nodes compile to a flat eval order.
//!
//! Owner: `engine-anim`; consumers: blend-space, body-layer, event-track
//! rows via [`AnimGraph::compile`] plus [`Evaluator::eval`]. Units: ticks
//! (`u32`), ms (`i64`); poses meters plus unit quats. Determinism: topo
//! order, ties by [`NodeId`], `+ - * /` and `sqrt` only, no clock. Errors:
//! [`GraphError`]; cycles and bad refs name nodes. Budget: eval reuses
//! scratch, zero steady-state allocation (counting-allocator test below).
//!
//! Mirrors (std-only stand-in): [`Pose`], [`ClipBank`], [`EventBank`] copy
//! the contract shapes.
//! The mirrors stay until body-layers owns the shared imports.

use std::collections::BTreeMap;

/// Typed integer key for graph nodes; key order decides every tie.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId(pub u32);

/// Graph node kinds as data. Weights are unitless `0..=1` blends.
#[derive(Clone, Debug)]
pub enum NodeKind {
    /// Sample clip `clip` from the bank at the eval tick.
    Clip { clip: u32 },
    /// Mix poses `a` and `b` by `weight`.
    Blend { a: NodeId, b: NodeId, weight: f32 },
    /// Add `over` onto `base` scaled by `weight` (mask tables arrive later).
    Layer {
        base: NodeId,
        over: NodeId,
        weight: f32,
    },
    /// Pass `source` through, firing track `track` events on the eval tick.
    Emit { source: NodeId, track: u32 },
}

impl NodeKind {
    /// Direct dependencies; eval reads these before the node itself.
    fn deps(&self) -> [Option<NodeId>; 2] {
        match *self {
            NodeKind::Clip { .. } => [None, None],
            NodeKind::Blend { a, b, .. } => [Some(a), Some(b)],
            NodeKind::Layer { base, over, .. } => [Some(base), Some(over)],
            NodeKind::Emit { source, .. } => [Some(source), None],
        }
    }

    /// Blend/layer weight, if the kind carries one.
    fn weight(&self) -> Option<f32> {
        match *self {
            NodeKind::Blend { weight, .. } | NodeKind::Layer { weight, .. } => Some(weight),
            NodeKind::Clip { .. } | NodeKind::Emit { .. } => None,
        }
    }
}

/// One named graph node; `name` exists so errors point at authoring data.
#[derive(Clone, Debug)]
pub struct Node {
    /// Integer key, unique inside one graph.
    pub id: NodeId,
    /// Authoring name, carried into every diagnostic.
    pub name: String,
    /// Node behavior as data.
    pub kind: NodeKind,
}

/// Typed graph failures; every variant names the node or key at fault.
#[derive(Clone, PartialEq, Debug)]
pub enum GraphError {
    /// No nodes to compile.
    EmptyGraph,
    /// Root id is not in the graph.
    UnknownRoot(NodeId),
    /// Two nodes share one id.
    DuplicateId(NodeId),
    /// Node `node` depends on `missing`, which is not in the graph.
    UnknownRef { node: String, missing: NodeId },
    /// Node `node` samples clip `clip`, absent from the bank.
    UnknownClip { node: String, clip: u32 },
    /// Node `node` fires track `track`, absent from the bank.
    UnknownTrack { node: String, track: u32 },
    /// Dependency cycle; `nodes` holds the sorted member names.
    Cycle { nodes: Vec<String> },
    /// Non-finite blend weight on `node`.
    BadWeight { node: String, weight: f32 },
    /// Joint-count mismatch; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Zero-joint pose or prepare width.
    EmptyPose,
    /// Negative step in milliseconds.
    BadTime(i64),
    /// Eval ran before prepare, or the shape changed since.
    NotPrepared,
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::EmptyGraph => write!(f, "graph: no nodes to compile"),
            GraphError::UnknownRoot(id) => {
                write!(f, "graph: root node {} is not in the graph", id.0)
            }
            GraphError::DuplicateId(id) => {
                write!(f, "graph: duplicate node id {}", id.0)
            }
            GraphError::UnknownRef { node, missing } => {
                write!(
                    f,
                    "graph: node '{node}' depends on missing node {}",
                    missing.0
                )
            }
            GraphError::UnknownClip { node, clip } => {
                write!(f, "graph: node '{node}' samples missing clip {clip}")
            }
            GraphError::UnknownTrack { node, track } => {
                write!(f, "graph: node '{node}' fires missing event track {track}")
            }
            GraphError::Cycle { nodes } => {
                write!(
                    f,
                    "graph: dependency cycle refuses to compile: {}",
                    nodes.join(", ")
                )
            }
            GraphError::BadWeight { node, weight } => {
                write!(f, "graph: node '{node}' carries non-finite weight {weight}")
            }
            GraphError::LengthMismatch { expected, got } => {
                write!(
                    f,
                    "graph: joint count {got} differs from prepared {expected}"
                )
            }
            GraphError::NonFinite(lane) => {
                write!(f, "graph: non-finite float in {lane}")
            }
            GraphError::EmptyPose => write!(f, "graph: zero-joint pose is invalid"),
            GraphError::BadTime(dt) => {
                write!(f, "graph: negative step {dt} ms is invalid")
            }
            GraphError::NotPrepared => {
                write!(f, "graph: eval ran before prepare")
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// Runtime graph as data: unordered nodes plus a compile step.
#[derive(Clone, Debug, Default)]
pub struct AnimGraph {
    /// Authoring nodes in any order; [`AnimGraph::compile`] sorts them.
    pub nodes: Vec<Node>,
}

impl AnimGraph {
    /// Empty graph.
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Push one named node; duplicates fail loudly at [`AnimGraph::compile`].
    pub fn add(&mut self, id: u32, name: &str, kind: NodeKind) {
        self.nodes.push(Node {
            id: NodeId(id),
            name: name.to_string(),
            kind,
        });
    }

    /// Compile the reachable-from-`root` subgraph to a flat eval order.
    ///
    /// Dependencies come before dependents; ties break by [`NodeId`], so
    /// insertion order never changes the result. Cycles, missing refs and
    /// non-finite weights fail with the node names attached.
    pub fn compile(&self, root: NodeId) -> Result<CompiledGraph, GraphError> {
        if self.nodes.is_empty() {
            return Err(GraphError::EmptyGraph);
        }
        let mut by_id: BTreeMap<u32, &Node> = BTreeMap::new();
        for node in &self.nodes {
            if by_id.insert(node.id.0, node).is_some() {
                return Err(GraphError::DuplicateId(node.id));
            }
        }
        // Reachable set from the root; missing deps name their referrer.
        let mut seen: BTreeMap<u32, ()> = BTreeMap::new();
        let mut stack: Vec<(u32, Option<String>)> = vec![(root.0, None)];
        while let Some((id, referrer)) = stack.pop() {
            if seen.contains_key(&id) {
                continue;
            }
            let node = match by_id.get(&id) {
                Some(node) => *node,
                None => {
                    return Err(match referrer {
                        Some(name) => GraphError::UnknownRef {
                            node: name,
                            missing: NodeId(id),
                        },
                        None => GraphError::UnknownRoot(NodeId(id)),
                    });
                }
            };
            seen.insert(id, ());
            for dep in node.kind.deps().into_iter().flatten() {
                stack.push((dep.0, Some(node.name.clone())));
            }
        }
        // Weights over reachable ids in key order: first bad node wins.
        for id in seen.keys() {
            let node = by_id[id];
            if let Some(weight) = node.kind.weight() {
                if !weight.is_finite() {
                    return Err(GraphError::BadWeight {
                        node: node.name.clone(),
                        weight,
                    });
                }
            }
        }
        // Kahn over the reachable set, smallest ready id first.
        let mut indegree: BTreeMap<u32, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for id in seen.keys() {
            let mut count = 0;
            for dep in by_id[id].kind.deps().into_iter().flatten() {
                count += 1;
                dependents.entry(dep.0).or_default().push(*id);
            }
            indegree.insert(*id, count);
        }
        let mut order: Vec<NodeId> = Vec::with_capacity(seen.len());
        loop {
            let next = indegree
                .iter()
                .find(|entry| *entry.1 == 0)
                .map(|entry| *entry.0);
            match next {
                Some(id) => {
                    indegree.remove(&id);
                    order.push(NodeId(id));
                    if let Some(children) = dependents.remove(&id) {
                        for child in children {
                            if let Some(degree) = indegree.get_mut(&child) {
                                *degree -= 1;
                            }
                        }
                    }
                }
                None => break,
            }
        }
        if order.len() != seen.len() {
            let mut names: Vec<String> = indegree.keys().map(|id| by_id[id].name.clone()).collect();
            names.sort();
            return Err(GraphError::Cycle { nodes: names });
        }
        Ok(CompiledGraph { order, root })
    }
}

/// Flat compiled eval order: dependencies before dependents.
#[derive(Clone, Debug)]
pub struct CompiledGraph {
    /// Dependency-first node ids; ties broken by [`NodeId`].
    pub order: Vec<NodeId>,
    /// Output node; its pose is copied to the eval result.
    pub root: NodeId,
}

impl CompiledGraph {
    /// Node count in the flat order.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// True when the order holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Minimal pose mirror of the contract local-pose shape; see module docs.
#[derive(Clone, PartialEq, Debug)]
pub struct Pose {
    /// Joint positions in meters, one per joint.
    pub positions: Vec<[f32; 3]>,
    /// Unit quats `[x, y, z, w]`, one per joint.
    pub rotations: Vec<[f32; 4]>,
}

impl Pose {
    /// Bind-safe fallback: zero positions, identity rotations.
    pub fn zero(joints: usize) -> Self {
        Self {
            positions: vec![[0.0; 3]; joints],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; joints],
        }
    }

    /// Joint count; `None` when position/rotation lanes disagree.
    fn checked_joints(&self) -> Option<usize> {
        if self.positions.len() == self.rotations.len() {
            Some(self.positions.len())
        } else {
            None
        }
    }

    /// Bank-insert validation: paired lanes, non-empty, finite values.
    fn check(&self) -> Result<(), GraphError> {
        let joints = self.checked_joints().ok_or(GraphError::LengthMismatch {
            expected: self.positions.len(),
            got: self.rotations.len(),
        })?;
        if joints == 0 {
            return Err(GraphError::EmptyPose);
        }
        if self.positions.iter().flatten().any(|v| !v.is_finite()) {
            return Err(GraphError::NonFinite("clip.positions"));
        }
        if self.rotations.iter().flatten().any(|v| !v.is_finite()) {
            return Err(GraphError::NonFinite("clip.rotations"));
        }
        Ok(())
    }
}

/// Clip stand-in: integer key to constant pose, keys kept sorted.
#[derive(Clone, Debug, Default)]
pub struct ClipBank {
    clips: Vec<ClipEntry>,
}

/// One banked clip pose.
#[derive(Clone, Debug)]
struct ClipEntry {
    key: u32,
    pose: Pose,
}

impl ClipBank {
    /// Empty bank.
    pub fn new() -> Self {
        Self { clips: Vec::new() }
    }

    /// Insert or replace clip `key`; rejects bad lanes, keeps keys sorted.
    pub fn insert(&mut self, key: u32, pose: Pose) -> Result<(), GraphError> {
        pose.check()?;
        match self.clips.binary_search_by_key(&key, |entry| entry.key) {
            Ok(at) => self.clips[at].pose = pose,
            Err(at) => self.clips.insert(at, ClipEntry { key, pose }),
        }
        Ok(())
    }

    /// Lookup by integer key; `None` when the bank holds no such clip.
    pub fn get(&self, key: u32) -> Option<&Pose> {
        self.clips
            .binary_search_by_key(&key, |entry| entry.key)
            .ok()
            .map(|at| &self.clips[at].pose)
    }

    /// Clip count.
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// True when the bank holds no clips.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
}

/// One frame-exact cue: fires when the eval tick equals `tick`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimedEvent {
    /// Fixed tick the event fires on; off-by-one fails.
    pub tick: u32,
    /// Opaque cue code for combat/audio consumers.
    pub code: u32,
}

/// Sorted event-track stand-in: integer key to `(tick, code)` cues.
#[derive(Clone, Debug, Default)]
pub struct EventBank {
    tracks: Vec<EventTrack>,
}

/// One banked event track.
#[derive(Clone, Debug)]
struct EventTrack {
    key: u32,
    events: Vec<TimedEvent>,
}

impl EventBank {
    /// Empty bank.
    pub fn new() -> Self {
        Self { tracks: Vec::new() }
    }

    /// Insert or replace track `key`; sorts by `(tick, code)` on the way in.
    pub fn insert(&mut self, key: u32, mut events: Vec<TimedEvent>) {
        events.sort_by_key(|event| (event.tick, event.code));
        match self.tracks.binary_search_by_key(&key, |track| track.key) {
            Ok(at) => self.tracks[at].events = events,
            Err(at) => self.tracks.insert(at, EventTrack { key, events }),
        }
    }

    /// Events on `key` at exactly `tick`, in `(tick, code)` order.
    pub fn at(&self, key: u32, tick: u32) -> &[TimedEvent] {
        let Ok(at) = self.tracks.binary_search_by_key(&key, |track| track.key) else {
            return &[];
        };
        let events = &self.tracks[at].events;
        let start = events.partition_point(|event| event.tick < tick);
        let len = events[start..].partition_point(|event| event.tick == tick);
        &events[start..start + len]
    }

    /// Track count.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// True when the bank holds no tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

/// One fired cue: the emit node, the tick, and the track code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FiredEvent {
    /// Emit node that fired.
    pub node: NodeId,
    /// Tick fired on; always equals the eval tick.
    pub tick: u32,
    /// Cue code from the track.
    pub code: u32,
}

/// One prepared eval step: resolved node behavior plus its scratch slot.
#[derive(Clone, Debug)]
struct Op {
    id: NodeId,
    name: String,
    kind: NodeKind,
}

/// Value equality on node kinds for the prepare shape check: same variant
/// and same fields means the kept plan still applies. Plain `==` on the
/// `f32` weights is exact change detection, never arithmetic.
fn kinds_equal(left: &NodeKind, right: &NodeKind) -> bool {
    match (left, right) {
        (NodeKind::Clip { clip: a }, NodeKind::Clip { clip: b }) => a == b,
        (
            NodeKind::Blend {
                a: a0,
                b: b0,
                weight: w0,
            },
            NodeKind::Blend {
                a: a1,
                b: b1,
                weight: w1,
            },
        ) => a0 == a1 && b0 == b1 && w0 == w1,
        (
            NodeKind::Layer {
                base: a0,
                over: b0,
                weight: w0,
            },
            NodeKind::Layer {
                base: a1,
                over: b1,
                weight: w1,
            },
        ) => a0 == a1 && b0 == b1 && w0 == w1,
        (
            NodeKind::Emit {
                source: s0,
                track: t0,
            },
            NodeKind::Emit {
                source: s1,
                track: t1,
            },
        ) => s0 == s1 && t0 == t1,
        _ => false,
    }
}

/// Reusable eval scratch: one pose slot per compiled node.
///
/// `prepare` owns the only allocation; `eval` reuses slots, so steady-state
/// ticks allocate nothing (proven by the counting-allocator test below).
#[derive(Clone, Debug, Default)]
pub struct Evaluator {
    slots: Vec<Pose>,
    plan: Vec<Op>,
    lookup: Vec<(u32, usize)>,
    root: Option<NodeId>,
    joints: usize,
}

impl Evaluator {
    /// Empty evaluator; [`Evaluator::prepare`] before [`Evaluator::eval`].
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            plan: Vec::new(),
            lookup: Vec::new(),
            root: None,
            joints: 0,
        }
    }

    /// Size scratch for `compiled` at `joints` width.
    ///
    /// Reallocates only when the shape changes; repeat prepares with the
    /// same shape reuse every buffer. Resolves node ids against `graph`,
    /// so a graph edited after compile fails here, never silently.
    pub fn prepare(
        &mut self,
        graph: &AnimGraph,
        compiled: &CompiledGraph,
        joints: usize,
    ) -> Result<(), GraphError> {
        if joints == 0 {
            return Err(GraphError::EmptyPose);
        }
        if compiled.is_empty() {
            return Err(GraphError::EmptyGraph);
        }
        if self.same_shape(graph, compiled, joints) {
            // Same ids, names and kinds: the plan stays, only the slots
            // re-zero. Lookup entries are identical too, so they stay.
            for slot in &mut self.slots {
                slot.positions.fill([0.0; 3]);
                slot.rotations.fill([0.0, 0.0, 0.0, 1.0]);
            }
            return Ok(());
        }
        self.plan.clear();
        self.plan.reserve(compiled.len());
        for id in &compiled.order {
            let node = graph
                .nodes
                .iter()
                .find(|node| node.id == *id)
                .ok_or_else(|| GraphError::UnknownRef {
                    node: String::from("prepare"),
                    missing: *id,
                })?;
            self.plan.push(Op {
                id: node.id,
                name: node.name.clone(),
                kind: node.kind.clone(),
            });
        }
        let plan_len = self.plan.len();
        if self.joints != joints || self.slots.len() != plan_len {
            self.slots.clear();
            self.slots.reserve(plan_len);
            for _ in 0..plan_len {
                self.slots.push(Pose::zero(joints));
            }
            self.joints = joints;
        } else {
            for slot in &mut self.slots {
                slot.positions.fill([0.0; 3]);
                slot.rotations.fill([0.0, 0.0, 0.0, 1.0]);
            }
        }
        self.lookup.clear();
        self.lookup.reserve(plan_len);
        for (slot, op) in self.plan.iter().enumerate() {
            self.lookup.push((op.id.0, slot));
        }
        self.lookup.sort_by_key(|entry| entry.0);
        self.root = Some(compiled.root);
        Ok(())
    }

    /// True when the kept plan already resolves `compiled` against `graph`:
    /// same joint width, root, order length, and per-node id, name and
    /// kind. Any edit (rename, rekind, add, drop) answers false so the
    /// caller takes the rebuild path and its errors. Reads only.
    fn same_shape(&self, graph: &AnimGraph, compiled: &CompiledGraph, joints: usize) -> bool {
        if self.joints != joints || self.root != Some(compiled.root) {
            return false;
        }
        if self.plan.len() != compiled.order.len() {
            return false;
        }
        for (op, id) in self.plan.iter().zip(compiled.order.iter()) {
            if op.id != *id {
                return false;
            }
            let Some(node) = graph.nodes.iter().find(|node| node.id == *id) else {
                return false;
            };
            if node.name != op.name || !kinds_equal(&node.kind, &op.kind) {
                return false;
            }
        }
        true
    }

    /// Prepared joint width.
    pub fn joints(&self) -> usize {
        self.joints
    }

    fn slot_of(&self, id: NodeId) -> Result<usize, GraphError> {
        self.lookup
            .binary_search_by_key(&id.0, |entry| entry.0)
            .map(|at| self.lookup[at].1)
            .map_err(|_| GraphError::UnknownRef {
                node: String::from("eval"),
                missing: id,
            })
    }

    /// Emit node: pass the source slot through, then push the track's cues
    /// for `tick` in track order; a track the bank never heard of is loud.
    fn eval_emit(
        &mut self,
        slot: usize,
        source: NodeId,
        track: u32,
        events: &EventBank,
        tick: u32,
        fired: &mut Vec<FiredEvent>,
    ) -> Result<(), GraphError> {
        let from = self.slot_of(source)?;
        if from >= slot {
            // Compiled order guarantees sources first; loud if broken.
            return Err(GraphError::UnknownRef {
                node: self.plan[slot].name.clone(),
                missing: source,
            });
        }
        let (head, tail) = self.slots.split_at_mut(slot);
        tail[0].positions.copy_from_slice(&head[from].positions);
        tail[0].rotations.copy_from_slice(&head[from].rotations);
        let mut known = false;
        for event in events.at(track, tick) {
            known = true;
            fired.push(FiredEvent {
                node: self.plan[slot].id,
                tick,
                code: event.code,
            });
        }
        if !known && events.tracks.iter().all(|t| t.key != track) {
            return Err(GraphError::UnknownTrack {
                node: self.plan[slot].name.clone(),
                track,
            });
        }
        Ok(())
    }

    /// Evaluate one tick: run the flat order, copy the root pose to `out`,
    /// push frame-exact cues to `fired`.
    ///
    /// `dt_ms` is validated non-negative and carried for the sampler rows;
    /// clip poses here are constant per key. `out` must be sized to the
    /// prepared width; `fired` is cleared first and keeps its capacity.
    /// Steady-state calls allocate nothing.
    pub fn eval(
        &mut self,
        clips: &ClipBank,
        events: &EventBank,
        tick: u32,
        dt_ms: i64,
        out: &mut Pose,
        fired: &mut Vec<FiredEvent>,
    ) -> Result<(), GraphError> {
        if dt_ms < 0 {
            return Err(GraphError::BadTime(dt_ms));
        }
        if self.plan.is_empty() || self.root.is_none() {
            return Err(GraphError::NotPrepared);
        }
        if out.positions.len() != self.joints || out.rotations.len() != self.joints {
            let got = if out.positions.len() != self.joints {
                out.positions.len()
            } else {
                out.rotations.len()
            };
            return Err(GraphError::LengthMismatch {
                expected: self.joints,
                got,
            });
        }
        fired.clear();
        for slot in 0..self.plan.len() {
            match self.plan[slot].kind.clone() {
                NodeKind::Clip { clip } => {
                    let pose = clips.get(clip).ok_or_else(|| GraphError::UnknownClip {
                        node: self.plan[slot].name.clone(),
                        clip,
                    })?;
                    if pose.positions.len() != self.joints || pose.rotations.len() != self.joints {
                        return Err(GraphError::LengthMismatch {
                            expected: self.joints,
                            got: pose.positions.len(),
                        });
                    }
                    self.slots[slot].positions.copy_from_slice(&pose.positions);
                    self.slots[slot].rotations.copy_from_slice(&pose.rotations);
                }
                NodeKind::Blend { a, b, weight } => {
                    let source_a = self.slot_of(a)?;
                    let source_b = self.slot_of(b)?;
                    if source_a >= slot || source_b >= slot {
                        // Hand-built orders never reach eval silently.
                        let missing = if source_a >= slot { a } else { b };
                        return Err(GraphError::UnknownRef {
                            node: self.plan[slot].name.clone(),
                            missing,
                        });
                    }
                    let (head, tail) = self.slots.split_at_mut(slot);
                    blend_poses(&mut tail[0], &head[source_a], &head[source_b], weight)?;
                }
                NodeKind::Layer { base, over, weight } => {
                    let source_base = self.slot_of(base)?;
                    let source_over = self.slot_of(over)?;
                    if source_base >= slot || source_over >= slot {
                        let missing = if source_base >= slot { base } else { over };
                        return Err(GraphError::UnknownRef {
                            node: self.plan[slot].name.clone(),
                            missing,
                        });
                    }
                    let (head, tail) = self.slots.split_at_mut(slot);
                    layer_into(&mut tail[0], &head[source_base], &head[source_over], weight)?;
                }
                NodeKind::Emit { source, track } => {
                    self.eval_emit(slot, source, track, events, tick, fired)?;
                }
            }
        }
        let Some(root) = self.root else {
            return Err(GraphError::NotPrepared);
        };
        let root_slot = self.slot_of(root)?;
        out.positions
            .copy_from_slice(&self.slots[root_slot].positions);
        out.rotations
            .copy_from_slice(&self.slots[root_slot].rotations);
        Ok(())
    }
}

use super::clip::clamp01;

/// Shortest-arc normalized lerp between unit quats; NaN-safe identity out.
/// Separate from `pose::nlerp` on purpose: the zero-length (antipodal) case
/// returns identity here while `pose::nlerp` returns `a`, so the two are
/// close but not bit-for-bit identical.
pub(crate) fn nlerp(a: [f32; 4], b: [f32; 4], w: f32) -> [f32; 4] {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if !dot.is_finite() {
        return IDENTITY;
    }
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let q = [
        a[0] + (sign * b[0] - a[0]) * w,
        a[1] + (sign * b[1] - a[1]) * w,
        a[2] + (sign * b[2] - a[2]) * w,
        a[3] + (sign * b[3] - a[3]) * w,
    ];
    let norm2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !norm2.is_finite() || norm2 <= 0.0 {
        return IDENTITY;
    }
    let inv = 1.0 / norm2.sqrt();
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// Mix `a` into `b` by `weight` (clamped to `0..=1`); a state machine fade
/// uses this same blend, so it matches a `Blend` node bit for bit.
pub fn blend_poses(dest: &mut Pose, a: &Pose, b: &Pose, weight: f32) -> Result<(), GraphError> {
    let w = clamp01(weight);
    if dest.positions.len() != a.positions.len() || dest.positions.len() != b.positions.len() {
        return Err(GraphError::LengthMismatch {
            expected: dest.positions.len(),
            got: a.positions.len().min(b.positions.len()),
        });
    }
    for joint in 0..dest.positions.len() {
        for lane in 0..3 {
            dest.positions[joint][lane] = a.positions[joint][lane]
                + (b.positions[joint][lane] - a.positions[joint][lane]) * w;
        }
        dest.rotations[joint] = nlerp(a.rotations[joint], b.rotations[joint], w);
    }
    Ok(())
}

fn layer_into(dest: &mut Pose, base: &Pose, over: &Pose, weight: f32) -> Result<(), GraphError> {
    let w = clamp01(weight);
    if dest.positions.len() != base.positions.len() || dest.positions.len() != over.positions.len()
    {
        return Err(GraphError::LengthMismatch {
            expected: dest.positions.len(),
            got: base.positions.len().min(over.positions.len()),
        });
    }
    // Additive position offsets; rotations inherit the base until the
    // mask-table row layers quats too.
    for joint in 0..dest.positions.len() {
        for lane in 0..3 {
            dest.positions[joint][lane] =
                base.positions[joint][lane] + over.positions[joint][lane] * w;
        }
        dest.rotations[joint] = base.rotations[joint];
    }
    Ok(())
}

/// Test-only capacity probe for the scratch-reuse assertion.
#[cfg(test)]
impl Evaluator {
    fn slots_capacity(&self) -> usize {
        self.slots.capacity()
    }
}

/// FNV-1a64 over every pose lane bit: the byte-exact replay proof the unit
/// tests of this crate share.
#[cfg(test)]
pub(crate) fn pose_bits(pose: &Pose) -> u64 {
    let mut hash = engine_core::Fnv64::new();
    for lane in pose
        .positions
        .iter()
        .flatten()
        .chain(pose.rotations.iter().flatten())
    {
        hash.write_f32(*lane);
    }
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;
    use engine_core::Fnv64;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn pose(p0: [f32; 3], p1: [f32; 3]) -> Pose {
        Pose {
            positions: vec![p0, p1],
            rotations: vec![IDENTITY, IDENTITY],
        }
    }

    fn bank_two() -> ClipBank {
        let mut bank = ClipBank::new();
        bank.insert(10, pose([0.0, 0.0, 0.0], [2.0, 0.0, 0.0]))
            .unwrap();
        bank.insert(11, pose([4.0, 0.0, 0.0], [6.0, 0.0, 0.0]))
            .unwrap();
        bank
    }

    fn events_four() -> EventBank {
        let mut bank = EventBank::new();
        bank.insert(
            7,
            vec![
                TimedEvent { tick: 6, code: 102 },
                TimedEvent { tick: 5, code: 100 },
                TimedEvent { tick: 6, code: 101 },
                TimedEvent { tick: 7, code: 103 },
            ],
        );
        bank
    }

    /// Chain rig: clips 1/2, blend 3, layer 4, emit 5 rooted at 5.
    fn chain() -> AnimGraph {
        let mut graph = AnimGraph::new();
        graph.add(1, "idle", NodeKind::Clip { clip: 10 });
        graph.add(2, "walk", NodeKind::Clip { clip: 11 });
        graph.add(
            3,
            "mix",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(2),
                weight: 0.25,
            },
        );
        graph.add(
            4,
            "hit-add",
            NodeKind::Layer {
                base: NodeId(3),
                over: NodeId(1),
                weight: 0.5,
            },
        );
        graph.add(
            5,
            "cue",
            NodeKind::Emit {
                source: NodeId(4),
                track: 7,
            },
        );
        graph
    }

    fn prepared_chain() -> (AnimGraph, CompiledGraph, ClipBank, EventBank, Evaluator) {
        let graph = chain();
        let compiled = graph.compile(NodeId(5)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 2).unwrap();
        (graph, compiled, bank_two(), events_four(), eval)
    }

    #[test]
    fn compile_order_is_dependency_first_and_stable() {
        let graph = chain();
        let compiled = graph.compile(NodeId(5)).unwrap();
        // Compile-order proof: flat ids in dependency-first order.
        assert_eq!(
            compiled.order,
            vec![NodeId(1), NodeId(2), NodeId(3), NodeId(4), NodeId(5)]
        );
        assert_eq!(compiled.root, NodeId(5));
        // Same nodes pushed backwards compile to the same flat order.
        let mut shuffled = AnimGraph::new();
        for node in graph.nodes.iter().rev() {
            shuffled.add(node.id.0, &node.name, node.kind.clone());
        }
        assert_eq!(shuffled.compile(NodeId(5)).unwrap().order, compiled.order);
    }

    #[test]
    fn compile_reaches_only_the_root_subgraph() {
        let mut graph = chain();
        graph.add(9, "unused", NodeKind::Clip { clip: 10 });
        let compiled = graph.compile(NodeId(2)).unwrap();
        assert_eq!(compiled.order, vec![NodeId(2)]);
    }

    #[test]
    fn cycle_names_members_loudly() {
        let mut graph = AnimGraph::new();
        graph.add(1, "alpha", NodeKind::Clip { clip: 10 });
        graph.add(
            2,
            "beta",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(3),
                weight: 0.5,
            },
        );
        graph.add(
            3,
            "gamma",
            NodeKind::Blend {
                a: NodeId(2),
                b: NodeId(1),
                weight: 0.5,
            },
        );
        let err = graph.compile(NodeId(3)).unwrap_err();
        assert_eq!(
            err,
            GraphError::Cycle {
                nodes: vec![String::from("beta"), String::from("gamma")]
            }
        );
        let text = err.to_string();
        assert!(
            text.contains("cycle") && text.contains("beta") && text.contains("gamma"),
            "{text}"
        );
    }

    #[test]
    fn self_cycle_is_refused() {
        let mut graph = AnimGraph::new();
        graph.add(
            1,
            "ouro",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(1),
                weight: 0.5,
            },
        );
        assert_eq!(
            graph.compile(NodeId(1)).unwrap_err(),
            GraphError::Cycle {
                nodes: vec![String::from("ouro")]
            }
        );
    }

    #[test]
    fn unknown_ref_names_the_referrer() {
        let mut graph = AnimGraph::new();
        graph.add(
            1,
            "seeker",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(99),
                weight: 0.0,
            },
        );
        let err = graph.compile(NodeId(1)).unwrap_err();
        assert_eq!(
            err,
            GraphError::UnknownRef {
                node: String::from("seeker"),
                missing: NodeId(99)
            }
        );
        assert!(err.to_string().contains("seeker"), "{err}");
    }

    #[test]
    fn misordered_hand_built_graph_is_refused() {
        let mut graph = AnimGraph::new();
        graph.add(1, "a", NodeKind::Clip { clip: 10 });
        graph.add(
            2,
            "mix",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(1),
                weight: 0.5,
            },
        );
        // Dependents before dependencies: prepare resolves, eval refuses.
        let bad = CompiledGraph {
            order: vec![NodeId(2), NodeId(1)],
            root: NodeId(2),
        };
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &bad, 2).unwrap();
        let mut out = Pose::zero(2);
        assert_eq!(
            eval.eval(
                &bank_two(),
                &EventBank::new(),
                0,
                16,
                &mut out,
                &mut Vec::new()
            )
            .unwrap_err(),
            GraphError::UnknownRef {
                node: String::from("mix"),
                missing: NodeId(1)
            }
        );
    }

    #[test]
    fn duplicate_unknown_root_and_empty_are_refused() {
        let mut graph = AnimGraph::new();
        graph.add(1, "a", NodeKind::Clip { clip: 10 });
        graph.add(1, "b", NodeKind::Clip { clip: 10 });
        assert_eq!(
            graph.compile(NodeId(1)).unwrap_err(),
            GraphError::DuplicateId(NodeId(1))
        );
        let mut graph = AnimGraph::new();
        graph.add(1, "a", NodeKind::Clip { clip: 10 });
        assert_eq!(
            graph.compile(NodeId(77)).unwrap_err(),
            GraphError::UnknownRoot(NodeId(77))
        );
        assert_eq!(
            AnimGraph::new().compile(NodeId(1)).unwrap_err(),
            GraphError::EmptyGraph
        );
    }

    #[test]
    fn non_finite_weight_names_the_node() {
        for bad in [f32::NAN, f32::INFINITY] {
            let mut graph = AnimGraph::new();
            graph.add(
                1,
                "wobbly",
                NodeKind::Blend {
                    a: NodeId(2),
                    b: NodeId(2),
                    weight: bad,
                },
            );
            graph.add(2, "still", NodeKind::Clip { clip: 10 });
            match graph.compile(NodeId(1)).unwrap_err() {
                GraphError::BadWeight { node, weight } => {
                    assert_eq!(node, "wobbly");
                    // NaN never equals itself; compare by finiteness instead.
                    assert!(!weight.is_finite());
                    assert_eq!(weight.is_nan(), bad.is_nan());
                }
                other => panic!("expected BadWeight, got {other:?}"),
            }
        }
    }

    #[test]
    fn eval_matches_hand_rolled_blend() {
        let mut graph = AnimGraph::new();
        graph.add(1, "a", NodeKind::Clip { clip: 10 });
        graph.add(2, "b", NodeKind::Clip { clip: 11 });
        graph.add(
            3,
            "mix",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(2),
                weight: 0.25,
            },
        );
        let compiled = graph.compile(NodeId(3)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 2).unwrap();
        let mut out = Pose::zero(2);
        let mut fired = Vec::new();
        eval.eval(&bank_two(), &EventBank::new(), 0, 16, &mut out, &mut fired)
            .unwrap();
        // Hand-rolled baseline: a + (b - a) * 0.25 per lane.
        assert_eq!(out.positions[0], [1.0, 0.0, 0.0]);
        assert_eq!(out.positions[1], [3.0, 0.0, 0.0]);
        assert_eq!(out.rotations, vec![IDENTITY, IDENTITY]);
        assert!(fired.is_empty());
        // Endpoints are exact: weight 0 keeps `a`, weight 1 keeps `b`.
        for (weight, want) in [(0.0, [0.0, 0.0, 0.0]), (1.0, [4.0, 0.0, 0.0])] {
            let mut graph = AnimGraph::new();
            graph.add(1, "a", NodeKind::Clip { clip: 10 });
            graph.add(2, "b", NodeKind::Clip { clip: 11 });
            graph.add(
                3,
                "mix",
                NodeKind::Blend {
                    a: NodeId(1),
                    b: NodeId(2),
                    weight,
                },
            );
            let compiled = graph.compile(NodeId(3)).unwrap();
            let mut eval = Evaluator::new();
            eval.prepare(&graph, &compiled, 2).unwrap();
            eval.eval(&bank_two(), &EventBank::new(), 0, 16, &mut out, &mut fired)
                .unwrap();
            assert_eq!(out.positions[0], want);
        }
    }

    #[test]
    fn blend_rotations_take_shortest_arc() {
        use std::f32::consts::FRAC_1_SQRT_2;
        let mut bank = ClipBank::new();
        bank.insert(
            20,
            Pose {
                positions: vec![[0.0; 3]],
                rotations: vec![IDENTITY],
            },
        )
        .unwrap();
        bank.insert(
            21,
            Pose {
                positions: vec![[0.0; 3]],
                rotations: vec![[0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2]],
            },
        )
        .unwrap();
        let mut graph = AnimGraph::new();
        graph.add(1, "a", NodeKind::Clip { clip: 20 });
        graph.add(2, "b", NodeKind::Clip { clip: 21 });
        graph.add(
            3,
            "mix",
            NodeKind::Blend {
                a: NodeId(1),
                b: NodeId(2),
                weight: 0.5,
            },
        );
        let compiled = graph.compile(NodeId(3)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 1).unwrap();
        let mut out = Pose::zero(1);
        eval.eval(&bank, &EventBank::new(), 0, 16, &mut out, &mut Vec::new())
            .unwrap();
        // Midpoint of identity to 90-deg yaw is the 45-deg yaw quat.
        for (got, want) in out.rotations[0]
            .iter()
            .zip([0.0, 0.38268343, 0.0, 0.923_879_5])
        {
            assert!((got - want).abs() < 1e-5, "{got} vs {want}");
        }
    }

    #[test]
    fn layer_adds_scaled_offsets_over_base() {
        let (_graph, _compiled, clips, events, mut eval) = prepared_chain();
        let mut out = Pose::zero(2);
        let mut fired = Vec::new();
        eval.eval(&clips, &events, 0, 16, &mut out, &mut fired)
            .unwrap();
        // Blend gives [1,0,0]/[3,0,0]; layer adds idle*[0.5] on top.
        assert_eq!(out.positions[0], [1.0, 0.0, 0.0]);
        assert_eq!(out.positions[1], [4.0, 0.0, 0.0]);
        assert_eq!(out.rotations, vec![IDENTITY, IDENTITY]);
    }

    #[test]
    fn emit_fires_frame_exact_in_track_order() {
        let (_graph, _compiled, clips, events, mut eval) = prepared_chain();
        let mut out = Pose::zero(2);
        let mut fired = Vec::new();
        eval.eval(&clips, &events, 5, 16, &mut out, &mut fired)
            .unwrap();
        assert_eq!(
            fired,
            vec![FiredEvent {
                node: NodeId(5),
                tick: 5,
                code: 100
            }]
        );
        eval.eval(&clips, &events, 6, 16, &mut out, &mut fired)
            .unwrap();
        assert_eq!(
            fired,
            vec![
                FiredEvent {
                    node: NodeId(5),
                    tick: 6,
                    code: 101
                },
                FiredEvent {
                    node: NodeId(5),
                    tick: 6,
                    code: 102
                },
            ]
        );
        // Off-by-one ticks stay silent.
        eval.eval(&clips, &events, 8, 16, &mut out, &mut fired)
            .unwrap();
        assert!(fired.is_empty());
        // Emit passes the layered pose through untouched.
        assert_eq!(out.positions[0], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn emit_nodes_fire_in_eval_order() {
        let graph = chain();
        let mut graph2 = graph.clone();
        graph2.add(
            6,
            "second",
            NodeKind::Emit {
                source: NodeId(1),
                track: 7,
            },
        );
        graph2.add(
            7,
            "root2",
            NodeKind::Blend {
                a: NodeId(5),
                b: NodeId(6),
                weight: 1.0,
            },
        );
        let compiled = graph2.compile(NodeId(7)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph2, &compiled, 2).unwrap();
        let mut out = Pose::zero(2);
        let mut fired = Vec::new();
        eval.eval(&bank_two(), &events_four(), 6, 16, &mut out, &mut fired)
            .unwrap();
        assert_eq!(
            fired,
            vec![
                FiredEvent {
                    node: NodeId(5),
                    tick: 6,
                    code: 101
                },
                FiredEvent {
                    node: NodeId(5),
                    tick: 6,
                    code: 102
                },
                FiredEvent {
                    node: NodeId(6),
                    tick: 6,
                    code: 101
                },
                FiredEvent {
                    node: NodeId(6),
                    tick: 6,
                    code: 102
                },
            ]
        );
    }

    #[test]
    fn replay_is_identical_across_runs() {
        let (_graph, _compiled, clips, events, mut eval) = prepared_chain();
        let mut first = Pose::zero(2);
        let mut second = Pose::zero(2);
        let mut fired_first = Vec::new();
        let mut fired_second = Vec::new();
        eval.eval(&clips, &events, 6, 16, &mut first, &mut fired_first)
            .unwrap();
        eval.eval(&clips, &events, 6, 16, &mut second, &mut fired_second)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(fired_first, fired_second);
    }

    #[test]
    fn missing_bank_entries_name_the_node() {
        let graph = chain();
        let compiled = graph.compile(NodeId(5)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 2).unwrap();
        let mut out = Pose::zero(2);
        let mut fired = Vec::new();
        assert_eq!(
            eval.eval(
                &ClipBank::new(),
                &events_four(),
                0,
                16,
                &mut out,
                &mut fired
            )
            .unwrap_err(),
            GraphError::UnknownClip {
                node: String::from("idle"),
                clip: 10
            }
        );
        assert_eq!(
            eval.eval(&bank_two(), &EventBank::new(), 0, 16, &mut out, &mut fired)
                .unwrap_err(),
            GraphError::UnknownTrack {
                node: String::from("cue"),
                track: 7
            }
        );
    }

    #[test]
    fn width_mismatches_are_refused() {
        let graph = chain();
        let compiled = graph.compile(NodeId(5)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 2).unwrap();
        let mut out = Pose::zero(3);
        let mut fired = Vec::new();
        assert_eq!(
            eval.eval(&bank_two(), &events_four(), 0, 16, &mut out, &mut fired)
                .unwrap_err(),
            GraphError::LengthMismatch {
                expected: 2,
                got: 3
            }
        );
        // A bank pose wider than the prepared width fails too.
        let mut wide = bank_two();
        wide.insert(10, pose([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]))
            .unwrap();
        wide.insert(
            12,
            Pose {
                positions: vec![[0.0; 3]; 3],
                rotations: vec![IDENTITY; 3],
            },
        )
        .unwrap();
        let mut graph = AnimGraph::new();
        graph.add(1, "wide", NodeKind::Clip { clip: 12 });
        let compiled = graph.compile(NodeId(1)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 2).unwrap();
        let mut out = Pose::zero(2);
        assert_eq!(
            eval.eval(&wide, &EventBank::new(), 0, 16, &mut out, &mut fired)
                .unwrap_err(),
            GraphError::LengthMismatch {
                expected: 2,
                got: 3
            }
        );
    }

    #[test]
    fn prepare_and_eval_reject_bad_shapes() {
        let graph = chain();
        let compiled = graph.compile(NodeId(5)).unwrap();
        let mut eval = Evaluator::new();
        assert_eq!(
            eval.prepare(&graph, &compiled, 0).unwrap_err(),
            GraphError::EmptyPose
        );
        let empty = CompiledGraph {
            order: Vec::new(),
            root: NodeId(5),
        };
        assert_eq!(
            eval.prepare(&graph, &empty, 2).unwrap_err(),
            GraphError::EmptyGraph
        );
        // Eval before prepare is loud, not silent zeros.
        let mut out = Pose::zero(2);
        assert_eq!(
            eval.eval(
                &bank_two(),
                &events_four(),
                0,
                16,
                &mut out,
                &mut Vec::new()
            )
            .unwrap_err(),
            GraphError::NotPrepared
        );
        // A graph edited after compile fails at prepare.
        let mut edited = chain();
        edited.nodes.retain(|node| node.id != NodeId(1));
        assert_eq!(
            eval.prepare(&edited, &compiled, 2).unwrap_err(),
            GraphError::UnknownRef {
                node: String::from("prepare"),
                missing: NodeId(1)
            }
        );
        // Negative time steps are refused.
        let (_g, _c, clips, events, mut eval) = prepared_chain();
        assert_eq!(
            eval.eval(&clips, &events, 0, -1, &mut out, &mut Vec::new())
                .unwrap_err(),
            GraphError::BadTime(-1)
        );
        assert_eq!(eval.joints(), 2);
    }

    #[test]
    fn bank_inserts_validate_and_replace() {
        let mut bank = ClipBank::new();
        assert!(bank.is_empty());
        assert_eq!(bank.len(), 0);
        let mut nan = pose([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        nan.positions[1][0] = f32::NAN;
        assert_eq!(
            bank.insert(1, nan).unwrap_err(),
            GraphError::NonFinite("clip.positions")
        );
        let mut nan_rot = pose([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        nan_rot.rotations[0][3] = f32::INFINITY;
        assert_eq!(
            bank.insert(1, nan_rot).unwrap_err(),
            GraphError::NonFinite("clip.rotations")
        );
        assert_eq!(
            bank.insert(
                1,
                Pose {
                    positions: vec![[0.0; 3]],
                    rotations: vec![IDENTITY, IDENTITY],
                }
            )
            .unwrap_err(),
            GraphError::LengthMismatch {
                expected: 1,
                got: 2
            }
        );
        assert_eq!(
            bank.insert(
                1,
                Pose {
                    positions: Vec::new(),
                    rotations: Vec::new(),
                }
            )
            .unwrap_err(),
            GraphError::EmptyPose
        );
        // Out-of-order keys stay sorted; replace keeps the key count.
        bank.insert(30, pose([3.0, 0.0, 0.0], [3.0, 0.0, 0.0]))
            .unwrap();
        bank.insert(10, pose([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]))
            .unwrap();
        bank.insert(20, pose([2.0, 0.0, 0.0], [2.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(bank.len(), 3);
        assert_eq!(bank.get(20).unwrap().positions[0], [2.0, 0.0, 0.0]);
        assert!(bank.get(99).is_none());
        bank.insert(20, pose([9.0, 0.0, 0.0], [9.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(bank.len(), 3);
        assert_eq!(bank.get(20).unwrap().positions[0], [9.0, 0.0, 0.0]);
    }

    #[test]
    fn event_tracks_sort_and_slice_frame_exact() {
        let mut bank = EventBank::new();
        assert!(bank.is_empty());
        bank.insert(
            3,
            vec![
                TimedEvent { tick: 9, code: 2 },
                TimedEvent { tick: 9, code: 1 },
                TimedEvent { tick: 4, code: 7 },
            ],
        );
        assert_eq!(bank.len(), 1);
        assert_eq!(
            bank.at(3, 9),
            &[
                TimedEvent { tick: 9, code: 1 },
                TimedEvent { tick: 9, code: 2 }
            ]
        );
        assert_eq!(bank.at(3, 4), &[TimedEvent { tick: 4, code: 7 }]);
        assert!(bank.at(3, 5).is_empty());
        assert!(bank.at(99, 9).is_empty());
        bank.insert(3, vec![TimedEvent { tick: 1, code: 1 }]);
        assert_eq!(bank.len(), 1);
        assert_eq!(bank.at(3, 1).len(), 1);
    }

    #[test]
    fn steady_state_eval_allocates_nothing() {
        let (_graph, _compiled, clips, events, mut eval) = prepared_chain();
        let mut out = Pose::zero(2);
        let mut fired: Vec<FiredEvent> = Vec::with_capacity(256);
        // Warm up outside the probe, then count only the measured window.
        for tick in 0..8 {
            eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                .unwrap();
        }
        // Ticks 5..7 fire cues, proving event push allocates nothing either.
        let ((), heap) = measure(|| {
            for tick in 0..60 {
                eval.eval(&clips, &events, tick, 16, &mut out, &mut fired)
                    .unwrap();
            }
        });
        assert_eq!(heap.calls, 0, "steady-state eval allocated");
        // Repeat prepares with the same shape reuse scratch too.
        let (graph, compiled, _, _, _) = prepared_chain();
        let before = eval.slots_capacity();
        eval.prepare(&graph, &compiled, 2).unwrap();
        assert_eq!(eval.slots_capacity(), before);
    }

    #[test]
    fn repeat_prepare_same_shape_allocates_nothing() {
        let (graph, compiled, _, _, mut eval) = prepared_chain();
        // One repeat prepare outside the probe: buffers grow here, never
        // inside the measured window.
        eval.prepare(&graph, &compiled, 2).unwrap();
        let ((), heap) = measure(|| {
            for _ in 0..16 {
                eval.prepare(&graph, &compiled, 2).unwrap();
            }
        });
        assert_eq!(
            heap.calls, 0,
            "repeat prepare allocated: {} calls, {} bytes",
            heap.calls, heap.bytes
        );
    }

    #[test]
    fn errors_display_loudly() {
        let err = GraphError::DuplicateId(NodeId(4));
        assert!(err.to_string().contains('4'), "{err}");
        let err = GraphError::BadTime(-3);
        assert!(err.to_string().contains("-3"), "{err}");
    }

    /// One shared eval-path world for the audit: graph eval plus space
    /// tick, masked layers, event routing, foot locks, full-body reach
    /// and hit-react. Setup allocates once; ticks reuse every buffer.
    #[derive(Clone)]
    struct Am16World {
        clips: ClipBank,
        tracks: EventBank,
        eval: Evaluator,
        graph_out: Pose,
        fired: Vec<FiredEvent>,
        router: crate::events::Router,
        log: crate::events::CallbackLog,
        space: crate::space::BlendSpace,
        space_clips: ClipBank,
        player: crate::space::SpacePlayer,
        space_out: Pose,
        layers_base: Pose,
        layers_arms: Pose,
        layers_hit: Pose,
        arms_mask: crate::layers::MaskTable,
        add_mask: crate::layers::MaskTable,
        layers_out: Pose,
        locker: crate::feet::FootLocker,
        fbik_pose: Pose,
        fbik_chains: crate::fbik::BodyChains,
        fbik_legs: [crate::feet::LegMap; 2],
        fbik_locker: crate::feet::FootLocker,
        hit_base: Pose,
        hit_mask: crate::layers::MaskTable,
        hit_strike: crate::hit_react::HitStrike,
        hit_out: Pose,
        hit_scratch: Pose,
        warp: Vec<[f32; 2]>,
    }

    /// Fresh world with all scratch prepared and the traversal warp baked.
    /// The warped end lands exactly on the edge; that assert is the
    /// traversal-tape wiring receipt.
    fn am16_world() -> Am16World {
        let graph = chain();
        let compiled = graph.compile(NodeId(5)).unwrap();
        let mut eval = Evaluator::new();
        eval.prepare(&graph, &compiled, 2).unwrap();
        let clips = bank_two();
        let mut tracks = EventBank::new();
        tracks.insert(
            7,
            vec![
                TimedEvent { tick: 5, code: 1 },
                TimedEvent { tick: 6, code: 1 },
                TimedEvent { tick: 6, code: 2 },
                TimedEvent { tick: 20, code: 2 },
            ],
        );
        let router = crate::events::Router::with_footstep_hit();
        let log = crate::events::CallbackLog::with_capacity(256);
        let space = crate::space::bake(vec![
            crate::space::SpaceSample::new(0, 0, 10).unwrap(),
            crate::space::SpaceSample::new(2048, 0, 11).unwrap(),
        ])
        .unwrap();
        let mut space_clips = ClipBank::new();
        space_clips
            .insert(
                10,
                Pose {
                    positions: vec![[0.0; 3], [0.0; 3]],
                    rotations: vec![IDENTITY, IDENTITY],
                },
            )
            .unwrap();
        space_clips
            .insert(
                11,
                Pose {
                    positions: vec![[4.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
                    rotations: vec![IDENTITY, IDENTITY],
                },
            )
            .unwrap();
        let mut player = crate::space::SpacePlayer::new();
        player.prepare(2, 2);
        let layers_base = Pose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, 0.5, 0.1],
                [0.2, 0.5, -0.1],
                [-0.3, 1.4, 0.0],
                [0.3, 1.4, 0.0],
                [0.0, 1.2, 0.0],
            ],
            rotations: vec![IDENTITY; 6],
        };
        let layers_arms = Pose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, 0.5, 0.1],
                [0.2, 0.5, -0.1],
                [-0.5, 1.5, 0.4],
                [0.5, 1.5, 0.4],
                [0.0, 1.2, 0.0],
            ],
            rotations: vec![IDENTITY; 6],
        };
        let layers_hit = Pose {
            positions: vec![
                [-0.1, 0.0, -0.2],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                [-0.15, 0.05, -0.3],
            ],
            rotations: vec![IDENTITY; 6],
        };
        let arms_mask = crate::layers::MaskTable::from_vec(vec![0, 0, 0, 255, 255, 0]).unwrap();
        let add_mask = crate::layers::MaskTable::from_vec(vec![255, 0, 0, 0, 0, 255]).unwrap();
        let fbik_pose = Pose {
            positions: vec![
                [0.0, 0.95, 0.0],
                [0.0, 1.20, 0.0],
                [0.0, 1.50, 0.0],
                [0.22, 1.42, 0.0],
                [0.24, 1.12, 0.02],
                [0.26, 0.70, 0.05],
                [-0.22, 1.42, 0.0],
                [-0.24, 1.12, 0.02],
                [-0.26, 0.70, 0.05],
                [0.11, 0.90, 0.0],
                [0.11, 0.50, 0.02],
                [0.11, 0.05, 0.05],
                [-0.11, 0.90, 0.0],
                [-0.11, 0.50, -0.02],
                [-0.11, 0.12, -0.05],
            ],
            rotations: vec![IDENTITY; 15],
        };
        let fbik_chains = crate::fbik::BodyChains {
            arm_l: crate::fbik::ArmChain::new(3, 4, 5),
            arm_r: crate::fbik::ArmChain::new(6, 7, 8),
            head: 2,
            aim: 1,
        };
        let fbik_legs = [
            crate::feet::LegMap::new(9, 10, 11, 0).expect("left map"),
            crate::feet::LegMap::new(12, 13, 14, 1).expect("right map"),
        ];
        let hit_base = layers_base.clone();
        let hit_mask = crate::hit_react::hit_mask();
        let hit_strike = crate::hit_react::HitStrike::new(1, 512, 8).expect("strike");
        let probe = crate::traversal::Probe {
            height_m: 1.0,
            depth_m: 0.6,
            gap_m: 0.5,
            speed_m_s: 2.0,
            has_hold: false,
        };
        let edge = [4.0, 1.0];
        let plan = crate::traversal::plan(&probe, edge).expect("traversal plans");
        let warp = crate::traversal::apply_warp(&plan, &[[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]])
            .expect("traversal warps");
        assert_eq!(
            crate::traversal::warp_error_mm(*warp.last().expect("warped nonempty"), edge),
            0.0
        );
        Am16World {
            clips,
            tracks,
            eval,
            graph_out: Pose::zero(2),
            fired: Vec::with_capacity(8),
            router,
            log,
            space,
            space_clips,
            player,
            space_out: Pose::zero(2),
            layers_base,
            layers_arms,
            layers_hit,
            arms_mask,
            add_mask,
            layers_out: Pose::zero(6),
            locker: crate::feet::FootLocker::with_defaults(),
            fbik_pose,
            fbik_chains,
            fbik_legs,
            fbik_locker: crate::feet::FootLocker::with_defaults(),
            hit_base,
            hit_mask,
            hit_strike,
            hit_out: Pose::zero(6),
            hit_scratch: Pose::zero(6),
            warp,
        }
    }

    /// One scripted tick across the whole eval path: locomotion params
    /// plus interrupts, traversal-warped feet, sweeping reach and a
    /// hit-react strike. Steady-state calls allocate nothing.
    fn am16_tick(world: &mut Am16World, tick: u32) {
        world
            .eval
            .eval(
                &world.clips,
                &world.tracks,
                tick,
                16,
                &mut world.graph_out,
                &mut world.fired,
            )
            .expect("graph tick");
        world
            .log
            .collect(tick, &world.fired, &world.router)
            .expect("event tick");
        const SCRIPT: [(i32, i32, bool); 6] = [
            (0, 0, false),
            (1024, 0, false),
            (2048, 0, true),
            (2048, 450, false),
            (0, -900, true),
            (1024, 0, false),
        ];
        let (speed_q, angle_q, interrupt) = SCRIPT[tick as usize % SCRIPT.len()];
        let params = crate::space::SpaceParams::from_fixed(speed_q, angle_q)
            .expect("script params in range");
        world
            .player
            .tick(
                &world.space,
                &world.space_clips,
                params,
                interrupt,
                &mut world.space_out,
            )
            .expect("space tick");
        crate::layers::compose(
            &world.layers_base,
            &world.layers_arms,
            &world.layers_hit,
            &world.arms_mask,
            &world.add_mask,
            crate::layers::GAIN_Q8_ONE,
            &mut world.layers_out,
        )
        .expect("layers tick");
        let warped = world.warp[tick as usize % world.warp.len()];
        let planted = tick.is_multiple_of(2);
        world
            .locker
            .update(0, [warped[0], 0.03, warped[1]], planted)
            .expect("left foot");
        world
            .locker
            .update(1, [warped[0], 0.03, -warped[1]], !planted)
            .expect("right foot");
        let phase = (tick % 64) as f32 / 64.0;
        let targets = crate::fbik::BodyTargets {
            hand_l: None,
            hand_r: Some([-0.10 - phase * 0.02, 1.15, 0.30 + phase * 0.10]),
            look: Some([0.0, 1.45, 2.0]),
            aim: Some([0.0, 1.10, 2.0]),
            grip: crate::fbik::GripMode::Single,
            shield_l: None,
        };
        crate::fbik::combat_reach_tick(
            &mut world.fbik_pose,
            &world.fbik_legs,
            [planted, !planted],
            &mut world.fbik_locker,
            &world.fbik_chains,
            &targets,
        )
        .expect("reach tick");
        crate::hit_react::apply_hit_react(
            &world.hit_base,
            &world.hit_strike,
            tick,
            &world.hit_mask,
            &mut world.hit_out,
            &mut world.hit_scratch,
        )
        .expect("hit-react tick");
    }

    /// One digest over every eval-path output plus the routed event tape.
    /// Pure integer and bit work; allocates nothing.
    fn am16_digest(world: &Am16World) -> u64 {
        // Graph lanes enter through the shared bit-hash; the rest feeds
        // the same FNV-1a64 stream without allocating.
        let mut hash = Fnv64::with_state(pose_bits(&world.graph_out));
        for lane in world
            .space_out
            .positions
            .iter()
            .flatten()
            .chain(world.space_out.rotations.iter().flatten())
            .chain(world.space_out.rotations.iter().flatten())
            .chain(world.layers_out.positions.iter().flatten())
            .chain(world.layers_out.rotations.iter().flatten())
            .chain(world.fbik_pose.positions.iter().flatten())
            .chain(world.fbik_pose.rotations.iter().flatten())
            .chain(world.hit_out.positions.iter().flatten())
            .chain(world.hit_out.rotations.iter().flatten())
        {
            hash.write_f32(*lane);
        }
        hash.write_len(world.log.len());
        for call in world.log.calls() {
            hash.write(&call.tick.to_le_bytes());
            hash.write(&call.code.to_le_bytes());
            hash.write(&call.node.0.to_le_bytes());
        }
        // Anchor lanes prove the foot tape; 0/1 always resolve here.
        for foot in 0..2usize {
            if let Some(anchor) = world.locker.anchor(foot).expect("anchor in range") {
                for lane in anchor {
                    hash.write_f32(lane);
                }
            }
            hash.write_u8(foot as u8);
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
    fn am16_eval_path_replays_x2_and_restores_equal() {
        let run = || {
            let mut world = am16_world();
            let mut digests = Vec::with_capacity(64);
            for tick in 0..64u32 {
                am16_tick(&mut world, tick);
                digests.push(am16_digest(&world));
            }
            digests
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "eval path must replay x2");
        assert_ne!(first[0], first[63], "scripted tape must move");
        // Routed-cue receipt: ticks 5, 6 (x2) and 20 each land in the log.
        let mut wired = am16_world();
        for tick in 0..32u32 {
            am16_tick(&mut wired, tick);
        }
        assert_eq!(wired.log.len(), 4);
        // Snapshot mid-tape, run out, restore, run out again.
        let mut world = am16_world();
        for tick in 0..16u32 {
            am16_tick(&mut world, tick);
        }
        let snapshot = world.clone();
        let mut reference = Vec::with_capacity(48);
        for tick in 16..64u32 {
            am16_tick(&mut world, tick);
            reference.push(am16_digest(&world));
        }
        world.clone_from(&snapshot);
        let mut replay = Vec::with_capacity(48);
        for tick in 16..64u32 {
            am16_tick(&mut world, tick);
            replay.push(am16_digest(&world));
        }
        assert_eq!(reference, replay, "restore-then-eval must equal");
    }

    #[test]
    fn am16_eval_path_allocates_nothing_per_tick() {
        let mut world = am16_world();
        for tick in 0..8u32 {
            am16_tick(&mut world, tick);
        }
        let ((), heap) = measure(|| {
            for tick in 8..72u32 {
                am16_tick(&mut world, tick);
                std::hint::black_box(am16_digest(&world));
            }
        });
        assert_eq!(
            heap.calls, 0,
            "eval path must not allocate per tick after warmup"
        );
    }
}
