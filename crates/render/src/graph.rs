//! graph: render-graph as data. Null order == wgpu order, hashed.

use std::error::Error;
use std::fmt;
use std::time::Instant;

/// Named pass node index.
pub type NodeId = usize;

/// DAG of render passes: names + edges (producer → consumer).
pub struct Graph {
    names: Vec<String>,
    edges: Vec<(NodeId, NodeId)>,
}

impl Graph {
    /// Construct empty graph.
    pub fn new() -> Self {
        Self {
            names: vec![],
            edges: vec![],
        }
    }
    /// Add a named pass, returns its id.
    pub fn add_node(&mut self, name: &str) -> NodeId {
        self.names.push(name.to_string());
        self.names.len() - 1
    }
    /// Add a dependency edge (panics on out-of-range in debug, ignored in release).
    pub fn add_edge(&mut self, from: NodeId, to: NodeId) {
        debug_assert!(from < self.names.len() && to < self.names.len());
        if from < self.names.len() && to < self.names.len() {
            self.edges.push((from, to));
        }
    }
    /// Deterministic topological order (Kahn, smallest id first). None on cycle.
    pub fn order(&self) -> Option<Vec<NodeId>> {
        let n = self.names.len();
        let mut indeg = vec![0usize; n];
        let mut out: Vec<Vec<NodeId>> = vec![vec![]; n];
        for (a, b) in &self.edges {
            out[*a].push(*b);
            indeg[*b] += 1;
        }
        let mut ready: Vec<NodeId> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut done = vec![];
        while !ready.is_empty() {
            ready.sort_unstable();
            let id = ready.remove(0);
            done.push(id);
            let mut next = vec![];
            for m in out[id].drain(..) {
                indeg[m] -= 1;
                if indeg[m] == 0 {
                    next.push(m);
                }
            }
            ready.extend(next);
        }
        if done.len() == n {
            Some(done)
        } else {
            None
        }
    }
    /// FNV-1a64 over names + edges + order (graph-as-data hash gate).
    /// Hashed with the one owner [`engine_core::Fnv64`], so the digest
    /// matches every other crate's FNV-1a 64 byte for byte.
    pub fn graph_hash(&self) -> u64 {
        let mut hasher = engine_core::Fnv64::new();
        for name in &self.names {
            hasher.write(name.as_bytes());
            hasher.write_u8(0xFF);
        }
        for (a, b) in &self.edges {
            hasher.write_u64(u64::try_from(*a).unwrap_or(u64::MAX));
            hasher.write_u64(u64::try_from(*b).unwrap_or(u64::MAX));
        }
        hasher.finish()
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

// ----- RA2: passes, resources, transient aliasing, CPU execution -----
// Declared passes run through `Graph::order`, alias transient bytes by
// non-overlapping lifetime, record explicit barriers, then execute
// recorded CPU fns into one receipt. Owner: render lane; next consumer
// RA8 diagnostics (RA1 presenter after). Units: bytes, whole ms,
// integer percent. Errors: `GraphError`; GPU submit is loud `GpuUnsupported`.

/// Pass index in a [`FrameGraph`]; identical to [`NodeId`] so the
/// [`Graph::order`] core applies unchanged.
pub type PassId = NodeId;

/// Transient attachment: graph-owned backing bytes with a lifetime.
/// Explicit `first`/`last` are add-order pass ids that must cover every
/// use; `None` derives the tight span from the owner plus readers.
#[derive(Clone, Debug)]
pub struct TransientDesc {
    /// Resource name; matches entries in pass `inputs`/`outputs`.
    pub name: String,
    /// Backing size in bytes.
    pub size_bytes: u64,
    /// Explicit first pass id, or `None` to derive.
    pub first: Option<u32>,
    /// Explicit last pass id, or `None` to derive.
    pub last: Option<u32>,
}

impl TransientDesc {
    /// Transient live exactly where it is used.
    pub fn new(name: &str, size_bytes: u64) -> Self {
        Self {
            name: name.to_string(),
            size_bytes,
            first: None,
            last: None,
        }
    }
    /// Transient with an explicit lifetime; `compile` refuses spans that
    /// miss a use or leave the pass range (`GraphError::BadLifetime`).
    pub fn spanned(name: &str, size_bytes: u64, first: u32, last: u32) -> Self {
        Self {
            name: name.to_string(),
            size_bytes,
            first: Some(first),
            last: Some(last),
        }
    }
}

/// One pass declaration: persistent inputs/outputs plus transient scratch
/// and the CPU work it records (`draws`, `triangles`).
#[derive(Clone, Debug, Default)]
pub struct PassDesc {
    /// Pass name; also the [`Graph`] node label.
    pub name: String,
    /// Persistent resources read (owned elsewhere or cross-frame).
    pub inputs: Vec<String>,
    /// Persistent resources written.
    pub outputs: Vec<String>,
    /// Transient scratch with lifetimes (see [`TransientDesc`]).
    pub transients: Vec<TransientDesc>,
    /// Draw calls recorded by this pass.
    pub draws: u64,
    /// Triangles recorded by this pass.
    pub triangles: u64,
}

impl PassDesc {
    /// Empty pass with a name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }
    /// Declare a persistent input.
    pub fn input(mut self, resource: &str) -> Self {
        self.inputs.push(resource.to_string());
        self
    }
    /// Declare a persistent output.
    pub fn output(mut self, resource: &str) -> Self {
        self.outputs.push(resource.to_string());
        self
    }
    /// Declare transient scratch with a derived lifetime.
    pub fn transient(mut self, name: &str, size_bytes: u64) -> Self {
        self.transients.push(TransientDesc::new(name, size_bytes));
        self
    }
    /// Declare transient scratch with an explicit lifetime.
    pub fn transient_span(mut self, name: &str, size_bytes: u64, first: u32, last: u32) -> Self {
        self.transients
            .push(TransientDesc::spanned(name, size_bytes, first, last));
        self
    }
    /// Recorded CPU work for the receipt.
    pub fn work(mut self, draws: u64, triangles: u64) -> Self {
        self.draws = draws;
        self.triangles = triangles;
        self
    }
}

/// Typed frame-graph failures: cycles name their nodes, never `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// Dependency cycle; `nodes` holds the pass names around the loop.
    Cycle {
        /// Pass names around the detected loop, in loop order.
        nodes: Vec<String>,
    },
    /// Edge or fn recorded against a pass id that was never added.
    UnknownPass {
        /// The rejected pass id.
        id: PassId,
    },
    /// Explicit transient span outside the pass range or missing a use.
    BadLifetime {
        /// Transient resource name.
        name: String,
        /// Rejected first compiled position.
        first: usize,
        /// Rejected last compiled position.
        last: usize,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cycle { nodes } => write!(f, "render graph cycle: {}", nodes.join(" -> ")),
            Self::UnknownPass { id } => write!(f, "unknown render pass id {id}"),
            Self::BadLifetime { name, first, last } => {
                write!(
                    f,
                    "transient `{name}` span [{first}, {last}] misses a use or the pass range"
                )
            }
        }
    }
}

impl Error for GraphError {}

/// Loud GPU-submit stub (headless twin): the CPU executor records only, so
/// submission names the missing wgpu device plus bind groups instead of
/// pretending. The window build gates the stub out and submits for real;
/// the type stays for the headless twin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuUnsupported(pub String);

impl fmt::Display for GpuUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gpu submit unsupported: {}", self.0)
    }
}

impl Error for GpuUnsupported {}

/// Frame identity for one execution: index plus target size in pixels.
#[derive(Clone, Debug)]
pub struct FrameInfo {
    /// Monotonic frame index for receipts.
    pub frame_index: u64,
    /// Target width in pixels.
    pub width_px: u32,
    /// Target height in pixels.
    pub height_px: u32,
}

/// Per-pass scratch pad handed to recorded fns; extras add to the receipt.
#[derive(Debug)]
pub struct PassCtx {
    pass: PassId,
    frame_index: u64,
    extra_draws: u64,
    extra_triangles: u64,
}

impl PassCtx {
    /// Pass id running now.
    pub fn pass(&self) -> PassId {
        self.pass
    }
    /// Frame index running now.
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }
    /// Add draws/triangles found at record time (saturates, never wraps).
    pub fn drew(&mut self, draws: u64, triangles: u64) {
        self.extra_draws = self.extra_draws.saturating_add(draws);
        self.extra_triangles = self.extra_triangles.saturating_add(triangles);
    }
}

/// Barrier kind: layout transitions plus alias acquire/release markers.
/// Alias markers carry `from == to ==` the pass where a shared slot is
/// taken or freed; same-position sharing assumes the pass reads released
/// resources before writing acquired ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarrierKind {
    /// Producer `from` hands a resource to consumer `to`.
    Transition,
    /// Shared slot taken at the start of `from`.
    AliasAcquire,
    /// Shared slot freed at the end of `to`.
    AliasRelease,
}

fn barrier_rank(kind: BarrierKind) -> u8 {
    match kind {
        BarrierKind::Transition => 0,
        BarrierKind::AliasAcquire => 1,
        BarrierKind::AliasRelease => 2,
    }
}

/// One explicit ordering edge between passes, in pass ids.
#[derive(Clone, Debug)]
pub struct Barrier {
    /// Resource handed over or slot renamed.
    pub resource: String,
    /// Producer (or acquire) pass.
    pub from: PassId,
    /// Consumer (or release) pass.
    pub to: PassId,
    /// What the barrier does.
    pub kind: BarrierKind,
}

/// One transient placement: closed lifetime plus shared slot.
/// `offset` is always zero (whole-slot aliasing); kept for RA8 spans.
#[derive(Clone, Debug)]
pub struct TransientAlloc {
    /// Transient resource name.
    pub name: String,
    /// Backing size in bytes.
    pub size_bytes: u64,
    /// First compiled-order position using it.
    pub first: usize,
    /// Last compiled-order position using it.
    pub last: usize,
    /// Backing slot shared with disjoint-lifetime transients.
    pub slot: usize,
    /// Byte offset inside the slot (zero: whole-slot aliasing).
    pub offset: u64,
}

