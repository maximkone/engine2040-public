// Undo-coalescing gesture folding for the command service.
//
// Included via `include!("commands_coalesce.rs")` inside `commands.rs` so the
// journal types stay in one module without touching `lib.rs`. This file has
// no `use` block by construction: it shares the parent scope (`Fnv64`,
// `ObjectId`, `Document`, `Batch`, `Journal`, `JournalEntry`, `CommandMeta`,
// `NO_GROUP`, `BatchError`, `Applied`, plus `stage`, `build_record` and
// `commit` helpers defined in the parent). The public surface
// (`COALESCE_WINDOW`, `merge_key`, `coalescing_open`, `seal_coalescing`,
// `apply_coalesced`) keeps its signatures; the fold path reports through
// `Option` instead of panicking.

/// Idle-gap window an open coalescing gesture survives, in revisions.
///
/// Consecutive slider ticks land one revision apart, so a 100-tick sweep
/// folds into one entry; a pause with more than this many unrelated
/// revisions since the last fold seals the gesture and the next tick starts
/// a fresh entry. Pure revision math: no wall clock enters the journal.
pub const COALESCE_WINDOW: u64 = 30;

/// Open undo-coalescing gesture: the lane folding into the journal top.
/// Transient UI state only (see [`Journal`]): never journaled, never saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CoalesceCursor {
    /// Lane folding into the top entry (see [`merge_key`]).
    lane: u64,
    /// Journal key of the gesture entry (its first tick's batch key).
    entry_key: u64,
    /// Document revision the gesture started from (the entry baseline).
    base_revision: u64,
}

/// Merge key for one undo-coalescing lane: the target node plus the
/// caller-named lane (for example `"pose.px"`).
///
/// Same node plus same lane folds into the journal top; any other node or
/// lane seals the open gesture and starts a fresh entry. Hashed with the
/// engine-core FNV owner over the fixed-size node id plus the lane bytes, so
/// the same node and lane seal the same key on every platform and build.
/// Pure and allocation-free: safe on any tick or panel path.
pub fn merge_key(id: ObjectId, lane: &str) -> u64 {
    let mut hash = Fnv64::new();
    hash.write(id.as_uuid().as_bytes());
    hash.write(lane.as_bytes());
    hash.finish()
}

/// True while a coalescing gesture holds the journal top open: the next
/// [`apply_coalesced`] tick with the same lane folds instead of pushing. Any
/// seal ([`seal_coalescing`], a plain apply, a group edit, undo, redo)
/// closes it. No production Shell reader yet; tests observe the gesture state.
pub fn coalescing_open(journal: &Journal) -> bool {
    journal.coalesce.is_some()
}

/// Seal the open coalescing gesture, if any: the next [`apply_coalesced`]
/// tick starts a fresh undo entry even on the same lane.
///
/// A future slider caller would call this on gesture release, on apply of a
/// staged edit and on focus or selection change, so one slider sweep never
/// swallows a later one. Currently test-only; the live Shell drag uses
/// `begin_group`/`end_group`. Sealing an already-sealed journal is a no-op.
pub fn seal_coalescing(journal: &mut Journal) {
    journal.coalesce = None;
}

/// True when `batch` may fold into the journal top under `key`: no group
/// holds the journal, the cursor still names this lane, the top entry is the
/// gesture entry on the same node with the same value-kind op, and the idle
/// gap since its last fold fits [`COALESCE_WINDOW`].
fn foldable(journal: &Journal, doc: &Document, key: u64, batch: &Batch) -> bool {
    if !journal.groups.is_empty() || batch.ops.len() != 1 {
        return false;
    }
    let op = &batch.ops[0];
    if !matches!(op, Op::SetLocal { .. } | Op::SetMaterial { .. }) {
        return false;
    }
    let Some(cursor) = journal.coalesce else {
        return false;
    };
    if cursor.lane != key {
        return false;
    }
    let Some(entry) = journal.past.last() else {
        return false;
    };
    if entry.group != NO_GROUP
        || (entry.key != cursor.entry_key && !entry.keys.contains(&cursor.entry_key))
        || entry.base_revision != cursor.base_revision
        || entry.forward.len() != 1
    {
        return false;
    }
    let first = &entry.forward[0];
    if first.id() != op.id() || std::mem::discriminant(first) != std::mem::discriminant(op) {
        return false;
    }
    doc.revision.saturating_sub(entry.result_revision) <= COALESCE_WINDOW
}

