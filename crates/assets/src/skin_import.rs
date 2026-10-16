//! skin_import: glTF skins, skeletons and clips into the anim contract.
//!
//! Consumer: `engine-render` (GPU skinning matrices) and `engine-arena`
//! (tick-root intent) consume [`Skeleton`], [`ImportedClip`],
//! [`RootTable`] and [`ContactTable`]; the cooker/registry owns identity.
//! Data owner: `engine-assets`; `engine-anim` owns the contract types and
//! never imports assets back (see the anim direction rule in `ARCHITECTURE.md`).
//! Units: meters, seconds, radians in; the same out. Ticks cook at 60 Hz.
//!
//! Pipeline: [`import_skeleton`] reads the node hierarchy parent-first with
//! names sanitized into the anim charset; [`import_skin_primitive`] reads
//! `JOINTS_0`/`WEIGHTS_0` against the skin's joint list; [`import_clip`]
//! reads translation/rotation channels with unanimated joints filled from the
//! node bind pose; [`cook_root_table`]/[`cook_contact_table`] sample one
//! clip into validated tick tables via the anim validators. The `*_doc`
//! variants validate synthetic [`SkeletonDoc`]/[`SkinDoc`]/[`ClipDoc`] input
//! with the same rules so tests never need donor bytes.
//!
//! Documented fallbacks (never guesses): unnamed nodes become
//! `joint_{index}`; duplicate sanitized names gain `_{n}`; unanimated joints
//! use the node bind translation/rotation; single-key tracks hold constant.
//! Rejected instead: sparse accessors, scale/morph-target channels,
//! non-`LINEAR` interpolation, joint indices past the skeleton, rows with
//! more than [`MAX_INFLUENCES`] entries, non-finite data, negative weights,
//! weight rows whose sum differs from `1.0` by more than
//! [`WEIGHT_SUM_EPS`], non-unit quaternions and unsorted key times. Inverse
//! bind matrices are not consumed at import (render composes them with
//! `skinning_matrices`); bind scale is ignored. Every failure is a typed
//! [`SkinImportError`], never a panic.
//!
//! Fixture provenance: mesh-path fixtures are the Khronos CC0 glTF samples
//! under `content/incoming/`; every skinned/clip fixture in the test module
//! is synthetic JSON authored in code (no donor bytes, no hero O7 content).

use super::mesh_import::{bound_detail, required_extension_name, MAX_BYTES, MAX_PRIMITIVES};
use super::registry::content_hash;
use super::wire::bound_chars;
use engine_anim::clip::TICK_DT;
use engine_anim::contract::{
    validate_contacts, validate_skeleton, AnimError, Contact, ContactTable, Joint, RootTable,
    Skeleton, MAX_JOINTS, NAME_MAX_LEN,
};
use engine_anim::pose::{extract_root, QuatTrack};
use engine_anim::{Clip, Track};
use std::collections::BTreeSet;
use std::fmt;

/// Max joint influences per vertex row; longer rows are rejected, never cut.
pub const MAX_INFLUENCES: usize = 4;
/// Max keyframes per clip track; bounds total cook work.
pub const MAX_TRACK_KEYS: usize = 65_536;
/// Max animations cooked per file; bounds total work.
pub const MAX_ANIMATIONS: usize = 64;
/// Max ticks cooked per table (60 s at 60 Hz); longer clips are rejected.
pub const MAX_COOK_TICKS: u32 = 3_600;
/// Max clip-name chars carried; longer names are truncated, never rejected.
pub const MAX_CLIP_NAME_CHARS: usize = 128;
/// Weight rows must sum to `1.0` within this tolerance or they are rejected.
pub const WEIGHT_SUM_EPS: f32 = 1e-3;
/// Quaternions must be unit length within this tolerance (mirrors the anim contract).
pub const QUAT_NORM_EPS: f32 = 1e-3;

/// Why a skeleton/skin/clip import or cook failed; every malformed input
/// maps to one variant with bounded detail.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinImportError {
    /// Container or JSON parse/validation failed; detail bounded.
    Parse {
        /// Truncated parser message.
        detail: String,
    },
    /// File requires a glTF extension this importer does not interpret.
    UnsupportedExtension {
        /// Truncated extension name.
        name: String,
    },
    /// A buffer reference has no resolvable bytes in a slice import.
    MissingBuffer {
        /// Truncated detail naming the missing reference.
        detail: String,
    },
    /// The document has no nodes to build a skeleton from.
    NoSkeleton,
    /// A joint name or parent link failed anim-contract validation; joint
    /// index carried.
    BadJoint {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// A cook or reference names a joint past the skeleton; index carried.
    UnknownJoint {
        /// Offending joint index.
        joint: usize,
    },
    /// A skin row addresses a joint past the skeleton.
    JointOutOfRange {
        /// Vertex row index.
        vertex: usize,
        /// Offending joint index.
        joint: usize,
    },
    /// A skin row carries more than [`MAX_INFLUENCES`] entries.
    TooManyInfluences {
        /// Vertex row index.
        vertex: usize,
        /// Offending entry count.
        count: usize,
    },
    /// A skin weight is negative; weights are non-negative parts of one.
    NegativeWeight {
        /// Vertex row index.
        vertex: usize,
    },
    /// A skin weight row does not sum to `1.0` within [`WEIGHT_SUM_EPS`].
    BadWeightSum {
        /// Vertex row index.
        vertex: usize,
    },
    /// A skin row's joint/weight lengths disagree.
    SkinRowMismatch {
        /// Vertex row index.
        vertex: usize,
    },
    /// Skin row counts disagree with the vertex count.
    SkinLengthMismatch {
        /// Expected row count (the vertex count).
        expected: usize,
        /// Fewer rows actually present.
        got: usize,
    },
    /// One side of the `JOINTS_0`/`WEIGHTS_0` pair is missing.
    IncompleteSkin {
        /// Mesh index in document order.
        mesh: usize,
        /// Primitive index within the mesh.
        primitive: usize,
    },
    /// Skin attributes exist but no skin serves the mesh.
    NoSkin {
        /// Mesh index in document order.
        mesh: usize,
        /// Primitive index within the mesh.
        primitive: usize,
    },
    /// A sparse accessor needs de-sparsifying this importer does not do.
    SparseUnsupported {
        /// `joints`, `weights`, `clip.times` or `clip.values`.
        field: &'static str,
    },
    /// A key track has mismatched, unsorted or missing data.
    BadClip {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// A key track is empty.
    EmptyTrack {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// Clip track counts differ from the skeleton joint count.
    ClipLengthMismatch {
        /// Expected track count (the joint count).
        expected: usize,
        /// Tracks actually present.
        got: usize,
    },
    /// A rotation key is not unit length.
    BadQuat {
        /// Joint index in skeleton order.
        joint: usize,
        /// Key index within the track.
        key: usize,
    },
    /// A channel targets scale or morph weights, which this cook refuses to
    /// guess at; detail names the target.
    UnsupportedTarget {
        /// `scale` or `weights`.
        detail: String,
    },
    /// A channel uses non-`LINEAR` interpolation; the name is carried.
    UnsupportedInterpolation {
        /// `STEP` or `CUBICSPLINE`.
        name: String,
    },
    /// Two channels drive the same joint property; joint index carried.
    DuplicateTrack {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// A cooked root table failed [`validate_root`]; delta index carried.
    BadRoot {
        /// Delta index.
        index: usize,
    },
    /// A cooked contact table failed [`validate_contacts`]; index carried.
    BadContact {
        /// Sample index.
        index: usize,
    },
    /// An input exceeds a budget (bytes, joints, keys, ticks, animations).
    TooLarge {
        /// Offending count.
        count: usize,
    },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
}

impl fmt::Display for SkinImportError {
    /// Diagnostics stay bounded: carried strings render truncated to
    /// [`MAX_DIAGNOSTIC_CHARS`](super::mesh_import::MAX_DIAGNOSTIC_CHARS) chars
    /// no matter how they were built.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { detail } => {
                write!(f, "skin import failed to parse: {}", bound_detail(detail))
            }
            Self::UnsupportedExtension { name } => {
                write!(
                    f,
                    "skin import needs unsupported extension {}",
                    bound_detail(name)
                )
            }
            Self::MissingBuffer { detail } => {
                write!(f, "skin import missing buffer: {}", bound_detail(detail))
            }
            Self::NoSkeleton => write!(f, "skin import found no skeleton nodes"),
            Self::BadJoint { joint } => write!(f, "skin import bad joint {joint}"),
            Self::UnknownJoint { joint } => write!(f, "skin import unknown joint {joint}"),
            Self::JointOutOfRange { vertex, joint } => {
                write!(f, "skin import vertex {vertex} addresses joint {joint}")
            }
            Self::TooManyInfluences { vertex, count } => {
                write!(f, "skin import vertex {vertex} has {count} influences")
            }
            Self::NegativeWeight { vertex } => {
                write!(f, "skin import vertex {vertex} has a negative weight")
            }
            Self::BadWeightSum { vertex } => {
                write!(f, "skin import vertex {vertex} weights do not sum to one")
            }
            Self::SkinRowMismatch { vertex } => {
                write!(f, "skin import vertex {vertex} joints/weights disagree")
            }
            Self::SkinLengthMismatch { expected, got } => {
                write!(f, "skin import wants {expected} rows, found {got}")
            }
            Self::IncompleteSkin { mesh, primitive } => {
                write!(
                    f,
                    "skin import mesh {mesh} primitive {primitive} is half-skinned"
                )
            }
            Self::NoSkin { mesh, primitive } => {
                write!(
                    f,
                    "skin import mesh {mesh} primitive {primitive} names no skin"
                )
            }
            Self::SparseUnsupported { field } => {
                write!(f, "skin import sparse accessor unsupported: {field}")
            }
            Self::BadClip { joint } => write!(f, "skin import bad clip track {joint}"),
            Self::EmptyTrack { joint } => write!(f, "skin import empty clip track {joint}"),
            Self::ClipLengthMismatch { expected, got } => {
                write!(f, "skin import wants {expected} tracks, found {got}")
            }
            Self::BadQuat { joint, key } => {
                write!(f, "skin import joint {joint} key {key} is not a unit quat")
            }
            Self::UnsupportedTarget { detail } => {
                write!(
                    f,
                    "skin import refuses channel target {}",
                    bound_detail(detail)
                )
            }
            Self::UnsupportedInterpolation { name } => {
                write!(
                    f,
                    "skin import refuses {} interpolation",
                    bound_detail(name)
                )
            }
            Self::DuplicateTrack { joint } => {
                write!(f, "skin import joint {joint} driven twice")
            }
            Self::BadRoot { index } => write!(f, "skin import bad root delta {index}"),
            Self::BadContact { index } => write!(f, "skin import bad contact {index}"),
            Self::TooLarge { count } => write!(f, "skin import too large: {count}"),
            Self::NonFinite(field) => write!(f, "skin import non-finite {field}"),
        }
    }
}