/// Per-pass CPU timing in whole ms.
#[derive(Clone, Debug)]
pub struct PassTiming {
    /// Pass name.
    pub pass: String,
    /// Wall time spent recording this pass, in whole ms.
    pub cpu_ms: u64,
}

/// One executed frame: draws, triangles, per-pass CPU ms, transient
/// bytes plus the integer reuse percent. Units: bytes, ms, percent.
#[derive(Clone, Debug)]
pub struct FrameReceipt {
    /// Frame index executed.
    pub frame_index: u64,
    /// Target width in pixels.
    pub width_px: u32,
    /// Target height in pixels.
    pub height_px: u32,
    /// Draw calls: declared work plus fn extras.
    pub draws: u64,
    /// Triangles: declared work plus fn extras.
    pub triangles: u64,
    /// CPU ms per pass, in compiled order.
    pub per_pass_cpu_ms: Vec<PassTiming>,
    /// Sum of per-pass CPU ms.
    pub total_cpu_ms: u64,
    /// Sum of transient sizes in bytes (before aliasing).
    pub transient_bytes_total: u64,
    /// Backing bytes after aliasing (sum of slot peaks).
    pub transient_bytes_backing: u64,
    /// Bytes in shared slots over total transient bytes, 0..=100.
    pub transient_reuse_pct: u32,
}

/// Recorded pass fn: runs at `execute`, may add work via [`PassCtx`].
type PassFn = Box<dyn FnMut(&mut PassCtx) + Send>;

/// Frame graph builder: passes as data plus explicit edges.
/// Resource flow (`outputs` into `inputs`) infers the remaining edges.
#[derive(Default)]
pub struct FrameGraph {
    passes: Vec<PassDesc>,
    edges: Vec<(PassId, PassId)>,
    fns: Vec<Option<PassFn>>,
}

impl FrameGraph {
    /// Empty builder.
    pub fn new() -> Self {
        Self::default()
    }
    /// Add a pass declaration; returns its id.
    pub fn add_pass(&mut self, desc: PassDesc) -> PassId {
        let id = self.passes.len();
        self.passes.push(desc);
        self.fns.push(None);
        id
    }
    /// Record the CPU fn a pass runs at `execute`.
    pub fn set_pass_fn(
        &mut self,
        id: PassId,
        f: impl FnMut(&mut PassCtx) + Send + 'static,
    ) -> Result<(), GraphError> {
        if id >= self.passes.len() {
            return Err(GraphError::UnknownPass { id });
        }
        self.fns[id] = Some(Box::new(f));
        Ok(())
    }
    /// Explicit dependency edge (producer `from` before consumer `to`).
    pub fn add_edge(&mut self, from: PassId, to: PassId) -> Result<(), GraphError> {
        if from >= self.passes.len() {
            return Err(GraphError::UnknownPass { id: from });
        }
        if to >= self.passes.len() {
            return Err(GraphError::UnknownPass { id: to });
        }
        if !self.edges.contains(&(from, to)) {
            self.edges.push((from, to));
        }
        Ok(())
    }
    /// Pass count.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }
    /// Pass declaration by id.
    pub fn pass(&self, id: PassId) -> Option<&PassDesc> {
        self.passes.get(id)
    }
    /// Order, alias, emit barriers; refuses cycles with their node names.
    /// Consumes the builder; the plan moves into [`CompiledGraph`].
    pub fn compile(self) -> Result<CompiledGraph, GraphError> {
        let n = self.passes.len();
        let mut dag = Graph::new();
        for p in &self.passes {
            dag.add_node(&p.name);
        }
        let mut edge_set: Vec<(PassId, PassId)> = Vec::with_capacity(self.edges.len());
        for &(a, b) in &self.edges {
            if !edge_set.contains(&(a, b)) {
                edge_set.push((a, b));
            }
        }
        let mut resources: Vec<&str> = Vec::new();
        for p in &self.passes {
            for r in p.inputs.iter().chain(p.outputs.iter()) {
                if !resources.contains(&r.as_str()) {
                    resources.push(r.as_str());
                }
            }
        }
        resources.sort_unstable();
        // Producer -> consumer, skipping read-write pairs (explicit
        // edges sequence those; guessing would invent cycles).
        for &r in &resources {
            for (a, pa) in self.passes.iter().enumerate() {
                if !pa.outputs.iter().any(|o| o == r) {
                    continue;
                }
                for (c, pc) in self.passes.iter().enumerate() {
                    if a == c
                        || pa.inputs.iter().any(|i| i == r)
                        || pc.outputs.iter().any(|o| o == r)
                        || !pc.inputs.iter().any(|i| i == r)
                    {
                        continue;
                    }
                    if !edge_set.contains(&(a, c)) {
                        edge_set.push((a, c));
                    }
                }
            }
        }
        for &(a, b) in &edge_set {
            dag.add_edge(a, b);
        }
        let order = match dag.order() {
            Some(order) => order,
            None => {
                let mut adj: Vec<Vec<NodeId>> = vec![Vec::new(); n];
                for &(a, b) in &edge_set {
                    adj[a].push(b);
                }
                for peers in adj.iter_mut() {
                    peers.sort_unstable();
                    peers.dedup();
                }
                let ids = find_cycle(n, &adj);
                let nodes = ids.iter().map(|&i| self.passes[i].name.clone()).collect();
                return Err(GraphError::Cycle { nodes });
            }
        };
        let mut pos = vec![0usize; n];
        for (index, &id) in order.iter().enumerate() {
            pos[id] = index;
        }
        // Transient lifetimes in compiled-order positions.
        let mut rows: Vec<LifeRow> = Vec::new();
        for (owner, p) in self.passes.iter().enumerate() {
            for t in &p.transients {
                let mut first = pos[owner];
                let mut last = pos[owner];
                for (id, q) in self.passes.iter().enumerate() {
                    if q.inputs.iter().any(|i| i == &t.name)
                        || q.outputs.iter().any(|o| o == &t.name)
                    {
                        if pos[id] < first {
                            first = pos[id];
                        }
                        if pos[id] > last {
                            last = pos[id];
                        }
                    }
                }
                let mut ef = first;
                let mut el = last;
                if let Some(v) = t.first {
                    match pos.get(v as usize) {
                        Some(&mapped) => ef = mapped,
                        None => {
                            return Err(GraphError::BadLifetime {
                                name: t.name.clone(),
                                first: v as usize,
                                last,
                            });
                        }
                    }
                }
                if let Some(v) = t.last {
                    match pos.get(v as usize) {
                        Some(&mapped) => el = mapped,
                        None => {
                            return Err(GraphError::BadLifetime {
                                name: t.name.clone(),
                                first: ef,
                                last: v as usize,
                            });
                        }
                    }
                }
                if ef > el || ef > first || el < last {
                    return Err(GraphError::BadLifetime {
                        name: t.name.clone(),
                        first: ef,
                        last: el,
                    });
                }
                rows.push(LifeRow {
                    name: t.name.clone(),
                    size_bytes: t.size_bytes,
                    first: ef,
                    last: el,
                });
            }
        }
        // Largest-first sweep (ties: first, then name); a row joins the
        // first slot whose members are all strictly disjoint. Twin of
        // `executor::plan_aliases`; unify once both modules register.
        let mut seq: Vec<usize> = (0..rows.len()).collect();
        seq.sort_by(|&a, &b| {
            rows[b]
                .size_bytes
                .cmp(&rows[a].size_bytes)
                .then_with(|| rows[a].first.cmp(&rows[b].first))
                .then_with(|| rows[a].name.cmp(&rows[b].name))
        });
        let mut slots: Vec<Vec<usize>> = Vec::new();
        let mut slot_of = vec![0usize; rows.len()];
        for &ri in &seq {
            let mut placed = usize::MAX;
            for (si, slot) in slots.iter().enumerate() {
                let fits = slot
                    .iter()
                    .all(|&m| rows[m].last < rows[ri].first || rows[ri].last < rows[m].first);
                if fits {
                    placed = si;
                    break;
                }
            }
            if placed == usize::MAX {
                placed = slots.len();
                slots.push(Vec::new());
            }
            slots[placed].push(ri);
            slot_of[ri] = placed;
        }
        let mut allocs: Vec<TransientAlloc> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| TransientAlloc {
                name: r.name.clone(),
                size_bytes: r.size_bytes,
                first: r.first,
                last: r.last,
                slot: slot_of[i],
                offset: 0,
            })
            .collect();
        allocs.sort_by(|a, b| {
            a.slot
                .cmp(&b.slot)
                .then_with(|| a.first.cmp(&b.first))
                .then_with(|| a.name.cmp(&b.name))
        });
        let mut total_bytes = 0u64;
        let mut backing_bytes = 0u64;
        let mut shared_bytes = 0u64;
        for slot in &slots {
            let mut peak = 0u64;
            for &m in slot {
                total_bytes = total_bytes.saturating_add(rows[m].size_bytes);
                peak = peak.max(rows[m].size_bytes);
            }
            backing_bytes = backing_bytes.saturating_add(peak);
            if slot.len() > 1 {
                for &m in slot {
                    shared_bytes = shared_bytes.saturating_add(rows[m].size_bytes);
                }
            }
        }
        let reuse_pct = if total_bytes == 0 {
            100
        } else {
            u32::try_from(shared_bytes as u128 * 100 / total_bytes as u128).unwrap_or(100)
        };
        // Explicit barriers: producer -> consumer transitions plus
        // acquire/release markers for every shared slot member.
        let mut barriers: Vec<Barrier> = Vec::new();
        for &r in &resources {
            for (a, pa) in self.passes.iter().enumerate() {
                if !pa.outputs.iter().any(|o| o == r) || pa.inputs.iter().any(|i| i == r) {
                    continue;
                }
                for (c, pc) in self.passes.iter().enumerate() {
                    if a == c
                        || pc.outputs.iter().any(|o| o == r)
                        || !pc.inputs.iter().any(|i| i == r)
                    {
                        continue;
                    }
                    barriers.push(Barrier {
                        resource: r.to_string(),
                        from: a,
                        to: c,
                        kind: BarrierKind::Transition,
                    });
                }
            }
        }
        for slot in &slots {
            if slot.len() < 2 {
                continue;
            }
            for &m in slot {
                let at_acquire = order[rows[m].first];
                let at_release = order[rows[m].last];
                barriers.push(Barrier {
                    resource: rows[m].name.clone(),
                    from: at_acquire,
                    to: at_acquire,
                    kind: BarrierKind::AliasAcquire,
                });
                barriers.push(Barrier {
                    resource: rows[m].name.clone(),
                    from: at_release,
                    to: at_release,
                    kind: BarrierKind::AliasRelease,
                });
            }
        }
        barriers.sort_by(|a, b| {
            a.from
                .cmp(&b.from)
                .then_with(|| a.to.cmp(&b.to))
                .then_with(|| a.resource.cmp(&b.resource))
                .then_with(|| barrier_rank(a.kind).cmp(&barrier_rank(b.kind)))
        });
        Ok(CompiledGraph {
            order,
            passes: self.passes,
            fns: self.fns,
            allocs,
            barriers,
            total_bytes,
            backing_bytes,
            reuse_pct,
        })
    }
}

