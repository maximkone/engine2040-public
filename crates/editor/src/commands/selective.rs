//! Selective undo: revert chosen receipts alone, keep what later work pinned.
//!
//! Owner: the command service (`super`, which re-exports every public name
//! here). Status: test-only with no production undo-panel or door caller
//! yet. Reads the journal only; the revert itself is one ordinary atomic batch,
//! so units, determinism and error shapes are the command service's own.
//! Errors: [`UndoError`] (unknown receipt, or the revert batch refused).

use std::collections::BTreeSet;
use std::fmt;

use engine_scene::ids::ObjectId;

use super::{
    apply_named, Applied, Batch, BatchError, CommandMeta, Document, Journal, JournalEntry, Op,
    ProvenanceStamp,
};

/// Why one selected op stays in place under selective undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictReason {
    /// A later unselected batch wrote the same node, so reverting would
    /// clobber newer work (for example `SetLocal` over `SetLocal`).
    OverwrittenByLater,
    /// A later unselected batch assumed this op's effect (a child added under
    /// an inserted node, a touch of a created node), so reverting would pull
    /// the floor out from under it.
    DependsOnLater,
}

/// One selected op left in place plus why. The selective-undo panel shows these;
/// [`apply_selective_undo`] never rewrites them.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    /// The selected forward op that cannot be reverted alone.
    pub op: Op,
    /// Which later-work hazard pins it.
    pub reason: ConflictReason,
}

/// Dry-run of a selective undo: inverse ops ready to apply plus the ops that
/// must stay. `reverted_ops` run in apply order (newest selected batch first,
/// last-op-first inside each batch); `conflicts` list in journal order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UndoPreview {
    /// Inverse ops for the revertible subset, in apply order.
    pub reverted_ops: Vec<Op>,
    /// Selected ops pinned by later unselected work, in journal order.
    pub conflicts: Vec<Conflict>,
}

/// What rejected a selective undo. Unknown receipts write nothing; a failed
/// revert batch also writes nothing (the underlying [`apply`] is atomic).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    /// No applied batch carries this receipt id.
    UnknownReceipt {
        /// The receipt id with no applied batch.
        receipt: u64,
    },
    /// The revert batch itself failed to apply; the document is unchanged.
    ApplyFailed {
        /// The underlying batch failure.
        source: BatchError,
    },
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReceipt { receipt } => {
                write!(f, "no applied batch carries receipt {receipt}")
            }
            Self::ApplyFailed { source } => write!(f, "selective undo failed to apply: {source}"),
        }
    }
}

impl std::error::Error for UndoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownReceipt { .. } => None,
            Self::ApplyFailed { source } => Some(source),
        }
    }
}

impl From<BatchError> for UndoError {
    fn from(source: BatchError) -> Self {
        Self::ApplyFailed { source }
    }
}

/// Preview reverting only the batches named by `receipt_ids`
/// ([`Provenance::receipt`] ids over [`Journal::past`]).
///
/// Later batches that are also selected revert together, so they never block
/// each other; only later UNSELECTED batches pin ops. A pinned op pins its
/// batchmates below it too (their inverses were staged on top of it), reported
/// as [`ConflictReason::DependsOnLater`]. Reads the journal only: pure,
/// no side effects, deterministic in journal order.
pub fn undo_select(journal: &Journal, receipt_ids: &[u64]) -> Result<UndoPreview, UndoError> {
    let mut selected: BTreeSet<usize> = BTreeSet::new();
    for receipt in receipt_ids {
        let mut hit = false;
        for (index, entry) in journal.past.iter().enumerate() {
            if entry.provenance.receipt() == *receipt {
                selected.insert(index);
                hit = true;
            }
        }
        if !hit {
            return Err(UndoError::UnknownReceipt { receipt: *receipt });
        }
    }
    let mut preview = UndoPreview::default();
    // Revertible flags per selected entry in ascending selected order, reused
    // by the apply pass below.
    let mut revertible: Vec<Vec<bool>> = Vec::new();
    for index in selected.iter() {
        let entry = &journal.past[*index];
        let direct = direct_conflicts(journal, &selected, *index);
        // Downward propagation inside the batch: op `i` reverts only when
        // every op after it reverts too, since its inverse was staged before
        // them. A batchmate pinned by later work transitively pins the ops
        // below it.
        let mut keep = vec![true; entry.forward.len()];
        for i in (0..entry.forward.len()).rev() {
            let after_ok = i + 1 >= entry.forward.len() || keep[i + 1];
            if let Some(reason) = direct[i] {
                keep[i] = false;
                preview.conflicts.push(Conflict {
                    op: entry.forward[i].clone(),
                    reason,
                });
            } else if !after_ok {
                keep[i] = false;
                preview.conflicts.push(Conflict {
                    op: entry.forward[i].clone(),
                    reason: ConflictReason::DependsOnLater,
                });
            }
        }
        revertible.push(keep);
    }
    // Apply order: newest selected batch first, last-op-first inside a batch.
    for (order, index) in selected.iter().enumerate().rev() {
        let entry = &journal.past[*index];
        let keep = &revertible[order];
        for i in (0..entry.forward.len()).rev() {
            if keep[i] {
                preview
                    .reverted_ops
                    .extend(entry.inverse_groups[i].iter().cloned());
            }
        }
    }
    Ok(preview)
}