/// Idempotency gate shared by [`apply_named`] and [`apply_coalesced`]:
/// `Some` answers a known key without touching the document, `None` means
/// the batch is fresh. Checked before the revision gate so a retry after a
/// successful apply replays its receipt instead of tripping as stale.
/// Retired keys (merged group members, folded tick keys) stay conflicts,
/// never silent reapplies.
fn replay_known_key(journal: &Journal, batch: &Batch) -> Option<Result<Applied, BatchError>> {
    let key = batch.idempotency_key;
    if let Some(index) = journal
        .past
        .iter()
        .position(|entry| entry.key == key || entry.keys.contains(&key))
    {
        let seen = &journal.past[index];
        if seen.key == key && seen.forward == batch.ops && seen.base_revision == batch.base_revision
        {
            return Some(Ok(Applied {
                revision: seen.result_revision,
                before_digest: seen.before_digest,
                staged_digest: seen.staged_digest,
                after_digest: seen.after_digest,
                effects: seen.effects.clone(),
                key,
                provenance: seen.provenance.clone(),
                journal_index: index,
            }));
        }
        return Some(Err(BatchError::DuplicateKey { key }));
    }
    if journal
        .future
        .iter()
        .any(|entry| entry.key == key || entry.keys.contains(&key))
    {
        return Some(Err(BatchError::UndoneKey { key }));
    }
    None
}

/// Fold one tick into the open gesture top: rewrite the forward op to the
/// newest value and rebuild the receipt, keeping the gesture baseline
/// (original inverse ops, base revision, name, flag, group). The caller
/// checked [`foldable`]. Each fold bumps the revision by one and clears the
/// redo stack, like any new edit.
///
/// Returns `None` when the gesture entry vanished between the [`foldable`]
/// check and the fold; the caller then takes the fresh-gesture path instead
/// of panicking. Staging failures still report through `Some(Err(..))`.
fn fold_tick(
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
) -> Option<Result<Applied, BatchError>> {
    let staged = match stage(doc, batch) {
        Ok(staged) => staged,
        Err(error) => return Some(Err(error)),
    };
    // Pop the gesture entry while the fresh receipt builds: its previous
    // receipt is superseded by the fold, not a parent of it.
    let top = journal.past.pop()?;
    let result_revision = doc.revision + 1;
    let provenance = build_record(batch, result_revision, journal);
    doc.hierarchy = staged.tree;
    doc.materials = staged.materials;
    doc.components = staged.components;
    doc.bonds = staged.bonds;
    doc.material_set = staged.material_set;
    doc.revision = result_revision;
    let after_digest = document_digest(doc);
    let mut keys = top.keys;
    if !keys.contains(&batch.idempotency_key) {
        keys.push(batch.idempotency_key);
    }
    let gesture_key = top.key;
    journal.past.push(JournalEntry {
        key: gesture_key,
        forward: batch.ops.clone(),
        inverse: top.inverse,
        inverse_groups: top.inverse_groups,
        base_revision: top.base_revision,
        result_revision,
        before_digest: top.before_digest,
        staged_digest: after_digest,
        after_digest,
        effects: staged.effects.clone(),
        provenance: provenance.clone(),
        name: top.name,
        significant: top.significant,
        group: top.group,
        keys,
    });
    journal.future.clear();
    Some(Ok(Applied {
        revision: result_revision,
        before_digest: top.before_digest,
        staged_digest: after_digest,
        after_digest,
        effects: staged.effects,
        key: gesture_key,
        provenance,
        journal_index: journal.past.len() - 1,
    }))
}

/// Commit a staged batch: publish the revision, push one journal entry and
/// clear the redo stack. Shared by [`apply_named`] and the fresh-gesture
/// path of [`apply_coalesced`] so both seal the same way.
fn commit_staged(
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
    meta: &CommandMeta,
    staged: Staged,
    provenance: Provenance,
    before_digest: [u8; 32],
) -> Applied {
    let result_revision = doc.revision + 1;
    doc.hierarchy = staged.tree;
    doc.materials = staged.materials;
    doc.components = staged.components;
    doc.bonds = staged.bonds;
    doc.material_set = staged.material_set;
    doc.revision = result_revision;
    let after_digest = document_digest(doc);
    let group = journal
        .groups
        .last()
        .map(|frame| frame.id)
        .unwrap_or(NO_GROUP);
    journal.past.push(JournalEntry {
        key: batch.idempotency_key,
        forward: batch.ops.clone(),
        inverse: staged.inverse,
        inverse_groups: staged.per_op,
        base_revision: batch.base_revision,
        result_revision: doc.revision,
        before_digest,
        staged_digest: after_digest,
        after_digest,
        effects: staged.effects.clone(),
        provenance: provenance.clone(),
        name: meta.name.clone(),
        significant: meta.significant,
        group,
        keys: vec![batch.idempotency_key],
    });
    // Eviction pauses inside an open group so the later merge still restores
    // exact bytes; groups are short drags the caller closes promptly, and
    // [`end_group`] evicts once the last group commits.
    if journal.groups.is_empty() && journal.past.len() > JOURNAL_CAP {
        journal.past.remove(0);
    }
    journal.future.clear();
    Applied {
        revision: doc.revision,
        before_digest,
        staged_digest: after_digest,
        after_digest,
        effects: staged.effects,
        key: batch.idempotency_key,
        provenance,
        journal_index: journal.past.len() - 1,
    }
}