/// Internal alias row: closed lifetime plus size.
#[derive(Clone, Debug)]
struct LifeRow {
    name: String,
    size_bytes: u64,
    first: usize,
    last: usize,
}

/// Deterministic cycle finder: ascending nodes and peers, first back
/// edge unwinds the loop stack. Empty when acyclic (unreachable here:
/// callers only invoke it after `order` refused).
fn find_cycle(n: usize, adj: &[Vec<NodeId>]) -> Vec<NodeId> {
    let mut color = vec![0u8; n];
    let mut path: Vec<NodeId> = Vec::new();
    for start in 0..n {
        if color[start] != 0 {
            continue;
        }
        let mut stack: Vec<(NodeId, usize)> = vec![(start, 0)];
        color[start] = 1;
        path.push(start);
        while let Some((node, next)) = stack.pop() {
            if next < adj[node].len() {
                stack.push((node, next + 1));
                let peer = adj[node][next];
                if color[peer] == 0 {
                    color[peer] = 1;
                    path.push(peer);
                    stack.push((peer, 0));
                } else if color[peer] == 1 {
                    let from = path.iter().position(|&x| x == peer).unwrap_or(0);
                    return path[from..].to_vec();
                }
            } else {
                color[node] = 2;
                if path.last() == Some(&node) {
                    path.pop();
                }
            }
        }
    }
    Vec::new()
}

/// Compiled frame: fixed order, alias plan, barriers and recorded fns.
/// `execute` runs the CPU fns in order and reports one receipt; the GPU
/// path stays a loud stub until the bind groups land (RL1/RL2/RP1).
pub struct CompiledGraph {
    order: Vec<PassId>,
    passes: Vec<PassDesc>,
    fns: Vec<Option<PassFn>>,
    allocs: Vec<TransientAlloc>,
    barriers: Vec<Barrier>,
    total_bytes: u64,
    backing_bytes: u64,
    reuse_pct: u32,
}

impl fmt::Debug for CompiledGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledGraph")
            .field("order", &self.order)
            .field("passes", &self.passes)
            .field("fn_count", &self.fns.len())
            .field("allocs", &self.allocs)
            .field("barriers", &self.barriers)
            .finish()
    }
}

impl CompiledGraph {
    /// Pass ids in execution order (from [`Graph::order`]).
    pub fn order(&self) -> &[PassId] {
        &self.order
    }
    /// Pass count.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }
    /// Pass declaration by id.
    pub fn pass(&self, id: PassId) -> Option<&PassDesc> {
        self.passes.get(id)
    }
    /// Transient placements with lifetimes and slots.
    pub fn transient_allocs(&self) -> &[TransientAlloc] {
        &self.allocs
    }
    /// Explicit barriers in (from, to, resource) order.
    pub fn barriers(&self) -> &[Barrier] {
        &self.barriers
    }
    /// Transient bytes before aliasing.
    pub fn transient_bytes_total(&self) -> u64 {
        self.total_bytes
    }
    /// Backing bytes after aliasing.
    pub fn transient_bytes_backing(&self) -> u64 {
        self.backing_bytes
    }
    /// Bytes in shared slots over total transient bytes, 0..=100.
    pub fn transient_reuse_pct(&self) -> u32 {
        self.reuse_pct
    }
    /// RP5 scale seam: true while the compiled order names exactly `want`
    /// in order (SMAA-first check for the TAA-off graph: the post seam
    /// passes the RP4 TAA-off names and refuses anything else). Compares
    /// by name without allocating; planning-time only, never per-tick.
    pub fn order_names_match(&self, want: &[&str]) -> bool {
        if self.order.len() != want.len() {
            return false;
        }
        for (pos, id) in self.order.iter().enumerate() {
            match self.passes.get(*id) {
                Some(desc) if desc.name == want[pos] => {}
                _ => return false,
            }
        }
        true
    }
    /// Run recorded CPU fns in compiled order; time each in whole ms.
    /// Needs `&mut` because recorded fns may mutate captured state.
    pub fn execute(&mut self, frame: &FrameInfo) -> FrameReceipt {
        let mut receipt = FrameReceipt {
            frame_index: frame.frame_index,
            width_px: frame.width_px,
            height_px: frame.height_px,
            draws: 0,
            triangles: 0,
            per_pass_cpu_ms: Vec::with_capacity(self.order.len()),
            total_cpu_ms: 0,
            transient_bytes_total: self.total_bytes,
            transient_bytes_backing: self.backing_bytes,
            transient_reuse_pct: self.reuse_pct,
        };
        for i in 0..self.order.len() {
            let id = self.order[i];
            let started = Instant::now();
            let mut ctx = PassCtx {
                pass: id,
                frame_index: frame.frame_index,
                extra_draws: 0,
                extra_triangles: 0,
            };
            if let Some(f) = self.fns[id].as_mut() {
                f(&mut ctx);
            }
            let desc = &self.passes[id];
            receipt.draws = receipt
                .draws
                .saturating_add(desc.draws)
                .saturating_add(ctx.extra_draws);
            receipt.triangles = receipt
                .triangles
                .saturating_add(desc.triangles)
                .saturating_add(ctx.extra_triangles);
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            receipt.total_cpu_ms = receipt.total_cpu_ms.saturating_add(ms);
            receipt.per_pass_cpu_ms.push(PassTiming {
                pass: desc.name.clone(),
                cpu_ms: ms,
            });
        }
        receipt
    }
    /// GPU submit (headless twin): typed refusal until a wgpu device plus
    /// bind groups exist. Never silent, never a pretend success. The window
    /// build gates this stub out and submits for real (GX1 below).
    #[cfg(not(feature = "window"))]
    pub fn submit_gpu(&self) -> Result<(), GpuUnsupported> {
        Err(GpuUnsupported(format!(
            "frame holds {} passes with no device: needs wgpu plus RL1/RL2/RP1 bind groups",
            self.order.len()
        )))
    }
}

// ----- GX1: window GPU submit over the same compiled graph -----
// Owner: render lane; consumers WIN1 window loop and ED1 viewport: both own
// one `GpuSubmitter` (pass pipeline, look bind layout, uniform buffer plus
// slot-target and bind-group caches) and call `CompiledGraph::submit_gpu`
// per frame with a validated `gtao::LookFrame`. Units: bytes, whole ms,
// integer percent. Budget: `GPU_SUBMIT_BUDGET_MS` covers encode + submit +
// readback at test sizes (arena-size timing belongs to PF2); transient bytes
// stay under `GPU_SUBMIT_TRANSIENT_BUDGET_BYTES`. Determinism: compiled order
// runs as compiled and explicit barriers become load ops; pixels are evidence
// only, never hashed. Errors: `GpuSubmitError` names the bad input, the
// device call or the validation scope; this path has no `Unsupported`
// variant because the caller hands over a live device.

/// Posted encode + submit + readback budget at test sizes in ms (debug
/// build on the reference GPU, evidence only, never hashed; arena-size
/// timing belongs to PF2).
#[cfg(feature = "window")]
pub const GPU_SUBMIT_BUDGET_MS: f64 = 16.0;
/// Largest transient backing the submit accepts in bytes. The arena fixture
/// backs about 27 MB (HDR 16.6 + depth 8.3 + decal 2.1 + smallest bloom mip).
#[cfg(feature = "window")]
pub const GPU_SUBMIT_TRANSIENT_BUDGET_BYTES: u64 = 32 * 1024 * 1024;
/// Fewest compiled passes the acceptance submit runs: the forward pass plus
/// at least 3 graph passes (the arena fixture runs 7).
#[cfg(feature = "window")]
pub const GPU_SUBMIT_MIN_PASSES: usize = 4;
/// Byte size of the packed `gtao::LookFrame` uniform: frame index, viewport,
/// exposure, sun direction plus pad, 16-byte aligned for the shader below.
#[cfg(feature = "window")]
pub const LOOK_UNIFORM_BYTES: u64 = 32;
/// Most slot targets the submitter ever sizes (the `gtao::LookFrame` bound
/// already enforces the per-dimension limit; this caps the slot fan-out).
#[cfg(feature = "window")]
pub const GPU_SUBMIT_MAX_SLOTS: usize = 64;

/// Fullscreen-triangle pass shader over the shared look uniform: no vertex
/// buffers, no depth, one `Rgba8Unorm` target per pass. Engine-owned code.
/// The fragment taps `look.exposure` so the uniform stays live in every pass
/// and the covered-pixel count in the receipt proves real writes.
#[cfg(feature = "window")]
pub const GX_PASS_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = p[vi] * 0.5 + 0.5;
    return out;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let e = clamp(look.exposure / 10.0, 0.5, 2.0);
    return vec4<f32>(0.4 * e, 0.5, 0.6, 1.0);
}
"#;

