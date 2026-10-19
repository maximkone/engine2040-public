//! Command service: validated, previewable, atomic, undoable scene edits.
//!
//! Single write path for the inspector, gizmo, scripts, CLI and AI proposals.
//! Units follow `engine-scene`: meters for positions, radians for rotations.
//! No IO, no threads; every function here is pure in-memory work.
//!
//! Flow: [`validate`] a [`Batch`] against a [`Document`] for a [`Preview`]
//! with no side effects, then [`apply`] it atomically through a [`Journal`].
//! [`undo`] and [`redo`] move entries between the journal stacks. Every apply,
//! undo and redo bumps the revision by exactly one, so a revision number is
//! never reused: a batch prepared against an older state always fails the
//! revision gate, even when undo brought back an identical hierarchy.
//! Every apply also stamps a provenance record (source, provider/model
//! IDs, request digest) and links it into each touched object's chain,
//! readable through [`chain_for`].
//! Grouped commands: entries carry a
//! [`CommandMeta`] name/flag plus a group id; [`begin_group`]/[`end_group`]
//! fold a gizmo drag into one undo step while insignificant steps skip the
//! durable journal (significant-only depth via test-only `durable_count`).
//! PORT(mikwielgus/undoredo@e0659096f26e4b07a68e549835756957b08af0b0:src/delta.rs
//! Delta commit shape + src/undoredo.rs + examples/cmd_commit.rs): one door
//! call is one group commit carrying its tool plus args in the entry name.

use std::collections::BTreeMap;
use std::fmt;

use engine_core::Fnv64;
use engine_material::{MaterialSet, RecordError};
use engine_scene::component::{Component, ComponentKind};
use engine_scene::ids::ObjectId;
use engine_scene::provenance::{Provenance, Source, MAX_PARENTS, MAX_TEXT_LEN, PROVENANCE_VERSION};
use engine_scene::transform::{Hierarchy, HierarchyError, Transform};

/// Kept journal entries per stack; the oldest entry is forgotten first.
pub const JOURNAL_CAP: usize = 100;

/// Authoring document: the scene hierarchy plus a revision counter.
///
/// The revision starts at zero and grows by one per apply, undo and redo. A
/// full undo run restores the starting hierarchy exactly; the revision keeps
/// counting so stale batches from before the undo stay detectable. Material
/// bindings ride alongside the hierarchy: the same staging, journal and
/// save path covers them, so a bound material survives undo, redo, save and
/// reopen exactly like a pose.
///
/// [`Document::material_set`] is the other half of materials: the table of
/// records ([`MaterialSet`]) the bindings name. It starts as
/// [`MaterialSet::siege_kit`] and is edited through [`Op::PutMaterial`] and
/// [`Op::SetMaterialParam`], which journal their own inverses like every op.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    /// Parent links plus local poses, keyed by persistent [`ObjectId`].
    pub hierarchy: Hierarchy,
    /// Monotonic: bumped once per apply, undo and redo; never restored.
    pub revision: u64,
    /// Material asset name per node, keyed by node id text. Only nodes
    /// present in [`Hierarchy`] may carry a binding: remove drops the entry,
    /// undo restores it. Text keys keep the map ordered without an `Ord`
    /// bound on the id type; every read converts once on this cold path.
    pub materials: BTreeMap<String, MaterialName>,
    /// Typed components per node, keyed by node id text then kind. Same
    /// rules as [`Document::materials`]: only nodes present in the
    /// hierarchy may carry entries, remove drops them, undo restores them,
    /// and the save codec writes them in key order so bytes are stable.
    pub components: ComponentTable,
    /// Node graph for material and VFX authoring; saved in the v4 section.
    pub graph: crate::graph::Graph,
    /// The material table: records in wire order, edited by
    /// [`Op::PutMaterial`] and [`Op::SetMaterialParam`]. Undo, redo and the
    /// journal cover it exactly like the hierarchy. Not in the scene bytes
    /// yet (a later save section owns that): a reopen starts from
    /// [`MaterialSet::siege_kit`] again.
    pub material_set: MaterialSet,
}

impl Default for Document {
    /// Empty hierarchy at revision zero with the shipped material table.
    ///
    /// The table defaults to [`MaterialSet::siege_kit`], the fourteen shipped
    /// records a fresh document binds against; the plain [`MaterialSet`]
    /// default (empty) would leave a document with nothing to bind.
    fn default() -> Self {
        Self {
            hierarchy: Hierarchy::default(),
            revision: 0,
            materials: BTreeMap::new(),
            components: ComponentTable::new(),
            graph: crate::graph::Graph::default(),
            material_set: MaterialSet::siege_kit(),
        }
    }
}

/// Components per node (id text) and kind, in key order.
pub type ComponentTable = BTreeMap<String, BTreeMap<ComponentKind, Component>>;

mod component;
pub(crate) mod digest;
mod selective;
use component::{stage_remove_component, stage_set_component};
use digest::artifact_digest;
pub use selective::{
    apply_selective_undo, apply_selective_undo_with_meta, undo_select, Conflict, ConflictReason,
    UndoError, UndoPreview,
};
pub mod material;
pub use material::{MaterialName, MaterialNameError, MAX_MATERIAL_NAME_LEN};

impl Document {
    /// Empty document at revision zero.
    pub fn new() -> Self {
        Self::default()
    }
}

/// One schema-typed edit inside a [`Batch`].
///
/// Not `Copy`: the material-table ops ([`Op::PutMaterial`],
/// [`Op::SetMaterialParam`]) carry record text, which is owned. Every other
/// op is a small value type.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Add a fresh node under `parent` (`nil` for a root) with `local` pose.
    Insert {
        /// Fresh id; must not be nil and must not exist yet.
        id: ObjectId,
        /// Parent node, or [`ObjectId::NIL`] for a root.
        parent: ObjectId,
        /// Pose in meters/radians relative to the parent.
        local: Transform,
    },
    /// Delete a node; its children attach to its parent, keeping world poses.
    Remove {
        /// Node to delete.
        id: ObjectId,
    },
    /// Replace a node's local pose (meters/radians).
    SetLocal {
        /// Node to move in its parent frame.
        id: ObjectId,
        /// New pose relative to the parent.
        local: Transform,
    },
    /// Move a node to `new_parent`, recomputing its local pose so its world
    /// pose is unchanged (`nil` detaches it to a root).
    Reparent {
        /// Node to move.
        id: ObjectId,
        /// New parent, or [`ObjectId::NIL`] for a root.
        new_parent: ObjectId,
    },
    /// Bind a material asset to a node (`None` clears the binding).
    ///
    /// Validated like every op (node must exist, revision must match) and
    /// journaled with its previous binding as the inverse, so material edits
    /// undo, redo, survive crash recovery and persist in the save bytes.
    SetMaterial {
        /// Node carrying the binding.
        id: ObjectId,
        /// Asset to bind, or [`None`] to clear.
        material: Option<MaterialName>,
    },
    /// Set (insert or replace) one typed component on a node. The value is
    /// validated at stage time; the inverse restores the previous value of
    /// that kind, or removes the kind when the node had none.
    SetComponent {
        /// Node carrying the component.
        id: ObjectId,
        /// New component value; its kind picks the slot.
        component: Component,
    },
    /// Remove one component kind from a node; refused when the node has none.
    RemoveComponent {
        /// Node carrying the component.
        id: ObjectId,
        /// Kind to remove.
        kind: ComponentKind,
    },
    /// Add or replace one material record in [`Document::material_set`] from
    /// its JSON text (see `engine_material::MaterialRecord`).
    ///
    /// A name the table already holds is replaced, and the inverse re-puts
    /// the previous record's text; a fresh name is appended, and the inverse
    /// is [`Op::RemoveLastMaterial`]. A bad text is refused with the record
    /// path and leaves the document (and its revision) untouched.
    PutMaterial {
        /// Record JSON text; its `material` key is the name.
        text: String,
    },
    /// Change one value of one material record: the table's
    /// `set_param` path, which re-validates the whole record.
    ///
    /// The inverse is the same op with the value `set_param` reported as
    /// `before`, so undo restores the pre-edit value exactly.
    SetMaterialParam {
        /// Record name (`MaterialRecord::name`).
        material: String,
        /// Dotted parameter path (`look.specular_roughness`,
        /// `layers[0].darken`).
        path: String,
        /// JSON text of the new value.
        value: String,
    },
    /// Drop the newest material record from [`Document::material_set`]; the
    /// inverse of appending a fresh record through [`Op::PutMaterial`].
    RemoveLastMaterial,
}

impl Op {
    /// The node this op reads or writes (the culprit when it fails).
    fn id(&self) -> ObjectId {
        match self {
            Op::Insert { id, .. }
            | Op::Remove { id }
            | Op::SetLocal { id, .. }
            | Op::SetMaterial { id, .. }
            | Op::SetComponent { id, .. }
            | Op::RemoveComponent { id, .. }
            | Op::Reparent { id, .. } => *id,
            // The material-table ops touch no node.
            Op::PutMaterial { .. } | Op::SetMaterialParam { .. } | Op::RemoveLastMaterial => {
                ObjectId::NIL
            }
        }
    }

    /// The linked parent for ops that carry one, else nil.
    fn linked_parent(&self) -> ObjectId {
        match self {
            Op::Insert { parent, .. } => *parent,
            Op::Reparent { new_parent, .. } => *new_parent,
            Op::Remove { .. }
            | Op::SetLocal { .. }
            | Op::SetMaterial { .. }
            | Op::SetComponent { .. }
            | Op::RemoveComponent { .. }
            | Op::PutMaterial { .. }
            | Op::SetMaterialParam { .. }
            | Op::RemoveLastMaterial => ObjectId::NIL,
        }
    }
}

/// Atomic transaction: all ops apply together or none does.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// Edits in order; later ops see earlier ones.
    pub ops: Vec<Op>,
    /// Must equal the document revision, else nothing is staged.
    pub base_revision: u64,
    /// Retry identity: same key plus same payload replays the stored receipt,
    /// same key with a different payload is a conflict.
    pub idempotency_key: u64,
    /// Provenance stamp for the receipt. `None` (old call sites, human UI
    /// edits) applies as [`ProvenanceStamp::human`]: human source, zero
    /// request digest. The stamp never affects idempotency identity: same key
    /// plus same ops replays the recorded receipt, first stamp wins.
    pub provenance: Option<ProvenanceStamp>,
}

impl Batch {
    /// Unstamped batch: applies as a human edit with a zero request digest.
    pub fn new(ops: Vec<Op>, base_revision: u64, idempotency_key: u64) -> Self {
        Self {
            ops,
            base_revision,
            idempotency_key,
            provenance: None,
        }
    }

    /// Attach a provenance stamp (script or model source, request digest).
    pub fn with_stamp(mut self, stamp: ProvenanceStamp) -> Self {
        self.provenance = Some(stamp);
        self
    }
}

/// Where an applied batch came from. Editor-side mirror of the provenance
/// [`Source`]: provider and model IDs only — never prompts, credentials or
/// session secrets. No such field exists on this type by construction (see
/// the `no_secret_like_field_exists` test, which destructures the full shape).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiptSource {
    /// Human editor action (inspector, gizmo, viewport).
    Human,
    /// Script or automation, named.
    Script {
        /// Automation name, bounded to [`MAX_TEXT_LEN`] on apply.
        name: String,
    },
    /// Model proposal through the provider service.
    Model {
        /// Provider ID (adapter name), bounded to [`MAX_TEXT_LEN`] on apply.
        provider: String,
        /// Model ID, bounded to [`MAX_TEXT_LEN`] on apply.
        model: String,
    },
}

/// Provenance input stamped on a [`Batch`]: source plus the digest of the
/// originating request. Human edits carry a zero digest. There are no fields
/// for prompts, credentials or secrets anywhere on this path by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceStamp {
    /// What made this change.
    pub source: ReceiptSource,
    /// Digest of the originating request (zeros for human/script edits).
    pub request_digest: [u8; 32],
}

impl ProvenanceStamp {
    /// Human edit with a zero digest (also the [`Default`]).
    pub fn human() -> Self {
        Self {
            source: ReceiptSource::Human,
            request_digest: [0u8; 32],
        }
    }

    /// Script edit with a zero digest.
    pub fn script(name: impl Into<String>) -> Self {
        Self {
            source: ReceiptSource::Script { name: name.into() },
            request_digest: [0u8; 32],
        }
    }

    /// Model edit carrying its request digest.
    pub fn model(
        provider: impl Into<String>,
        model: impl Into<String>,
        request_digest: [u8; 32],
    ) -> Self {
        Self {
            source: ReceiptSource::Model {
                provider: provider.into(),
                model: model.into(),
            },
            request_digest,
        }
    }
}

impl Default for ProvenanceStamp {
    /// Human source with a zero request digest.
    fn default() -> Self {
        Self::human()
    }
}

/// Dry-run outcome: one human-readable line per op plus the revision an
/// apply would produce. Validation never mutates the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// `base_revision + 1`; the revision [`apply`] would publish.
    pub resulting_revision: u64,
}

/// Receipt for an applied (or replayed) batch.
///
/// The consumer is per-object provenance reads (MCP) and approval display:
/// [`provenance`](Self::provenance) is the provenance record stored
/// through its codec roundtrip at apply time, and
/// [`chain_for`] rebuilds an object's history from the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// Document revision after the apply.
    pub revision: u64,
    /// The batch key this receipt answers.
    pub key: u64,
    /// Provenance record: source, provider/model IDs, request and
    /// artifact digests, verifier results and parent receipt IDs.
    pub provenance: Provenance,
    /// Position of the journal entry in the stack that now holds it (`past`
    /// for apply/redo, `future` for undo). A positional hint only: entries
    /// shift on eviction and undo/redo, so [`batch_id`](Self::batch_id) plus
    /// the revision is the stable identity.
    pub journal_index: usize,
}

impl Applied {
    /// Stable batch identity: the idempotency key this receipt answers.
    pub fn batch_id(&self) -> u64 {
        self.key
    }
}