impl std::error::Error for SkinImportError {}

/// One synthetic skeleton joint: a strict name plus its parent index.
#[derive(Clone, Debug, PartialEq)]
pub struct JointDoc {
    /// Joint name; validated strictly against the anim charset (no sanitize).
    pub name: String,
    /// Parent joint index (`None` = root); must be parent-first.
    pub parent: Option<usize>,
}

/// Synthetic skeleton input: strict names/parents validated as-is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkeletonDoc {
    /// Joints in parent-first order.
    pub joints: Vec<JointDoc>,
}

/// Synthetic skin input: variable-length per-vertex rows so the
/// [`MAX_INFLUENCES`] bound is testable; glTF adapters pack `vec4` attrs here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinDoc {
    /// Vertex count; both row lists must match it.
    pub vertex_count: usize,
    /// Joint indices per vertex, at most [`MAX_INFLUENCES`] entries.
    pub joints: Vec<Vec<u16>>,
    /// Weights per vertex, same length as the joint row, summing to one.
    pub weights: Vec<Vec<f32>>,
}

/// Validated skin: packed influences, one fixed row per vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skin {
    /// Joint indices per vertex, packed from the validated rows.
    pub joints: Vec<[u16; 4]>,
    /// Weights per vertex, packed from the validated rows (each sums to one).
    pub weights: Vec<[f32; 4]>,
}

/// One translation key: a time plus a local offset in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranslationKey {
    /// Key time in seconds.
    pub time: f32,
    /// Local offset in meters.
    pub value: [f32; 3],
}

/// One rotation key: a time plus a unit quaternion `[x, y, z, w]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotationKey {
    /// Key time in seconds.
    pub time: f32,
    /// Unit quaternion `[x, y, z, w]`.
    pub value: [f32; 4],
}

/// Synthetic clip input: per-joint key lists validated into anim tracks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipDoc {
    /// Clip name; carried truncated to [`MAX_CLIP_NAME_CHARS`].
    pub name: String,
    /// Translation keys per joint; one list per skeleton joint.
    pub translations: Vec<Vec<TranslationKey>>,
    /// Rotation keys per joint; one list per skeleton joint.
    pub rotations: Vec<Vec<RotationKey>>,
}

/// Validated clip: anim position tracks plus rotation tracks plus duration.
#[derive(Clone, Debug, Default)]
pub struct ImportedClip {
    /// Clip name (truncated on import).
    pub name: String,
    /// Duration in seconds: the latest key time across all tracks.
    pub duration: f32,
    /// Position tracks, one per joint in skeleton order.
    pub positions: Clip,
    /// Rotation tracks, one per joint in skeleton order.
    pub rotations: Vec<QuatTrack>,
}

impl PartialEq for ImportedClip {
    /// Field-wise equality: the anim track types predate `PartialEq`, so
    /// the comparison spells out times plus values per joint here.
    fn eq(&self, other: &Self) -> bool {
        if self.name != other.name || self.duration != other.duration {
            return false;
        }
        if self.positions.tracks.len() != other.positions.tracks.len()
            || self.rotations.len() != other.rotations.len()
        {
            return false;
        }
        for (a, b) in self
            .positions
            .tracks
            .iter()
            .zip(other.positions.tracks.iter())
        {
            if a.times != b.times || a.values != b.values {
                return false;
            }
        }
        for (a, b) in self.rotations.iter().zip(other.rotations.iter()) {
            if a.times != b.times || a.values != b.values {
                return false;
            }
        }
        true
    }
}

/// One skinned primitive: its document indices plus the validated skin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinnedPrimitive {
    /// Mesh index in document order.
    pub mesh: usize,
    /// Primitive index within the mesh.
    pub primitive: usize,
    /// Validated joint/weight rows.
    pub skin: Skin,
}

/// One skinned-file import: skeleton plus per-primitive skins plus clips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinnedImport {
    /// Skeleton in parent-first order.
    pub skeleton: Skeleton,
    /// Skins for primitives carrying `JOINTS_0`/`WEIGHTS_0`, document order.
    pub skins: Vec<SkinnedPrimitive>,
    /// Clips in document order (empty when the file holds no animations).
    pub clips: Vec<ImportedClip>,
}

/// Cook configuration: which joints drive the authoritative tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CookConfig {
    /// Joint sampled for planar root motion.
    pub root_joint: usize,
    /// Joint sampled for left-foot contact (`foot == 0`).
    pub left_foot: usize,
    /// Joint sampled for right-foot contact (`foot == 1`).
    pub right_foot: usize,
    /// Foot height at or below which a foot counts as planted (meters).
    pub ground_y: f32,
    /// First tick stamp of the cooked tables.
    pub start_tick: u32,
}

/// Stable identity for skinned/clip source bytes: the registry content-hash
/// canon, so reimporting identical bytes keeps the same id.
pub fn skin_content_id(bytes: &[u8]) -> u64 {
    content_hash(bytes)
}

/// Map an anim-contract skeleton failure onto the joint it names; the remaining variants
/// cannot come out of [`validate_skeleton`] and map to joint zero.
fn map_skeleton_error(error: AnimError) -> SkinImportError {
    match error {
        AnimError::EmptySkeleton => SkinImportError::NoSkeleton,
        AnimError::TooManyJoints(count) => SkinImportError::TooLarge { count },
        AnimError::BadName(joint) | AnimError::BadParent(joint) => {
            SkinImportError::BadJoint { joint }
        }
        AnimError::LengthMismatch { .. }
        | AnimError::NonFinite(_)
        | AnimError::UnnormalizedQuat(_)
        | AnimError::BadRootOrder(_)
        | AnimError::BadFoot(_)
        | AnimError::BadContactOrder(_)
        | AnimError::UnknownRole(_)
        | AnimError::DuplicateBone(_)
        | AnimError::MissingRoles(_) => SkinImportError::BadJoint { joint: 0 },
    }
}

/// Import a synthetic skeleton doc: names and parents validated strictly
/// as-is through the anim validator (no sanitizing on this path).
pub fn import_skeleton_doc(doc: &SkeletonDoc) -> Result<Skeleton, SkinImportError> {
    if doc.joints.is_empty() {
        return Err(SkinImportError::NoSkeleton);
    }
    if doc.joints.len() > MAX_JOINTS {
        return Err(SkinImportError::TooLarge {
            count: doc.joints.len(),
        });
    }
    let skeleton = Skeleton {
        joints: doc
            .joints
            .iter()
            .map(|joint| Joint {
                name: joint.name.clone(),
                parent: joint.parent,
            })
            .collect(),
    };
    validate_skeleton(&skeleton).map_err(map_skeleton_error)?;
    Ok(skeleton)
}

/// Sanitize a glTF node name into the anim charset: lowercase, ASCII
/// alphanumerics plus `_` (anything else becomes `_`), a leading `_` when
/// the first character is a digit, `joint_{index}` when nothing survives,
/// truncated to [`NAME_MAX_LEN`]. Deterministic per input.
fn sanitize_joint_name(raw: &str, index: usize) -> String {
    let mut base = String::new();
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
            base.push(c);
        } else {
            base.push('_');
        }
    }
    if base.is_empty() {
        base = format!("joint_{index}");
    } else if base.as_bytes()[0].is_ascii_digit() {
        base.insert(0, '_');
    }
    if base.len() > NAME_MAX_LEN {
        base.truncate(NAME_MAX_LEN);
    }
    base
}