/// Window submit failures. There is no `Unsupported` variant: the caller
/// hands over a wgpu device, so every failure names the bad input, the
/// device call or the validation scope outcome.
#[cfg(feature = "window")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuSubmitError {
    /// Caller input is unusable (invalid frame, over-budget transients,
    /// uncompilable pass wiring); reason carried.
    Invalid(String),
    /// A device or queue call failed (readback map or poll); message carried.
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud with
    /// the scope debug text.
    Validation(String),
}

#[cfg(feature = "window")]
impl fmt::Display for GpuSubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => write!(f, "gx1 submit input: {detail}"),
            Self::Device(detail) => write!(f, "gx1 submit device: {detail}"),
            Self::Validation(detail) => write!(f, "gx1 submit validation: {detail}"),
        }
    }
}

#[cfg(feature = "window")]
impl Error for GpuSubmitError {}

/// One window submit: pass and draw totals from the compiled declarations,
/// slot counts from the RA2 alias plan, the validation outcome plus the
/// pixels the last target actually covered (proof the passes wrote pixels,
/// not just validated). Units: counts, bytes, ms, integer percent.
#[cfg(feature = "window")]
#[derive(Clone, Debug)]
pub struct GpuFrameReceipt {
    /// Frame index from the submitted `gtao::LookFrame`.
    pub frame_index: u64,
    /// Target width in pixels from the submitted frame.
    pub width_px: u32,
    /// Target height in pixels from the submitted frame.
    pub height_px: u32,
    /// Compiled passes recorded as wgpu passes.
    pub passes_submitted: usize,
    /// Declared draw calls summed over the compiled passes.
    pub draws: u64,
    /// Declared triangles summed over the compiled passes.
    pub triangles: u64,
    /// Backing slot targets sized (one texture per alias slot).
    pub transient_slots: usize,
    /// Transient bytes before aliasing, from the alias plan.
    pub transient_bytes_total: u64,
    /// Backing bytes after aliasing, from the alias plan.
    pub transient_bytes_backing: u64,
    /// Bytes in shared slots over total transient bytes, 0..=100.
    pub transient_reuse_pct: u32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Pixels in the last target differing from the clear color.
    pub covered_pixels: u64,
    /// Wall time for encode + submit + readback in ms (evidence only).
    pub submit_ms: f64,
    /// Per-pass GPU ms in compiled order; `None` while the device lacks
    /// `TIMESTAMP_QUERY` or a pair fails to resolve. Evidence only,
    /// never hashed.
    pub per_pass_gpu_ms: Vec<Option<f64>>,
}

#[cfg(feature = "window")]
impl GpuFrameReceipt {
    /// Posted budget this submit is charged against.
    pub fn budget_ms(&self) -> f64 {
        GPU_SUBMIT_BUDGET_MS
    }
    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= GPU_SUBMIT_BUDGET_MS
    }
    /// True while the alias plan stayed inside the transient budget.
    pub fn within_transient_budget(&self) -> bool {
        self.transient_bytes_backing <= GPU_SUBMIT_TRANSIENT_BUDGET_BYTES
    }
    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Pack a `gtao::LookFrame` into the 32-byte uniform image the pass shader
/// reads. Reuses the RL3 bind shape (no mirror): field order matches
/// `GX_PASS_WGSL` (`frame_idx`, viewport, exposure, sun dir, pad). Pure
/// bytes, no allocation; validation stays on the caller.
#[cfg(feature = "window")]
pub fn look_frame_uniform_bytes(frame: &crate::gtao::LookFrame) -> [u8; 32] {
    let mut out = [0u8; 32];
    let index = u32::try_from(frame.frame_index & 0xffff_ffff).unwrap_or(u32::MAX);
    out[0..4].copy_from_slice(&index.to_le_bytes());
    out[4..8].copy_from_slice(&frame.viewport_px[0].to_le_bytes());
    out[8..12].copy_from_slice(&frame.viewport_px[1].to_le_bytes());
    out[12..16].copy_from_slice(&frame.exposure_ev100.to_le_bytes());
    out[16..20].copy_from_slice(&frame.sun_dir[0].to_le_bytes());
    out[20..24].copy_from_slice(&frame.sun_dir[1].to_le_bytes());
    out[24..28].copy_from_slice(&frame.sun_dir[2].to_le_bytes());
    out
}

// Per-pass GPU timestamps ride the window submit below: one timestamp pair
// per compiled pass, resolved into per-pass ms. The pair helpers live with
// the timestamp consumer in `executor` (one owner); this file only writes
// the `timestamp_writes` arg and posts `per_pass_gpu_ms`.

/// Slot target plus load op for one compiled-order position: the first
/// transient live here picks the slot (whole-slot aliasing); a lifetime
/// start clears, any later use loads. Position zero always clears so a
/// leading pass with no transients never loads an uninitialized target.
/// Pure scan over the alias plan, no allocation.
#[cfg(feature = "window")]
fn slot_target_for_position(
    allocs: &[TransientAlloc],
    pos: usize,
    slot_count: usize,
) -> (usize, bool) {
    debug_assert!(slot_count > 0, "submit sizes at least one slot");
    for a in allocs {
        if a.first <= pos && pos <= a.last {
            debug_assert!(a.slot < slot_count, "alias slots stay dense 0..slot_count");
            return (a.slot, a.first == pos || pos == 0);
        }
    }
    (0, pos == 0)
}

/// Reusable window submit state: the pass pipeline plus the look bind layout
/// over one uniform buffer, with slot-target and per-pass bind-group caches.
/// Built once (WIN1 and ED1 each own one); the submit below reuses both
/// caches on matching shape, so steady-state frames create no pipelines,
/// bind groups or targets (the reuse test proves the counts stay flat).
/// Per-submit encoder and readback buffers are driver resources, same as the
/// RA1 twin, and sit outside the reuse guarantee.
#[cfg(feature = "window")]
pub struct GpuSubmitter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    binds: Vec<wgpu::BindGroup>,
    slots: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    cached_size: Option<(u32, u32)>,
    cached_slots: usize,
}

#[cfg(feature = "window")]
impl GpuSubmitter {
    /// Compile the pass pipeline and the look bind layout; allocate the
    /// 32-byte uniform buffer. No frame-size state yet: slot targets size on
    /// the first submit and resize only when the frame size changes.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx1-pass"),
            source: wgpu::ShaderSource::Wgsl(GX_PASS_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gx1-look-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gx1-pass-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gx1-pass-pipeline"),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx1-look-uniform"),
            size: LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            uniform_buf,
            binds: Vec::new(),
            slots: Vec::new(),
            views: Vec::new(),
            cached_size: None,
            cached_slots: 0,
        }
    }
    /// Resident slot-target count (probe for the reuse test).
    pub fn slot_texture_count(&self) -> usize {
        self.slots.len()
    }
    /// Resident per-pass bind-group count (probe for the reuse test).
    pub fn bind_group_count(&self) -> usize {
        self.binds.len()
    }
}