/// Per-forward-op direct conflict against later unselected batches. Depends
/// wins over overwritten when several later ops touch the same node.
fn direct_conflicts(
    journal: &Journal,
    selected: &BTreeSet<usize>,
    index: usize,
) -> Vec<Option<ConflictReason>> {
    let entry = &journal.past[index];
    let mut kids_per_op: Vec<Vec<ObjectId>> = vec![Vec::new(); entry.forward.len()];
    for (i, op) in entry.forward.iter().enumerate() {
        if let Op::Remove { id } = op {
            kids_per_op[i] = removed_kids(entry, i, *id);
        }
    }
    let mut out = vec![None; entry.forward.len()];
    for later in journal.past.iter().enumerate().skip(index + 1) {
        if selected.contains(&later.0) {
            continue;
        }
        for later_op in &later.1.forward {
            for (i, op) in entry.forward.iter().enumerate() {
                if let Some(reason) = classify_pair(op, later_op, &kids_per_op[i]) {
                    out[i] = Some(match out[i] {
                        Some(ConflictReason::DependsOnLater) => ConflictReason::DependsOnLater,
                        Some(ConflictReason::OverwrittenByLater) => match reason {
                            ConflictReason::DependsOnLater => reason,
                            ConflictReason::OverwrittenByLater => {
                                ConflictReason::OverwrittenByLater
                            }
                        },
                        None => reason,
                    });
                }
            }
        }
    }
    out
}

/// Children a `Remove` relinked, read off its stored inverse group
/// (`Reparent` rows back under the removed node). Journal-only: no live read.
fn removed_kids(entry: &JournalEntry, group: usize, removed: ObjectId) -> Vec<ObjectId> {
    let mut kids = Vec::new();
    if let Some(inverses) = entry.inverse_groups.get(group) {
        for op in inverses {
            if let Op::Reparent { id, new_parent } = op {
                if *new_parent == removed && !kids.contains(id) {
                    kids.push(*id);
                }
            }
        }
    }
    kids
}

/// One later op's hazard over one selected op, if any.
fn classify_pair(
    selected_op: &Op,
    later_op: &Op,
    selected_kids: &[ObjectId],
) -> Option<ConflictReason> {
    // A later batch parenting under the selected node assumed it where it is.
    if later_op.linked_parent() != ObjectId::NIL && later_op.linked_parent() == selected_op.id() {
        return Some(ConflictReason::DependsOnLater);
    }
    // A later touch of a relinked child assumed the removal stands.
    if selected_kids.contains(&later_op.id())
        || (later_op.linked_parent() != ObjectId::NIL
            && selected_kids.contains(&later_op.linked_parent()))
    {
        return Some(ConflictReason::DependsOnLater);
    }
    if later_op.id() != selected_op.id() {
        return None;
    }
    match (selected_op, later_op) {
        // Later work on a created node assumed the insert; later work after a
        // removal assumed the removal. Both are structural, not value writes.
        (Op::Insert { .. } | Op::Remove { .. }, _) => Some(ConflictReason::DependsOnLater),
        // A newer value (or deletion) supersedes the selected value.
        // Bindings are values like poses: a newer binding supersedes.
        (
            Op::SetLocal { .. }
            | Op::Reparent { .. }
            | Op::SetMaterial { .. }
            | Op::SetComponent { .. }
            | Op::RemoveComponent { .. },
            Op::SetLocal { .. }
            | Op::Reparent { .. }
            | Op::SetMaterial { .. }
            | Op::SetComponent { .. }
            | Op::RemoveComponent { .. }
            | Op::Remove { .. },
        ) => Some(ConflictReason::OverwrittenByLater),
        // The node was removed and recreated after the selected value write:
        // the recreation assumed the selected value was gone.
        (
            Op::SetLocal { .. }
            | Op::Reparent { .. }
            | Op::SetMaterial { .. }
            | Op::SetComponent { .. }
            | Op::RemoveComponent { .. },
            Op::Insert { .. },
        ) => Some(ConflictReason::DependsOnLater),
        // Material-table ops touch no node (their target id is nil), so only
        // two table ops reach this arm. A later table edit supersedes an
        // earlier one: pin the earlier edit instead of reverting it blind.
        (Op::PutMaterial { .. } | Op::SetMaterialParam { .. } | Op::RemoveLastMaterial, _)
        | (_, Op::PutMaterial { .. } | Op::SetMaterialParam { .. } | Op::RemoveLastMaterial) => {
            Some(ConflictReason::OverwrittenByLater)
        }
    }
}

/// Apply the non-conflicting subset of `receipt_ids` as one undoable batch
/// with its own receipt.
///
/// Builds the [`UndoPreview::reverted_ops`] as a fresh [`Batch`] (key
/// `idempotency_key`, stamp or human) and [`apply`]s it: revision grows by
/// one, standard [`undo`] restores it, and conflicted ops stay untouched. An
/// empty selection (or a fully pinned one) applies as a no-op batch so the
/// attempt itself stays journalled. Unknown receipts or a failed revert write
/// nothing.
pub fn apply_selective_undo(
    doc: &mut Document,
    journal: &mut Journal,
    receipt_ids: &[u64],
    idempotency_key: u64,
    stamp: Option<ProvenanceStamp>,
) -> Result<Applied, UndoError> {
    apply_selective_undo_with_meta(
        doc,
        journal,
        receipt_ids,
        idempotency_key,
        stamp,
        &CommandMeta::default(),
    )
}

/// [`apply_selective_undo`] with a command name and durable flag, so the
/// revert batch can join a group or skip durability like any other edit.
pub fn apply_selective_undo_with_meta(
    doc: &mut Document,
    journal: &mut Journal,
    receipt_ids: &[u64],
    idempotency_key: u64,
    stamp: Option<ProvenanceStamp>,
    meta: &CommandMeta,
) -> Result<Applied, UndoError> {
    let preview = undo_select(journal, receipt_ids)?;
    let batch = Batch {
        ops: preview.reverted_ops,
        base_revision: doc.revision,
        idempotency_key,
        provenance: stamp,
    };
    Ok(apply_named(doc, journal, &batch, meta)?)
}