/// What rejected a batch. Hierarchy failures name the culprit node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchError {
    /// No node has this id.
    UnknownNode {
        /// The missing node.
        id: ObjectId,
    },
    /// The linked parent is not nil and names no known node.
    UnknownParent {
        /// The node that names the missing parent.
        id: ObjectId,
        /// The missing parent.
        parent: ObjectId,
    },
    /// A node with this id already exists.
    Duplicate {
        /// The repeated id.
        id: ObjectId,
    },
    /// The reparent would make the node its own ancestor.
    Cycle {
        /// The node that would loop.
        id: ObjectId,
        /// The parent that would close the loop.
        new_parent: ObjectId,
    },
    /// A pose held NaN or infinity.
    NonFinite {
        /// The node holding the bad pose.
        id: ObjectId,
    },
    /// A scale on the path was too close to zero to invert across.
    DegenerateScale {
        /// The node being moved across the flat scale.
        id: ObjectId,
    },
    /// The node id itself was nil; nil is the "no parent" slot, not a node.
    NilId,
    /// A material name failed validation after construction (corrupt journal
    /// frame or save bytes carrying a bad name).
    BadMaterial {
        /// The node holding the bad binding.
        id: ObjectId,
    },
    /// A component value failed its schema check (non-finite or out-of-range
    /// field); the door reports the exact field before this point.
    BadComponent {
        /// The node the value was aimed at.
        id: ObjectId,
        /// The component kind that failed.
        kind: ComponentKind,
    },
    /// `RemoveComponent` named a kind the node does not carry.
    MissingComponent {
        /// The node asked.
        id: ObjectId,
        /// The kind it does not carry.
        kind: ComponentKind,
    },
    /// The batch base does not match the document revision; nothing ran.
    StaleRevision {
        /// Current document revision.
        expected: u64,
        /// Revision stamped on the batch.
        got: u64,
    },
    /// The key was already used by a different payload; nothing ran.
    DuplicateKey {
        /// The reused key.
        key: u64,
    },
    /// The key belongs to a batch that was applied and then undone. A retry
    /// must not silently resurrect it; nothing ran.
    UndoneKey {
        /// The key of the undone batch.
        key: u64,
    },
    /// A material-table edit was refused: the record text or the value did
    /// not parse, or the edit itself failed its check. Nothing ran.
    ///
    /// `path` is the record path the refusal names (`look.specular_roughness`,
    /// `body.toughness`, `material`); `detail` carries the reason. The table
    /// is left exactly as it was, which is why the document revision holds.
    BadMaterialRecord {
        /// Record path the refusal named.
        path: String,
        /// Reason below that path.
        detail: String,
    },
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode { id } => write!(f, "unknown node id {id}"),
            Self::UnknownParent { id, parent } => {
                write!(f, "node {id} names unknown parent {parent}")
            }
            Self::Duplicate { id } => write!(f, "node id {id} already exists"),
            Self::Cycle { id, new_parent } => {
                write!(
                    f,
                    "reparent of {id} under {new_parent} would create a cycle"
                )
            }
            Self::NonFinite { id } => write!(f, "node {id} holds a non-finite transform"),
            Self::DegenerateScale { id } => {
                write!(f, "node {id} sits across a degenerate scale")
            }
            Self::NilId => f.write_str("node id must not be nil"),
            Self::BadMaterial { id } => {
                write!(f, "node {id} holds an invalid material binding")
            }
            Self::BadComponent { id, kind } => write!(
                f,
                "node {id} was given an invalid {kind} component (non-finite or out of range)"
            ),
            Self::MissingComponent { id, kind } => {
                write!(f, "node {id} carries no {kind} component to remove")
            }
            Self::StaleRevision { expected, got } => write!(
                f,
                "stale revision: document is at {expected}, batch expects {got}"
            ),
            Self::DuplicateKey { key } => {
                write!(
                    f,
                    "idempotency key {key} was already used with another payload"
                )
            }
            Self::UndoneKey { key } => write!(
                f,
                "idempotency key {key} belongs to an undone batch; submit a new batch to reapply"
            ),
            Self::BadMaterialRecord { path, detail } => {
                write!(f, "material record refused at `{path}`: {detail}")
            }
        }
    }
}

impl std::error::Error for BatchError {}

/// One applied batch kept for undo, redo and idempotent replay.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalEntry {
    /// Retry identity from the batch.
    pub key: u64,
    /// Original ops, replayed by redo and compared for idempotency.
    pub forward: Vec<Op>,
    /// Ops restoring the pre-batch state, run last-op-first by [`undo`].
    pub inverse: Vec<Op>,
    /// Per-forward-op inverse groups in forward order (`inverse_groups[i]`
    /// undoes `forward[i]`); [`undo_select`] reverts clean ops through these
    /// so a conflicted batchmate never shifts another op's restore. The
    /// flattened [`JournalEntry::inverse`] stays the undo/redo path.
    pub inverse_groups: Vec<Vec<Op>>,
    /// Document revision before the batch.
    pub base_revision: u64,
    /// Document revision after the batch.
    pub result_revision: u64,
    /// Preview lines recorded at apply time.
    pub effects: Vec<String>,
    /// Provenance record, stored as the decoded form of its own encoding
    /// (see [`apply`]): receipts and [`chain_for`] always carry validated
    /// records, never unencoded input.
    pub provenance: Provenance,
    /// Command name for the undo panel (Fyrox `Command::name`); plain
    /// [`apply`] stamps `"edit"`. Bounded to [`MAX_COMMAND_NAME_LEN`].
    pub name: String,
    /// False for drag-preview steps: still applied and undoable, but skipped
    /// by the durable journal (significant-only depth; see test-only
    /// `durable_count`).
    pub significant: bool,
    /// Group this entry belongs to ([`NO_GROUP`] when ungrouped);
    /// [`end_group`] merges one group's entries into a single entry.
    pub group: u64,
    /// Every idempotency key folded into this entry (merged groups keep
    /// their members' keys); `keys[0]` repeats the entry [`key`](Self::key).
    pub keys: Vec<u64>,
}

/// Bounded undo/redo stacks plus the idempotency record.
///
/// `past` holds applied batches newest-last; `future` holds undone batches
/// newest-last for redo. Each stack caps at [`JOURNAL_CAP`]; the oldest entry
/// is forgotten first, which also releases its idempotency key. A new apply
/// clears the redo stack.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    /// Applied batches available to [`undo`], newest last.
    pub past: Vec<JournalEntry>,
    /// Undone batches available to [`redo`], newest last.
    pub future: Vec<JournalEntry>,
    /// Open command groups, outermost first. Entries applied while open
    /// carry the innermost id until [`end_group`] merges them.
    groups: Vec<GroupFrame>,
    /// Next group id; [`NO_GROUP`] means ungrouped and is never issued.
    next_group: u64,
    /// Open undo-coalescing gesture, if a slider lane is folding into the
    /// journal top. Transient UI state only: never journaled and never saved,
    /// so a reopen starts sealed. A plain apply, a group edit, undo, redo or
    /// an explicit [`seal_coalescing`] closes it.
    coalesce: Option<CoalesceCursor>,
}

/// One open [`begin_group`] frame: the name, flag and key the merged entry
/// takes when [`end_group`] commits.
#[derive(Clone, Debug, Default)]
struct GroupFrame {
    /// Id issued for this group (never [`NO_GROUP`]).
    id: u64,
    /// Merged entry name for the undo panel.
    name: String,
    /// Merged entry significance for the durable journal.
    significant: bool,
    /// Retry identity the merged entry answers.
    key: u64,
}

impl Journal {
    /// Empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Depth of the undo stack.
    pub fn past_len(&self) -> usize {
        self.past.len()
    }

    /// Depth of the redo stack.
    pub fn future_len(&self) -> usize {
        self.future.len()
    }

    /// True when both stacks are empty.
    pub fn is_empty(&self) -> bool {
        self.past.is_empty() && self.future.is_empty()
    }

    /// Install rehydrated stacks after a restart (pure, no IO).
    ///
    /// The caller loads the sealed stacks (see `durable::reopen_sealed`) and
    /// hands them here; open groups and coalescing gestures never seal, so
    /// they reopen closed and the next group id moves past any sealed id.
    pub fn rehydrate(&mut self, past: Vec<JournalEntry>, future: Vec<JournalEntry>) {
        let mut next = 1u64;
        for entry in past.iter().chain(future.iter()) {
            if entry.group >= next {
                next = entry.group.wrapping_add(1).max(1);
            }
        }
        self.past = past;
        self.future = future;
        self.groups.clear();
        self.next_group = next;
        self.coalesce = None;
    }
}

/// Longest kept command or group name; overlong names truncate at a
/// character boundary instead of failing the batch.
pub const MAX_COMMAND_NAME_LEN: usize = 128;

/// Group id of ungrouped entries; [`begin_group`] never issues it.
pub const NO_GROUP: u64 = 0;

/// Name plus significance of one command (Fyrox `Command::name` plus the
/// durable-journal flag). `Batch` itself stays a 4-field schema type so old
/// call sites keep compiling; the meta travels beside the batch into
/// [`apply_named`] and [`begin_group`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandMeta {
    /// Undo-panel name, bounded to [`MAX_COMMAND_NAME_LEN`].
    name: String,
    /// False skips the durable journal; the entry stays undoable.
    significant: bool,
}

impl CommandMeta {
    /// Named command with an explicit durable flag.
    pub fn new(name: impl Into<String>, significant: bool) -> Self {
        Self {
            name: bound_name(&name.into()),
            significant,
        }
    }

    /// Durable named command (inspector edits, committed drags, templates).
    pub fn significant(name: impl Into<String>) -> Self {
        Self::new(name, true)
    }

    /// Preview step (gizmo drag ticks): undoable, skipped by durability.
    pub fn insignificant(name: impl Into<String>) -> Self {
        Self::new(name, false)
    }

    /// Undo-panel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True when the durable journal must seal this entry.
    pub fn is_significant(&self) -> bool {
        self.significant
    }
}

impl Default for CommandMeta {
    /// Plain [`apply`] stamps `"edit"` and stays durable, as before.
    fn default() -> Self {
        Self {
            name: "edit".to_string(),
            significant: true,
        }
    }
}