/// Deterministic de-duplication: first use wins, later collisions gain
/// `_{n}` (re-truncated to fit [`NAME_MAX_LEN`]).
fn dedup_joint_name(name: String, used: &mut BTreeSet<String>) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    let mut n: u32 = 2;
    loop {
        let suffix = format!("_{n}");
        let stem_len = NAME_MAX_LEN.saturating_sub(suffix.len());
        let mut candidate = name.clone();
        candidate.truncate(stem_len);
        candidate.push_str(&suffix);
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

/// Parent node index per node (`None` = root), from the children links.
/// A child claimed twice keeps its first claimant; validation settles order.
fn node_parents(doc: &gltf::Document) -> Vec<Option<usize>> {
    let count = doc.nodes().len();
    let mut parent: Vec<Option<usize>> = vec![None; count];
    for node in doc.nodes() {
        for child in node.children() {
            if let Some(slot) = parent.get_mut(child.index()) {
                if slot.is_none() {
                    *slot = Some(node.index());
                }
            }
        }
    }
    parent
}

/// Node indices in parent-first order: roots in index order, then breadth
/// first. Cycles cannot loop forever (visited set); leftover nodes append in
/// index order and fail anim parent validation instead of hanging.
pub(crate) fn skeleton_order(doc: &gltf::Document) -> Vec<usize> {
    let count = doc.nodes().len();
    let parent = node_parents(doc);
    let mut seen = vec![false; count];
    let mut order = Vec::with_capacity(count);
    let mut queue: Vec<usize> = (0..count).filter(|i| parent[*i].is_none()).collect();
    let mut head = 0;
    while head < queue.len() {
        let current = queue[head];
        head += 1;
        if seen[current] {
            continue;
        }
        seen[current] = true;
        order.push(current);
        if let Some(node) = doc.nodes().nth(current) {
            for child in node.children() {
                if !seen[child.index()] {
                    queue.push(child.index());
                }
            }
        }
    }
    for (index, was_seen) in seen.iter().enumerate() {
        if !was_seen {
            order.push(index);
        }
    }
    order
}

/// Import a skeleton from a glTF document: every node becomes a joint in
/// parent-first order with sanitized unique names, validated through the anim
/// contract.
pub fn import_skeleton(doc: &gltf::Document) -> Result<Skeleton, SkinImportError> {
    let order = skeleton_order(doc);
    if order.is_empty() {
        return Err(SkinImportError::NoSkeleton);
    }
    if order.len() > MAX_JOINTS {
        return Err(SkinImportError::TooLarge { count: order.len() });
    }
    let parent_of = node_parents(doc);
    let mut position_of: Vec<Option<usize>> = vec![None; doc.nodes().len()];
    for (position, node) in order.iter().enumerate() {
        position_of[*node] = Some(position);
    }
    let mut used = BTreeSet::new();
    let mut joints = Vec::with_capacity(order.len());
    for node_index in &order {
        let node = doc
            .nodes()
            .nth(*node_index)
            .ok_or(SkinImportError::NoSkeleton)?;
        let name = dedup_joint_name(
            sanitize_joint_name(node.name().unwrap_or(""), *node_index),
            &mut used,
        );
        let parent = parent_of[*node_index].and_then(|p| position_of[p]);
        joints.push(Joint { name, parent });
    }
    let skeleton = Skeleton { joints };
    validate_skeleton(&skeleton).map_err(map_skeleton_error)?;
    Ok(skeleton)
}

/// Shared row validation: row counts, influence bound, joint range,
/// finiteness, non-negativity and unit sum. Returns the packed skin.
fn validate_skin_rows(
    joints: &[Vec<u16>],
    weights: &[Vec<f32>],
    vertex_count: usize,
    skeleton_len: usize,
) -> Result<Skin, SkinImportError> {
    if joints.len() != vertex_count || weights.len() != vertex_count {
        return Err(SkinImportError::SkinLengthMismatch {
            expected: vertex_count,
            got: joints.len().min(weights.len()),
        });
    }
    let mut packed_joints = Vec::with_capacity(vertex_count);
    let mut packed_weights = Vec::with_capacity(vertex_count);
    for vertex in 0..vertex_count {
        let (row_joints, row_weights) = (&joints[vertex], &weights[vertex]);
        if row_joints.len() != row_weights.len() {
            return Err(SkinImportError::SkinRowMismatch { vertex });
        }
        if row_joints.len() > MAX_INFLUENCES {
            return Err(SkinImportError::TooManyInfluences {
                vertex,
                count: row_joints.len(),
            });
        }
        for joint in row_joints {
            if usize::from(*joint) >= skeleton_len {
                return Err(SkinImportError::JointOutOfRange {
                    vertex,
                    joint: usize::from(*joint),
                });
            }
        }
        let mut sum = 0.0f32;
        for weight in row_weights {
            if !weight.is_finite() {
                return Err(SkinImportError::NonFinite("weights"));
            }
            if *weight < 0.0 {
                return Err(SkinImportError::NegativeWeight { vertex });
            }
            sum += *weight;
        }
        if (sum - 1.0).abs() > WEIGHT_SUM_EPS {
            return Err(SkinImportError::BadWeightSum { vertex });
        }
        let mut packed_joint = [0u16; MAX_INFLUENCES];
        let mut packed_weight = [0.0f32; MAX_INFLUENCES];
        for (slot, (joint, weight)) in row_joints.iter().zip(row_weights.iter()).enumerate() {
            packed_joint[slot] = *joint;
            packed_weight[slot] = *weight;
        }
        packed_joints.push(packed_joint);
        packed_weights.push(packed_weight);
    }
    Ok(Skin {
        joints: packed_joints,
        weights: packed_weights,
    })
}

/// Import a synthetic skin doc against a skeleton: same row rules as the
/// glTF path (range, bound, finiteness, unit sum), packed to fixed rows.
pub fn import_skin_doc(doc: &SkinDoc, skeleton: &Skeleton) -> Result<Skin, SkinImportError> {
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    if doc.vertex_count == 0 || doc.joints.is_empty() {
        return Err(SkinImportError::SkinLengthMismatch {
            expected: doc.vertex_count.max(1),
            got: 0,
        });
    }
    validate_skin_rows(&doc.joints, &doc.weights, doc.vertex_count, skeleton.len())
}

/// Reject sparse skin accessors: silently de-sparsifying would guess data.
fn reject_sparse_skin(primitive: &gltf::Primitive<'_>) -> Result<(), SkinImportError> {
    use gltf::mesh::Semantic;
    for (semantic, accessor) in primitive.attributes() {
        let field = match semantic {
            Semantic::Joints(_) => "joints",
            Semantic::Weights(_) => "weights",
            _ => continue,
        };
        if accessor.sparse().is_some() {
            return Err(SkinImportError::SparseUnsupported { field });
        }
    }
    Ok(())
}

/// Import skin data for one glTF primitive: `None` when the primitive
/// carries no `JOINTS_0`/`WEIGHTS_0` (unskinned, valid); otherwise the
/// validated rows with joint indices mapped through the serving skin.
/// `mesh_index` names the owning mesh for diagnostics and skin lookup: the
/// first node using this mesh that also names a skin wins (documented).
pub fn import_skin_primitive(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    primitive: &gltf::Primitive<'_>,
    mesh_index: usize,
    skeleton: &Skeleton,
) -> Result<Option<Skin>, SkinImportError> {
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    let get_buffer = |buffer: gltf::Buffer<'_>| buffers.get(buffer.index()).map(|data| &*data.0);
    let reader = primitive.reader(get_buffer);
    let joints = reader.read_joints(0);
    let weights = reader.read_weights(0);
    match (joints, weights) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(SkinImportError::IncompleteSkin {
            mesh: mesh_index,
            primitive: primitive.index(),
        }),
        (Some(read_joints), Some(read_weights)) => {
            reject_sparse_skin(primitive)?;
            let mut serving: Option<gltf::Skin<'_>> = None;
            for node in doc.nodes() {
                if node.mesh().map(|mesh| mesh.index()) == Some(mesh_index) {
                    if let Some(skin) = node.skin() {
                        serving = Some(skin);
                        break;
                    }
                }
            }
            let Some(skin) = serving else {
                return Err(SkinImportError::NoSkin {
                    mesh: mesh_index,
                    primitive: primitive.index(),
                });
            };
            let skin_joints: Vec<usize> = skin.joints().map(|node| node.index()).collect();
            let order = skeleton_order(doc);
            let mut node_to_joint = vec![usize::MAX; doc.nodes().len()];
            for (position, node) in order.iter().enumerate() {
                node_to_joint[*node] = position;
            }
            let joint_rows: Vec<[u16; 4]> = read_joints.into_u16().collect();
            let weight_rows: Vec<[f32; 4]> = read_weights.into_f32().collect();
            let vertex_count: usize = primitive
                .reader(get_buffer)
                .read_positions()
                .map(|positions| positions.count())
                .unwrap_or(0);
            if joint_rows.len() != weight_rows.len() || joint_rows.len() != vertex_count {
                return Err(SkinImportError::SkinLengthMismatch {
                    expected: vertex_count,
                    got: joint_rows.len().min(weight_rows.len()),
                });
            }
            let mut rows_joints = Vec::with_capacity(vertex_count);
            let mut rows_weights = Vec::with_capacity(vertex_count);
            for (vertex, (joint_row, weight_row)) in
                joint_rows.iter().zip(weight_rows.iter()).enumerate()
            {
                let mut row_joints = Vec::with_capacity(MAX_INFLUENCES);
                for joint in joint_row {
                    let slot = usize::from(*joint) % skin_joints.len().max(1);
                    let node = skin_joints.get(slot).copied().unwrap_or(usize::MAX);
                    let mapped = node_to_joint.get(node).copied().unwrap_or(usize::MAX);
                    if mapped == usize::MAX || mapped >= skeleton.len() {
                        return Err(SkinImportError::JointOutOfRange {
                            vertex,
                            joint: usize::from(*joint),
                        });
                    }
                    row_joints.push(mapped as u16);
                }
                rows_joints.push(row_joints);
                rows_weights.push(weight_row.to_vec());
            }
            let skin =
                validate_skin_rows(&rows_joints, &rows_weights, vertex_count, skeleton.len())?;
            Ok(Some(skin))
        }
    }
}

