//! Per-op staging on a private copy: the all-or-nothing core of the command
//! service.
//!
//! [`crate::commands::apply`], [`crate::commands::undo`], [`crate::commands::redo`]
//! and the geometry readers stage every op on clones first and commit only
//! when the whole batch succeeded. Lives beside `commands.rs` instead of inside
//! it because that file is over the size pin and may not grow
//! (`docs/RUST-QUALITY.md`).

use std::collections::BTreeMap;

use engine_material::{MaterialSet, RecordError};
use engine_scene::ids::ObjectId;
use engine_scene::transform::{Hierarchy, HierarchyError, Transform};

use super::component::{stage_remove_component, stage_set_component};
use super::fracture::{stage_set_bond, take_bonds};
use super::{Batch, BatchError, BondTable, ComponentTable, Document, MaterialName, Op};

/// Staged batch: the resulting tree, its inverse ops and its preview lines.
pub(crate) struct Staged {
    pub(crate) tree: Hierarchy,
    /// Resulting material bindings (staged beside the tree, committed with it).
    pub(crate) materials: BTreeMap<String, MaterialName>,
    /// Resulting components (staged beside the tree, committed with it).
    pub(crate) components: ComponentTable,
    /// Resulting bond paint (staged beside the tree, committed with it).
    pub(crate) bonds: BondTable,
    /// Resulting material table (staged beside the tree, committed with it).
    pub(crate) material_set: MaterialSet,
    pub(crate) inverse: Vec<Op>,
    /// Per-forward-op inverse groups in forward order; flattened (reversed)
    /// into `inverse` above. Stored on the journal entry for selective undo.
    pub(crate) per_op: Vec<Vec<Op>>,
    pub(crate) effects: Vec<String>,
}

/// Revision gate plus per-op staging on a private copy. The caller commits
/// the tree only after every op succeeds, which keeps batches all-or-nothing.
pub(crate) fn stage(doc: &Document, batch: &Batch) -> Result<Staged, BatchError> {
    if batch.base_revision != doc.revision {
        return Err(BatchError::StaleRevision {
            expected: doc.revision,
            got: batch.base_revision,
        });
    }
    let mut tree = doc.hierarchy.clone();
    let mut materials = doc.materials.clone();
    let mut components = doc.components.clone();
    let mut bonds = doc.bonds.clone();
    let mut material_set = doc.material_set.clone();
    let mut per_op_inverse = Vec::with_capacity(batch.ops.len());
    let mut effects = Vec::with_capacity(batch.ops.len());
    for op in &batch.ops {
        let (inverse, line) = apply_one(
            &mut tree,
            &mut materials,
            &mut components,
            &mut bonds,
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
        bonds,
        material_set,
        inverse,
        per_op: per_op_inverse,
        effects,
    })
}

/// Run staged ops without recording (undo/redo internals). Atomic by the
/// same rule: the caller swaps the tree in only on full success.
pub(crate) fn apply_ops(
    tree: &mut Hierarchy,
    materials: &mut BTreeMap<String, MaterialName>,
    components: &mut ComponentTable,
    bonds: &mut BondTable,
    material_set: &mut MaterialSet,
    ops: &[Op],
) -> Result<(), BatchError> {
    for op in ops {
        apply_one(tree, materials, components, bonds, material_set, op).map(|_| ())?;
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
    bonds: &mut BondTable,
    material_set: &mut MaterialSet,
    op: &Op,
) -> Result<(Vec<Op>, String), BatchError> {
    match op {
        Op::Insert { id, parent, local } => stage_insert(tree, op, *id, *parent, *local),
        Op::Remove { id } => stage_remove(tree, materials, components, bonds, op, *id),
        Op::SetComponent { id, component } => {
            stage_set_component(tree, components, *id, *component)
        }
        Op::RemoveComponent { id, kind } => stage_remove_component(tree, components, *id, *kind),
        Op::SetLocal { id, local } => stage_set_local(tree, op, *id, *local),
        Op::Reparent { id, new_parent } => stage_reparent(tree, op, *id, *new_parent),
        Op::SetMaterial { id, material } => stage_set_material(tree, materials, *id, *material),
        Op::SetBond { id, a, b, strength } => stage_set_bond(tree, bonds, *id, *a, *b, *strength),
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
    bonds: &mut BondTable,
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
    // Bond paint dies with its node too; undo re-paints after the re-insert.
    inverse.extend(take_bonds(bonds, id));
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
