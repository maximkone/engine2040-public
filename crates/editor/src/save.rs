//! Document save: versioned binary snapshot of a [`Document`].
//!
//! The consumer is the editor shell (save/reopen menu, end-to-end workflow); the
//! data owner is [`Document`](crate::commands::Document) from the command
//! service. Units follow `engine-scene`: meters for positions, radians for
//! rotations. Everything here is std-only plus `engine-scene`: no serde, no
//! filesystem, no new dependency.
//!
//! Layout (all integers little-endian) is `MAGIC` (4) + version (`u32`) +
//! revision (`u64`) + node count (`u32`) + nodes + material bindings (v2
//! only) + CRC32 (`u32`). Each node is a hyphenated id (36 bytes) + a
//! hyphenated parent (36 bytes, nil for a root) + ten `f32` pose components
//! (40 bytes), so one node is 112 bytes. The v2 material section is a count
//! (`u32`) plus entries sorted by id text, each a hyphenated id (36 bytes) +
//! name length (`u16`) + name bytes; it names the edited material per node so
//! the binding survives close/reopen in the scene bytes. The v3 component
//! section follows it: a count (`u32`) plus entries sorted by node id text
//! then component tag, each a hyphenated id (36 bytes) + length (`u16`) + the
//! canonical component bytes ([`Component::to_bytes`]); v2 files open with no
//! components. Nodes encode sorted
//! by id text, so the same document always saves to the same bytes. The CRC
//! covers every byte before it.
//!
//! Bounds: at most [`MAX_SAVE_NODES`] nodes and [`MAX_SAVE_BYTES`] total
//! bytes (64k nodes need ~7.3 MiB, inside the 8 MiB cap). Decoding checks
//! the magic, the version gate, the exact length, the CRC and then the
//! reference structure (nil ids, duplicates, finite poses, known parents,
//! acyclic links, bindings naming known nodes), naming the culprit id on
//! every structural failure. Any failure returns an error; a bad save never
//! panics and never leaves a partial document behind.

use std::fmt;

use engine_scene::component::Component;
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::commands::{ComponentTable, Document};
use crate::commands::{MaterialName, MAX_MATERIAL_NAME_LEN};

/// File magic for editor document saves. Distinct from the `E204` game-save
/// magic in `engine-save`: these are authoring documents, not progression.
pub const SAVE_MAGIC: [u8; 4] = *b"EDOC";
/// Newest save version this module writes and reads. Version 2 adds the
/// material section after the nodes, version 3 the component section after
/// that, and version 4 the graph section after that; older files still open
/// with empty bindings, components, and graphs.
pub const SAVE_VERSION: u32 = 4;
/// Oldest save version this module still opens. Older saves, and saves newer
/// than [`SAVE_VERSION`], fail with [`SaveError::UnsupportedVersion`].
pub const MIN_SAVE_VERSION: u32 = 1;
/// Largest accepted save in bytes (8 MiB).
pub const MAX_SAVE_BYTES: usize = 8 << 20;
/// Largest accepted node count (64k nodes need ~7.3 MiB, inside the byte cap).
pub const MAX_SAVE_NODES: usize = 65_536;
/// Bytes per node: 36 id + 36 parent + 10 f32 pose components.
const NODE_BYTES: usize = 36 + 36 + 10 * 4;
/// Header bytes: magic (4) + version (4) + revision (8) + count (4).
const HEADER_BYTES: usize = 4 + 4 + 8 + 4;
/// Trailing CRC32 bytes.
const CRC_BYTES: usize = 4;