/// Validate one key track: non-empty, bounded, paired finite times/values
/// with strictly increasing times. Quaternions additionally need unit norm.
fn check_translation_track(joint: usize, keys: &[TranslationKey]) -> Result<(), SkinImportError> {
    if keys.is_empty() {
        return Err(SkinImportError::EmptyTrack { joint });
    }
    if keys.len() > MAX_TRACK_KEYS {
        return Err(SkinImportError::TooLarge { count: keys.len() });
    }
    let mut prev = keys[0].time;
    if !prev.is_finite() {
        return Err(SkinImportError::NonFinite("clip.times"));
    }
    if keys[0].value.iter().any(|v| !v.is_finite()) {
        return Err(SkinImportError::NonFinite("clip.values"));
    }
    for key in &keys[1..] {
        if !key.time.is_finite() {
            return Err(SkinImportError::NonFinite("clip.times"));
        }
        if key.value.iter().any(|v| !v.is_finite()) {
            return Err(SkinImportError::NonFinite("clip.values"));
        }
        if key.time <= prev {
            return Err(SkinImportError::BadClip { joint });
        }
        prev = key.time;
    }
    Ok(())
}

/// Same as [`check_translation_track`], plus unit-quaternion validation.
fn check_rotation_track(joint: usize, keys: &[RotationKey]) -> Result<(), SkinImportError> {
    if keys.is_empty() {
        return Err(SkinImportError::EmptyTrack { joint });
    }
    if keys.len() > MAX_TRACK_KEYS {
        return Err(SkinImportError::TooLarge { count: keys.len() });
    }
    for (key_index, key) in keys.iter().enumerate() {
        if !key.time.is_finite() {
            return Err(SkinImportError::NonFinite("clip.times"));
        }
        if key.value.iter().any(|v| !v.is_finite()) {
            return Err(SkinImportError::NonFinite("clip.rotations"));
        }
        let norm = key.value.iter().map(|v| v * v).sum::<f32>().sqrt();
        if (norm - 1.0).abs() > QUAT_NORM_EPS {
            return Err(SkinImportError::BadQuat {
                joint,
                key: key_index,
            });
        }
    }
    let mut prev = keys[0].time;
    for key in &keys[1..] {
        if key.time <= prev {
            return Err(SkinImportError::BadClip { joint });
        }
        prev = key.time;
    }
    Ok(())
}

/// Truncate a carried clip name to [`MAX_CLIP_NAME_CHARS`] chars.
pub(crate) fn bound_clip_name(name: &str) -> String {
    bound_chars(name, MAX_CLIP_NAME_CHARS)
}

/// Import a synthetic clip doc against a skeleton: per-joint tracks
/// validated (lengths, finiteness, order, unit quats) into anim tracks.
/// Duration is the latest key time across all tracks.
pub fn import_clip_doc(
    doc: &ClipDoc,
    skeleton: &Skeleton,
) -> Result<ImportedClip, SkinImportError> {
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    let joints = skeleton.len();
    if doc.translations.len() != joints || doc.rotations.len() != joints {
        return Err(SkinImportError::ClipLengthMismatch {
            expected: joints,
            got: doc.translations.len().min(doc.rotations.len()),
        });
    }
    let mut positions = Vec::with_capacity(joints);
    let mut rotations = Vec::with_capacity(joints);
    let mut duration = 0.0f32;
    for joint in 0..joints {
        check_translation_track(joint, &doc.translations[joint])?;
        check_rotation_track(joint, &doc.rotations[joint])?;
        let times: Vec<f32> = doc.translations[joint].iter().map(|key| key.time).collect();
        let values: Vec<[f32; 3]> = doc.translations[joint]
            .iter()
            .map(|key| key.value)
            .collect();
        if let Some(last) = times.last() {
            duration = duration.max(*last);
        }
        positions.push(Track { times, values });
        let times: Vec<f32> = doc.rotations[joint].iter().map(|key| key.time).collect();
        let values: Vec<[f32; 4]> = doc.rotations[joint].iter().map(|key| key.value).collect();
        if let Some(last) = times.last() {
            duration = duration.max(*last);
        }
        rotations.push(QuatTrack { times, values });
    }
    Ok(ImportedClip {
        name: bound_clip_name(&doc.name),
        duration,
        positions: Clip { tracks: positions },
        rotations,
    })
}

/// Reject sparse animation accessors: silently de-sparsifying would guess keys.
fn reject_sparse_sampler(sampler: &gltf::animation::Sampler<'_>) -> Result<(), SkinImportError> {
    if sampler.input().sparse().is_some() {
        return Err(SkinImportError::SparseUnsupported {
            field: "clip.times",
        });
    }
    if sampler.output().sparse().is_some() {
        return Err(SkinImportError::SparseUnsupported {
            field: "clip.values",
        });
    }
    Ok(())
}

/// Import clips from a glTF document against an already-imported skeleton:
/// translation/rotation channels become validated tracks; unanimated joints
/// hold their node bind pose from time zero. Scale and morph-target channels
/// are refused (no guessing), as is non-`LINEAR` interpolation.
/// `buffers` carries the slice-import data (same pattern as mesh import:
/// external references without bytes fail typed, never panic).
pub fn import_clip(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    skeleton: &Skeleton,
) -> Result<Vec<ImportedClip>, SkinImportError> {
    use gltf::animation::util::ReadOutputs;
    use gltf::animation::{Interpolation, Property};
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    let animation_count = doc.animations().len();
    if animation_count > MAX_ANIMATIONS {
        return Err(SkinImportError::TooLarge {
            count: animation_count,
        });
    }
    let order = skeleton_order(doc);
    let mut node_to_joint = vec![usize::MAX; doc.nodes().len()];
    for (position, node) in order.iter().enumerate() {
        node_to_joint[*node] = position;
    }
    let mut bind_translation = vec![[0.0f32; 3]; skeleton.len()];
    let mut bind_rotation = vec![[0.0f32, 0.0, 0.0, 1.0]; skeleton.len()];
    for (position, node_index) in order.iter().enumerate() {
        let node = doc
            .nodes()
            .nth(*node_index)
            .ok_or(SkinImportError::NoSkeleton)?;
        let (translation, rotation, _scale) = node.transform().decomposed();
        bind_translation[position] = translation;
        bind_rotation[position] = rotation;
    }
    let get_buffer = |buffer: gltf::Buffer<'_>| buffers.get(buffer.index()).map(|data| &*data.0);
    let mut clips = Vec::with_capacity(animation_count);
    for (animation_index, animation) in doc.animations().enumerate() {
        let mut translations: Vec<Vec<TranslationKey>> = vec![Vec::new(); skeleton.len()];
        let mut rotations: Vec<Vec<RotationKey>> = vec![Vec::new(); skeleton.len()];
        let mut filled_translation = vec![false; skeleton.len()];
        let mut filled_rotation = vec![false; skeleton.len()];
        for channel in animation.channels() {
            let node_index = channel.target().node().index();
            let joint = node_to_joint.get(node_index).copied().unwrap_or(usize::MAX);
            if joint == usize::MAX || joint >= skeleton.len() {
                return Err(SkinImportError::UnknownJoint { joint: node_index });
            }
            let sampler = channel.sampler();
            if sampler.interpolation() != Interpolation::Linear {
                let name = match sampler.interpolation() {
                    Interpolation::Linear => "LINEAR",
                    Interpolation::Step => "STEP",
                    Interpolation::CubicSpline => "CUBICSPLINE",
                };
                return Err(SkinImportError::UnsupportedInterpolation {
                    name: name.to_string(),
                });
            }
            reject_sparse_sampler(&sampler)?;
            let reader = channel.reader(get_buffer);
            let times: Vec<f32> = reader
                .read_inputs()
                .ok_or(SkinImportError::BadClip { joint })?
                .collect();
            match channel.target().property() {
                Property::Translation => {
                    if filled_translation[joint] {
                        return Err(SkinImportError::DuplicateTrack { joint });
                    }
                    filled_translation[joint] = true;
                    let Some(ReadOutputs::Translations(outputs)) = reader.read_outputs() else {
                        return Err(SkinImportError::BadClip { joint });
                    };
                    let values: Vec<[f32; 3]> = outputs.collect();
                    if times.len() != values.len() {
                        return Err(SkinImportError::BadClip { joint });
                    }
                    translations[joint] = times
                        .into_iter()
                        .zip(values)
                        .map(|(time, value)| TranslationKey { time, value })
                        .collect();
                }
                Property::Rotation => {
                    if filled_rotation[joint] {
                        return Err(SkinImportError::DuplicateTrack { joint });
                    }
                    filled_rotation[joint] = true;
                    let Some(ReadOutputs::Rotations(outputs)) = reader.read_outputs() else {
                        return Err(SkinImportError::BadClip { joint });
                    };
                    let values: Vec<[f32; 4]> = outputs.into_f32().collect();
                    if times.len() != values.len() {
                        return Err(SkinImportError::BadClip { joint });
                    }
                    rotations[joint] = times
                        .into_iter()
                        .zip(values)
                        .map(|(time, value)| RotationKey { time, value })
                        .collect();
                }
                Property::Scale => {
                    return Err(SkinImportError::UnsupportedTarget {
                        detail: "scale".to_string(),
                    });
                }
                Property::MorphTargetWeights => {
                    return Err(SkinImportError::UnsupportedTarget {
                        detail: "weights".to_string(),
                    });
                }
            }
        }
        for joint in 0..skeleton.len() {
            if !filled_translation[joint] {
                translations[joint].push(TranslationKey {
                    time: 0.0,
                    value: bind_translation[joint],
                });
            }
            if !filled_rotation[joint] {
                rotations[joint].push(RotationKey {
                    time: 0.0,
                    value: bind_rotation[joint],
                });
            }
        }
        let name = animation
            .name()
            .unwrap_or(&format!("clip_{animation_index}"))
            .to_string();
        let clip_doc = ClipDoc {
            name,
            translations,
            rotations,
        };
        clips.push(import_clip_doc(&clip_doc, skeleton)?);
    }
    Ok(clips)
}

