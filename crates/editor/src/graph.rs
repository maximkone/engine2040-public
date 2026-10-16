//! Node-graph substrate for material and VFX graphs.
//! Consumer is editor document plus save codec. Owner is [`Graph`].
//! Units are ids and pins as `u32`, pos in editor pixels.
//! Determinism is `BTreeMap`/`BTreeSet` order plus canonical bytes.
//! Errors name the culprit node.
//! PORT(zakarumych/egui-snarl@23e7489:src/lib.rs): pattern only.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
/// Magic for graph bytes.
pub const GRAPH_MAGIC: [u8; 4] = *b"GRPH";
/// Newest graph version written and read.
pub const GRAPH_VERSION: u32 = 1;
/// Oldest graph version still opened.
pub const GRAPH_MIN_VERSION: u32 = 1;
/// Largest accepted node count.
pub const MAX_GRAPH_NODES: usize = 1024;
/// Largest accepted wire count.
pub const MAX_GRAPH_WIRES: usize = 4096;
/// Largest accepted graph bytes.
pub const MAX_GRAPH_BYTES: usize = 1 << 16;
/// Graph node id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphId(pub u32);
/// Input pin: node plus index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InPinId {
    pub node: GraphId,
    pub pin: u32,
}
/// Output pin: node plus index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutPinId {
    pub node: GraphId,
    pub pin: u32,
}
/// Node payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeKind {
    Const(f32),
    Op(u8),
    Material(u8),
    Vfx(u8),
}
/// One node: payload plus pos plus open flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphNode {
    pub kind: NodeKind,
    pub pos: [f32; 2],
    pub open: bool,
}
/// One wire: output pin to input pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Wire {
    pub out_node: GraphId,
    pub out_pin: u32,
    pub in_node: GraphId,
    pub in_pin: u32,
}
/// What refused a graph edit or decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    UnknownNode { id: u32 },
    Dangling { node: u32 },
    Cycle { node: u32 },
    NonFinite { node: u32 },
    Duplicate { id: u32 },
    BadMagic,
    UnsupportedVersion { got: u32 },
    Truncated,
    CrcMismatch,
    TooLarge,
}
impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for GraphError {}
/// Data-only node graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    nodes: BTreeMap<u32, GraphNode>,
    wires: BTreeSet<Wire>,
    next: u32,
}
impl Graph {
    /// Empty graph.
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            wires: BTreeSet::new(),
            next: 1,
        }
    }
    /// Node count.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    /// Wire count.
    pub fn wire_count(&self) -> usize {
        self.wires.len()
    }
    /// Node at an id.
    pub fn get(&self, id: GraphId) -> Option<&GraphNode> {
        self.nodes.get(&id.0)
    }
    /// Insert a node and return its fresh id.
    pub fn insert(&mut self, kind: NodeKind, pos: [f32; 2], open: bool) -> GraphId {
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        if id == 0 {
            return self.insert(kind, pos, open);
        }
        self.nodes.insert(id, GraphNode { kind, pos, open });
        GraphId(id)
    }
    /// Remove a node plus its wires. False when absent.
    pub fn remove(&mut self, id: GraphId) -> bool {
        if self.nodes.remove(&id.0).is_none() {
            return false;
        }
        self.wires.retain(|w| w.out_node != id && w.in_node != id);
        true
    }
    /// Connect output to input. Ok(false) when the wire already exists.
    pub fn connect(&mut self, out: OutPinId, inp: InPinId) -> Result<bool, GraphError> {
        if !self.nodes.contains_key(&out.node.0) {
            return Err(GraphError::UnknownNode { id: out.node.0 });
        }
        if !self.nodes.contains_key(&inp.node.0) {
            return Err(GraphError::UnknownNode { id: inp.node.0 });
        }
        let wire = Wire {
            out_node: out.node,
            out_pin: out.pin,
            in_node: inp.node,
            in_pin: inp.pin,
        };
        if self.wires.contains(&wire) {
            return Ok(false);
        }
        if inp.node == out.node || self.reaches(inp.node.0, out.node.0) {
            return Err(GraphError::Cycle { node: inp.node.0 });
        }
        self.wires.insert(wire);
        Ok(true)
    }
    /// Drop one wire. False when absent.
    pub fn disconnect(&mut self, out: OutPinId, inp: InPinId) -> bool {
        self.wires.remove(&Wire {
            out_node: out.node,
            out_pin: out.pin,
            in_node: inp.node,
            in_pin: inp.pin,
        })
    }
    /// Check dangling wires, finite values, and cycles.
    pub fn validate(&self) -> Result<(), GraphError> {
        for (id, node) in &self.nodes {
            let bad = match node.kind {
                NodeKind::Const(v) => !v.is_finite(),
                _ => false,
            } || !node.pos[0].is_finite()
                || !node.pos[1].is_finite();
            if bad {
                return Err(GraphError::NonFinite { node: *id });
            }
        }
        for w in &self.wires {
            if !self.nodes.contains_key(&w.out_node.0) {
                return Err(GraphError::Dangling { node: w.out_node.0 });
            }
            if !self.nodes.contains_key(&w.in_node.0) {
                return Err(GraphError::Dangling { node: w.in_node.0 });
            }
        }
        for id in self.nodes.keys() {
            if self.reaches(*id, *id) {
                return Err(GraphError::Cycle { node: *id });
            }
        }
        Ok(())
    }
    /// Canonical bytes: sorted ids and wires, sealed end, CRC32.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&GRAPH_MAGIC);
        out.extend_from_slice(&GRAPH_VERSION.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.nodes.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for (id, node) in &self.nodes {
            let (tag, payload) = match node.kind {
                NodeKind::Const(v) => (0u8, v.to_bits()),
                NodeKind::Op(c) => (1u8, u32::from(c)),
                NodeKind::Material(c) => (2u8, u32::from(c)),
                NodeKind::Vfx(c) => (3u8, u32::from(c)),
            };
            out.extend_from_slice(&id.to_le_bytes());
            out.push(tag);
            out.extend_from_slice(&payload.to_le_bytes());
            out.extend_from_slice(&node.pos[0].to_le_bytes());
            out.extend_from_slice(&node.pos[1].to_le_bytes());
            out.push(u8::from(node.open));
        }
        out.extend_from_slice(
            &u32::try_from(self.wires.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for w in &self.wires {
            out.extend_from_slice(&w.out_node.0.to_le_bytes());
            out.extend_from_slice(&w.out_pin.to_le_bytes());
            out.extend_from_slice(&w.in_node.0.to_le_bytes());
            out.extend_from_slice(&w.in_pin.to_le_bytes());
        }
        let checksum = crate::save::crc32(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        out
    }
    /// Decode canonical bytes; never panics.
    pub fn decode(bytes: &[u8]) -> Result<Self, GraphError> {
        if bytes.len() > MAX_GRAPH_BYTES {
            return Err(GraphError::TooLarge);
        }
        if bytes.len() < 20 {
            return Err(GraphError::Truncated);
        }
        if bytes[0..4] != GRAPH_MAGIC {
            return Err(GraphError::BadMagic);
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version < GRAPH_MIN_VERSION || version > GRAPH_VERSION {
            return Err(GraphError::UnsupportedVersion { got: version });
        }
        let count = usize::try_from(u32::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11],
        ]))
        .unwrap_or(usize::MAX);
        if count > MAX_GRAPH_NODES {
            return Err(GraphError::TooLarge);
        }
        let mut pos = 12;
        let mut nodes = BTreeMap::new();
        for _ in 0..count {
            let id = u32::from_le_bytes(take4(bytes, &mut pos)?);
            let tag = take1(bytes, &mut pos)?;
            let payload = u32::from_le_bytes(take4(bytes, &mut pos)?);
            let px = f32::from_le_bytes(take4(bytes, &mut pos)?);
            let py = f32::from_le_bytes(take4(bytes, &mut pos)?);
            let open = match take1(bytes, &mut pos)? {
                0 => false,
                1 => true,
                _ => return Err(GraphError::Truncated),
            };
            let kind = match tag {
                0 => NodeKind::Const(f32::from_bits(payload)),
                1 => NodeKind::Op(u8::try_from(payload).map_err(|_| GraphError::Truncated)?),
                2 => NodeKind::Material(u8::try_from(payload).map_err(|_| GraphError::Truncated)?),
                3 => NodeKind::Vfx(u8::try_from(payload).map_err(|_| GraphError::Truncated)?),
                _ => return Err(GraphError::Truncated),
            };
            if nodes
                .insert(
                    id,
                    GraphNode {
                        kind,
                        pos: [px, py],
                        open,
                    },
                )
                .is_some()
            {
                return Err(GraphError::Duplicate { id });
            }
        }
        let wcount =
            usize::try_from(u32::from_le_bytes(take4(bytes, &mut pos)?)).unwrap_or(usize::MAX);
        if wcount > MAX_GRAPH_WIRES {
            return Err(GraphError::TooLarge);
        }
        let mut wires = BTreeSet::new();
        for _ in 0..wcount {
            wires.insert(Wire {
                out_node: GraphId(u32::from_le_bytes(take4(bytes, &mut pos)?)),
                out_pin: u32::from_le_bytes(take4(bytes, &mut pos)?),
                in_node: GraphId(u32::from_le_bytes(take4(bytes, &mut pos)?)),
                in_pin: u32::from_le_bytes(take4(bytes, &mut pos)?),
            });
        }
        if pos != bytes.len() - 4 {
            return Err(GraphError::Truncated);
        }
        let mut tail = [0u8; 4];
        tail.copy_from_slice(&bytes[bytes.len() - 4..]);
        if u32::from_le_bytes(tail) != crate::save::crc32(&bytes[..bytes.len() - 4]) {
            return Err(GraphError::CrcMismatch);
        }
        let mut next = 1u32;
        for id in nodes.keys() {
            next = next.max(id.wrapping_add(1));
        }
        let graph = Self { nodes, wires, next };
        graph.validate()?;
        Ok(graph)
    }
    /// True when `to` is reachable from `from` over directed wires.
    fn reaches(&self, from: u32, to: u32) -> bool {
        let mut seen = BTreeSet::new();
        let mut stack = Vec::new();
        for w in &self.wires {
            if w.out_node.0 == from {
                stack.push(w.in_node.0);
            }
        }
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.insert(n) {
                for w in &self.wires {
                    if w.out_node.0 == n {
                        stack.push(w.in_node.0);
                    }
                }
            }
        }
        false
    }
}
/// Take four bytes, advancing the cursor.
fn take4(bytes: &[u8], pos: &mut usize) -> Result<[u8; 4], GraphError> {
    let end = pos.saturating_add(4);
    if end > bytes.len() {
        return Err(GraphError::Truncated);
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[*pos..end]);
    *pos = end;
    Ok(buf)
}
/// Take one byte, advancing the cursor.
fn take1(bytes: &[u8], pos: &mut usize) -> Result<u8, GraphError> {
    let end = pos.saturating_add(1);
    if end > bytes.len() {
        return Err(GraphError::Truncated);
    }
    let b = bytes[*pos];
    *pos = end;
    Ok(b)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_roundtrip_x2_ignores_duplicate_and_refuses_cycle() {
        let mut graph = Graph::new();
        let kinds = [
            NodeKind::Const(1.0),
            NodeKind::Const(2.0),
            NodeKind::Op(1),
            NodeKind::Op(2),
            NodeKind::Material(3),
            NodeKind::Material(4),
            NodeKind::Vfx(5),
            NodeKind::Vfx(6),
        ];
        let mut ids = Vec::new();
        let xs = [0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5];
        for (i, kind) in kinds.into_iter().enumerate() {
            ids.push(graph.insert(kind, [xs[i], 0.0], true));
        }
        assert_eq!(graph.node_count(), 8);
        let wires = [
            (0, 0, 2, 0),
            (1, 0, 2, 1),
            (2, 0, 3, 0),
            (3, 0, 4, 0),
            (4, 0, 5, 0),
            (5, 0, 6, 0),
            (6, 0, 7, 0),
            (0, 0, 3, 1),
            (1, 0, 4, 1),
            (2, 0, 5, 1),
            (3, 0, 6, 1),
        ];
        for (a, ap, b, bp) in wires {
            assert_eq!(
                graph
                    .connect(
                        OutPinId {
                            node: ids[a],
                            pin: ap
                        },
                        InPinId {
                            node: ids[b],
                            pin: bp
                        }
                    )
                    .unwrap(),
                true
            );
        }
        assert_eq!(
            graph
                .connect(
                    OutPinId {
                        node: ids[0],
                        pin: 0
                    },
                    InPinId {
                        node: ids[2],
                        pin: 0
                    }
                )
                .unwrap(),
            false
        );
        assert_eq!(graph.wire_count(), 11);
        let err = graph
            .connect(
                OutPinId {
                    node: ids[7],
                    pin: 0,
                },
                InPinId {
                    node: ids[2],
                    pin: 2,
                },
            )
            .unwrap_err();
        assert!(matches!(err, GraphError::Cycle { .. }));
        assert!(err.to_string().contains(&ids[2].0.to_string()));
        let first = graph.encode();
        assert_eq!(first, graph.encode());
        let back = Graph::decode(&first).unwrap();
        assert_eq!(back, graph);
        assert_eq!(back.encode(), first);
    }
}