/// Window submit over the compiled graph (GX1): one wgpu pass per compiled
/// pass in compiled order, each bound to the shared look bind group, writing
/// RA2-aliased slot targets; a single submit plus a last-target readback.
/// `submitter` caches stay flat on matching shape (steady-state proof in the
/// reuse test); the encoder and readback buffer are per-submit driver work.
#[cfg(feature = "window")]
impl CompiledGraph {
    /// Record the compiled passes on `device` and submit once. Consumes no
    /// `submitter` capacity on matching shape; fails loudly (never silent,
    /// never `Unsupported`) on invalid frames, over-budget transients or a
    /// validation scope hit.
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
        submitter: &mut GpuSubmitter,
    ) -> Result<GpuFrameReceipt, GpuSubmitError> {
        frame
            .validate()
            .map_err(|e| GpuSubmitError::Invalid(format!("look frame: {e}")))?;
        let (width_px, height_px) = (frame.viewport_px[0], frame.viewport_px[1]);
        if self.backing_bytes > GPU_SUBMIT_TRANSIENT_BUDGET_BYTES {
            return Err(GpuSubmitError::Invalid(format!(
                "transient backing {} over budget {GPU_SUBMIT_TRANSIENT_BUDGET_BYTES}",
                self.backing_bytes
            )));
        }
        let mut draws = 0u64;
        let mut triangles = 0u64;
        for pass in &self.passes {
            draws = draws.saturating_add(pass.draws);
            triangles = triangles.saturating_add(pass.triangles);
        }
        if self.order.is_empty() {
            return Ok(GpuFrameReceipt {
                frame_index: frame.frame_index,
                width_px,
                height_px,
                passes_submitted: 0,
                draws,
                triangles,
                transient_slots: 0,
                transient_bytes_total: self.total_bytes,
                transient_bytes_backing: self.backing_bytes,
                transient_reuse_pct: self.reuse_pct,
                validation_errors: 0,
                covered_pixels: 0,
                submit_ms: 0.0,
                per_pass_gpu_ms: crate::executor::none_gpu_timings(0),
            });
        }
        let mut slot_count = 0usize;
        for a in &self.allocs {
            slot_count = slot_count.max(a.slot + 1);
        }
        if slot_count == 0 {
            slot_count = 1;
        }
        if slot_count > GPU_SUBMIT_MAX_SLOTS {
            return Err(GpuSubmitError::Invalid(format!(
                "alias slots {slot_count} over the {GPU_SUBMIT_MAX_SLOTS} cap"
            )));
        }
        if submitter.cached_size != Some((width_px, height_px))
            || submitter.cached_slots != slot_count
        {
            submitter.slots.clear();
            submitter.views.clear();
            for _ in 0..slot_count {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("gx1-slot-target"),
                    size: wgpu::Extent3d {
                        width: width_px,
                        height: height_px,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                submitter.slots.push(texture);
                submitter.views.push(view);
            }
            submitter.cached_size = Some((width_px, height_px));
            submitter.cached_slots = slot_count;
        }
        if submitter.binds.len() != self.order.len() {
            submitter.binds.clear();
            for _ in &self.order {
                submitter
                    .binds
                    .push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("gx1-look-bind"),
                        layout: &submitter.layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: submitter.uniform_buf.as_entire_binding(),
                        }],
                    }));
            }
        }
        let pass_count = self.order.len();
        let ts = crate::executor::timestamp_setup(device, queue, pass_count);
        queue.write_buffer(&submitter.uniform_buf, 0, &look_frame_uniform_bytes(frame));
        let started = Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gx1-frame"),
        });
        let mut last_slot = 0usize;
        for (pos, _) in self.order.iter().enumerate() {
            let (slot, clear) = slot_target_for_position(&self.allocs, pos, slot_count);
            last_slot = slot;
            let view = &submitter.views[slot];
            let bind = &submitter.binds[pos];
            let ts_writes = match ts.as_ref() {
                Some(ts) => {
                    let (begin, end) = crate::executor::query_pair(pos);
                    Some(wgpu::RenderPassTimestampWrites {
                        query_set: &ts.query_set,
                        beginning_of_pass_write_index: Some(begin),
                        end_of_pass_write_index: Some(end),
                    })
                }
                None => None,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gx1-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            })
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: ts_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&submitter.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(ts) = ts.as_ref() {
            let size = u64::from(ts.query_count).saturating_mul(8);
            encoder.resolve_query_set(&ts.query_set, 0..ts.query_count, &ts.resolve_buf, 0);
            encoder.copy_buffer_to_buffer(&ts.resolve_buf, 0, &ts.staging_buf, 0, size);
        }
        let bytes_per_row = width_px * 4;
        let padded = bytes_per_row.div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx1-readback"),
            size: u64::from(padded) * u64::from(height_px),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &submitter.slots[last_slot],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height_px),
                },
            },
            wgpu::Extent3d {
                width: width_px,
                height: height_px,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let ts_staging = ts.as_ref().map(|ts| &ts.staging_buf);
        let ts_slice = ts_staging.as_ref().map(|buf| buf.slice(..));
        let (ts_tx, ts_rx) = std::sync::mpsc::channel();
        if let Some(ts_slice) = ts_slice.as_ref() {
            ts_slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = ts_tx.send(r);
            });
        }
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuSubmitError::Device(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| GpuSubmitError::Device(format!("map channel: {e}")))?
            .map_err(|e| GpuSubmitError::Device(format!("map: {e}")))?;
        let mut covered = 0u64;
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| GpuSubmitError::Device(format!("mapped range: {e}")))?;
            let height = usize::try_from(height_px).unwrap_or(usize::MAX);
            let pad = usize::try_from(padded).unwrap_or(usize::MAX);
            let row_bytes = usize::try_from(bytes_per_row).unwrap_or(usize::MAX);
            for row in 0..height {
                let off = row * pad;
                let bytes = &mapped[off..off + row_bytes];
                for px in bytes.chunks_exact(4) {
                    if px[0] > 10 {
                        covered += 1;
                    }
                }
            }
        }
        readback.unmap();
        let period = ts.as_ref().map(|ts| ts.period).unwrap_or(0.0);
        let per_pass_gpu_ms =
            crate::executor::read_pass_ms(ts_staging, ts_slice.as_ref(), ts_rx, pass_count, period);
        let scope_err = pollster::block_on(scope.pop());
        if let Some(err) = scope_err {
            return Err(GpuSubmitError::Validation(format!("{err:?}")));
        }
        Ok(GpuFrameReceipt {
            frame_index: frame.frame_index,
            width_px,
            height_px,
            passes_submitted: self.order.len(),
            draws,
            triangles,
            transient_slots: slot_count,
            transient_bytes_total: self.total_bytes,
            transient_bytes_backing: self.backing_bytes,
            transient_reuse_pct: self.reuse_pct,
            validation_errors: 0,
            covered_pixels: covered,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
            per_pass_gpu_ms,
        })
    }
}

// ----- arena-shape fixture (sizes: docs/archive/2026-10-08/plan/render-frame.md section 3) -----

/// Arena target width in pixels.
pub const ARENA_WIDTH_PX: u32 = 1920;
/// Arena target height in pixels.
pub const ARENA_HEIGHT_PX: u32 = 1080;
/// HDR target: 1920 * 1080 * 8 RGBA16F bytes.
pub const ARENA_HDR_BYTES: u64 = 16_588_800;
/// Depth target: 1920 * 1080 * 4 bytes.
pub const ARENA_DEPTH_BYTES: u64 = 8_294_400;
/// TAA resolve scratch: full LDR target size in bytes.
pub const ARENA_RESOLVE_BYTES: u64 = 8_294_400;
/// Shadow cascade scratch: 4 * 1024 * 1024 * 4 bytes.
pub const ARENA_CASCADE_BYTES: u64 = 4_194_304;
/// Forward+ cluster grid: 8160 tiles * 24 lights * 4 bytes.
pub const ARENA_CLUSTER_BYTES: u64 = 783_360;
/// Decal atlas tile scratch in bytes.
pub const ARENA_DECAL_BYTES: u64 = 2_097_152;
/// Bloom chain mips (half, quarter, eighth, sixteenth RGBA16F) in bytes.
pub const ARENA_BLOOM_MIP_BYTES: [u64; 4] = [4_147_200, 1_036_800, 259_200, 65_280];

/// Arena-shape frame: shadow, forward, decals, TAA resolve, bloom, HUD
/// and present with section-3 sizes. History stays a persistent
/// output (never aliases); everything else aliases by strict
/// non-overlapping lifetime. First consumer: the reuse-gate test below;
/// next: RA8 diagnostics and the RA1 presenter running one graph.
pub fn arena_frame_graph() -> FrameGraph {
    let mut graph = FrameGraph::new();
    graph.add_pass(
        PassDesc::new("shadow")
            .output("cascade")
            .transient("cascade_scratch", ARENA_CASCADE_BYTES)
            .work(64, 500_000),
    );
    graph.add_pass(
        PassDesc::new("forward")
            .input("cascade")
            .input("scene")
            .output("hdr")
            .output("depth")
            .transient("hdr", ARENA_HDR_BYTES)
            .transient("depth", ARENA_DEPTH_BYTES)
            .transient("cluster_grid", ARENA_CLUSTER_BYTES)
            .work(256, 1_800_000),
    );
    let decals = graph.add_pass(
        PassDesc::new("decals")
            .input("depth")
            .input("hdr")
            .output("hdr")
            .transient("decal_tile", ARENA_DECAL_BYTES)
            .work(48, 96_000),
    );
    let resolve = graph.add_pass(
        PassDesc::new("resolve")
            .input("hdr")
            .input("history")
            .output("ldr")
            .output("history")
            .transient("resolve_scratch", ARENA_RESOLVE_BYTES)
            .work(2, 2),
    );
    let bloom = graph.add_pass(
        PassDesc::new("bloom")
            .input("ldr")
            .output("ldr")
            .transient("bloom_mip0", ARENA_BLOOM_MIP_BYTES[0])
            .transient("bloom_mip1", ARENA_BLOOM_MIP_BYTES[1])
            .transient("bloom_mip2", ARENA_BLOOM_MIP_BYTES[2])
            .transient("bloom_mip3", ARENA_BLOOM_MIP_BYTES[3])
            .work(8, 8),
    );
    graph.add_pass(
        PassDesc::new("hud")
            .input("ldr")
            .output("swapchain")
            .work(32, 4_000),
    );
    graph.add_pass(PassDesc::new("present").input("swapchain").work(1, 1));
    // Read-write chains need explicit sequence: inference alone cannot
    // tell decals-then-resolve (via hdr) from the reverse.
    graph
        .add_edge(decals, resolve)
        .expect("arena fixture pass ids are dense");
    graph
        .add_edge(resolve, bloom)
        .expect("arena fixture pass ids are dense");
    graph
}

#[cfg(all(test, feature = "window"))]
static GX1_DEVICE: std::sync::OnceLock<Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)>> =
    std::sync::OnceLock::new();

/// Window device shared by the GX1 submit tests: high-performance first
/// (the reference GPU on the operator machine), then any fallback adapter
/// (the llvmpipe twin). One device for the whole suite so parallel tests
/// never exhaust adapters; `None` is skip-not-green, never a fake pass.
/// Test-only owner for the graph and executor submit tests.
#[cfg(all(test, feature = "window"))]
pub(crate) fn gx1_test_device() -> Option<(
    &'static wgpu::Device,
    &'static wgpu::Queue,
    &'static wgpu::AdapterInfo,
)> {
    let slot = GX1_DEVICE.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .or_else(|_| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: true,
                apply_limit_buckets: false,
            }))
        })
        .ok()?;
        let info = adapter.get_info();
        eprintln!(
            "GX1 device under test: {} backend={:?} driver=\"{} {}\"",
            info.name, info.backend, info.driver, info.driver_info
        );
        // Timestamp-capable device first so the reference GPU posts `Some`
        // per-pass ms; adapters without `TIMESTAMP_QUERY` fall back to a
        // plain device and post `None` (never a startup failure).
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("engine2040-gx1"),
            required_features: wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        }))
        .or_else(|_| {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("engine2040-gx1"),
                ..Default::default()
            }))
        })
        .ok()?;
        eprintln!(
            "window timestamps supported: {}",
            device.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        );
        Some((device, queue, info))
    });
    slot.as_ref()
        .map(|(device, queue, info)| (device, queue, info))
}