/// Sample one position track with clamping and lerp; structural problems in
/// hand-built tracks fail typed instead of panicking.
fn sample_track3(track: &Track, joint: usize, time: f32) -> Result<[f32; 3], SkinImportError> {
    if track.times.len() != track.values.len() || track.times.is_empty() {
        return Err(SkinImportError::BadClip { joint });
    }
    if time <= track.times[0] {
        return Ok(track.values[0]);
    }
    let last = track.times.len() - 1;
    if time >= track.times[last] {
        return Ok(track.values[last]);
    }
    let mut low = 0usize;
    let mut high = last;
    while high - low > 1 {
        let mid = (low + high) / 2;
        if track.times[mid] <= time {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = track.times[high] - track.times[low];
    let weight = if span <= 0.0 {
        0.0
    } else {
        ((time - track.times[low]) / span).clamp(0.0, 1.0)
    };
    let a = track.values[low];
    let b = track.values[high];
    Ok([
        a[0] + (b[0] - a[0]) * weight,
        a[1] + (b[1] - a[1]) * weight,
        a[2] + (b[2] - a[2]) * weight,
    ])
}

/// Sample one rotation track with normalized lerp; degenerate hand-built
/// input falls back to identity (the documented safe bind pose).
fn sample_quat_track(
    track: &QuatTrack,
    joint: usize,
    time: f32,
) -> Result<[f32; 4], SkinImportError> {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    if track.times.len() != track.values.len() || track.times.is_empty() {
        return Err(SkinImportError::BadClip { joint });
    }
    if time <= track.times[0] {
        return Ok(track.values[0]);
    }
    let last = track.times.len() - 1;
    if time >= track.times[last] {
        return Ok(track.values[last]);
    }
    let mut low = 0usize;
    let mut high = last;
    while high - low > 1 {
        let mid = (low + high) / 2;
        if track.times[mid] <= time {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = track.times[high] - track.times[low];
    let weight = if span <= 0.0 {
        0.0
    } else {
        ((time - track.times[low]) / span).clamp(0.0, 1.0)
    };
    let a = track.values[low];
    let mut b = track.values[high];
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
    }
    let mixed = [
        a[0] + (b[0] - a[0]) * weight,
        a[1] + (b[1] - a[1]) * weight,
        a[2] + (b[2] - a[2]) * weight,
        a[3] + (b[3] - a[3]) * weight,
    ];
    let norm_squared =
        mixed[0] * mixed[0] + mixed[1] * mixed[1] + mixed[2] * mixed[2] + mixed[3] * mixed[3];
    if !norm_squared.is_finite() {
        return Ok(IDENTITY);
    }
    if norm_squared <= 0.0 {
        return Ok(a);
    }
    let inverse = 1.0 / norm_squared.sqrt();
    Ok([
        mixed[0] * inverse,
        mixed[1] * inverse,
        mixed[2] * inverse,
        mixed[3] * inverse,
    ])
}

/// Yaw about `+Y` from a unit quaternion: positive yaw rotates `+Z` toward
/// `+X`, matching the locomotion convention. Import-time conversion only;
/// the cooked deltas keep the runtime trig-free.
fn quat_yaw(quaternion: &[f32; 4]) -> f32 {
    let (x, y, z, w) = (quaternion[0], quaternion[1], quaternion[2], quaternion[3]);
    (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// Tick count for a clip duration: at least one sample, bounded by
/// [`MAX_COOK_TICKS`]. The duration comes from validated keys, but
/// hand-built clips are rechecked here so no path can hang the cook.
fn cook_tick_count(duration: f32) -> Result<u32, SkinImportError> {
    if !duration.is_finite() || duration < 0.0 {
        return Err(SkinImportError::NonFinite("clip.duration"));
    }
    let mut ticks = (duration / TICK_DT).ceil() as u32;
    if ticks == 0 {
        ticks = 1;
    }
    if ticks > MAX_COOK_TICKS {
        return Err(SkinImportError::TooLarge {
            count: ticks as usize,
        });
    }
    Ok(ticks)
}

/// Cook per-tick root motion from a clip: the root joint's local XZ journey
/// plus yaw per 60 Hz tick, extracted through the anim helper so the result
/// always satisfies [`validate_root`].
pub fn cook_root_table(
    clip: &ImportedClip,
    skeleton: &Skeleton,
    root_joint: usize,
    start_tick: u32,
) -> Result<RootTable, SkinImportError> {
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    if root_joint >= skeleton.len() {
        return Err(SkinImportError::UnknownJoint { joint: root_joint });
    }
    let ticks = cook_tick_count(clip.duration)?;
    let positions = clip
        .positions
        .tracks
        .get(root_joint)
        .ok_or(SkinImportError::BadClip { joint: root_joint })?;
    let rotations = clip
        .rotations
        .get(root_joint)
        .ok_or(SkinImportError::BadClip { joint: root_joint })?;
    let mut planar = Vec::with_capacity(ticks as usize);
    let mut yaw = Vec::with_capacity(ticks as usize);
    for tick in 0..ticks {
        let time = tick as f32 * TICK_DT;
        let position = sample_track3(positions, root_joint, time)?;
        if position.iter().any(|v| !v.is_finite()) {
            return Err(SkinImportError::NonFinite("clip.values"));
        }
        let rotation = sample_quat_track(rotations, root_joint, time)?;
        if rotation.iter().any(|v| !v.is_finite()) {
            return Err(SkinImportError::NonFinite("clip.rotations"));
        }
        planar.push([position[0], position[2]]);
        yaw.push(quat_yaw(&rotation));
    }
    extract_root(&planar, &yaw, start_tick).map_err(|error| match error {
        AnimError::BadRootOrder(index) => SkinImportError::BadRoot { index },
        _ => SkinImportError::BadRoot { index: 0 },
    })
}

/// Cook per-tick foot contacts from a clip: each foot is planted while its
/// joint height stays at or below `ground_y` (the locomotion threshold rule),
/// validated through [`validate_contacts`].
pub fn cook_contact_table(
    clip: &ImportedClip,
    skeleton: &Skeleton,
    left_foot: usize,
    right_foot: usize,
    ground_y: f32,
    start_tick: u32,
) -> Result<ContactTable, SkinImportError> {
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    if left_foot >= skeleton.len() {
        return Err(SkinImportError::UnknownJoint { joint: left_foot });
    }
    if right_foot >= skeleton.len() {
        return Err(SkinImportError::UnknownJoint { joint: right_foot });
    }
    if !ground_y.is_finite() {
        return Err(SkinImportError::NonFinite("ground_y"));
    }
    let ticks = cook_tick_count(clip.duration)?;
    let left = clip
        .positions
        .tracks
        .get(left_foot)
        .ok_or(SkinImportError::BadClip { joint: left_foot })?;
    let right = clip
        .positions
        .tracks
        .get(right_foot)
        .ok_or(SkinImportError::BadClip { joint: right_foot })?;
    let mut contacts = Vec::with_capacity(2 * ticks as usize);
    for tick in 0..ticks {
        let time = tick as f32 * TICK_DT;
        let left_position = sample_track3(left, left_foot, time)?;
        let right_position = sample_track3(right, right_foot, time)?;
        if left_position.iter().any(|v| !v.is_finite())
            || right_position.iter().any(|v| !v.is_finite())
        {
            return Err(SkinImportError::NonFinite("clip.values"));
        }
        let stamp = start_tick.wrapping_add(tick);
        contacts.push(Contact {
            tick: stamp,
            foot: 0,
            down: left_position[1] <= ground_y,
        });
        contacts.push(Contact {
            tick: stamp,
            foot: 1,
            down: right_position[1] <= ground_y,
        });
    }
    let table = ContactTable { contacts };
    validate_contacts(&table).map_err(|error| match error {
        AnimError::BadFoot(index) | AnimError::BadContactOrder(index) => {
            SkinImportError::BadContact { index }
        }
        _ => SkinImportError::BadContact { index: 0 },
    })?;
    Ok(table)
}

/// Cook both authoritative tables for one clip from one [`CookConfig`].
pub fn cook_clip_tables(
    clip: &ImportedClip,
    skeleton: &Skeleton,
    config: &CookConfig,
) -> Result<(RootTable, ContactTable), SkinImportError> {
    let root = cook_root_table(clip, skeleton, config.root_joint, config.start_tick)?;
    let contacts = cook_contact_table(
        clip,
        skeleton,
        config.left_foot,
        config.right_foot,
        config.ground_y,
        config.start_tick,
    )?;
    Ok((root, contacts))
}

/// Import skinned-file bytes (`.gltf` text or `.glb` binary): skeleton from
/// the node hierarchy, skins for primitives carrying joint/weight data, and
/// clips from animation channels. Identical bytes import identically, so
/// reimport keeps every [`skin_content_id`].
pub fn import_skinned_bytes(bytes: &[u8]) -> Result<SkinnedImport, SkinImportError> {
    if bytes.len() > MAX_BYTES {
        return Err(SkinImportError::TooLarge { count: bytes.len() });
    }
    if bytes.is_empty() {
        return Err(SkinImportError::Parse {
            detail: "empty input".to_string(),
        });
    }
    let parsed = gltf::Gltf::from_slice(bytes).map_err(|error| match &error {
        gltf::Error::Validation(errors) => match required_extension_name(errors) {
            Some(name) => SkinImportError::UnsupportedExtension {
                name: bound_detail(&name),
            },
            None => SkinImportError::Parse {
                detail: bound_detail(&format!("{error:?}")),
            },
        },
        _ => SkinImportError::Parse {
            detail: bound_detail(&format!("{error:?}")),
        },
    })?;
    import_skinned_document(&parsed.document, parsed.blob)
}

/// Document half of [`import_skinned_bytes`]: shared parse plus per-section
/// imports over already-loaded buffers.
fn import_skinned_document(
    document: &gltf::Document,
    blob: Option<Vec<u8>>,
) -> Result<SkinnedImport, SkinImportError> {
    if let Some(name) = document.extensions_required().next() {
        return Err(SkinImportError::UnsupportedExtension {
            name: bound_detail(name),
        });
    }
    let buffers = gltf::import_buffers(document, None, blob).map_err(|error| match error {
        gltf::Error::ExternalReferenceInSliceImport
        | gltf::Error::UnsupportedScheme
        | gltf::Error::MissingBlob => SkinImportError::MissingBuffer {
            detail: bound_detail(&format!("{error:?}")),
        },
        gltf::Error::BufferLength {
            buffer,
            expected,
            actual,
        } => SkinImportError::MissingBuffer {
            detail: bound_detail(&format!("buffer {buffer}: {actual} of {expected} bytes")),
        },
        other => SkinImportError::Parse {
            detail: bound_detail(&format!("{other:?}")),
        },
    })?;
    let primitive_count: usize = document.meshes().map(|mesh| mesh.primitives().len()).sum();
    if primitive_count > MAX_PRIMITIVES {
        return Err(SkinImportError::TooLarge {
            count: primitive_count,
        });
    }
    let skeleton = import_skeleton(document)?;
    let mut skins = Vec::new();
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if let Some(skin) =
                import_skin_primitive(document, &buffers, &primitive, mesh.index(), &skeleton)?
            {
                skins.push(SkinnedPrimitive {
                    mesh: mesh.index(),
                    primitive: primitive.index(),
                    skin,
                });
            }
        }
    }
    let clips = import_clip(document, &buffers, &skeleton)?;
    Ok(SkinnedImport {
        skeleton,
        skins,
        clips,
    })
}

#[cfg(test)]
mod tests {
    use super::super::mesh_import::MAX_DIAGNOSTIC_CHARS;
    use super::*;
    use engine_anim::contract::{validate_contacts, validate_root};

    fn skeleton_doc() -> SkeletonDoc {
        SkeletonDoc {
            joints: vec![
                JointDoc {
                    name: "root".to_string(),
                    parent: None,
                },
                JointDoc {
                    name: "leg_l".to_string(),
                    parent: Some(0),
                },
                JointDoc {
                    name: "leg_r".to_string(),
                    parent: Some(0),
                },
            ],
        }
    }

    fn skeleton_fixture() -> Skeleton {
        import_skeleton_doc(&skeleton_doc()).expect("skeleton doc must import")
    }

    fn skin_doc() -> SkinDoc {
        SkinDoc {
            vertex_count: 3,
            joints: vec![vec![0, 1], vec![1, 2, 0], vec![0, 1, 2, 0]],
            weights: vec![
                vec![0.6, 0.4],
                vec![0.5, 0.25, 0.25],
                vec![0.34, 0.33, 0.33, 0.0],
            ],
        }
    }

    fn clip_doc() -> ClipDoc {
        ClipDoc {
            name: "stride".to_string(),
            translations: vec![
                vec![
                    TranslationKey {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0],
                    },
                    TranslationKey {
                        time: 0.5,
                        value: [0.5, 0.0, 0.0],
                    },
                    TranslationKey {
                        time: 1.0,
                        value: [1.0, 0.0, 0.0],
                    },
                ],
                vec![TranslationKey {
                    time: 0.0,
                    value: [0.11, -0.9, 0.0],
                }],
                vec![
                    TranslationKey {
                        time: 0.0,
                        value: [-0.11, -0.9, 0.0],
                    },
                    TranslationKey {
                        time: 0.5,
                        value: [-0.11, -0.4, 0.0],
                    },
                    TranslationKey {
                        time: 1.0,
                        value: [-0.11, -0.9, 0.0],
                    },
                ],
            ],
            rotations: vec![
                vec![RotationKey {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                }],
                vec![RotationKey {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                }],
                vec![RotationKey {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                }],
            ],
        }
    }

    #[test]
    fn skeleton_doc_validates_strictly() {
        let skeleton = skeleton_fixture();
        assert_eq!(skeleton.len(), 3);
        validate_skeleton(&skeleton).expect("doc skeleton passes C4");
        // Duplicates, self-parents, forward parents and bad names are
        // rejected with the joint index, never sanitized here.
        let mut dupe = skeleton_doc();
        dupe.joints[2].name = "leg_l".to_string();
        assert_eq!(
            import_skeleton_doc(&dupe),
            Err(SkinImportError::BadJoint { joint: 2 })
        );
        let mut looping = skeleton_doc();
        looping.joints[1].parent = Some(1);
        assert_eq!(
            import_skeleton_doc(&looping),
            Err(SkinImportError::BadJoint { joint: 1 })
        );
        let mut forward = skeleton_doc();
        forward.joints[0].parent = Some(2);
        assert_eq!(
            import_skeleton_doc(&forward),
            Err(SkinImportError::BadJoint { joint: 0 })
        );
        let mut bad_name = skeleton_doc();
        bad_name.joints[1].name = "Leg L!".to_string();
        assert_eq!(
            import_skeleton_doc(&bad_name),
            Err(SkinImportError::BadJoint { joint: 1 })
        );
        assert_eq!(
            import_skeleton_doc(&SkeletonDoc { joints: vec![] }),
            Err(SkinImportError::NoSkeleton)
        );
        let huge = SkeletonDoc {
            joints: (0..=MAX_JOINTS)
                .map(|i| JointDoc {
                    name: format!("j{i}"),
                    parent: if i == 0 { None } else { Some(i - 1) },
                })
                .collect(),
        };
        assert_eq!(
            import_skeleton_doc(&huge),
            Err(SkinImportError::TooLarge {
                count: MAX_JOINTS + 1
            })
        );
    }

    #[test]
    fn skin_doc_accepts_normalized_rejects_bad() {
        let skeleton = skeleton_fixture();
        let skin = import_skin_doc(&skin_doc(), &skeleton).expect("skin doc must import");
        assert_eq!(skin.joints.len(), 3);
        for row in &skin.weights {
            let sum: f32 = row.iter().sum();
            assert!(
                (sum - 1.0).abs() <= WEIGHT_SUM_EPS,
                "row sums to one: {row:?}"
            );
        }
        // Padding slots carry zero weight on joint zero.
        assert_eq!(skin.joints[0], [0, 1, 0, 0]);
        assert_eq!(skin.weights[0], [0.6, 0.4, 0.0, 0.0]);

        let mut unnormalized = skin_doc();
        unnormalized.weights[0] = vec![0.5, 0.4];
        assert_eq!(
            import_skin_doc(&unnormalized, &skeleton),
            Err(SkinImportError::BadWeightSum { vertex: 0 })
        );
        let mut negative = skin_doc();
        negative.weights[1] = vec![0.7, -0.1, 0.4];
        assert_eq!(
            import_skin_doc(&negative, &skeleton),
            Err(SkinImportError::NegativeWeight { vertex: 1 })
        );
        let mut out_of_range = skin_doc();
        out_of_range.joints[2] = vec![0, 1, 2, 9];
        assert_eq!(
            import_skin_doc(&out_of_range, &skeleton),
            Err(SkinImportError::JointOutOfRange {
                vertex: 2,
                joint: 9
            })
        );
        let mut too_many = skin_doc();
        too_many.joints[0] = vec![0, 1, 2, 0, 1];
        too_many.weights[0] = vec![0.2; 5];
        assert_eq!(
            import_skin_doc(&too_many, &skeleton),
            Err(SkinImportError::TooManyInfluences {
                vertex: 0,
                count: 5
            })
        );
        let mut ragged = skin_doc();
        ragged.weights[0] = vec![1.0];
        assert_eq!(
            import_skin_doc(&ragged, &skeleton),
            Err(SkinImportError::SkinRowMismatch { vertex: 0 })
        );
        let mut short = skin_doc();
        short.joints.pop();
        short.weights.pop();
        assert_eq!(
            import_skin_doc(&short, &skeleton),
            Err(SkinImportError::SkinLengthMismatch {
                expected: 3,
                got: 2
            })
        );
        let mut non_finite = skin_doc();
        non_finite.weights[0] = vec![f32::NAN, 1.0];
        assert_eq!(
            import_skin_doc(&non_finite, &skeleton),
            Err(SkinImportError::NonFinite("weights"))
        );
    }

    #[test]
    fn clip_doc_validates_tracks_and_cooks_tables() {
        let skeleton = skeleton_fixture();
        let clip = import_clip_doc(&clip_doc(), &skeleton).expect("clip doc must import");
        assert_eq!(clip.name, "stride");
        assert_eq!(clip.duration, 1.0);
        assert_eq!(clip.positions.tracks.len(), 3);
        assert_eq!(clip.rotations.len(), 3);

        let config = CookConfig {
            root_joint: 0,
            left_foot: 1,
            right_foot: 2,
            ground_y: -0.5,
            start_tick: 7,
        };
        let (root, contacts) =
            cook_clip_tables(&clip, &skeleton, &config).expect("tables must cook");
        // Both tables pass the anim validators (the gate for consumers).
        validate_root(&root).expect("cooked root passes C4");
        validate_contacts(&contacts).expect("cooked contacts pass C4");
        assert_eq!(root.deltas.len(), 60);
        assert_eq!(root.deltas[0].tick, 7);
        // Loop-aware travel: the first n-1 deltas sum to last-minus-first.
        let travel_x: f32 = root.deltas[..59].iter().map(|delta| delta.dp[0]).sum();
        assert!((travel_x - 59.0 / 60.0).abs() < 1e-4, "travel {travel_x}");
        assert!(root.deltas.iter().all(|delta| delta.dyaw == 0.0));
        // Left foot holds at -0.9 (planted); right foot lifts to -0.4
        // mid-clip, so both states appear with the ground at -0.5.
        assert_eq!(contacts.contacts.len(), 120);
        let left_down = contacts
            .contacts
            .iter()
            .filter(|c| c.foot == 0 && c.down)
            .count();
        assert_eq!(left_down, 60, "left foot never lifts");
        let right_down = contacts
            .contacts
            .iter()
            .filter(|c| c.foot == 1 && c.down)
            .count();
        assert!(
            right_down > 0 && right_down < 60,
            "right foot lifts: {right_down}/60"
        );

        assert_eq!(
            cook_root_table(&clip, &skeleton, 9, 0),
            Err(SkinImportError::UnknownJoint { joint: 9 })
        );
        assert_eq!(
            cook_contact_table(&clip, &skeleton, 1, 9, -0.5, 0),
            Err(SkinImportError::UnknownJoint { joint: 9 })
        );
        assert_eq!(
            cook_contact_table(&clip, &skeleton, 1, 2, f32::NAN, 0),
            Err(SkinImportError::NonFinite("ground_y"))
        );
    }

    #[test]
    fn clip_doc_rejects_bad_tracks() {
        let skeleton = skeleton_fixture();
        let mut unsorted = clip_doc();
        unsorted.translations[0][1].time = 0.0;
        assert_eq!(
            import_clip_doc(&unsorted, &skeleton),
            Err(SkinImportError::BadClip { joint: 0 })
        );
        let mut empty = clip_doc();
        empty.translations[1] = vec![];
        assert_eq!(
            import_clip_doc(&empty, &skeleton),
            Err(SkinImportError::EmptyTrack { joint: 1 })
        );
        let mut flat = clip_doc();
        flat.rotations[2][0].value = [0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            import_clip_doc(&flat, &skeleton),
            Err(SkinImportError::BadQuat { joint: 2, key: 0 })
        );
        let mut short = clip_doc();
        short.rotations.pop();
        assert_eq!(
            import_clip_doc(&short, &skeleton),
            Err(SkinImportError::ClipLengthMismatch {
                expected: 3,
                got: 2
            })
        );
        let mut non_finite = clip_doc();
        non_finite.translations[0][2].value[0] = f32::INFINITY;
        assert_eq!(
            import_clip_doc(&non_finite, &skeleton),
            Err(SkinImportError::NonFinite("clip.values"))
        );
    }

    // Synthetic glTF fixtures: authored JSON over one data-URI buffer, so no
    // donor bytes and no hero O7 content are needed for skin/clip coverage.

    /// Base64 without dependencies: standard alphabet, padded (test-only).
    fn b64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
            let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
            let packed = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((packed >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((packed >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[((packed >> 6) & 63) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(packed & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 * values.len());
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn u16_bytes(values: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 * values.len());
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Build the synthetic skinned fixture: 3 joints (root, leg_l, leg_r),
    /// one skinned triangle, one skin, and one animation. `channel_path` and
    /// `channel_interp` rewire the third channel to prove refusals;
    /// `weight_scale` corrupts the first weight row when not one.
    fn skinned_fixture(channel_path: &str, channel_interp: &str, weight_scale: f32) -> Vec<u8> {
        let w0 = [0.6 * weight_scale, 0.4 * weight_scale, 0.0, 0.0];
        let mut parts: Vec<Vec<u8>> = vec![
            f32_bytes(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            f32_bytes(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
            u16_bytes(&[0, 1, 0, 0, 1, 2, 0, 0, 0, 2, 1, 0]),
            f32_bytes(&[
                w0[0], w0[1], w0[2], w0[3], 0.5, 0.5, 0.0, 0.0, 0.34, 0.33, 0.33, 0.0,
            ]),
            u16_bytes(&[0, 1, 2]),
            f32_bytes(&[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]),
            f32_bytes(&[0.0, 0.5, 1.0]),
            f32_bytes(&[0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0]),
            f32_bytes(&[0.0, 1.0]),
            f32_bytes(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
            f32_bytes(&[0.0, 0.5, 1.0]),
            f32_bytes(&[0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0]),
        ];
        if channel_path == "sparse" {
            parts.push(u16_bytes(&[1]));
            parts.push(u16_bytes(&[0, 0, 0, 0]));
        }
        let mut offsets = Vec::with_capacity(parts.len());
        let mut cursor = 0usize;
        for part in &parts {
            offsets.push((cursor, part.len()));
            cursor += part.len();
        }
        let mut buffer = Vec::with_capacity(cursor);
        for part in &parts {
            buffer.extend_from_slice(part);
        }
        let uri = format!(
            "data:application/octet-stream;base64,{}",
            b64_encode(&buffer)
        );
        let views: String = offsets
            .iter()
            .enumerate()
            .map(|(i, (offset, len))| {
                let target = match i {
                    0 | 1 => ",\"target\":34962",
                    4 => ",\"target\":34963",
                    _ => "",
                };
                format!("{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":{len}{target}}}")
            })
            .collect::<Vec<_>>()
            .join(",");
        // Accessor table: (view, offset, component, count, kind, min, max).
        let accessors = [
            (0, 0, 5126, 3, "VEC3", "[0.0,0.0,0.0]", "[1.0,1.0,0.0]"),
            (1, 0, 5126, 3, "VEC3", "[0.0,0.0,1.0]", "[0.0,0.0,1.0]"),
            (2, 0, 5123, 3, "VEC4", "", ""),
            (3, 0, 5126, 3, "VEC4", "", ""),
            (4, 0, 5123, 3, "SCALAR", "", ""),
            (5, 0, 5126, 3, "MAT4", "", ""),
            (6, 0, 5126, 3, "SCALAR", "[0.0]", "[1.0]"),
            (7, 0, 5126, 3, "VEC3", "[0.0,0.0,0.0]", "[1.0,0.0,0.0]"),
            (8, 0, 5126, 2, "SCALAR", "[0.0]", "[1.0]"),
            (
                9,
                0,
                5126,
                2,
                "VEC4",
                "[0.0,0.0,0.0,1.0]",
                "[0.0,0.0,0.0,1.0]",
            ),
            (10, 0, 5126, 3, "SCALAR", "[0.0]", "[1.0]"),
            (11, 0, 5126, 3, "VEC3", "[0.0,0.0,0.0]", "[0.0,0.5,0.0]"),
        ];
        let accessor_json: String = accessors
            .iter()
            .map(|(view, offset, component, count, kind, min, max)| {
                let bounds = if min.is_empty() {
                    String::new()
                } else {
                    format!(",\"min\":{min},\"max\":{max}")
                };
                format!("{{\"bufferView\":{view},\"byteOffset\":{offset},\"componentType\":{component},\"count\":{count},\"type\":\"{kind}\"{bounds}}}")
            })
            .collect::<Vec<_>>()
            .join(",");
        let joints_accessor = if channel_path == "sparse" {
            let sparse_views = offsets.len() - 2;
            let sparse_values = offsets.len() - 1;
            format!(
                "{{\"bufferView\":2,\"byteOffset\":0,\"componentType\":5123,\"count\":3,\"type\":\"VEC4\",\
                \"sparse\":{{\"count\":1,\"indices\":{{\"bufferView\":{sparse_views},\"byteOffset\":0,\"componentType\":5123}},\
                \"values\":{{\"bufferView\":{sparse_values},\"byteOffset\":0}}}}}}"
            )
        } else {
            String::new()
        };
        let accessor_block = if joints_accessor.is_empty() {
            accessor_json
        } else {
            let head: String = accessors[..2]
                .iter()
                .map(|(view, offset, component, count, kind, min, max)| {
                    let bounds = if min.is_empty() {
                        String::new()
                    } else {
                        format!(",\"min\":{min},\"max\":{max}")
                    };
                    format!("{{\"bufferView\":{view},\"byteOffset\":{offset},\"componentType\":{component},\"count\":{count},\"type\":\"{kind}\"{bounds}}}")
                })
                .collect::<Vec<_>>()
                .join(",");
            let tail: String = accessors[3..]
                .iter()
                .map(|(view, offset, component, count, kind, min, max)| {
                    let bounds = if min.is_empty() {
                        String::new()
                    } else {
                        format!(",\"min\":{min},\"max\":{max}")
                    };
                    format!("{{\"bufferView\":{view},\"byteOffset\":{offset},\"componentType\":{component},\"count\":{count},\"type\":\"{kind}\"{bounds}}}")
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{head},{joints_accessor},{tail}")
        };
        let third_path = if channel_path == "sparse" {
            "translation"
        } else {
            channel_path
        };
        let text = format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
            \"nodes\":[{{\"name\":\"Root\",\"children\":[1,2],\"mesh\":0,\"skin\":0}},\
            {{\"name\":\"leg_l\"}},{{\"name\":\"leg_r\"}}],\
            \"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1,\"JOINTS_0\":2,\"WEIGHTS_0\":3}},\"indices\":4}}]}}],\
            \"skins\":[{{\"joints\":[0,1,2],\"inverseBindMatrices\":5}}],\
            \"animations\":[{{\"name\":\"stride\",\"samplers\":[\
            {{\"input\":6,\"interpolation\":\"LINEAR\",\"output\":7}},\
            {{\"input\":8,\"interpolation\":\"LINEAR\",\"output\":9}},\
            {{\"input\":10,\"interpolation\":\"{channel_interp}\",\"output\":11}}],\
            \"channels\":[{{\"sampler\":0,\"target\":{{\"node\":0,\"path\":\"translation\"}}}},\
            {{\"sampler\":1,\"target\":{{\"node\":1,\"path\":\"rotation\"}}}},\
            {{\"sampler\":2,\"target\":{{\"node\":2,\"path\":\"{third_path}\"}}}}]}}],\
            \"buffers\":[{{\"uri\":\"{uri}\",\"byteLength\":{cursor}}}],\
            \"bufferViews\":[{views}],\"accessors\":[{accessor_block}]}}"
        );
        text.into_bytes()
    }

    #[test]
    fn skinned_fixture_imports_with_normalized_weights() {
        let bytes = skinned_fixture("translation", "LINEAR", 1.0);
        let imported = import_skinned_bytes(&bytes).expect("skinned fixture must import");
        assert_eq!(imported.skeleton.len(), 3);
        validate_skeleton(&imported.skeleton).expect("skeleton passes C4");
        let names: Vec<&str> = imported
            .skeleton
            .joints
            .iter()
            .map(|joint| joint.name.as_str())
            .collect();
        assert_eq!(names, vec!["root", "leg_l", "leg_r"]);
        assert_eq!(imported.skins.len(), 1);
        let skin = &imported.skins[0].skin;
        assert_eq!(
            (imported.skins[0].mesh, imported.skins[0].primitive),
            (0, 0)
        );
        for row in &skin.weights {
            let sum: f32 = row.iter().sum();
            assert!(
                (sum - 1.0).abs() <= WEIGHT_SUM_EPS,
                "row sums to one: {row:?}"
            );
        }
        assert_eq!(imported.clips.len(), 1);
        assert_eq!(imported.clips[0].name, "stride");
        assert_eq!(imported.clips[0].duration, 1.0);
        // Root/contact cook validates through the anim validators.
        let config = CookConfig {
            root_joint: 0,
            left_foot: 1,
            right_foot: 2,
            ground_y: 0.05,
            start_tick: 0,
        };
        let (root, contacts) = cook_clip_tables(&imported.clips[0], &imported.skeleton, &config)
            .expect("tables must cook");
        validate_root(&root).expect("cooked root passes C4");
        validate_contacts(&contacts).expect("cooked contacts pass C4");
        assert_eq!(root.deltas.len(), 60);
        let travel_x: f32 = root.deltas[..59].iter().map(|delta| delta.dp[0]).sum();
        assert!((travel_x - 59.0 / 60.0).abs() < 1e-4, "travel {travel_x}");
        let right_down = contacts
            .contacts
            .iter()
            .filter(|c| c.foot == 1 && c.down)
            .count();
        assert!(
            right_down > 0 && right_down < 60,
            "right foot lifts: {right_down}/60"
        );
    }

    #[test]
    fn skinned_fixture_rejects_bad_weights_and_targets() {
        // Unnormalized first row (sums to 0.9) fails at the glTF skin path.
        let bytes = skinned_fixture("translation", "LINEAR", 0.9);
        assert_eq!(
            import_skinned_bytes(&bytes),
            Err(SkinImportError::BadWeightSum { vertex: 0 })
        );
        // Scale channels are refused, never guessed.
        let bytes = skinned_fixture("scale", "LINEAR", 1.0);
        assert_eq!(
            import_skinned_bytes(&bytes),
            Err(SkinImportError::UnsupportedTarget {
                detail: "scale".to_string()
            })
        );
        // Non-LINEAR interpolation is refused by name.
        let bytes = skinned_fixture("translation", "CUBICSPLINE", 1.0);
        assert_eq!(
            import_skinned_bytes(&bytes),
            Err(SkinImportError::UnsupportedInterpolation {
                name: "CUBICSPLINE".to_string()
            })
        );
        // Sparse skin accessors are refused, never de-sparsified silently.
        let bytes = skinned_fixture("sparse", "LINEAR", 1.0);
        assert_eq!(
            import_skinned_bytes(&bytes),
            Err(SkinImportError::SparseUnsupported { field: "joints" })
        );
    }

    #[test]
    fn oversized_skeleton_rejected_without_panic() {
        // 257 unnamed nodes: sanitized names stay unique, the joint budget
        // still rejects the file with a typed error.
        let mut nodes = String::from("[");
        for i in 0..=MAX_JOINTS {
            if i > 0 {
                nodes.push(',');
            }
            nodes.push_str(&format!("{{\"name\":\"n{i}\"}}"));
        }
        nodes.push(']');
        let text = format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
            \"nodes\":{nodes}}}"
        );
        assert_eq!(
            import_skinned_bytes(text.as_bytes()),
            Err(SkinImportError::TooLarge {
                count: MAX_JOINTS + 1
            })
        );
    }

    #[test]
    fn reimport_keeps_everything() {
        let bytes = skinned_fixture("translation", "LINEAR", 1.0);
        let first = import_skinned_bytes(&bytes).expect("first import");
        let second = import_skinned_bytes(&bytes).expect("second import");
        assert_eq!(first, second, "identical bytes import identically");
        assert_eq!(skin_content_id(&bytes), skin_content_id(&bytes));
        assert_ne!(skin_content_id(&bytes), skin_content_id(b"different bytes"));
    }

    #[test]
    fn malformed_files_rejected_without_panic() {
        for (label, bytes) in [
            ("empty", b"".as_slice()),
            ("garbage", b"not gltf at all".as_slice()),
            ("truncated-json", b"{\"asset\":".as_slice()),
            (
                "wrong-version",
                br#"{"asset":{"version":"1.0"}}"#.as_slice(),
            ),
            ("oversize", &vec![0u8; MAX_BYTES + 1]),
        ] {
            let err = import_skinned_bytes(bytes).expect_err(&format!("{label} must fail"));
            assert!(!format!("{err}").is_empty(), "{label} names the reason");
            assert!(
                format!("{err:?}").chars().count() <= MAX_DIAGNOSTIC_CHARS + 128,
                "{label} diagnostics bounded"
            );
        }
        // External buffer with no bytes: typed MissingBuffer, no panic.
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"name":"root","children":[1]},{"name":"leg_l"}],
            "meshes":[{"primitives":[
                {"attributes":{"POSITION":1,"JOINTS_0":2,"WEIGHTS_0":3},"indices":0}]}],
            "skins":[{"joints":[0,1]}],
            "buffers":[
                {"uri":"missing-external.bin","byteLength":36},
                {"uri":"data:application/octet-stream;base64,AAABAAIAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPwAAAAA=","byteLength":44},
                {"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","byteLength":24},
                {"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","byteLength":48}],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":6},
                {"buffer":1,"byteOffset":0,"byteLength":36},
                {"buffer":2,"byteOffset":0,"byteLength":24},
                {"buffer":3,"byteOffset":0,"byteLength":48}],
            "accessors":[
                {"bufferView":0,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"SCALAR"},
                {"bufferView":1,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC3",
                    "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]},
                {"bufferView":2,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"VEC4"},
                {"bufferView":3,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC4"}]
        }"#;
        let err = import_skinned_bytes(text.as_bytes()).expect_err("external buffer must fail");
        assert!(
            matches!(err, SkinImportError::MissingBuffer { .. }),
            "want MissingBuffer, got {err}"
        );
        // Required extensions we cannot interpret are named, not ignored.
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "extensionsRequired":["KHR_future_guess"],
            "extensionsUsed":["KHR_future_guess"],
            "nodes":[{"name":"root"}]
        }"#;
        let err = import_skinned_bytes(text.as_bytes()).expect_err("extension must fail");
        assert!(
            matches!(err, SkinImportError::UnsupportedExtension { .. }),
            "want UnsupportedExtension, got {err}"
        );
    }

    #[test]
    fn errors_display_bounded() {
        for error in [
            SkinImportError::NoSkeleton,
            SkinImportError::BadJoint { joint: 3 },
            SkinImportError::UnknownJoint { joint: 9 },
            SkinImportError::JointOutOfRange {
                vertex: 1,
                joint: 9,
            },
            SkinImportError::TooManyInfluences {
                vertex: 1,
                count: 5,
            },
            SkinImportError::NegativeWeight { vertex: 1 },
            SkinImportError::BadWeightSum { vertex: 1 },
            SkinImportError::SkinRowMismatch { vertex: 1 },
            SkinImportError::SkinLengthMismatch {
                expected: 3,
                got: 2,
            },
            SkinImportError::IncompleteSkin {
                mesh: 0,
                primitive: 0,
            },
            SkinImportError::NoSkin {
                mesh: 0,
                primitive: 0,
            },
            SkinImportError::SparseUnsupported { field: "joints" },
            SkinImportError::BadClip { joint: 1 },
            SkinImportError::EmptyTrack { joint: 1 },
            SkinImportError::ClipLengthMismatch {
                expected: 3,
                got: 2,
            },
            SkinImportError::BadQuat { joint: 1, key: 0 },
            SkinImportError::UnsupportedTarget {
                detail: "scale".to_string(),
            },
            SkinImportError::UnsupportedInterpolation {
                name: "STEP".to_string(),
            },
            SkinImportError::DuplicateTrack { joint: 1 },
            SkinImportError::BadRoot { index: 4 },
            SkinImportError::BadContact { index: 4 },
            SkinImportError::TooLarge { count: 1 },
            SkinImportError::NonFinite("weights"),
        ] {
            assert!(!format!("{error}").is_empty());
        }
        let long = "x".repeat(4096);
        let error = SkinImportError::Parse { detail: long };
        assert!(format!("{error}").chars().count() <= MAX_DIAGNOSTIC_CHARS + 64);
    }

    #[cfg(feature = "registry")]
    #[test]
    fn registry_reimport_keeps_asset_id() {
        use super::super::registry::AssetRegistry;
        let bytes = skinned_fixture("translation", "LINEAR", 1.0);
        let mut registry = AssetRegistry::new();
        let id = registry.register(&bytes, &[]).expect("register fixture");
        let kept = registry.reimport(id, &bytes, &[]).expect("reimport same");
        assert_eq!(kept.kept, id);
        assert!(!kept.changed, "identical bytes change nothing");
        assert_eq!(
            import_skinned_bytes(&bytes).expect("import"),
            import_skinned_bytes(&bytes).expect("reimport")
        );
    }
}