/// What went wrong saving or reopening a document.
///
/// Every structural variant names the culprit node (and the missing parent
/// where a link dangles), so the shell can quote it in diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// Magic mismatch: not an editor document save.
    BadMagic,
    /// Version newer than this reader or older than the oldest migration.
    /// Carries what was found plus the accepted range.
    UnsupportedVersion {
        /// Version stamped on the save.
        got: u32,
        /// Oldest version this reader opens ([`MIN_SAVE_VERSION`]).
        min: u32,
        /// Newest version this reader opens ([`SAVE_VERSION`]).
        current: u32,
    },
    /// Short input, a length field that does not match the input, or bytes
    /// past the sealed end.
    Truncated,
    /// CRC32 mismatch: corrupt or tampered bytes.
    CrcMismatch,
    /// Node count or total size past the bounded caps.
    TooLarge,
    /// A node id was nil; nil is the "no parent" slot, not a node.
    NilId,
    /// Two entries share one id.
    Duplicate {
        /// The repeated id.
        id: ObjectId,
    },
    /// A parent link names no node in the same save.
    MissingParent {
        /// The node holding the dangling link.
        node: ObjectId,
        /// The parent id no entry provides.
        parent: ObjectId,
    },
    /// Parent links form a loop, so no insertion order can satisfy them.
    Cycle {
        /// A node stuck in the loop.
        node: ObjectId,
    },
    /// A saved pose holds NaN or infinity.
    NonFinite {
        /// The node holding the bad pose.
        id: ObjectId,
    },
    /// A v2 material binding is invalid: the name is malformed or the node
    /// is not in the same save.
    BadMaterial {
        /// The node holding the bad binding.
        id: ObjectId,
    },
    /// A v3 component entry is invalid: the bytes fail their schema, the
    /// kind repeats on one node, or the node is not in the same save.
    BadComponent {
        /// The node holding the bad component.
        id: ObjectId,
    },
    /// The v4 graph section failed its own checksum or structure gate.
    BadGraph,
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not an editor document save (bad magic)"),
            Self::UnsupportedVersion { got, min, current } => write!(
                f,
                "unsupported document version v{got}; this reader opens v{min}..=v{current} \
                 (save is newer than the reader or older than the oldest migration)"
            ),
            Self::Truncated => f.write_str("document save is truncated or has trailing bytes"),
            Self::CrcMismatch => f.write_str("document save failed its checksum (corrupt)"),
            Self::TooLarge => f.write_str("document save exceeds its size bounds"),
            Self::NilId => f.write_str("document node id must not be nil"),
            Self::Duplicate { id } => write!(f, "duplicate document node id {id}"),
            Self::MissingParent { node, parent } => write!(
                f,
                "document node {node} names unknown parent {parent}; \
                 the parent entry is missing from the save"
            ),
            Self::Cycle { node } => {
                write!(f, "document parent links form a cycle at node {node}")
            }
            Self::NonFinite { id } => {
                write!(f, "document node {id} holds a non-finite transform")
            }
            Self::BadMaterial { id } => {
                write!(
                    f,
                    "document node {id} holds an invalid material binding \
                     (bad name or unknown node)"
                )
            }
            Self::BadComponent { id } => {
                write!(
                    f,
                    "document node {id} holds an invalid component \
                     (bad bytes, repeated kind or unknown node)"
                )
            }
            Self::BadGraph => f.write_str("document graph section is invalid"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Bitwise IEEE CRC32 (table-free, deterministic, std-only). Same polynomial
/// as the `engine-save` envelope so checksums agree across the two formats;
/// the algorithm is reimplemented here so this module needs no new crate edge.
/// Crate-visible: the durable journal, the WAL and round snapshots seal their
/// frames with the same checksum.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Save a document to versioned bytes: hierarchy plus revision plus v2
/// material bindings.
///
/// Nodes write sorted by id text, and bindings write sorted by node id text,
/// so repeated saves of the same document are byte-identical. ObjectIds, the
/// revision and every binding survive the round trip.
pub fn save_to_bytes(doc: &Document) -> Vec<u8> {
    // The command service keeps every stored id reachable with a parent and
    // a local pose, so these lookups resolve for a well-formed document. A
    // node that somehow lost its data is skipped rather than panicked on:
    // the reopened save still validates, it just lacks the broken entry.
    // Bindings ride the same rule: only nodes present in the hierarchy are
    // written, so the bytes never name a missing node.
    let mut ids = doc.hierarchy.propagation_order();
    ids.sort_by_key(|id| id.to_string());
    // Bindings follow their nodes: only hierarchy members are written (so
    // the bytes never name a missing node), already in id-text order.
    let bound: Vec<(ObjectId, MaterialName)> = ids
        .iter()
        .filter_map(|id| doc.materials.get(&id.to_string()).map(|name| (*id, *name)))
        .collect();
    let mut out = Vec::with_capacity(
        HEADER_BYTES
            + ids.len() * NODE_BYTES
            + 4
            + bound.len() * (36 + 2 + MAX_MATERIAL_NAME_LEN)
            + CRC_BYTES,
    );
    out.extend_from_slice(&SAVE_MAGIC);
    out.extend_from_slice(&SAVE_VERSION.to_le_bytes());
    out.extend_from_slice(&doc.revision.to_le_bytes());
    // A `u32` entry count needs 4B nodes, far past any in-memory document:
    // saturate to a count the loader rejects as truncated instead of
    // truncating silently or panicking the saver.
    let node_count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&node_count.to_le_bytes());
    for id in ids.iter().copied() {
        let (Some(parent), Some(local)) = (doc.hierarchy.parent(id), doc.hierarchy.local(id))
        else {
            continue;
        };
        push_id(&mut out, id);
        push_id(&mut out, parent);
        push_transform(&mut out, local);
    }
    let binding_count = u32::try_from(bound.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&binding_count.to_le_bytes());
    for (id, name) in &bound {
        push_id(&mut out, *id);
        let text = name.as_str();
        // Material names are capped at 64 bytes, so this never saturates;
        // the owner length keeps saves on the same conversion as digests.
        let name_len = u16::try_from(name.len_bytes()).unwrap_or(u16::MAX);
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }
    // Components follow the same rule as bindings: only hierarchy members,
    // node id text order, then component tag order (the table's own order).
    let mut entries: Vec<(ObjectId, Vec<u8>)> = Vec::new();
    for id in &ids {
        if let Some(own) = doc.components.get(&id.to_string()) {
            for component in own.values() {
                entries.push((*id, component.to_bytes()));
            }
        }
    }
    let component_count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&component_count.to_le_bytes());
    for (id, bytes) in &entries {
        push_id(&mut out, *id);
        // Component bytes are bounded well under 200 by their schemas.
        let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(bytes);
    }
    let graph_bytes = doc.graph.encode();
    let graph_len = u32::try_from(graph_bytes.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&graph_len.to_le_bytes());
    out.extend_from_slice(&graph_bytes);
    let checksum = crc32(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

/// Reopen document bytes saved by [`save_to_bytes`].
///
/// Rejects bad magic, unsupported versions, truncated or trailing bytes,
/// checksum failures, oversize input and broken reference structure, naming
/// the culprit on structural failures. Never panics on any input.
pub fn load_from_bytes(bytes: &[u8]) -> Result<Document, SaveError> {
    let Header {
        version,
        revision,
        count,
    } = read_header(bytes)?;
    let mut pos = HEADER_BYTES;
    let mut nodes = read_nodes(bytes, &mut pos, count)?;
    let materials = if version >= 2 {
        read_materials(bytes, &mut pos)?
    } else {
        Vec::new()
    };
    let components = if version >= 3 {
        read_components(bytes, &mut pos)?
    } else {
        Vec::new()
    };
    let graph = if version >= 4 {
        canonical_graph(read_graph(bytes, &mut pos)?)
    } else {
        crate::graph::Graph::default()
    };
    if version >= 2 {
        seal_check(bytes, pos)?;
    }
    let bound = check_structure(&mut nodes, &materials, &components)?;
    let tree = build_hierarchy(&nodes)?;
    let mut table = ComponentTable::new();
    for (id, component) in components {
        table
            .entry(id.to_string())
            .or_default()
            .insert(component.kind(), component);
    }
    Ok(Document {
        hierarchy: tree,
        revision,
        materials: bound
            .into_iter()
            .map(|(id, name)| (id.to_string(), name))
            .collect(),
        components: table,
        graph,
    })
}

/// One decoded node: id, parent id (nil for a root) and local pose.
type Node = (ObjectId, ObjectId, Transform);

/// Fixed header fields that passed the length, magic and version gates.
struct Header {
    version: u32,
    revision: u64,
    count: usize,
}

/// Validate the header and the exact-length gates for the version.
fn read_header(bytes: &[u8]) -> Result<Header, SaveError> {
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(SaveError::TooLarge);
    }
    if bytes.len() < HEADER_BYTES + CRC_BYTES {
        return Err(SaveError::Truncated);
    }
    if bytes[0..4] != SAVE_MAGIC {
        return Err(SaveError::BadMagic);
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if !(MIN_SAVE_VERSION..=SAVE_VERSION).contains(&version) {
        return Err(SaveError::UnsupportedVersion {
            got: version,
            min: MIN_SAVE_VERSION,
            current: SAVE_VERSION,
        });
    }
    let mut revision_bytes = [0u8; 8];
    revision_bytes.copy_from_slice(&bytes[8..16]);
    let revision = u64::from_le_bytes(revision_bytes);
    let count = usize::try_from(u32::from_le_bytes([
        bytes[16], bytes[17], bytes[18], bytes[19],
    ]))
    .unwrap_or(usize::MAX);
    if count > MAX_SAVE_NODES {
        return Err(SaveError::TooLarge);
    }
    // Version 1 seals right after the nodes; version 2 appends the material
    // section. Exact match in both cases: short bodies are torn writes and
    // trailing bytes are not ours. The checksum runs after the length gate
    // so torn writes report truncation, not corruption.
    let nodes_end = HEADER_BYTES
        .saturating_add(count.saturating_mul(NODE_BYTES))
        .saturating_add(CRC_BYTES);
    if version == 1 {
        if bytes.len() != nodes_end {
            return Err(SaveError::Truncated);
        }
        let body = &bytes[..bytes.len() - CRC_BYTES];
        let mut checksum_bytes = [0u8; 4];
        checksum_bytes.copy_from_slice(&bytes[bytes.len() - CRC_BYTES..]);
        if u32::from_le_bytes(checksum_bytes) != crc32(body) {
            return Err(SaveError::CrcMismatch);
        }
    } else if bytes.len() < nodes_end {
        return Err(SaveError::Truncated);
    }
    Ok(Header {
        version,
        revision,
        count,
    })
}

/// Parse `count` nodes starting at the cursor. Version 1 bytes are
/// checksum-clean past this point; version 2 seals its checksum after the
/// material section, so every slice here stays bounds-checked: a corrupt
/// length field fails as Truncated, never as a panic.
fn read_nodes(bytes: &[u8], pos: &mut usize, count: usize) -> Result<Vec<Node>, SaveError> {
    let mut nodes: Vec<Node> = Vec::with_capacity(count);
    for _ in 0..count {
        let id = parse_id(take(bytes, pos, 36)?)?;
        let parent = parse_id(take(bytes, pos, 36)?)?;
        let mut pose = [0f32; 10];
        for slot in pose.iter_mut() {
            let chunk = take(bytes, pos, 4)?;
            *slot = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        nodes.push((
            id,
            parent,
            Transform::new(
                [pose[0], pose[1], pose[2]],
                [pose[3], pose[4], pose[5], pose[6]],
                [pose[7], pose[8], pose[9]],
            ),
        ));
    }
    Ok(nodes)
}

/// Parse the v2 material section: count plus id/name entries, then the sealed
/// end and the checksum. Every slice stays bounds-checked; a corrupt section
/// fails bounded and names its node, never as a panic.
fn read_materials(
    bytes: &[u8],
    pos: &mut usize,
) -> Result<Vec<(ObjectId, MaterialName)>, SaveError> {
    let mut materials: Vec<(ObjectId, MaterialName)> = Vec::new();
    let raw_count = take(bytes, pos, 4)?;
    let mat_count = usize::try_from(u32::from_le_bytes([
        raw_count[0],
        raw_count[1],
        raw_count[2],
        raw_count[3],
    ]))
    .unwrap_or(usize::MAX);
    if mat_count > MAX_SAVE_NODES {
        return Err(SaveError::TooLarge);
    }
    for _ in 0..mat_count {
        let id = parse_id(take(bytes, pos, 36)?)?;
        if id.is_nil() {
            return Err(SaveError::BadMaterial { id });
        }
        let raw_len = take(bytes, pos, 2)?;
        let name_len = usize::from(u16::from_le_bytes([raw_len[0], raw_len[1]]));
        if name_len == 0 || name_len > MAX_MATERIAL_NAME_LEN {
            return Err(SaveError::BadMaterial { id });
        }
        let raw_name = take(bytes, pos, name_len)?;
        let text = core::str::from_utf8(raw_name).map_err(|_| SaveError::BadMaterial { id })?;
        let name = MaterialName::try_from(text).map_err(|_| SaveError::BadMaterial { id })?;
        if materials.iter().any(|(other, _)| other == &id) {
            return Err(SaveError::Duplicate { id });
        }
        materials.push((id, name));
    }
    Ok(materials)
}

/// Parse the v3 component section: count plus id/length/bytes entries. Each
/// entry must parse as a valid [`Component`]; a bad entry names its node.
fn read_components(bytes: &[u8], pos: &mut usize) -> Result<Vec<(ObjectId, Component)>, SaveError> {
    let mut components: Vec<(ObjectId, Component)> = Vec::new();
    let raw_count = take(bytes, pos, 4)?;
    let count = usize::try_from(u32::from_le_bytes([
        raw_count[0],
        raw_count[1],
        raw_count[2],
        raw_count[3],
    ]))
    .unwrap_or(usize::MAX);
    if count > MAX_SAVE_NODES * 8 {
        return Err(SaveError::TooLarge);
    }
    for _ in 0..count {
        let id = parse_id(take(bytes, pos, 36)?)?;
        if id.is_nil() {
            return Err(SaveError::BadComponent { id });
        }
        let raw_len = take(bytes, pos, 2)?;
        let len = usize::from(u16::from_le_bytes([raw_len[0], raw_len[1]]));
        let raw = take(bytes, pos, len)?;
        let component = Component::from_bytes(raw).map_err(|_| SaveError::BadComponent { id })?;
        if components
            .iter()
            .any(|(other, kept)| *other == id && kept.kind() == component.kind())
        {
            return Err(SaveError::BadComponent { id });
        }
        components.push((id, component));
    }
    Ok(components)
}

/// Parse the v4 graph section: length plus canonical graph bytes. Old saves
/// carry no section and open with an empty graph; bad bytes fail as BadGraph.
fn read_graph(bytes: &[u8], pos: &mut usize) -> Result<crate::graph::Graph, SaveError> {
    let raw_len = take(bytes, pos, 4)?;
    let len = usize::try_from(u32::from_le_bytes([
        raw_len[0], raw_len[1], raw_len[2], raw_len[3],
    ]))
    .unwrap_or(usize::MAX);
    if len > crate::graph::MAX_GRAPH_BYTES {
        return Err(SaveError::TooLarge);
    }
    let raw = take(bytes, pos, len)?;
    crate::graph::Graph::decode(raw).map_err(|_| SaveError::BadGraph)
}

/// Canonicalize a decoded graph to the document default when empty.
///
/// `Document::new()` carries `Graph::default()` (`next` 0) while
/// `Graph::new()` and `Graph::decode` of empty bytes carry `next` 1; both
/// allocate id 1 first and behave identically, but `PartialEq` includes the
/// counter so a naive roundtrip fails. The codec normalizes empty graphs to
/// the document default so `load(save(doc)) == doc`.
fn canonical_graph(graph: crate::graph::Graph) -> crate::graph::Graph {
    if graph.node_count() == 0 && graph.wire_count() == 0 {
        crate::graph::Graph::default()
    } else {
        graph
    }
}

/// Check the sealed end and the checksum of a v2 or v3 save: the sections
/// must end exactly at the checksum, then the checksum decides corruption.
fn seal_check(bytes: &[u8], pos: usize) -> Result<(), SaveError> {
    if pos != bytes.len() - CRC_BYTES {
        return Err(SaveError::Truncated);
    }
    let body = &bytes[..bytes.len() - CRC_BYTES];
    let mut checksum_bytes = [0u8; 4];
    checksum_bytes.copy_from_slice(&bytes[bytes.len() - CRC_BYTES..]);
    if u32::from_le_bytes(checksum_bytes) != crc32(body) {
        return Err(SaveError::CrcMismatch);
    }
    Ok(())
}

/// Structural checks mirror the scene codec so diagnostics agree: the
/// reported culprit is deterministic for a given set of entries. Sorts
/// `nodes` into canonical id order and returns the bindings in id order.
fn check_structure(
    nodes: &mut [Node],
    materials: &[(ObjectId, MaterialName)],
    components: &[(ObjectId, Component)],
) -> Result<Vec<(ObjectId, MaterialName)>, SaveError> {
    nodes.sort_by_key(|(id, _, _)| id.to_string());
    let mut previous: Option<ObjectId> = None;
    for (id, _, local) in nodes.iter() {
        if id.is_nil() {
            return Err(SaveError::NilId);
        }
        // Sorted ids put repeats next to each other, so the first repeat in
        // canonical order is the named culprit.
        if Some(*id) == previous {
            return Err(SaveError::Duplicate { id: *id });
        }
        previous = Some(*id);
        if !local.is_finite() {
            return Err(SaveError::NonFinite { id: *id });
        }
    }
    for (id, parent, _) in nodes.iter() {
        if !parent.is_nil() && !nodes.iter().any(|(other, _, _)| other == parent) {
            return Err(SaveError::MissingParent {
                node: *id,
                parent: *parent,
            });
        }
    }
    // Bindings may only name nodes in the same save (sorted ids keep the
    // culprit deterministic).
    let mut bound = materials.to_vec();
    bound.sort_by_key(|(id, _)| id.to_string());
    for (id, _) in &bound {
        if !nodes.iter().any(|(other, _, _)| other == id) {
            return Err(SaveError::BadMaterial { id: *id });
        }
    }
    for (id, _) in components {
        if !nodes.iter().any(|(other, _, _)| other == id) {
            return Err(SaveError::BadComponent { id: *id });
        }
    }
    Ok(bound)
}

/// Insert parents before children. No progress with nodes left means the
/// remaining links loop, since every named parent is known to exist.
fn build_hierarchy(nodes: &[Node]) -> Result<engine_scene::transform::Hierarchy, SaveError> {
    let mut tree = engine_scene::transform::Hierarchy::new();
    let mut done = 0usize;
    while done < nodes.len() {
        let mut progressed = false;
        for (id, parent, local) in nodes {
            if tree.contains(*id) {
                continue;
            }
            if parent.is_nil() || tree.contains(*parent) {
                match tree.insert(*id, *parent, *local) {
                    Ok(()) => {
                        done += 1;
                        progressed = true;
                    }
                    Err(engine_scene::transform::HierarchyError::NonFinite) => {
                        return Err(SaveError::NonFinite { id: *id });
                    }
                    Err(_) => return Err(SaveError::Cycle { node: *id }),
                }
            }
        }
        if !progressed {
            let stuck = nodes
                .iter()
                .find(|(id, _, _)| !tree.contains(*id))
                .expect("loop exits only with nodes left");
            return Err(SaveError::Cycle { node: stuck.0 });
        }
    }
    tree.validate().map_err(|_| SaveError::Truncated)?;
    Ok(tree)
}

/// Take `count` bytes, advancing the cursor. Short reads fail bounded.
fn take<'a>(bytes: &'a [u8], pos: &mut usize, count: usize) -> Result<&'a [u8], SaveError> {
    let end = pos.saturating_add(count);
    if end > bytes.len() {
        return Err(SaveError::Truncated);
    }
    let slice = &bytes[*pos..end];
    *pos = end;
    Ok(slice)
}