/// Shared 64x64 fixture frame for the GX1 submit tests (valid exposure and
/// sun direction; the submit validates it again through `LookFrame`).
#[cfg(all(test, feature = "window"))]
pub(crate) fn gx1_test_frame(index: u64) -> crate::gtao::LookFrame {
    crate::gtao::LookFrame {
        frame_index: index,
        viewport_px: [64, 64],
        exposure_ev100: 10.0,
        sun_dir: [0.3, 0.8, 0.5],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_order() {
        let mut g = Graph::new();
        let (a, b, c) = (
            g.add_node("depth"),
            g.add_node("forward"),
            g.add_node("post"),
        );
        g.add_edge(a, b);
        g.add_edge(b, c);
        assert_eq!(g.order(), Some(vec![a, b, c]));
    }

    #[test]
    fn fork_deterministic_smallest_first() {
        let mut g = Graph::new();
        let (root, left, right) = (g.add_node("root"), g.add_node("a"), g.add_node("b"));
        g.add_edge(root, right);
        g.add_edge(root, left);
        assert_eq!(g.order(), Some(vec![root, left, right]));
    }

    #[test]
    fn cycle_returns_none() {
        let mut g = Graph::new();
        let (a, b) = (g.add_node("a"), g.add_node("b"));
        g.add_edge(a, b);
        g.add_edge(b, a);
        assert_eq!(g.order(), None);
    }

    #[test]
    fn hash_stable() {
        let mut g = Graph::new();
        let a = g.add_node("depth");
        let b = g.add_node("forward");
        g.add_edge(a, b);
        assert_eq!(g.graph_hash(), g.graph_hash());
    }

    #[test]
    fn inferred_cycle_names_both_passes() {
        let mut g = FrameGraph::new();
        g.add_pass(PassDesc::new("a").input("y").output("x"));
        g.add_pass(PassDesc::new("b").input("x").output("y"));
        match g.compile() {
            Err(GraphError::Cycle { nodes }) => {
                assert!(nodes.contains(&"a".to_string()), "cycle names a: {nodes:?}");
                assert!(nodes.contains(&"b".to_string()), "cycle names b: {nodes:?}");
            }
            other => panic!("expected a loud cycle, got {other:?}"),
        }
    }

    #[test]
    fn explicit_edge_cycle_is_loud() {
        let mut g = FrameGraph::new();
        let a = g.add_pass(PassDesc::new("a"));
        let b = g.add_pass(PassDesc::new("b"));
        g.add_edge(a, b).expect("dense ids");
        g.add_edge(b, a).expect("dense ids");
        match g.compile() {
            Err(GraphError::Cycle { nodes }) => assert_eq!(nodes.len(), 2, "both nodes: {nodes:?}"),
            other => panic!("expected a loud cycle, got {other:?}"),
        }
    }

    #[test]
    fn compiled_order_stable_under_edge_permutation() {
        let build = |flip: bool| {
            let mut g = FrameGraph::new();
            let root = g.add_pass(PassDesc::new("root"));
            let left = g.add_pass(PassDesc::new("left"));
            let right = g.add_pass(PassDesc::new("right"));
            if flip {
                g.add_edge(root, right).expect("dense ids");
                g.add_edge(root, left).expect("dense ids");
            } else {
                g.add_edge(root, left).expect("dense ids");
                g.add_edge(root, right).expect("dense ids");
            }
            g.compile().expect("acyclic").order().to_vec()
        };
        assert_eq!(build(false), build(true));
        assert_eq!(build(false), vec![0, 1, 2]);
    }

    #[test]
    fn forward_ibl_bind_names_match_descriptor() {
        // Graph side of the forward-IBL lock: the bake -> forward wiring
        // names exactly the resources the compiled forward bind image
        // carries, so a renamed resource fails here instead of binding
        // silently wrong on the device.
        assert_eq!(crate::ibl::FORWARD_IBL_INPUTS.len(), 3);
        assert_eq!(
            crate::ibl::FORWARD_IBL_BIND_BYTES,
            8 + crate::ibl::FORWARD_IBL_SH_UNIFORM_BYTES,
            "bind image is the hash head plus the SH tail"
        );
        let compiled = crate::ibl::compile_ibl_for_forward().expect("ibl wiring compiles");
        let forward = compiled
            .pass(compiled.order()[1])
            .expect("forward is second");
        assert_eq!(forward.name.as_str(), crate::ibl::IBL_FORWARD_PASS);
        for resource in crate::ibl::FORWARD_IBL_INPUTS {
            assert!(
                forward.inputs.iter().any(|i| i == resource),
                "forward binds {resource}: {:?}",
                forward.inputs
            );
        }
        // The image forward would upload carries the pinned chunk hash ahead
        // of the SH bytes (the byte contract, headless).
        let mut image = [0u8; crate::ibl::FORWARD_IBL_BIND_BYTES];
        crate::ibl::forward_ibl_bind_image(&crate::ibl::IrradianceSh::zero(), &mut image);
        let mut head = [0u8; 8];
        head.copy_from_slice(&image[0..8]);
        assert_eq!(
            head,
            crate::ibl::FORWARD_IBL_CHUNK_HASH.to_le_bytes(),
            "bind head is the pinned chunk hash"
        );
        assert!(
            image[8..].iter().all(|b| *b == 0),
            "zero SH packs a zero tail"
        );
    }

    #[test]
    fn ibl_wiring_stable_under_pass_insertion_flip() {
        // Proof-flip refresh for the IBL edge: pass insertion order never
        // moves the compiled bake -> forward handover (names, barriers, or
        // receipt draws), so the wiring proof cannot go stale unnoticed.
        let build = |flip: bool| {
            let mut graph = FrameGraph::new();
            let bake_desc = PassDesc::new(crate::ibl::IBL_BAKE_PASS)
                .output(crate::ibl::FORWARD_IBL_INPUTS[0])
                .output(crate::ibl::FORWARD_IBL_INPUTS[1])
                .output(crate::ibl::FORWARD_IBL_INPUTS[2])
                .work(1, 4);
            let forward_desc = PassDesc::new(crate::ibl::IBL_FORWARD_PASS)
                .input(crate::ibl::FORWARD_IBL_INPUTS[0])
                .input(crate::ibl::FORWARD_IBL_INPUTS[1])
                .input(crate::ibl::FORWARD_IBL_INPUTS[2])
                .work(2, 6);
            let (bake, forward) = if flip {
                let forward = graph.add_pass(forward_desc);
                let bake = graph.add_pass(bake_desc);
                (bake, forward)
            } else {
                let bake = graph.add_pass(bake_desc);
                let forward = graph.add_pass(forward_desc);
                (bake, forward)
            };
            graph.add_edge(bake, forward).expect("dense ids");
            graph.compile().expect("acyclic")
        };
        let named_order = |compiled: &CompiledGraph| {
            compiled
                .order()
                .iter()
                .map(|id| compiled.pass(*id).expect("dense ids").name.clone())
                .collect::<Vec<_>>()
        };
        let named_barriers = |compiled: &CompiledGraph| {
            let mut rows = compiled
                .barriers()
                .iter()
                .map(|b| {
                    (
                        b.resource.clone(),
                        b.kind,
                        compiled.pass(b.from).expect("dense ids").name.clone(),
                        compiled.pass(b.to).expect("dense ids").name.clone(),
                    )
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.2.cmp(&b.2))
                    .then_with(|| a.3.cmp(&b.3))
            });
            rows
        };
        let (mut straight, mut flipped) = (build(false), build(true));
        assert_eq!(named_order(&straight), named_order(&flipped));
        assert_eq!(
            named_order(&straight),
            vec![
                crate::ibl::IBL_BAKE_PASS.to_string(),
                crate::ibl::IBL_FORWARD_PASS.to_string()
            ]
        );
        assert_eq!(named_barriers(&straight), named_barriers(&flipped));
        assert_eq!(
            named_barriers(&straight).len(),
            3,
            "one handover per resource"
        );
        for compiled in [&mut straight, &mut flipped] {
            let receipt = compiled.execute(&FrameInfo {
                frame_index: 7,
                width_px: 16,
                height_px: 16,
            });
            assert_eq!(receipt.draws, 3, "both passes record draws");
        }
    }

    #[test]
    fn unknown_pass_ids_are_rejected() {
        let mut g = FrameGraph::new();
        g.add_pass(PassDesc::new("only"));
        assert_eq!(g.add_edge(0, 7), Err(GraphError::UnknownPass { id: 7 }));
        assert_eq!(g.add_edge(7, 0), Err(GraphError::UnknownPass { id: 7 }));
        assert_eq!(
            g.set_pass_fn(7, |_| {}),
            Err(GraphError::UnknownPass { id: 7 })
        );
    }

    #[test]
    fn explicit_span_missing_a_use_is_rejected() {
        let mut g = FrameGraph::new();
        g.add_pass(PassDesc::new("writer").output("hdr"));
        // Reader at pass 1, but the span ends at pass 0: use after free.
        g.add_pass(
            PassDesc::new("owner")
                .transient_span("hdr", ARENA_HDR_BYTES, 0, 0)
                .input("hdr"),
        );
        match g.compile() {
            Err(GraphError::BadLifetime { name, .. }) => assert_eq!(name, "hdr"),
            other => panic!("expected a loud lifetime refusal, got {other:?}"),
        }
    }

    #[test]
    fn arena_fixture_clears_the_reuse_gate() {
        let compiled = arena_frame_graph()
            .compile()
            .expect("arena frame is acyclic");
        assert!(
            compiled.transient_reuse_pct() >= 80,
            "reuse {}% below the 80% gate",
            compiled.transient_reuse_pct()
        );
        let expect_total = ARENA_HDR_BYTES
            + ARENA_DEPTH_BYTES
            + ARENA_RESOLVE_BYTES
            + ARENA_CASCADE_BYTES
            + ARENA_CLUSTER_BYTES
            + ARENA_DECAL_BYTES
            + ARENA_BLOOM_MIP_BYTES.iter().sum::<u64>();
        assert_eq!(compiled.transient_bytes_total(), expect_total);
        // Peaks: HDR slot, depth/resolve slot, decal/cluster/mip2 slot,
        // plus the smallest bloom mip alone (same-pass mips overlap).
        assert_eq!(
            compiled.transient_bytes_backing(),
            ARENA_HDR_BYTES + ARENA_DEPTH_BYTES + ARENA_DECAL_BYTES + ARENA_BLOOM_MIP_BYTES[3]
        );
        // HDR aliases with bloom scratch; history stays persistent outside the pool.
        let slot_of = |name: &str| {
            compiled
                .transient_allocs()
                .iter()
                .find(|a| a.name == name)
                .map(|a| a.slot)
        };
        assert_eq!(slot_of("hdr"), slot_of("bloom_mip0"));
        assert_ne!(slot_of("hdr"), slot_of("depth"));
        assert!(slot_of("history").is_none(), "history never aliases");
        // Explicit barrier: the forward HDR write reaches resolve.
        assert!(
            compiled.barriers().iter().any(|b| {
                b.resource == "hdr" && b.kind == BarrierKind::Transition && b.from == 1 && b.to == 3
            }),
            "missing forward -> resolve hdr barrier"
        );
    }

    #[test]
    fn execute_runs_compiled_order_and_sums_work() {
        let mut g = arena_frame_graph();
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        for id in 0..g.pass_count() {
            let seen_each = std::sync::Arc::clone(&seen);
            let name = g.pass(id).expect("dense ids").name.clone();
            g.set_pass_fn(id, move |ctx: &mut PassCtx| {
                seen_each.lock().expect("test lock").push(name.clone());
                ctx.drew(1, 10);
            })
            .expect("dense ids");
        }
        let mut compiled = g.compile().expect("acyclic");
        let order_names: Vec<String> = compiled
            .order()
            .iter()
            .map(|&id| compiled.pass(id).expect("dense").name.clone())
            .collect();
        let frame = FrameInfo {
            frame_index: 7,
            width_px: ARENA_WIDTH_PX,
            height_px: ARENA_HEIGHT_PX,
        };
        let receipt = compiled.execute(&frame);
        assert_eq!(
            *seen.lock().expect("test lock"),
            order_names,
            "fns run in compiled order"
        );
        let mut expect_draws = 0u64;
        let mut expect_tris = 0u64;
        for id in 0..compiled.pass_count() {
            let p = compiled.pass(id).expect("dense ids");
            expect_draws += p.draws;
            expect_tris += p.triangles;
        }
        let extra = compiled.pass_count() as u64;
        assert_eq!(receipt.draws, expect_draws + extra);
        assert_eq!(receipt.triangles, expect_tris + extra * 10);
        assert_eq!(receipt.per_pass_cpu_ms.len(), compiled.pass_count());
        assert_eq!(
            receipt.total_cpu_ms,
            receipt
                .per_pass_cpu_ms
                .iter()
                .map(|t| t.cpu_ms)
                .sum::<u64>()
        );
        assert_eq!(
            (receipt.frame_index, receipt.width_px, receipt.height_px),
            (7, 1920, 1080)
        );
        assert_eq!(
            receipt.transient_bytes_total,
            compiled.transient_bytes_total()
        );
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn gpu_submit_refuses_with_typed_unsupported() {
        let compiled = arena_frame_graph().compile().expect("acyclic");
        let err = compiled
            .submit_gpu()
            .expect_err("no GPU backend in the CPU executor");
        assert!(!err.0.is_empty(), "the stub is never silent");
        assert!(err.0.contains("wgpu"), "names the missing path: {}", err.0);
    }

    #[test]
    fn rp5_order_names_match_checks_smaa_first() {
        // RP5 graph seam: the post seam passes the TAA-off names and
        // refuses anything else; a wrong order or length never matches.
        let mut g = FrameGraph::new();
        let smaa = g.add_pass(PassDesc::new("rp4-smaa"));
        let taau = g.add_pass(PassDesc::new("rp1-taau"));
        g.add_edge(smaa, taau).expect("dense ids");
        let compiled = g.compile().expect("acyclic");
        assert!(compiled.order_names_match(&["rp4-smaa", "rp1-taau"]));
        assert!(!compiled.order_names_match(&["rp1-taau", "rp4-smaa"]));
        assert!(!compiled.order_names_match(&["rp4-smaa"]));
        assert!(!compiled.order_names_match(&[]));
        assert!(!compiled.order_names_match(&["rp4-smaa", "rp1-taau", "rp1-bloom"]));
    }

    #[test]
    fn empty_graph_compiles_to_a_zero_receipt() {
        let mut compiled = FrameGraph::new().compile().expect("empty graph is acyclic");
        let receipt = compiled.execute(&FrameInfo {
            frame_index: 0,
            width_px: 64,
            height_px: 64,
        });
        assert_eq!(
            (receipt.draws, receipt.triangles, receipt.total_cpu_ms),
            (0, 0, 0)
        );
        assert!(receipt.per_pass_cpu_ms.is_empty());
        assert_eq!(
            (
                receipt.transient_bytes_total,
                receipt.transient_bytes_backing
            ),
            (0, 0)
        );
        assert_eq!(receipt.transient_reuse_pct, 100);
    }

    // ----- GX1 window tests (behind `window`; skip loudly without a device) -----

    #[test]
    #[cfg(feature = "window")]
    fn gx1_window_submit_runs_forward_plus_three_clean() {
        let Some((device, queue, info)) = gx1_test_device() else {
            eprintln!("SKIPPED gx1 window submit: no wgpu adapter on this machine");
            return;
        };
        let compiled = arena_frame_graph()
            .compile()
            .expect("arena frame is acyclic");
        assert!(
            compiled.order().len() >= GPU_SUBMIT_MIN_PASSES,
            "acceptance needs forward + 3 passes, got {}",
            compiled.order().len()
        );
        assert_eq!(
            compiled.pass(1).expect("dense arena ids").name,
            "forward",
            "pass 1 is the forward pass"
        );
        let mut submitter = GpuSubmitter::new(device);
        let receipt = compiled
            .submit_gpu(device, queue, &gx1_test_frame(11), &mut submitter)
            .expect("window submit runs clean");
        assert_eq!(
            receipt.passes_submitted,
            compiled.order().len(),
            "every compiled pass becomes a wgpu pass"
        );
        assert!(
            receipt.is_clean(),
            "0 validation errors, got {}",
            receipt.validation_errors
        );
        assert!(
            receipt.covered_pixels > 0,
            "passes must write pixels, got 0 covered"
        );
        assert!(
            receipt.transient_reuse_pct >= 80,
            "reuse {}% below the 80% gate",
            receipt.transient_reuse_pct
        );
        assert!(
            receipt.within_transient_budget(),
            "backing {} over budget {GPU_SUBMIT_TRANSIENT_BUDGET_BYTES}",
            receipt.transient_bytes_backing
        );
        assert_eq!(
            submitter.slot_texture_count(),
            receipt.transient_slots,
            "one target per alias slot"
        );
        assert_eq!(
            submitter.bind_group_count(),
            receipt.passes_submitted,
            "one look bind group per pass"
        );
        eprintln!(
            "GX1 receipt on {}: passes={} draws={} slots={} reuse={}% covered={} submit_ms={:.2}",
            info.name,
            receipt.passes_submitted,
            receipt.draws,
            receipt.transient_slots,
            receipt.transient_reuse_pct,
            receipt.covered_pixels,
            receipt.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_twin_runs_same_compiled_graph() {
        let Some((device, queue, _)) = gx1_test_device() else {
            eprintln!("SKIPPED gx1 twin: no wgpu adapter on this machine");
            return;
        };
        // CPU twin over the same declarations (no fns): sums match the GPU
        // receipt exactly, so both legs prove the same compiled graph.
        let mut cpu = arena_frame_graph().compile().expect("acyclic");
        let cpu_receipt = cpu.execute(&FrameInfo {
            frame_index: 11,
            width_px: 64,
            height_px: 64,
        });
        let gpu_compiled = arena_frame_graph().compile().expect("acyclic");
        let mut submitter = GpuSubmitter::new(device);
        let gpu = gpu_compiled
            .submit_gpu(device, queue, &gx1_test_frame(11), &mut submitter)
            .expect("gpu leg runs");
        assert_eq!(gpu_compiled.order(), cpu.order(), "same compiled order");
        assert_eq!(gpu.draws, cpu_receipt.draws, "twin draws match");
        assert_eq!(gpu.triangles, cpu_receipt.triangles, "twin triangles match");
        assert_eq!(gpu.passes_submitted, cpu_receipt.per_pass_cpu_ms.len());
        assert_eq!(gpu.transient_reuse_pct, cpu_receipt.transient_reuse_pct);
        assert!(gpu.is_clean() && gpu.covered_pixels > 0);
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_invalid_inputs_refuse_before_submit() {
        let Some((device, queue, _)) = gx1_test_device() else {
            eprintln!("SKIPPED gx1 invalid inputs: no wgpu adapter on this machine");
            return;
        };
        let compiled = arena_frame_graph().compile().expect("acyclic");
        let mut submitter = GpuSubmitter::new(device);
        let bad = crate::gtao::LookFrame {
            viewport_px: [0, 64],
            ..gx1_test_frame(11)
        };
        match compiled.submit_gpu(device, queue, &bad, &mut submitter) {
            Err(GpuSubmitError::Invalid(detail)) => assert!(!detail.is_empty(), "never silent"),
            other => panic!("zero viewport must fail Invalid, got {other:?}"),
        }
        // Over-budget backing refuses before any target is sized.
        let mut hog = FrameGraph::new();
        hog.add_pass(
            PassDesc::new("hog")
                .transient("hog", GPU_SUBMIT_TRANSIENT_BUDGET_BYTES + 1)
                .work(1, 1),
        );
        let hog = hog.compile().expect("single pass compiles");
        match hog.submit_gpu(device, queue, &gx1_test_frame(12), &mut submitter) {
            Err(GpuSubmitError::Invalid(detail)) => {
                assert!(detail.contains("budget"), "names the budget: {detail}");
            }
            other => panic!("over-budget backing must fail Invalid, got {other:?}"),
        }
        assert_eq!(
            (submitter.slot_texture_count(), submitter.bind_group_count()),
            (0, 0),
            "refused submits size nothing"
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_cpu_planning_allocates_nothing() {
        let frame = gx1_test_frame(11);
        frame.validate().expect("fixture frame validates");
        let compiled = arena_frame_graph().compile().expect("acyclic");
        let mut slots = 0usize;
        for a in compiled.transient_allocs() {
            slots = slots.max(a.slot + 1);
        }
        if slots == 0 {
            slots = 1;
        }
        let (sink, heap) = engine_core::alloc_probe::measure(|| {
            let bytes = look_frame_uniform_bytes(&frame);
            let mut sink = usize::from(bytes[0]);
            for pos in 0..compiled.order().len() {
                let (slot, clear) =
                    slot_target_for_position(compiled.transient_allocs(), pos, slots);
                sink = sink.wrapping_add(slot).wrapping_add(clear as usize);
            }
            sink.wrapping_add(compiled.transient_bytes_backing() as usize)
                .wrapping_add(usize::try_from(compiled.transient_reuse_pct()).unwrap_or(0))
        });
        std::hint::black_box(sink);
        assert_eq!(heap.calls, 0, "steady-state planning must not allocate");
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_submitter_reuses_targets_and_binds() {
        let Some((device, queue, _)) = gx1_test_device() else {
            eprintln!("SKIPPED gx1 reuse: no wgpu adapter on this machine");
            return;
        };
        let compiled = arena_frame_graph().compile().expect("acyclic");
        let mut submitter = GpuSubmitter::new(device);
        assert_eq!(
            (submitter.slot_texture_count(), submitter.bind_group_count()),
            (0, 0)
        );
        let first = compiled
            .submit_gpu(device, queue, &gx1_test_frame(11), &mut submitter)
            .expect("first submits");
        let shape = (submitter.slot_texture_count(), submitter.bind_group_count());
        assert_eq!(shape.0, first.transient_slots);
        assert_eq!(shape.1, first.passes_submitted);
        let second = compiled
            .submit_gpu(device, queue, &gx1_test_frame(12), &mut submitter)
            .expect("second submits");
        assert_eq!(
            (submitter.slot_texture_count(), submitter.bind_group_count()),
            shape,
            "steady-state submit creates no new targets or bind groups"
        );
        assert!(second.is_clean() && second.covered_pixels > 0);
        // Wall-clock budget lives in gx1_submit_wall_budget (--perf only):
        // shared-device submit_ms is load-dependent, never a default gate.
    }

    #[test]
    #[ignore = "timing: GPU submit wall-clock needs --perf on the idle reference GPU"]
    #[cfg(feature = "window")]
    fn gx1_submit_wall_budget() {
        let Some((device, queue, _)) = gx1_test_device() else {
            eprintln!("SKIPPED gx1 wall budget: no wgpu adapter on this machine");
            return;
        };
        let compiled = arena_frame_graph().compile().expect("acyclic");
        let mut submitter = GpuSubmitter::new(device);
        // Perf harness lines: warm once so the budget reads steady state, not
        // compile (same convention as ibl_chart_submit_wall_budget), then log it.
        let _ = compiled
            .submit_gpu(device, queue, &gx1_test_frame(12), &mut submitter)
            .expect("warmup submits");
        let second = compiled
            .submit_gpu(device, queue, &gx1_test_frame(12), &mut submitter)
            .expect("perf submit runs");
        eprintln!(
            "gx1_submit_wall_budget: steady submit {:.2}ms vs {:.1}ms budget",
            second.submit_ms,
            second.budget_ms()
        );
        assert!(
            second.within_budget(),
            "submit {:.2}ms over budget",
            second.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_receipt_budget_accessors_are_exact() {
        // Deterministic budget logic (no wall clock): at, under and over
        // the posted budget plus the transient cap.
        let at = GpuFrameReceipt {
            frame_index: 1,
            width_px: 64,
            height_px: 64,
            passes_submitted: 7,
            draws: 411,
            triangles: 2_396_011,
            transient_slots: 4,
            transient_bytes_total: 40_000_000,
            transient_bytes_backing: GPU_SUBMIT_TRANSIENT_BUDGET_BYTES,
            transient_reuse_pct: 99,
            validation_errors: 0,
            covered_pixels: 4096,
            submit_ms: GPU_SUBMIT_BUDGET_MS,
            per_pass_gpu_ms: vec![Some(0.5); 7],
        };
        assert_eq!(at.budget_ms(), GPU_SUBMIT_BUDGET_MS);
        assert!(at.within_budget(), "exactly at budget is within");
        assert!(at.within_transient_budget(), "exactly at cap is within");
        assert!(at.is_clean());
        let over = GpuFrameReceipt {
            submit_ms: GPU_SUBMIT_BUDGET_MS + 0.5,
            transient_bytes_backing: GPU_SUBMIT_TRANSIENT_BUDGET_BYTES + 1,
            validation_errors: 1,
            ..at
        };
        assert!(!over.within_budget(), "over budget fails loudly");
        assert!(!over.within_transient_budget());
        assert!(!over.is_clean());
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_timestamp_ticks_convert_by_period() {
        use crate::executor::{pass_ms_from_ticks, query_pair, ticks_to_ms, timestamp_query_count};
        assert_eq!(ticks_to_ms(0, 1.0), 0.0);
        assert!((ticks_to_ms(1_000, 1.0) - 0.001).abs() < 1e-12);
        assert!((ticks_to_ms(1_000_000, 1.0) - 1.0).abs() < 1e-9);
        assert!((pass_ms_from_ticks(10, 11_010, 1.0) - 0.011).abs() < 1e-9);
        assert_eq!(pass_ms_from_ticks(50, 20, 1.0), 0.0);
        assert_eq!(query_pair(0), (0, 1));
        assert_eq!(query_pair(3), (6, 7));
        assert_eq!(timestamp_query_count(7), 14);
        assert_eq!(timestamp_query_count(0), 0);
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_timestamp_decode_needs_full_pairs() {
        use crate::executor::{decode_pass_bytes, none_gpu_timings};
        let period = 1.0;
        let mut bytes = Vec::new();
        for (begin, end) in [(100u64, 1_100u64), (2_000u64, 5_000u64)] {
            bytes.extend_from_slice(&begin.to_le_bytes());
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        let rows = decode_pass_bytes(&bytes, 2, period);
        assert_eq!(rows.len(), 2);
        assert!((rows[0].expect("first pair resolves") - 0.001).abs() < 1e-9);
        assert!((rows[1].expect("second pair resolves") - 0.003).abs() < 1e-9);
        let short = decode_pass_bytes(&bytes[..8], 2, period);
        assert_eq!(short.len(), 2);
        assert!(short[0].is_none(), "half a pair never resolves");
        assert!(short[1].is_none(), "missing tail stays None");
        let none = none_gpu_timings(3);
        assert_eq!(none, vec![None, None, None]);
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_timestamps_resolve_per_pass_or_degrade_clean() {
        let Some((device, queue, info)) = gx1_test_device() else {
            eprintln!("SKIPPED gx1 timestamps: no wgpu adapter on this machine");
            return;
        };
        let compiled = arena_frame_graph().compile().expect("acyclic");
        let mut submitter = GpuSubmitter::new(device);
        let receipt = compiled
            .submit_gpu(device, queue, &gx1_test_frame(13), &mut submitter)
            .expect("timestamp submit runs");
        assert!(receipt.is_clean(), "no validation error with timestamps");
        assert_eq!(receipt.per_pass_gpu_ms.len(), receipt.passes_submitted);
        assert!(receipt.covered_pixels > 0, "passes still write pixels");
        if crate::executor::timestamp_supported(device) {
            assert!(
                receipt
                    .per_pass_gpu_ms
                    .iter()
                    .all(|ms| { matches!(ms, Some(ms) if ms.is_finite() && *ms >= 0.0) }),
                "reference GPU posts finite ms per pass: {:?}",
                receipt.per_pass_gpu_ms
            );
        } else {
            assert!(
                receipt.per_pass_gpu_ms.iter().all(|ms| ms.is_none()),
                "fallback without TIMESTAMP_QUERY degrades to None: {:?}",
                receipt.per_pass_gpu_ms
            );
        }
        eprintln!(
            "window timestamps on {} (supported={}): {:?}",
            info.name,
            crate::executor::timestamp_supported(device),
            receipt.per_pass_gpu_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gx1_error_names_cause_without_unsupported() {
        for err in [
            GpuSubmitError::Invalid("bad frame".to_string()),
            GpuSubmitError::Device("poll".to_string()),
            GpuSubmitError::Validation("scope".to_string()),
        ] {
            let text = format!("{err}");
            assert!(!text.is_empty(), "errors are never silent");
            assert!(
                !text.contains("unsupported"),
                "window path never reports Unsupported: {text}"
            );
        }
    }

    #[test]
    #[cfg(feature = "window")]
    fn sl21_window_submit_alloc_audit_reports_na_floor() {
        // SL21 audit over the RA1 window submit slice, compiled with the
        // `window` feature: the same functions the offscreen/surface submits
        // call (`decode_depth_feed` per live frame plus the zero-alloc pack
        // and config helpers). Pattern:
        // `docs/library/counting-allocator-floors.md`; counts ride the
        // shared `probe` above. JAN28-1 floor: the decode is N/A for
        // zero-alloc (4 buffers per call); the helpers post 0.
        let ident = [
            [1.0f32, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let raw = vec![0.5f32; 16 * 16];
        let warm =
            crate::present::decode_depth_feed(&raw, 16, 16, ident, [0.0, 0.0, 0.0]).expect("warms");
        assert_eq!(warm.texels(), 256);
        std::hint::black_box(&warm);
        let (feed, heap) = engine_core::alloc_probe::measure(|| {
            crate::present::decode_depth_feed(&raw, 16, 16, ident, [0.0, 0.0, 0.0])
                .expect("decodes")
        });
        std::hint::black_box(&feed);
        let count = heap.calls;
        eprintln!(
            "SL21 window submit audit (decode 16x16, window cfg): {count} allocs (floor 4: world+depth+height+normals)"
        );
        assert_eq!(
            count, 4,
            "decode floor is 4 buffers per frame (JAN28-1 N/A)"
        );
        assert_eq!(feed.texels(), 256);
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            let packed = crate::present::fog_blit_uniform_bytes([0.5, 0.25, 1.0], 64, 32);
            std::hint::black_box(packed);
            let contact = crate::present::contact_for_frame(64, 64).expect("live size builds");
            std::hint::black_box(&contact);
        });
        assert_eq!(
            heap.calls, 0,
            "uniform pack plus contact config allocate nothing"
        );
        assert!(
            matches!(
                crate::present::decode_depth_feed(&raw[..10], 16, 16, ident, [0.0, 0.0, 0.0]),
                Err(crate::present::PresentError::Readback(_))
            ),
            "ragged depth is refused"
        );
    }
}
