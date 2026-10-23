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
/// Newest graph version written and read. Version 2 adds the rule node tag (`4`), whose
/// payload is a rule slot; version 1 files still open and simply never carry one.
pub const GRAPH_VERSION: u32 = 2;
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
/// Node payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeKind {
    Const(f32),
    Op(u8),
    Material(u8),
    Vfx(u8),
    /// A rule node: its payload is the slot of the Rhai text this node stands for, and
    /// [`Graph::compile_rules`] turns the graph into that text.
    Rule(u16),
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
    Dangling {
        node: u32,
    },
    Cycle {
        node: u32,
    },
    NonFinite {
        node: u32,
    },
    Duplicate {
        id: u32,
    },
    /// A rule node names a slot whose text the caller did not supply.
    MissingRule {
        slot: u16,
    },
    BadMagic,
    UnsupportedVersion {
        got: u32,
    },
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
    /// The Rhai source this graph stands for: every [`NodeKind::Rule`] node's slot text, in
    /// node-id order, one blank line between two rules.
    ///
    /// The text is not generated: a rule node names the script the rules crate already runs,
    /// so the compiled bytes are the exact text the text path hashes and a rule hash cannot
    /// drift between the two. A slot with no text is refused, never skipped.
    pub fn compile_rules(&self, texts: &[&str]) -> Result<String, GraphError> {
        let mut out = String::new();
        for node in self.nodes.values() {
            let NodeKind::Rule(slot) = node.kind else {
                continue;
            };
            let text = texts
                .get(usize::from(slot))
                .ok_or(GraphError::MissingRule { slot })?;
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
            if out.len() > MAX_GRAPH_BYTES {
                return Err(GraphError::TooLarge);
            }
        }
        Ok(out)
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
                NodeKind::Rule(slot) => (4u8, u32::from(slot)),
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
        if !(GRAPH_MIN_VERSION..=GRAPH_VERSION).contains(&version) {
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
                4 => NodeKind::Rule(u16::try_from(payload).map_err(|_| GraphError::Truncated)?),
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

    /// One node row in canonical bytes: id, tag, payload, x, y, open flag.
    fn push_node(out: &mut Vec<u8>, id: u32, tag: u8, payload: u32, x: f32, y: f32, open: bool) {
        out.extend_from_slice(&id.to_le_bytes());
        out.push(tag);
        out.extend_from_slice(&payload.to_le_bytes());
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        out.push(u8::from(open));
    }

    /// One wire row in canonical bytes: out node/pin, in node/pin.
    fn push_wire(out: &mut Vec<u8>, out_node: u32, out_pin: u32, in_node: u32, in_pin: u32) {
        out.extend_from_slice(&out_node.to_le_bytes());
        out.extend_from_slice(&out_pin.to_le_bytes());
        out.extend_from_slice(&in_node.to_le_bytes());
        out.extend_from_slice(&in_pin.to_le_bytes());
    }

    /// Seal canonical bytes: header plus node/wire rows plus CRC32.
    fn seal(nodes: &[(u32, u8, u32, f32, f32, bool)], wires: &[(u32, u32, u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&GRAPH_MAGIC);
        out.extend_from_slice(&GRAPH_VERSION.to_le_bytes());
        out.extend_from_slice(&u32::try_from(nodes.len()).unwrap_or(u32::MAX).to_le_bytes());
        for (id, tag, payload, x, y, open) in nodes {
            push_node(&mut out, *id, *tag, *payload, *x, *y, *open);
        }
        out.extend_from_slice(&u32::try_from(wires.len()).unwrap_or(u32::MAX).to_le_bytes());
        for (a, ap, b, bp) in wires {
            push_wire(&mut out, *a, *ap, *b, *bp);
        }
        let checksum = crate::save::crc32(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        out
    }

    #[test]
    fn graph_roundtrip_x2_through_decode_only() {
        let bytes = seal(
            &[
                (1, 0, 1.0f32.to_bits(), 0.5, 0.0, true),
                (2, 0, 2.0f32.to_bits(), 1.5, 0.0, true),
                (3, 1, 1, 2.5, 0.0, false),
            ],
            &[(1, 0, 3, 0), (2, 0, 3, 1)],
        );
        let graph = Graph::decode(&bytes).expect("sealed bytes decode");
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.wire_count(), 2);
        assert_eq!(
            graph.get(GraphId(1)).map(|node| node.kind),
            Some(NodeKind::Const(1.0))
        );
        let first = graph.encode();
        assert_eq!(first, bytes, "decode then encode is byte-identical");
        let back = Graph::decode(&first).expect("re-decode works");
        assert_eq!(back, graph);
        assert_eq!(back.encode(), first);
    }

    #[test]
    fn a_rule_node_compiles_to_the_exact_rhai_text() {
        // WG6: the graph compiles to the text the rules crate already runs, so the rule
        // hash matches the text path by construction (the same bytes, not a rewrite).
        let text = "fn on_tick(me) {\n    damage(me, 1);\n}\n";
        let other = "fn on_hit(me, by) {\n    say(\"hit\");\n}\n";
        let graph = Graph::decode(&seal(
            &[
                (1, 4, 1, 0.0, 0.0, true),
                (2, 4, 0, 1.0, 0.0, true),
                (3, 0, 1.5f32.to_bits(), 2.0, 0.0, true),
            ],
            &[],
        ))
        .expect("a graph with rule nodes decodes");
        assert_eq!(
            graph.encode(),
            seal(
                &[
                    (1, 4, 1, 0.0, 0.0, true),
                    (2, 4, 0, 1.0, 0.0, true),
                    (3, 0, 1.5f32.to_bits(), 2.0, 0.0, true),
                ],
                &[]
            ),
            "a rule node round-trips through the bytes"
        );
        assert_eq!(
            graph.compile_rules(&[other, text]).unwrap(),
            format!("{text}\n{other}"),
            "node-id order, one blank line between rules, text unchanged"
        );
        let refused = graph.compile_rules(&[other]).unwrap_err();
        assert_eq!(refused, GraphError::MissingRule { slot: 1 });
        assert!(refused.to_string().contains("MissingRule"));
        // A graph with no rule node compiles to nothing, never to a stub script.
        let constants =
            Graph::decode(&seal(&[(1, 0, 1.0f32.to_bits(), 0.0, 0.0, true)], &[])).unwrap();
        assert_eq!(constants.compile_rules(&[text]).unwrap(), "");
    }

    #[test]
    fn decode_refuses_cycle_truncation_and_bad_checksum() {
        // Wires 1 -> 2 and 2 -> 1 form a cycle over directed wires.
        let cyclic = seal(
            &[
                (1, 0, 1.0f32.to_bits(), 0.0, 0.0, true),
                (2, 0, 2.0f32.to_bits(), 1.0, 0.0, true),
            ],
            &[(1, 0, 2, 0), (2, 0, 1, 0)],
        );
        let err = Graph::decode(&cyclic).unwrap_err();
        assert!(matches!(err, GraphError::Cycle { .. }));
        assert!(err.to_string().contains("Cycle"));

        let good = seal(&[(1, 0, 1.0f32.to_bits(), 0.0, 0.0, true)], &[]);
        assert!(matches!(
            Graph::decode(&good[..good.len() - 5]).unwrap_err(),
            GraphError::Truncated
        ));
        let mut corrupt = good.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xFF;
        assert!(matches!(
            Graph::decode(&corrupt).unwrap_err(),
            GraphError::CrcMismatch
        ));
        let mut bad_magic = good;
        bad_magic[0] = b'X';
        assert!(matches!(
            Graph::decode(&bad_magic).unwrap_err(),
            GraphError::BadMagic | GraphError::CrcMismatch
        ));
    }
}