/// Parse a 36-byte hyphenated id. Unparseable text fails bounded.
fn parse_id(chunk: &[u8]) -> Result<ObjectId, SaveError> {
    let text = core::str::from_utf8(chunk).map_err(|_| SaveError::Truncated)?;
    ObjectId::parse(text).map_err(|_| SaveError::Truncated)
}

/// Push a hyphenated id (always 36 bytes).
/// Crate-visible: the durable journal encodes its ops with the same ids.
pub(crate) fn push_id(out: &mut Vec<u8>, id: ObjectId) {
    out.extend_from_slice(id.to_string().as_bytes());
}

/// Push a pose as ten little-endian f32 values.
/// Crate-visible: the durable journal encodes its ops with the same poses.
pub(crate) fn push_transform(out: &mut Vec<u8>, local: Transform) {
    for value in [
        local.position[0],
        local.position[1],
        local.position[2],
        local.rotation[0],
        local.rotation[1],
        local.rotation[2],
        local.rotation[3],
        local.scale[0],
        local.scale[1],
        local.scale[2],
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{apply, Batch, Journal, Op};
    use engine_scene::transform::Transform;

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    /// Root plus child plus grandchild over three applied batches.
    fn sample_doc() -> Document {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        let grand = ObjectId::new();
        for (i, (id, parent)) in [(root, ObjectId::NIL), (child, root), (grand, child)]
            .into_iter()
            .enumerate()
        {
            let rev = doc.revision;
            apply(
                &mut doc,
                &mut journal,
                &Batch {
                    ops: vec![Op::Insert {
                        id,
                        parent,
                        local: pose_at(i as f32),
                    }],
                    base_revision: rev,
                    idempotency_key: 100 + i as u64,
                    provenance: None,
                },
            )
            .unwrap();
        }
        doc
    }

    #[test]
    fn save_reopen_roundtrip_preserves_ids_and_revisions() {
        let doc = sample_doc();
        let first = save_to_bytes(&doc);
        // Same document saves to the same bytes twice (sorted canonical order).
        assert_eq!(first, save_to_bytes(&doc));

        let back = load_from_bytes(&first).unwrap();
        assert_eq!(back, doc);
        assert_eq!(back.revision, 3);
        for id in doc.hierarchy.propagation_order() {
            assert_eq!(back.hierarchy.parent(id), doc.hierarchy.parent(id));
            assert_eq!(back.hierarchy.local(id), doc.hierarchy.local(id));
            assert_eq!(back.hierarchy.world(id), doc.hierarchy.world(id));
        }

        // Re-saving the reopened document is byte-identical (stable references).
        assert_eq!(save_to_bytes(&back), first);

        // Empty documents roundtrip with their revision intact.
        let mut empty = Document::new();
        empty.revision = 41;
        let back = load_from_bytes(&save_to_bytes(&empty)).unwrap();
        assert_eq!(back, empty);
    }

    /// Bind one material through the command batch, then roundtrip.
    fn bound_doc() -> (Document, ObjectId) {
        let mut doc = sample_doc();
        let mut journal = Journal::new();
        let target = doc.hierarchy.propagation_order()[0];
        let rev = doc.revision;
        apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::SetMaterial {
                    id: target,
                    material: Some(MaterialName::try_from("hero-mat").unwrap()),
                }],
                base_revision: rev,
                idempotency_key: 900,
                provenance: None,
            },
        )
        .unwrap();
        (doc, target)
    }

    #[test]
    fn material_binding_persists_in_scene_bytes() {
        let (doc, target) = bound_doc();
        let first = save_to_bytes(&doc);
        // The binding is in the scene bytes, not a sidecar: the asset name
        // is literally present in the file.
        let name = b"hero-mat";
        assert!(
            first.windows(name.len()).any(|w| w == name),
            "scene bytes carry the bound material name"
        );

        let back = load_from_bytes(&first).unwrap();
        assert_eq!(
            back.materials
                .get(&target.to_string())
                .map(MaterialName::as_str),
            Some("hero-mat")
        );
        assert_eq!(back, doc);

        // Re-saving the reopened document is byte-identical (stable refs).
        assert_eq!(save_to_bytes(&back), first);
    }

    #[test]
    fn version_one_files_open_with_empty_bindings() {
        let doc = sample_doc();
        let plain = save_to_bytes(&doc);
        let count = 3usize;
        // Strip the v2 material section back to the v1 seal and stamp v1.
        let v1_len = HEADER_BYTES + count * NODE_BYTES + CRC_BYTES;
        let mut v1 = plain[..v1_len].to_vec();
        v1[4..8].copy_from_slice(&1u32.to_le_bytes());
        reseal(&mut v1);
        let back = load_from_bytes(&v1).unwrap();
        assert!(back.materials.is_empty());
        assert_eq!(back.hierarchy, doc.hierarchy);
        assert_eq!(back.revision, doc.revision);
    }

    #[test]
    fn corrupt_material_section_names_its_node() {
        let (doc, target) = bound_doc();
        let good = save_to_bytes(&doc);
        // Material section layout: nodes, mat count (4), then id (36) plus
        // name length (2) plus name bytes for the single entry.
        let entry = HEADER_BYTES + 3 * NODE_BYTES + 4;

        // Zero name length resealed still fails, naming the node.
        let mut zeroed = good.clone();
        zeroed[entry + 36..entry + 38].copy_from_slice(&0u16.to_le_bytes());
        reseal(&mut zeroed);
        assert_eq!(
            load_from_bytes(&zeroed),
            Err(SaveError::BadMaterial { id: target })
        );
        let report = load_from_bytes(&zeroed).unwrap_err().to_string();
        assert!(report.contains(&target.to_string()));

        // A binding for a node the save does not contain fails the same way.
        let mut dangling = good.clone();
        let ghost = ObjectId::new();
        assert!(!doc.hierarchy.contains(ghost));
        dangling[entry..entry + 36].copy_from_slice(ghost.to_string().as_bytes());
        reseal(&mut dangling);
        assert_eq!(
            load_from_bytes(&dangling),
            Err(SaveError::BadMaterial { id: ghost })
        );
    }

    #[test]
    fn corrupt_truncated_and_future_versions_are_rejected() {
        let good = save_to_bytes(&sample_doc());

        assert_eq!(load_from_bytes(&[]), Err(SaveError::Truncated));
        assert_eq!(load_from_bytes(&good[..10]), Err(SaveError::Truncated));

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert_eq!(load_from_bytes(&bad_magic), Err(SaveError::BadMagic));

        // Trailing bytes are not ours: exact-length match only.
        let mut trailed = good.clone();
        trailed.push(0);
        assert_eq!(load_from_bytes(&trailed), Err(SaveError::Truncated));

        // Torn body: declared count no longer matches the bytes.
        assert_eq!(
            load_from_bytes(&good[..good.len() - 4]),
            Err(SaveError::Truncated)
        );

        // Any flipped content byte breaks the checksum: flip the low byte of
        // the first pose lane (a small value stays finite, so the checksum,
        // not the structure gate, answers).
        let mut tampered = good.clone();
        let pose_byte = HEADER_BYTES + 72;
        tampered[pose_byte] ^= 0xFF;
        assert_eq!(load_from_bytes(&tampered), Err(SaveError::CrcMismatch));

        // Past and future versions name what was found and what opens.
        for bad in [0u32, SAVE_VERSION + 1, u32::MAX] {
            let mut versioned = good.clone();
            versioned[4..8].copy_from_slice(&bad.to_le_bytes());
            // Re-seal so the version gate (not the checksum) answers.
            let checksum = crc32(&versioned[..versioned.len() - CRC_BYTES]);
            let tail = versioned.len() - CRC_BYTES;
            versioned[tail..].copy_from_slice(&checksum.to_le_bytes());
            assert_eq!(
                load_from_bytes(&versioned),
                Err(SaveError::UnsupportedVersion {
                    got: bad,
                    min: MIN_SAVE_VERSION,
                    current: SAVE_VERSION,
                })
            );
            let report = load_from_bytes(&versioned).unwrap_err().to_string();
            assert!(report.contains(&bad.to_string()));
            assert!(report.contains(&SAVE_VERSION.to_string()));
        }

        // Oversize input fails before any parsing.
        assert_eq!(
            load_from_bytes(&vec![0u8; MAX_SAVE_BYTES + 1]),
            Err(SaveError::TooLarge)
        );
        let mut huge_count = good.clone();
        huge_count[16..20].copy_from_slice(&((MAX_SAVE_NODES + 1) as u32).to_le_bytes());
        let checksum = crc32(&huge_count[..huge_count.len() - CRC_BYTES]);
        let tail = huge_count.len() - CRC_BYTES;
        huge_count[tail..].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(load_from_bytes(&huge_count), Err(SaveError::TooLarge));
    }

    /// Rewrite the trailing CRC after patching node bytes in tests.
    fn reseal(bytes: &mut [u8]) {
        let checksum = crc32(&bytes[..bytes.len() - CRC_BYTES]);
        let tail = bytes.len() - CRC_BYTES;
        bytes[tail..].copy_from_slice(&checksum.to_le_bytes());
    }

    #[test]
    fn missing_reference_names_culprits_and_duplicates_and_cycles_fail() {
        let doc = sample_doc();
        let ids = doc.hierarchy.propagation_order();
        let mut sorted = ids.clone();
        sorted.sort_by_key(|id| id.to_string());
        let child = sorted[1];

        // Dangling parent: point the second node at an id no entry provides.
        let mut orphaned = save_to_bytes(&doc);
        let ghost = ObjectId::new();
        assert!(!doc.hierarchy.contains(ghost));
        let parent_at = HEADER_BYTES + NODE_BYTES + 36;
        orphaned[parent_at..parent_at + 36].copy_from_slice(ghost.to_string().as_bytes());
        reseal(&mut orphaned);
        assert_eq!(
            load_from_bytes(&orphaned),
            Err(SaveError::MissingParent {
                node: child,
                parent: ghost,
            })
        );
        let report = load_from_bytes(&orphaned).unwrap_err().to_string();
        assert!(report.contains(&child.to_string()));
        assert!(report.contains(&ghost.to_string()));

        // Doubled id: stamp the second node with the first node's id.
        let mut doubled = save_to_bytes(&doc);
        doubled[HEADER_BYTES + NODE_BYTES..HEADER_BYTES + NODE_BYTES + 36]
            .copy_from_slice(sorted[0].to_string().as_bytes());
        reseal(&mut doubled);
        assert_eq!(
            load_from_bytes(&doubled),
            Err(SaveError::Duplicate { id: sorted[0] })
        );

        // Two-node loop: parent each of the first two nodes under the other.
        let mut looping = save_to_bytes(&doc);
        looping[HEADER_BYTES + 36..HEADER_BYTES + 72]
            .copy_from_slice(sorted[1].to_string().as_bytes());
        looping[HEADER_BYTES + NODE_BYTES + 36..HEADER_BYTES + NODE_BYTES + 72]
            .copy_from_slice(sorted[0].to_string().as_bytes());
        reseal(&mut looping);
        assert!(matches!(
            load_from_bytes(&looping),
            Err(SaveError::Cycle { .. })
        ));
    }

    #[test]
    fn byte_mutation_sweep_never_panics_and_never_sneaks_through() {
        let good = save_to_bytes(&sample_doc());
        // Every single-byte flip must fail bounded: the exact-length gate,
        // the version gate and the CRC leave no mutation undetected, and no
        // input path panics.
        for pos in 0..good.len() {
            for flip in [0x00u8, 0xFFu8] {
                let mut mutated = good.clone();
                mutated[pos] ^= flip;
                if mutated[pos] == good[pos] {
                    continue;
                }
                assert!(
                    load_from_bytes(&mutated).is_err(),
                    "mutated byte {pos} opened as valid"
                );
            }
        }
    }
}