/// Bound a command/group name to [`MAX_COMMAND_NAME_LEN`] at a character
/// boundary. Display bounds never reject an otherwise-valid edit.
fn bound_name(name: &str) -> String {
    if name.len() <= MAX_COMMAND_NAME_LEN {
        return name.to_owned();
    }
    let mut end = MAX_COMMAND_NAME_LEN;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// Open a command group: entries applied from now on carry the returned id
/// (nested groups tag the innermost one) until [`end_group`] merges them
/// into one undo step or [`cancel_group`] releases them. Returns the group
/// id (never [`NO_GROUP`]).
pub fn begin_group(journal: &mut Journal, meta: &CommandMeta, idempotency_key: u64) -> u64 {
    // A grouped gesture supersedes any open slider: seal it first.
    seal_coalescing(journal);
    let id = journal.next_group.max(1);
    journal.next_group = id.wrapping_add(1).max(1);
    journal.groups.push(GroupFrame {
        id,
        name: meta.name.clone(),
        significant: meta.significant,
        key: idempotency_key,
    });
    id
}

/// Commit the innermost open group: its entries merge into one entry named
/// and flagged by the [`begin_group`] meta, answering the group key. The
/// merge publishes no revision (steps already bumped one each); undo
/// restores the pre-group hierarchy bit-exactly and redo replays the merged
/// forwards. None when no group is open or nothing was staged since
/// [`begin_group`] (empty groups commit nothing).
pub fn end_group(journal: &mut Journal) -> Option<Applied> {
    // A group release seals any open slider gesture with it.
    seal_coalescing(journal);
    let frame = journal.groups.pop()?;
    // Trailing entries tagged with this group survive interleaved undo/redo:
    // scan back past foreign entries instead of trusting a start index.
    let start = journal
        .past
        .iter()
        .rposition(|entry| entry.group != frame.id)
        .map(|index| index + 1)
        .unwrap_or(0);
    if start >= journal.past.len() {
        return None;
    }
    let members: Vec<JournalEntry> = journal.past.drain(start..).collect();
    if members.is_empty() {
        return None;
    }
    let mut forward = Vec::new();
    let mut inverse_groups: Vec<Vec<Op>> = Vec::new();
    let mut effects: Vec<String> = Vec::new();
    let mut keys: Vec<u64> = Vec::new();
    for member in &members {
        forward.extend(member.forward.iter().cloned());
        inverse_groups.extend(member.inverse_groups.iter().cloned());
        effects.extend(member.effects.iter().cloned());
        for key in &member.keys {
            if !keys.contains(key) {
                keys.push(*key);
            }
        }
    }
    if !keys.contains(&frame.key) {
        keys.insert(0, frame.key);
    }
    // Undo runs last entry first; each member inverse is already last-op
    // first, so reversing member order restores the pre-group state exactly.
    let mut inverse = Vec::new();
    for member in members.iter().rev() {
        inverse.extend(member.inverse.iter().cloned());
    }
    let base_revision = members[0].base_revision;
    let last = &members[members.len() - 1];
    let result_revision = last.result_revision;
    let provenance = last.provenance.clone();
    let group = journal
        .groups
        .last()
        .map(|open| open.id)
        .unwrap_or(NO_GROUP);
    journal.past.push(JournalEntry {
        key: frame.key,
        forward,
        inverse,
        inverse_groups,
        base_revision,
        result_revision,
        effects: effects.clone(),
        provenance: provenance.clone(),
        name: frame.name.clone(),
        significant: frame.significant,
        group,
        keys,
    });
    // Merging shrinks the stack; evict only with no group left open.
    if journal.groups.is_empty() {
        while journal.past.len() > JOURNAL_CAP {
            journal.past.remove(0);
        }
    }
    let index = journal.past.len() - 1;
    Some(Applied {
        effects,
        revision: result_revision,
        key: frame.key,
        provenance,
        journal_index: index,
    })
}

/// Drop the innermost open group without merging: its steps stay individual
/// entries. True when a group was open.
pub fn cancel_group(journal: &mut Journal) -> bool {
    seal_coalescing(journal);
    journal.groups.pop().is_some()
}

/// Count of open command groups (nesting depth).
pub fn open_group_depth(journal: &Journal) -> usize {
    journal.groups.len()
}

/// Id of the innermost open group, if any.
pub fn open_group_id(journal: &Journal) -> Option<u64> {
    journal.groups.last().map(|frame| frame.id)
}

/// Retry key the innermost open group will answer when [`end_group`]
/// commits it, if any. Lets crash-safe wrappers stage the END marker
/// before popping the frame.
pub fn open_group_key(journal: &Journal) -> Option<u64> {
    journal.groups.last().map(|frame| frame.key)
}

/// True when the durable journal must seal this entry: significant entries
/// only. Insignificant drag steps skip durability but stay undoable.
///
/// Test-only: no prod or endpoint caller seals through this; the four
/// test suites asserting significant-only semantics call it directly.
#[cfg(test)]
pub fn needs_persist(entry: &JournalEntry) -> bool {
    entry.significant
}

/// Durable-journal depth: significant entries on the undo stack. Sealing
/// one generation per counted entry keeps crash recovery while a 20-step
/// gizmo drag seals one generation instead of twenty.
///
/// Test-only: kept visible (not deleted) so the significant-only
/// semantics stay asserted by the test suites.
#[cfg(test)]
pub fn durable_count(journal: &Journal) -> usize {
    journal
        .past
        .iter()
        .filter(|entry| entry.significant)
        .count()
}

// Undo-coalescing gesture folding (slider lanes): window, cursor, merge
// keys, fold checks and the coalesced apply path. Lives in
// `commands_coalesce.rs` and is included here so the journal types stay in
// one module without touching `lib.rs`; the public surface
// (`apply_coalesced`, `merge_key`, `seal_coalescing`, `coalescing_open`,
// `COALESCE_WINDOW`) is unchanged.
include!("commands_coalesce.rs");

/// Validate a batch and describe it, without touching the document.
///
/// Checks the revision gate first, then stages every op on a private copy.
/// Any failure reports the culprit and stages nothing.
pub fn validate(doc: &Document, batch: &Batch) -> Result<Preview, BatchError> {
    let staged = stage(doc, batch)?;
    Ok(Preview {
        effects: staged.effects,
        resulting_revision: doc.revision + 1,
    })
}

/// Apply a batch atomically: validate everything first, then commit.
///
/// A stale revision or any bad op applies NOTHING: the document and the
/// journal are untouched. On success the revision grows by one, the inverse
/// ops plus the key land on the journal, and the redo stack is cleared.
/// Repeating a recorded key with the same payload replays the stored receipt
/// without re-applying; the same key with another payload is a conflict, and
/// the key of an undone batch is refused rather than reported as applied.
/// Stamps the default [`CommandMeta`] (`"edit"`, durable); use
/// [`apply_named`] for drag steps and grouped commits.
pub fn apply(
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
) -> Result<Applied, BatchError> {
    apply_named(doc, journal, batch, &CommandMeta::default())
}

/// [`apply`] with a command name and durable flag.
///
/// The entry carries `meta` (and the innermost open group id, if any), so a
/// gizmo drag applies each tick as an insignificant step through the same
/// validated batch path, then [`end_group`] folds the steps into one durable
/// undo step. Semantics match [`apply`] otherwise: atomic, revision-gated,
/// idempotent, redo-clearing.
pub fn apply_named(
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
    meta: &CommandMeta,
) -> Result<Applied, BatchError> {
    if let Some(known) = replay_known_key(journal, batch) {
        return known;
    }
    // A genuinely new batch seals any open slider gesture: plain applies
    // (staged inspector commits, inspector auto-edits) always start fresh.
    // Replays above return early and leave the gesture alone.
    seal_coalescing(journal);
    let staged = stage(doc, batch)?;
    let result_revision = doc.revision + 1;
    // Parents name earlier receipts before this entry lands, so the new
    // record links onto every touched object's chain.
    let provenance = build_record(batch, result_revision, journal);
    Ok(commit_staged(doc, journal, batch, meta, staged, provenance))
}

/// Undo the newest applied batch as a new revision and queue the entry for
/// [`redo`]. None when the undo stack is empty or the stored inverse no longer
/// applies, leaving the document untouched.
pub fn undo(doc: &mut Document, journal: &mut Journal) -> Option<Applied> {
    seal_coalescing(journal);
    let entry = journal.past.pop()?;
    let mut tree = doc.hierarchy.clone();
    let mut materials = doc.materials.clone();
    let mut components = doc.components.clone();
    let mut material_set = doc.material_set.clone();
    if apply_ops(
        &mut tree,
        &mut materials,
        &mut components,
        &mut material_set,
        &entry.inverse,
    )
    .is_err()
    {
        journal.past.push(entry);
        return None;
    }
    doc.hierarchy = tree;
    doc.materials = materials;
    doc.components = components;
    doc.material_set = material_set;
    doc.revision += 1;
    journal.future.push(entry);
    if journal.future.len() > JOURNAL_CAP {
        journal.future.remove(0);
    }
    let index = journal.future.len() - 1;
    let entry = &journal.future[index];
    let receipt = Applied {
        revision: doc.revision,
        effects: vec![format!(
            "undo batch key {} ({} inverse ops)",
            entry.key,
            entry.inverse.len()
        )],
        key: entry.key,
        provenance: entry.provenance.clone(),
        journal_index: index,
    };
    Some(receipt)
}

/// Redo the newest undone batch as a new revision and queue the entry for
/// [`undo`]. None when the redo stack is empty or the stored ops no longer
/// apply, leaving the document untouched.
pub fn redo(doc: &mut Document, journal: &mut Journal) -> Option<Applied> {
    seal_coalescing(journal);
    let entry = journal.future.pop()?;
    let mut tree = doc.hierarchy.clone();
    let mut materials = doc.materials.clone();
    let mut components = doc.components.clone();
    let mut material_set = doc.material_set.clone();
    if apply_ops(
        &mut tree,
        &mut materials,
        &mut components,
        &mut material_set,
        &entry.forward,
    )
    .is_err()
    {
        journal.future.push(entry);
        return None;
    }
    doc.hierarchy = tree;
    doc.materials = materials;
    doc.components = components;
    doc.material_set = material_set;
    doc.revision += 1;
    journal.past.push(entry);
    if journal.past.len() > JOURNAL_CAP {
        journal.past.remove(0);
    }
    let index = journal.past.len() - 1;
    let entry = &journal.past[index];
    let receipt = Applied {
        revision: doc.revision,
        effects: entry.effects.clone(),
        key: entry.key,
        provenance: entry.provenance.clone(),
        journal_index: index,
    };
    Some(receipt)
}

/// Per-object provenance history from the applied journal, oldest first.
///
/// One entry per applied batch whose ops target `object` (the op's own node,
/// see [`Op`]: insert/remove/move/measurement targets, not linked parents).
/// Each record's parents name earlier receipts, so the returned chain links
/// newest back to the object's first record. Undone batches live on the redo
/// stack and rejoin the chain on redo. No production reader yet: the endpoint
/// assistant covers it in tests; a future MCP per-object verb and the
/// approval display would consume it.
pub fn chain_for(journal: &Journal, object: ObjectId) -> Vec<Provenance> {
    journal
        .past
        .iter()
        .filter(|entry| entry.forward.iter().any(|op| op.id() == object))
        .map(|entry| entry.provenance.clone())
        .collect()
}

/// Build the provenance record for a staged batch and run it through the
/// provenance codec roundtrip: the stored receipt is the decoded form of its
/// own encoding, so receipts and [`chain_for`] always carry validated
/// records, never raw input.
///
/// The roundtrip is infallible by construction: the version is current,
/// texts are bounded by [`bound_to`], verifiers are empty (the capability
/// layer threads spatial outcomes through later) and parents are capped by
/// [`parent_receipts`]. The `expect` messages name that invariant.
fn build_record(batch: &Batch, result_revision: u64, journal: &Journal) -> Provenance {
    let stamp = batch.provenance.clone().unwrap_or_default();
    let source = match stamp.source {
        ReceiptSource::Human => Source::Human,
        ReceiptSource::Script { name } => Source::Script(bound_to(&name, MAX_TEXT_LEN)),
        ReceiptSource::Model { provider, model } => Source::Model {
            provider: bound_to(&provider, MAX_TEXT_LEN),
            model: bound_to(&model, MAX_TEXT_LEN),
        },
    };
    let mut touched: Vec<ObjectId> = Vec::with_capacity(batch.ops.len());
    for op in &batch.ops {
        let id = op.id();
        if !touched.contains(&id) {
            touched.push(id);
        }
    }
    let record = Provenance {
        version: PROVENANCE_VERSION,
        source,
        request_digest: stamp.request_digest,
        artifact_digest: artifact_digest(
            &batch.ops,
            batch.idempotency_key,
            batch.base_revision,
            result_revision,
        ),
        verifiers: Vec::new(),
        parents: parent_receipts(journal, &touched),
    };
    let bytes = record
        .encode()
        .expect("provenance record bounded by construction");
    Provenance::decode(&bytes).expect("provenance record just encoded")
}

/// Parent receipt IDs for the touched objects: the newest applied receipt per
/// object is not tracked separately, so scan the journal oldest-first and
/// keep each distinct receipt that touches the batch, capped at
/// [`MAX_PARENTS`]. Order is journal order, fully deterministic.
fn parent_receipts(journal: &Journal, touched: &[ObjectId]) -> Vec<u64> {
    let mut parents = Vec::new();
    for entry in &journal.past {
        if entry.forward.iter().any(|op| touched.contains(&op.id())) {
            let receipt = entry.provenance.receipt();
            if !parents.contains(&receipt) {
                parents.push(receipt);
                if parents.len() >= MAX_PARENTS {
                    break;
                }
            }
        }
    }
    parents
}

/// `text` cut to at most `max` bytes at a character boundary; shorter text
/// comes back whole. Display bounds (provenance names, panel echoes, cell
/// texts) must never reject an otherwise-valid edit, so overlong text
/// truncates deterministically instead of failing the batch.
pub(crate) fn bound_to(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Staged batch: the resulting tree, its inverse ops and its preview lines.
struct Staged {
    tree: Hierarchy,
    /// Resulting material bindings (staged beside the tree, committed with it).
    materials: BTreeMap<String, MaterialName>,
    /// Resulting components (staged beside the tree, committed with it).
    components: ComponentTable,
    /// Resulting material table (staged beside the tree, committed with it).
    material_set: MaterialSet,
    inverse: Vec<Op>,
    /// Per-forward-op inverse groups in forward order; flattened (reversed)
    /// into `inverse` above. Stored on the journal entry for selective undo.
    per_op: Vec<Vec<Op>>,
    effects: Vec<String>,
}

/// Revision gate plus per-op staging on a private copy. The caller commits
/// the tree only after every op succeeds, which keeps batches all-or-nothing.
fn stage(doc: &Document, batch: &Batch) -> Result<Staged, BatchError> {
    if batch.base_revision != doc.revision {
        return Err(BatchError::StaleRevision {
            expected: doc.revision,
            got: batch.base_revision,
        });
    }
    let mut tree = doc.hierarchy.clone();
    let mut materials = doc.materials.clone();
    let mut components = doc.components.clone();
    let mut material_set = doc.material_set.clone();
    let mut per_op_inverse = Vec::with_capacity(batch.ops.len());
    let mut effects = Vec::with_capacity(batch.ops.len());
    for op in &batch.ops {
        let (inverse, line) = apply_one(
            &mut tree,
            &mut materials,
            &mut components,
            &mut material_set,
            op,
        )?;
        per_op_inverse.push(inverse);
        effects.push(line);
    }
    // Undo runs last op first, so reverse the per-op inverses.
    let inverse = per_op_inverse.iter().rev().flatten().cloned().collect();
    Ok(Staged {
        tree,
        materials,
        components,
        material_set,
        inverse,
        per_op: per_op_inverse,
        effects,
    })
}

/// Run staged ops without recording (undo/redo internals). Atomic by the
/// same rule: the caller swaps the tree in only on full success.
fn apply_ops(
    tree: &mut Hierarchy,
    materials: &mut BTreeMap<String, MaterialName>,
    components: &mut ComponentTable,
    material_set: &mut MaterialSet,
    ops: &[Op],
) -> Result<(), BatchError> {
    for op in ops {
        apply_one(tree, materials, components, material_set, op).map(|_| ())?;
    }
    Ok(())
}

/// Stage one op, returning its inverse ops plus its preview line.
///
/// Inverses capture pre-op state so undo restores parents, locals, material
/// bindings and material records exactly (bit-for-bit), even though
/// reparent/remove recompute poses in flight.
fn apply_one(
    tree: &mut Hierarchy,
    materials: &mut BTreeMap<String, MaterialName>,
    components: &mut ComponentTable,
    material_set: &mut MaterialSet,
    op: &Op,
) -> Result<(Vec<Op>, String), BatchError> {
    match op {
        Op::Insert { id, parent, local } => stage_insert(tree, op, *id, *parent, *local),
        Op::Remove { id } => stage_remove(tree, materials, components, op, *id),
        Op::SetComponent { id, component } => {
            stage_set_component(tree, components, *id, *component)
        }
        Op::RemoveComponent { id, kind } => stage_remove_component(tree, components, *id, *kind),
        Op::SetLocal { id, local } => stage_set_local(tree, op, *id, *local),
        Op::Reparent { id, new_parent } => stage_reparent(tree, op, *id, *new_parent),
        Op::SetMaterial { id, material } => stage_set_material(tree, materials, *id, *material),
        Op::PutMaterial { text } => stage_put_material(material_set, text),
        Op::SetMaterialParam {
            material,
            path,
            value,
        } => stage_set_material_param(material_set, material, path, value),
        Op::RemoveLastMaterial => stage_remove_last_material(material_set),
    }
}

/// Stage an insert. Fresh ids have no children yet, so a plain remove inverts.
fn stage_insert(
    tree: &mut Hierarchy,
    op: &Op,
    id: ObjectId,
    parent: ObjectId,
    local: Transform,
) -> Result<(Vec<Op>, String), BatchError> {
    tree.insert(id, parent, local)
        .map_err(|error| map_error(error, op))?;
    let line = format!("insert {id} under {parent} local {}", fmt_local(&local));
    Ok((vec![Op::Remove { id }], line))
}

/// Stage a remove. Children relink to the grandparent, so their locals are
/// captured first and the inverse pins each one back exactly.
fn stage_remove(
    tree: &mut Hierarchy,
    materials: &mut BTreeMap<String, MaterialName>,
    components: &mut ComponentTable,
    op: &Op,
    id: ObjectId,
) -> Result<(Vec<Op>, String), BatchError> {
    let parent = tree.parent(id).ok_or(BatchError::UnknownNode { id })?;
    let local = tree.local(id).ok_or(BatchError::UnknownNode { id })?;
    // Children relink to the grandparent on remove; capture their
    // locals now so undo can pin each one back exactly.
    let mut kids = Vec::new();
    for child in tree.children(id) {
        let child_local = tree
            .local(child)
            .ok_or(BatchError::UnknownNode { id: child })?;
        kids.push((child, child_local));
    }
    tree.remove(id).map_err(|error| map_error(error, op))?;
    let mut inverse = Vec::with_capacity(1 + 2 * kids.len());
    inverse.push(Op::Insert { id, parent, local });
    for (child, child_local) in &kids {
        inverse.push(Op::Reparent {
            id: *child,
            new_parent: id,
        });
        inverse.push(Op::SetLocal {
            id: *child,
            local: *child_local,
        });
    }
    // Bindings die with their node; undo re-binds after the re-insert
    // so the restore order stays valid (insert first, then bind).
    let old_material = materials.remove(&id.to_string());
    if let Some(material) = old_material {
        inverse.push(Op::SetMaterial {
            id,
            material: Some(material),
        });
    }
    // Components die with their node the same way; undo re-sets each kind
    // (kind order) after the re-insert.
    if let Some(own) = components.remove(&id.to_string()) {
        for component in own.into_values() {
            inverse.push(Op::SetComponent { id, component });
        }
    }
    let line = format!("remove {id} (relinked {} children to {parent})", kids.len());
    Ok((inverse, line))
}

/// Stage a pose replacement; the inverse restores the captured pose.
fn stage_set_local(
    tree: &mut Hierarchy,
    op: &Op,
    id: ObjectId,
    local: Transform,
) -> Result<(Vec<Op>, String), BatchError> {
    let old = tree.local(id).ok_or(BatchError::UnknownNode { id })?;
    tree.set_local(id, local)
        .map_err(|error| map_error(error, op))?;
    let line = format!(
        "set-local {id} from {} to {}",
        fmt_local(&old),
        fmt_local(&local)
    );
    Ok((vec![Op::SetLocal { id, local: old }], line))
}

/// Stage a reparent; a move to the current parent is a no-op with no inverse.
fn stage_reparent(
    tree: &mut Hierarchy,
    op: &Op,
    id: ObjectId,
    new_parent: ObjectId,
) -> Result<(Vec<Op>, String), BatchError> {
    let old_parent = tree.parent(id).ok_or(BatchError::UnknownNode { id })?;
    let old_local = tree.local(id).ok_or(BatchError::UnknownNode { id })?;
    tree.reparent(id, new_parent)
        .map_err(|error| map_error(error, op))?;
    if new_parent == old_parent {
        let line = format!("reparent {id} to {new_parent} (no-op, already there)");
        return Ok((Vec::new(), line));
    }
    // The move kept the world pose; overwrite with the captured local
    // on undo so the restore is bit-exact, not just close.
    let new_local = tree.local(id).ok_or(BatchError::UnknownNode { id })?;
    let inverse = vec![
        Op::Reparent {
            id,
            new_parent: old_parent,
        },
        Op::SetLocal {
            id,
            local: old_local,
        },
    ];
    let line = format!(
        "reparent {id} from {old_parent} to {new_parent} local {}",
        fmt_local(&new_local)
    );
    Ok((inverse, line))
}

/// Stage a material bind or clear; the inverse restores the old binding.
fn stage_set_material(
    tree: &Hierarchy,
    materials: &mut BTreeMap<String, MaterialName>,
    id: ObjectId,
    material: Option<MaterialName>,
) -> Result<(Vec<Op>, String), BatchError> {
    if !tree.contains(id) {
        return Err(BatchError::UnknownNode { id });
    }
    let key = id.to_string();
    let old = materials.get(&key).copied();
    match material {
        Some(name) => {
            materials.insert(key, name);
        }
        None => {
            materials.remove(&key);
        }
    }
    let line = match (old, material) {
        (Some(from), Some(to)) => {
            format!("set-material {id} from `{from}` to `{to}`")
        }
        (None, Some(to)) => format!("set-material {id} to `{to}`"),
        (Some(from), None) => format!("set-material {id} cleared (was `{from}`)"),
        (None, None) => format!("set-material {id} already clear"),
    };
    Ok((vec![Op::SetMaterial { id, material: old }], line))
}

/// Stage a material record put: replace the record of the same name, or
/// append a fresh one.
///
/// A replaced record inverts to a `put` of the previous text; an appended
/// one inverts to [`Op::RemoveLastMaterial`]. The text parses against the
/// live table, so `extends` resolves exactly as the door's push does.
fn stage_put_material(
    material_set: &mut MaterialSet,
    text: &str,
) -> Result<(Vec<Op>, String), BatchError> {
    let record = material_set.parse_against(text).map_err(map_record_error)?;
    let name = record.name.clone();
    match material_set.put(record).map_err(map_record_error)? {
        Some(previous) => {
            let line = format!("put-material `{name}` (replaced)");
            Ok((
                vec![Op::PutMaterial {
                    text: previous.to_text(),
                }],
                line,
            ))
        }
        None => {
            let line = format!("put-material `{name}` (added)");
            Ok((vec![Op::RemoveLastMaterial], line))
        }
    }
}

/// Stage one parameter edit; the inverse carries the old value text, so undo
/// restores the pre-edit value exactly.
fn stage_set_material_param(
    material_set: &mut MaterialSet,
    material: &str,
    path: &str,
    value: &str,
) -> Result<(Vec<Op>, String), BatchError> {
    let edit = material_set
        .set_param(material, path, value)
        .map_err(map_record_error)?;
    let line = format!(
        "set-material-param `{material}` {path} from {} to {}",
        edit.before, edit.after
    );
    let inverse = vec![Op::SetMaterialParam {
        material: edit.material,
        path: edit.path,
        value: edit.before,
    }];
    Ok((inverse, line))
}

/// Stage a drop of the newest material record; the inverse re-puts it.
fn stage_remove_last_material(
    material_set: &mut MaterialSet,
) -> Result<(Vec<Op>, String), BatchError> {
    let removed = material_set
        .remove_last()
        .ok_or_else(|| BatchError::BadMaterialRecord {
            path: "$".to_string(),
            detail: "material table is empty; nothing to remove".to_string(),
        })?;
    let line = format!("remove-last-material `{}`", removed.name);
    let inverse = vec![Op::PutMaterial {
        text: removed.to_text(),
    }];
    Ok((inverse, line))
}

/// Translate a record refusal into the matching batch error, keeping the
/// record path the caller asked about and the reason under it.
fn map_record_error(error: RecordError) -> BatchError {
    BatchError::BadMaterialRecord {
        path: error.path,
        detail: error.kind.to_string(),
    }
}

/// Translate a hierarchy failure into the matching batch error, naming the
/// op's node (and linked parent where the op carries one).
fn map_error(error: HierarchyError, op: &Op) -> BatchError {
    let id = op.id();
    let parent = op.linked_parent();
    match error {
        HierarchyError::UnknownNode => BatchError::UnknownNode { id },
        HierarchyError::UnknownParent => BatchError::UnknownParent { id, parent },
        HierarchyError::Duplicate => BatchError::Duplicate { id },
        HierarchyError::Cycle => BatchError::Cycle {
            id,
            new_parent: parent,
        },
        HierarchyError::NonFinite => BatchError::NonFinite { id },
        HierarchyError::DegenerateScale => BatchError::DegenerateScale { id },
        HierarchyError::NilId => BatchError::NilId,
    }
}

/// Short deterministic pose rendering for previews (meters, radians).
fn fmt_local(local: &Transform) -> String {
    format!(
        "pos=[{:.3},{:.3},{:.3}] rot=[{:.3},{:.3},{:.3},{:.3}] scale=[{:.3},{:.3},{:.3}]",
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn insert_batch(id: ObjectId, parent: ObjectId, x: f32, rev: u64, key: u64) -> Batch {
        Batch {
            ops: vec![Op::Insert {
                id,
                parent,
                local: pose_at(x),
            }],
            base_revision: rev,
            idempotency_key: key,
            provenance: None,
        }
    }

    #[test]
    fn material_names_validate_once_at_the_boundary() {
        assert_eq!(MaterialName::try_from(""), Err(MaterialNameError::Empty));
        assert!(matches!(
            MaterialName::try_from("m".repeat(MAX_MATERIAL_NAME_LEN + 1).as_str()),
            Err(MaterialNameError::TooLong { .. })
        ));
        assert!(matches!(
            MaterialName::try_from("hero mat!"),
            Err(MaterialNameError::BadChar { .. })
        ));
        let name = MaterialName::try_from("hero-mat_01.glb").unwrap();
        assert_eq!(name.as_str(), "hero-mat_01.glb");
        assert_eq!(name.to_string(), "hero-mat_01.glb");
        // Copy-sized name: the binding rides inside an op without allocating.
        fn is_copy<T: Copy>() {}
        is_copy::<MaterialName>();
        // Ops travel by value through the journal, so they clone; the
        // material-table ops own their record text, so they are not `Copy`.
        fn is_clone<T: Clone>() {}
        is_clone::<Op>();
    }

    #[test]
    fn set_material_binds_undoes_redoes_and_refuses_unknown_nodes() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let node = ObjectId::new();
        apply(
            &mut doc,
            &mut journal,
            &insert_batch(node, ObjectId::NIL, 0.0, 0, 1),
        )
        .unwrap();
        let hero = MaterialName::try_from("hero-mat").unwrap();
        let stone = MaterialName::try_from("stone").unwrap();

        // Unknown nodes refuse and write nothing (revision holds).
        let ghost = ObjectId::new();
        let rev = doc.revision;
        let refused = apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::SetMaterial {
                    id: ghost,
                    material: Some(hero),
                }],
                base_revision: rev,
                idempotency_key: 2,
                provenance: None,
            },
        );
        assert_eq!(refused, Err(BatchError::UnknownNode { id: ghost }));
        assert_eq!(doc.revision, rev);
        assert!(doc.materials.is_empty());

        // Bind, rebind, clear: each step journals its own inverse.
        for (key, material, want) in [
            (3u64, Some(hero), Some(hero)),
            (4u64, Some(stone), Some(stone)),
            (5u64, None, None),
        ] {
            let rev = doc.revision;
            let applied = apply(
                &mut doc,
                &mut journal,
                &Batch {
                    ops: vec![Op::SetMaterial { id: node, material }],
                    base_revision: rev,
                    idempotency_key: key,
                    provenance: None,
                },
            )
            .unwrap();
            assert_eq!(applied.revision, rev + 1);
            assert_eq!(doc.materials.get(&node.to_string()).copied(), want);
        }
        // Undo restores stone, redo clears again: bindings undo like poses.
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.materials.get(&node.to_string()).copied(), Some(stone));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.materials.get(&node.to_string()).copied(), Some(hero));
        redo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.materials.get(&node.to_string()).copied(), Some(stone));

        // Removing the node drops its binding; undo brings both back.
        let rev = doc.revision;
        apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Remove { id: node }],
                base_revision: rev,
                idempotency_key: 6,
                provenance: None,
            },
        )
        .unwrap();
        assert!(!doc.materials.contains_key(&node.to_string()));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.materials.get(&node.to_string()).copied(), Some(stone));
    }

    #[test]
    fn hundred_edits_undo_redo_restore_exact_document() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let initial = doc.clone();
        // Chain each node under the previous one so every batch is valid.
        let mut parent = ObjectId::NIL;
        for i in 0..100u64 {
            let id = ObjectId::new();
            let batch = insert_batch(id, parent, i as f32, doc.revision, 1000 + i);
            let applied = apply(&mut doc, &mut journal, &batch).unwrap();
            assert_eq!(applied.revision, doc.revision);
            parent = id;
        }
        assert_eq!(doc.revision, 100);
        assert_eq!(journal.past_len(), 100);
        let snapshot = doc.clone();

        for _ in 0..100 {
            undo(&mut doc, &mut journal).expect("undo stack holds 100 batches");
        }
        assert_eq!(doc.hierarchy, initial.hierarchy);
        // Undo publishes new revisions; old numbers are never reused.
        assert_eq!(doc.revision, 200);
        assert!(journal.past.is_empty());
        assert_eq!(journal.future_len(), 100);

        for _ in 0..100 {
            redo(&mut doc, &mut journal).expect("redo stack holds 100 batches");
        }
        assert_eq!(doc.hierarchy, snapshot.hierarchy);
        assert_eq!(doc.revision, 300);
    }

    #[test]
    fn batch_prepared_before_undo_and_new_edit_stays_stale() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let first = insert_batch(ObjectId::new(), ObjectId::NIL, 1.0, doc.revision, 1);
        apply(&mut doc, &mut journal, &first).unwrap();
        // A slow client prepares an edit against the state that includes `first`.
        let late = insert_batch(ObjectId::new(), ObjectId::NIL, 2.0, doc.revision, 2);
        undo(&mut doc, &mut journal).unwrap();
        let other = insert_batch(ObjectId::new(), ObjectId::NIL, 3.0, doc.revision, 3);
        apply(&mut doc, &mut journal, &other).unwrap();
        let before = doc.clone();
        assert!(matches!(
            apply(&mut doc, &mut journal, &late),
            Err(BatchError::StaleRevision { .. })
        ));
        assert_eq!(doc, before);
    }

    #[test]
    fn retrying_an_undone_batch_is_refused_not_reported_applied() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let batch = insert_batch(ObjectId::new(), ObjectId::NIL, 1.0, doc.revision, 9);
        apply(&mut doc, &mut journal, &batch).unwrap();
        undo(&mut doc, &mut journal).unwrap();
        let before = doc.clone();
        assert_eq!(
            apply(&mut doc, &mut journal, &batch),
            Err(BatchError::UndoneKey { key: 9 })
        );
        assert_eq!(doc, before);
        // Redo still restores it through the journal.
        assert!(redo(&mut doc, &mut journal).is_some());
        assert_eq!(doc.hierarchy.len(), 1);
    }

    #[test]
    fn stale_revision_batch_changes_nothing() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let batch = insert_batch(id, ObjectId::NIL, 1.0, doc.revision, 1);
        apply(&mut doc, &mut journal, &batch).unwrap();
        let snapshot = doc.clone();
        assert_eq!(doc.revision, 1);

        let stale = Batch {
            ops: vec![Op::SetLocal {
                id,
                local: pose_at(9.0),
            }],
            base_revision: 0,
            idempotency_key: 2,
            provenance: None,
        };
        assert_eq!(
            apply(&mut doc, &mut journal, &stale),
            Err(BatchError::StaleRevision {
                expected: 1,
                got: 0
            })
        );
        assert_eq!(
            validate(&doc, &stale),
            Err(BatchError::StaleRevision {
                expected: 1,
                got: 0
            })
        );
        assert_eq!(doc, snapshot);
        assert_eq!(journal.past_len(), 1);
    }

    #[test]
    fn partial_batch_changes_nothing() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let kept = ObjectId::new();
        let seeded = insert_batch(kept, ObjectId::NIL, 0.0, doc.revision, 11);
        apply(&mut doc, &mut journal, &seeded).unwrap();
        let snapshot = doc.clone();

        // Valid op first, unknown node second: the valid one must not stick.
        let ghost = ObjectId::new();
        let partial = Batch {
            ops: vec![
                Op::SetLocal {
                    id: kept,
                    local: pose_at(5.0),
                },
                Op::Remove { id: ghost },
            ],
            base_revision: doc.revision,
            idempotency_key: 12,
            provenance: None,
        };
        assert_eq!(
            apply(&mut doc, &mut journal, &partial),
            Err(BatchError::UnknownNode { id: ghost })
        );
        assert_eq!(doc, snapshot);

        // Two inserts sharing one fresh id: the first must not stick either.
        let clash = ObjectId::new();
        let doubled = Batch {
            ops: vec![
                Op::Insert {
                    id: clash,
                    parent: ObjectId::NIL,
                    local: pose_at(1.0),
                },
                Op::Insert {
                    id: clash,
                    parent: ObjectId::NIL,
                    local: pose_at(2.0),
                },
            ],
            base_revision: doc.revision,
            idempotency_key: 13,
            provenance: None,
        };
        assert_eq!(
            apply(&mut doc, &mut journal, &doubled),
            Err(BatchError::Duplicate { id: clash })
        );
        assert_eq!(doc, snapshot);
        assert_eq!(journal.past_len(), 1);
    }

    #[test]
    fn duplicate_key_replays_without_reapplying() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let batch = insert_batch(id, ObjectId::NIL, 3.0, doc.revision, 7);
        let first = apply(&mut doc, &mut journal, &batch).unwrap();
        let snapshot = doc.clone();

        // Same key plus same payload replays the stored receipt.
        let replayed = apply(&mut doc, &mut journal, &batch).unwrap();
        assert_eq!(replayed, first);
        assert_eq!(doc, snapshot);
        assert_eq!(doc.revision, 1);
        assert_eq!(journal.past_len(), 1);

        // Same key with another payload is a conflict and writes nothing.
        let conflict = Batch {
            ops: vec![Op::SetLocal {
                id,
                local: pose_at(8.0),
            }],
            base_revision: doc.revision,
            idempotency_key: 7,
            provenance: None,
        };
        assert_eq!(
            apply(&mut doc, &mut journal, &conflict),
            Err(BatchError::DuplicateKey { key: 7 })
        );
        assert_eq!(doc, snapshot);
    }

    #[test]
    fn preview_reports_without_mutating() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let seeded = insert_batch(id, ObjectId::NIL, 0.0, doc.revision, 21);
        apply(&mut doc, &mut journal, &seeded).unwrap();
        let snapshot = doc.clone();

        let batch = Batch {
            ops: vec![
                Op::SetLocal {
                    id,
                    local: pose_at(4.0),
                },
                Op::Reparent {
                    id,
                    new_parent: ObjectId::NIL,
                },
            ],
            base_revision: doc.revision,
            idempotency_key: 22,
            provenance: None,
        };
        let preview = validate(&doc, &batch).unwrap();
        assert_eq!(preview.effects.len(), 2);
        assert_eq!(preview.resulting_revision, doc.revision + 1);
        assert!(preview.effects[0].contains(&id.to_string()));
        assert_eq!(doc, snapshot);
        assert!(journal.is_empty() || journal.past_len() == 1);
    }

    #[test]
    fn remove_undo_redo_restore_children_exactly() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let root = ObjectId::new();
        let mid = ObjectId::new();
        let leaf = ObjectId::new();
        for (i, (id, parent)) in [(root, ObjectId::NIL), (mid, root), (leaf, mid)]
            .into_iter()
            .enumerate()
        {
            let batch = insert_batch(id, parent, i as f32, doc.revision, 30 + i as u64);
            apply(&mut doc, &mut journal, &batch).unwrap();
        }
        let before = doc.clone();

        // Removing the middle relinks the leaf to the root.
        let cut = Batch {
            ops: vec![Op::Remove { id: mid }],
            base_revision: doc.revision,
            idempotency_key: 40,
            provenance: None,
        };
        let preview = validate(&doc, &cut).unwrap();
        assert!(preview.effects[0].contains("1 children"));
        apply(&mut doc, &mut journal, &cut).unwrap();
        assert_eq!(doc.hierarchy.parent(leaf), Some(root));
        let removed = doc.clone();

        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before.hierarchy);
        redo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, removed.hierarchy);
    }

    #[test]
    fn reparent_undo_restores_local_exactly() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        let child = ObjectId::new();
        for (i, (id, parent)) in [(a, ObjectId::NIL), (b, ObjectId::NIL), (child, a)]
            .into_iter()
            .enumerate()
        {
            let batch = insert_batch(id, parent, i as f32, doc.revision, 50 + i as u64);
            apply(&mut doc, &mut journal, &batch).unwrap();
        }
        let before = doc.clone();

        let hop = Batch {
            ops: vec![Op::Reparent {
                id: child,
                new_parent: b,
            }],
            base_revision: doc.revision,
            idempotency_key: 60,
            provenance: None,
        };
        apply(&mut doc, &mut journal, &hop).unwrap();
        assert_eq!(doc.hierarchy.parent(child), Some(b));
        // World pose is preserved by the move.
        assert_eq!(doc.hierarchy.world(child), before.hierarchy.world(child));
        let moved = doc.clone();

        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before.hierarchy);
        redo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, moved.hierarchy);
    }

    #[test]
    fn hierarchy_failures_map_to_culprits() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let root = ObjectId::new();
        let seeded = insert_batch(root, ObjectId::NIL, 0.0, doc.revision, 70);
        apply(&mut doc, &mut journal, &seeded).unwrap();
        let ghost = ObjectId::new();

        fn check(doc: &Document, batch: &Batch) -> BatchError {
            validate(doc, batch).unwrap_err()
        }

        let missing = Batch {
            ops: vec![Op::Remove { id: ghost }],
            base_revision: doc.revision,
            idempotency_key: 71,
            provenance: None,
        };
        assert_eq!(check(&doc, &missing), BatchError::UnknownNode { id: ghost });

        let dangling = Batch {
            ops: vec![Op::Insert {
                id: ObjectId::new(),
                parent: ghost,
                local: pose_at(0.0),
            }],
            base_revision: doc.revision,
            idempotency_key: 72,
            provenance: None,
        };
        assert!(matches!(
            check(&doc, &dangling),
            BatchError::UnknownParent { .. }
        ));

        let dupe = insert_batch(root, ObjectId::NIL, 0.0, doc.revision, 73);
        assert_eq!(check(&doc, &dupe), BatchError::Duplicate { id: root });

        let child = ObjectId::new();
        let grown = insert_batch(child, root, 1.0, doc.revision, 74);
        apply(&mut doc, &mut journal, &grown).unwrap();
        let looping = Batch {
            ops: vec![Op::Reparent {
                id: root,
                new_parent: child,
            }],
            base_revision: doc.revision,
            idempotency_key: 75,
            provenance: None,
        };
        assert_eq!(
            check(&doc, &looping),
            BatchError::Cycle {
                id: root,
                new_parent: child
            }
        );

        let nan = Batch {
            ops: vec![Op::SetLocal {
                id: root,
                local: Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }],
            base_revision: doc.revision,
            idempotency_key: 76,
            provenance: None,
        };
        assert_eq!(check(&doc, &nan), BatchError::NonFinite { id: root });

        // Zero scale is finite so it inserts, but nothing can move under it.
        let flat = ObjectId::new();
        let squashed = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 1.0]);
        let flat_batch = Batch {
            ops: vec![Op::Insert {
                id: flat,
                parent: ObjectId::NIL,
                local: squashed,
            }],
            base_revision: doc.revision,
            idempotency_key: 77,
            provenance: None,
        };
        apply(&mut doc, &mut journal, &flat_batch).unwrap();
        let sink = Batch {
            ops: vec![Op::Reparent {
                id: child,
                new_parent: flat,
            }],
            base_revision: doc.revision,
            idempotency_key: 78,
            provenance: None,
        };
        assert_eq!(
            check(&doc, &sink),
            BatchError::DegenerateScale { id: child }
        );

        let nil = Batch {
            ops: vec![Op::Insert {
                id: ObjectId::NIL,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            base_revision: doc.revision,
            idempotency_key: 79,
            provenance: None,
        };
        assert_eq!(check(&doc, &nil), BatchError::NilId);

        // Diagnostics quote the culprit.
        let report = BatchError::UnknownNode { id: ghost }.to_string();
        assert!(report.contains(&ghost.to_string()));
    }

    #[test]
    fn journal_caps_at_100_and_new_apply_clears_redo() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut parent = ObjectId::NIL;
        for i in 0..120u64 {
            let id = ObjectId::new();
            let batch = insert_batch(id, parent, 0.0, doc.revision, 200 + i);
            apply(&mut doc, &mut journal, &batch).unwrap();
            parent = id;
        }
        assert_eq!(doc.revision, 120);
        assert_eq!(journal.past_len(), JOURNAL_CAP);

        // The oldest keys fell off, so key 200 is free again.
        let fresh = insert_batch(ObjectId::new(), parent, 0.0, doc.revision, 200);
        assert!(apply(&mut doc, &mut journal, &fresh).is_ok());
        assert_eq!(journal.past_len(), JOURNAL_CAP);

        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(journal.future_len(), 1);
        let extra = insert_batch(ObjectId::new(), ObjectId::NIL, 0.0, doc.revision, 999);
        apply(&mut doc, &mut journal, &extra).unwrap();
        assert_eq!(journal.future_len(), 0);
        assert!(redo(&mut doc, &mut journal).is_none());
    }

    #[test]
    fn empty_stacks_undo_redo_to_none() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        assert!(journal.is_empty());
        assert!(undo(&mut doc, &mut journal).is_none());
        assert!(redo(&mut doc, &mut journal).is_none());
        assert_eq!(doc, Document::new());
    }

    #[test]
    fn model_stamped_batch_preserves_provider_and_model_ids() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let digest = [0xABu8; 32];
        let batch = Batch {
            ops: vec![Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: pose_at(1.0),
            }],
            base_revision: doc.revision,
            idempotency_key: 501,
            provenance: Some(ProvenanceStamp::model(
                "mock-provider",
                "mock-model-7",
                digest,
            )),
        };
        let applied = apply(&mut doc, &mut journal, &batch).unwrap();
        assert_eq!(applied.batch_id(), 501);
        assert_eq!(applied.journal_index, 0);
        match &applied.provenance.source {
            engine_scene::provenance::Source::Model { provider, model } => {
                assert_eq!(provider, "mock-provider");
                assert_eq!(model, "mock-model-7");
            }
            other => panic!("expected model source, got {other:?}"),
        }
        assert_eq!(applied.provenance.request_digest, digest);
        // Binary leg: the receipt is already the decoded form of its encoding.
        let bytes = applied.provenance.encode().unwrap();
        assert_eq!(Provenance::decode(&bytes).unwrap(), applied.provenance);
        // JSON leg (engine-scene `json` feature, test-only dev-dependency).
        let text = engine_scene::provenance::json::encode(&applied.provenance).unwrap();
        assert_eq!(
            engine_scene::provenance::json::decode(&text).unwrap(),
            applied.provenance
        );
        // Retry replays the identical receipt, stamp included.
        let replayed = apply(&mut doc, &mut journal, &batch).unwrap();
        assert_eq!(replayed, applied);
    }

    #[test]
    fn human_default_carries_zero_digest() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        // Old call sites pass no stamp: human source, zero digest.
        let id = ObjectId::new();
        let plain = Batch::new(
            vec![Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            doc.revision,
            502,
        );
        assert_eq!(plain.provenance, None);
        let applied = apply(&mut doc, &mut journal, &plain).unwrap();
        assert_eq!(
            applied.provenance.source,
            engine_scene::provenance::Source::Human
        );
        assert_eq!(applied.provenance.request_digest, [0u8; 32]);
        assert_eq!(ProvenanceStamp::default(), ProvenanceStamp::human());
        // Script stamps map with a zero digest too.
        let scripted = Batch::new(
            vec![Op::SetLocal {
                id,
                local: pose_at(2.0),
            }],
            doc.revision,
            503,
        )
        .with_stamp(ProvenanceStamp::script("auto-layout"));
        let applied = apply(&mut doc, &mut journal, &scripted).unwrap();
        assert_eq!(
            applied.provenance.source,
            engine_scene::provenance::Source::Script("auto-layout".into())
        );
        assert_eq!(applied.provenance.request_digest, [0u8; 32]);
    }

    #[test]
    fn chains_link_parents_and_roundtrip_both_codecs() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let root = ObjectId::new();
        let leaf = ObjectId::new();
        let first = insert_batch(root, ObjectId::NIL, 0.0, doc.revision, 601);
        let first_receipt = apply(&mut doc, &mut journal, &first).unwrap();
        let second = Batch {
            ops: vec![
                Op::Insert {
                    id: leaf,
                    parent: root,
                    local: pose_at(1.0),
                },
                Op::SetLocal {
                    id: root,
                    local: pose_at(5.0),
                },
            ],
            base_revision: doc.revision,
            idempotency_key: 602,
            provenance: Some(ProvenanceStamp::model("p", "m", [3u8; 32])),
        };
        let second_receipt = apply(&mut doc, &mut journal, &second).unwrap();
        // The second record's parents name the first receipt, and the index
        // tracks the journal position.
        assert_eq!(
            second_receipt.provenance.parents,
            vec![first_receipt.provenance.receipt()]
        );
        assert_eq!(second_receipt.journal_index, 1);
        // Per-object chains: root links across both batches, the leaf holds
        // only its own insert, unknown objects read empty.
        assert_eq!(
            chain_for(&journal, root),
            vec![
                first_receipt.provenance.clone(),
                second_receipt.provenance.clone()
            ]
        );
        assert_eq!(
            chain_for(&journal, leaf),
            vec![second_receipt.provenance.clone()]
        );
        assert!(chain_for(&journal, ObjectId::new()).is_empty());
        // Every receipt and chain record survives both provenance codec legs.
        for record in [
            first_receipt.provenance.clone(),
            second_receipt.provenance.clone(),
        ] {
            let bytes = record.encode().unwrap();
            assert_eq!(Provenance::decode(&bytes).unwrap(), record);
            let text = engine_scene::provenance::json::encode(&record).unwrap();
            assert_eq!(
                engine_scene::provenance::json::decode(&text).unwrap(),
                record
            );
        }
        // Verifier rows ride the same codec legs (the carried record type
        // already holds them).
        let mut with_checks = second_receipt.provenance.clone();
        with_checks
            .verifiers
            .push(engine_scene::provenance::VerifierResult {
                verifier: "spatial".into(),
                passed: true,
                detail: "no overlap".into(),
            });
        let bytes = with_checks.encode().unwrap();
        assert_eq!(Provenance::decode(&bytes).unwrap(), with_checks);
        let text = engine_scene::provenance::json::encode(&with_checks).unwrap();
        assert_eq!(
            engine_scene::provenance::json::decode(&text).unwrap(),
            with_checks
        );
        // Undo lifts the batch off the chain; redo restores the same record.
        let undone = undo(&mut doc, &mut journal).unwrap();
        assert_eq!(undone.provenance, second_receipt.provenance);
        assert_eq!(undone.journal_index, 0);
        assert!(chain_for(&journal, leaf).is_empty());
        let redone = redo(&mut doc, &mut journal).unwrap();
        assert_eq!(redone.provenance, second_receipt.provenance);
        assert_eq!(redone.journal_index, 1);
        assert_eq!(
            chain_for(&journal, leaf),
            vec![second_receipt.provenance.clone()]
        );
    }

    #[test]
    fn journal_replay_rebuilds_identical_chains() {
        let node_a = ObjectId::new();
        let node_b = ObjectId::new();
        let batches = vec![
            Batch::new(
                vec![Op::Insert {
                    id: node_a,
                    parent: ObjectId::NIL,
                    local: pose_at(0.0),
                }],
                0,
                701,
            )
            .with_stamp(ProvenanceStamp::human()),
            Batch::new(
                vec![Op::Insert {
                    id: node_b,
                    parent: node_a,
                    local: pose_at(1.0),
                }],
                1,
                702,
            )
            .with_stamp(ProvenanceStamp::model("mock", "m1", [9u8; 32])),
            Batch::new(
                vec![Op::SetLocal {
                    id: node_a,
                    local: pose_at(4.0),
                }],
                2,
                703,
            ),
        ];
        let run = |batches: &[Batch]| {
            let mut doc = Document::new();
            let mut journal = Journal::new();
            let mut receipts = Vec::new();
            for batch in batches {
                receipts.push(apply(&mut doc, &mut journal, batch).unwrap());
            }
            let chains = [node_a, node_b].map(|id| chain_for(&journal, id));
            (doc, receipts, chains)
        };
        let (doc_a, receipts_a, chains_a) = run(&batches);
        let (doc_b, receipts_b, chains_b) = run(&batches);
        assert_eq!(doc_a, doc_b);
        assert_eq!(receipts_a, receipts_b);
        assert_eq!(chains_a, chains_b);
        // Chains match the receipts that built them, oldest first.
        assert_eq!(
            chains_a[0],
            vec![
                receipts_a[0].provenance.clone(),
                receipts_a[2].provenance.clone()
            ]
        );
        assert_eq!(chains_a[1], vec![receipts_a[1].provenance.clone()]);
    }

    #[test]
    fn overlong_names_truncate_instead_of_failing() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let long = "p".repeat(300);
        let batch = Batch::new(
            vec![Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            doc.revision,
            801,
        )
        .with_stamp(ProvenanceStamp::model(
            long.clone(),
            long.clone(),
            [0u8; 32],
        ));
        // Provenance display bounds never reject an otherwise-valid edit.
        let applied = apply(&mut doc, &mut journal, &batch).unwrap();
        match &applied.provenance.source {
            engine_scene::provenance::Source::Model { provider, model } => {
                assert_eq!(provider.len(), engine_scene::provenance::MAX_TEXT_LEN);
                assert_eq!(model.len(), engine_scene::provenance::MAX_TEXT_LEN);
            }
            other => panic!("expected model source, got {other:?}"),
        }
        let bytes = applied.provenance.encode().unwrap();
        assert_eq!(Provenance::decode(&bytes).unwrap(), applied.provenance);
    }

    #[test]
    fn no_secret_like_field_exists() {
        // Compile-time API-shape assertion: these patterns are exhaustive
        // (no `..`), so adding ANY field — a raw prompt, credential, session
        // secret or otherwise — fails to compile here until reviewed. The
        // only provenance inputs are source identity plus opaque digests,
        // which is what makes "construct with prompts impossible" true.
        let stamp = ProvenanceStamp::model("provider", "model", [1u8; 32]);
        let ProvenanceStamp {
            source,
            request_digest,
        } = stamp.clone();
        assert_eq!(request_digest, [1u8; 32]);
        match source {
            ReceiptSource::Human => panic!("expected the model stamp"),
            ReceiptSource::Script { name } => {
                panic!("expected the model stamp, got script {name}")
            }
            ReceiptSource::Model { provider, model } => {
                assert_eq!(provider, "provider");
                assert_eq!(model, "model");
            }
        }
        let batch = Batch::new(Vec::new(), 0, 0).with_stamp(stamp);
        let Batch {
            ops,
            base_revision,
            idempotency_key,
            provenance,
        } = batch;
        assert!(ops.is_empty() && base_revision == 0 && idempotency_key == 0);
        assert!(provenance.is_some());
        // Digests are opaque bytes, never rendered secrets: the stamp's debug
        // form carries no prompt or credential text because no such field
        // exists to render.
        let rendered = format!("{provenance:?}");
        assert!(!rendered.contains("prompt"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("credential"));
    }

    fn selective_apply(doc: &mut Document, journal: &mut Journal, ops: Vec<Op>, key: u64) -> u64 {
        let batch = Batch::new(ops, doc.revision, key);
        let applied = apply(doc, journal, &batch).unwrap();
        applied.provenance.receipt()
    }

    #[test]
    fn selective_revert_restores_exact_bytes() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        let receipt_b = selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: b,
                parent: ObjectId::NIL,
                local: pose_at(1.0),
            }],
            2,
        );
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::SetLocal {
                id: a,
                local: pose_at(5.0),
            }],
            3,
        );
        // The later batch touches only `a`, so reverting `b` is clean.
        let preview = undo_select(&journal, &[receipt_b]).unwrap();
        assert!(preview.conflicts.is_empty());
        assert_eq!(preview.reverted_ops, vec![Op::Remove { id: b }]);
        let before = doc.clone();
        let applied = apply_selective_undo(&mut doc, &mut journal, &[receipt_b], 10, None).unwrap();
        assert_eq!(doc.revision, before.revision + 1);
        assert!(!doc.hierarchy.contains(b));
        assert_eq!(doc.hierarchy.local(a), Some(pose_at(5.0)));
        // Exact bytes: matches a fresh replay of insert-a plus set-local-a.
        let mut replay = Document::new();
        let mut replay_journal = Journal::new();
        selective_apply(
            &mut replay,
            &mut replay_journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        selective_apply(
            &mut replay,
            &mut replay_journal,
            vec![Op::SetLocal {
                id: a,
                local: pose_at(5.0),
            }],
            3,
        );
        assert_eq!(doc.hierarchy, replay.hierarchy);
        // The selective batch is itself undoable with its own receipt.
        assert_eq!(applied.provenance.parents.len(), 1);
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before.hierarchy);
    }

    #[test]
    fn selective_overwrite_reports_conflict_and_leaves_value() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        let first = selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::SetLocal {
                id: a,
                local: pose_at(1.0),
            }],
            2,
        );
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::SetLocal {
                id: a,
                local: pose_at(2.0),
            }],
            3,
        );
        let preview = undo_select(&journal, &[first]).unwrap();
        assert!(preview.reverted_ops.is_empty());
        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(
            preview.conflicts[0],
            Conflict {
                op: Op::SetLocal {
                    id: a,
                    local: pose_at(1.0),
                },
                reason: ConflictReason::OverwrittenByLater,
            }
        );
        // The pinned value stays: the attempt journals a no-op batch only.
        let before = doc.hierarchy.clone();
        apply_selective_undo(&mut doc, &mut journal, &[first], 10, None).unwrap();
        assert_eq!(doc.hierarchy, before);
        assert_eq!(doc.hierarchy.local(a), Some(pose_at(2.0)));
    }

    #[test]
    fn selective_dependent_reports_conflict() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let x = ObjectId::new();
        let y = ObjectId::new();
        let receipt_x = selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: x,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: y,
                parent: x,
                local: pose_at(1.0),
            }],
            2,
        );
        // The child assumed `x` where it is, so the insert stays pinned.
        let preview = undo_select(&journal, &[receipt_x]).unwrap();
        assert!(preview.reverted_ops.is_empty());
        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(preview.conflicts[0].reason, ConflictReason::DependsOnLater);
        assert_eq!(
            preview.conflicts[0].op,
            Op::Insert {
                id: x,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }
        );
        let before = doc.hierarchy.clone();
        apply_selective_undo(&mut doc, &mut journal, &[receipt_x], 10, None).unwrap();
        assert_eq!(doc.hierarchy, before);
        assert!(doc.hierarchy.contains(x) && doc.hierarchy.contains(y));
    }

    #[test]
    fn selective_multi_op_batch_propagates_to_batchmates() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        // One batch inserts `a` then `b` under it; later work touches `b`.
        let batch = Batch::new(
            vec![
                Op::Insert {
                    id: a,
                    parent: ObjectId::NIL,
                    local: pose_at(0.0),
                },
                Op::Insert {
                    id: b,
                    parent: a,
                    local: pose_at(1.0),
                },
            ],
            doc.revision,
            1,
        );
        let receipt = apply(&mut doc, &mut journal, &batch)
            .unwrap()
            .provenance
            .receipt();
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::SetLocal {
                id: b,
                local: pose_at(9.0),
            }],
            2,
        );
        // `b` is pinned directly; `a` below it is pinned transitively, since
        // its inverse was staged before `b` existed.
        let preview = undo_select(&journal, &[receipt]).unwrap();
        assert!(preview.reverted_ops.is_empty());
        assert_eq!(preview.conflicts.len(), 2);
        assert!(preview
            .conflicts
            .iter()
            .all(|conflict| conflict.reason == ConflictReason::DependsOnLater));
        let before = doc.hierarchy.clone();
        apply_selective_undo(&mut doc, &mut journal, &[receipt], 10, None).unwrap();
        assert_eq!(doc.hierarchy, before);
    }

    #[test]
    fn selective_subset_reverts_only_clean_batches() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        let receipt_a = selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        let receipt_b = selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: b,
                parent: ObjectId::NIL,
                local: pose_at(1.0),
            }],
            2,
        );
        // Later work overwrites `a` only; `b` stays revertible.
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::SetLocal {
                id: a,
                local: pose_at(9.0),
            }],
            3,
        );
        let preview = undo_select(&journal, &[receipt_a, receipt_b]).unwrap();
        assert_eq!(preview.reverted_ops, vec![Op::Remove { id: b }]);
        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(preview.conflicts[0].reason, ConflictReason::DependsOnLater);
        apply_selective_undo(&mut doc, &mut journal, &[receipt_a, receipt_b], 10, None).unwrap();
        assert!(!doc.hierarchy.contains(b));
        assert_eq!(doc.hierarchy.local(a), Some(pose_at(9.0)));
    }

    #[test]
    fn empty_selection_is_a_noop_batch() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        let preview = undo_select(&journal, &[]).unwrap();
        assert!(preview.reverted_ops.is_empty());
        assert!(preview.conflicts.is_empty());
        let before = doc.clone();
        let applied = apply_selective_undo(&mut doc, &mut journal, &[], 10, None).unwrap();
        assert_eq!(doc.hierarchy, before.hierarchy);
        assert_eq!(doc.revision, before.revision + 1);
        assert!(applied.effects.is_empty());
        assert_eq!(journal.past_len(), 2);
        // The no-op batch itself undoes cleanly.
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before.hierarchy);
    }

    #[test]
    fn unknown_receipt_writes_nothing() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            1,
        );
        let before = (doc.clone(), journal.past_len());
        assert_eq!(
            undo_select(&journal, &[0xDEAD_BEEF]),
            Err(UndoError::UnknownReceipt {
                receipt: 0xDEAD_BEEF
            })
        );
        assert_eq!(
            apply_selective_undo(&mut doc, &mut journal, &[0xDEAD_BEEF], 10, None),
            Err(UndoError::UnknownReceipt {
                receipt: 0xDEAD_BEEF
            })
        );
        assert_eq!(doc, before.0);
        assert_eq!(journal.past_len(), before.1);
    }

    // Grouped commands: gizmo plus inspector call sites, durability
    // skips, nesting, empty groups and redo-after-new-edit.

    fn drag_step_batch(id: ObjectId, x: f32, rev: u64, key: u64) -> Batch {
        Batch {
            ops: vec![Op::SetLocal {
                id,
                local: pose_at(x),
            }],
            base_revision: rev,
            idempotency_key: key,
            provenance: None,
        }
    }

    fn seed_single_node(doc: &mut Document, journal: &mut Journal) -> ObjectId {
        let id = ObjectId::new();
        apply(
            doc,
            journal,
            &insert_batch(id, ObjectId::NIL, 0.0, doc.revision, 9000),
        )
        .unwrap();
        id
    }

    #[test]
    fn grouped_gizmo_drag_is_one_undo_step_with_fewer_entries() {
        const STEPS: u64 = 20;
        // Reference: one durable entry per drag tick. Both sides seed the
        // same fixture node id, so "same end digest" compares content:
        // random ids would differ by construction.
        let mut plain_doc = Document::new();
        let mut plain_journal = Journal::new();
        let node =
            ObjectId::parse("11111111-2222-3333-4444-555555555555").expect("fixture id parses");
        let seed = insert_batch(node, ObjectId::NIL, 0.0, plain_doc.revision, 9000);
        apply(&mut plain_doc, &mut plain_journal, &seed).unwrap();
        for i in 1..=STEPS {
            let batch = drag_step_batch(node, i as f32, plain_doc.revision, 9100 + i);
            apply(&mut plain_doc, &mut plain_journal, &batch).unwrap();
        }
        assert_eq!(plain_journal.past_len(), STEPS as usize + 1);

        // Grouped: insignificant steps, one durable commit.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = node;
        let seed = insert_batch(id, ObjectId::NIL, 0.0, doc.revision, 9000);
        apply(&mut doc, &mut journal, &seed).unwrap();
        let start = doc.hierarchy.clone();
        let (base_past, base_durable) = (journal.past_len(), durable_count(&journal));
        let step = CommandMeta::insignificant("gizmo drag step");
        let commit = CommandMeta::significant("gizmo drag");
        let group = begin_group(&mut journal, &commit, 9200);
        assert_ne!(group, NO_GROUP);
        for i in 1..=STEPS {
            let batch = drag_step_batch(id, i as f32, doc.revision, 9201 + i);
            apply_named(&mut doc, &mut journal, &batch, &step).unwrap();
        }
        // Mid-drag every tick is undoable but seals nothing durable.
        assert_eq!(journal.past_len(), base_past + STEPS as usize);
        assert_eq!(durable_count(&journal), base_durable);
        let merged = end_group(&mut journal).expect("drag commits one entry");
        assert_eq!(merged.key, 9200);
        assert_eq!(journal.past_len(), base_past + 1);
        assert_eq!(durable_count(&journal), base_durable + 1);
        let entry = journal.past.last().expect("merged entry");
        assert_eq!(entry.name, "gizmo drag");
        assert!(entry.significant);
        assert_eq!(entry.group, NO_GROUP);
        // Fewer journal entries for the same drag, past the 80% bar.
        let added = journal.past_len() - base_past;
        let saved_percent = (STEPS as usize - added) * 100 / STEPS as usize;
        assert!(
            saved_percent >= 80,
            "grouped drag saved {saved_percent}% of entries"
        );
        // Same end digest as the ungrouped reference.
        assert_eq!(doc.hierarchy, plain_doc.hierarchy);
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(STEPS as f32)));
        // One undo restores the drag start; redo replays it exactly.
        undo(&mut doc, &mut journal).expect("drag undoes in one step");
        assert_eq!(doc.hierarchy, start);
        redo(&mut doc, &mut journal).expect("drag redoes in one step");
        assert_eq!(doc.hierarchy, plain_doc.hierarchy);
    }

    #[test]
    fn gizmo_call_site_routes_through_grouped_batches() {
        use crate::gizmo::{drag_batch, drag_to_local, GizmoDrag};

        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let before = doc.hierarchy.clone();
        let step = CommandMeta::insignificant("gizmo drag step");
        begin_group(&mut journal, &CommandMeta::significant("gizmo drag"), 9300);
        // Ten viewport ticks: parent-frame deltas staged exactly like
        // `Shell::stage_gizmo_drag`, committed here through the group.
        for i in 0..10u64 {
            let current = doc.hierarchy.local(id).expect("selection lives");
            let drag = GizmoDrag::Translate {
                delta: [0.5, 0.0, 0.0],
            };
            let posed = drag_to_local(current, &drag).expect("finite delta");
            // The gizmo helper shapes the same single-op batch the shell
            // validates; the group only adds the name and the flag.
            let shaped = drag_batch(id, posed, doc.revision, 9301 + i);
            let manual = drag_step_batch(id, posed.position[0], doc.revision, 9301 + i);
            assert_eq!(shaped.ops, manual.ops);
            assert_eq!(shaped.base_revision, manual.base_revision);
            apply_named(&mut doc, &mut journal, &shaped, &step).unwrap();
        }
        // Plus one ring-handle spin in the same drag.
        let current = doc.hierarchy.local(id).expect("selection lives");
        let spun = drag_to_local(
            current,
            &GizmoDrag::Rotate {
                axis: [0.0, 1.0, 0.0],
                angle_rad: 0.25,
            },
        )
        .expect("valid spin");
        let batch = drag_batch(id, spun, doc.revision, 9412);
        apply_named(&mut doc, &mut journal, &batch, &step).unwrap();
        end_group(&mut journal).expect("gizmo drag commits");
        assert_eq!(journal.past_len(), 2);
        assert_eq!(durable_count(&journal), 2);
        let landed = doc.hierarchy.local(id).expect("selection lives");
        assert!((landed.position[0] - 5.0).abs() <= 1e-5);
        undo(&mut doc, &mut journal).expect("drag undoes in one step");
        assert_eq!(doc.hierarchy, before);
        redo(&mut doc, &mut journal).expect("drag redoes in one step");
        let back = doc.hierarchy.local(id).expect("selection lives");
        assert!((back.position[0] - 5.0).abs() <= 1e-5);
        assert_eq!(back.rotation, landed.rotation);
    }

    #[test]
    fn inspector_call_site_routes_through_grouped_batches() {
        // Three staged field edits (pose lanes) commit as one entry, the way
        // `Shell::stage_local` shapes single-op `SetLocal` batches.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let before = doc.hierarchy.clone();
        let base = journal.past_len();
        begin_group(
            &mut journal,
            &CommandMeta::significant("inspector edit"),
            9400,
        );
        for (index, key) in [9401, 9402, 9403].into_iter().enumerate() {
            let batch = Batch {
                ops: vec![Op::SetLocal {
                    id,
                    local: pose_at((index + 1) as f32),
                }],
                base_revision: doc.revision,
                idempotency_key: key,
                provenance: None,
            };
            apply_named(
                &mut doc,
                &mut journal,
                &batch,
                &CommandMeta::significant("inspector edit"),
            )
            .unwrap();
        }
        let merged = end_group(&mut journal).expect("inspector edits commit");
        assert_eq!(merged.key, 9400);
        assert_eq!(journal.past_len(), base + 1);
        let entry = journal.past.last().expect("merged entry");
        assert_eq!(entry.name, "inspector edit");
        assert!(entry.significant);
        assert!(needs_persist(entry));
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(3.0)));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before);
        redo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(3.0)));
    }

    #[test]
    fn insignificant_steps_skip_durable_journal_but_stay_undoable() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let (base_past, base_durable) = (journal.past_len(), durable_count(&journal));
        let step = CommandMeta::insignificant("preview");
        for i in 1..=10u64 {
            let batch = drag_step_batch(id, i as f32, doc.revision, 9500 + i);
            apply_named(&mut doc, &mut journal, &batch, &step).unwrap();
        }
        assert_eq!(journal.past_len(), base_past + 10);
        assert_eq!(durable_count(&journal), base_durable);
        assert!(journal
            .past
            .iter()
            .skip(base_past)
            .all(|entry| !needs_persist(entry)));
        for _ in 0..10 {
            undo(&mut doc, &mut journal).expect("preview steps undo");
        }
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(0.0)));
    }

    #[test]
    fn nested_groups_merge_into_one_entry() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let before = doc.hierarchy.clone();
        let base = journal.past_len();
        let outer = begin_group(&mut journal, &CommandMeta::significant("outer"), 9600);
        let inner = begin_group(&mut journal, &CommandMeta::significant("inner"), 9601);
        assert_ne!(outer, inner);
        assert_eq!(open_group_depth(&journal), 2);
        assert_eq!(open_group_id(&journal), Some(inner));
        let step = CommandMeta::insignificant("tick");
        for (index, key) in [9602, 9603].into_iter().enumerate() {
            let batch = drag_step_batch(id, (index + 1) as f32, doc.revision, key);
            apply_named(&mut doc, &mut journal, &batch, &step).unwrap();
        }
        let committed_inner = end_group(&mut journal).expect("inner commits");
        assert_eq!(committed_inner.key, 9601);
        // The inner merge stays inside the outer group as one tagged entry.
        assert_eq!(journal.past_len(), base + 1);
        assert_eq!(journal.past.last().expect("inner entry").group, outer);
        assert_eq!(open_group_depth(&journal), 1);
        for (x, key) in [(3.0, 9604), (4.0, 9605)] {
            let batch = drag_step_batch(id, x, doc.revision, key);
            apply_named(&mut doc, &mut journal, &batch, &step).unwrap();
        }
        let committed = end_group(&mut journal).expect("outer commits");
        assert_eq!(committed.key, 9600);
        assert_eq!(journal.past_len(), base + 1);
        let entry = journal.past.last().expect("merged entry");
        assert_eq!(entry.group, NO_GROUP);
        assert_eq!(entry.name, "outer");
        assert_eq!(entry.forward.len(), 4);
        assert_eq!(entry.keys.len(), 6);
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(4.0)));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before);
        redo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(4.0)));
        assert_eq!(open_group_depth(&journal), 0);
    }

    #[test]
    fn empty_group_commits_nothing() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let _ = seed_single_node(&mut doc, &mut journal);
        let (past, durable, revision) = (journal.past_len(), durable_count(&journal), doc.revision);
        begin_group(&mut journal, &CommandMeta::significant("empty"), 9700);
        assert_eq!(end_group(&mut journal), None);
        assert_eq!(
            (journal.past_len(), durable_count(&journal), doc.revision),
            (past, durable, revision)
        );
        assert_eq!(open_group_depth(&journal), 0);
        // Ending with no open group is also a no-op.
        assert_eq!(end_group(&mut journal), None);
        assert!(!cancel_group(&mut journal));
    }

    #[test]
    fn redo_after_new_grouped_edit_clears() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        begin_group(&mut journal, &CommandMeta::significant("drag"), 9800);
        let batch = drag_step_batch(id, 1.0, doc.revision, 9801);
        apply_named(
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::insignificant("tick"),
        )
        .unwrap();
        end_group(&mut journal).unwrap();
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(journal.future_len(), 1);
        // A new grouped edit clears the redo stack like any other apply.
        begin_group(&mut journal, &CommandMeta::significant("fresh"), 9810);
        let batch = drag_step_batch(id, 9.0, doc.revision, 9811);
        apply_named(
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::significant("fresh"),
        )
        .unwrap();
        end_group(&mut journal).unwrap();
        assert_eq!(journal.future_len(), 0);
        assert!(redo(&mut doc, &mut journal).is_none());
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(9.0)));
    }

    #[test]
    fn command_names_bound_and_plain_apply_stays_durable() {
        let long = "d".repeat(MAX_COMMAND_NAME_LEN + 40);
        let meta = CommandMeta::significant(long);
        assert_eq!(meta.name().len(), MAX_COMMAND_NAME_LEN);
        // Multibyte truncation stops at a character boundary.
        let wide = "é".repeat(MAX_COMMAND_NAME_LEN);
        let bounded = CommandMeta::new(wide.clone(), true);
        assert!(bounded.name().len() <= MAX_COMMAND_NAME_LEN);
        assert!(wide.starts_with(bounded.name()));
        assert!(bounded.is_significant());
        assert!(!CommandMeta::insignificant("tick").is_significant());
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let batch = insert_batch(id, ObjectId::NIL, 0.0, doc.revision, 9900);
        apply(&mut doc, &mut journal, &batch).unwrap();
        let entry = journal.past.last().expect("entry");
        assert_eq!(entry.name, "edit");
        assert!(entry.significant && needs_persist(entry));
        assert_eq!(entry.group, NO_GROUP);
        assert_eq!(entry.keys, vec![9900]);
        assert_eq!(CommandMeta::default(), CommandMeta::significant("edit"));
    }

    #[test]
    fn duplicate_key_inside_group_replays_or_conflicts() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        begin_group(&mut journal, &CommandMeta::significant("drag"), 9910);
        let first = drag_step_batch(id, 1.0, doc.revision, 9911);
        let applied = apply_named(
            &mut doc,
            &mut journal,
            &first,
            &CommandMeta::insignificant("tick"),
        )
        .unwrap();
        // Same key plus same payload replays without a new entry.
        let replayed = apply_named(
            &mut doc,
            &mut journal,
            &first,
            &CommandMeta::insignificant("tick"),
        )
        .unwrap();
        assert_eq!(replayed.revision, applied.revision);
        // Same key with another payload is a conflict and writes nothing.
        let before = doc.clone();
        let clash = drag_step_batch(id, 2.0, doc.revision, 9911);
        assert_eq!(
            apply_named(
                &mut doc,
                &mut journal,
                &clash,
                &CommandMeta::insignificant("tick")
            ),
            Err(BatchError::DuplicateKey { key: 9911 })
        );
        assert_eq!(doc, before);
        end_group(&mut journal).unwrap();
        // After the merge the member key stays retired: no silent reapply.
        let retry = drag_step_batch(id, 1.0, doc.revision, 9911);
        assert!(apply_named(
            &mut doc,
            &mut journal,
            &retry,
            &CommandMeta::insignificant("tick")
        )
        .is_err());
    }

    #[test]
    fn cancel_group_keeps_steps_individual() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let base = journal.past_len();
        begin_group(&mut journal, &CommandMeta::significant("aborted"), 9920);
        let step = CommandMeta::insignificant("tick");
        let first = drag_step_batch(id, 1.0, doc.revision, 9921);
        apply_named(&mut doc, &mut journal, &first, &step).unwrap();
        let second = drag_step_batch(id, 2.0, doc.revision, 9922);
        apply_named(&mut doc, &mut journal, &second, &step).unwrap();
        assert!(cancel_group(&mut journal));
        assert_eq!(journal.past_len(), base + 2);
        assert_eq!(open_group_depth(&journal), 0);
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(2.0)));
        // Each step still undoes on its own.
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(1.0)));
    }

    #[test]
    fn selective_revert_can_join_a_group() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: a,
                parent: ObjectId::NIL,
                local: pose_at(0.0),
            }],
            9930,
        );
        let receipt_b = selective_apply(
            &mut doc,
            &mut journal,
            vec![Op::Insert {
                id: b,
                parent: ObjectId::NIL,
                local: pose_at(1.0),
            }],
            9931,
        );
        let before = doc.hierarchy.clone();
        let base = journal.past_len();
        begin_group(&mut journal, &CommandMeta::significant("cleanup"), 9932);
        apply_selective_undo_with_meta(
            &mut doc,
            &mut journal,
            &[receipt_b],
            9933,
            None,
            &CommandMeta::insignificant("revert step"),
        )
        .unwrap();
        end_group(&mut journal).expect("revert commits");
        assert_eq!(journal.past_len(), base + 1);
        assert!(!doc.hierarchy.contains(b));
        assert_eq!(doc.hierarchy.local(a), Some(pose_at(0.0)));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy, before);
    }

    // Undo coalescing: slider ticks on one node plus lane fold into a single
    // entry (renzora/Fyrox school, reimplemented on the journal).

    fn slider_tick(id: ObjectId, x: f32, rev: u64, key: u64) -> Batch {
        Batch::new(
            vec![Op::SetLocal {
                id,
                local: pose_at(x),
            }],
            rev,
            key,
        )
    }

    #[test]
    fn slider_ticks_fold_to_one_entry_and_undo_restores_exact() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let start = doc.hierarchy.clone();
        let base_past = journal.past_len();
        let lane = merge_key(id, "pose.px");
        let meta = CommandMeta::significant("inspector edit");
        const TICKS: u64 = 100;
        for i in 1..=TICKS {
            let batch = slider_tick(id, i as f32, doc.revision, 5000 + i);
            let applied = apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
            assert_eq!(applied.revision, doc.revision);
        }
        // Proof: 100 ticks on one lane hold journal depth at +1 with one
        // revision bump each.
        assert_eq!(journal.past_len(), base_past + 1);
        assert_eq!(doc.revision, TICKS + 1);
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(TICKS as f32)));
        assert!(coalescing_open(&journal));
        let entry = journal.past.last().expect("folded entry");
        assert_eq!(entry.keys.len(), TICKS as usize);
        assert!(entry.significant && needs_persist(entry));
        assert!(!entry
            .provenance
            .parents
            .contains(&entry.provenance.receipt()));
        // One undo restores the pre-gesture hierarchy bit-exactly.
        undo(&mut doc, &mut journal).expect("folded gesture undoes in one step");
        assert_eq!(doc.hierarchy, start);
        assert!(!coalescing_open(&journal));
        // Redo replays the folded end value, and a fresh tick after undo
        // clears the redo stack like any other new edit.
        redo(&mut doc, &mut journal).expect("folded gesture redoes");
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(TICKS as f32)));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(journal.future_len(), 1);
        let batch = slider_tick(id, 1000.0, doc.revision, 6000);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        assert_eq!(journal.future_len(), 0);
        assert!(redo(&mut doc, &mut journal).is_none());
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(1000.0)));
    }

    #[test]
    fn other_lane_or_node_starts_a_fresh_entry() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let other = ObjectId::new();
        let rev = doc.revision;
        apply(
            &mut doc,
            &mut journal,
            &insert_batch(other, ObjectId::NIL, 0.0, rev, 5100),
        )
        .unwrap();
        let base = journal.past_len();
        let meta = CommandMeta::significant("inspector edit");
        let lane_x = merge_key(id, "pose.px");
        for i in 1..=3u64 {
            let batch = slider_tick(id, i as f32, doc.revision, 5200 + i);
            apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane_x).unwrap();
        }
        assert_eq!(journal.past_len(), base + 1);
        // Same node, other lane: seals the sweep and starts fresh.
        let lane_y = merge_key(id, "pose.py");
        let batch = slider_tick(id, 9.0, doc.revision, 5300);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane_y).unwrap();
        assert_eq!(journal.past_len(), base + 2);
        assert!(coalescing_open(&journal));
        // Same lane name, other node: a separate gesture too.
        let lane_other = merge_key(other, "pose.px");
        let batch = slider_tick(other, 7.0, doc.revision, 5400);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane_other).unwrap();
        assert_eq!(journal.past_len(), base + 3);
        // Each gesture undoes on its own, newest first.
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy.local(other), Some(pose_at(0.0)));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(3.0)));
        undo(&mut doc, &mut journal).unwrap();
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(0.0)));
    }

    #[test]
    fn release_apply_and_group_seal_the_gesture() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let base = journal.past_len();
        let meta = CommandMeta::significant("inspector edit");
        let lane = merge_key(id, "pose.px");
        for i in 1..=5u64 {
            let batch = slider_tick(id, i as f32, doc.revision, 5500 + i);
            apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        }
        assert_eq!(journal.past_len(), base + 1);
        // Release seals: the next tick on the same lane starts fresh.
        seal_coalescing(&mut journal);
        assert!(!coalescing_open(&journal));
        seal_coalescing(&mut journal);
        let batch = slider_tick(id, 6.0, doc.revision, 5510);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        assert_eq!(journal.past_len(), base + 2);
        // A plain apply seals too: the staged inspector commit breaks the run.
        let batch = slider_tick(id, 7.0, doc.revision, 5520);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        assert_eq!(journal.past_len(), base + 2);
        let plain = slider_tick(id, 8.0, doc.revision, 5530);
        apply_named(&mut doc, &mut journal, &plain, &meta).unwrap();
        assert_eq!(journal.past_len(), base + 3);
        assert!(!coalescing_open(&journal));
        let batch = slider_tick(id, 9.0, doc.revision, 5540);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        assert_eq!(journal.past_len(), base + 4);
        // A group drag seals as well: its steps merge through end_group.
        begin_group(&mut journal, &CommandMeta::significant("gizmo drag"), 5550);
        assert!(!coalescing_open(&journal));
        let batch = slider_tick(id, 10.0, doc.revision, 5551);
        apply_named(
            &mut doc,
            &mut journal,
            &batch,
            &CommandMeta::insignificant("tick"),
        )
        .unwrap();
        end_group(&mut journal).expect("drag commits");
        assert!(!coalescing_open(&journal));
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(10.0)));
    }

    #[test]
    fn structural_ops_push_and_seal_instead_of_folding() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let base = journal.past_len();
        let meta = CommandMeta::significant("inspector edit");
        let lane = merge_key(id, "pose.px");
        for i in 1..=2u64 {
            let batch = slider_tick(id, i as f32, doc.revision, 5600 + i);
            apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        }
        assert!(coalescing_open(&journal));
        // An insert never folds: it pushes and seals the gesture.
        let fresh = ObjectId::new();
        let batch = insert_batch(fresh, ObjectId::NIL, 0.0, doc.revision, 5610);
        apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        assert_eq!(journal.past_len(), base + 2);
        assert!(!coalescing_open(&journal));
        undo(&mut doc, &mut journal).unwrap();
        assert!(!doc.hierarchy.contains(fresh));
        assert_eq!(doc.hierarchy.local(id), Some(pose_at(2.0)));
    }

    #[test]
    fn folded_panel_tick_costs_under_a_millisecond() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let lane = merge_key(id, "pose.px");
        let meta = CommandMeta::significant("inspector edit");
        let started = std::time::Instant::now();
        for i in 1..=100u64 {
            let batch = slider_tick(id, i as f32, doc.revision, 5700 + i);
            apply_coalesced(&mut doc, &mut journal, &batch, &meta, lane).unwrap();
        }
        let mean_nanos = started.elapsed().as_nanos() / 100;
        assert!(
            mean_nanos < 1_000_000,
            "panel tick mean {mean_nanos}ns exceeds the 1ms budget"
        );
        // The panel lists one entry for the whole sweep, not a hundred.
        assert_eq!(journal.past_len(), 2);
        assert_eq!(durable_count(&journal), 2);
    }

    #[test]
    fn folded_member_keys_stay_retired_like_group_members() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let lane = merge_key(id, "pose.px");
        let meta = CommandMeta::significant("inspector edit");
        let first = slider_tick(id, 1.0, doc.revision, 5801);
        apply_coalesced(&mut doc, &mut journal, &first, &meta, lane).unwrap();
        let second = slider_tick(id, 2.0, doc.revision, 5802);
        apply_coalesced(&mut doc, &mut journal, &second, &meta, lane).unwrap();
        // Retrying the folded-away first tick is a conflict, never a replay.
        let before = doc.clone();
        assert_eq!(
            apply_coalesced(&mut doc, &mut journal, &first, &meta, lane),
            Err(BatchError::DuplicateKey { key: 5801 })
        );
        assert_eq!(doc, before);
    }

    #[test]
    fn grouped_gizmo_drag_fifty_tick_sweep_is_one_entry_with_tool_and_args() {
        use crate::gizmo::{drag_batch, drag_to_local, GizmoDrag};
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = seed_single_node(&mut doc, &mut journal);
        let start = doc.hierarchy.clone();
        let base_past = journal.past_len();
        let commit = CommandMeta::significant("door scene.place next_to wall:north");
        begin_group(&mut journal, &commit, 9850);
        let step = CommandMeta::insignificant("gizmo drag step");
        for i in 0..50u64 {
            let current = doc.hierarchy.local(id).expect("selection lives");
            let posed = drag_to_local(
                current,
                &GizmoDrag::Translate {
                    delta: [0.02, 0.0, 0.0],
                },
            )
            .expect("finite delta");
            let shaped = drag_batch(id, posed, doc.revision, 9851 + i);
            apply_named(&mut doc, &mut journal, &shaped, &step).unwrap();
        }
        let merged = end_group(&mut journal).expect("sweep commits one entry");
        assert_eq!(journal.past_len(), base_past + 1);
        let entry = journal.past.last().expect("merged entry");
        assert_eq!(entry.name, "door scene.place next_to wall:north");
        assert!(entry.significant);
        assert_eq!(merged.key, entry.key);
        let landed = doc.hierarchy.local(id).expect("selection lives");
        assert!((landed.position[0] - 1.0).abs() <= 1e-3);
        undo(&mut doc, &mut journal).expect("sweep undoes in one step");
        assert_eq!(doc.hierarchy, start);
        redo(&mut doc, &mut journal).expect("sweep redoes in one step");
        assert!((doc.hierarchy.local(id).expect("pose").position[0] - 1.0).abs() <= 1e-3);
    }

    #[test]
    fn grouped_gizmo_drag_merge_ends_annuls_and_folds_after_noop() {
        use crate::gizmo::drag_batch;
        let (mut doc, mut journal) = (Document::new(), Journal::new());
        let id = seed_single_node(&mut doc, &mut journal);
        let lane = merge_key(id, "pose.px");
        let meta = CommandMeta::significant("edit");
        let first = drag_batch(id, pose_at(1.0), doc.revision, 7010);
        apply_coalesced(&mut doc, &mut journal, &first, &meta, lane).unwrap();
        let second = slider_tick(id, 2.0, doc.revision, 7011);
        apply_coalesced(&mut doc, &mut journal, &second, &meta, lane).unwrap();
        assert_eq!(journal.past.last().expect("fold").forward, second.ops);
        seal_coalescing(&mut journal);
        let cur = doc.hierarchy.local(id).expect("live");
        let same = Batch::new(vec![Op::SetLocal { id, local: cur }], doc.revision, 7012);
        let (rev, base) = (doc.revision, journal.past_len());
        apply_coalesced(&mut doc, &mut journal, &same, &meta, lane).unwrap();
        assert_eq!((journal.past_len(), doc.revision), (base, rev));
        let third = slider_tick(id, 3.0, doc.revision, 7013);
        apply_coalesced(&mut doc, &mut journal, &third, &meta, lane).unwrap();
        let fourth = slider_tick(id, 4.0, doc.revision, 7014);
        apply_coalesced(&mut doc, &mut journal, &fourth, &meta, lane).unwrap();
        assert_eq!(journal.past_len(), base + 1);
        assert_eq!(journal.past.last().expect("fold").forward, fourth.ops);
    }
}