/// Apply a slider-like tick with undo coalescing under `key` (see
/// [`merge_key`]): consecutive ticks on the same node plus lane fold into one
/// journal entry, keeping journal depth at +1 no matter the tick count.
///
/// A fold rewrites the entry's forward op to the newest value and rebuilds
/// its receipt, but keeps the gesture baseline: the original inverse ops,
/// base revision, name, flag and group. Each tick still bumps the revision
/// by exactly one, so undo restores the pre-gesture hierarchy bit-exactly
/// and redo replays the folded value. The folded receipt keeps the gesture
/// key and retires each tick key (like merged group members: retrying an
/// intermediate tick key is a conflict, never a silent reapply). A new edit
/// clears the redo stack, as with every apply.
///
/// A tick starts a fresh entry (and seals the old gesture) when the lane or
/// node differs, the op is not a single value write (`SetLocal`,
/// `SetMaterial`), a group is open, the idle gap since the last fold exceeds
/// [`COALESCE_WINDOW`], or the gesture was sealed ([`seal_coalescing`], a
/// plain apply, a group edit, undo, redo). Structural ops never fold: they
/// push and seal. Errors match [`apply_named`]: stale revisions, bad ops,
/// duplicate keys and undone keys write nothing.
///
/// Status: test-only with no production slider caller yet; the live Shell
/// drag uses `begin_group`/`end_group`. A future caller would drive one lane
/// per gesture: `merge_key` per tick, [`seal_coalescing`] on release,
/// staged-apply and focus change.
pub fn apply_coalesced(
    doc: &mut Document,
    journal: &mut Journal,
    batch: &Batch,
    meta: &CommandMeta,
    key: u64,
) -> Result<Applied, BatchError> {
    if let Some(known) = replay_known_key(journal, batch) {
        return known;
    }
    if foldable(journal, doc, key, batch) {
        if let Some(folded) = fold_tick(doc, journal, batch) {
            return folded;
        }
        // The gesture entry vanished between the check and the fold: fall
        // through to the fresh-gesture path rather than panicking.
    }
    // Fresh gesture: the old one seals, then the tick commits like any apply.
    seal_coalescing(journal);
    let before_digest = document_digest(doc);
    let staged = stage(doc, batch)?;
    if batch.ops == staged.inverse {
        seal_coalescing(journal);
        // The tick is its own inverse: nothing lands, so all three digests
        // describe the unchanged document.
        let digest = before_digest;
        return Ok(Applied {
            revision: doc.revision,
            before_digest: digest,
            staged_digest: digest,
            after_digest: digest,
            effects: staged.effects,
            key: batch.idempotency_key,
            provenance: build_record(batch, doc.revision, journal),
            journal_index: journal.past.len().saturating_sub(1),
        });
    }
    let result_revision = doc.revision + 1;
    let provenance = build_record(batch, result_revision, journal);
    let applied = commit_staged(doc, journal, batch, meta, staged, provenance, before_digest);
    // Only value writes arm the gesture, and only outside groups (group
    // steps merge through `end_group` instead).
    if journal.groups.is_empty()
        && batch.ops.len() == 1
        && matches!(batch.ops[0], Op::SetLocal { .. } | Op::SetMaterial { .. })
    {
        journal.coalesce = Some(CoalesceCursor {
            lane: key,
            entry_key: batch.idempotency_key,
            base_revision: batch.base_revision,
        });
    }
    Ok(applied)
}

#[cfg(test)]
mod coalesce_unit_tests {
    use super::merge_key;
    use engine_scene::ids::ObjectId;

    #[test]
    fn merge_keys_split_nodes_and_lanes_deterministically() {
        let a = ObjectId::new();
        let b = ObjectId::new();
        assert_eq!(merge_key(a, "pose.px"), merge_key(a, "pose.px"));
        assert_ne!(merge_key(a, "pose.px"), merge_key(a, "pose.py"));
        assert_ne!(merge_key(a, "pose.px"), merge_key(b, "pose.px"));
        assert_ne!(merge_key(a, ""), merge_key(a, "pose.px"));
    }
}
